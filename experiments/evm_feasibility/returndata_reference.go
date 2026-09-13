// Actual Go EVM oracle using the existing MCOPY harness's immutable input port.
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
)

type returnRow struct {
	mcopyRow
	Panic    string `json:"panic"`
	GasLimit uint64 `json:"gas_limit"`
}

func returnProgram(seed bool, destination, source, length *big.Int) []byte {
	code := []byte{}
	zero := new(big.Int)
	if seed {
		pattern := make([]byte, 32)
		for i := range pattern {
			pattern[i] = byte(i + 1)
		}
		code = push(code, new(big.Int).SetBytes(pattern))
		code = push(code, zero)
		code = append(code, 0x52)
		// Identity CALL seeds a real 32-byte last_retval without copying its output.
		for _, value := range []int64{0, 0, 32, 0, 0, 4, 1000} {
			code = push(code, big.NewInt(value))
		}
		code = append(code, 0xf1, 0x50)
	}
	code = push(code, length)
	code = push(code, source)
	code = push(code, destination)
	code = append(code, 0x3e)
	code = push(code, big.NewInt(96))
	code = push(code, zero)
	return append(code, 0xf3)
}

func observeReturn(name, phase string, code []byte, gasLimit uint64) (row returnRow) {
	row.mcopyRow = mcopyRow{Name: name, Phase: phase, Code: hex.EncodeToString(code)}
	row.GasLimit = gasLimit
	defer func() {
		if p := recover(); p != nil {
			row.Panic = fmt.Sprint(p)
		}
	}()
	row.mcopyRow = executeReturn(name, phase, code, gasLimit)
	return
}

func main() {
	z := new(big.Int)
	one := big.NewInt(1)
	wide := new(big.Int).Lsh(big.NewInt(1), 255)
	max := new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 256), one)
	max64 := new(big.Int).SetUint64(^uint64(0))
	cases := []struct {
		name           string
		seed           bool
		dst, src, size *big.Int
	}{
		{"empty-zero", false, z, z, z},
		{"zero-wide-destination", false, wide, z, z},
		{"zero-wide-source", false, z, wide, z},
		{"empty-source-bounds", false, z, z, one},
		{"source-bounds-memory-oog", false, big.NewInt(1 << 20), z, one},
		{"source-bounds-copy-oog", false, z, z, big.NewInt(1 << 20)},
		{"source-bounds-memory-uint-overflow", false, wide, z, one},
		{"source-bounds-memory-round-overflow", false, max64, z, one},
		{"source-bounds-memory-word-round-overflow", false, new(big.Int).Sub(max64, one), z, one},
		{"source-bounds-memory-gas-overflow", false, new(big.Int).SetUint64(0x10000000000), z, one},
		{"wrapped-memory-source-bounds", false, max, z, one},
		{"copy-whole", true, big.NewInt(32), z, big.NewInt(32)},
		{"copy-tail", true, big.NewInt(64), big.NewInt(17), big.NewInt(15)},
		{"copy-source-overrun", true, z, one, big.NewInt(32)},
		{"zero-exact-source-end", true, wide, big.NewInt(32), z},
		{"zero-past-source-end", true, z, big.NewInt(33), z},
		{"source-add-overflow", true, z, max64, one},
		{"wrapped-destination-reference-panic", true, max, z, one},
		{"wide-length-memory-wrap", false, one, z, max},
	}
	rows := []returnRow{}
	for _, phase := range []string{"californicum", "ficus", "cacti"} {
		for _, c := range cases {
			rows = append(rows, observeReturn(phase+"-"+c.name, phase, returnProgram(c.seed, c.dst, c.src, c.size), 100_000))
		}
		rows = append(rows, observeReturn(phase+"-stack-underflow", phase, []byte{0x3e, 0x00}, 100_000))
		rows = append(rows, observeReturn(phase+"-stack-before-base-oog", phase, []byte{0x3e, 0}, 21_000))
		rows = append(rows, observeReturn(phase+"-memory-overflow-before-base-oog", phase, returnProgram(false, wide, z, one), 21_009))
		rows = append(rows, observeReturn(phase+"-base-oog-before-source-bounds", phase, returnProgram(false, z, z, one), 21_009))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"returndata": rows}); err != nil {
		panic(err)
	}
}

func executeReturn(name, phase string, code []byte, gasLimit uint64) mcopyRow {
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
		Value: new(big.Int), Gas: gasLimit,
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
