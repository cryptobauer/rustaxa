// Candidate probes are generated here, then checked by the unchanged C++ search
// body in the Python harness. Each probe executes actual pinned DryRunner.Apply.
package main

import (
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

func main() {
	var requests []nativeSimulationCase
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	seed := seedNativeSimulation()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := nativeSimulationSnapshot(seed, api)
	cases := []map[string]any{}
	for _, request := range requests {
		input, err := hex.DecodeString(request.Input)
		if err != nil {
			panic(err)
		}
		to := common.HexToAddress(request.To)
		parse := func(s string) *big.Int {
			n, ok := new(big.Int).SetString(s, 10)
			if !ok {
				panic("bad integer")
			}
			return n
		}
		probes := []nativeSimulationCase{}
		probe := func(gas uint64) nativeSimulationOutput {
			transaction := vm.Transaction{From: nativeSimulationSender, To: &to, Nonce: parse(request.SuppliedNonce), GasPrice: parse(request.GasPrice), Gas: gas, Value: parse(request.Value), Input: input}
			first := runNativeSimulationCaptured(runner, block, request.Name, transaction)
			transaction.Nonce = parse(request.SuppliedNonce)
			second := runNativeSimulationCaptured(runner, block, request.Name, transaction)
			if !reflect.DeepEqual(first, second) {
				panic("probe isolation failed")
			}
			probes = append(probes, first)
			return first.Output
		}
		initial := probe(request.Gas)
		if initial.ConsensusError == "" && initial.ExecutionError == "" {
			low, high := initial.GasUsed, request.Gas
			if low > high {
				panic("invalid actual gas")
			}
			for high-low > high/20 {
				mid := low + (high-low)/2
				result := probe(mid)
				if result.ConsensusError != "" {
					break
				}
				if result.ExecutionError == "" {
					high = mid
				} else {
					low = mid
				}
			}
		}
		cases = append(cases, map[string]any{"name": request.Name, "cap": request.Gas, "probes": probes})
	}
	after := nativeSimulationSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("estimation changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": cases, "state_before": before, "state_after": after}); err != nil {
		panic(err)
	}
}
