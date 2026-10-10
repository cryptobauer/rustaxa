// Actual seven-cap P2/P1 traces with empty or ordinary prefix over canonical P1.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"reflect"
	"strconv"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var h1TraceReaderArmed bool
var h1TraceReaderPeriods []uint64

// Shared copied-fixture commit hook is deliberately inert for this P1 profile.
func observeH1TraceCommit(database *nativeSimulationDB) {}
func observeH1TraceReader(period types.BlockNum) {
	if h1TraceReaderArmed {
		h1TraceReaderPeriods = append(h1TraceReaderPeriods, uint64(period))
	}
}
func h1TraceIdentity(key byte) common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{key}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	return common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
}
func main() {
	nativeSimulationSender = h1TraceIdentity(0x31)
	nativeSimulationDelegator = h1TraceIdentity(0xa1)
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	p1Before := currentSignedSnapshot(seed, api)
	contract := *dpos.ContractAddress()
	control := os.Getenv("CLAIM_ALL_H1_TRACE_CONTROL") == "1"
	stage := ""
	var reads, accountReads, quotes, runs []map[string]any
	var writes, balances []map[string]string
	var post map[string]any
	var prefixResults []map[string]any
	vm.SetClaimAllBoundaryObserver(func(kind string, start bool, frame vm.CallFrame, e *vm.EVM, required uint64, output []byte, err error) {
		if start {
			stage = kind
			if kind == "run" {
				runs = append(runs, map[string]any{"supplied_native_gas": frame.Gas})
			}
			return
		}
		if kind == "quote" {
			caller := frame.CallerAccount.Address()
			quotes = append(quotes, map[string]any{"required_gas": required, "supplied_native_gas": frame.Gas, "caller": hex.EncodeToString(caller[:]), "input": hex.EncodeToString(frame.Input), "depth": e.GetDepth(), "value": frame.Value.String()})
		} else {
			row := runs[len(runs)-1]
			row["output"], row["error"] = hex.EncodeToString(output), ""
			if err != nil {
				row["error"] = err.Error()
			}
		}
		stage = ""
	})
	storage.SetClaimAllBoundaryReadObserver(func(kind string, address *common.Address, key *common.Hash) {
		if !control && stage != "" {
			reads = append(reads, map[string]any{"stage": stage, "kind": kind, "address": hex.EncodeToString(address[:]), "key": hex.EncodeToString(key[:])})
		}
	})
	state_evm.SetNativeV1CustodyRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		if !control && stage != "" {
			writes = append(writes, map[string]string{"address": hex.EncodeToString(address[:]), "key": hex.EncodeToString(key[:]), "value": hex.EncodeToString(value)})
		}
	})
	state_evm.SetRewardBalanceObserver(func(address common.Address, operation string, amount, balance *big.Int) {
		if !control && stage != "" {
			balances = append(balances, map[string]string{"address": hex.EncodeToString(address[:]), "operation": operation, "amount": amount.String(), "balance": balance.String()})
		}
	})
	state_evm.SetClaimAllH1AccountReadObserver(func(kind string, address *common.Address) {
		if !control && stage != "" {
			accountReads = append(accountReads, map[string]any{"stage": stage, "kind": kind, "address": hex.EncodeToString(address[:])})
		}
	})
	state_dry_runner.SetClaimAllH1TraceTargetObserver(func(ret vm.ExecutionResult, err error, state *state_evm.BlockState, transaction *vm.Transaction) {
		accounts := map[string]any{}
		for _, address := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, contract} {
			account := state.GetAccountConcrete(&address)
			accounts[hex.EncodeToString(address[:])] = map[string]string{"nonce": account.GetNonce().String(), "balance": account.GetBalance().String()}
		}
		logs := []map[string]any{}
		for _, log := range ret.Logs {
			topics := []string{}
			for _, topic := range log.Topics {
				topics = append(topics, hex.EncodeToString(topic[:]))
			}
			logs = append(logs, map[string]any{"address": hex.EncodeToString(log.Address[:]), "topics": topics, "data": hex.EncodeToString(log.Data)})
		}
		parentError := ""
		if err != nil {
			parentError = err.Error()
		}
		post = map[string]any{"gas_used": ret.GasUsed, "consensus_error": string(ret.ConsensusErr), "execution_error": string(ret.ExecutionErr), "parent_error": parentError, "return": hex.EncodeToString(ret.CodeRetval), "logs": logs, "supplied_nonce": transaction.Nonce.String(), "accounts": accounts, "after_raw": h1RawCopy(state)}
	})
	state_dry_runner.SetClaimAllBoundaryPrefixObserver(func(ret vm.ExecutionResult, err error, state *state_evm.BlockState, transaction *vm.Transaction) {
		outer := ""
		if err != nil {
			outer = err.Error()
		}
		sender := state.GetAccountConcrete(&transaction.From)
		native := state.GetAccountConcrete(&contract)
		prefixResults = append(prefixResults, map[string]any{"gas": ret.GasUsed, "consensus_error": string(ret.ConsensusErr), "execution_error": string(ret.ExecutionErr), "outer_error": outer, "return_value": hex.EncodeToString(ret.CodeRetval), "logs": len(ret.Logs), "sender_nonce": sender.GetNonce().String(), "sender_balance": sender.GetBalance().String(), "native_balance": native.GetBalance().String()})
	})
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: 2, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)}}
	priorRaw := h1RawCopy(state_evm.GetBlockState(seed.database, 1, 2))
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []map[string]any{}
	for layout := 0; layout < 2; layout++ {
		for _, cap := range []uint64{21271, 21272, 21273, 90000, 111271, 111272, 111273} {
			attempts := []map[string]any{}
			for repeat := 0; repeat < 2; repeat++ {
				reads, accountReads, quotes, runs = []map[string]any{}, []map[string]any{}, []map[string]any{}, []map[string]any{}
				writes, balances = []map[string]string{}, []map[string]string{}
				h1TraceReaderPeriods, stage, post = []uint64{}, "", nil
				prefixResults = []map[string]any{}
				transaction := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: cap, Value: new(big.Int), Input: common.FromHex("0b83a727")}
				transaction.Nonce.Add(transaction.Nonce, big.NewInt(int64(layout)))
				prefix := []vm.Transaction{}
				if layout == 1 {
					prefix = append(prefix, vm.Transaction{From: nativeSimulationSender, To: &nativeSimulationSender, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: 21000, Value: new(big.Int)})
				}
				prefixRequest := []map[string]any{}
				for _, p := range prefix {
					prefixRequest = append(prefixRequest, traceRequest(p))
				}
				original := traceRequest(transaction)
				h1TraceReaderArmed = true
				result, diagnostics := redelegateTrace(runner, block, prefix, []vm.Transaction{transaction})
				h1TraceReaderArmed = false
				if !reflect.DeepEqual(original, traceRequest(transaction)) || post == nil || (layout == 1 && !reflect.DeepEqual(prefixRequest[0], traceRequest(prefix[0]))) {
					panic("trace input changed or post observer missing")
				}
				if !reflect.DeepEqual(h1TraceReaderPeriods, []uint64{1}) {
					panic("trace did not select exact historical1 reader")
				}
				if !reflect.DeepEqual(p1Before, currentSignedSnapshot(seed, api)) {
					panic("trace changed complete canonical P1")
				}
				attempts = append(attempts, map[string]any{"name": "claim_all_P2_P1_prefix" + strconv.Itoa(layout) + "_gas" + strconv.FormatUint(cap, 10), "prefix": prefixRequest, "prefix_results": prefixResults, "targets": []map[string]any{original}, "result": json.RawMessage(result), "diagnostics": diagnostics, "reader_periods": h1TraceReaderPeriods, "execution_period": uint64(block.Number), "quotes": quotes, "runs": runs, "ordered_reads": reads, "ordered_account_scalar_reads": accountReads, "ordered_raw_writes": writes, "ordered_balance_effects": balances, "prior_raw": priorRaw, "post": post})
			}
			if !reflect.DeepEqual(attempts[0], attempts[1]) {
				panic("P2 P1 trace repeat differs")
			}
			cases = append(cases, attempts[0])
		}
	}
	output := map[string]any{"schema": 1, "caller": hex.EncodeToString(nativeSimulationSender[:]), "owner": hex.EncodeToString(nativeSimulationDelegator[:]), "p1_before": p1Before, "p1_after": currentSignedSnapshot(seed, api), "repeat_identical": true, "source_unchanged_after_each": true, "cases": cases, "scope": "actual seven-cap P2 TraceRunner over immutable canonical92P1, empty/ordinary zero-self-transfer prefix, original wide nonces; no runtime/root/network/production acceptance"}
	if err := json.NewEncoder(os.Stdout).Encode(output); err != nil {
		panic(err)
	}
}
