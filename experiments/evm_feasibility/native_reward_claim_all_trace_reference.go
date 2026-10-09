// Actual direct positive claim-all public trace, empty or ordinary self-transfer prefix, over canonical P1.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

var traceRaw, traceBalances []map[string]string
var traceLogs []map[string]any
var tracePrefixes []map[string]any

func resetRewardTraceWitness() {
	traceRaw = []map[string]string{}
	traceBalances = []map[string]string{}
	traceLogs = []map[string]any{}
	tracePrefixes = []map[string]any{}
}
func rewardTraceWitness() map[string]any {
	return map[string]any{"raw": append([]map[string]string{}, traceRaw...), "balances": append([]map[string]string{}, traceBalances...), "logs": append([]map[string]any{}, traceLogs...)}
}
func main() {
	state_evm.SetNativeV1CustodyRawWriteObserver(func(a common.Address, k common.Hash, v []byte) {
		traceRaw = append(traceRaw, map[string]string{"address": hex.EncodeToString(a[:]), "key": hex.EncodeToString(k[:]), "value": hex.EncodeToString(v)})
	})
	state_evm.SetRewardBalanceObserver(func(a common.Address, op string, amount, balance *big.Int) {
		traceBalances = append(traceBalances, map[string]string{"address": hex.EncodeToString(a[:]), "operation": op, "amount": amount.String(), "balance": balance.String()})
	})
	state_dry_runner.SetRewardTraceLogObserver(func(logs []vm.LogRecord) {
		for _, l := range logs {
			topics := []string{}
			for _, t := range l.Topics {
				topics = append(topics, hex.EncodeToString(t[:]))
			}
			traceLogs = append(traceLogs, map[string]any{"address": hex.EncodeToString(l.Address[:]), "topics": topics, "data": hex.EncodeToString(l.Data)})
		}
	})

	state_dry_runner.SetRewardTracePrefixObserver(func(ret vm.ExecutionResult, err error, state *state_evm.BlockState, trx *vm.Transaction) {
		outerError := ""
		if err != nil {
			outerError = err.Error()
		}
		sender := state.GetAccountConcrete(&trx.From)
		native := state.GetAccountConcrete(dpos.ContractAddress())
		tracePrefixes = append(tracePrefixes, map[string]any{"gas": ret.GasUsed, "consensus_error": string(ret.ConsensusErr), "execution_error": string(ret.ExecutionErr), "outer_error": outerError, "return_value": hex.EncodeToString(ret.CodeRetval), "logs": len(ret.Logs), "sender_nonce": sender.GetNonce().String(), "sender_balance": sender.GetBalance().String(), "native_balance": native.GetBalance().String()})
	})
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0x31}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationSender = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	x, y = secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0xa1}, 32))
	pub = append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationDelegator = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: 2, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)}}
	before := currentSignedSnapshot(seed, api)
	address := *dpos.ContractAddress()
	tx := func(input []byte) vm.Transaction {
		return vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Lsh(big.NewInt(1), 512), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: input}
	}
	first := tx(common.FromHex("0b83a727"))
	ordinary := tx(nil)
	ordinary.To = &nativeSimulationSender
	ordinary.Gas = 21000
	prefixedTarget := tx(first.Input)
	prefixedTarget.Nonce.Add(prefixedTarget.Nonce, big.NewInt(1))
	definitions := []struct {
		name            string
		prefix, targets []vm.Transaction
	}{
		{name: "claim_all_empty_prefix", prefix: []vm.Transaction{}, targets: []vm.Transaction{first}},
		{name: "claim_all_ordinary_prefix", prefix: []vm.Transaction{ordinary}, targets: []vm.Transaction{prefixedTarget}},
	}
	cases := []map[string]any{}
	for _, definition := range definitions {
		resetRewardTraceWitness()
		result, diagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		witness := rewardTraceWitness()
		prefixResults := append([]map[string]any{}, tracePrefixes...)
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("first trace mutatedP1")
		}
		resetRewardTraceWitness()
		repeated, repeatedDiagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("repeat trace mutatedP1")
		}
		if !reflect.DeepEqual(witness, rewardTraceWitness()) || !reflect.DeepEqual(prefixResults, tracePrefixes) {
			panic("trace witness differs on repeat")
		}
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
		cases = append(cases, map[string]any{"name": definition.name, "prefix": prefix, "targets": targets, "result": json.RawMessage(result), "diagnostics": diagnostics, "observation_witness": witness, "prefix_results": prefixResults})
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("trace changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "period": uint64(block.Number), "cases": cases, "scope": "actual one selected positive noarg claim-all direct TraceRunner target/native2 over canonical92rowP1; empty or ordinary zero self-transfer prefix; original wide nonces; copied prefix result in both controls and native raw/account/target-log witnesses only in observed run; no committed mutation, prior native prefix, repeated claim-all, nested, historical, root parity or production acceptance"}); err != nil {
		panic(err)
	}
}
