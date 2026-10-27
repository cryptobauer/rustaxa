// Actual default TraceRunner sequences over the unchanged complete retained P2 seed.
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
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

func retainedTraceRequest(tx vm.Transaction) map[string]any {
	return map[string]any{"sender": hex.EncodeToString(tx.From[:]), "to": hex.EncodeToString(tx.To[:]), "nonce": tx.Nonce.String(), "gas_price": tx.GasPrice.String(), "gas": tx.Gas, "value": tx.Value.String(), "input": hex.EncodeToString(tx.Input)}
}

// Read-only snapshots of the live private journal. Copy all returned values.
func retainedTraceState(state *state_evm.BlockState) map[string]any {
	raw := map[string]string{}
	for key := range retainedKeys {
		state.GetAccountConcrete(dpos.ContractAddress()).GetRawState(&key, func(b []byte) {
			raw[hex.EncodeToString(key[:])] = hex.EncodeToString(b)
		})
	}
	accounts := map[string]any{}
	for _, a := range []common.Address{custodyOwner, custodyDelegator, *dpos.ContractAddress()} {
		acc := state.GetAccountConcrete(&a)
		accounts[hex.EncodeToString(a[:])] = map[string]any{"exists": !acc.IsNIL(), "balance": acc.GetBalance().String(), "nonce": acc.GetNonce().String()}
	}
	return map[string]any{"raw": raw, "accounts": accounts}
}

func main() {
	var baseline map[string]json.RawMessage
	if err := json.NewDecoder(os.Stdin).Decode(&baseline); err != nil {
		panic(err)
	}
	seed := estimateRetainedSeed()
	before := retainedSnapshot(seed)
	encoded, _ := json.Marshal(before)
	var normalized, expected map[string]any
	if err := json.Unmarshal(encoded, &normalized); err != nil {
		panic(err)
	}
	if err := json.Unmarshal(baseline["state_before"], &expected); err != nil {
		panic(err)
	}
	if !reflect.DeepEqual(normalized, expected) {
		panic("complete retained P2 identity differs")
	}
	state_evm.SetNativeV1CustodyRawWriteObserver(func(a common.Address, k common.Hash, v []byte) {
		retainedWrites = append(retainedWrites, custodyRawWrite{Address: hex.EncodeToString(a[:]), Key: hex.EncodeToString(k[:]), Value: hex.EncodeToString(v)})
	})
	state_evm.SetRewardBalanceObserver(func(a common.Address, op string, amount, balance *big.Int) {
		retainedBalances = append(retainedBalances, map[string]string{"address": hex.EncodeToString(a[:]), "operation": op, "amount": amount.String(), "balance": balance.String()})
	})
	block := &vm.Block{Number: 3, BlockInfo: vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000003, Difficulty: new(big.Int)}}
	a := *dpos.ContractAddress()
	tx := func(sender common.Address, offset int64) vm.Transaction {
		return vm.Transaction{From: sender, To: &a, Nonce: new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 512), big.NewInt(offset)), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: custodyInput("claimCommissionRewards(address)", nil)}
	}
	definitions := []struct {
		name            string
		prefix, targets []vm.Transaction
	}{
		{"positive", []vm.Transaction{}, []vm.Transaction{tx(custodyOwner, 0)}},
		{"prefix_then_zero", []vm.Transaction{tx(custodyOwner, 0)}, []vm.Transaction{tx(custodyOwner, 1)}},
		{"wrong_owner", []vm.Transaction{}, []vm.Transaction{tx(custodyDelegator, 0)}},
	}
	cases := []map[string]any{}
	for _, def := range definitions {
		prefix, targets := []map[string]any{}, []map[string]any{}
		for _, t := range def.prefix {
			prefix = append(prefix, retainedTraceRequest(t))
		}
		for _, t := range def.targets {
			targets = append(targets, retainedTraceRequest(t))
		}
		var first map[string]any
		for repeat := 0; repeat < 2; repeat++ {
			calls := []map[string]any{}
			var current map[string]any
			state_dry_runner.SetRetainedTraceObserver(func(stage string, after bool, state *state_evm.BlockState, request *vm.Transaction, result *vm.ExecutionResult, err error) {
				if !after {
					retainedWrites = []custodyRawWrite{}
					retainedBalances = []map[string]string{}
					current = map[string]any{"stage": stage, "request": retainedTraceRequest(*request), "before": retainedTraceState(state)}
					return
				}
				message := ""
				if err != nil {
					message = err.Error()
				}
				current["after"] = retainedTraceState(state)
				current["output"] = map[string]any{"gas_used": result.GasUsed, "execution_error": string(result.ExecutionErr), "consensus_error": string(result.ConsensusErr), "error": message, "return": hex.EncodeToString(result.CodeRetval), "logs": retainedLogs(result.Logs)}
				current["raw"] = retainedWrites
				current["balances"] = retainedBalances
				calls = append(calls, current)
			})
			api := new(dpos.API).Init(seed.config)
			factory := func(p types.BlockNum) storage.StorageReader {
				return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(p)}
			}
			runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
			// Capture actual diagnostics separately; never construct trace rows.
			r, w, err := os.Pipe()
			if err != nil {
				panic(err)
			}
			stdout := os.Stdout
			os.Stdout = w
			result := runner.Trace(block, &def.prefix, &def.targets, nil)
			os.Stdout = stdout
			if err := w.Close(); err != nil {
				panic(err)
			}
			var diagnostics bytes.Buffer
			if _, err := diagnostics.ReadFrom(r); err != nil {
				panic(err)
			}
			r.Close()
			actual := map[string]any{"name": def.name, "prefix": prefix, "targets": targets, "result": json.RawMessage(result), "diagnostics": diagnostics.String(), "observation_witness": calls}
			if !reflect.DeepEqual(before, retainedSnapshot(seed)) {
				panic("trace mutated committed P2")
			}
			for i, t := range def.prefix {
				if !reflect.DeepEqual(prefix[i], retainedTraceRequest(t)) {
					panic("prefix request changed")
				}
			}
			for i, t := range def.targets {
				if !reflect.DeepEqual(targets[i], retainedTraceRequest(t)) {
					panic("target request changed")
				}
			}
			if repeat == 0 {
				first = actual
			} else if !reflect.DeepEqual(first, actual) {
				panic("fresh trace repeat differs")
			}
		}
		cases = append(cases, first)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "period": 3, "timestamp": block.Time, "author": hex.EncodeToString(block.Author[:]), "block_gas": block.GasLimit, "repeat_count": 2, "configuration": seed.config, "state_before": before, "state_after": retainedSnapshot(seed), "cases": cases}); err != nil {
		panic(err)
	}
}
