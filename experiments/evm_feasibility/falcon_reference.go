// Pinned direct oracle for Taraxa's Cacti Falcon-512 precompile.
package main

import (
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/pornin/go-fn-dsa/fndsa"
	"golang.org/x/crypto/sha3"
)

const falconSelector = uint32(0xde8f50a1)

type falconVector struct {
	key       []byte
	signature []byte
	message   []byte
}

type falconRow struct {
	Name               string `json:"name"`
	Input              string `json:"input"`
	RequiredGas        uint64 `json:"required_gas"`
	Output             string `json:"output"`
	Error              string `json:"error"`
	Panic              string `json:"panic"`
	CryptographicValid bool   `json:"cryptographic_valid"`
}

func cloneBytes(value []byte) []byte { return append([]byte(nil), value...) }

func word(value uint64, highBits bool) []byte {
	result := make([]byte, 32)
	binary.BigEndian.PutUint64(result[24:], value)
	if highBits {
		result[0] = 1
	}
	return result
}

func selector() []byte {
	result := make([]byte, 4)
	binary.BigEndian.PutUint32(result, falconSelector)
	return result
}

// falconABI permits every layout accepted by the Go implementation: dynamic
// fields may be reordered or unaligned, and offset/length words use low 64 bits.
func falconABI(vector falconVector, order []int, prefixPadding int, highOffsets, highLengths bool, trailing []byte) []byte {
	fields := [][]byte{vector.signature, vector.key, vector.message}
	body := make([]byte, 96+prefixPadding)
	offsets := [3]uint64{}
	for _, field := range order {
		offsets[field] = uint64(len(body))
		body = append(body, word(uint64(len(fields[field])), highLengths)...)
		body = append(body, fields[field]...)
		for len(body)%32 != 0 {
			body = append(body, 0)
		}
	}
	for field, offset := range offsets {
		copy(body[field*32:], word(offset, highOffsets))
	}
	return append(append(selector(), body...), trailing...)
}

func lowWord(input []byte, bodyWord int) uint64 {
	start := 4 + bodyWord*32
	return binary.BigEndian.Uint64(input[start+24 : start+32])
}

func setLowWord(input []byte, absoluteStart int, value uint64) []byte {
	result := cloneBytes(input)
	for index := absoluteStart; index < absoluteStart+32; index++ {
		result[index] = 0
	}
	binary.BigEndian.PutUint64(result[absoluteStart+24:absoluteStart+32], value)
	return result
}

func vectors() []falconVector {
	result := make([]falconVector, 0, 4)
	for seed := byte(0); seed < 4; seed++ {
		rng := sha3.NewShake256()
		_, _ = rng.Write([]byte{0x52, 0x75, 0x73, 0x74, seed})
		sk, pk, err := fndsa.KeyGen(9, rng)
		if err != nil {
			panic(err)
		}
		message := []byte("Rustaxa FN-DSA compatibility")
		if seed == 0 {
			message = []byte{}
		}
		if seed == 2 {
			message = make([]byte, 257)
			for index := range message {
				message[index] = byte(index)
			}
		}
		if seed == 3 {
			message = append([]byte("implicit Go padding"), 0, 0, 0, 0)
		}
		signature, err := fndsa.Sign(rng, sk, fndsa.DOMAIN_NONE, 0, message)
		if err != nil {
			panic(err)
		}
		result = append(result, falconVector{key: pk, signature: signature, message: message})
	}
	return result
}

func main() {
	address := common.BytesToAddress([]byte{0xfa, 0x1c})
	contract := vm.PrecompiledContractsCacti.Get(&address)
	if contract == nil {
		panic("Cacti registry omitted Falcon address 0xfa1c")
	}
	all := vectors()
	emptyMessage, standard, longMessage, paddedMessage := all[0], all[1], all[2], all[3]
	canonical := falconABI(standard, []int{0, 1, 2}, 0, false, false, nil)
	longCanonical := falconABI(longMessage, []int{0, 1, 2}, 0, false, false, nil)
	paddedCanonical := falconABI(paddedMessage, []int{0, 1, 2}, 0, false, false, nil)

	signatureOffset := lowWord(canonical, 0)
	keyOffset := lowWord(canonical, 1)
	messageOffset := lowWord(canonical, 2)
	paddedMessageOffset := lowWord(paddedCanonical, 2)
	rightPadded := paddedCanonical[:4+int(paddedMessageOffset)+32+len(paddedMessage.message)-4]
	bodyLength := uint64(len(canonical) - 4)
	mutatedSignature := falconVector{
		key: cloneBytes(standard.key), signature: cloneBytes(standard.signature), message: cloneBytes(standard.message),
	}
	mutatedSignature.signature[40] ^= 1
	mutatedMessage := falconVector{
		key: cloneBytes(standard.key), signature: cloneBytes(standard.signature), message: append(cloneBytes(standard.message), 1),
	}

	type inputCase struct {
		name  string
		input []byte
		valid bool
	}
	cases := []inputCase{
		{"empty-input", nil, false},
		{"short-selector", selector()[:3], false},
		{"wrong-selector", []byte{0x11, 0x11, 0x11, 0x11}, false},
		{"selector-only", selector(), false},
		{"truncated-head", append(selector(), make([]byte, 95)...), false},
		{"zero-signature-offset", setLowWord(canonical, 4, 0), false},
		{"zero-key-offset", setLowWord(canonical, 4+32, 0), false},
		{"zero-message-offset", setLowWord(canonical, 4+64, 0), false},
		{"signature-offset-out-of-range", setLowWord(canonical, 4, bodyLength+1), false},
		{"key-offset-out-of-range", setLowWord(canonical, 4+32, bodyLength+1), false},
		{"message-offset-out-of-range", setLowWord(canonical, 4+64, bodyLength+1), false},
		{"zero-signature-length", setLowWord(canonical, 4+int(signatureOffset), 0), false},
		{"zero-key-length", setLowWord(canonical, 4+int(keyOffset), 0), false},
		{"zero-message-length", setLowWord(canonical, 4+int(messageOffset), 0), false},
		{"wrong-signature-length", setLowWord(canonical, 4+int(signatureOffset), uint64(len(standard.signature)-1)), false},
		{"wrong-key-length", setLowWord(canonical, 4+int(keyOffset), uint64(len(standard.key)-1)), false},
		{"truncated-signature", canonical[:4+int(signatureOffset)+32+len(standard.signature)-1], false},
		{"truncated-key", canonical[:4+int(keyOffset)+32+len(standard.key)-1], false},
		{"truncated-message", canonical[:4+int(messageOffset)+32+len(standard.message)-1], false},
		{"historical-empty-message", falconABI(emptyMessage, []int{0, 1, 2}, 0, false, false, nil), true},
		{"historical-valid", canonical, true},
		{"historical-valid-long-message", longCanonical, true},
		{"go-right-padded-message", rightPadded, true},
		{"beyond-go-right-padding", paddedCanonical[:4+int(paddedMessageOffset)+32+len(paddedMessage.message)-5], true},
		{"invalid-signature", falconABI(mutatedSignature, []int{0, 1, 2}, 0, false, false, nil), false},
		{"invalid-message", falconABI(mutatedMessage, []int{0, 1, 2}, 0, false, false, nil), false},
		{"reordered-fields", falconABI(standard, []int{2, 1, 0}, 0, false, false, nil), true},
		{"unaligned-fields", falconABI(standard, []int{0, 1, 2}, 1, false, false, nil), true},
		{"high-bits-offsets", falconABI(standard, []int{0, 1, 2}, 0, true, false, nil), true},
		{"high-bits-lengths", falconABI(standard, []int{0, 1, 2}, 0, false, true, nil), true},
		{"trailing-bytes", falconABI(standard, []int{0, 1, 2}, 0, false, false, []byte{0xaa, 0xbb, 0xcc}), true},
	}
	cases = append(cases,
		inputCase{"signed-message-length-tail", setLowWord(rightPadded, 4+int(paddedMessageOffset), uint64(1)<<63), true},
		inputCase{"wrapped-message-length-panic", setLowWord(rightPadded, 4+int(paddedMessageOffset), ^uint64(0)), true},
	)

	// All offsets are checked before any dynamic slice can panic.
	badSignature := setLowWord(canonical, 4+int(signatureOffset), ^uint64(0))
	cases = append(cases,
		inputCase{"signature-panic-zero-key-offset", setLowWord(badSignature, 4+32, 0), false},
		inputCase{"signature-panic-zero-message-offset", setLowWord(badSignature, 4+64, 0), false},
		inputCase{"wrapped-signature-length-panic", badSignature, false},
	)
	// A clamped tail of the fixed size does not authorize a different declared
	// length. Put each fixed-size field last and retain Go's four pad bytes.
	signatureLast := falconABI(standard, []int{2, 1, 0}, 0, false, false, nil)
	sigLastOffset := lowWord(signatureLast, 0)
	signatureLast = signatureLast[:int(sigLastOffset)+32+len(standard.signature)]
	keyLast := falconABI(standard, []int{2, 0, 1}, 0, false, false, nil)
	keyLastOffset := lowWord(keyLast, 1)
	keyLast = keyLast[:int(keyLastOffset)+32+len(standard.key)]
	cases = append(cases,
		inputCase{"signed-signature-length-fixed-tail", setLowWord(signatureLast, 4+int(sigLastOffset), uint64(1)<<63), false},
		inputCase{"signed-key-length-fixed-tail", setLowWord(keyLast, 4+int(keyLastOffset), uint64(1)<<63), false},
	)

	signatureBoundary := falconABI(standard, []int{2, 1, 0}, 32, false, false, nil)
	signatureBoundaryOffset := lowWord(signatureBoundary, 0)
	signatureBoundary = signatureBoundary[:int(signatureBoundaryOffset)+32+len(standard.signature)]
	signatureBoundary = setLowWord(signatureBoundary, 4+int(signatureBoundaryOffset), uint64(1)<<63)
	signatureBoundary = setLowWord(signatureBoundary, 4+32, 96)
	signatureBoundary = setLowWord(signatureBoundary, 4+96, ^uint64(0))
	keyBoundary := falconABI(standard, []int{2, 0, 1}, 32, false, false, nil)
	keyBoundaryOffset := lowWord(keyBoundary, 1)
	keyBoundary = keyBoundary[:int(keyBoundaryOffset)+32+len(standard.key)]
	keyBoundary = setLowWord(keyBoundary, 4+int(keyBoundaryOffset), uint64(1)<<63)
	keyBoundary = setLowWord(keyBoundary, 4+64, 96)
	keyBoundary = setLowWord(keyBoundary, 4+96, ^uint64(0))
	cases = append(cases,
		inputCase{"declared-signature-size-before-key-panic", signatureBoundary, false},
		inputCase{"declared-key-size-before-message-panic", keyBoundary, false},
	)

	rows := make([]falconRow, 0, len(cases))
	for _, item := range cases {
		frame := vm.CallFrame{Input: item.input, Value: new(big.Int)}
		requiredGas := contract.RequiredGas(frame, nil)
		output, err, panicText := runFalcon(contract, frame)
		errorText := ""
		if err != nil {
			errorText = err.Error()
		}
		rows = append(rows, falconRow{
			Name: item.name, Input: hex.EncodeToString(item.input), RequiredGas: requiredGas,
			Output: hex.EncodeToString(output), Error: errorText, Panic: panicText, CryptographicValid: item.valid,
		})
	}
	if len(standard.signature) != fndsa.SignatureSize(9) || len(standard.key) != fndsa.VerifyingKeySize(9) {
		panic(fmt.Sprintf("unexpected Falcon-512 sizes: signature=%d key=%d", len(standard.signature), len(standard.key)))
	}
	document := map[string]any{
		"address":                hex.EncodeToString(address[:]),
		"signature_size":         len(standard.signature),
		"verifying_key_size":     len(standard.key),
		"method_selector":        fmt.Sprintf("%08x", falconSelector),
		"valid_output_semantics": "bytes32(0)=valid, bytes32(1)=invalid",
		"falcon":                 rows,
	}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}

func runFalcon(contract vm.PrecompiledContract, frame vm.CallFrame) (output []byte, err error, panicText string) {
	defer func() {
		if recovered := recover(); recovered != nil {
			output = nil
			err = nil
			panicText = fmt.Sprint(recovered)
		}
	}()
	output, err = contract.Run(frame, nil)
	return
}
