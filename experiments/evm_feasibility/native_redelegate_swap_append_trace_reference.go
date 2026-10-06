// Actual source-first two-member swap-append success direct TraceRunner; shared seed and capture owners unchanged.
package main

import (
	"bytes"
	"encoding/json"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

func main() {
	nativeSimulationSender = common.BytesToAddress([]byte{0xd1})
	nativeSimulationDelegator = common.BytesToAddress([]byte{0xa1})
	seed := seedRedelegateSwapAppendSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod + 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)}}
	before := swapAppendSimulationSnapshot(seed, api)
	address := *dpos.ContractAddress()
	tx := func(input []byte) vm.Transaction {
		return vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Lsh(big.NewInt(1), 512), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: input}
	}
	zero := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 1000))
	stale := tx(zero.Input)
	stale.Nonce = new(big.Int)
	definitions := []struct {
		name            string
		prefix, targets []vm.Transaction
	}{
		{name: "single_swap_append", targets: []vm.Transaction{zero}},
		{name: "swap_append_then_missing_source", prefix: []vm.Transaction{zero}, targets: []vm.Transaction{zero}},
		{name: "stale_nonce_swap_append", targets: []vm.Transaction{stale}},
	}
	cases := []map[string]any{}
	for _, definition := range definitions {
		if definition.name != "stale_nonce_swap_append" {
			base := new(big.Int).Lsh(big.NewInt(1), 512)
			index := int64(0)
			for position := range definition.prefix {
				definition.prefix[position].Nonce = new(big.Int).Add(base, big.NewInt(index))
				index++
			}
			for position := range definition.targets {
				definition.targets[position].Nonce = new(big.Int).Add(base, big.NewInt(index))
				index++
			}
		}
		result, diagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		repeated, repeatedDiagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		if !bytes.Equal(result, repeated) || diagnostics != repeatedDiagnostics {
			panic("trace did not repeat")
		}
		prefix, targets := []map[string]any{}, []map[string]any{}
		for _, transaction := range definition.prefix {
			prefix = append(prefix, traceRequest(transaction))
		}
		for _, transaction := range definition.targets {
			targets = append(targets, traceRequest(transaction))
		}
		cases = append(cases, map[string]any{"name": definition.name, "prefix": prefix, "targets": targets, "result": json.RawMessage(result), "diagnostics": diagnostics})
	}
	after := swapAppendSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("trace changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "period": uint64(block.Number), "cases": cases, "scope": "actual direct redelegation default structured TraceRunner, live retained prefix/target state; no delayed eligibility, nested target, OE or RPC claim"}); err != nil {
		panic(err)
	}
}
