// Persisted trie reopen/update fixtures. The memory IO contains the exact node
// and value-column bytes a database adapter supplies, including tombstones.
package main

import (
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
)

func nodeHistory() []map[string]any {
	m := &mem{map[string]string{}, map[string]string{}}
	var root *common.Hash
	var rows []map[string]any
	for _, op := range []struct {
		key   byte
		value []byte
	}{{1, []byte{1}}, {2, make([]byte, 80)}, {0xf1, []byte{3}}, {1, []byte{0, 4}}, {2, nil}, {1, nil}, {0xf1, nil}, {2, []byte{5}}} {
		w := new(trie.Writer).Init(state_db.AccountTrieSchema{}, root, trie.WriterOpts{})
		k := common.Hash{}
		k[0] = op.key
		k[31] = op.key
		if len(op.value) == 0 {
			w.Delete(m, &k)
		} else {
			w.Put(m, &k, state_db.NewAccStorageTrieValue(op.value))
		}
		root = w.Commit(m)
		hash := state_common.EmptyRLPListHash
		if root != nil {
			hash = *root
		}
		values := map[string]string{}
		for k, v := range m.Values {
			values[k] = v
		}
		nodes := map[string]string{}
		for k, v := range m.Nodes {
			nodes[k] = v
		}
		rows = append(rows, map[string]any{"key": hx(k[:]), "value": hx(op.value), "root": hx(hash[:]), "nodes": nodes, "values": values})
	}
	return rows
}
