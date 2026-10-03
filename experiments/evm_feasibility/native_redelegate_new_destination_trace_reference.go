// Actual default structured TraceRunner over the complete synthetic native seed.
package main

import (
	"bytes"
	"encoding/hex"
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

func redelegateTrace(runner *state_dry_runner.TraceRunner, block *vm.Block, prefix, targets []vm.Transaction) ([]byte, string) {
	read, write, err := os.Pipe()
	if err != nil {
		panic(err)
	}
	stdout := os.Stdout
	os.Stdout = write
	result := runner.Trace(block, &prefix, &targets, nil)
	os.Stdout = stdout
	if err := write.Close(); err != nil {
		panic(err)
	}
	printed := new(bytes.Buffer)
	if _, err := printed.ReadFrom(read); err != nil {
		panic(err)
	}
	if err := read.Close(); err != nil {
		panic(err)
	}
	return result, printed.String()
}

func traceRequest(transaction vm.Transaction) map[string]any {
	return map[string]any{"sender": hex.EncodeToString(transaction.From[:]), "to": hex.EncodeToString(transaction.To[:]), "nonce": transaction.Nonce.String(), "gas_price": transaction.GasPrice.String(), "gas": transaction.Gas, "value": transaction.Value.String(), "input": hex.EncodeToString(transaction.Input)}
}

func main() {
	seed := seedRedelegateNewDestinationSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod + 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)}}
	before := nativeSimulationSnapshot(seed, api)
	address := *dpos.ContractAddress()
	tx := func(input []byte) vm.Transaction {
		return vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Lsh(big.NewInt(1), 512), GasPrice: big.NewInt(1), Gas: 200000, Value: new(big.Int), Input: input}
	}
	first := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 300))
	second := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 200))
	remainder := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 650))
	missing := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x99}), 300))
	low := tx(first.Input)
	low.Gas = 30000
	nonpayable := tx(first.Input[:4])
	nonpayable.Value = big.NewInt(1)
	below := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 50))
	stale := tx(first.Input)
	stale.Nonce = new(big.Int)
	definitions := []struct {
		name            string
		prefix, targets []vm.Transaction
	}{
		{name: "single_partial", targets: []vm.Transaction{first}},
		{name: "prefix_partial_then_partial", prefix: []vm.Transaction{first}, targets: []vm.Transaction{second}},
		{name: "two_partials", targets: []vm.Transaction{first, second}},
		{name: "prefix_partial_then_remainder_failure", prefix: []vm.Transaction{first}, targets: []vm.Transaction{remainder}},
		{name: "missing_destination_then_partial", targets: []vm.Transaction{missing, first}},
		{name: "low_gas_then_partial", targets: []vm.Transaction{low, first}},
		{name: "nonpayable_malformed_then_partial", targets: []vm.Transaction{nonpayable, first}},
		{name: "stale_nonce_preserved", targets: []vm.Transaction{stale}},
		{name: "single_below_minimum", targets: []vm.Transaction{below}},
	}
	cases := []map[string]any{}
	for _, definition := range definitions {
		if definition.name != "stale_nonce_preserved" {
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
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("trace changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "period": uint64(block.Number), "cases": cases, "scope": "actual direct new-destination redelegation default structured TraceRunner, live retained prefix/target state; no delayed eligibility, nested target, OE or RPC claim"}); err != nil {
		panic(err)
	}
}
