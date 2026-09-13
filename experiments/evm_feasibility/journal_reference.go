// Independent journal contract oracle. Run only through journal_reference.py
// against its pinned Go exports. Uses real TransitionState and TrieSink; the
// thread-safe memory adapter records physical rows, not a replacement journal.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
)

type memoryRows struct {
	mu   sync.Mutex
	rows [state_db.COL_COUNT]map[common.Hash][]byte
}

func newMemoryRows() *memoryRows {
	m := new(memoryRows)
	for i := range m.rows {
		m.rows[i] = make(map[common.Hash][]byte)
	}
	return m
}
func (m *memoryRows) Get(col state_db.Column, key *common.Hash, cb func([]byte)) {
	m.mu.Lock()
	v := common.CopyBytes(m.rows[col][*key])
	m.mu.Unlock()
	if len(v) != 0 {
		cb(v)
	}
}
func (m *memoryRows) Put(col state_db.Column, key *common.Hash, value []byte) {
	m.mu.Lock()
	m.rows[col][*key] = common.CopyBytes(value)
	m.mu.Unlock()
}
func (m *memoryRows) export() []map[string]string {
	m.mu.Lock()
	defer m.mu.Unlock()
	out := make([]map[string]string, len(m.rows))
	for col, rows := range m.rows {
		out[col] = make(map[string]string)
		for k, v := range rows {
			out[col][hex.EncodeToString(k[:])] = hex.EncodeToString(v)
		}
	}
	return out
}

var address = common.BytesToAddress([]byte{0xaa})
var key = common.BytesToHash([]byte{1})

// Build canonical prior physical state using the actual reference trie writers.
func seed(m *memoryRows, exists bool) *common.Hash {
	if !exists {
		return nil
	}
	slot := new(trie.Writer).Init(state_db.AccountTrieSchema{}, nil, trie.WriterOpts{})
	slotKey := crypto.Keccak256Hash(key[:])
	slotIO := state_db.AccountTrieIOAdapter{Addr: &address, ReadWriter: m}
	slot.Put(slotIO, &slotKey, state_db.NewAccStorageTrieValue([]byte{0x11}))
	acc := state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(100), StorageRootHash: slot.Commit(slotIO)}
	main := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
	addrKey := crypto.Keccak256Hash(address[:])
	mainIO := state_db.MainTrieIOAdapter{ReadWriter: m}
	main.Put(mainIO, &addrKey, &acc)
	return main.Commit(mainIO)
}

func observe(s *state_evm.TransitionState) map[string]any {
	a := s.GetAccountConcrete(&address)
	raw := map[string]any{"present": false, "bytes": ""}
	a.GetRawState(&key, func(v []byte) { raw["present"], raw["bytes"] = true, hex.EncodeToString(v) })
	t := s.GetTransientState(&address, key)
	return map[string]any{
		"exists": !a.IsNIL(), "nonce": a.GetNonce().String(), "balance": a.GetBalance().String(),
		"ordinary": a.GetState(big.NewInt(1)).String(), "original": a.GetCommittedState(big.NewInt(1)).String(),
		"raw": raw, "transient": hex.EncodeToString(t[:]), "logs": len(s.GetLogs()), "refund": s.GetRefund(),
	}
}

func main() {
	if len(os.Args) == 2 && os.Args[1] == "mutators" {
		runMutators()
		return
	}
	if len(os.Args) == 2 && os.Args[1] == "extended" {
		runExtended()
		return
	}
	var out []map[string]any
	for _, c := range []struct {
		name                    string
		exists, revert, keepNew bool
		raw                     []byte
	}{
		{"existing-commit", true, false, false, []byte{0, 0x44}},
		{"existing-revert", true, true, false, []byte{0, 0x44}},
		{"new-empty-commit", false, false, false, []byte{0, 0x44}},
		{"new-empty-revert", false, true, false, []byte{0, 0x44}},
		{"new-nonempty-commit", false, false, true, []byte{0, 0x44}},
		{"new-nonempty-revert", false, true, true, []byte{0, 0x44}},
		{"raw-delete-commit", true, false, false, nil},
		{"raw-delete-revert", true, true, false, nil},
		{"raw-wide-commit", true, false, false, append([]byte{1}, make([]byte, 40)...)},
		{"raw-wide-revert", true, true, false, append([]byte{1}, make([]byte, 40)...)},
	} {
		m := newMemoryRows()
		priorRoot := seed(m, c.exists)
		priorRootValue := crypto.Keccak256Hash([]byte{0x80})
		if priorRoot != nil {
			priorRootValue = *priorRoot
		}
		priorRows := m.export()
		var s state_evm.TransitionState
		s.Init(state_evm.Opts{})
		s.SetInput(state_db.ExtendedReader{Reader: m})
		before := observe(&s)
		checkpoint := s.Snapshot()
		a := s.GetAccountConcrete(&address)
		if c.keepNew {
			a.SetNonce(big.NewInt(1))
		}
		a.SetState(big.NewInt(1), big.NewInt(0x33))
		a.SetStateRawIrreversibly(&key, c.raw)
		s.SetTransientState(&address, key, common.BytesToHash([]byte{0x55}))
		s.AddLog(vm.LogRecord{Address: address, Data: []byte{0x66}})
		s.AddRefund(123)
		afterWrites := observe(&s)
		if c.revert {
			s.RevertToSnapshot(checkpoint)
		}
		afterFrame := observe(&s)
		sink := new(state_transition.TrieSink).Init(priorRoot, state_transition.TrieSinkOpts{})
		sink.SetIO(m)
		s.CommitTransaction(sink)
		// TrieSink writes asynchronously until Commit. Do not observe physical
		// account/slot rows during that interval; only transaction-local resets.
		transient := s.GetTransientState(&address, key)
		afterTransaction := map[string]any{
			"transient": hex.EncodeToString(transient[:]), "logs": len(s.GetLogs()), "refund": s.GetRefund(),
		}
		s.Commit()
		root := sink.Commit()
		sink.Close()
		var reopened state_evm.TransitionState
		reopened.Init(state_evm.Opts{})
		reopened.SetInput(state_db.ExtendedReader{Reader: m})
		out = append(out, map[string]any{
			"case": c.name, "exists": c.exists, "revert": c.revert, "keep_new": c.keepNew,
			"ordinary_write": "51", "raw_write": hex.EncodeToString(c.raw),
			"before": before, "after_writes": afterWrites, "after_frame": afterFrame,
			"after_transaction": afterTransaction, "reopened": observe(&reopened),
			"prior_rows": priorRows, "prior_root": hex.EncodeToString(priorRootValue[:]),
			"rows": m.export(), "root": hex.EncodeToString(root[:]),
		})
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}

// Extended observations are additive: reverse write order, nested rollback and
// an existing nil-storage-root account with a retained physical raw row. No
// physical reads occur between transaction flush and the final sink join.
func runExtended() {
	var out []map[string]any
	for _, c := range []struct {
		name                                             string
		exists, nested, revertOuter, nilRoot, orphanRoot bool
	}{
		{"reverse-existing-commit", true, false, false, false, false},
		{"reverse-existing-revert", true, false, true, false, false},
		{"reverse-new-commit", false, false, false, false, false},
		{"reverse-new-revert", false, false, true, false, false},
		{"nested-existing-commit", true, true, false, false, false},
		{"nested-existing-revert", true, true, true, false, false},
		{"nested-new-commit", false, true, false, false, false},
		{"nested-new-revert", false, true, true, false, false},
		{"nil-root-commit", true, false, false, true, false},
		{"nil-root-revert", true, false, true, true, false},
		{"orphan-nonnil-root-commit", true, false, false, false, true},
		{"orphan-nonnil-root-revert", true, false, true, false, true},
	} {
		m := newMemoryRows()
		priorRoot := seed(m, c.exists)
		if c.nilRoot || c.orphanRoot {
			// Leave the physical slot row in place but remove account reachability.
			acc := state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(100)}
			if c.orphanRoot {
				// A new storage body references only slot 2; slot 1's old physical
				// row remains readable by GetState and GetRawState alike.
				slot := new(trie.Writer).Init(state_db.AccountTrieSchema{}, nil, trie.WriterOpts{})
				otherKey := common.BytesToHash([]byte{2})
				otherPath := crypto.Keccak256Hash(otherKey[:])
				slotIO := state_db.AccountTrieIOAdapter{Addr: &address, ReadWriter: m}
				slot.Put(slotIO, &otherPath, state_db.NewAccStorageTrieValue([]byte{0x22}))
				acc.StorageRootHash = slot.Commit(slotIO)
			}
			main := new(trie.Writer).Init(state_db.MainTrieSchema{}, priorRoot, trie.WriterOpts{})
			addrKey := crypto.Keccak256Hash(address[:])
			mainIO := state_db.MainTrieIOAdapter{ReadWriter: m}
			main.Put(mainIO, &addrKey, &acc)
			priorRoot = main.Commit(mainIO)
		}
		priorRootValue := crypto.Keccak256Hash([]byte{0x80})
		if priorRoot != nil {
			priorRootValue = *priorRoot
		}
		priorRows := m.export()
		var s state_evm.TransitionState
		s.Init(state_evm.Opts{})
		s.SetInput(state_db.ExtendedReader{Reader: m})
		before := observe(&s)
		var steps []map[string]any
		record := func(op string, value any) {
			steps = append(steps, map[string]any{"op": op, "value": value, "view": observe(&s)})
		}
		outer := s.Snapshot()
		record("snapshot", 0)
		if !c.exists {
			s.GetAccountConcrete(&address).SetNonce(big.NewInt(1))
			record("nonce", "1")
		}
		s.GetAccountConcrete(&address).SetStateRawIrreversibly(&key, []byte{0, 0x44})
		record("raw", "0044")
		s.GetAccountConcrete(&address).SetState(big.NewInt(1), big.NewInt(0x33))
		record("ordinary", "51")
		s.SetTransientState(&address, key, common.BytesToHash([]byte{0x55}))
		record("transient", "55")
		s.AddRefund(123)
		record("refund", 123)
		s.AddLog(vm.LogRecord{Address: address, Data: []byte{0x66}})
		record("log", "66")
		if c.nested {
			inner := s.Snapshot()
			record("snapshot", 1)
			s.GetAccountConcrete(&address).SetState(big.NewInt(1), big.NewInt(0x77))
			record("ordinary", "119")
			s.GetAccountConcrete(&address).SetStateRawIrreversibly(&key, []byte{0, 0x88})
			record("raw", "0088")
			s.SetTransientState(&address, key, common.BytesToHash([]byte{0x99}))
			record("transient", "99")
			s.AddRefund(10)
			record("refund", 10)
			s.AddLog(vm.LogRecord{Address: address, Data: []byte{0xaa}})
			record("log", "aa")
			s.RevertToSnapshot(inner)
			record("revert", 1)
		}
		if c.revertOuter {
			s.RevertToSnapshot(outer)
			record("revert", 0)
		}
		sink := new(state_transition.TrieSink).Init(priorRoot, state_transition.TrieSinkOpts{})
		sink.SetIO(m)
		s.CommitTransaction(sink)
		transient := s.GetTransientState(&address, key)
		afterTransaction := map[string]any{"transient": hex.EncodeToString(transient[:]), "logs": len(s.GetLogs()), "refund": s.GetRefund()}
		s.Commit()
		root := sink.Commit()
		sink.Close()
		var reopened state_evm.TransitionState
		reopened.Init(state_evm.Opts{})
		reopened.SetInput(state_db.ExtendedReader{Reader: m})
		out = append(out, map[string]any{
			"case": c.name, "exists": c.exists, "nil_root": c.nilRoot, "orphan_root": c.orphanRoot,
			"before": before, "steps": steps, "after_transaction": afterTransaction,
			"reopened": observe(&reopened), "prior_rows": priorRows,
			"prior_root": hex.EncodeToString(priorRootValue[:]), "rows": m.export(), "root": hex.EncodeToString(root[:]),
		})
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}

// Mutator regressions exercise real account methods, including no-op behavior.
// The deliberately rejected nonce is recovered as an observation; no Go source
// assertion or validation is weakened to obtain the fixture.
func runMutators() {
	var out []map[string]any
	for _, name := range []string{"storage-noop-leading-zero", "empty-by-balance", "existing-empty-touch", "existing-empty-raw", "empty-code-noop", "reverted-new", "nonce-decrease", "ripemd-touch-revert", "empty-nonce-raw-revert"} {
		address = common.BytesToAddress([]byte{0xaa})
		if name == "ripemd-touch-revert" {
			address = common.BytesToAddress([]byte{3})
		}
		m := newMemoryRows()
		var priorRoot *common.Hash
		if name != "reverted-new" {
			acc := state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(100)}
			if name == "empty-by-balance" {
				acc.Nonce, acc.Balance = big.NewInt(0), big.NewInt(1)
			}
			if name == "existing-empty-touch" || name == "existing-empty-raw" || name == "ripemd-touch-revert" || name == "empty-nonce-raw-revert" {
				acc.Nonce, acc.Balance = big.NewInt(0), big.NewInt(0)
			}
			if name == "storage-noop-leading-zero" {
				slot := new(trie.Writer).Init(state_db.AccountTrieSchema{}, nil, trie.WriterOpts{})
				slotKey := crypto.Keccak256Hash(key[:])
				slotIO := state_db.AccountTrieIOAdapter{Addr: &address, ReadWriter: m}
				slot.Put(slotIO, &slotKey, state_db.NewAccStorageTrieValue([]byte{0, 0x11}))
				acc.StorageRootHash = slot.Commit(slotIO)
			}
			if name == "empty-code-noop" {
				code := []byte{0x60, 0}
				hash := crypto.Keccak256Hash(code)
				acc.CodeHash, acc.CodeSize = &hash, uint64(len(code))
				m.Put(state_db.COL_code, &hash, code)
			}
			main := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
			addrKey := crypto.Keccak256Hash(address[:])
			mainIO := state_db.MainTrieIOAdapter{ReadWriter: m}
			main.Put(mainIO, &addrKey, &acc)
			priorRoot = main.Commit(mainIO)
		}
		priorRootValue := crypto.Keccak256Hash([]byte{0x80})
		if priorRoot != nil {
			priorRootValue = *priorRoot
		}
		priorRows := m.export()
		var s state_evm.TransitionState
		s.Init(state_evm.Opts{})
		s.SetInput(state_db.ExtendedReader{Reader: m})
		view := func(state *state_evm.TransitionState) map[string]any {
			v := observe(state)
			v["code"] = hex.EncodeToString(state.GetAccountConcrete(&address).GetCode())
			return v
		}
		before := view(&s)
		checkpoint := s.Snapshot()
		a := s.GetAccountConcrete(&address)
		panicked := false
		func() {
			defer func() {
				if cause := recover(); cause != nil {
					if name != "nonce-decrease" {
						panic(cause)
					}
					panicked = true
				}
			}()
			switch name {
			case "storage-noop-leading-zero":
				a.SetState(big.NewInt(1), big.NewInt(0x11))
			case "empty-by-balance":
				a.SubBalance(big.NewInt(1))
			case "existing-empty-touch", "ripemd-touch-revert":
				a.AddBalance(big.NewInt(0))
			case "existing-empty-raw":
				a.SetStateRawIrreversibly(&key, []byte{0x44})
			case "empty-code-noop":
				a.SetCode(nil)
			case "reverted-new":
				a.SetNonce(big.NewInt(1))
			case "nonce-decrease":
				a.SetNonce(big.NewInt(0))
			case "empty-nonce-raw-revert":
				a.SetNonce(big.NewInt(1))
				a.SetStateRawIrreversibly(&key, []byte{0x44})
			}
		}()
		if name == "nonce-decrease" && !panicked {
			panic("expected decreasing nonce to be rejected")
		}
		afterMutation := view(&s)
		if name == "reverted-new" || name == "ripemd-touch-revert" || name == "empty-nonce-raw-revert" {
			s.RevertToSnapshot(checkpoint)
		}
		afterFrame := view(&s)
		sink := new(state_transition.TrieSink).Init(priorRoot, state_transition.TrieSinkOpts{})
		sink.SetIO(m)
		s.CommitTransaction(sink)
		s.Commit()
		root := sink.Commit()
		sink.Close()
		var reopened state_evm.TransitionState
		reopened.Init(state_evm.Opts{})
		reopened.SetInput(state_db.ExtendedReader{Reader: m})
		out = append(out, map[string]any{
			"case": name, "address": hex.EncodeToString(address[:]), "before": before,
			"after_mutation": afterMutation, "after_frame": afterFrame, "panicked": panicked,
			"prior_rows": priorRows, "prior_root": hex.EncodeToString(priorRootValue[:]),
			"rows": m.export(), "root": hex.EncodeToString(root[:]), "reopened": view(&reopened),
		})
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
