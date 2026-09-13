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
