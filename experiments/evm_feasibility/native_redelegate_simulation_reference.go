// Actual redelegation DryRunner.Apply over a complete synthetic two-validator H=1.
package main

import (
	"bytes"
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

func seedRedelegateSimulation() nativeSimulationSeed {
	database := newNativeSimulationDB()
	config := nativeSimulationConfig()
	config.DPOS.MinimumDeposit = big.NewInt(100)
	config.DPOS.InitialValidators = []chain_config.GenesisValidator{}
	for _, last := range []byte{0x31, 0x32} {
		vrf := byte(0x44)
		if last == 0x32 {
			vrf = 0x55
		}
		config.DPOS.InitialValidators = append(config.DPOS.InitialValidators, chain_config.GenesisValidator{
			Address: common.BytesToAddress([]byte{last}), Owner: nativeSimulationSender, VrfKey: bytes.Repeat([]byte{vrf}, 32), Commission: 100,
			Delegations: core.BalanceMap{nativeSimulationSender: big.NewInt(1000)},
		})
	}
	transition := nativeSimulationTransition(database, &config)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Difficulty: new(big.Int)})
	transition.GetEvmState().GetAccount(&nativeSimulationSender).SetNonce(new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(5)))
	transition.EndBlock()
	transition.Commit()
	if database.descriptor.BlockNum != nativeSimulationPeriod {
		panic("redelegation seed is not H=1")
	}
	return nativeSimulationSeed{database: database, config: config, wrapper: nativeSimulationDelegator}
}
func redelegateSimulationABI(from, to common.Address, amount int64) []byte {
	input := append(nativeSimulationSelector("reDelegate(address,address,uint256)"), make([]byte, 12)...)
	input = append(input, from[:]...)
	input = append(input, make([]byte, 12)...)
	input = append(input, to[:]...)
	word := make([]byte, 32)
	big.NewInt(amount).FillBytes(word)
	return append(input, word...)
}
func main() {
	seed := seedRedelegateSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := nativeSimulationSnapshot(seed, api)
	contract := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []nativeSimulationCase{}
	for _, c := range []struct {
		name                  string
		from, to              byte
		amount                int64
		gas                   uint64
		malformed, nonpayable bool
	}{
		{name: "partial", from: 0x31, to: 0x32, amount: 300, gas: 200000},
		{name: "missing_source", from: 0x99, to: 0x32, amount: 300, gas: 200000},
		{name: "missing_destination", from: 0x31, to: 0x99, amount: 300, gas: 200000},
		{name: "insufficient_source", from: 0x31, to: 0x32, amount: 1100, gas: 200000},
		{name: "remainder_below_minimum", from: 0x31, to: 0x32, amount: 950, gas: 200000},
		{name: "same_validator", from: 0x31, to: 0x31, amount: 300, gas: 200000},
		{name: "malformed", from: 0x31, to: 0x32, gas: 200000, malformed: true},
		{name: "nonpayable_malformed", from: 0x31, to: 0x32, gas: 200000, malformed: true, nonpayable: true},
		{name: "insufficient_native_gas", from: 0x31, to: 0x32, amount: 300, gas: 30000},
		{name: "intrinsic_gas", from: 0x31, to: 0x32, amount: 300, gas: 21000},
	} {
		input := redelegateSimulationABI(common.BytesToAddress([]byte{c.from}), common.BytesToAddress([]byte{c.to}), c.amount)
		if c.malformed {
			input = input[:4]
		}
		value := new(big.Int)
		if c.nonpayable {
			value.SetInt64(1)
		}
		tx := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: big.NewInt(1), Gas: c.gas, Value: value, Input: input}
		first := runNativeSimulationCaptured(runner, block, c.name, tx)
		tx.Nonce = new(big.Int).Set(wide)
		second := runNativeSimulationCaptured(runner, block, c.name, tx)
		if !reflect.DeepEqual(first, second) {
			panic("redelegation probe differs on repeat")
		}
		cases = append(cases, first)
	}
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("redelegation simulation changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "repeat_identical": true, "committed_state_unchanged": true, "cases": cases, "scope": "actual redelegation DryRunner.Apply at synthetic persisted H=1, zero rewards existing delegations; no production acceptance"}); err != nil {
		panic(err)
	}
}
