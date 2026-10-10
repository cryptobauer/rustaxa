// Literal H1 TraceRunner over actual captured P0 from unchanged canonical history.
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
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var h1TraceCaptureArmed, h1TraceReaderArmed bool
var h1TraceCaptured, h1TraceOriginal *nativeSimulationDB
var h1TraceCaptureCount int
var h1TraceReaderPeriods []uint64

// These hooks run only in a guarded copy of the existing fixture DB.
func observeH1TraceCommit(database *nativeSimulationDB) {
	if !h1TraceCaptureArmed || database.descriptor.BlockNum != 0 {
		return
	}
	h1TraceCaptureCount++
	if h1TraceCaptureCount != 1 {
		panic("multiple P0 captures")
	}
	h1TraceOriginal = database
	captured := newNativeSimulationDB()
	captured.descriptor = database.descriptor
	database.rows.mu.Lock()
	captured.rows.period = database.rows.period
	for column := range database.rows.physical {
		for key, value := range database.rows.physical[column] {
			captured.rows.physical[column][key] = common.CopyBytes(value)
		}
		for key, value := range database.rows.latest[column] {
			captured.rows.latest[column][key] = common.CopyBytes(value)
		}
	}
	database.rows.mu.Unlock()
	h1TraceCaptured = captured
}
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
func h1TraceP0View(seed nativeSimulationSeed, database *nativeSimulationDB) map[string]any {
	reader := state_db.ExtendedReader{Reader: database.GetBlockStateReader(0)}
	accounts := []map[string]any{}
	for _, address := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, *dpos.ContractAddress()} {
		row := map[string]any{"address": hex.EncodeToString(address[:]), "exists": false}
		reader.GetRawAccount(&address, func(encoded []byte) {
			account := state_db.DecodeAccountFromTrie(encoded)
			row["exists"], row["encoded"] = true, hex.EncodeToString(encoded)
			row["nonce"], row["balance"] = account.Nonce.String(), account.Balance.String()
			if account.CodeHash != nil {
				row["code_hash"] = hex.EncodeToString(account.CodeHash[:])
				row["code"] = hex.EncodeToString(reader.GetCode(account.CodeHash))
			}
		})
		accounts = append(accounts, row)
	}
	return map[string]any{"accounts": accounts, "native_facts": signedH1Facts(reader), "native_raw": signedH1Raw(reader), "configuration": seed.config}
}
func verifyH1TraceP0Bytes(database *nativeSimulationDB, rows []nativeSimulationSeedRow) {
	reader := database.GetBlockStateReader(0)
	for _, row := range rows {
		keyBytes := common.FromHex(row.Key)
		if len(keyBytes) != 32 && len(keyBytes) != 40 {
			panic("unsupported captured physical key")
		}
		key := common.BytesToHash(keyBytes[:32])
		value := []byte{}
		reader.Get(state_db.Column(row.Column), &key, func(data []byte) { value = common.CopyBytes(data) })
		if hex.EncodeToString(value) != row.Value {
			panic("P0 byte readback differs: " + row.Key)
		}
	}
}
func main() {
	nativeSimulationSender = h1TraceIdentity(0x31)
	nativeSimulationDelegator = h1TraceIdentity(0xa1)
	h1TraceCaptureArmed = true
	seed := seedRewardH1()
	h1TraceCaptureArmed = false
	if h1TraceCaptureCount != 1 || h1TraceOriginal != seed.database || h1TraceCaptured.descriptor.BlockNum != 0 {
		panic("missing exact genesis capture")
	}
	api := new(dpos.API).Init(seed.config)
	p1Before := currentSignedSnapshot(seed, api)
	p0Rows := nativeSimulationSeedRows(h1TraceCaptured)
	p0View := h1TraceP0View(seed, h1TraceCaptured)
	verifyH1TraceP0Bytes(seed.database, p0Rows)
	if !reflect.DeepEqual(p0View, h1TraceP0View(seed, seed.database)) {
		panic("historical P0 view differs from captured view")
	}
	counts := make([]int, state_db.COL_COUNT)
	for _, row := range p0Rows {
		counts[row.Column]++
	}
	root := h1TraceCaptured.descriptor.StateRoot
	p0 := map[string]any{"period": 0, "root": hex.EncodeToString(root[:]), "seed_rows": p0Rows, "column_counts": counts, "view": p0View}
	contract := *dpos.ContractAddress()
	control := os.Getenv("CLAIM_ALL_H1_TRACE_CONTROL") == "1"
	stage := ""
	var reads, accountReads, quotes, runs []map[string]any
	var writes, balances []map[string]string
	var post map[string]any
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
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	priorRaw := h1RawCopy(state_evm.GetBlockState(seed.database, 0, 1))
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	attempts := []map[string]any{}
	for repeat := 0; repeat < 2; repeat++ {
		reads, accountReads, quotes, runs = []map[string]any{}, []map[string]any{}, []map[string]any{}, []map[string]any{}
		writes, balances = []map[string]string{}, []map[string]string{}
		h1TraceReaderPeriods, stage, post = []uint64{}, "", nil
		transaction := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: common.FromHex("0b83a727")}
		original := traceRequest(transaction)
		h1TraceReaderArmed = true
		result, diagnostics := redelegateTrace(runner, block, []vm.Transaction{}, []vm.Transaction{transaction})
		h1TraceReaderArmed = false
		if !reflect.DeepEqual(original, traceRequest(transaction)) || post == nil {
			panic("trace input changed or post observer missing")
		}
		if !reflect.DeepEqual(h1TraceReaderPeriods, []uint64{0}) {
			panic("trace did not select exact historical0 reader")
		}
		verifyH1TraceP0Bytes(seed.database, p0Rows)
		if !reflect.DeepEqual(p0View, h1TraceP0View(seed, seed.database)) || !reflect.DeepEqual(p0Rows, nativeSimulationSeedRows(h1TraceCaptured)) || !reflect.DeepEqual(p1Before, currentSignedSnapshot(seed, api)) {
			panic("trace changed P0 source or committed P1")
		}
		attempts = append(attempts, map[string]any{"name": "claim_all_H1_P0_empty_prefix", "prefix": []map[string]any{}, "targets": []map[string]any{original}, "result": json.RawMessage(result), "diagnostics": diagnostics, "reader_periods": h1TraceReaderPeriods, "execution_period": uint64(block.Number), "quotes": quotes, "runs": runs, "ordered_reads": reads, "ordered_account_scalar_reads": accountReads, "ordered_raw_writes": writes, "ordered_balance_effects": balances, "prior_raw": priorRaw, "post": post})
	}
	if !reflect.DeepEqual(attempts[0], attempts[1]) {
		panic("H1 P0 trace repeat differs")
	}
	p0After := map[string]any{"period": 0, "root": hex.EncodeToString(root[:]), "seed_rows": nativeSimulationSeedRows(h1TraceCaptured), "column_counts": counts, "view": h1TraceP0View(seed, seed.database)}
	output := map[string]any{"schema": 1, "caller": hex.EncodeToString(nativeSimulationSender[:]), "owner": hex.EncodeToString(nativeSimulationDelegator[:]), "p0_capture_count": h1TraceCaptureCount, "p0_source_before": p0, "p0_source_after": p0After, "p1_before": p1Before, "p1_after": currentSignedSnapshot(seed, api), "repeat_identical": true, "source_unchanged_after_each": true, "cases": attempts[:1], "scope": "literal empty-prefix H1 TraceRunner with actual historicalP0 from unchanged canonical92P1 history; supplied wide nonce; source fixture provenance only; no Rust/root/network/production acceptance"}
	if err := json.NewEncoder(os.Stdout).Encode(output); err != nil {
		panic(err)
	}
}
