// Ordered TrieSink observer fixtures. Run only through
// ordered_overlay_reference.py, which exports both pinned EVM source trees.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/Taraxa-project/taraxa-evm/taraxa/util/bigutil"
)

type overlayMemory struct {
	mu      sync.Mutex
	columns map[byte]map[string]string
}

func newOverlayMemory() *overlayMemory {
	return &overlayMemory{columns: map[byte]map[string]string{}}
}

func (m *overlayMemory) Get(column byte, key *common.Hash, cb func([]byte)) {
	m.mu.Lock()
	value, present := m.columns[column][hex.EncodeToString(key[:])]
	m.mu.Unlock()
	if !present {
		return
	}
	decoded, err := hex.DecodeString(value)
	if err != nil {
		panic(err)
	}
	cb(decoded)
}

func (m *overlayMemory) Put(column byte, key *common.Hash, value []byte) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.columns[column] == nil {
		m.columns[column] = map[string]string{}
	}
	m.columns[column][hex.EncodeToString(key[:])] = hex.EncodeToString(value)
}

func (m *overlayMemory) snapshot() map[string]map[string]string {
	m.mu.Lock()
	defer m.mu.Unlock()
	columns := map[string]map[string]string{}
	for column, name := range map[byte]string{
		state_db.COL_code:            "code",
		state_db.COL_main_trie_node:  "main_nodes",
		state_db.COL_main_trie_value: "accounts",
		state_db.COL_acc_trie_node:   "storage_nodes",
		state_db.COL_acc_trie_value:  "slots",
	} {
		rows := map[string]string{}
		for key, value := range m.columns[column] {
			rows[key] = value
		}
		columns[name] = rows
	}
	return columns
}

func rootHex(root common.Hash) string { return hex.EncodeToString(root[:]) }

func accountAt(m *overlayMemory, address common.Address) state_db.Account {
	path := crypto.Keccak256Hash(address[:])
	var account state_db.Account
	found := false
	m.Get(state_db.COL_main_trie_value, &path, func(value []byte) {
		if len(value) != 0 {
			account = state_db.DecodeAccountFromTrie(value)
			found = true
		}
	})
	if !found {
		panic("expected live account")
	}
	return account
}

func applyAccount(
	sink *state_transition.TrieSink,
	address common.Address,
	account state_db.Account,
	ordinary state_evm.EVMStorage,
	raw state_evm.RawStorage,
) common.Hash {
	mutation := sink.StartMutation(&address)
	mutation.Update(state_evm.AccountChange{
		Account:         account,
		StorageDirty:    ordinary,
		RawStorageDirty: raw,
	})
	mutation.Commit()
	return sink.Commit()
}

func orderedPutRawDelete() map[string]any {
	memory := newOverlayMemory()
	sink := new(state_transition.TrieSink).Init(nil, state_transition.TrieSinkOpts{})
	defer sink.Close()
	sink.SetIO(memory)
	logical := common.BytesToHash([]byte{1})
	root := applyAccount(
		sink,
		common.BytesToAddress([]byte{0xaa}),
		state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(100)},
		state_evm.EVMStorage{bigutil.UnsignedStr(bigutil.UnsafeUnsignedBytes(big.NewInt(1))): big.NewInt(1)},
		state_evm.RawStorage{logical: nil},
	)
	return map[string]any{
		"logical_key": hex.EncodeToString(logical[:]),
		"root":        rootHex(root),
		"columns":     memory.snapshot(),
	}
}

func deleteRecreate() []map[string]any {
	memory := newOverlayMemory()
	sink := new(state_transition.TrieSink).Init(nil, state_transition.TrieSinkOpts{})
	defer sink.Close()
	sink.SetIO(memory)
	address := common.BytesToAddress([]byte{0xaa})
	keyOne := common.BytesToHash([]byte{1})
	keyTwo := common.BytesToHash([]byte{2})
	var phases []map[string]any

	root := applyAccount(
		sink,
		address,
		state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(100)},
		state_evm.EVMStorage{bigutil.UnsignedStr(bigutil.UnsafeUnsignedBytes(big.NewInt(1))): big.NewInt(0x11)},
		nil,
	)
	phases = append(phases, map[string]any{"phase": "seed-slot", "root": rootHex(root), "columns": memory.snapshot()})

	sink.Delete(&address)
	root = sink.Commit()
	phases = append(phases, map[string]any{"phase": "delete-account", "root": rootHex(root), "columns": memory.snapshot()})

	root = applyAccount(
		sink,
		address,
		state_db.Account{Nonce: big.NewInt(2), Balance: big.NewInt(90)},
		nil,
		nil,
	)
	phases = append(phases, map[string]any{"phase": "recreate-nil-root", "root": rootHex(root), "columns": memory.snapshot()})

	root = applyAccount(
		sink,
		address,
		accountAt(memory, address),
		state_evm.EVMStorage{bigutil.UnsignedStr(bigutil.UnsafeUnsignedBytes(big.NewInt(2))): big.NewInt(0x22)},
		nil,
	)
	phases = append(phases, map[string]any{
		"phase": "new-root-retains-orphan", "root": rootHex(root), "columns": memory.snapshot(),
		"orphan_logical_key": hex.EncodeToString(keyOne[:]), "member_logical_key": hex.EncodeToString(keyTwo[:]),
	})
	return phases
}

func main() {
	result := map[string]any{
		"empty_root":             rootHex(state_common.EmptyRLPListHash),
		"ordered_put_raw_delete": orderedPutRawDelete(),
		"delete_recreate":        deleteRecreate(),
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
