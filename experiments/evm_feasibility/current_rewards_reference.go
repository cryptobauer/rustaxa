// Aspen-part-two reward-phase oracle, copied with the existing mixed-period
// memory database into disposable pinned taraxa-evm source archives.
package main

import (
	"encoding/hex"
	"math/big"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var (
	currentValidatorOne = common.HexToAddress("0x0000000000000000000000000000000000000031")
	currentValidatorTwo = common.HexToAddress("0x0000000000000000000000000000000000000041")
	currentDelegatorOne = common.HexToAddress("0x0000000000000000000000000000000000000032")
	currentDelegatorTwo = common.HexToAddress("0x0000000000000000000000000000000000000042")
	currentDpos         = common.HexToAddress("0x00000000000000000000000000000000000000fe")
)

type currentRawWrite struct {
	Address string `json:"address"`
	Key     string `json:"key"`
	Value   string `json:"value"`
}

var currentRawWrites struct {
	sync.Mutex
	writes []currentRawWrite
}

func beginCurrentRawWrites() {
	currentRawWrites.Lock()
	if currentRawWrites.writes != nil {
		currentRawWrites.Unlock()
		panic("raw-write trace already active")
	}
	currentRawWrites.writes = make([]currentRawWrite, 0)
	currentRawWrites.Unlock()
	state_evm.SetMixedPeriodRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		currentRawWrites.Lock()
		currentRawWrites.writes = append(currentRawWrites.writes, currentRawWrite{
			Address: hex.EncodeToString(address[:]), Key: hex.EncodeToString(key[:]), Value: hex.EncodeToString(value),
		})
		currentRawWrites.Unlock()
	})
}

func finishCurrentRawWrites() []currentRawWrite {
	state_evm.SetMixedPeriodRawWriteObserver(nil)
	currentRawWrites.Lock()
	defer currentRawWrites.Unlock()
	if currentRawWrites.writes == nil {
		panic("raw-write trace is not active")
	}
	writes := append(make([]currentRawWrite, 0, len(currentRawWrites.writes)), currentRawWrites.writes...)
	currentRawWrites.writes = nil
	return writes
}

func currentRewardsConfig() chain_config.ChainConfig {
	return chain_config.ChainConfig{
		EVMChainConfig: params.ChainConfig{ChainId: chainID},
		GenesisBalances: core.BalanceMap{
			currentDelegatorOne: big.NewInt(2_000), currentDelegatorTwo: big.NewInt(3_000),
		},
		DPOS: chain_config.DPOSConfig{
			EligibilityBalanceThreshold: big.NewInt(100), VoteEligibilityBalanceStep: big.NewInt(10),
			ValidatorMaximumStake: big.NewInt(1_000_000), MinimumDeposit: big.NewInt(1),
			MaxBlockAuthorReward: 10, DagProposersReward: 50, BlocksPerYear: 10, YieldPercentage: 1,
			InitialValidators: []chain_config.GenesisValidator{
				{Address: currentValidatorOne, Owner: currentDelegatorOne, VrfKey: make([]byte, 32), Commission: 100, Delegations: core.BalanceMap{currentDelegatorOne: big.NewInt(1_000)}},
				{Address: currentValidatorTwo, Owner: currentDelegatorTwo, VrfKey: make([]byte, 32), Commission: 2_500, Delegations: core.BalanceMap{currentDelegatorTwo: big.NewInt(2_000)}},
			},
		},
		Hardforks: chain_config.HardforksConfig{
			FixRedelegateBlockNum: maxPeriod, RewardsDistributionFrequency: map[uint64]uint32{0: 1},
			MagnoliaHf:   chain_config.MagnoliaHfConfig{BlockNum: 0},
			AspenHf:      chain_config.AspenHfConfig{BlockNumPartOne: 0, BlockNumPartTwo: 1, MaxSupply: big.NewInt(6_000), GeneratedRewards: new(big.Int)},
			FicusHf:      chain_config.FicusHfConfig{BlockNum: 0},
			CornusHf:     chain_config.CornusHfConfig{BlockNum: 0},
			SoleiroliaHf: chain_config.SoleiroliaHfConfig{BlockNum: maxPeriod},
			CactiHf:      chain_config.CactiHfConfig{BlockNum: maxPeriod},
		},
	}
}

func zeroYieldRewardsConfig(sender common.Address) chain_config.ChainConfig {
	cfg := currentRewardsConfig()
	cfg.DPOS.YieldPercentage = 0
	cfg.GenesisBalances[sender] = big.NewInt(1_000_000)
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1_006_000)
	return cfg
}

func currentRewardSlots(reader state_db.ExtendedReader) []map[string]any {
	keys := []struct {
		name string
		key  *common.Hash
	}{
		{"validator_one_rewards", contract_storage.Stor_k_1([]byte{0, 2}, currentValidatorOne[:])},
		{"validator_two_rewards", contract_storage.Stor_k_1([]byte{0, 2}, currentValidatorTwo[:])},
		{"eligible_vote_count", contract_storage.Stor_k_1([]byte{4})},
		{"amount_delegated", contract_storage.Stor_k_1([]byte{5})},
		{"minted_tokens", contract_storage.Stor_k_1([]byte{6})},
		{"total_supply", contract_storage.Stor_k_1([]byte{7})},
		{"current_yield", contract_storage.Stor_k_1([]byte{8})},
	}
	rows := make([]map[string]any, 0, len(keys))
	for _, item := range keys {
		row := map[string]any{"name": item.name, "key": hex.EncodeToString(item.key[:]), "present": false, "value": ""}
		reader.GetAccountStorage(&currentDpos, item.key, func(value []byte) {
			row["present"] = true
			row["value"] = hex.EncodeToString(value)
		})
		rows = append(rows, row)
	}
	return rows
}

func currentStats(author common.Address, validator common.Address, fee int64) rewards_stats.RewardsStats {
	return rewards_stats.RewardsStats{
		BlockAuthor: author, BlocksPerYear: 10,
		ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{
			validator: {DagBlocksCount: 1, VoteWeight: 10, FeesRewards: big.NewInt(fee)},
		},
		TotalDagBlocksCount: 1, TotalVotesWeight: 10, MaxVotesWeight: 10,
	}
}

func runCurrentRewardsWitness() map[string]any {
	cfg := currentRewardsConfig()
	latest := newMemoryLatest()
	transition := newStateTransition(latest, &cfg)
	defer transition.Close()
	genesis := state_db.ExtendedReader{Reader: latest.readerAt(0)}
	genesisDescriptor := latest.GetCommittedDescriptor()
	before := map[string]any{
		"descriptor":   map[string]any{"period": 0, "root": hex.EncodeToString(genesisDescriptor.StateRoot[:])},
		"dpos_account": observeAccountReader(genesis, currentDpos), "slots": currentRewardSlots(genesis),
	}

	transition.BeginBlock(&vm.BlockInfo{Author: currentValidatorOne, GasLimit: blockGas, Difficulty: new(big.Int)})
	firstStats := currentStats(currentValidatorOne, currentValidatorOne, 11)
	secondStats := currentStats(currentValidatorTwo, currentValidatorTwo, 17)
	beginCurrentRawWrites()
	firstMinted := transition.DistributeRewards(&firstStats)
	firstWrites := finishCurrentRawWrites()
	beginCurrentRawWrites()
	secondMinted := transition.DistributeRewards(&secondStats)
	secondWrites := finishCurrentRawWrites()
	beginCurrentRawWrites()
	transition.EndBlock()
	endBlockWrites := finishCurrentRawWrites()
	root := transition.Commit()
	afterReader := state_db.ExtendedReader{Reader: latest.readerAt(1)}
	dposReader := currentDposReader(latest, &cfg, 1)
	after := map[string]any{
		"descriptor":   map[string]any{"period": 1, "root": hex.EncodeToString(root[:])},
		"dpos_account": observeAccountReader(afterReader, currentDpos), "slots": currentRewardSlots(afterReader),
		"total_supply": dposReader.GetTotalSupply().String(), "current_yield": dposReader.GetYield(),
	}
	if firstMinted == nil || secondMinted == nil {
		panic("reward distribution unexpectedly disabled")
	}
	return map[string]any{
		"schema": 1,
		"scope":  "synthetic Aspen-part-two activation at period 1; two actual DistributeRewards calls and one EndBlockCall; no snapshot, production route, jailed cleanup, redelegation fix, or map-order claim",
		"configuration": map[string]any{
			"period": 1, "chain_id": chainID, "genesis_balance_sum": "5000", "aspen_generated_rewards": "0",
			"aspen_max_supply": "6000", "aspen_part_one": 0, "aspen_part_two": 1, "cacti": maxPeriod,
			"blocks_per_year": 10, "yield_percentage_gate": 1, "max_author_reward_percent": 10, "dag_reward_percent": 50,
		},
		"validators": []map[string]any{
			{"address": hex.EncodeToString(currentValidatorOne[:]), "delegator": hex.EncodeToString(currentDelegatorOne[:]), "stake": "1000", "commission": 100},
			{"address": hex.EncodeToString(currentValidatorTwo[:]), "delegator": hex.EncodeToString(currentDelegatorTwo[:]), "stake": "2000", "commission": 2_500},
		},
		"before": before,
		"distributions": []map[string]any{
			{"index": 0, "validator": hex.EncodeToString(currentValidatorOne[:]), "fee": "11", "minted": firstMinted.ToBig().String(), "ordered_raw_writes": firstWrites},
			{"index": 1, "validator": hex.EncodeToString(currentValidatorTwo[:]), "fee": "17", "minted": secondMinted.ToBig().String(), "ordered_raw_writes": secondWrites},
		},
		"end_block_ordered_raw_writes": endBlockWrites,
		"total_minted":                 new(big.Int).Add(firstMinted.ToBig(), secondMinted.ToBig()).String(),
		"after":                        after,
		"map_order":                    "each ValidatorsStats map intentionally has one entry; distribution slice order is exact, while Go multi-entry map iteration is not a stable protocol order",
		"ordering_evidence": map[string]any{
			"go_sources":               []string{"taraxa/state/state_transition/state_transition.go:StateTransition.DistributeRewards/EndBlock", "taraxa/state/contracts/dpos/precompiled/dpos_contract.go:Contract.DistributeRewards/EndBlockCall", "taraxa/state/contracts/dpos/precompiled/dpos_hardforks.go:Contract.processBlockReward"},
			"go_sequence":              "inside each ValidatorsStats map iteration, fee balance addition and that validator's reward-row write precede accumulation of minted total; contract balance receives the accumulated minted total and Aspen total supply is saved after the map loop",
			"commutative_final_state":  "for distinct live validators, reward rows are disjoint and fee/minted balance additions are commutative; Aspen supply uses only the summed minted total, so every map iteration order has the same final logical rows, account balance, supply, and state root",
			"exact_trace_limit":        "raw reward-row writes and intermediate account replacements follow Go runtime map order and therefore are not stable for a multi-entry map",
			"physical_retention_limit": "this bounded witness does not prove equality of retained intermediate trie nodes or other physical database artifacts across multi-entry map orders; full acceptance must inventory those artifacts independently even though the committed root is order-independent",
		},
		"zero_yield_end_block": runZeroYieldEndBlockWitness(),
	}
}

func runZeroYieldEndBlockWitness() map[string]any {
	sender := testSender()
	cfg := zeroYieldRewardsConfig(sender)
	latest := newMemoryLatest()
	transition := newStateTransition(latest, &cfg)
	defer transition.Close()
	genesis := state_db.ExtendedReader{Reader: latest.readerAt(0)}
	beforeDescriptor := latest.GetCommittedDescriptor()
	before := map[string]any{
		"descriptor":   map[string]any{"period": 0, "root": hex.EncodeToString(beforeDescriptor.StateRoot[:])},
		"dpos_account": observeAccountReader(genesis, currentDpos), "slots": currentRewardSlots(genesis),
	}

	transition.BeginBlock(&vm.BlockInfo{Author: currentValidatorOne, GasLimit: blockGas, Difficulty: new(big.Int)})
	delegateInput := append([]byte{0x5c, 0x19, 0xa9, 0x5c}, make([]byte, 12)...)
	delegateInput = append(delegateInput, currentValidatorOne[:]...)
	spec := transactionSpec{Name: "delegate", Nonce: 0, GasPrice: 1, Gas: transactionGas, To: &currentDpos, Value: 100, Input: delegateInput}
	signed := signTransaction(spec, sender, testPrivateKey)
	tx := transactionFromSigned(spec, signed, sender)
	result := transition.ExecuteTransaction(&tx)
	transaction := transactionRow(spec, signed, tx, result, transition.GetEvmState().GetRefund())

	stats := currentStats(currentValidatorOne, currentValidatorOne, 11)
	beginCurrentRawWrites()
	minted := transition.DistributeRewards(&stats)
	distributionWrites := finishCurrentRawWrites()
	if minted != nil {
		panic("zero configured yield unexpectedly called reward distribution")
	}
	beginCurrentRawWrites()
	transition.EndBlock()
	endBlockWrites := finishCurrentRawWrites()
	root := transition.Commit()
	afterReader := state_db.ExtendedReader{Reader: latest.readerAt(1)}
	after := map[string]any{
		"descriptor":   map[string]any{"period": 1, "root": hex.EncodeToString(root[:])},
		"dpos_account": observeAccountReader(afterReader, currentDpos), "slots": currentRewardSlots(afterReader),
	}
	return map[string]any{
		"configuration": map[string]any{"period": 1, "yield_percentage": 0, "aspen_part_two": 1},
		"transaction":   transaction,
		"before":        before, "distribution_return": nil, "distribution_ordered_raw_writes": distributionWrites,
		"end_block_ordered_raw_writes": endBlockWrites, "after": after,
		"contract": "RewardsEnabled is false, so StateTransition.DistributeRewards returns nil without calling the DPoS contract; EndBlock still flushes DPoS counters changed by the preceding delegate transaction",
	}
}

func currentDposReader(latest *memoryLatest, cfg *chain_config.ChainConfig, period uint64) dpos.Reader {
	storageFactory := func(block uint64) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(block)}
	}
	return new(dpos.API).Init(*cfg).NewReader(period, storageFactory)
}
