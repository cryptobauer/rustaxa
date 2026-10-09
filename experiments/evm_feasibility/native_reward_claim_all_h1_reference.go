// Actual positive noarg claim-all at literal same-H1 over accepted reward-only P1.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

var h1RawWrites []map[string]string
var h1Quotes []map[string]any
var h1Accounts map[string]any
var h1AfterRaw map[string]string

// Fixed union from the accepted P2 witness plus both literal H1 node keys.
// The union is independent of this run's observed writes and is read in controls.
func h1WatchedKeys() []common.Hash {
	keys := []common.Hash{}

	keys = append(keys, common.HexToHash("0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe"))
	keys = append(keys, common.HexToHash("0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60"))
	keys = append(keys, common.HexToHash("1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7"))
	keys = append(keys, common.HexToHash("18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c"))
	keys = append(keys, common.HexToHash("4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05"))
	keys = append(keys, common.HexToHash("4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92"))
	keys = append(keys, common.HexToHash("4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a"))
	keys = append(keys, common.HexToHash("5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b"))
	keys = append(keys, common.HexToHash("753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3"))
	keys = append(keys, common.HexToHash("754536fdc3837580b0c148ccad50cac1c2d84dc9a1f8df2790b6f964f4af3180"))
	keys = append(keys, common.HexToHash("9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e"))
	keys = append(keys, common.HexToHash("9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a"))
	keys = append(keys, common.HexToHash("a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9"))
	keys = append(keys, common.HexToHash("b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5"))
	keys = append(keys, common.HexToHash("cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1"))
	keys = append(keys, common.HexToHash("d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873"))
	keys = append(keys, common.HexToHash("dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795"))
	keys = append(keys, common.HexToHash("e8595be39863dce96c5e29d86669ffb30f4e4d19ef14907f331391288d7eaab6"))
	keys = append(keys, common.HexToHash("ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc"))
	keys = append(keys, common.HexToHash("f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393"))
	keys = append(keys, common.HexToHash("fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0"))
	for _, v := range []common.Address{nativeSimulationValidator, common.BytesToAddress([]byte{0x32})} {
		data := append([]byte{1}, v[:]...)
		data = append(data, 1) // current block minimal BigEndian, not LE64
		keys = append(keys, common.BytesToHash(crypto.Keccak256(data)))
	}
	return keys
}
func h1RawCopy(state *state_evm.BlockState) map[string]string {
	result := map[string]string{}
	address := *dpos.ContractAddress()
	for _, key := range h1WatchedKeys() {
		value := []byte{}
		state.GetAccountConcrete(&address).GetRawState(&key, func(v []byte) { value = common.CopyBytes(v) })
		result[hex.EncodeToString(key[:])] = hex.EncodeToString(value)
	}
	return result
}

func main() {
	vm.SetRewardH1QuoteObserver(func(frame vm.CallFrame, required uint64) {
		caller := frame.CallerAccount.Address()
		h1Quotes = append(h1Quotes, map[string]any{"required_gas": required, "supplied_native_gas": frame.Gas, "input": hex.EncodeToString(frame.Input), "caller": hex.EncodeToString(caller[:])})
	})
	state_dry_runner.SetRewardH1PostObserver(func(state *state_evm.BlockState) {
		h1Accounts = map[string]any{}
		for _, a := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, *dpos.ContractAddress()} {
			account := state.GetAccountConcrete(&a)
			h1Accounts[hex.EncodeToString(a[:])] = map[string]string{"nonce": account.GetNonce().String(), "balance": account.GetBalance().String()}
		}
		h1AfterRaw = h1RawCopy(state)
	})
	state_evm.SetNativeV1CustodyRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		h1RawWrites = append(h1RawWrites, map[string]string{"address": hex.EncodeToString(address[:]), "key": hex.EncodeToString(key[:]), "value": hex.EncodeToString(value)})
	})
	state_evm.SetRewardBalanceObserver(func(address common.Address, operation string, amount, balance *big.Int) {
		rewardBalances = append(rewardBalances, map[string]string{"address": hex.EncodeToString(address[:]), "operation": operation, "amount": amount.String(), "balance": balance.String()})
	})
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0x31}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationSender = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	x, y = secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0xa1}, 32))
	pub = append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationDelegator = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	identities := []common.Address{nativeSimulationSender, nativeSimulationDelegator, nativeSimulationValidator, common.BytesToAddress([]byte{0x32})}
	for i, a := range identities {
		for _, b := range identities[i+1:] {
			if a == b {
				panic("signer/validator identity collision")
			}
		}
	}
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	// Actual Go and Rust same-height native/EVM block1 over committedP1.
	// Caller effective nonce1 is independent of this block identity.
	block := &vm.Block{Number: 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := currentSignedSnapshot(seed, api)
	priorRaw := h1RawCopy(state_evm.GetBlockState(seed.database, 1, 1))
	contract := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []map[string]any{}
	for _, c := range []struct {
		name                  string
		from, to              byte
		amount                int64
		gas                   uint64
		malformed, nonpayable bool
	}{
		{name: "claim_all_positive", from: 0x31, gas: 200000},
	} {
		input := common.FromHex("0b83a727")
		if c.malformed {
			input = input[:4]
		}
		value := new(big.Int)
		if c.nonpayable {
			value.SetInt64(1)
		}
		tx := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: c.gas, Value: value, Input: input}
		rewardBalances = []map[string]string{}
		h1RawWrites = []map[string]string{}
		h1Quotes = []map[string]any{}
		first := runNativeSimulationCaptured(runner, block, c.name, tx)
		firstQuotes := h1Quotes
		firstAccounts := h1Accounts
		firstAfterRaw := h1AfterRaw
		firstRaw := append([]map[string]string{}, h1RawWrites...)
		firstBalances := append([]map[string]string{}, rewardBalances...)
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("first simulation mutatedP1")
		}
		tx.Nonce = new(big.Int).Set(wide)
		rewardBalances = []map[string]string{}
		h1RawWrites = []map[string]string{}
		h1Quotes = []map[string]any{}
		second := runNativeSimulationCaptured(runner, block, c.name, tx)
		if !reflect.DeepEqual(firstQuotes, h1Quotes) || !reflect.DeepEqual(firstAccounts, h1Accounts) || !reflect.DeepEqual(firstAfterRaw, h1AfterRaw) {
			panic("copied post-state or quote differs on repeat")
		}
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("repeat simulation mutatedP1")
		}
		if !reflect.DeepEqual(firstRaw, h1RawWrites) {
			panic("raw probe differs on repeat")
		}
		if !reflect.DeepEqual(firstBalances, rewardBalances) {
			panic("balance probe differs on repeat")
		}
		if !reflect.DeepEqual(first, second) {
			panic("redelegation probe differs on repeat")
		}
		encoded, err := json.Marshal(first)
		if err != nil {
			panic(err)
		}
		var row map[string]any
		if err = json.Unmarshal(encoded, &row); err != nil {
			panic(err)
		}
		row["ordered_balance_effects"] = firstBalances
		row["ordered_raw_writes"] = firstRaw
		row["quotes"] = firstQuotes
		row["accounts"] = firstAccounts
		row["prior_raw"] = priorRaw
		row["after_raw"] = firstAfterRaw
		for _, w := range firstRaw {
			if _, ok := priorRaw[w["key"]]; !ok {
				panic("unmeasured raw mutation")
			}
		}
		cases = append(cases, row)
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("simulation changed committedP1")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "caller": hex.EncodeToString(nativeSimulationSender[:]), "owner": hex.EncodeToString(nativeSimulationDelegator[:]), "key_provenance": "known fixture keys31/A1 repeated32 derive identities only; no ordinary or principal P1 transactions", "reward_minted": "800", "state_before": before, "state_after": after, "cases": cases, "repeat_identical": true, "committed_state_unchanged": true, "state_unchanged_after_each": true, "go_native_period": 1, "go_evm_block": 1, "scope": "actualcanonicalP1rewardcommit and sameH1noargClaimAllDryRunner; completephysicalidentity; no Rustroot/productionacceptance"}); err != nil {
		panic(err)
	}
}
