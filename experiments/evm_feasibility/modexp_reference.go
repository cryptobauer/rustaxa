// Pinned direct address-5 MODEXP oracle. Operand bodies stay bounded by design.
package main

import (
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
)

func header(x *big.Int) []byte { b := make([]byte, 32); x.FillBytes(b); return b }
func input(a, b, c *big.Int, body []byte) []byte {
	return append(append(append(header(a), header(b)...), header(c)...), body...)
}
func row(name string, in []byte) map[string]any {
	a := common.BytesToAddress([]byte{5})
	p := vm.PrecompiledContractsCalifornicum.Get(&a)
	out, err := p.Run(vm.CallFrame{Input: in}, nil)
	e := ""
	if err != nil {
		e = err.Error()
	}
	return map[string]any{"case": name, "input": hex.EncodeToString(in), "required_gas": p.RequiredGas(vm.CallFrame{Input: in}, nil), "output": hex.EncodeToString(out), "error": e}
}
func main() {
	one := big.NewInt(1)
	zero := big.NewInt(0)
	rows := []map[string]any{}
	add := func(n string, a, b, c *big.Int, body []byte) { rows = append(rows, row(n, input(a, b, c, body))) }
	add("empty", zero, zero, zero, nil)
	add("short-header", zero, zero, zero, []byte{1, 2})
	add("two_pow5_mod13", one, one, one, []byte{2, 5, 13})
	add("zero_modulus", one, one, one, []byte{2, 5, 0})
	add("zero_exponent", one, one, one, []byte{0, 0, 13})
	add("zero_pow_zero", one, one, one, []byte{0, 0, 13})
	add("truncated", big.NewInt(2), one, one, []byte{2})
	add("trailing_ignored", one, one, one, []byte{2, 5, 13, 99, 88})
	add("leading_zero", big.NewInt(2), one, big.NewInt(2), []byte{0, 2, 3, 0, 13})
	for _, n := range []uint64{64, 65, 1024, 1025} {
		l := new(big.Int).SetUint64(n)
		body := make([]byte, n+1+1)
		body[n-1] = 2
		body[n] = 3
		body[n+1-1] = 13
		add("base_len_"+new(big.Int).SetUint64(n).String(), l, one, one, body)
		add("mod_len_"+new(big.Int).SetUint64(n).String(), one, one, l, append([]byte{2, 3}, append(make([]byte, n-1), 13)...))
	}
	exp32 := append([]byte{2}, make([]byte, 31)...)
	exp32 = append(exp32, 3, 13)
	add("exp32_zero_head", one, big.NewInt(32), one, exp32)
	add("exp33_high_head", one, big.NewInt(33), one, append([]byte{2}, append(append([]byte{0x80}, make([]byte, 32)...), 13)...))
	wide := new(big.Int).Add(new(big.Int).Lsh(one, 64), one)
	add("wide_base_low1", wide, one, one, []byte{2, 3, 13})
	add("wide_exp_low1", one, wide, one, []byte{2, 3, 13})
	add("wide_mod_low1", one, one, wide, []byte{2, 3, 13})
	add("wide_exp_early", zero, wide, zero, nil)
	_ = binary.BigEndian
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"modexp": rows}); err != nil {
		panic(err)
	}
}
