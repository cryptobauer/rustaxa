// Actual claim failures at literal same-H1 over accepted reward-only P1.
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

func main() {
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
	contract := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	cases := []map[string]any{}
	for _, c := range []struct {
		name, selector               string
		validator                    byte
		gas                          uint64
		owner, malformed, nonpayable bool
	}{
		{name: "commission_wrong_owner", selector: "d0eebfe2", validator: 0x31, gas: 200000},
		{name: "delegator_missing", selector: "ef5cfb8c", validator: 0x99, gas: 200000},
		{name: "commission_missing", selector: "d0eebfe2", validator: 0x99, gas: 200000, owner: true},
		{name: "delegator_malformed", selector: "ef5cfb8c", validator: 0x31, gas: 200000, malformed: true},
		{name: "delegator_nonpayable_malformed", selector: "ef5cfb8c", validator: 0x31, gas: 200000, malformed: true, nonpayable: true},
		{name: "delegator_intrinsic", selector: "ef5cfb8c", validator: 0x31, gas: 21000},
	} {
		input := append(common.FromHex(c.selector), make([]byte, 32)...)
		input[35] = c.validator
		if c.malformed {
			input = input[:4]
		}
		value := new(big.Int)
		if c.nonpayable {
			value.SetInt64(1)
		}
		sender := nativeSimulationSender
		if c.owner {
			sender = nativeSimulationDelegator
		}
		tx := vm.Transaction{From: sender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: c.gas, Value: value, Input: input}
		rewardBalances = []map[string]string{}
		h1RawWrites = []map[string]string{}
		first := runNativeSimulationCaptured(runner, block, c.name, tx)
		firstRaw := append([]map[string]string{}, h1RawWrites...)
		firstBalances := append([]map[string]string{}, rewardBalances...)
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("first simulation mutatedP1")
		}
		tx.Nonce = new(big.Int).Set(wide)
		rewardBalances = []map[string]string{}
		h1RawWrites = []map[string]string{}
		second := runNativeSimulationCaptured(runner, block, c.name, tx)
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
		row["sender"] = hex.EncodeToString(sender[:])
		row["ordered_balance_effects"] = firstBalances
		row["ordered_raw_writes"] = firstRaw
		cases = append(cases, row)
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("simulation changed committedP1")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "caller": hex.EncodeToString(nativeSimulationSender[:]), "owner": hex.EncodeToString(nativeSimulationDelegator[:]), "key_provenance": "known fixture keys31/A1 repeated32 derive identities only; no ordinary or principal P1 transactions", "reward_minted": "800", "state_before": before, "state_after": after, "cases": cases, "repeat_identical": true, "committed_state_unchanged": true, "state_unchanged_after_each": true, "go_native_period": 1, "go_evm_block": 1, "scope": "actualcanonicalP1rewardcommit and six sameH1claimFailureDryRunner cases; completephysicalidentity; no Rustroot/productionacceptance"}); err != nil {
		panic(err)
	}
}
