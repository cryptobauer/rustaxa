// Real DPoS setCommission calls from EVM frames, with exact old/new raw rows.
package main

import (
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/trie"
	"math/big"
)

type nativeInput struct {
	frameInput
	raw map[common.Hash][]byte
}

func (i nativeInput) GetAccountStorage(a *common.Address, k *common.Hash, cb func([]byte)) {
	if *a != *dpos.ContractAddress() {
		panic("unexpected native fixture address")
	}
	if v, ok := i.raw[*k]; ok {
		cb(v)
	}
}

type nativeBackend struct {
	storage.EVMStateStorage
	writes []map[string]string
}

func (n *nativeBackend) Put(a *common.Address, k *common.Hash, v []byte) {
	n.writes = append(n.writes, map[string]string{"key": hx(k[:]), "value": hx(v)})
	n.EVMStateStorage.Put(a, k, v)
}
func nativeCalls() []map[string]any {
	var rows []map[string]any
	validator := common.BytesToAddress([]byte{0x31})
	contract := *dpos.ContractAddress()
	for _, c := range []struct {
		name                          string
		revert, wrong, preFix, static bool
		commission                    uint16
	}{
		{"call-success", false, false, false, false, 200}, {"call-parent-revert", true, false, false, false, 200},
		{"call-wrong-owner", false, true, false, false, 200}, {"call-overflow", false, false, false, false, 10001},
		{"call-before-fix", false, false, true, false, 200}, {"staticcall-mutation", false, false, false, true, 200},
	} {
		abi := common.FromHex("f000322c")
		abi = append(abi, make([]byte, 12)...)
		abi = append(abi, validator[:]...)
		word := make([]byte, 32)
		word[30] = byte(c.commission >> 8)
		word[31] = byte(c.commission)
		abi = append(abi, word...)
		code := []byte{0x60, 68, 0x60, 0, 0x60, 0, 0x39, 0x60, 0, 0x60, 0, 0x60, 68, 0x60, 0}
		if !c.static {
			code = append(code, 0x60, 0)
		}
		code = append(code, 0x60, 0xfe, 0x61, 0xc3, 0x50)
		if c.static {
			code = append(code, 0xfa)
		} else {
			code = append(code, 0xf1)
		}
		code = append(code, 0x60, 0, 0x52, 0x60, 32, 0x60, 0)
		if c.revert {
			code = append(code, 0xfd)
		} else {
			code = append(code, 0xf3)
		}
		code[3] = byte(len(code))
		code = append(code, abi...)
		owner := target
		if c.wrong {
			owner = address
		}
		vkey := *storage.Stor_k_1([]byte{0, 0}, validator[:])
		okey := *storage.Stor_k_1([]byte{0, 3}, validator[:])
		old := rlp.MustEncodeToBytes(&dpos.ValidatorV1{TotalStake: big.NewInt(10000), Commission: 100})
		h := crypto.Keccak256Hash(code)
		in := nativeInput{frameInput{map[common.Address]state_db.Account{address: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)}, target: {Nonce: big.NewInt(1), Balance: big.NewInt(0), CodeHash: &h, CodeSize: uint64(len(code))}, contract: {Nonce: big.NewInt(1), Balance: big.NewInt(10000)}}, map[common.Hash][]byte{h: code}}, map[common.Hash][]byte{vkey: old, okey: owner[:]}}
		in.raw[*storage.Stor_k_1([]byte{0, 5, 1})] = []byte{1, 0, 0, 0}
		in.raw[*storage.Stor_k_1([]byte{0, 5, 0}, []byte{1, 0, 0, 0})] = validator[:]
		in.raw[*storage.Stor_k_1([]byte{0, 5, 2}, validator[:])] = []byte{1, 0, 0, 0}
		var s state_evm.TransitionState
		s.Init(state_evm.Opts{})
		s.SetInput(in)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &s, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true, IsMagnolia: true})
		cfg := chain_config.ChainConfig{}
		cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
		if c.preFix {
			cfg.Hardforks.FixRedelegateBlockNum = 2
		}
		backend := &nativeBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &s}}
		native := new(dpos.Contract).Init(cfg, backend, dpos.Reader{}, &e)
		native.Register(e.RegisterPrecompiledContract)
		result, err := e.Main(&vm.Transaction{From: address, To: &target, Nonce: big.NewInt(1), Value: big.NewInt(0), GasPrice: big.NewInt(1), Gas: 100000})
		raw := map[string]string{}
		m := &mem{map[string]string{}, map[string]string{}}
		w := new(trie.Writer).Init(state_db.AccountTrieSchema{}, nil, trie.WriterOpts{})
		for k := range in.raw {
			s.GetAccountConcrete(&contract).GetRawState(&k, func(v []byte) {
				raw[hx(k[:])] = hx(v)
				key := crypto.Keccak256Hash(k[:])
				w.Put(m, &key, state_db.NewAccStorageTrieValue(v))
			})
		}
		storageRoot := w.Commit(m)
		main := new(trie.Writer).Init(state_db.MainTrieSchema{}, nil, trie.WriterOpts{})
		mainIO := &mem{map[string]string{}, map[string]string{}}
		accounts := map[string]any{}
		for _, a := range []common.Address{address, target, contract} {
			acc := s.GetAccountConcrete(&a)
			copy := acc.Account
			if a == contract {
				copy.StorageRootHash = storageRoot
			}
			disk, leaf := copy.EncodeForTrie()
			accounts[hx(a[:])] = map[string]string{"nonce": acc.GetNonce().String(), "balance": acc.GetBalance().String(), "code": hx(acc.GetCode()), "disk": hx(disk), "leaf": hx(leaf)}
			key := crypto.Keccak256Hash(a[:])
			main.Put(mainIO, &key, &copy)
		}
		root := main.Commit(mainIO)
		logs := []map[string]any{}
		for _, l := range result.Logs {
			topics := []string{}
			for _, t := range l.Topics {
				topics = append(topics, hx(t[:]))
			}
			logs = append(logs, map[string]any{"address": hx(l.Address[:]), "topics": topics, "data": hx(l.Data)})
		}
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		prior := map[string]string{}
		for k, v := range in.raw {
			prior[hx(k[:])] = hx(v)
		}
		rows = append(rows, map[string]any{"case": c.name, "code": hx(code), "abi": hx(abi), "owner": hx(owner[:]), "validator": hx(validator[:]), "commission": c.commission, "pre_fix": c.preFix, "parent_revert": c.revert, "static": c.static, "prior_raw": prior, "writes": backend.writes, "raw": raw, "storage_root": hx(storageRoot[:]), "accounts": accounts, "root": hx(root[:]), "gas_used": result.GasUsed, "return": hx(result.CodeRetval), "error": errorText, "logs": logs})
	}
	return rows
}
