// Deterministic public test vectors from the pinned historical FN-DSA verifier.
// Synthetic signing keys are transient and never written to artifacts.
package main

import (
	"fmt"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/pornin/go-fn-dsa/fndsa"
	"golang.org/x/crypto/sha3"
	"math/big"
)

func falconABI(sig, key, msg []byte) []byte {
	fields := [][]byte{sig, key, msg}
	payload := make([]byte, 96)
	for i, v := range fields {
		word := new(big.Int).SetInt64(int64(len(payload))).FillBytes(make([]byte, 32))
		copy(payload[i*32:], word)
		payload = append(payload, new(big.Int).SetInt64(int64(len(v))).FillBytes(make([]byte, 32))...)
		payload = append(payload, v...)
		for len(payload)%32 != 0 {
			payload = append(payload, 0)
		}
	}
	return append(common.FromHex("de8f50a1"), payload...)
}
func cryptoFixtures() []map[string]any {
	var rows []map[string]any
	for seed := byte(0); seed < 3; seed++ {
		rng := sha3.NewShake256()
		rng.Write([]byte{0x52, 0x75, 0x73, 0x74, seed})
		sk, pk, err := fndsa.KeyGen(9, rng)
		if err != nil {
			panic(err)
		}
		msg := []byte("Rustaxa FN-DSA compatibility")
		if seed == 0 {
			msg = []byte{}
		}
		if seed == 2 {
			msg = make([]byte, 257)
			for i := range msg {
				msg[i] = byte(i)
			}
		}
		sig, err := fndsa.Sign(rng, sk, fndsa.DOMAIN_NONE, 0, msg)
		if err != nil {
			panic(err)
		}
		for _, variant := range []string{"valid", "message-flip", "signature-flip", "short-signature", "short-key"} {
			s, k, m := append([]byte(nil), sig...), append([]byte(nil), pk...), append([]byte(nil), msg...)
			switch variant {
			case "message-flip":
				m = append(m, 1)
			case "signature-flip":
				s[40] ^= 1
			case "short-signature":
				s = s[:len(s)-1]
			case "short-key":
				k = k[:len(k)-1]
			}
			valid := fndsa.Verify(k, fndsa.DOMAIN_NONE, 0, m, s)
			abi := falconABI(s, k, m)
			state := state(input{big.NewInt(1), big.NewInt(1000000), false})
			var e vm.EVM
			e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
			e.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: big.NewInt(0)}}, vm.Rules{IsCornus: true, IsCacti: true})
			to := common.BytesToAddress([]byte{0xfa, 0x1c})
			r, err := e.Main(&vm.Transaction{From: address, To: &to, Nonce: big.NewInt(1), Value: big.NewInt(0), GasPrice: big.NewInt(1), Gas: 300000, Input: abi})
			errorText := ""
			if err != nil {
				errorText = err.Error()
			}
			rows = append(rows, map[string]any{"case": fmt.Sprintf("%d-%s", seed, variant), "key": hx(k), "signature": hx(s), "message": hx(m), "valid": valid, "abi": hx(abi), "return": hx(r.CodeRetval), "gas_used": r.GasUsed, "error": errorText})
		}
	}
	return rows
}
