// Actual StateTransition redelegation observations. Archive-only storage
// observers record requests and values; they do not change storage decisions.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
)

func fullSourceConfig(source byte) chain_config.ChainConfig {
	cfg := custodyConfig(true, true, false)
	cfg.GenesisBalances[custodyDelegator] = big.NewInt(4000)
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(5000)
	cfg.DPOS.MinimumDeposit = big.NewInt(100)
	cfg.DPOS.InitialValidators = append(cfg.DPOS.InitialValidators, chain_config.GenesisValidator{
		Address: common.BytesToAddress([]byte{0x32}), Owner: custodyOwner,
		VrfKey: bytes.Repeat([]byte{0x55}, 32), Commission: 100,
		Delegations: core.BalanceMap{custodyDelegator: big.NewInt(1000)},
	})
	for i := range cfg.DPOS.InitialValidators {
		if cfg.DPOS.InitialValidators[i].Address == common.BytesToAddress([]byte{source}) {
			cfg.DPOS.InitialValidators[i].Delegations[custodyOwner] = big.NewInt(1000)
		}
	}
	return cfg
}

func main() {
	rows := []map[string]any{}
	for _, c := range []struct {
		name                      string
		from, to                  byte
		amount, maximum, value    int64
		gas                       uint64
		missingDelegation, repeat bool
	}{
		{name: "full_source_swap_last", from: 0x31, to: 0x32, amount: 1000, gas: 200000, repeat: true},
		{name: "full_source_last_item", from: 0x32, to: 0x31, amount: 1000, gas: 200000, repeat: true},
	} {
		cfg := fullSourceConfig(c.from)
		if c.maximum != 0 {
			cfg.DPOS.ValidatorMaximumStake = big.NewInt(c.maximum)
		}
		database := newNativeSimulationDB()
		transition := nativeSimulationTransition(database, &cfg)
		advanceCustodyBlock(transition, 1)
		caller := custodyDelegator
		if c.missingDelegation {
			caller = custodyOwner
		}
		from := common.BytesToAddress([]byte{c.from})
		to := common.BytesToAddress([]byte{c.to})
		input := append(nativeSimulationSelector("reDelegate(address,address,uint256)"), make([]byte, 12)...)
		input = append(input, from[:]...)
		input = append(input, make([]byte, 12)...)
		input = append(input, to[:]...)
		word := make([]byte, 32)
		big.NewInt(c.amount).FillBytes(word)
		input = append(input, word...)
		balances := func() map[string]string {
			result := map[string]string{}
			for _, account := range []common.Address{custodyDelegator, custodyOwner, *dpos.ContractAddress()} {
				result[hex.EncodeToString(account[:])] = transition.GetEvmState().GetAccount(&account).GetBalance().String()
			}
			return result
		}
		facts := func(backend contract_storage.StorageReader) map[string]any {
			address := *dpos.ContractAddress()
			wrapper := new(contract_storage.StorageWrapper)
			wrapper.StorageReaderWrapper.Init(&address, backend)
			delegations := new(dpos.Delegations)
			delegations.Init(wrapper, []byte{2})
			members := map[string][]string{}
			pairs := map[string]any{}
			for _, owner := range []common.Address{custodyDelegator, custodyOwner} {
				ownerHex := hex.EncodeToString(owner[:])
				list, _ := delegations.GetDelegatorValidatorsAddresses(&owner, 0, 20)
				members[ownerHex] = []string{}
				for _, validator := range list {
					members[ownerHex] = append(members[ownerHex], hex.EncodeToString(validator[:]))
				}
				for _, last := range []byte{0x31, 0x32} {
					validator := common.BytesToAddress([]byte{last})
					pair := ownerHex + "/" + hex.EncodeToString(validator[:])
					row := delegations.GetDelegation(&owner, &validator)
					if row == nil {
						pairs[pair] = nil
					} else {
						pairs[pair] = map[string]any{"stake": row.Stake.String(), "last_updated": uint64(row.LastUpdated)}
					}
				}
			}
			return map[string]any{"memberships": members, "delegations": pairs}
		}
		beforeFacts := facts(contract_storage.EVMStateStorage{EVMStateFace: transition.GetEvmState()})
		beforeNonce := transition.GetEvmState().GetAccount(&caller).GetNonce().String()
		beforeBalances := balances()
		// Seed inspection is separate from execution observation and native caches.
		seedRaw := []map[string]any{}
		seedBackend := contract_storage.EVMStateStorage{EVMStateFace: transition.GetEvmState()}
		for _, prefix := range [][]byte{{0, 5}, append([]byte{2, 1}, caller[:]...)} {
			keys := []*common.Hash{contract_storage.Stor_k_1(prefix, []byte{1})}
			for _, last := range []byte{0x31, 0x32} {
				member := common.BytesToAddress([]byte{last})
				keys = append(keys, contract_storage.Stor_k_1(prefix, []byte{2}, member[:]))
			}
			for _, position := range []byte{1, 2} {
				keys = append(keys, contract_storage.Stor_k_1(prefix, []byte{2}, []byte{position, 0, 0, 0}))
			}
			for _, key := range keys {
				present := false
				value := []byte{}
				contract := *dpos.ContractAddress()
				seedBackend.GetAccountStorage(&contract, key, func(v []byte) { present = true; value = common.CopyBytes(v) })
				seedRaw = append(seedRaw, map[string]any{"key": hex.EncodeToString(key[:]), "present": present, "value": hex.EncodeToString(value)})
			}
		}
		attempts := []map[string]any{}
		count := 1
		if c.repeat {
			count = 2
		}
		for nonce := 0; nonce < count; nonce++ {
			reads := []map[string]any{}
			contract_storage.SetRedelegateReadObserver(func(address common.Address, key common.Hash, value []byte, present bool) {
				reads = append(reads, map[string]any{"address": hex.EncodeToString(address[:]), "key": hex.EncodeToString(key[:]), "value": hex.EncodeToString(value), "present": present})
			})
			beginCustodyWrites()
			address := *dpos.ContractAddress()
			result := transition.ExecuteTransaction(&vm.Transaction{From: caller, To: &address,
				Nonce: big.NewInt(int64(nonce)), GasPrice: new(big.Int), Gas: c.gas, Value: big.NewInt(c.value), Input: input})
			writes := finishCustodyWrites()
			contract_storage.SetRedelegateReadObserver(nil)
			logs := []custodyLog{}
			for _, log := range result.Logs {
				topics := []string{}
				for _, topic := range log.Topics {
					topics = append(topics, hex.EncodeToString(topic[:]))
				}
				logs = append(logs, custodyLog{Address: hex.EncodeToString(log.Address[:]), Topics: topics, Data: hex.EncodeToString(log.Data)})
			}
			attempts = append(attempts, map[string]any{"nonce": nonce, "gas_used": result.GasUsed,
				"consensus_error": string(result.ConsensusErr), "execution_error": string(result.ExecutionErr),
				"output": hex.EncodeToString(result.CodeRetval), "logs": logs,
				"ordered_reads": reads, "ordered_raw_writes": writes, "balances": balances(), "caller_nonce": transition.GetEvmState().GetAccount(&caller).GetNonce().String()})
		}
		finishCustodyBlock(transition)
		factory := func(period types.BlockNum) contract_storage.StorageReader {
			return state_db.ExtendedReader{Reader: database.GetBlockStateReader(period)}
		}
		reader := new(dpos.API).Init(cfg).NewReader(1, factory)
		source, destination := from, to
		committedReader := state_db.ExtendedReader{Reader: database.GetBlockStateReader(1)}
		finalRaw := map[string]any{}
		keys := map[common.Hash]bool{}
		for _, read := range seedRaw {
			keys[common.HexToHash(read["key"].(string))] = true
		}
		for _, attempt := range attempts {
			for _, read := range attempt["ordered_reads"].([]map[string]any) {
				keys[common.HexToHash(read["key"].(string))] = true
			}
			for _, write := range attempt["ordered_raw_writes"].([]custodyRawWrite) {
				keys[common.HexToHash(write.Key)] = true
			}
		}
		for key := range keys {
			present := false
			value := []byte{}
			contract := *dpos.ContractAddress()
			committedReader.GetAccountStorage(&contract, &key, func(v []byte) { present = true; value = common.CopyBytes(v) })
			finalRaw[hex.EncodeToString(key[:])] = map[string]any{"present": present, "value": hex.EncodeToString(value)}
		}
		rows = append(rows, map[string]any{"name": c.name, "input": hex.EncodeToString(input), "caller": hex.EncodeToString(caller[:]),
			"seed_raw": seedRaw, "final_raw": finalRaw, "committed_rows": nativeSimulationSeedRows(database), "from": hex.EncodeToString(from[:]), "to": hex.EncodeToString(to[:]), "attempts": attempts, "source_stake": reader.GetStakingBalance(&source).String(),
			"destination_stake": reader.GetStakingBalance(&destination).String(), "total_delegated": reader.TotalAmountDelegated().String(), "balances_before": beforeBalances, "caller_nonce_before": beforeNonce, "facts_before": beforeFacts, "facts_after": facts(state_db.ExtendedReader{Reader: database.GetBlockStateReader(1)})})
		transition.Close()
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "action_gas": dpos.ReDelegateGas,
		"scope": "synthetic actual StateTransition read/write observations; no Rust routing or historical acceptance", "cases": rows}); err != nil {
		panic(err)
	}
}
