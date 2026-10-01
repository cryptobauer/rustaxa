// Actual metadata DryRunner calls over the existing complete synthetic seed.
// Shared exporters are copied unchanged except for their unused main names.
package main

import (
	"bytes"
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
	definitions := []struct {
		name                                                    string
		description, endpoint                                   []byte
		missing, malformed, nonpayable, insufficient, intrinsic bool
	}{
		{name: "replace", description: []byte{0xff, 0, 0x61}, endpoint: []byte("endpoint")},
		{name: "empty"},
		{name: "maximum", description: bytes.Repeat([]byte{0x64}, 100), endpoint: bytes.Repeat([]byte{0x65}, 50)},
		{name: "both_too_long", description: bytes.Repeat([]byte{0x64}, 101), endpoint: bytes.Repeat([]byte{0x65}, 51)},
		{name: "missing_validator", missing: true},
		{name: "malformed", malformed: true},
		{name: "nonpayable_malformed", malformed: true, nonpayable: true},
		{name: "insufficient_gas", insufficient: true},
		{name: "intrinsic_gas", intrinsic: true},
	}
	for _, definition := range definitions {
		validator := nativeSimulationValidator
		if definition.missing {
			validator = nativeSimulationMissing
		}
		input := infoABI(validator, definition.description, definition.endpoint)
		if definition.malformed {
			input = input[:4]
		}
		gas := uint64(100000)
		if definition.insufficient {
			gas = 30000
		}
		if definition.intrinsic {
			gas = 22000
		}
		value := new(big.Int)
		if definition.nonpayable {
			value.SetInt64(1)
		}
		transaction := vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Set(wide), GasPrice: big.NewInt(1), Gas: gas, Value: value, Input: input}
		first := runNativeSimulationCaptured(runner, block, definition.name, transaction)
		// Apply mutates the transaction nonce, so restore the supplied request.
		transaction.Nonce = new(big.Int).Set(wide)
		second := runNativeSimulationCaptured(runner, block, definition.name, transaction)
		if !reflect.DeepEqual(first, second) {
			panic("metadata probe changed on repetition")
		}
		cases = append(cases, first)
	}
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("metadata simulation changed committed state")
	}
	result := map[string]any{"schema": 1, "state_before": before, "state_after": after, "committed_state_unchanged": true, "repeat_identical": true, "cases": cases, "scope": "actual metadata DryRunner.Apply over synthetic persisted H=1; no RPC, trace or production routing"}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
