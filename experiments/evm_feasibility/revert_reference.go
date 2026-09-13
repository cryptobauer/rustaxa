// Actual pinned ABI decoding; strings are hex-encoded to preserve invalid UTF-8.
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/accounts/abi"
	"os"
)

func word(n int) []byte {
	b := make([]byte, 32)
	for i := 31; n != 0; i-- {
		b[i] = byte(n)
		n >>= 8
	}
	return b
}
func packed(offset int, reason []byte) []byte {
	body := make([]byte, offset+32+len(reason))
	copy(body, word(offset))
	copy(body[offset:], word(len(reason)))
	copy(body[offset+32:], reason)
	return append([]byte{8, 195, 121, 160}, body...)
}
func main() {
	valid := packed(32, []byte("oracle boom"))
	highOffset := append([]byte(nil), valid...)
	highOffset[4] = 1
	highLength := append([]byte(nil), valid...)
	highLength[36] = 1
	signedLength := append([]byte(nil), valid...)
	signedLength[60] = 128
	wrong := append([]byte(nil), valid...)
	wrong[0] = 9
	zeroOffset := append([]byte{8, 195, 121, 160}, word(0)...)
	cases := []struct {
		Name  string
		Input []byte
	}{
		{"empty", nil}, {"short-selector", []byte{8, 195, 121}}, {"selector-only", []byte{8, 195, 121, 160}},
		{"valid", valid}, {"empty-reason", packed(32, nil)}, {"unaligned-offset", packed(33, []byte("odd"))},
		{"zero-offset", zeroOffset}, {"high-offset", highOffset}, {"high-length", highLength},
		{"signed-length", signedLength}, {"wrong-selector", wrong}, {"truncated-word", valid[:35]},
		{"missing-length", valid[:36]}, {"truncated-reason", valid[:len(valid)-1]},
		{"trailing", append(append([]byte(nil), valid...), 1, 2, 3)},
		{"non-utf8", packed(32, []byte{0xf0, 0x9f, 0, 0xff})},
		{"embedded-zero", packed(32, []byte{'a', 0, 'b'})},
	}
	rows := make([]map[string]interface{}, 0, len(cases))
	for _, c := range cases {
		reason, err := abi.UnpackRevert(c.Input)
		diagnostic := []byte("execution reverted")
		if err == nil {
			diagnostic = append(diagnostic, []byte(": "+reason)...)
		}
		rows = append(rows, map[string]interface{}{"name": c.Name, "input": hex.EncodeToString(c.Input), "valid": err == nil, "reason": hex.EncodeToString([]byte(reason)), "diagnostic": hex.EncodeToString(diagnostic)})
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
