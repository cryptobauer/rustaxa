// Actual funded-only noarg claim-all H1 probes are generated here, then checked by the unchanged C++ search
// body in the Python harness. Each probe executes actual pinned DryRunner.Apply.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

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
	requests := []nativeSimulationCase{}
	for _, cap := range []uint64{200000, 111272, 111273} {
		name := "claim_all_H1_cap200000"
		if cap == 111272 {
			name = "claim_all_H1_minimum"
		}
		if cap == 111273 {
			name = "claim_all_H1_minimum_plus_one"
		}
		requests = append(requests, nativeSimulationCase{Name: name, To: hex.EncodeToString(dpos.ContractAddress()[:]), SuppliedNonce: new(big.Int).Lsh(big.NewInt(1), 512).String(), GasPrice: "0", Value: "0", Input: "0b83a727", Gas: cap})
	}
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := currentSignedSnapshot(seed, api)
	priorRaw := h1RawCopy(state_evm.GetBlockState(seed.database, 1, 1))
	cases := []map[string]any{}
	for _, request := range requests {
		input, err := hex.DecodeString(request.Input)
		if err != nil {
			panic(err)
		}
		to := common.HexToAddress(request.To)
		parse := func(s string) *big.Int {
			n, ok := new(big.Int).SetString(s, 10)
			if !ok {
				panic("bad integer")
			}
			return n
		}
		probes := []map[string]any{}
		probe := func(gas uint64) nativeSimulationOutput {
			transaction := vm.Transaction{From: nativeSimulationSender, To: &to, Nonce: parse(request.SuppliedNonce), GasPrice: parse(request.GasPrice), Gas: gas, Value: parse(request.Value), Input: input}
			rewardBalances = []map[string]string{}
			h1RawWrites = []map[string]string{}
			h1Quotes = []map[string]any{}
			first := runNativeSimulationCaptured(runner, block, request.Name, transaction)
			firstRaw := append([]map[string]string{}, h1RawWrites...)
			firstBalances := append([]map[string]string{}, rewardBalances...)
			firstQuotes := h1Quotes
			firstAccounts := h1Accounts
			firstAfterRaw := h1AfterRaw
			if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
				panic("first probe mutatedP1")
			}
			transaction.Nonce = parse(request.SuppliedNonce)
			rewardBalances = []map[string]string{}
			h1RawWrites = []map[string]string{}
			h1Quotes = []map[string]any{}
			second := runNativeSimulationCaptured(runner, block, request.Name, transaction)
			if !reflect.DeepEqual(firstRaw, h1RawWrites) || !reflect.DeepEqual(firstBalances, rewardBalances) || !reflect.DeepEqual(firstQuotes, h1Quotes) || !reflect.DeepEqual(firstAccounts, h1Accounts) || !reflect.DeepEqual(firstAfterRaw, h1AfterRaw) {
				panic("probe witness or copied state differs on repeat")
			}
			if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
				panic("repeat probe mutatedP1")
			}
			if !reflect.DeepEqual(first, second) {
				panic("probe isolation failed")
			}
			encoded, err := json.Marshal(first)
			if err != nil {
				panic(err)
			}
			var row map[string]any
			if err = json.Unmarshal(encoded, &row); err != nil {
				panic(err)
			}
			row["observation_witness"] = map[string]any{"raw": firstRaw, "balances": firstBalances}
			row["quotes"] = firstQuotes
			row["accounts"] = firstAccounts
			row["prior_raw"] = priorRaw
			row["after_raw"] = firstAfterRaw
			probes = append(probes, row)
			return first.Output
		}
		initial := probe(request.Gas)
		if initial.ConsensusError == "" && initial.ExecutionError == "" {
			low, high := initial.GasUsed, request.Gas
			if low > high {
				panic("invalid actual gas")
			}
			for high-low > high/20 {
				mid := low + (high-low)/2
				result := probe(mid)
				if result.ConsensusError != "" {
					break
				}
				if result.ExecutionError == "" {
					high = mid
				} else {
					low = mid
				}
			}
		}
		cases = append(cases, map[string]any{"name": request.Name, "cap": request.Gas, "probes": probes})
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("estimation changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "go_native_period": 1, "go_evm_block": 1, "repeat_identical": true, "cases": cases, "state_before": before, "state_after": after}); err != nil {
		panic(err)
	}
}
