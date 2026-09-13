// Pinned EVM-level oracle for Taraxa's Ficus/Cacti MCOPY instruction.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

type mcopyInput struct {
	sender common.Address
	target common.Address
	code   []byte
}

func (i mcopyInput) GetCode(hash *common.Hash) []byte {
	if crypto.Keccak256Hash(i.code) != *hash {
		panic("unexpected code hash")
	}
	return clone(i.code)
}

func (i mcopyInput) GetAccount(address *common.Address, callback func(state_db.Account)) {
	if *address == i.sender {
		callback(state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(1_000_000)})
		return
	}
	if *address == i.target {
		hash := crypto.Keccak256Hash(i.code)
		callback(state_db.Account{
			Nonce: big.NewInt(1), Balance: big.NewInt(0), CodeHash: &hash, CodeSize: uint64(len(i.code)),
		})
	}
}

func (mcopyInput) GetAccountStorage(*common.Address, *common.Hash, func([]byte)) {
	panic("unexpected storage read")
}

type mcopyRow struct {
	Name           string `json:"name"`
	Phase          string `json:"phase"`
	Code           string `json:"code"`
	GasUsed        uint64 `json:"gas_used"`
	Output         string `json:"output"`
	ExecutionError string `json:"execution_error"`
	OuterError     string `json:"outer_error"`
}

func clone(value []byte) []byte { return append([]byte(nil), value...) }

func push(code []byte, value *big.Int) []byte {
	bytes := value.Bytes()
	if len(bytes) == 0 {
		bytes = []byte{0}
	}
	if len(bytes) > 32 {
		panic("EVM push exceeds one word")
	}
	return append(append(code, byte(0x5f+len(bytes))), bytes...)
}

func mcopyProgram(initial []byte, dst, src, length *big.Int, returnSize uint64) []byte {
	code := []byte{}
	if initial != nil {
		if len(initial) != 32 {
			panic("initial MCOPY word must contain 32 bytes")
		}
		code = push(code, new(big.Int).SetBytes(initial))
		code = push(code, new(big.Int))
		code = append(code, 0x52) // MSTORE
	}
	code = push(code, length)
	code = push(code, src)
	code = push(code, dst)
	code = append(code, 0x5e) // MCOPY
	code = push(code, new(big.Int).SetUint64(returnSize))
	code = push(code, new(big.Int))
	return append(code, 0xf3) // RETURN
}

func execute(name, phase string, code []byte) mcopyRow {
	sender := common.BytesToAddress([]byte{0xaa})
	target := common.BytesToAddress([]byte{0xbb})
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{})
	state.SetInput(mcopyInput{sender: sender, target: target, code: code})
	var evm vm.EVM
	evm.Init(
		func(uint64) *big.Int { return new(big.Int) },
		state,
		vm.DefaultOpts(),
		params.TestChainConfig,
		vm.Config{},
	)
	rules := vm.Rules{IsCornus: true}
	switch phase {
	case "californicum":
	case "ficus":
		rules.IsFicus = true
	case "cacti":
		rules.IsFicus = true
		rules.IsCacti = true
	default:
		panic("unknown MCOPY phase")
	}
	evm.SetBlock(
		&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1_000_000, Difficulty: new(big.Int)}},
		rules,
	)
	result, err := evm.Main(&vm.Transaction{
		From: sender, To: &target, Nonce: big.NewInt(1), GasPrice: big.NewInt(1),
		Value: new(big.Int), Gas: 100_000,
	})
	outerError := ""
	if err != nil {
		outerError = err.Error()
	}
	return mcopyRow{
		Name: name, Phase: phase, Code: hex.EncodeToString(code), GasUsed: result.GasUsed,
		Output: hex.EncodeToString(result.CodeRetval), ExecutionError: string(result.ExecutionErr),
		OuterError: outerError,
	}
}

func main() {
	pattern := make([]byte, 32)
	for i := range pattern {
		pattern[i] = byte(i)
	}
	zero := new(big.Int)
	one := big.NewInt(1)
	sixteen := big.NewInt(16)
	thirtyTwo := big.NewInt(32)
	thirtyThree := big.NewInt(33)
	wide := new(big.Int).Lsh(big.NewInt(1), 255)
	large := new(big.Int).SetUint64(1 << 20)

	cases := []struct {
		name  string
		phase string
		code  []byte
	}{
		{"before-ficus", "californicum", mcopyProgram(nil, zero, zero, zero, 0)},
		{"ficus-zero-length-wide-dst", "ficus", mcopyProgram(nil, wide, zero, zero, 0)},
		{"cacti-inherits-zero-length-wide-dst", "cacti", mcopyProgram(nil, wide, zero, zero, 0)},
		{"ficus-copy-forward-overlap", "ficus", mcopyProgram(pattern, one, zero, sixteen, 32)},
		{"ficus-copy-backward-overlap", "ficus", mcopyProgram(pattern, zero, one, sixteen, 32)},
		{"cacti-copy-forward-overlap", "cacti", mcopyProgram(pattern, one, zero, sixteen, 32)},
		{"ficus-expand-empty-memory", "ficus", mcopyProgram(nil, sixteen, thirtyTwo, one, 64)},
		{"ficus-copy-two-words", "ficus", mcopyProgram(pattern, thirtyTwo, zero, thirtyThree, 96)},
		{"ficus-expansion-out-of-gas", "ficus", mcopyProgram(nil, large, zero, one, 0)},
		{"ficus-stack-underflow", "ficus", []byte{0x5e, 0x00}},
	}
	rows := make([]mcopyRow, 0, len(cases))
	for _, item := range cases {
		rows = append(rows, execute(item.name, item.phase, item.code))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"mcopy": rows}); err != nil {
		panic(err)
	}
}
