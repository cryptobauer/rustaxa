// Pinned CreateAddress fixture exporter, copied into archived Go references.
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/crypto"
)

func main() {
	creator := common.HexToAddress("0x11223344556677889900aabbccddeeff00112233")
	cases := []struct {
		name  string
		nonce *big.Int
	}{
		{"zero", big.NewInt(0)},
		{"bytes55_max", new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 440), big.NewInt(1))},
		{"bytes56_first", new(big.Int).Lsh(big.NewInt(1), 440)},
		{"bytes65_2pow512", new(big.Int).Lsh(big.NewInt(1), 512)},
		{"bytes257_2pow2048", new(big.Int).Lsh(big.NewInt(1), 2048)},
	}
	rows := make([]map[string]string, 0, len(cases))
	for _, c := range cases {
		address := crypto.CreateAddress(&creator, c.nonce)
		rows = append(rows, map[string]string{"case": c.name, "creator": hex.EncodeToString(creator[:]), "nonce": c.nonce.String(), "nonce_bytes": fmt.Sprint(len(c.nonce.Bytes())), "address": hex.EncodeToString(address[:])})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"creation_addresses": rows}); err != nil {
		panic(err)
	}
}
