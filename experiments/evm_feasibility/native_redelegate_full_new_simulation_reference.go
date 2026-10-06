// One complete H1 full-source/new-destination DryRunner case; shared owners unchanged.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"math/big"
	"os"
	"reflect"
)

func seedRedelegateFullNewSimulation() nativeSimulationSeed {
	database := newNativeSimulationDB()
	config := nativeSimulationConfig()
	config.DPOS.MinimumDeposit = big.NewInt(100)
	config.GenesisBalances = core.BalanceMap{nativeSimulationSender: big.NewInt(4000), nativeSimulationDelegator: big.NewInt(2000)}
	config.Hardforks.AspenHf.MaxSupply = big.NewInt(6000)
	config.DPOS.InitialValidators = []chain_config.GenesisValidator{}
	for _, last := range []byte{0x31, 0x32} {
		vrf := byte(0x44)
		if last == 0x32 {
			vrf = 0x55
		}
		config.DPOS.InitialValidators = append(config.DPOS.InitialValidators, chain_config.GenesisValidator{
			Address: common.BytesToAddress([]byte{last}), Owner: nativeSimulationDelegator, VrfKey: bytes.Repeat([]byte{vrf}, 32), Commission: 100,
			Delegations: core.BalanceMap{nativeSimulationDelegator: big.NewInt(1000)},
		})
	}
	config.DPOS.InitialValidators[0].Delegations[nativeSimulationSender] = big.NewInt(1000)
	transition := nativeSimulationTransition(database, &config)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)})
	transition.GetEvmState().GetAccount(&nativeSimulationSender).SetNonce(new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(5)))
	transition.EndBlock()
	transition.Commit()
	if database.descriptor.BlockNum != nativeSimulationPeriod {
		panic("redelegation seed is not H=1")
	}
	return nativeSimulationSeed{database: database, config: config, wrapper: nativeSimulationDelegator}
}

func fullNewSimulationFacts(backend contract_storage.StorageReader) map[string]any {
	address := *dpos.ContractAddress()
	wrapper := new(contract_storage.StorageWrapper)
	wrapper.StorageReaderWrapper.Init(&address, backend)
	delegations := new(dpos.Delegations)
	delegations.Init(wrapper, []byte{2})
	members := map[string][]string{}
	pairs := map[string]any{}
	for _, owner := range []common.Address{nativeSimulationSender, nativeSimulationDelegator} {
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

func fullNewSimulationSnapshot(seed nativeSimulationSeed, api *dpos.API) map[string]any {
	result := nativeSimulationSnapshot(seed, api)
	result["configuration"] = seed.config
	backend := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(nativeSimulationPeriod)}
	result["native_facts"] = fullNewSimulationFacts(backend)
	current := []map[string]any{}
	for _, last := range []byte{0x31, 0x32} {
		validator := common.BytesToAddress([]byte{last})
		key := contract_storage.Stor_k_1([]byte{1}, validator[:], []byte{1})
		present := false
		value := []byte{}
		contract := *dpos.ContractAddress()
		backend.GetAccountStorage(&contract, key, func(v []byte) { present = true; value = common.CopyBytes(v) })
		current = append(current, map[string]any{"validator": hex.EncodeToString(validator[:]), "block": 1, "key": hex.EncodeToString(key[:]), "present": present, "value": hex.EncodeToString(value)})
		if present {
			panic("actual H1 current reward node exists; bounded full-new history blocked")
		}
	}
	result["current_reward_nodes"] = current
	return result
}
func main() {
	nativeSimulationSender = common.BytesToAddress([]byte{0xd1})
	nativeSimulationDelegator = common.BytesToAddress([]byte{0xa1})
	seed := seedRedelegateFullNewSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := fullNewSimulationSnapshot(seed, api)
	contract := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []nativeSimulationCase{}
	tx := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: redelegateSimulationABI(nativeSimulationValidator, common.BytesToAddress([]byte{0x32}), 1000)}
	first := runNativeSimulationCaptured(runner, block, "full_new", tx)
	tx.Nonce = new(big.Int).Set(wide)
	second := runNativeSimulationCaptured(runner, block, "full_new", tx)
	if !reflect.DeepEqual(first, second) {
		panic("full-new redelegation probe differs on repeat")
	}
	cases = append(cases, first)
	after := fullNewSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("redelegation simulation changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "repeat_identical": true, "committed_state_unchanged": true, "cases": cases, "scope": "actual full-source new-destination redelegation DryRunner.Apply at complete synthetic H=1; one-member caller, retained validators, zero rewards, pre-Aspen-two; no production acceptance"}); err != nil {
		panic(err)
	}
}
