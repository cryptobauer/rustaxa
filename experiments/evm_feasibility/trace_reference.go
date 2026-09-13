// Runs the actual pinned TraceRunner over the API oracle's committed trie.
// The companion exporter reuses api_reference.go without invoking its main.
package main

import (
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

// Keep a reference panic distinct from a successful JSON result. The wrapper
// observes the unmodified public entrypoint; it does not repair the Go tracer.
func traceObserved(runner *state_dry_runner.TraceRunner, block *vm.Block, prefix, targets *[]vm.Transaction, cfg *vm.TracingConfig) (out map[string]any) {
	out = map[string]any{}
	defer func() {
		if p := recover(); p != nil {
			out["panic"] = fmt.Sprint(p)
		}
	}()
	out["result"] = json.RawMessage(runner.Trace(block, prefix, targets, cfg))
	return
}

func main() {
	memory := apiSeed()
	before := apiObserve(memory)
	config := &chain_config.ChainConfig{EVMChainConfig: params.TestChainConfig}
	config.Hardforks.MagnoliaHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartOne = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartTwo = types.BlockNumberNIL
	config.Hardforks.FicusHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.CactiHf.BlockNum = types.BlockNumberNIL
	block := &vm.Block{Number: apiPeriod + 1, BlockInfo: vm.BlockInfo{GasLimit: 1_000_000, Difficulty: big.NewInt(0)}}
	runner := new(state_dry_runner.TraceRunner).Init(memory, func(types.BlockNum) *big.Int { return new(big.Int) }, nil, nil, config)
	nonce := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(6))
	tx := func(target *common.Address, offset int64) vm.Transaction {
		return vm.Transaction{From: apiSender, To: target, Nonce: new(big.Int).Add(nonce, big.NewInt(offset)), GasPrice: big.NewInt(2), Gas: 100_000, Value: big.NewInt(3)}
	}
	type scenario struct {
		name            string
		prefix, targets []vm.Transaction
	}
	stale := tx(&apiTarget, 0)
	stale.Nonce = big.NewInt(0)
	create := tx(nil, 0)
	create.Input = apiInitCode()
	scenarios := []scenario{
		{"storage_call", nil, []vm.Transaction{tx(&apiTarget, 0)}},
		{"prefix_then_two_calls", []vm.Transaction{tx(&apiTarget, 0)}, []vm.Transaction{tx(&apiTarget, 1), tx(&apiTarget, 2)}},
		{"stale_nonce_is_preserved", nil, []vm.Transaction{stale}},
		{"revert", nil, []vm.Transaction{tx(&apiRevert, 0)}},
		{"create", nil, []vm.Transaction{create}},
		{"empty_code", nil, []vm.Transaction{tx(&apiEmpty, 0)}},
		{"return_bounds", nil, []vm.Transaction{tx(&apiReturnBounds, 0)}},
	}
	rows := []any{}
	for _, item := range scenarios {
		outputs := map[string]any{}
		for _, mode := range []string{"struct", "trace", "vmTrace", "both", "neither"} {
			var cfg *vm.TracingConfig
			if mode != "struct" {
				cfg = &vm.TracingConfig{Trace: mode == "trace" || mode == "both", VmTrace: mode == "vmTrace" || mode == "both"}
			}
			outputs[mode] = traceObserved(runner, block, &item.prefix, &item.targets, cfg)
		}
		rows = append(rows, map[string]any{"name": item.name, "prefix": item.prefix, "targets": item.targets, "outputs": outputs})
	}
	after := apiObserve(memory)
	if !reflect.DeepEqual(before, after) {
		panic("TraceRunner mutated committed state")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{
		"schema": 1, "state_before": before, "state_after": after, "committed_state_unchanged": true,
		"block": block, "cases": rows,
		"scope": "actual TraceRunner; ordinary synthetic calls, explicit supplied nonce, preceding-block state, ordered prefix and targets; no native registry or RPC routing",
	}); err != nil {
		panic(err)
	}
}
