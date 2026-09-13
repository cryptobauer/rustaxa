// Initial mixed-period witness using the pinned Go StateTransition and TrieSink.
// Run through mixed_period_reference.py so public batching and the local
// concrete observer are built independently. The memory database preserves all
// five concrete column families, version history, and empty tombstones.
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"math/big"
	"os"
	"sort"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

const (
	chainID        = uint64(841)
	transactionGas = uint64(100000)
	blockGas       = uint64(1000000)
	maxPeriod      = ^uint64(0)
)

var (
	testPrivateKey = bytes.Repeat([]byte{0x01}, 32)
	recipient      = common.HexToAddress("0x2222222222222222222222222222222222222222")
	validator      = common.HexToAddress("0x0000000000000000000000000000000000000031")
	delegator      = common.HexToAddress("0x0000000000000000000000000000000000000032")
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
	if present {
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

type memoryPending struct {
	rows   *memoryRows
	number types.BlockNum
}

func (p *memoryPending) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	p.rows.Get(column, key, callback)
}

func (p *memoryPending) Put(column state_db.Column, key *common.Hash, value []byte) {
	p.rows.Put(column, key, value)
}

func (p *memoryPending) GetNumber() types.BlockNum { return p.number }

type memoryLatest struct {
	rows       *memoryRows
	descriptor state_db.StateDescriptor
	pending    *memoryPending
}

func newMemoryLatest() *memoryLatest {
	return &memoryLatest{
		rows: newMemoryRows(),
		descriptor: state_db.StateDescriptor{
			BlockNum:  types.BlockNumberNIL,
			StateRoot: common.ZeroHash,
		},
	}
}

func (m *memoryLatest) GetCommittedDescriptor() state_db.StateDescriptor { return m.descriptor }

func (m *memoryLatest) BeginPendingBlock() state_db.PendingBlockState {
	number := m.descriptor.BlockNum + 1
	m.rows.setPeriod(uint64(number))
	m.pending = &memoryPending{rows: m.rows, number: number}
	return m.pending
}

func (m *memoryLatest) Commit(root common.Hash) error {
	if m.pending == nil {
		return fmt.Errorf("memory latest commit has no pending block")
	}
	m.descriptor = state_db.StateDescriptor{BlockNum: m.pending.number, StateRoot: root}
	m.pending = nil
	return nil
}

type historicalRows struct {
	latest *memoryLatest
	period types.BlockNum
}

func (h historicalRows) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	if h.latest.descriptor.BlockNum == types.BlockNumberNIL {
		return
	}
	if h.period > h.latest.descriptor.BlockNum {
		panic(fmt.Sprintf("historical read %d exceeds committed %d", h.period, h.latest.descriptor.BlockNum))
	}
	h.latest.rows.mu.Lock()
	if column != state_db.COL_main_trie_value && column != state_db.COL_acc_trie_value {
		value, present := h.latest.rows.latest[column][*key]
		value = common.CopyBytes(value)
		h.latest.rows.mu.Unlock()
		if present && len(value) != 0 {
			callback(value)
		}
		return
	}
	var selected []byte
	var selectedPeriod uint64
	found := false
	for physicalKey, value := range h.latest.rows.physical[column] {
		bytesKey := []byte(physicalKey)
		if len(bytesKey) != 40 || !bytes.Equal(bytesKey[:32], key[:]) {
			continue
		}
		period := binary.BigEndian.Uint64(bytesKey[32:])
		if period <= uint64(h.period) && (!found || period > selectedPeriod) {
			selected = common.CopyBytes(value)
			selectedPeriod = period
			found = true
		}
	}
	h.latest.rows.mu.Unlock()
	if found && len(selected) != 0 {
		callback(selected)
	}
}

func (m *memoryLatest) readerAt(period types.BlockNum) state_db.Reader {
	return historicalRows{latest: m, period: period}
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
	Name     string
	Nonce    uint64
	GasPrice uint64
	Gas      uint64
	To       *common.Address
	Value    uint64
	Input    []byte
}

type signedTransaction struct {
	RLP     []byte
	Hash    common.Hash
	Decoded signedLegacyTransaction
	Sender  common.Address
	ChainID uint64
}

func signTransaction(spec transactionSpec, sender common.Address) signedTransaction {
	unsigned := unsignedLegacyTransaction{
		Nonce: new(big.Int).SetUint64(spec.Nonce), GasPrice: new(big.Int).SetUint64(spec.GasPrice), Gas: spec.Gas,
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
	return decodeSignedTransaction(encoded)
}

func decodeSignedTransaction(encoded []byte) signedTransaction {
	var decoded signedLegacyTransaction
	must(rlp.DecodeBytes(encoded, &decoded))
	if decoded.V == nil || decoded.V.Cmp(big.NewInt(35)) < 0 {
		panic("signed legacy transaction has no EIP-155 chain ID")
	}
	vBase := new(big.Int).Sub(new(big.Int).Set(decoded.V), big.NewInt(35))
	recoveryID := new(big.Int).And(new(big.Int).Set(vBase), big.NewInt(1)).Uint64()
	decodedChainID := new(big.Int).Rsh(vBase, 1)
	if !decodedChainID.IsUint64() {
		panic("signed legacy transaction chain ID exceeds uint64")
	}
	unsigned := unsignedLegacyTransaction{
		Nonce: decoded.Nonce, GasPrice: decoded.GasPrice, Gas: decoded.Gas, To: decoded.To,
		Value: decoded.Value, Input: decoded.Input, ChainID: decodedChainID, ZeroR: new(big.Int), ZeroS: new(big.Int),
	}
	digest := crypto.Keccak256Hash(rlp.MustEncodeToBytes(&unsigned))
	signature := make([]byte, 65)
	decoded.R.FillBytes(signature[:32])
	decoded.S.FillBytes(signature[32:64])
	signature[64] = byte(recoveryID)
	publicKey, err := secp256k1.RecoverPubkey(digest[:], signature)
	must(err)
	sender := common.BytesToAddress(crypto.Keccak256(publicKey[1:])[12:])
	return signedTransaction{
		RLP: common.CopyBytes(encoded), Hash: crypto.Keccak256Hash(encoded), Decoded: decoded,
		Sender: sender, ChainID: decodedChainID.Uint64(),
	}
}

func testSender() common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(testPrivateKey)
	publicKey := secp256k1.S256().Marshal(x, y)
	return common.BytesToAddress(crypto.Keccak256(publicKey[1:])[12:])
}

type catalogSlot struct {
	Address common.Address `json:"address"`
	Key     common.Hash    `json:"key"`
}

type catalogIdentities struct {
	Accounts    []common.Address `json:"accounts"`
	Slots       []catalogSlot    `json:"slots"`
	Invocations any              `json:"invocations"`
}

func jsonValue(value any) any {
	encoded, err := json.Marshal(value)
	must(err)
	var ret any
	must(json.Unmarshal(encoded, &ret))
	return ret
}

func witnessConfig(sender common.Address) chain_config.ChainConfig {
	return chain_config.ChainConfig{
		EVMChainConfig: params.ChainConfig{ChainId: chainID},
		GenesisBalances: core.BalanceMap{
			sender:    big.NewInt(1_000_000),
			delegator: big.NewInt(2_000),
		},
		DPOS: chain_config.DPOSConfig{
			EligibilityBalanceThreshold: big.NewInt(100),
			VoteEligibilityBalanceStep:  big.NewInt(10),
			ValidatorMaximumStake:       big.NewInt(1_000_000),
			MinimumDeposit:              big.NewInt(1),
			MaxBlockAuthorReward:        10,
			DagProposersReward:          50,
			CommissionChangeDelta:       0,
			CommissionChangeFrequency:   0,
			DelegationDelay:             1,
			DelegationLockingPeriod:     1,
			BlocksPerYear:               1,
			YieldPercentage:             20,
			InitialValidators: []chain_config.GenesisValidator{{
				Address: validator, Owner: sender, VrfKey: bytes.Repeat([]byte{0x44}, 32), Commission: 0,
				Endpoint: "", Description: "", Delegations: core.BalanceMap{delegator: big.NewInt(1_000)},
			}},
		},
		Hardforks: chain_config.HardforksConfig{
			FixRedelegateBlockNum:        0,
			RewardsDistributionFrequency: map[uint64]uint32{0: 1},
			MagnoliaHf:                   chain_config.MagnoliaHfConfig{BlockNum: 0, JailTime: 1},
			PhalaenopsisHfBlockNum:       0,
			FixClaimAllBlockNum:          0,
			AspenHf: chain_config.AspenHfConfig{
				BlockNumPartOne: 0, BlockNumPartTwo: maxPeriod,
				MaxSupply: big.NewInt(1_200_000), GeneratedRewards: new(big.Int),
			},
			FicusHf: chain_config.FicusHfConfig{BlockNum: 0, PillarBlocksInterval: 1_000},
			CornusHf: chain_config.CornusHfConfig{
				BlockNum: 0, DelegationLockingPeriod: 1, DagGasLimit: blockGas, PbftGasLimit: blockGas,
			},
			SoleiroliaHf: chain_config.SoleiroliaHfConfig{BlockNum: maxPeriod},
			CactiHf: chain_config.CactiHfConfig{
				BlockNum: maxPeriod, DelegationLockingPeriod: 1, JailTime: 1,
			},
		},
	}
}

func newStateTransition(latest *memoryLatest, cfg *chain_config.ChainConfig) *state_transition.StateTransition {
	dposAPI := new(dpos.API).Init(*cfg)
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(period)}
	}
	getDposReader := func(period types.BlockNum) dpos.Reader {
		return dposAPI.NewDelayedReader(period, storageFactory)
	}
	getSlashingReader := func(period types.BlockNum) slashing.Reader {
		return dposAPI.NewSlashingReader(period, storageFactory)
	}
	return new(state_transition.StateTransition).Init(
		latest,
		func(types.BlockNum) *big.Int { return new(big.Int) },
		dposAPI,
		getDposReader,
		getSlashingReader,
		cfg,
		state_transition.Opts{
			EVMState: state_evm.Opts{NumTransactionsToBuffer: 1},
			Trie:     state_transition.TrieSinkOpts{},
		},
	)
}

func mergeCatalogs(catalogs ...catalogIdentities) catalogIdentities {
	accounts := make(map[common.Address]struct{})
	slots := make(map[catalogSlot]struct{})
	var invocations []any
	for _, catalog := range catalogs {
		for _, address := range catalog.Accounts {
			accounts[address] = struct{}{}
		}
		for _, slot := range catalog.Slots {
			slots[slot] = struct{}{}
		}
		if values, ok := catalog.Invocations.([]any); ok {
			invocations = append(invocations, values...)
		}
	}
	ret := catalogIdentities{Invocations: invocations}
	for address := range accounts {
		ret.Accounts = append(ret.Accounts, address)
	}
	sort.Slice(ret.Accounts, func(i, j int) bool { return string(ret.Accounts[i][:]) < string(ret.Accounts[j][:]) })
	for slot := range slots {
		ret.Slots = append(ret.Slots, slot)
	}
	sort.Slice(ret.Slots, func(i, j int) bool {
		if ret.Slots[i].Address != ret.Slots[j].Address {
			return string(ret.Slots[i].Address[:]) < string(ret.Slots[j].Address[:])
		}
		return string(ret.Slots[i].Key[:]) < string(ret.Slots[j].Key[:])
	})
	return ret
}

func captureCatalog(reader state_db.ExtendedReader, identities catalogIdentities) map[string]any {
	accounts := make([]map[string]any, len(identities.Accounts))
	for index, address := range identities.Accounts {
		accounts[index] = observeAccountReader(reader, address)
	}
	slots := make([]map[string]any, len(identities.Slots))
	for index, slot := range identities.Slots {
		hashedPath := crypto.Keccak256Hash(slot.Key[:])
		row := map[string]any{
			"address": hex.EncodeToString(slot.Address[:]), "key": hex.EncodeToString(slot.Key[:]),
			"hashed_trie_path": hex.EncodeToString(hashedPath[:]), "present": false, "value": "",
		}
		reader.GetAccountStorage(&slot.Address, &slot.Key, func(value []byte) {
			row["present"] = true
			row["value"] = hex.EncodeToString(value)
		})
		slots[index] = row
	}
	return map[string]any{"accounts": accounts, "slots": slots, "invocations": identities.Invocations}
}

func captureNativeCatalog(reader state_db.ExtendedReader, identities catalogIdentities) map[string]any {
	catalog := captureCatalog(reader, identities)
	live := make(map[common.Hash][]byte)
	address := *dpos.ContractAddress()
	reader.ForEachStorage(&address, func(path *common.Hash, value []byte) {
		live[*path] = common.CopyBytes(value)
	})
	covered := make(map[common.Hash][]byte)
	absentSlots := make([]map[string]string, 0)
	for _, slot := range identities.Slots {
		present := false
		reader.GetAccountStorage(&slot.Address, &slot.Key, func(value []byte) {
			present = true
			if slot.Address == address {
				covered[crypto.Keccak256Hash(slot.Key[:])] = common.CopyBytes(value)
			}
		})
		if !present {
			absentSlots = append(absentSlots, map[string]string{
				"address": hex.EncodeToString(slot.Address[:]), "key": hex.EncodeToString(slot.Key[:]),
			})
		}
	}
	for path, value := range live {
		if !bytes.Equal(covered[path], value) {
			panic(fmt.Sprintf("native catalog does not cover live DPoS path %x", path))
		}
	}
	for path, value := range covered {
		if !bytes.Equal(live[path], value) {
			panic(fmt.Sprintf("native catalog has unmatched live DPoS path %x", path))
		}
	}
	absentAccounts := make([]string, 0)
	for _, account := range identities.Accounts {
		present := false
		reader.GetRawAccount(&account, func([]byte) { present = true })
		if !present {
			absentAccounts = append(absentAccounts, hex.EncodeToString(account[:]))
		}
	}
	catalog["coverage"] = map[string]any{
		"all_live_dpos_storage_covered": true,
		"live_dpos_storage_rows":        len(live),
		"covered_dpos_catalog_rows":     len(covered),
		"absent_account_identities":     absentAccounts,
		"absent_slot_identities":        absentSlots,
	}
	return catalog
}

func nativeStorageByPath(reader state_db.ExtendedReader) []map[string]string {
	address := *dpos.ContractAddress()
	var ret []map[string]string
	reader.ForEachStorage(&address, func(path *common.Hash, value []byte) {
		ret = append(ret, map[string]string{"hashed_trie_path": hex.EncodeToString(path[:]), "value": hex.EncodeToString(value)})
	})
	sort.Slice(ret, func(i, j int) bool { return ret[i]["hashed_trie_path"] < ret[j]["hashed_trie_path"] })
	return ret
}

func sameAddress(left, right *common.Address) bool {
	if left == nil || right == nil {
		return left == nil && right == nil
	}
	return *left == *right
}

func transactionFromSigned(spec transactionSpec, signed signedTransaction, expectedSender common.Address) vm.Transaction {
	decoded := signed.Decoded
	if signed.ChainID != chainID || signed.Sender != expectedSender ||
		!decoded.Nonce.IsUint64() || decoded.Nonce.Uint64() != spec.Nonce ||
		!decoded.GasPrice.IsUint64() || decoded.GasPrice.Uint64() != spec.GasPrice ||
		decoded.Gas != spec.Gas || !sameAddress(decoded.To, spec.To) ||
		!decoded.Value.IsUint64() || decoded.Value.Uint64() != spec.Value ||
		!bytes.Equal(decoded.Input, spec.Input) {
		panic("decoded signed transaction does not match the immutable witness facts")
	}
	return vm.Transaction{
		From: signed.Sender, To: decoded.To, Nonce: new(big.Int).Set(decoded.Nonce),
		GasPrice: new(big.Int).Set(decoded.GasPrice), Value: new(big.Int).Set(decoded.Value),
		Gas: decoded.Gas, Input: common.CopyBytes(decoded.Input),
	}
}

func transactionRow(spec transactionSpec, signed signedTransaction, tx vm.Transaction, result vm.ExecutionResult, refund uint64) map[string]any {
	if result.ExecutionErr != "" || result.ConsensusErr != "" {
		panic(fmt.Sprintf("%s failed: execution=%q consensus=%q", spec.Name, result.ExecutionErr, result.ConsensusErr))
	}
	if tx.From != signed.Sender || !sameAddress(tx.To, signed.Decoded.To) || tx.Nonce.Cmp(signed.Decoded.Nonce) != 0 ||
		tx.GasPrice.Cmp(signed.Decoded.GasPrice) != 0 || tx.Gas != signed.Decoded.Gas ||
		tx.Value.Cmp(signed.Decoded.Value) != 0 || !bytes.Equal(tx.Input, signed.Decoded.Input) {
		panic("StateAPI transaction does not match the decoded signed transaction")
	}
	logs := make([]map[string]any, 0, len(result.Logs))
	for _, log := range result.Logs {
		topics := make([]string, len(log.Topics))
		for index, topic := range log.Topics {
			topics[index] = hex.EncodeToString(topic[:])
		}
		logs = append(logs, map[string]any{
			"address": hex.EncodeToString(log.Address[:]), "topics": topics, "data": hex.EncodeToString(log.Data),
		})
	}
	receipt := externalReceipt{Status: 1, GasUsed: result.GasUsed, CumulativeGasUsed: result.GasUsed, Logs: result.Logs}
	return map[string]any{
		"index": 0, "name": spec.Name, "nonce": spec.Nonce, "to": addressHex(spec.To),
		"sender": hex.EncodeToString(signed.Sender[:]), "chain_id": signed.ChainID,
		"value": signed.Decoded.Value.String(), "gas_price": signed.Decoded.GasPrice.String(),
		"gas_limit": signed.Decoded.Gas, "input": hex.EncodeToString(signed.Decoded.Input),
		"signed_rlp": hex.EncodeToString(signed.RLP), "hash": hex.EncodeToString(signed.Hash[:]),
		"v": signed.Decoded.V.String(), "r": signed.Decoded.R.String(), "s": signed.Decoded.S.String(),
		"state_api_transaction_rlp":      hex.EncodeToString(rlp.MustEncodeToBytes(&tx)),
		"state_api_execution_result_rlp": hex.EncodeToString(rlp.MustEncodeToBytes(&result)),
		"receipt_rlp":                    hex.EncodeToString(rlp.MustEncodeToBytes(&receipt)),
		"status":                         1, "gas_used": result.GasUsed, "refund_after_transaction_commit": refund,
		"output": hex.EncodeToString(result.CodeRetval), "created": hex.EncodeToString(result.NewContractAddr[:]),
		"logs": logs, "execution_error": string(result.ExecutionErr), "consensus_error": string(result.ConsensusErr),
	}
}

func runWitness(mode string) map[string]any {
	observer := mode == "observer"
	if mode != "batched" && !observer {
		panic("mode must be batched or observer")
	}
	if observer && !observerAvailable() {
		panic("observer mode requested from a pin without the concrete observer API")
	}
	sender := testSender()
	expectedSender := common.HexToAddress("0x1a642f0e3c3af545e7acbd38b07251b3990914f1")
	if sender != expectedSender {
		panic(fmt.Sprintf("fixed key produced %x, want %x", sender, expectedSender))
	}
	cfg := witnessConfig(sender)
	latest := newMemoryLatest()
	st := newStateTransition(latest, &cfg)
	defer st.Close()
	genesisDescriptor := latest.GetCommittedDescriptor()
	if genesisDescriptor.BlockNum != 0 || genesisDescriptor.StateRoot == common.ZeroHash {
		panic(fmt.Sprintf("invalid committed genesis descriptor: %#v", genesisDescriptor))
	}
	genesisReader := state_db.ExtendedReader{Reader: latest.readerAt(0)}
	genesis := map[string]any{
		"period": 0, "root": hex.EncodeToString(genesisDescriptor.StateRoot[:]),
		"rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(),
		"accounts": observeAccounts(genesisReader), "native_storage_by_hashed_path": nativeStorageByPath(genesisReader),
	}
	var genesisIdentities catalogIdentities
	if observer {
		identities, reader := observerGenesisCatalog(st)
		genesisIdentities = identities
		genesis["native_catalog"] = captureNativeCatalog(reader, identities)
	}
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(period)}
	}
	genesisDposReader := new(dpos.API).Init(cfg).NewReader(0, storageFactory)
	eligibleVotes := genesisDposReader.TotalEligibleVoteCount()
	validatorEligibleVotes := genesisDposReader.GetEligibleVoteCount(&validator)
	if eligibleVotes != 100 || validatorEligibleVotes != 100 {
		panic(fmt.Sprintf("unexpected genesis eligible votes total/validator: %d/%d", eligibleVotes, validatorEligibleVotes))
	}

	st.BeginBlock(&vm.BlockInfo{Author: validator, GasLimit: blockGas, Difficulty: new(big.Int)})
	spec := transactionSpec{Name: "transfer", Nonce: 0, GasPrice: 1, Gas: transactionGas, To: &recipient, Value: 7}
	signed := signTransaction(spec, sender)
	tx := transactionFromSigned(spec, signed, sender)
	result := st.ExecuteTransaction(&tx)
	refund := st.GetEvmState().GetRefund()
	transaction := transactionRow(spec, signed, tx, result, refund)
	fee := new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed), tx.GasPrice)
	if result.GasUsed != 21_000 || fee.Cmp(big.NewInt(21_000)) != 0 {
		panic(fmt.Sprintf("unexpected transfer gas/fee: %d/%s", result.GasUsed, fee))
	}
	var transactionIdentityJSON any
	var postTransaction any
	if observer {
		root, identities, reader := observerFinalizeTransaction(st, 0)
		transactionIdentityJSON = jsonValue(identities)
		postTransaction = map[string]any{
			"root": hex.EncodeToString(root[:]), "catalog": captureCatalog(reader, identities),
			"rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(),
		}
	}

	const committeeSize = uint64(1)
	maxVotesWeight := committeeSize
	if eligibleVotes < maxVotesWeight {
		maxVotesWeight = eligibleVotes
	}
	stats := rewards_stats.RewardsStats{
		BlockAuthor: validator, BlocksPerYear: 1,
		ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{
			validator: {DagBlocksCount: 1, VoteWeight: 1, FeesRewards: new(big.Int).Set(fee)},
		},
		TotalDagBlocksCount: 1, TotalVotesWeight: 1, MaxVotesWeight: maxVotesWeight,
	}
	minted := st.DistributeRewards(&stats)
	if minted == nil || minted.ToBig().Cmp(big.NewInt(200)) != 0 {
		panic(fmt.Sprintf("unexpected minted reward: %v", minted))
	}
	st.EndBlock()
	if observer {
		observerRecordRewards(st)
	}
	preparedRoot := st.PrepareCommit()
	var periodIdentityJSON any
	var finalCatalog any
	if observer {
		identities, reader := observerPeriodCatalog(st)
		periodIdentityJSON = jsonValue(identities)
		full := mergeCatalogs(genesisIdentities, identities)
		finalCatalog = captureNativeCatalog(reader, full)
	}
	committedRoot := st.Commit()
	if committedRoot != preparedRoot {
		panic("prepared and committed roots differ")
	}
	finalReader := state_db.ExtendedReader{Reader: latest.readerAt(1)}
	final := map[string]any{
		"root": hex.EncodeToString(preparedRoot[:]), "committed_root": hex.EncodeToString(committedRoot[:]),
		"rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(),
		"accounts": observeAccounts(finalReader), "native_storage_by_hashed_path": nativeStorageByPath(finalReader),
	}
	if observer {
		final["native_catalog"] = finalCatalog
	}
	return map[string]any{
		"schema":         1,
		"scope":          "synthetic StateTransition genesis and one reward-bearing period using incremental TrieSink roots; no application header, RocksDB, production route, or reconstructed-root claim",
		"execution_mode": mode,
		"configuration":  configurationJSON(cfg, committeeSize),
		"inputs": map[string]any{
			"private_key": hex.EncodeToString(testPrivateKey), "sender": hex.EncodeToString(sender[:]),
			"recipient": hex.EncodeToString(recipient[:]), "validator": hex.EncodeToString(validator[:]),
			"delegator": hex.EncodeToString(delegator[:]),
			"genesis_allocations": []map[string]string{
				{"address": hex.EncodeToString(sender[:]), "balance": "1000000"},
				{"address": hex.EncodeToString(delegator[:]), "balance": "2000"},
			},
			"initial_validator": map[string]any{
				"address": hex.EncodeToString(validator[:]), "owner": hex.EncodeToString(sender[:]),
				"vrf_key": hex.EncodeToString(bytes.Repeat([]byte{0x44}, 32)), "commission": 0,
				"delegations": []map[string]string{{"delegator": hex.EncodeToString(delegator[:]), "amount": "1000"}},
			},
		},
		"genesis": genesis,
		"period": map[string]any{
			"number": 1, "author": hex.EncodeToString(validator[:]), "transaction": transaction,
			"post_transaction": postTransaction,
			"reward_input": map[string]any{
				"block_author": hex.EncodeToString(validator[:]), "blocks_per_year": 1,
				"committee_size": committeeSize, "eligible_vote_count": eligibleVotes,
				"validator_eligible_vote_count": validatorEligibleVotes,
				"certificate_votes":             []map[string]any{{"validator": hex.EncodeToString(validator[:]), "weight": 1}},
				"validators": []map[string]any{{
					"validator": hex.EncodeToString(validator[:]), "dag_blocks_count": 1,
					"vote_weight": 1, "fees_reward": fee.String(),
				}},
				"total_dag_blocks_count": 1, "total_votes_weight": 1, "max_votes_weight": maxVotesWeight,
			},
			"planner_facts": map[string]any{
				"dag_blocks": []map[string]any{{
					"author": hex.EncodeToString(validator[:]), "difficulty": "1",
					"transaction_hashes": []string{hex.EncodeToString(signed.Hash[:])},
				}},
				"certificate_votes": []map[string]any{{
					"validator": hex.EncodeToString(validator[:]), "period": 1, "weight": 1,
				}},
				"eligible_validator_count": 1, "total_eligible_vote_count": eligibleVotes,
				"validator_eligible_vote_count": validatorEligibleVotes,
			},
			"reward_output": map[string]any{
				"actual_transaction_fee": fee.String(), "minted_reward": minted.ToBig().String(),
				"expected_fixed_yield_formula": "1000*20/(100*1)=200",
			},
			"transaction_catalog_identities": transactionIdentityJSON,
			"period_catalog_identities":      periodIdentityJSON,
			"final":                          final,
		},
		"concrete_columns": []string{"CF1/code", "CF2/main_trie_node", "CF3/main_trie_value(versioned)", "CF4/account_trie_node", "CF5/account_trie_value(versioned)"},
		"row_model":        "memory LatestState and PendingBlockState preserve exact TrieSink writes; CF3/CF5 keys append big-endian period; pending empty values remain visible while historical tombstones suppress callbacks",
		"observer_api": map[string]any{
			"available": observerAvailable(),
			"scope":     "local bb0ab67 observer methods only; public 6c7e533 batched mode has no concrete catalog or intermediate-root API",
		},
	}
}

func configurationJSON(cfg chain_config.ChainConfig, committeeSize uint64) map[string]any {
	return map[string]any{
		"chain_id": chainID, "transaction_gas_limit": transactionGas, "block_gas_limit": blockGas,
		"dpos": map[string]any{
			"eligibility_balance_threshold":   cfg.DPOS.EligibilityBalanceThreshold.String(),
			"vote_eligibility_balance_step":   cfg.DPOS.VoteEligibilityBalanceStep.String(),
			"validator_maximum_stake":         cfg.DPOS.ValidatorMaximumStake.String(),
			"minimum_deposit":                 cfg.DPOS.MinimumDeposit.String(),
			"max_block_author_reward_percent": cfg.DPOS.MaxBlockAuthorReward,
			"dag_proposers_reward_percent":    cfg.DPOS.DagProposersReward,
			"commission_change_delta":         cfg.DPOS.CommissionChangeDelta,
			"commission_change_frequency":     cfg.DPOS.CommissionChangeFrequency,
			"delegation_delay":                cfg.DPOS.DelegationDelay,
			"delegation_locking_period":       cfg.DPOS.DelegationLockingPeriod,
			"blocks_per_year":                 cfg.DPOS.BlocksPerYear,
			"yield_percentage":                cfg.DPOS.YieldPercentage,
		},
		"committee_size": committeeSize,
		"hardforks": map[string]any{
			"fix_redelegate_block":           cfg.Hardforks.FixRedelegateBlockNum,
			"redelegations":                  []any{},
			"rewards_distribution_frequency": []map[string]any{{"start_period": 0, "frequency": 1}},
			"magnolia":                       map[string]any{"block": cfg.Hardforks.MagnoliaHf.BlockNum, "jail_time": cfg.Hardforks.MagnoliaHf.JailTime},
			"phalaenopsis_block":             cfg.Hardforks.PhalaenopsisHfBlockNum,
			"fix_claim_all_block":            cfg.Hardforks.FixClaimAllBlockNum,
			"aspen": map[string]any{
				"part_one_block":    cfg.Hardforks.AspenHf.BlockNumPartOne,
				"part_two_block":    fmt.Sprint(cfg.Hardforks.AspenHf.BlockNumPartTwo),
				"max_supply":        cfg.Hardforks.AspenHf.MaxSupply.String(),
				"generated_rewards": cfg.Hardforks.AspenHf.GeneratedRewards.String(),
			},
			"ficus": map[string]any{
				"block":                   cfg.Hardforks.FicusHf.BlockNum,
				"pillar_blocks_interval":  cfg.Hardforks.FicusHf.PillarBlocksInterval,
				"bridge_contract_address": hex.EncodeToString(cfg.Hardforks.FicusHf.BridgeContractAddress[:]),
			},
			"cornus": map[string]any{
				"block":                     cfg.Hardforks.CornusHf.BlockNum,
				"delegation_locking_period": cfg.Hardforks.CornusHf.DelegationLockingPeriod,
				"dag_gas_limit":             cfg.Hardforks.CornusHf.DagGasLimit,
				"pbft_gas_limit":            cfg.Hardforks.CornusHf.PbftGasLimit,
			},
			"soleirolia": map[string]any{
				"block":                     fmt.Sprint(cfg.Hardforks.SoleiroliaHf.BlockNum),
				"transaction_min_gas_price": cfg.Hardforks.SoleiroliaHf.TrxMinGasPrice,
				"transaction_max_gas_limit": cfg.Hardforks.SoleiroliaHf.TrxMaxGasLimit,
			},
			"cacti": map[string]any{
				"block":      fmt.Sprint(cfg.Hardforks.CactiHf.BlockNum),
				"lambda_min": cfg.Hardforks.CactiHf.LambdaMin, "lambda_max": cfg.Hardforks.CactiHf.LambdaMax,
				"lambda_default":            cfg.Hardforks.CactiHf.LambdaDefault,
				"lambda_change_interval":    cfg.Hardforks.CactiHf.LambdaChangeInterval,
				"lambda_change":             cfg.Hardforks.CactiHf.LambdaChange,
				"block_propagation_min":     cfg.Hardforks.CactiHf.BlockPropagationMin,
				"block_propagation_max":     cfg.Hardforks.CactiHf.BlockPropagationMax,
				"consensus_delay":           cfg.Hardforks.CactiHf.ConsensusDelay,
				"delegation_locking_period": cfg.Hardforks.CactiHf.DelegationLockingPeriod,
				"jail_time":                 cfg.Hardforks.CactiHf.JailTime,
			},
		},
		"active_rules": []string{"Magnolia", "AspenPartOne", "Ficus", "Cornus"},
	}
}

func observeAccountReader(reader state_db.ExtendedReader, address common.Address) map[string]any {
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
	return ret
}

func observeAccounts(reader state_db.ExtendedReader) []map[string]any {
	addresses := []common.Address{testSender(), recipient, validator, delegator, *dpos.ContractAddress()}
	ret := make([]map[string]any, len(addresses))
	for index, address := range addresses {
		ret[index] = observeAccountReader(reader, address)
	}
	return ret
}

func main() {
	mode := flag.String("mode", "batched", "batched or observer")
	flag.Parse()
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(runWitness(*mode)))
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

func must(err error) {
	if err != nil {
		panic(err)
	}
}
