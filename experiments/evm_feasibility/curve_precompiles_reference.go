// Pinned direct Go oracle for Taraxa's BN254 and BLAKE2F precompiles.
// Inputs deliberately bound BLAKE2F rounds so this helper cannot become an
// accidental expensive benchmark.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto/bn256"
)

const maxOracleBlakeRounds = 12

type curveRow struct {
	Name        string `json:"name"`
	Address     byte   `json:"address"`
	Input       string `json:"input"`
	RequiredGas uint64 `json:"required_gas"`
	Output      string `json:"output"`
	Error       string `json:"error"`
}

func precompile(address byte) vm.PrecompiledContract {
	a := common.BytesToAddress([]byte{address})
	registry := vm.PrecompiledContractsCalifornicum
	if address == 9 {
		registry = vm.PrecompiledContractsFicus
	}
	return registry.Get(&a)
}

func curveOracleRow(name string, address byte, input []byte) curveRow {
	if address == 9 && len(input) == 213 {
		rounds := uint64(input[0])<<24 | uint64(input[1])<<16 | uint64(input[2])<<8 | uint64(input[3])
		if rounds > maxOracleBlakeRounds {
			panic("unbounded BLAKE2F oracle rounds")
		}
	}
	p := precompile(address)
	frame := vm.CallFrame{Input: input}
	output, err := p.Run(frame, nil)
	errorText := ""
	if err != nil {
		errorText = err.Error()
	}
	return curveRow{
		Name: name, Address: address, Input: hex.EncodeToString(input),
		RequiredGas: p.RequiredGas(frame, nil), Output: hex.EncodeToString(output), Error: errorText,
	}
}

func clone(input []byte) []byte { return append([]byte(nil), input...) }

func leftPad32(value *big.Int) []byte {
	result := make([]byte, 32)
	value.FillBytes(result)
	return result
}

func blakeInput(rounds uint32, final byte) []byte {
	input := make([]byte, 213)
	input[0] = byte(rounds >> 24)
	input[1] = byte(rounds >> 16)
	input[2] = byte(rounds >> 8)
	input[3] = byte(rounds)
	// Deterministic nonzero words make byte order and both final modes visible.
	for i := 4; i < 212; i++ {
		input[i] = byte(i*29 + 7)
	}
	input[212] = final
	return input
}

func main() {
	one := big.NewInt(1)
	g1 := new(bn256.G1).ScalarBaseMult(one).Marshal()
	g1Negative := new(bn256.G1).Neg(new(bn256.G1).ScalarBaseMult(one)).Marshal()
	g2 := new(bn256.G2).ScalarBaseMult(one).Marshal()
	infinity := make([]byte, 64)

	fieldModulus, ok := new(big.Int).SetString("21888242871839275222246405745257275088696311157297823662689037894645226208583", 10)
	if !ok {
		panic("field modulus")
	}
	outOfField := append(leftPad32(fieldModulus), leftPad32(big.NewInt(2))...)
	aboveField := append(leftPad32(new(big.Int).Add(fieldModulus, one)), leftPad32(big.NewInt(2))...)
	offCurve := append(leftPad32(one), leftPad32(big.NewInt(3))...)

	rows := []curveRow{}
	add := func(name string, address byte, input []byte) {
		rows = append(rows, curveOracleRow(name, address, input))
	}

	add("add-empty-infinities", 6, nil)
	add("add-generator-infinity", 6, append(clone(g1), infinity...))
	add("add-generator-generator", 6, append(clone(g1), g1...))
	add("add-truncated-second", 6, append(clone(g1), g1[:17]...))
	add("add-trailing-ignored", 6, append(append(clone(g1), infinity...), 0xaa, 0xbb))
	add("add-off-curve-first", 6, append(clone(offCurve), infinity...))
	add("add-off-curve-before-noncanonical", 6, append(clone(offCurve), outOfField...))
	add("add-out-of-field-first", 6, append(clone(outOfField), infinity...))
	add("add-above-field-first", 6, append(clone(aboveField), infinity...))

	add("mul-empty-infinity", 7, nil)
	add("mul-generator-zero", 7, append(clone(g1), make([]byte, 32)...))
	add("mul-generator-one", 7, append(clone(g1), leftPad32(one)...))
	add("mul-generator-two", 7, append(clone(g1), leftPad32(big.NewInt(2))...))
	add("mul-truncated-scalar", 7, append(clone(g1), 0x01))
	add("mul-truncated-point", 7, clone(g1[:33]))
	add("mul-trailing-ignored", 7, append(append(clone(g1), leftPad32(one)...), 0xcc))
	add("mul-off-curve", 7, append(clone(offCurve), leftPad32(one)...))
	add("mul-out-of-field", 7, append(clone(outOfField), leftPad32(one)...))

	validPair := append(clone(g1), g2...)
	truePair := append(append(clone(validPair), g1Negative...), g2...)
	add("pairing-empty-true", 8, nil)
	add("pairing-one-false", 8, validPair)
	add("pairing-negated-product-true", 8, truePair)
	add("pairing-infinity-true", 8, make([]byte, 192))
	add("pairing-bad-length-one", 8, []byte{1})
	add("pairing-bad-length-191", 8, make([]byte, 191))
	add("pairing-bad-length-193", 8, append(clone(validPair), 0x01))
	add("pairing-off-curve-g1", 8, append(clone(offCurve), g2...))
	badG2 := clone(g2)
	badG2[0] ^= 0xff
	add("pairing-invalid-g2", 8, append(clone(g1), badG2...))
	offCurveG2 := clone(g2)
	offCurveG2[127] ^= 1
	add("pairing-in-field-off-curve-g2", 8, append(clone(g1), offCurveG2...))
	add("pairing-off-curve-before-noncanonical", 8, append(clone(offCurve), badG2...))
	add("pairing-valid-before-invalid-later", 8, append(clone(validPair), append(clone(g1), badG2...)...))
	add("pairing-out-of-field-g1", 8, append(clone(outOfField), g2...))

	// The canonical EIP-152 "abc" compression vector retained by Taraxa.
	known, err := hex.DecodeString("0000000c48c9bdf267e6096a3ba7ca8485ae67bb2bf894fe72f36e3cf1361d5f3af54fa5d182e6ad7f520e511f6c3e2b8c68059b6bbd41fbabd9831f79217e1319cde05b61626300000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000300000000000000000000000000000001")
	if err != nil || len(known) != 213 {
		panic("known BLAKE2F vector")
	}
	add("blake-known-abc-final", 9, known)
	for _, item := range []struct {
		name   string
		rounds uint32
		final  byte
	}{{"blake-zero-round-nonfinal", 0, 0}, {"blake-one-round-nonfinal", 1, 0}, {"blake-two-round-final", 2, 1}, {"blake-twelve-round-final", 12, 1}} {
		add(item.name, 9, blakeInput(item.rounds, item.final))
	}
	add("blake-empty", 9, nil)
	add("blake-short-212", 9, make([]byte, 212))
	add("blake-long-214", 9, make([]byte, 214))
	add("blake-invalid-final", 9, blakeInput(2, 2))

	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"curve_precompiles": rows}); err != nil {
		panic(err)
	}
}
