// Actual escrow DryRunner calls over the unchanged complete synthetic seed.
package main

import (
	"encoding/json"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

func main() {
	seed := seedNativeSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := nativeSimulationSnapshot(seed, api)
	address := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []nativeSimulationCase{}
	for _, definition := range []struct {
		name  string
		value int64
		gas   uint64
	}{
		{name: "zero", gas: 100000},
		{name: "one", value: 1, gas: 100000},
		{name: "forty_two", value: 42, gas: 100000},
		{name: "insufficient_native_gas", value: 1, gas: 22063},
		{name: "intrinsic_gas", value: 1, gas: 21063},
	} {
		transaction := vm.Transaction{From: nativeSimulationSender, To: &address,
			Nonce: new(big.Int).Set(wide), GasPrice: big.NewInt(1), Gas: definition.gas,
			Value: big.NewInt(definition.value), Input: []byte{0x44, 0xdf, 0x8e, 0x70}}
		first := runNativeSimulationCaptured(runner, block, definition.name, transaction)
		transaction.Nonce = new(big.Int).Set(wide)
		second := runNativeSimulationCaptured(runner, block, definition.name, transaction)
		if !reflect.DeepEqual(first, second) {
			panic("escrow probe changed on repetition")
		}
		cases = append(cases, first)
	}
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("escrow simulation changed committed state")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before,
		"state_after": after, "committed_state_unchanged": true, "repeat_identical": true,
		"cases": cases, "scope": "actual active escrow DryRunner.Apply over synthetic persisted H=1; no production or historical acceptance"}); err != nil {
		panic(err)
	}
}
