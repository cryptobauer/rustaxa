// One settled zero-success DryRunner case; seed/ABI owners are unchanged.
package main

import (
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"math/big"
	"os"
	"reflect"
)

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
	tx := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: big.NewInt(1), Gas: 200000, Value: new(big.Int), Input: redelegateSimulationABI(nativeSimulationValidator, common.BytesToAddress([]byte{0x32}), 0)}
	first := runNativeSimulationCaptured(runner, block, "zero_before_aspen_two", tx)
	tx.Nonce = new(big.Int).Set(wide)
	second := runNativeSimulationCaptured(runner, block, "zero_before_aspen_two", tx)
	if !reflect.DeepEqual(first, second) {
		panic("zero redelegation probe differs on repeat")
	}
	cases = append(cases, first)
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("redelegation simulation changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "repeat_identical": true, "committed_state_unchanged": true, "cases": cases, "scope": "actual zero-amount redelegation DryRunner.Apply at complete synthetic H=1; positive caller pairs, zero rewards, pre-Aspen-two; no production acceptance"}); err != nil {
		panic(err)
	}
}
