// Observe raw StructLogger entries from the actual pinned TraceRunner.
package main

import (
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"math/big"
	"os"
	"reflect"
)

func main() {
	config := &chain_config.ChainConfig{EVMChainConfig: params.TestChainConfig}
	config.Hardforks.MagnoliaHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartOne = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartTwo = types.BlockNumberNIL
	config.Hardforks.FicusHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.CactiHf.BlockNum = types.BlockNumberNIL
	block := &vm.Block{Number: apiPeriod + 1, BlockInfo: vm.BlockInfo{GasLimit: 1_000_000, Difficulty: big.NewInt(0)}}
	rows := []any{}
	for _, item := range []struct{ name, code string }{
		{"clear", "600060005500"},
		{"clear_restore", "6000600055600760005500"},
		{"clear_revert", "600060005560006000fd"},
		{"clear_restore_clear", "60006000556007600055600060005500"},
	} {
		memory := apiSeedWithCode(common.FromHex(item.code))
		before := apiObserve(memory)
		runner := new(state_dry_runner.TraceRunner).Init(memory, func(types.BlockNum) *big.Int { return new(big.Int) }, nil, nil, config)
		nonce := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(6))
		targets := []vm.Transaction{{From: apiSender, To: &apiTarget, Nonce: nonce, GasPrice: big.NewInt(2), Gas: 100_000, Value: big.NewInt(3)}}
		prefix := []vm.Transaction{}
		result := runner.Trace(block, &prefix, &targets, nil)
		after := apiObserve(memory)
		if !reflect.DeepEqual(before, after) {
			panic("trace mutated committed state")
		}
		rows = append(rows, map[string]any{"name": item.name, "code": item.code, "state_before": before, "state_after": after, "result": json.RawMessage(result)})
	}
	rows = append(rows, sequenceRefundCases(config, block)...)
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}

// Keep the public TraceRunner sequence lifetime; never commit/reset between calls.
func sequenceRefundCases(config *chain_config.ChainConfig, block *vm.Block) []any {
	rows := []any{}
	word := func(value byte) []byte { out := make([]byte, 32); out[31] = value; return out }
	for _, item := range []struct {
		name, code      string
		prefix, targets [][]byte
	}{
		{"prefix_clear_restore_set", "60003560005500", [][]byte{word(0)}, [][]byte{word(7), word(8)}},
		{"two_targets_clear_restore", "60003560005500", nil, [][]byte{word(0), word(7)}},
		{"prefix_transient_write_read", "3615600c576000356000b4005b6000b360005260206000f3", [][]byte{word(9)}, [][]byte{nil}},
	} {
		memory := apiSeedWithCode(common.FromHex(item.code))
		before := apiObserve(memory)
		runner := new(state_dry_runner.TraceRunner).Init(memory, func(types.BlockNum) *big.Int { return new(big.Int) }, nil, nil, config)
		nonce := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(6))
		prefix, targets := []vm.Transaction{}, []vm.Transaction{}
		next := func(input []byte) vm.Transaction {
			tx := vm.Transaction{From: apiSender, To: &apiTarget, Nonce: new(big.Int).Set(nonce), GasPrice: big.NewInt(2), Gas: 100_000, Value: big.NewInt(3), Input: input}
			nonce.Add(nonce, big.NewInt(1))
			return tx
		}
		for _, input := range item.prefix {
			prefix = append(prefix, next(input))
		}
		for _, input := range item.targets {
			targets = append(targets, next(input))
		}
		result := runner.Trace(block, &prefix, &targets, nil)
		after := apiObserve(memory)
		if !reflect.DeepEqual(before, after) {
			panic("sequence trace mutated committed state")
		}
		prefixInputs, targetInputs, prefixNonces, targetNonces := []string{}, []string{}, []string{}, []string{}
		for _, tx := range prefix {
			prefixInputs = append(prefixInputs, apiHex(tx.Input))
			prefixNonces = append(prefixNonces, tx.Nonce.String())
		}
		for _, tx := range targets {
			targetInputs = append(targetInputs, apiHex(tx.Input))
			targetNonces = append(targetNonces, tx.Nonce.String())
		}
		rows = append(rows, map[string]any{"name": item.name, "code": item.code, "prefix": prefix, "targets": targets,
			"prefix_inputs": prefixInputs, "target_inputs": targetInputs, "prefix_nonces": prefixNonces, "target_nonces": targetNonces,
			"state_before": before, "state_after": after, "result": json.RawMessage(result)})
	}
	return rows
}
