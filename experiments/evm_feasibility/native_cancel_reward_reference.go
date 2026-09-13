// Pinned cross-period accrued-reward cancellation oracle. The harness composes
// this exporter with the native simulation and V1 custody support sources, plus
// the guarded raw-storage observer installed in a disposable archive checkout.
package main

import (
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
)

type cancelRewardScenario struct {
	Name                     string               `json:"name"`
	Version                  string               `json:"version"`
	Timing                   string               `json:"timing"`
	Amount                   string               `json:"amount"`
	Before                   custodyState         `json:"before"`
	AfterUndelegate          *custodyState        `json:"after_undelegate,omitempty"`
	UndelegateEndBlockWrites []custodyRawWrite    `json:"undelegate_end_block_ordered_raw_writes"`
	AfterReward              custodyState         `json:"after_reward"`
	RewardMinted             string               `json:"reward_minted"`
	RewardWrites             []custodyRawWrite    `json:"reward_ordered_raw_writes"`
	RewardEndBlockWrites     []custodyRawWrite    `json:"reward_end_block_ordered_raw_writes"`
	AfterCancel              custodyState         `json:"after_cancel"`
	Transactions             []custodyTransaction `json:"transactions"`
}

func cancelRewardConfig() chain_config.ChainConfig {
	cfg := custodyConfig(true, true, false)
	cfg.DPOS.YieldPercentage = 20
	cfg.DPOS.BlocksPerYear = 1
	cfg.DPOS.MaxBlockAuthorReward = 0
	cfg.DPOS.DagProposersReward = 0
	return cfg
}

func accruedCancelRewardStats() rewards_stats.RewardsStats {
	return rewards_stats.RewardsStats{
		BlockAuthor: custodyValidator, BlocksPerYear: 1,
		ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{
			custodyValidator: {VoteWeight: 1, FeesRewards: new(big.Int)},
		},
		TotalVotesWeight: 1, MaxVotesWeight: 1,
	}
}

func runCancelRewardScenario(version string, samePeriod bool) cancelRewardScenario {
	cfg := cancelRewardConfig()
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	transactions := make([]custodyTransaction, 0, 2)

	advanceCustodyBlock(transition, 1)
	before := custodyCommittedState(database, cfg, 0)
	if version == "v1" {
		transactions = append(transactions, runCustodyTransaction(
			transition, 0, "undelegate_v1", "undelegate(address,uint256)", big.NewInt(300),
		))
	} else {
		transactions = append(transactions, runCustodyTransaction(
			transition, 0, "undelegate_v2", "undelegateV2(address,uint256)", big.NewInt(300),
		))
	}
	var afterUndelegate *custodyState
	var undelegateEndBlockWrites []custodyRawWrite
	rewardPeriod := uint64(1)
	if !samePeriod {
		beginCustodyWrites()
		transition.EndBlock()
		undelegateEndBlockWrites = finishCustodyWrites()
		transition.Commit()
		committed := custodyCommittedState(database, cfg, 1)
		afterUndelegate = &committed
		rewardPeriod = 2
		advanceCustodyBlock(transition, rewardPeriod)
	}
	stats := accruedCancelRewardStats()
	beginCustodyWrites()
	minted := transition.DistributeRewards(&stats)
	rewardWrites := finishCustodyWrites()
	beginCustodyWrites()
	transition.EndBlock()
	rewardEndBlockWrites := finishCustodyWrites()
	transition.Commit()
	afterReward := custodyCommittedState(database, cfg, rewardPeriod)

	cancelPeriod := rewardPeriod + 1
	advanceCustodyBlock(transition, cancelPeriod)
	if version == "v1" {
		transactions = append(transactions, runCustodyTransaction(
			transition, 1, "cancel_v1", "cancelUndelegate(address)", nil,
		))
	} else {
		transactions = append(transactions, runCustodyTransaction(
			transition, 1, "cancel_v2", "cancelUndelegateV2(address,uint64)", big.NewInt(1),
		))
	}
	finishCustodyBlock(transition)

	name := version + "_accrued_reward"
	timing := "separate_reward_period"
	if samePeriod {
		name = version + "_same_period_reward"
		timing = "undelegate_then_reward_same_period"
	}
	return cancelRewardScenario{
		Name: name, Version: version, Timing: timing, Amount: "300",
		Before: before, AfterUndelegate: afterUndelegate,
		UndelegateEndBlockWrites: undelegateEndBlockWrites,
		AfterReward:              afterReward, RewardMinted: minted.ToBig().String(), RewardWrites: rewardWrites,
		RewardEndBlockWrites: rewardEndBlockWrites, AfterCancel: custodyCommittedState(database, cfg, cancelPeriod),
		Transactions: transactions,
	}
}

func main() {
	document := map[string]any{
		"schema": 1,
		"scope":  "one live StateTransition for both P1 partial undelegation/P2 rewards/P3 cancellation and P1 undelegation+rewards/P2 cancellation",
		"configuration": map[string]any{
			"yield_percentage": 20, "blocks_per_year": 1, "commission": 100,
			"magnolia": 0, "ficus": 0, "cornus": 0, "cacti": "disabled",
		},
		"scenarios": []cancelRewardScenario{
			runCancelRewardScenario("v1", false),
			runCancelRewardScenario("v2", false),
			runCancelRewardScenario("v1", true),
			runCancelRewardScenario("v2", true),
		},
	}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}
