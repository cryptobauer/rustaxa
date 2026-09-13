// Pinned Go SELFDESTRUCT oracle: real EVM execution and TransitionState flush.
// The output sink records logical mutations, without inventing trie persistence.
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
)

var sender = common.BytesToAddress([]byte{0xaa})
var target = common.BytesToAddress([]byte{0xbb})
var child = common.BytesToAddress([]byte{0xcc})
var beneficiary = common.BytesToAddress([]byte{0xdd})

type input struct {
	accounts map[common.Address]state_db.Account
	codes    map[common.Hash][]byte
}

func (i input) GetCode(h *common.Hash) []byte {
	c, ok := i.codes[*h]
	if !ok {
		panic("missing code")
	}
	return c
}
func (i input) GetAccount(a *common.Address, cb func(state_db.Account)) {
	if v, ok := i.accounts[*a]; ok {
		cb(v)
	}
}
func (i input) GetAccountStorage(a *common.Address, k *common.Hash, cb func([]byte)) {
	if *a == target && *k == (common.Hash{}) {
		cb([]byte{7})
	}
}

type output struct{ changes map[string]any }
type mutation struct {
	out     *output
	address string
}

func (o *output) StartMutation(a *common.Address) state_evm.AccountMutation {
	return mutation{o, hex.EncodeToString(a[:])}
}
func (o *output) Delete(a *common.Address) {
	o.changes[hex.EncodeToString(a[:])] = map[string]any{"kind": "delete"}
}
func (m mutation) Update(c state_evm.AccountChange) {
	slots := map[string]string{}
	for k, v := range c.StorageDirty {
		slots[hex.EncodeToString([]byte(k))] = v.String()
	}
	raw := map[string]string{}
	for k, v := range c.RawStorageDirty {
		raw[hex.EncodeToString(k[:])] = hex.EncodeToString(v)
	}
	m.out.changes[m.address] = map[string]any{"kind": "upsert", "nonce": c.Nonce.String(), "balance": c.Balance.String(), "code_size": c.CodeSize, "storage": slots, "raw": raw}
}
func (m mutation) Commit()             {}
func suicide(to common.Address) []byte { c := append([]byte{0x73}, to[:]...); return append(c, 0xff) }
func calls(n int, static, revert bool) []byte {
	var code []byte
	for j := 0; j < n; j++ {
		code = append(code, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0)
		if !static {
			code = append(code, 0x60, 0)
		}
		code = append(code, 0x73)
		code = append(code, child[:]...)
		code = append(code, 0x61, 0xff, 0xff)
		if static {
			code = append(code, 0xfa)
		} else {
			code = append(code, 0xf1)
		}
		code = append(code, 0x50)
	}
	if revert {
		return append(code, 0x60, 0, 0x60, 0, 0xfd)
	}
	return append(code, 0)
}
func run(name string, balance *big.Int, dest common.Address, destKind string, gas uint64, parent, childCode []byte) map[string]any {
	in := input{accounts: map[common.Address]state_db.Account{}, codes: map[common.Hash][]byte{}}
	in.accounts[sender] = state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(1000000)}
	code := suicide(dest)
	if name == "stack-underflow" || name == "stack-underflow-no-gas" {
		code = []byte{0xff}
	}
	source := target
	if parent != nil {
		code = parent
		source = child
	}
	seed := func(a common.Address, c []byte, b *big.Int) {
		h := crypto.Keccak256Hash(c)
		root := crypto.EmptyBytesKeccak256
		in.accounts[a] = state_db.Account{Nonce: big.NewInt(1), Balance: new(big.Int).Set(b), CodeHash: &h, CodeSize: uint64(len(c)), StorageRootHash: &root}
		in.codes[h] = c
	}
	seed(target, code, big.NewInt(0))
	if parent != nil {
		seed(child, childCode, balance)
	} else {
		seed(target, code, balance)
	}
	if destKind != "absent" && dest != source {
		nonce := big.NewInt(0)
		if destKind == "nonempty" {
			nonce.SetInt64(1)
		}
		in.accounts[dest] = state_db.Account{Nonce: nonce, Balance: big.NewInt(0)}
	}
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{})
	state.SetInput(in)
	var evm vm.EVM
	evm.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
	evm.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true})
	tx := &vm.Transaction{From: sender, To: &target, Nonce: big.NewInt(1), GasPrice: big.NewInt(1), Value: big.NewInt(0), Gas: gas}
	if name == "create-suicide" {
		tx.To = nil
		tx.Input = code
		tx.Value = big.NewInt(7)
	}
	result, err := evm.Main(tx)
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	visible := map[string]any{}
	addresses := []common.Address{sender, target, child, dest}
	if name == "create-suicide" {
		addresses = append(addresses, result.NewContractAddr)
	}
	for _, a := range addresses {
		acc := state.GetAccountConcrete(&a)
		visible[hex.EncodeToString(a[:])] = map[string]any{"exists": !acc.IsNIL(), "nonce": acc.GetNonce().String(), "balance": acc.GetBalance().String(), "code_size": acc.GetCodeSize()}
	}
	row := map[string]any{"case": name, "balance": balance.String(), "beneficiary": hex.EncodeToString(dest[:]), "beneficiary_kind": destKind, "code": hex.EncodeToString(code), "child_code": hex.EncodeToString(childCode), "gas_limit": gas, "gas_used": result.GasUsed, "refund": state.GetRefund(), "execution_error": result.ExecutionErr, "consensus_error": result.ConsensusErr, "error": errText, "output": hex.EncodeToString(result.CodeRetval), "visible": visible}
	if name == "create-suicide" {
		row["created_address"] = hex.EncodeToString(result.NewContractAddr[:])
	}
	out := output{changes: map[string]any{}}
	state.CommitTransaction(&out)
	row["writes"] = out.changes
	return row
}
func lanes(name string, exists, revert bool) map[string]any {
	in := input{accounts: map[common.Address]state_db.Account{}, codes: map[common.Hash][]byte{}}
	if exists {
		root := crypto.EmptyBytesKeccak256
		in.accounts[target] = state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(7), StorageRootHash: &root}
	}
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{})
	state.SetInput(in)
	checkpoint := state.Snapshot()
	acc := state.GetAccountConcrete(&target)
	key := common.Hash{}
	rawKey := common.BytesToHash([]byte{1})
	transient := common.BytesToHash([]byte{9})
	if exists || name == "new-account-revert" || name == "new-account-recreate" {
		acc.SetState(big.NewInt(0), big.NewInt(8))
		if name != "ordinary-only-revert" && name != "revert-then-nonce" {
			acc.SetStateRawIrreversibly(&rawKey, []byte{0xaa, 0xbb})
		}
		state.SetTransientState(&target, key, transient)
	}
	dest := beneficiary
	if name == "self-revert" {
		dest = target
	}
	acc.Suicide(&dest)
	if revert {
		state.RevertToSnapshot(checkpoint)
	}
	if name == "revert-then-nonce" || name == "new-account-recreate" {
		acc.SetNonce(big.NewInt(2))
	}
	raw := []byte(nil)
	acc.GetRawState(&rawKey, func(v []byte) { raw = v })
	row := map[string]any{"case": name, "beneficiary": hex.EncodeToString(dest[:]), "exists": exists, "revert": revert, "source_exists": !acc.IsNIL(), "source_balance": acc.GetBalance().String(), "beneficiary_exists": !state.GetAccountConcrete(&dest).IsNIL(), "beneficiary_balance": state.GetAccountConcrete(&dest).GetBalance().String(), "storage": acc.GetState(big.NewInt(0)).String(), "raw": hex.EncodeToString(raw), "transient": state.GetTransientState(&target, key).Hex()}
	out := output{changes: map[string]any{}}
	state.CommitTransaction(&out)
	row["writes"] = out.changes
	return row
}

func main() {
	rows := []map[string]any{}
	wide := new(big.Int).Lsh(big.NewInt(1), 256)
	for _, kind := range []string{"absent", "empty", "nonempty"} {
		for _, b := range []*big.Int{big.NewInt(0), big.NewInt(7), wide} {
			rows = append(rows, run(kind+"-"+b.String(), b, beneficiary, kind, 200000, nil, nil))
		}
	}
	rows = append(rows, run("create-suicide", big.NewInt(0), beneficiary, "absent", 200000, nil, nil))
	rows = append(rows, run("stack-underflow", big.NewInt(0), beneficiary, "absent", 200000, nil, nil))
	rows = append(rows, run("stack-underflow-no-gas", big.NewInt(0), beneficiary, "absent", 21000, nil, nil))
	rows = append(rows, run("self-beneficiary", big.NewInt(7), target, "nonempty", 200000, nil, nil))
	rows = append(rows, run("unfunded-new-beneficiary", big.NewInt(7), beneficiary, "absent", 51002, nil, nil))
	ripemd := common.BytesToAddress([]byte{3})
	rows = append(rows, run("unfunded-ripemd-empty", big.NewInt(0), ripemd, "empty", 26002, nil, nil))
	rows = append(rows, run("funded-ripemd-empty", big.NewInt(0), ripemd, "empty", 26003, nil, nil))
	for _, c := range []struct {
		name           string
		n              int
		static, revert bool
	}{{"nested-success", 1, false, false}, {"nested-parent-revert", 1, false, true}, {"repeated-success", 2, false, false}, {"repeated-parent-revert", 2, false, true}, {"static-rejected", 1, true, false}} {
		rows = append(rows, run(c.name, big.NewInt(7), beneficiary, "absent", 200000, calls(c.n, c.static, c.revert), suicide(beneficiary)))
	}
	json.NewEncoder(os.Stdout).Encode(map[string]any{"selfdestruct": rows, "lanes": []map[string]any{lanes("existing-commit", true, false), lanes("existing-revert", true, true), lanes("absent-commit", false, false), lanes("absent-revert", false, true), lanes("ordinary-only-revert", true, true), lanes("revert-then-nonce", true, true), lanes("new-account-revert", false, true), lanes("self-revert", true, true), lanes("new-account-recreate", false, true)}})
}
