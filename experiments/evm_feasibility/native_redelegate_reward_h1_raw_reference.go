// Actual canonical positive-reward committed P1 and same-H1 DryRunner observations.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"
)

var rewardBalances = []map[string]string{}

func signedH1Raw(reader storage.StorageReader) map[string]any {
	keys := map[common.Hash]bool{}
	add := func(parts ...[]byte) { keys[*storage.Stor_k_1(parts...)] = true }
	for _, prefix := range [][]byte{{0, 5}, append([]byte{2, 1}, nativeSimulationSender[:]...), append([]byte{2, 1}, nativeSimulationDelegator[:]...)} {
		add(prefix, []byte{1})
		for _, last := range []byte{0x31, 0x32, 0x33} {
			v := common.BytesToAddress([]byte{last})
			add(prefix, []byte{2}, v[:])
		}
		for i := byte(1); i <= 4; i++ {
			add(prefix, []byte{2}, []byte{i, 0, 0, 0})
		}
	}
	for _, last := range []byte{0x31, 0x32, 0x33} {
		v := common.BytesToAddress([]byte{last})
		add([]byte{0, 0}, v[:])
		add([]byte{0, 3}, v[:])
		add([]byte{0, 2}, v[:])
		add([]byte{1}, v[:], []byte{})
		add([]byte{1}, v[:], []byte{1})
		add([]byte{1}, v[:], []byte{2})
		add([]byte{2, 0}, v[:], nativeSimulationSender[:])
		add([]byte{2, 0}, v[:], nativeSimulationDelegator[:])
	}
	for _, field := range []byte{4, 5, 6, 7} {
		add([]byte{field})
	}
	out := map[string]any{}
	a := *dpos.ContractAddress()
	for key := range keys {
		value := []byte{}
		present := false
		reader.GetAccountStorage(&a, &key, func(v []byte) {
			if len(v) > 0 {
				present = true
				value = common.CopyBytes(v)
			}
		})
		out[hex.EncodeToString(key[:])] = map[string]any{"present": present, "value": hex.EncodeToString(value)}
	}
	return out
}
func signedH1Facts(reader storage.StorageReader) map[string]any {
	result := swapAppendSimulationFacts(reader)
	address := *dpos.ContractAddress()
	wrapper := new(storage.StorageWrapper)
	wrapper.StorageReaderWrapper.Init(&address, reader)
	validators := new(dpos.Validators)
	validators.Init(wrapper, []byte{0})
	rows := map[string]any{}
	for _, last := range []byte{0x31, 0x32, 0x33} {
		v := common.BytesToAddress([]byte{last})
		row := validators.GetValidator(&v)
		if row == nil {
			rows[hex.EncodeToString(v[:])] = nil
			continue
		}
		pool := validators.GetValidatorRewards(&v)
		rows[hex.EncodeToString(v[:])] = map[string]any{"stake": row.TotalStake.String(), "head": uint64(row.LastUpdated), "rewards": pool.RewardsPool.String(), "commission_rewards": pool.CommissionRewardsPool.String()}
	}
	result["validators"] = rows
	return result
}
func currentSignedSnapshot(seed nativeSimulationSeed, api *dpos.API) map[string]any {
	snapshot := nativeSimulationSnapshot(seed, api)
	reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(1)}
	snapshot["configuration"] = seed.config
	snapshot["native_facts"] = signedH1Facts(reader)
	snapshot["native_raw"] = signedH1Raw(reader)
	return snapshot
}
func seedRewardH1() nativeSimulationSeed {
	database := newNativeSimulationDB()
	cfg := nativeSimulationConfig()
	cfg.DPOS.MinimumDeposit = big.NewInt(100)
	cfg.DPOS.YieldPercentage = 20
	cfg.DPOS.MaxBlockAuthorReward = 0
	cfg.DPOS.DagProposersReward = 0
	cfg.Hardforks.AspenHf.BlockNumPartOne = 0
	cfg.Hardforks.AspenHf.BlockNumPartTwo = ^uint64(0)
	cfg.DPOS.DelegationLockingPeriod = 2
	cfg.Hardforks.CornusHf.DelegationLockingPeriod = 3
	cfg.Hardforks.CactiHf.DelegationLockingPeriod = 7
	cfg.GenesisBalances = core.BalanceMap{nativeSimulationSender: big.NewInt(5000), nativeSimulationDelegator: big.NewInt(3000)}
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(8000)
	cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{}
	for _, last := range []byte{0x31, 0x32} {
		vrf := byte(0x44)
		if last == 0x32 {
			vrf = 0x55
		} else if last == 0x33 {
			vrf = 0x66
		}
		delegations := core.BalanceMap{nativeSimulationDelegator: big.NewInt(1000)}
		delegations[nativeSimulationSender] = big.NewInt(1000)
		cfg.DPOS.InitialValidators = append(cfg.DPOS.InitialValidators, chain_config.GenesisValidator{Address: common.BytesToAddress([]byte{last}), Owner: nativeSimulationDelegator, VrfKey: bytes.Repeat([]byte{vrf}, 32), Commission: 100, Delegations: delegations})
	}
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)})
	stats := rewards_stats.RewardsStats{BlockAuthor: nativeSimulationValidator, BlocksPerYear: 1, ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{nativeSimulationValidator: {VoteWeight: 1, FeesRewards: new(big.Int)}}, TotalVotesWeight: 1, MaxVotesWeight: 1}
	minted := transition.DistributeRewards(&stats)
	if minted.String() != "800" {
		panic("unexpected mint")
	}
	transition.EndBlock()
	transition.Commit()
	if database.descriptor.BlockNum != 1 {
		panic("seed not committedP1")
	}
	return nativeSimulationSeed{database: database, config: cfg, wrapper: nativeSimulationDelegator}
}

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
		name                  string
		from, to              byte
		amount                int64
		gas                   uint64
		malformed, nonpayable bool
	}{
		{name: "partial", from: 0x31, to: 0x32, amount: 300, gas: 200000},
		{name: "missing_source", from: 0x99, to: 0x32, amount: 300, gas: 200000},
		{name: "missing_destination", from: 0x31, to: 0x99, amount: 300, gas: 200000},
		{name: "insufficient_source", from: 0x31, to: 0x32, amount: 1100, gas: 200000},
		{name: "remainder_below_minimum", from: 0x31, to: 0x32, amount: 950, gas: 200000},
		{name: "same_validator", from: 0x31, to: 0x31, amount: 300, gas: 200000},
		{name: "malformed", from: 0x31, to: 0x32, gas: 200000, malformed: true},
		{name: "nonpayable_malformed", from: 0x31, to: 0x32, gas: 200000, malformed: true, nonpayable: true},
		{name: "insufficient_native_gas", from: 0x31, to: 0x32, amount: 300, gas: 30000},
		{name: "intrinsic_gas", from: 0x31, to: 0x32, amount: 300, gas: 21000},
	} {
		input := redelegateSimulationABI(common.BytesToAddress([]byte{c.from}), common.BytesToAddress([]byte{c.to}), c.amount)
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
		row["ordered_balance_effects"] = firstBalances
		row["ordered_raw_writes"] = firstRaw
		cases = append(cases, row)
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("simulation changed committedP1")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "caller": hex.EncodeToString(nativeSimulationSender[:]), "owner": hex.EncodeToString(nativeSimulationDelegator[:]), "key_provenance": "known fixture keys31/A1 repeated32 derive identities only; no ordinary or principal P1 transactions", "reward_minted": "800", "state_before": before, "state_after": after, "cases": cases, "repeat_identical": true, "committed_state_unchanged": true, "state_unchanged_after_each": true, "go_native_period": 1, "go_evm_block": 1, "scope": "actualcanonicalP1rewardcommit and sameH1DryRunner; completephysicalidentity; no Rustroot/productionacceptance"}); err != nil {
		panic(err)
	}
}
