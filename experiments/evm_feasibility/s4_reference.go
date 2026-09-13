// Synthetic two-period S4 oracle using the pinned Go EVM, TransitionState,
// and TrieSink. Run through s4_reference.py so both archived references are
// built independently. The in-memory database preserves the five concrete
// column families, including zero-length tombstones.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"sort"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
)

const (
	chainID        = uint64(841)
	transactionGas = uint64(100000)
	blockGas       = uint64(1000000)
)

var (
	testPrivateKey = bytes.Repeat([]byte{0x01}, 32)
	recipient      = common.HexToAddress("0x2222222222222222222222222222222222222222")
	runtimeCode    = mustHex("60003560005500")
	// SSTORE(0, 1), then return the seven-byte runtime from memory[25:32].
	initCode = mustHex("6001600055666000356000550060005260076019f3")
)

type memoryRows struct {
	mu       sync.Mutex
	period   uint64
	latest   [state_db.COL_COUNT]map[common.Hash][]byte
	physical [state_db.COL_COUNT]map[string][]byte
}

func newMemoryRows() *memoryRows {
	ret := new(memoryRows)
	for i := range ret.latest {
		ret.latest[i] = make(map[common.Hash][]byte)
		ret.physical[i] = make(map[string][]byte)
	}
	return ret
}

func (m *memoryRows) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	m.mu.Lock()
	value, present := m.latest[column][*key]
	value = common.CopyBytes(value)
	m.mu.Unlock()
	if present && len(value) != 0 {
		callback(value)
	}
}

func (m *memoryRows) Put(column state_db.Column, key *common.Hash, value []byte) {
	m.mu.Lock()
	m.latest[column][*key] = common.CopyBytes(value)
	physicalKey := append([]byte(nil), key[:]...)
	if column == state_db.COL_main_trie_value || column == state_db.COL_acc_trie_value {
		version := make([]byte, 8)
		binary.BigEndian.PutUint64(version, m.period)
		physicalKey = append(physicalKey, version...)
	}
	m.physical[column][string(physicalKey)] = common.CopyBytes(value)
	m.mu.Unlock()
}

func (m *memoryRows) setPeriod(period uint64) {
	m.mu.Lock()
	m.period = period
	m.mu.Unlock()
}

func exportRows[K comparable](rows []map[K][]byte, keyBytes func(K) []byte) []map[string]string {
	ret := make([]map[string]string, len(rows))
	for column, columnRows := range rows {
		ret[column] = make(map[string]string, len(columnRows))
		for key, value := range columnRows {
			ret[column][hex.EncodeToString(keyBytes(key))] = hex.EncodeToString(value)
		}
	}
	return ret
}

func (m *memoryRows) exportLatest() []map[string]string {
	m.mu.Lock()
	defer m.mu.Unlock()
	rows := make([]map[common.Hash][]byte, len(m.latest))
	for i := range m.latest {
		rows[i] = m.latest[i]
	}
	return exportRows(rows, func(key common.Hash) []byte { return key[:] })
}

func (m *memoryRows) exportPhysical() []map[string]string {
	m.mu.Lock()
	defer m.mu.Unlock()
	rows := make([]map[string][]byte, len(m.physical))
	for i := range m.physical {
		rows[i] = m.physical[i]
	}
	return exportRows(rows, func(key string) []byte { return []byte(key) })
}

type serializedRows struct {
	Period   uint64              `json:"period"`
	Latest   []map[string]string `json:"latest"`
	Physical []map[string]string `json:"physical"`
}

func (m *memoryRows) serialize() []byte {
	m.mu.Lock()
	period := m.period
	m.mu.Unlock()
	encoded, err := json.Marshal(serializedRows{Period: period, Latest: m.exportLatest(), Physical: m.exportPhysical()})
	must(err)
	return encoded
}

func rowsFromJSON(encoded []byte) *memoryRows {
	var snapshot serializedRows
	must(json.Unmarshal(encoded, &snapshot))
	if len(snapshot.Latest) != int(state_db.COL_COUNT) || len(snapshot.Physical) != int(state_db.COL_COUNT) {
		panic(fmt.Sprintf("serialized concrete rows do not have %d columns", state_db.COL_COUNT))
	}
	ret := newMemoryRows()
	ret.period = snapshot.Period
	for column, rows := range snapshot.Latest {
		for keyHex, valueHex := range rows {
			keyBytes, err := hex.DecodeString(keyHex)
			must(err)
			if len(keyBytes) != len(common.Hash{}) {
				panic(fmt.Sprintf("serialized concrete key has %d bytes", len(keyBytes)))
			}
			value, err := hex.DecodeString(valueHex)
			must(err)
			key := common.BytesToHash(keyBytes)
			ret.latest[column][key] = value
		}
	}
	for column, rows := range snapshot.Physical {
		for keyHex, valueHex := range rows {
			key, err := hex.DecodeString(keyHex)
			must(err)
			if len(key) != len(common.Hash{}) && len(key) != len(common.Hash{})+8 {
				panic(fmt.Sprintf("serialized physical key has %d bytes", len(key)))
			}
			value, err := hex.DecodeString(valueHex)
			must(err)
			ret.physical[column][string(key)] = value
		}
	}
	return ret
}

func cloneRows(rows *memoryRows) *memoryRows {
	return rowsFromJSON(rows.serialize())
}

type unsignedLegacyTransaction struct {
	Nonce    *big.Int
	GasPrice *big.Int
	Gas      uint64
	To       *common.Address `rlp:"nil"`
	Value    *big.Int
	Input    []byte
	ChainID  *big.Int
	ZeroR    *big.Int
	ZeroS    *big.Int
}

type signedLegacyTransaction struct {
	Nonce    *big.Int
	GasPrice *big.Int
	Gas      uint64
	To       *common.Address `rlp:"nil"`
	Value    *big.Int
	Input    []byte
	V        *big.Int
	R        *big.Int
	S        *big.Int
}

// externalReceipt is FinalChain's five-field persistence shape. StateAPI's
// six-field vm.ExecutionResult is exported separately for direct comparison.
type externalReceipt struct {
	Status            uint8
	GasUsed           uint64
	CumulativeGasUsed uint64
	Logs              []vm.LogRecord
	NewContract       *common.Address `rlp:"nil"`
}

type transactionSpec struct {
	Name  string
	Nonce uint64
	To    *common.Address
	Value uint64
	Input []byte
}

type signedTransaction struct {
	RLP  []byte
	Hash common.Hash
	V    *big.Int
	R    *big.Int
	S    *big.Int
}

func signTransaction(spec transactionSpec, sender common.Address) signedTransaction {
	unsigned := unsignedLegacyTransaction{
		Nonce: new(big.Int).SetUint64(spec.Nonce), GasPrice: new(big.Int), Gas: transactionGas,
		To: spec.To, Value: new(big.Int).SetUint64(spec.Value), Input: spec.Input,
		ChainID: new(big.Int).SetUint64(chainID), ZeroR: new(big.Int), ZeroS: new(big.Int),
	}
	digest := crypto.Keccak256Hash(rlp.MustEncodeToBytes(&unsigned))
	signature, err := secp256k1.Sign(digest[:], testPrivateKey)
	must(err)
	publicKey, err := secp256k1.RecoverPubkey(digest[:], signature)
	must(err)
	recovered := common.BytesToAddress(crypto.Keccak256(publicKey[1:])[12:])
	if recovered != sender {
		panic(fmt.Sprintf("signature recovered %x, want %x", recovered, sender))
	}
	r := new(big.Int).SetBytes(signature[:32])
	s := new(big.Int).SetBytes(signature[32:64])
	v := new(big.Int).SetUint64(chainID*2 + 35 + uint64(signature[64]))
	signed := signedLegacyTransaction{
		Nonce: unsigned.Nonce, GasPrice: unsigned.GasPrice, Gas: unsigned.Gas, To: unsigned.To,
		Value: unsigned.Value, Input: unsigned.Input, V: v, R: r, S: s,
	}
	encoded := rlp.MustEncodeToBytes(&signed)
	return signedTransaction{RLP: encoded, Hash: crypto.Keccak256Hash(encoded), V: v, R: r, S: s}
}

func testSender() common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(testPrivateKey)
	publicKey := secp256k1.S256().Marshal(x, y)
	return common.BytesToAddress(crypto.Keccak256(publicKey[1:])[12:])
}

func seedGenesis(sender common.Address) (*memoryRows, common.Hash) {
	rows := newMemoryRows()
	writer := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
	key := crypto.Keccak256Hash(sender[:])
	account := state_db.Account{Nonce: new(big.Int), Balance: big.NewInt(1000000)}
	io := state_db.MainTrieIOAdapter{ReadWriter: rows}
	writer.Put(io, &key, &account)
	root := writer.Commit(io)
	if root == nil {
		panic("seeded genesis produced an empty root")
	}
	return rows, *root
}

func executeTransaction(state *state_evm.TransitionState, period uint64, sender common.Address, spec transactionSpec) (vm.Transaction, vm.ExecutionResult, uint64, string) {
	transaction := vm.Transaction{
		From: sender, To: spec.To, Nonce: new(big.Int).SetUint64(spec.Nonce),
		GasPrice: new(big.Int), Value: new(big.Int).SetUint64(spec.Value), Gas: transactionGas,
		Input: common.CopyBytes(spec.Input),
	}
	var evm vm.EVM
	evm.Init(
		func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(),
		params.ChainConfig{ChainId: chainID}, vm.Config{},
	)
	evm.SetBlock(&vm.Block{
		Number:    types.BlockNum(period),
		BlockInfo: vm.BlockInfo{GasLimit: blockGas, Difficulty: new(big.Int)},
	}, vm.Rules{IsMagnolia: true, IsAspenPartOne: true, IsCornus: true})
	result, err := evm.Main(&transaction)
	errorText := ""
	if err != nil {
		errorText = err.Error()
	}
	return transaction, result, state.GetRefund(), errorText
}

func executePeriod(base *memoryRows, priorRoot common.Hash, period uint64, sender common.Address, specs []transactionSpec, concreteObserver bool) (map[string]any, *memoryRows, common.Hash) {
	rows := cloneRows(base)
	rows.setPeriod(period)
	var state state_evm.TransitionState
	state.Init(state_evm.Opts{})
	state.SetInput(state_db.ExtendedReader{Reader: rows})
	sink := new(state_transition.TrieSink).Init(&priorRoot, state_transition.TrieSinkOpts{})
	sink.SetIO(rows)
	transactions := make([]map[string]any, 0, len(specs))
	var cumulativeGasUsed uint64
	root := priorRoot
	for index, spec := range specs {
		transaction, result, refund, errorText := executeTransaction(&state, period, sender, spec)
		if errorText != "" || result.ExecutionErr != "" || result.ConsensusErr != "" {
			panic(fmt.Sprintf("%s failed: error=%q execution=%q consensus=%q", spec.Name, errorText, result.ExecutionErr, result.ConsensusErr))
		}
		signed := signTransaction(spec, sender)
		cumulativeGasUsed += result.GasUsed
		var newContract *common.Address
		if result.NewContractAddr != (common.Address{}) {
			address := result.NewContractAddr
			newContract = &address
		}
		receipt := externalReceipt{
			Status: 1, GasUsed: result.GasUsed, CumulativeGasUsed: cumulativeGasUsed,
			Logs: result.Logs, NewContract: newContract,
		}
		logs := make([]map[string]any, 0, len(result.Logs))
		for _, log := range result.Logs {
			topics := make([]string, len(log.Topics))
			for i, topic := range log.Topics {
				topics[i] = hex.EncodeToString(topic[:])
			}
			logs = append(logs, map[string]any{
				"address": hex.EncodeToString(log.Address[:]), "topics": topics, "data": hex.EncodeToString(log.Data),
			})
		}
		transactionRow := map[string]any{
			"index": index, "name": spec.Name, "nonce": spec.Nonce, "to": addressHex(spec.To),
			"sender": hex.EncodeToString(sender[:]),
			"value":  fmt.Sprint(spec.Value), "gas_price": "0", "gas_limit": transactionGas,
			"input": hex.EncodeToString(spec.Input), "signed_rlp": hex.EncodeToString(signed.RLP),
			"hash": hex.EncodeToString(signed.Hash[:]), "v": signed.V.String(), "r": signed.R.String(), "s": signed.S.String(),
			"state_api_transaction_rlp":        hex.EncodeToString(rlp.MustEncodeToBytes(&transaction)),
			"state_api_execution_result_rlp":   hex.EncodeToString(rlp.MustEncodeToBytes(&result)),
			"receipt_rlp":                      hex.EncodeToString(rlp.MustEncodeToBytes(&receipt)),
			"status":                           1,
			"gas_used":                         result.GasUsed,
			"cumulative_gas_used":              cumulativeGasUsed,
			"refund_before_transaction_commit": refund,
			"output":                           hex.EncodeToString(result.CodeRetval), "created": hex.EncodeToString(result.NewContractAddr[:]),
			"logs": logs, "code_error": string(result.ExecutionErr), "execution_error": string(result.ExecutionErr),
			"consensus_error": string(result.ConsensusErr),
			"error":           errorText,
		}
		state.CommitTransaction(sink)
		if state.GetRefund() != 0 || len(state.GetLogs()) != 0 {
			panic("transaction-local refund or logs survived CommitTransaction")
		}
		if concreteObserver {
			// Concrete StateAPI calls PrepareIntermediateRoot after every transaction.
			// Commit joins account writers; sink.Commit joins the main trie writer, so
			// physical rows and account reads below cannot race asynchronous writes.
			state.Commit()
			root = sink.Commit()
			transactionRow["intermediate_root"] = hex.EncodeToString(root[:])
			transactionRow["intermediate_root_source"] = "concrete StateAPI per-transaction PrepareIntermediateRoot"
			addresses := []common.Address{sender}
			if spec.To != nil {
				addresses = append(addresses, *spec.To)
			} else if newContract != nil {
				addresses = append(addresses, *newContract)
			}
			accountRows := make([]map[string]any, 0, len(addresses))
			for _, address := range addresses {
				accountRows = append(accountRows, observeAccount(rows, address))
			}
			transactionRow["accounts"] = accountRows
			transactionRow["rows"] = rows.exportPhysical()
			transactionRow["latest_rows"] = rows.exportLatest()
		}
		transactions = append(transactions, transactionRow)
	}
	if !concreteObserver || len(specs) == 0 {
		state.Commit()
		root = sink.Commit()
	}
	sink.Close()
	return map[string]any{
		"period": period, "prior_root": hex.EncodeToString(priorRoot[:]), "root": hex.EncodeToString(root[:]),
		"gas_used": cumulativeGasUsed, "execution_mode": executionMode(concreteObserver),
		"transactions": transactions, "rows": rows.exportPhysical(), "latest_rows": rows.exportLatest(),
		"row_model": "PendingBlockState projection: CF3/CF5 keys are logical-hash||big-endian-period; CF1/CF2/CF4 remain content-addressed",
	}, rows, root
}

func observeAccount(rows *memoryRows, address common.Address) map[string]any {
	reader := state_db.ExtendedReader{Reader: rows}
	ret := map[string]any{"address": hex.EncodeToString(address[:]), "present": false, "raw_account": ""}
	reader.GetRawAccount(&address, func(raw []byte) {
		account := state_db.DecodeAccountFromTrie(raw)
		ret["present"] = true
		ret["raw_account"] = hex.EncodeToString(raw)
		ret["nonce"] = account.Nonce.String()
		ret["balance"] = account.Balance.String()
		ret["storage_root"] = hashHex(account.StorageRootHash)
		ret["code_hash"] = hashHex(account.CodeHash)
		ret["code_size"] = account.CodeSize
		ret["code"] = hex.EncodeToString(reader.GetCodeByAddress(&address))
	})
	key := common.Hash{}
	ret["slot_zero_present"] = false
	ret["slot_zero"] = ""
	reader.GetAccountStorage(&address, &key, func(value []byte) {
		ret["slot_zero_present"] = true
		ret["slot_zero"] = hex.EncodeToString(value)
	})
	return ret
}

func requireAccount(observation map[string]any, nonce, balance, code, slot string) {
	if observation["present"] != true || observation["nonce"] != nonce || observation["balance"] != balance ||
		observation["code"] != code || observation["slot_zero"] != slot {
		panic(fmt.Sprintf("unexpected account observation: %#v", observation))
	}
}

func executionMode(concreteObserver bool) string {
	if concreteObserver {
		return "concrete_observer_per_transaction_intermediate_root"
	}
	return "legacy_period_batched_root"
}

func jsonEqual(left, right any) bool {
	leftJSON, err := json.Marshal(left)
	must(err)
	rightJSON, err := json.Marshal(right)
	must(err)
	return bytes.Equal(leftJSON, rightJSON)
}

func compareExecutionModes(concrete, legacy map[string]any) map[string]any {
	if concrete["root"] != legacy["root"] || concrete["gas_used"] != legacy["gas_used"] ||
		!jsonEqual(concrete["accounts"], legacy["accounts"]) {
		panic(fmt.Sprintf(
			"concrete observer and legacy batch final semantics disagree: roots %v/%v gas %v/%v accounts=%t",
			concrete["root"], legacy["root"], concrete["gas_used"], legacy["gas_used"],
			jsonEqual(concrete["accounts"], legacy["accounts"]),
		))
	}
	concreteTransactions := concrete["transactions"].([]map[string]any)
	legacyTransactions := legacy["transactions"].([]map[string]any)
	if len(concreteTransactions) != len(legacyTransactions) {
		panic("concrete observer and legacy batch transaction counts disagree")
	}
	for index := range concreteTransactions {
		for _, field := range []string{
			"signed_rlp", "hash", "state_api_transaction_rlp", "state_api_execution_result_rlp", "receipt_rlp",
			"status", "gas_used", "cumulative_gas_used", "refund_before_transaction_commit", "output", "created",
			"logs", "code_error", "consensus_error", "error",
		} {
			if !jsonEqual(concreteTransactions[index][field], legacyTransactions[index][field]) {
				panic(fmt.Sprintf("execution modes disagree at transaction %d field %s", index, field))
			}
		}
	}
	concreteRows := concrete["rows"].([]map[string]string)
	legacyRows := legacy["rows"].([]map[string]string)
	concreteLatest := concrete["latest_rows"].([]map[string]string)
	legacyLatest := legacy["latest_rows"].([]map[string]string)
	for column := range concreteRows {
		if column != int(state_db.COL_main_trie_node) && !jsonEqual(concreteRows[column], legacyRows[column]) {
			panic(fmt.Sprintf("execution modes unexpectedly differ in concrete column %d", column+1))
		}
		if column != int(state_db.COL_main_trie_node) && !jsonEqual(concreteLatest[column], legacyLatest[column]) {
			panic(fmt.Sprintf("execution modes unexpectedly differ in latest column %d", column+1))
		}
	}
	additionalMainNodes := make([]string, 0)
	for key := range concreteRows[state_db.COL_main_trie_node] {
		if _, present := legacyRows[state_db.COL_main_trie_node][key]; !present {
			additionalMainNodes = append(additionalMainNodes, key)
		}
	}
	for key, legacyValue := range legacyRows[state_db.COL_main_trie_node] {
		concreteValue, present := concreteRows[state_db.COL_main_trie_node][key]
		if !present {
			panic("legacy batch contains a CF2 node absent from concrete observer mode")
		}
		if concreteValue != legacyValue {
			panic("execution modes disagree on a shared physical CF2 node")
		}
	}
	for key, legacyValue := range legacyLatest[state_db.COL_main_trie_node] {
		concreteValue, present := concreteLatest[state_db.COL_main_trie_node][key]
		if !present {
			panic("legacy batch contains a latest CF2 node absent from concrete observer mode")
		}
		if concreteValue != legacyValue {
			panic("execution modes disagree on a shared latest CF2 node")
		}
	}
	sort.Strings(additionalMainNodes)
	return map[string]any{
		"final_semantics_equal": true, "transaction_results_equal": true,
		"latest_rows_differ_only_by_concrete_intermediate_cf2_nodes":   len(additionalMainNodes) != 0,
		"physical_rows_differ_only_by_concrete_intermediate_cf2_nodes": len(additionalMainNodes) != 0,
		"additional_concrete_cf2_keys":                                 additionalMainNodes,
		"concrete_cf2_row_count":                                       len(concreteRows[state_db.COL_main_trie_node]),
		"legacy_cf2_row_count":                                         len(legacyRows[state_db.COL_main_trie_node]),
	}
}

func main() {
	sender := testSender()
	expectedSender := common.HexToAddress("0x1a642f0e3c3af545e7acbd38b07251b3990914f1")
	if sender != expectedSender {
		panic(fmt.Sprintf("fixed key produced %x, want %x", sender, expectedSender))
	}
	created := crypto.CreateAddress(&sender, big.NewInt(1))
	genesisRows, genesisRoot := seedGenesis(sender)
	periodOneSpecs := []transactionSpec{
		{Name: "transfer", Nonce: 0, To: &recipient, Value: 7},
		{Name: "create", Nonce: 1, Input: initCode},
	}
	periodTwoSpecs := []transactionSpec{
		{Name: "call", Nonce: 2, To: &created, Input: append(make([]byte, 31), 2)},
	}

	periodOne, periodOneRows, periodOneRoot := executePeriod(genesisRows, genesisRoot, 1, sender, periodOneSpecs, true)
	legacyPeriodOne, legacyPeriodOneRows, legacyPeriodOneRoot := executePeriod(genesisRows, genesisRoot, 1, sender, periodOneSpecs, false)
	periodOneTransactions := periodOne["transactions"].([]map[string]any)
	if periodOneTransactions[0]["gas_used"] != uint64(21000) || periodOneTransactions[1]["gas_used"] != uint64(75532) ||
		periodOneTransactions[1]["output"] != hex.EncodeToString(runtimeCode) ||
		periodOneTransactions[1]["created"] != hex.EncodeToString(created[:]) {
		panic(fmt.Sprintf("unexpected period-one execution: %#v", periodOneTransactions))
	}
	periodOneAccounts := []map[string]any{
		observeAccount(periodOneRows, sender), observeAccount(periodOneRows, recipient), observeAccount(periodOneRows, created),
	}
	requireAccount(periodOneAccounts[0], "2", "999993", "", "")
	requireAccount(periodOneAccounts[1], "0", "7", "", "")
	requireAccount(periodOneAccounts[2], "1", "0", hex.EncodeToString(runtimeCode), "01")
	periodOne["accounts"] = periodOneAccounts
	legacyPeriodOne["accounts"] = []map[string]any{
		observeAccount(legacyPeriodOneRows, sender), observeAccount(legacyPeriodOneRows, recipient), observeAccount(legacyPeriodOneRows, created),
	}
	serializedRows := periodOneRows.serialize()
	reloadedRows := rowsFromJSON(serializedRows)
	reencodedRows := reloadedRows.serialize()
	if !bytes.Equal(serializedRows, reencodedRows) {
		panic("period-one concrete rows changed across JSON reload")
	}
	periodTwo, periodTwoRows, _ := executePeriod(reloadedRows, periodOneRoot, 2, sender, periodTwoSpecs, true)
	legacyPeriodTwo, legacyPeriodTwoRows, _ := executePeriod(legacyPeriodOneRows, legacyPeriodOneRoot, 2, sender, periodTwoSpecs, false)
	periodTwoTransactions := periodTwo["transactions"].([]map[string]any)
	if periodTwoTransactions[0]["gas_used"] != uint64(26201) || periodTwoTransactions[0]["output"] != "" {
		panic(fmt.Sprintf("unexpected period-two execution: %#v", periodTwoTransactions[0]))
	}
	periodTwoAccounts := []map[string]any{
		observeAccount(periodTwoRows, sender), observeAccount(periodTwoRows, recipient), observeAccount(periodTwoRows, created),
	}
	requireAccount(periodTwoAccounts[0], "3", "999993", "", "")
	requireAccount(periodTwoAccounts[1], "0", "7", "", "")
	requireAccount(periodTwoAccounts[2], "1", "0", hex.EncodeToString(runtimeCode), "02")
	periodTwo["accounts"] = periodTwoAccounts
	legacyPeriodTwo["accounts"] = []map[string]any{
		observeAccount(legacyPeriodTwoRows, sender), observeAccount(legacyPeriodTwoRows, recipient), observeAccount(legacyPeriodTwoRows, created),
	}
	modeComparison := []map[string]any{
		compareExecutionModes(periodOne, legacyPeriodOne), compareExecutionModes(periodTwo, legacyPeriodTwo),
	}
	if modeComparison[0]["physical_rows_differ_only_by_concrete_intermediate_cf2_nodes"] != true {
		panic("period one did not retain the expected concrete intermediate CF2 nodes")
	}

	rowsDigest := sha256.Sum256(serializedRows)
	out := map[string]any{
		"schema": 1,
		"scope":  "synthetic two-period concrete StateAPI execution, FinalChain receipt encoding, and exact PendingBlockState physical-row projection; no application headers, rewards, RocksDB open, or production routing",
		"configuration": map[string]any{
			"chain_id": chainID, "transaction_gas_limit": transactionGas, "block_gas_limit": blockGas,
			"gas_price": "0", "magnolia_activation_period": 0, "aspen_part_one_activation_period": 0,
			"aspen_part_two_activation_period": "18446744073709551615", "cornus_activation_period": 0,
			"cacti_activation_period": "18446744073709551615", "rules": []string{"Magnolia", "AspenPartOne", "Cornus"},
		},
		"inputs": map[string]any{
			"private_key": hex.EncodeToString(testPrivateKey), "sender": hex.EncodeToString(sender[:]),
			"sender_genesis_nonce": "0", "sender_genesis_balance": "1000000",
			"recipient": hex.EncodeToString(recipient[:]), "created": hex.EncodeToString(created[:]),
			"runtime_code": hex.EncodeToString(runtimeCode), "init_code": hex.EncodeToString(initCode),
		},
		"concrete_columns": []string{"CF1/code", "CF2/main_trie_node", "CF3/main_trie_value(versioned)", "CF4/account_trie_node", "CF5/account_trie_value(versioned)"},
		"row_model":        "TrieSink writes to a memory PendingBlockState model. latest_rows are its logical read view; rows project exact RocksDB keys, adding an 8-byte big-endian period suffix to CF3 and CF5. No RocksDB was opened.",
		"mode_limits":      "Concrete observer and legacy batching equivalence is asserted only for this transfer/CREATE/CALL fixture. It does not establish raw/native same-block parity or general pending-tombstone read behavior.",
		"genesis": map[string]any{
			"root": hex.EncodeToString(genesisRoot[:]), "rows": genesisRows.exportPhysical(),
			"latest_rows": genesisRows.exportLatest(), "sender": observeAccount(genesisRows, sender),
		},
		"periods": []map[string]any{periodOne, periodTwo},
		"legacy_batch_baseline": map[string]any{
			"scope":   "Pinned existing-network cache lifetime: one TransitionState across each period with trie preparation at period end",
			"periods": []map[string]any{legacyPeriodOne, legacyPeriodTwo},
		},
		"mode_comparison": modeComparison,
		"continuation": map[string]any{
			"format":                      "canonical JSON object containing logical latest rows, projected physical history, and current period",
			"period_one_serialized_bytes": len(serializedRows), "period_one_serialized_sha256": hex.EncodeToString(rowsDigest[:]),
			"roundtrip_equal": true, "period_two_opened_from_reloaded_rows": true,
		},
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(out))
}

func addressHex(address *common.Address) any {
	if address == nil {
		return nil
	}
	return hex.EncodeToString(address[:])
}

func hashHex(hash *common.Hash) any {
	if hash == nil {
		return nil
	}
	return hex.EncodeToString(hash[:])
}

func mustHex(value string) []byte {
	ret, err := hex.DecodeString(value)
	must(err)
	return ret
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}
