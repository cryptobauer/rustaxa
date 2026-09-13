// Pinned direct Go oracle for Taraxa's Cacti P-256 precompile.
package main

import (
	"crypto/elliptic"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
)

type p256Row struct {
	Name        string `json:"name"`
	Input       string `json:"input"`
	RequiredGas uint64 `json:"required_gas"`
	Output      string `json:"output"`
	Error       string `json:"error"`
}

func p256OracleRow(precompile vm.PrecompiledContract, name string, input []byte) p256Row {
	frame := vm.CallFrame{Input: input}
	output, err := precompile.Run(frame, nil)
	errorText := ""
	if err != nil {
		errorText = err.Error()
	}
	return p256Row{
		Name: name, Input: hex.EncodeToString(input),
		RequiredGas: precompile.RequiredGas(frame, nil), Output: hex.EncodeToString(output), Error: errorText,
	}
}

func clone(input []byte) []byte { return append([]byte(nil), input...) }

func replace(input []byte, start int, value []byte) []byte {
	result := clone(input)
	copy(result[start:start+len(value)], value)
	return result
}

func leftPad32(value []byte) []byte {
	result := make([]byte, 32)
	copy(result[32-len(value):], value)
	return result
}

func main() {
	address := common.BytesToAddress([]byte{0x01, 0x00})
	precompile := vm.PrecompiledContractsCacti.Get(&address)
	if precompile == nil {
		panic("Cacti P-256 precompile is absent")
	}
	valid, err := hex.DecodeString("4cee90eb86eaa050036147a12d49004b6b9c72bd725d39d4785011fe190f0b4da73bd4903f0ce3b639bbbf6e8e80d16931ff4bcf5993d58468e8fb19086e8cac36dbcd03009df8c59286b162af3bd7fcc0450c9aa81be5d10d312af6c66b1d604aebd3099c618202fcfe16ae7770b0c49ab5eadf74b754204a3bb6060e44eff37618b065f9832de4ca6ca971a7a1adc826d0f7c00181a5fb2ddf79ae00b4e10e")
	if err != nil || len(valid) != 160 {
		panic("valid P-256 vector")
	}

	order := elliptic.P256().Params().N
	s := clone(valid[64:96])
	highS := leftPad32(new(big.Int).Sub(new(big.Int).Set(order), new(big.Int).SetBytes(s)).Bytes())
	zero := make([]byte, 32)
	curveOrder := leftPad32(order.Bytes())
	fieldPrime := leftPad32(elliptic.P256().Params().P.Bytes())

	inputs := []struct {
		name  string
		input []byte
	}{
		{"empty", nil},
		{"one-byte", []byte{1}},
		{"zero-159", make([]byte, 159)},
		{"zero-160", make([]byte, 160)},
		{"zero-161", make([]byte, 161)},
		{"valid", valid},
		{"valid-truncated", clone(valid[:159])},
		{"valid-trailing", append(clone(valid), 0)},
		{"wrong-message", append([]byte{valid[0] ^ 1}, valid[1:]...)},
		{"high-s-valid", replace(valid, 64, highS)},
		{"r-zero", replace(valid, 32, zero)},
		{"r-order", replace(valid, 32, curveOrder)},
		{"s-zero", replace(valid, 64, zero)},
		{"s-order", replace(valid, 64, curveOrder)},
		{"public-x-zero", replace(valid, 96, zero)},
		{"public-y-zero", replace(valid, 128, zero)},
		{"public-x-field-prime", replace(valid, 96, fieldPrime)},
		{"public-y-field-prime", replace(valid, 128, fieldPrime)},
	}
	rows := make([]p256Row, 0, len(inputs))
	for _, input := range inputs {
		rows = append(rows, p256OracleRow(precompile, input.name, input.input))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"p256": rows}); err != nil {
		panic(err)
	}
}
