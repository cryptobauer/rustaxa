// Actual retained P2 DryRunner probes. Every invocation reopens committed state.
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
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

func main() {
	var baseline map[string]json.RawMessage
	if err := json.NewDecoder(os.Stdin).Decode(&baseline); err != nil {
		panic(err)
	}
	var requests []nativeSimulationCase
	if err := json.Unmarshal(baseline["simulation"], &requests); err != nil {
		panic(err)
	}
	if len(requests) != 2 || requests[0].Name != "positive" || requests[1].Name != "wrong_owner" {
		panic("missing cases")
	}
	seed := estimateRetainedSeed()
	before := retainedSnapshot(seed)
	var expected map[string]any
	if err := json.Unmarshal(baseline["state_before"], &expected); err != nil {
		panic(err)
	}
	// Compare JSON numbers without Go integer interface type differences.
	encoded, _ := json.Marshal(before)
	var normalized map[string]any
	if err := json.Unmarshal(encoded, &normalized); err != nil {
		panic(err)
	}
	if !reflect.DeepEqual(expected, normalized) {
		panic("retained P2 identity differs")
	}
	probe := func(request nativeSimulationCase, gas uint64) nativeSimulationCase {
		var first nativeSimulationCase
		for repeat := 0; repeat < 2; repeat++ {
			api := new(dpos.API).Init(seed.config)
			factory := func(period types.BlockNum) storage.StorageReader {
				return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
			}
			runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
			block := &vm.Block{Number: 2, BlockInfo: vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000002, Difficulty: new(big.Int)}}
			parse := func(s string) *big.Int {
				n, ok := new(big.Int).SetString(s, 10)
				if !ok {
					panic("bad integer")
				}
				return n
			}
			to := common.HexToAddress(request.To)
			sender := custodyOwner
			if request.Name == "wrong_owner" {
				sender = custodyDelegator
			}
			input, err := hex.DecodeString(request.Input)
			if err != nil {
				panic(err)
			}
			tx := vm.Transaction{From: sender, To: &to, Nonce: parse(request.SuppliedNonce), GasPrice: parse(request.GasPrice), Gas: gas, Value: parse(request.Value), Input: input}
			actual := runNativeSimulationCaptured(runner, block, request.Name, tx)
			if !reflect.DeepEqual(before, retainedSnapshot(seed)) {
				panic("probe changed committed P2")
			}
			if repeat == 0 {
				first = actual
			} else if !reflect.DeepEqual(first, actual) {
				panic("fresh repeat differs")
			}
		}
		return first
	}
	// Independently measure the first successful cap using actual executions.
	boundary := []nativeSimulationCase{}
	low, high := uint64(0), requests[0].Gas
	initial := probe(requests[0], high)
	if initial.Output.ExecutionError != "" || initial.Output.ConsensusError != "" {
		panic("invalid upper cap")
	}
	boundary = append(boundary, initial)
	for high-low > 1 {
		mid := low + (high-low)/2
		p := probe(requests[0], mid)
		boundary = append(boundary, p)
		if p.Output.ExecutionError == "" && p.Output.ConsensusError == "" {
			high = mid
		} else {
			low = mid
		}
	}
	valid := requests[0]
	minimum := valid
	minimum.Name = "minimum"
	minimum.Gas = high
	below := valid
	below.Name = "below_minimum"
	below.Gas = high - 1
	cases := []map[string]any{}
	for _, request := range []nativeSimulationCase{valid, minimum, below, requests[1]} {
		probes := []nativeSimulationCase{}
		run := func(gas uint64) nativeSimulationOutput {
			p := probe(request, gas)
			probes = append(probes, p)
			return p.Output
		}
		p := run(request.Gas)
		if p.ExecutionError == "" && p.ConsensusError == "" {
			lo, hi := p.GasUsed, request.Gas
			if lo > hi {
				panic("gas exceeds cap")
			}
			for hi-lo > hi/20 {
				mid := lo + (hi-lo)/2
				p = run(mid)
				if p.ConsensusError != "" {
					break
				}
				if p.ExecutionError == "" {
					hi = mid
				} else {
					if mid == lo {
						panic("nonprogress")
					}
					lo = mid
				}
			}
		}
		cases = append(cases, map[string]any{"name": request.Name, "cap": request.Gas, "probes": probes})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "minimum": high, "boundary_probes": boundary, "repeat_count": 2, "cases": cases, "state_before": before, "state_after": retainedSnapshot(seed), "configuration": seed.config}); err != nil {
		panic(err)
	}
}
