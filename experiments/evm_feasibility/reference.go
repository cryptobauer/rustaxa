// Isolated synthetic fixture exporter. Run only through reference.py, which pins
// the source tree. Inputs below are complete in-memory states, not network data.
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
	"math/big"
	"os"
)

type input struct {
	nonce, balance *big.Int
	missing        bool
}

func (i input) GetCode(*common.Hash) []byte { panic("unexpected code read") }
func (i input) GetAccount(a *common.Address, cb func(state_db.Account)) {
	if !i.missing && *a == address {
		cb(state_db.Account{Nonce: new(big.Int).Set(i.nonce), Balance: new(big.Int).Set(i.balance)})
	}
}
func (input) GetAccountStorage(*common.Address, *common.Hash, func([]byte)) {
	panic("unexpected storage read")
}

var address = common.BytesToAddress([]byte{0xaa})
var target = common.BytesToAddress([]byte{0xbb})

func state(i input) *state_evm.TransitionState {
	s := new(state_evm.TransitionState)
	s.Init(state_evm.Opts{})
	s.SetInput(i)
	return s
}
func hx(b []byte) string { return hex.EncodeToString(b) }
func integer(s string) *big.Int {
	n, ok := new(big.Int).SetString(s, 0)
	if !ok {
		panic(s)
	}
	return n
}

type output struct{ Changes []map[string]any }

func (o *output) StartMutation(*common.Address) state_evm.AccountMutation { return o }
func (o *output) Delete(*common.Address) {
	o.Changes = append(o.Changes, map[string]any{"delete": true})
}
func (o *output) Commit() {}
func (o *output) Update(c state_evm.AccountChange) {
	raw := map[string]string{}
	ordinary := map[string]string{}
	for k, v := range c.RawStorageDirty {
		raw[hx(k[:])] = hx(v)
	}
	for k, v := range c.StorageDirty {
		ordinary[k.Int().String()] = v.String()
	}
	o.Changes = append(o.Changes, map[string]any{"raw": raw, "ordinary": ordinary, "nonce": c.Nonce.String(), "balance": c.Balance.String()})
}

// Memory IO captures actual Taraxa persisted nodes separately from hash leaves.
// Any unexpected read fails, so fixtures cannot silently use missing state.
type mem struct{ Values, Nodes map[string]string }

func (m *mem) GetValue(k *common.Hash, cb func([]byte)) {
	v, ok := m.Values[hx(k[:])]
	if !ok {
		panic("missing value")
	}
	b, _ := hex.DecodeString(v)
	cb(b)
}
func (m *mem) GetNode(k *common.Hash, cb func([]byte)) {
	v, ok := m.Nodes[hx(k[:])]
	if !ok {
		panic("missing node")
	}
	b, _ := hex.DecodeString(v)
	cb(b)
}
func (m *mem) PutValue(k *common.Hash, v []byte) { m.Values[hx(k[:])] = hx(v) }
func (m *mem) PutNode(k *common.Hash, v []byte)  { m.Nodes[hx(k[:])] = hx(v) }
func main() {
	result := map[string]any{}
	var envelopes []map[string]any
	for _, cornus := range []bool{false, true} {
		for _, c := range []struct {
			name, nonce, balance, price string
			gas                         uint64
			create                      bool
			code                        []byte
		}{
			{"skip-u64", "0x10000000000000000", "1000000", "1", 60000, false, nil},
			{"successor-u256", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "1000000", "1", 60000, false, nil},
			{"create-wide", "0x10000000000000000", "1000000", "1", 60000, true, []byte{0}},
			{"create-revert", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", "1000000", "1", 60000, true, []byte{0x60, 0, 0x60, 0, 0xfd}},
			{"stale", "0", "1000000", "3", 60000, false, nil},
			{"affordability", "7", "100", "3", 60000, false, nil},
			{"intrinsic", "7", "1000000", "1", 20000, false, nil},
			{"wide-price", "7", "100000000000000000000000000000000000000000000000000", "0x100000000000000000000000000000000", 60000, false, nil},
		} {
			s := state(input{big.NewInt(1), integer(c.balance), false})
			var e vm.EVM
			e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, s, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
			e.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: cornus})
			tx := vm.Transaction{From: address, To: &target, Nonce: integer(c.nonce), GasPrice: integer(c.price), Value: big.NewInt(0), Gas: c.gas, Input: c.code}
			if c.create {
				tx.To = nil
			}
			r, err := e.Main(&tx)
			errstr := ""
			if err != nil {
				errstr = err.Error()
			}
			a := s.GetAccountConcrete(&address)
			envelopes = append(envelopes, map[string]any{"case": c.name, "cornus": cornus, "input_nonce": c.nonce, "input_balance": c.balance, "price": c.price, "gas_cap": c.gas, "create": c.create, "code": hx(c.code), "nonce": a.GetNonce().String(), "balance": a.GetBalance().String(), "gas_used": r.GasUsed, "consensus_error": r.ConsensusErr, "execution_error": r.ExecutionErr, "error": errstr, "return": hx(r.CodeRetval), "created": hx(r.NewContractAddr[:])})
		}
	}
	result["envelopes"] = envelopes
	var mutations []map[string]any
	for _, missing := range []bool{false, true} {
		for _, revert := range []bool{false, true} {
			s := state(input{big.NewInt(1), big.NewInt(100), missing})
			snapshot := s.Snapshot()
			a := s.GetAccountConcrete(&address)
			key := common.BytesToHash([]byte{1})
			a.SetState(big.NewInt(1), big.NewInt(0x33))
			a.SetStateRawIrreversibly(&key, []byte{0, 0x44})
			s.SetTransientState(&address, key, common.BytesToHash([]byte{0x55}))
			s.AddLog(vm.LogRecord{Address: address, Data: []byte{0x66}})
			if revert {
				s.RevertToSnapshot(snapshot)
			}
			raw := "absent"
			a.GetRawState(&key, func(v []byte) { raw = hx(v) })
			transient := s.GetTransientState(&address, key)
			row := map[string]any{"missing_prior_account": missing, "revert": revert, "raw_read": raw, "ordinary_read": a.GetState(big.NewInt(1)).String(), "transient": hx(transient[:]), "logs": len(s.GetLogs())}
			out := new(output)
			s.CommitTransaction(out)
			row["flushed"] = out.Changes
			after := s.GetTransientState(&address, key)
			row["transient_after_commit"] = hx(after[:])
			mutations = append(mutations, row)
		}
	}
	result["mutations"] = mutations
	var commitments []map[string]any
	for _, bits := range []uint{0, 64, 256, 264} {
		a := state_db.Account{Nonce: new(big.Int).Lsh(big.NewInt(1), bits), Balance: big.NewInt(100)}
		disk, leaf := a.EncodeForTrie()
		m := &mem{map[string]string{}, map[string]string{}}
		w := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
		k := crypto.Keccak256Hash(address[:])
		w.Put(m, &k, &a)
		root := w.Commit(m)
		commitments = append(commitments, map[string]any{"kind": "account", "nonce": a.Nonce.String(), "disk": hx(disk), "leaf": hx(leaf), "key": hx(k[:]), "root": hx(root[:]), "nodes": m.Nodes})
	}
	for _, size := range []int{1, 28, 29, 30, 31, 32, 55, 56, 80} {
		m := &mem{map[string]string{}, map[string]string{}}
		w := new(trie.Writer).Init(state_db.AccountTrieSchema{}, nil, trie.WriterOpts{})
		leaves := map[string]string{}
		// Adjacent prehashed keys force a 63-nibble extension and tiny child nodes.
		for j := byte(1); j <= 2; j++ {
			k := common.Hash{}
			k[31] = j
			raw := make([]byte, size)
			for n := range raw {
				raw[n] = j
			}
			v := state_db.NewAccStorageTrieValue(raw)
			_, leaf := v.EncodeForTrie()
			w.Put(m, &k, v)
			leaves[hx(k[:])] = hx(leaf)
		}
		root := w.Commit(m)
		commitments = append(commitments, map[string]any{"kind": "slots", "size": size, "leaves": leaves, "root": hx(root[:]), "nodes": m.Nodes, "values": m.Values})
	}
	result["commitments"] = commitments
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(result); err != nil {
		panic(fmt.Sprint(err))
	}
}
