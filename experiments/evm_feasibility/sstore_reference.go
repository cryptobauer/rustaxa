// Pinned Go SSTORE oracle. This uses Taraxa's real TransitionState and EVM;
// it is copied into a disposable git-archived source tree by sstore_reference.py.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var sstoreSender = common.BytesToAddress([]byte{0xaa})
var sstoreTarget = common.BytesToAddress([]byte{0xbb})
var sstoreChild = common.BytesToAddress([]byte{0xcc})

type sstoreInput struct {
	codes map[common.Address][]byte
	slots map[string][]byte
}

func (i sstoreInput) GetCode(hash *common.Hash) []byte {
	for _, code := range i.codes {
		if crypto.Keccak256Hash(code) == *hash {
			return code
		}
	}
	panic("missing seeded code")
}
func (i sstoreInput) GetAccount(address *common.Address, cb func(state_db.Account)) {
	if *address == sstoreSender {
		cb(state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(1000000)})
		return
	}
	if code, ok := i.codes[*address]; ok {
		hash := crypto.Keccak256Hash(code)
		root := crypto.EmptyBytesKeccak256
		cb(state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(0), CodeHash: &hash, CodeSize: uint64(len(code)), StorageRootHash: &root})
	}
}
func (i sstoreInput) GetAccountStorage(_ *common.Address, key *common.Hash, cb func([]byte)) {
	if value, ok := i.slots[hex.EncodeToString(key[:])]; ok {
		cb(value)
	}
}

func sstoreCode(values ...byte) []byte {
	code := make([]byte, 0, len(values)*5+1)
	for _, value := range values {
		code = append(code, 0x60, value, 0x60, 0x00, 0x55)
	}
	return append(code, 0x00)
}
func sstoreState(in sstoreInput) *state_evm.TransitionState {
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{})
	state.SetInput(in)
	return state
}
func sstoreRun(name string, original byte, code []byte, gas uint64) map[string]any {
	in := sstoreInput{codes: map[common.Address][]byte{sstoreTarget: code}, slots: map[string][]byte{}}
	if original != 0 {
		in.slots[hex.EncodeToString(make([]byte, 32))] = []byte{original}
	}
	state := sstoreState(in)
	var evm vm.EVM
	evm.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
	evm.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true})
	result, err := evm.Main(&vm.Transaction{From: sstoreSender, To: &sstoreTarget, Nonce: big.NewInt(1), GasPrice: big.NewInt(1), Value: big.NewInt(0), Gas: gas})
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	row := map[string]any{"case": name, "original": original, "code": hex.EncodeToString(code), "gas_cap": gas, "gas_used": result.GasUsed, "refund": state.GetRefund(), "execution_error": result.ExecutionErr, "consensus_error": result.ConsensusErr, "error": errText, "return": hex.EncodeToString(result.CodeRetval), "storage": state.GetAccountConcrete(&sstoreTarget).GetState(big.NewInt(0)).String()}
	if name == "sentry-2300" || name == "sentry-2301" {
		row["gas_at_sstore"] = gas - 21000 - 6
	}
	return row
}
func sstoreFixtures() []map[string]any {
	rows := []map[string]any{}
	for _, c := range []struct {
		name     string
		original byte
		values   []byte
	}{
		{"clean0-to0", 0, []byte{0}}, {"clean0-to1", 0, []byte{1}},
		{"dirty0-to1-to0", 0, []byte{1, 0}}, {"dirty0-to1-to2", 0, []byte{1, 2}},
		{"clean7-to7", 7, []byte{7}}, {"clean7-to0", 7, []byte{0}}, {"clean7-to8", 7, []byte{8}},
		{"dirty7-to8-to0", 7, []byte{8, 0}}, {"dirty7-to8-to7", 7, []byte{8, 7}},
	} {
		rows = append(rows, sstoreRun(c.name, c.original, sstoreCode(c.values...), 100000))
	}
	rows = append(rows, sstoreRun("sentry-2300", 0, sstoreCode(0), 23306))
	rows = append(rows, sstoreRun("sentry-2301", 0, sstoreCode(0), 23307))
	// STATICCALL to a child SSTORE returns false to its parent and preserves storage.
	child := sstoreCode(1)
	parent := []byte{0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0xcc, 0x61, 0xff, 0xff, 0xfa, 0x60, 0, 0x52, 0x60, 32, 0x60, 0, 0xf3}
	in := sstoreInput{codes: map[common.Address][]byte{sstoreTarget: parent, sstoreChild: child}, slots: map[string][]byte{}}
	state := sstoreState(in)
	var evm vm.EVM
	evm.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
	evm.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true})
	result, err := evm.Main(&vm.Transaction{From: sstoreSender, To: &sstoreTarget, Nonce: big.NewInt(1), GasPrice: big.NewInt(1), Value: big.NewInt(0), Gas: 100000})
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	rows = append(rows, map[string]any{"case": "static-rejection", "original": 0, "code": hex.EncodeToString(parent), "child_code": hex.EncodeToString(child), "gas_cap": 100000, "gas_used": result.GasUsed, "refund": state.GetRefund(), "execution_error": result.ExecutionErr, "consensus_error": result.ConsensusErr, "error": errText, "return": hex.EncodeToString(result.CodeRetval), "storage": state.GetAccountConcrete(&sstoreChild).GetState(big.NewInt(0)).String()})
	return rows
}
func main() { json.NewEncoder(os.Stdout).Encode(map[string]any{"sstore": sstoreFixtures()}) }
