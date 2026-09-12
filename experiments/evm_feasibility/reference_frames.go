// Creation frame fixtures use complete in-memory account state and the pinned
// concrete EVM. No block database, native precompile or production adapter runs.
package main

import (
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
	"math/big"
	"sort"
)

type frameInput struct {
	accounts map[common.Address]state_db.Account
	codes    map[common.Hash][]byte
}

func (i frameInput) GetAccount(a *common.Address, cb func(state_db.Account)) {
	if a, ok := i.accounts[*a]; ok {
		a.Nonce = new(big.Int).Set(a.Nonce)
		a.Balance = new(big.Int).Set(a.Balance)
		cb(a)
	}
}
func (i frameInput) GetCode(h *common.Hash) []byte {
	c, ok := i.codes[*h]
	if !ok {
		panic("missing fixture code")
	}
	return c
}
func (i frameInput) GetAccountStorage(*common.Address, *common.Hash, func([]byte)) {
	panic("unexpected frame fixture storage read")
}

// Assemble CODECOPY + CREATE/CREATE2, optionally twice, then expose the final
// stack result through RETURN or REVERT. PUSH1 offsets bound fixture code size.
func parentCode(init []byte, create2, twice, parentRevert, returnData bool) []byte {
	p := []byte{0x60, byte(len(init)), 0x60, 0, 0x60, 0, 0x39}
	emit := func() {
		if create2 {
			p = append(p, 0x60, 1)
		}
		p = append(p, 0x60, byte(len(init)), 0x60, 0, 0x60, 0)
		if create2 {
			p = append(p, 0xf5)
		} else {
			p = append(p, 0xf0)
		}
	}
	emit()
	if twice {
		p = append(p, 0x50)
		emit()
	}
	if returnData {
		p = append(p, 0x50, 0x3d, 0x60, 0, 0x60, 0, 0x3e, 0x3d, 0x60, 0)
	} else {
		p = append(p, 0x60, 0, 0x52, 0x60, 32, 0x60, 0)
	}
	if parentRevert {
		p = append(p, 0xfd)
	} else {
		p = append(p, 0xf3)
	}
	p[3] = byte(len(p))
	return append(p, init...)
}
func creationFrames() []map[string]any {
	var rows []map[string]any
	for _, c := range []struct {
		name, nonce                             string
		init                                    []byte
		create2, twice, collision, parentRevert bool
	}{
		{"create-u64-success", "0x10000000000000000", []byte{0}, false, false, false, false},
		{"create-u256-success", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0}, false, false, false, false},
		{"create-above-u256", "0x1000000000000000000000000000000000000000000000000000000000000000000", []byte{0}, false, false, false, false},
		{"create-collision", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0}, false, false, true, false},
		{"create-child-revert", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0x60, 0xab, 0x60, 0, 0x53, 0x60, 1, 0x60, 0, 0xfd}, false, false, false, false},
		{"create-child-invalid", "0x10000000000000000", []byte{0xfe}, false, false, false, false},
		{"create-parent-revert", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0}, false, false, false, true},
		{"create-grandchild", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0x60, 0, 0x60, 0, 0x60, 0, 0xf0, 0}, false, false, false, false},
		{"create-grandchild-parent-revert", "0x10000000000000000", []byte{0x60, 0, 0x60, 0, 0x60, 0, 0xf0, 0}, false, false, false, true},
		{"create-code-deposit", "0x10000000000000000", []byte{0x60, 0xef, 0x60, 0, 0x53, 0x60, 1, 0x60, 0, 0xf3}, false, false, false, false},
		{"create-grandchild-child-revert", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0x60, 0, 0x60, 0, 0x60, 0, 0xf0, 0x50, 0x60, 0, 0x60, 0, 0xfd}, false, false, false, false},
		{"create-code-deposit-oog", "0x10000000000000000", []byte{0x60, 10, 0x60, 0, 0xf3}, false, false, false, false},
		{"create2-u256-success", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0}, true, false, false, false},
		{"create2-second-collision", "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff", []byte{0}, true, true, false, false},
		{"create2-child-revert", "0x10000000000000000", []byte{0x60, 0, 0x60, 0, 0xfd}, true, false, false, false},
		{"create2-parent-revert", "0x10000000000000000", []byte{0}, true, false, false, true},
	} {
		code := parentCode(c.init, c.create2, c.twice, c.parentRevert, c.name == "create-child-revert")
		gasCap := uint64(300000)
		if c.name == "create-code-deposit-oog" {
			gasCap = 54500
		}
		h := crypto.Keccak256Hash(code)
		in := frameInput{map[common.Address]state_db.Account{
			address: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)},
			target:  {Nonce: integer(c.nonce), Balance: big.NewInt(0), CodeHash: &h, CodeSize: uint64(len(code))},
		}, map[common.Hash][]byte{h: code}}
		child := crypto.CreateAddress(&target, integer(c.nonce))
		if c.create2 {
			salt := common.BytesToHash([]byte{1})
			child = crypto.CreateAddress2(&target, &salt, crypto.Keccak256(c.init))
		}
		grandchild := crypto.CreateAddress(&child, big.NewInt(1))
		if c.collision {
			in.accounts[child] = state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(0)}
		}
		var s state_evm.TransitionState
		s.Init(state_evm.Opts{})
		s.SetInput(in)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &s, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true})
		r, err := e.Main(&vm.Transaction{From: address, To: &target, Nonce: big.NewInt(1), GasPrice: big.NewInt(1), Value: big.NewInt(0), Gas: gasCap})
		// The allowed bytecode creates only these identities; enumerate complete state
		// for the independent account-root calculation (including absent identities).
		candidates := []common.Address{address, target, child, grandchild}
		sort.Slice(candidates, func(i, j int) bool { return string(candidates[i][:]) < string(candidates[j][:]) })
		accounts := map[string]any{}
		m := &mem{map[string]string{}, map[string]string{}}
		w := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
		for _, addr := range candidates {
			a := s.GetAccountConcrete(&addr)
			if a.IsNIL() {
				accounts[hx(addr[:])] = nil
				continue
			}
			disk, leaf := a.Account.EncodeForTrie()
			accounts[hx(addr[:])] = map[string]any{"nonce": a.GetNonce().String(), "balance": a.GetBalance().String(), "code": hx(a.GetCode()), "disk": hx(disk), "leaf": hx(leaf)}
			key := crypto.Keccak256Hash(addr[:])
			w.Put(m, &key, &a.Account)
		}
		root := w.Commit(m)
		errtext := ""
		if err != nil {
			errtext = err.Error()
		}
		rows = append(rows, map[string]any{"case": c.name, "gas_cap": gasCap, "parent_nonce": c.nonce, "init": hx(c.init), "parent_code": hx(code), "create2": c.create2, "twice": c.twice, "collision": c.collision, "parent_revert": c.parentRevert, "child": hx(child[:]), "grandchild": hx(grandchild[:]), "gas_used": r.GasUsed, "return": hx(r.CodeRetval), "execution_error": string(r.ExecutionErr), "consensus_error": string(r.ConsensusErr), "error": errtext, "accounts": accounts, "root": hx(root[:])})
	}
	return rows
}
