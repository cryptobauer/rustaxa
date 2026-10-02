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

func redelegateConfig() chain_config.ChainConfig {
	cfg := custodyConfig(true, true, false)
	cfg.GenesisBalances[custodyDelegator] = big.NewInt(4000)
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(5000)
	cfg.DPOS.MinimumDeposit = big.NewInt(100)
	cfg.DPOS.InitialValidators = append(cfg.DPOS.InitialValidators, chain_config.GenesisValidator{
		Address: common.BytesToAddress([]byte{0x32}), Owner: custodyOwner,
		VrfKey: bytes.Repeat([]byte{0x55}, 32), Commission: 100,
		Delegations: core.BalanceMap{custodyDelegator: big.NewInt(1000)},
	})
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
		{name: "partial_and_repeat", from: 0x31, to: 0x32, amount: 300, gas: 200000, repeat: true},
		{name: "destination_cap_before_insufficient_source", from: 0x31, to: 0x32, amount: 1100, maximum: 1500, gas: 200000},
		{name: "missing_source", from: 0x99, to: 0x32, amount: 300, gas: 200000},
		{name: "missing_destination", from: 0x31, to: 0x99, amount: 300, gas: 200000},
		{name: "missing_source_delegation", from: 0x31, to: 0x32, amount: 300, gas: 200000, missingDelegation: true},
		{name: "insufficient_source", from: 0x31, to: 0x32, amount: 1100, gas: 200000},
		{name: "remainder_below_minimum", from: 0x31, to: 0x32, amount: 950, gas: 200000},
		{name: "same_validator", from: 0x31, to: 0x31, amount: 300, gas: 200000},
		{name: "zero_before_aspen_two", from: 0x31, to: 0x32, gas: 200000},
		{name: "full_source", from: 0x31, to: 0x32, amount: 1000, gas: 200000},
		{name: "insufficient_native_gas", from: 0x31, to: 0x32, amount: 300, gas: 30000},
		{name: "nonpayable", from: 0x31, to: 0x32, amount: 300, value: 1, gas: 200000},
	} {
		cfg := redelegateConfig()
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
				"ordered_reads": reads, "ordered_raw_writes": writes})
		}
		finishCustodyBlock(transition)
		factory := func(period types.BlockNum) contract_storage.StorageReader {
			return state_db.ExtendedReader{Reader: database.GetBlockStateReader(period)}
		}
		reader := new(dpos.API).Init(cfg).NewReader(1, factory)
		source, destination := custodyValidator, common.BytesToAddress([]byte{0x32})
		rows = append(rows, map[string]any{"name": c.name, "input": hex.EncodeToString(input), "caller": hex.EncodeToString(caller[:]),
			"attempts": attempts, "source_stake": reader.GetStakingBalance(&source).String(),
			"destination_stake": reader.GetStakingBalance(&destination).String(), "total_delegated": reader.TotalAmountDelegated().String()})
		transition.Close()
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "action_gas": dpos.ReDelegateGas,
		"scope": "synthetic actual StateTransition read/write observations; no Rust routing or historical acceptance", "cases": rows}); err != nil {
		panic(err)
	}
}
