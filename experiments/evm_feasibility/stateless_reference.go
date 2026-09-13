// Pinned direct Go precompile primitive oracle for addresses 1 through 4.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/btcsuite/btcd/btcec"
)

func row(address byte, input []byte) map[string]any {
	a := common.BytesToAddress([]byte{address})
	p := vm.PrecompiledContractsCalifornicum.Get(&a)
	out, err := p.Run(vm.CallFrame{Input: input}, nil)
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	return map[string]any{"address": address, "input": hex.EncodeToString(input), "required_gas": p.RequiredGas(vm.CallFrame{Input: input}, nil), "output": hex.EncodeToString(out), "error": errText}
}
func main() {
	rows := []map[string]any{}
	for _, n := range []int{0, 1, 31, 32, 33, 64, 65, 1025} {
		b := make([]byte, n)
		for i := range b {
			b[i] = byte(i + 1)
		}
		for _, a := range []byte{2, 3, 4} {
			rows = append(rows, row(a, b))
		}
	}
	key := make([]byte, 32)
	for i := range key {
		key[i] = 1
	}
	private, _ := btcec.PrivKeyFromBytes(btcec.S256(), key)
	hash := crypto.Keccak256([]byte("rustaxa-stateless"))
	compact, err := btcec.SignCompact(btcec.S256(), private, hash, false)
	if err != nil {
		panic(err)
	}
	base := make([]byte, 128)
	copy(base[:32], hash)
	base[63] = compact[0]
	copy(base[64:96], compact[1:33])
	copy(base[96:], compact[33:65])
	ecases := [][]byte{base, nil, base[:127], append(append([]byte{}, base...), 1)}
	bad := append([]byte{}, base...)
	bad[63] = 29
	ecases = append(ecases, bad)
	pad := append([]byte{}, base...)
	pad[32] = 1
	ecases = append(ecases, pad)
	zero := append([]byte{}, base...)
	for i := 64; i < 128; i++ {
		zero[i] = 0
	}
	ecases = append(ecases, zero)
	n := btcec.S256().Params().N
	high := append([]byte{}, base...)
	s := new(big.Int).SetBytes(compact[33:65])
	s.Sub(n, s)
	copy(high[96:], common.LeftPadBytes(s.Bytes(), 32))
	high[63] = 27 + ((compact[0] - 27) ^ 1)
	ecases = append(ecases, high)
	for _, in := range ecases {
		rows = append(rows, row(1, in))
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"stateless": rows}); err != nil {
		panic(err)
	}
}
