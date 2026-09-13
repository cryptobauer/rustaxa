// Pinned Go wide-SSTORE oracle. This uses Taraxa's real TransitionState and
// EVM and is copied into disposable git-archived source trees by
// wide_sstore_reference.py.
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

var wideSstoreSender = common.BytesToAddress([]byte{0xaa})
var wideSstoreTarget = common.BytesToAddress([]byte{0xbb})
var wideSstoreChild = common.BytesToAddress([]byte{0xcc})

type wideSstoreInput struct {
	codes map[common.Address][]byte
	slots map[string][]byte
}

func (i wideSstoreInput) GetCode(hash *common.Hash) []byte {
	for _, code := range i.codes {
		if crypto.Keccak256Hash(code) == *hash {
			return code
		}
	}
	panic("missing seeded code")
}

func (i wideSstoreInput) GetAccount(address *common.Address, cb func(state_db.Account)) {
	if *address == wideSstoreSender {
		cb(state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(1000000)})
		return
	}
	if code, ok := i.codes[*address]; ok {
		hash := crypto.Keccak256Hash(code)
		root := crypto.EmptyBytesKeccak256
		cb(state_db.Account{
			Nonce: big.NewInt(1), Balance: big.NewInt(0), CodeHash: &hash,
			CodeSize: uint64(len(code)), StorageRootHash: &root,
		})
	}
}

func (i wideSstoreInput) GetAccountStorage(address *common.Address, key *common.Hash, cb func([]byte)) {
	if *address != wideSstoreTarget {
		return
	}
	if value, ok := i.slots[hex.EncodeToString(key[:])]; ok {
		cb(value)
	}
}

func wideSstoreCode(values ...byte) []byte {
	code := make([]byte, 0, len(values)*5+1)
	for _, value := range values {
		code = append(code, 0x60, value, 0x60, 0x00, 0x55)
	}
	return append(code, 0x00)
}

func wideSstoreNested(childReverts, parentReverts bool) ([]byte, []byte) {
	child := []byte{0x60, 0x07, 0x60, 0x00, 0x55}
	if childReverts {
		child = append(child, 0x60, 0x00, 0x60, 0x00, 0xfd)
	} else {
		child = append(child, 0x00)
	}
	parent := []byte{0x60, 0x00, 0x60, 0x00, 0x55, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x73}
	parent = append(parent, wideSstoreChild[:]...)
	parent = append(parent, 0x61, 0xff, 0xff, 0xf4, 0x50)
	if parentReverts {
		parent = append(parent, 0x60, 0x00, 0x60, 0x00, 0xfd)
	} else {
		parent = append(parent, 0x00)
	}
	return parent, child
}

func wideSstoreRun(name string, original *big.Int, code, child []byte) map[string]any {
	codes := map[common.Address][]byte{wideSstoreTarget: code}
	if child != nil {
		codes[wideSstoreChild] = child
	}
	zeroKey := hex.EncodeToString(make([]byte, 32))
	in := wideSstoreInput{codes: codes, slots: map[string][]byte{zeroKey: original.Bytes()}}
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{})
	state.SetInput(in)
	var evm vm.EVM
	evm.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
	evm.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true})
	result, err := evm.Main(&vm.Transaction{
		From: wideSstoreSender, To: &wideSstoreTarget, Nonce: big.NewInt(1),
		GasPrice: big.NewInt(1), Value: big.NewInt(0), Gas: 200000,
	})
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	row := map[string]any{
		"case": name, "original": original.String(), "code": hex.EncodeToString(code),
		"gas_limit": 200000, "gas_used": result.GasUsed, "refund": state.GetRefund(),
		"execution_error": result.ExecutionErr, "consensus_error": result.ConsensusErr,
		"error": errText, "output": hex.EncodeToString(result.CodeRetval),
		"storage": state.GetAccountConcrete(&wideSstoreTarget).GetState(big.NewInt(0)).String(),
	}
	if child != nil {
		row["child_code"] = hex.EncodeToString(child)
	}
	return row
}

func wideSstoreFixtures() []map[string]any {
	original := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 256), big.NewInt(7))
	rows := []map[string]any{
		wideSstoreRun("wide-to-low", original, wideSstoreCode(7), nil),
		wideSstoreRun("wide-to-zero", original, wideSstoreCode(0), nil),
		wideSstoreRun("wide-low-zero-low", original, wideSstoreCode(7, 0, 7), nil),
	}
	parent, child := wideSstoreNested(false, false)
	rows = append(rows, wideSstoreRun("nested-child-success", original, parent, child))
	parent, child = wideSstoreNested(true, false)
	rows = append(rows, wideSstoreRun("nested-child-revert", original, parent, child))
	parent, child = wideSstoreNested(false, true)
	rows = append(rows, wideSstoreRun("nested-parent-revert", original, parent, child))
	return rows
}

func main() {
	json.NewEncoder(os.Stdout).Encode(map[string]any{"wide_sstore": wideSstoreFixtures()})
}
