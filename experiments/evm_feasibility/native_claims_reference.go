// Pinned reward-claim oracle. The harness composes this exporter with the
// in-memory StateTransition and guarded raw-write observer support.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

var missingClaimValidator = common.HexToAddress("0x0000000000000000000000000000000000000099")

type claimState struct {
	Period           uint64 `json:"period"`
	DelegatorBalance string `json:"delegator_balance"`
	OwnerBalance     string `json:"owner_balance"`
	ContractBalance  string `json:"contract_balance"`
}

type claimScenario struct {
	Name                 string               `json:"name"`
	Before               claimState           `json:"before"`
	AfterReward          claimState           `json:"after_reward"`
	RewardMinted         string               `json:"reward_minted"`
	RewardWrites         []custodyRawWrite    `json:"reward_ordered_raw_writes"`
	RewardEndBlockWrites []custodyRawWrite    `json:"reward_end_block_ordered_raw_writes"`
	AfterClaims          claimState           `json:"after_claims"`
	Transactions         []custodyTransaction `json:"transactions"`
}

func committedClaimState(database *nativeSimulationDB, period uint64) claimState {
	reader := state_db.ExtendedReader{Reader: database.GetBlockStateReader(types.BlockNum(period))}
	balance := func(address common.Address) string {
		value := new(big.Int)
		reader.GetRawAccount(&address, func(encoded []byte) {
			value = state_db.DecodeAccountFromTrie(encoded).Balance
		})
		return value.String()
	}
	dposAddress := *dpos.ContractAddress()
	return claimState{Period: period, DelegatorBalance: balance(custodyDelegator), OwnerBalance: balance(custodyOwner), ContractBalance: balance(dposAddress)}
}

func runClaimTransaction(transition *state_transition.StateTransition, caller common.Address, nonce uint64, name, signature string, validator common.Address) custodyTransaction {
	dposAddress := *dpos.ContractAddress()
	input := append(nativeSimulationSelector(signature), make([]byte, 12)...)
	input = append(input, validator[:]...)
	beginCustodyWrites()
	result := transition.ExecuteTransaction(&vm.Transaction{From: caller, To: &dposAddress, Nonce: new(big.Int).SetUint64(nonce), GasPrice: new(big.Int), Gas: 200_000, Value: new(big.Int), Input: input})
	writes := finishCustodyWrites()
	logs := make([]custodyLog, len(result.Logs))
	for index, log := range result.Logs {
		topics := make([]string, len(log.Topics))
		for topicIndex, topic := range log.Topics {
			topics[topicIndex] = hex.EncodeToString(topic[:])
		}
		logs[index] = custodyLog{Address: hex.EncodeToString(log.Address[:]), Topics: topics, Data: hex.EncodeToString(log.Data)}
	}
	return custodyTransaction{Name: name, Selector: hex.EncodeToString(input[:4]), Nonce: new(big.Int).SetUint64(nonce).String(), GasUsed: result.GasUsed, ConsensusError: string(result.ConsensusErr), ExecutionError: string(result.ExecutionErr), Output: hex.EncodeToString(result.CodeRetval), Logs: logs, RawWrites: writes}
}

func claimRewardStats() rewards_stats.RewardsStats {
	return rewards_stats.RewardsStats{BlockAuthor: custodyValidator, BlocksPerYear: 1, ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{custodyValidator: {VoteWeight: 1, FeesRewards: new(big.Int)}}, TotalVotesWeight: 1, MaxVotesWeight: 1}
}

func runClaimScenario(name string, commission bool) claimScenario {
	cfg := cancelRewardConfig()
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	before := committedClaimState(database, 0)
	advanceCustodyBlock(transition, 1)
	stats := claimRewardStats()
	beginCustodyWrites()
	minted := transition.DistributeRewards(&stats)
	rewardWrites := finishCustodyWrites()
	beginCustodyWrites()
	transition.EndBlock()
	rewardEndBlockWrites := finishCustodyWrites()
	transition.Commit()
	afterReward := committedClaimState(database, 1)
	advanceCustodyBlock(transition, 2)
	transactions := make([]custodyTransaction, 0, 2)
	if commission {
		transactions = append(transactions,
			runClaimTransaction(transition, custodyOwner, 0, "claim_commission_nonzero", "claimCommissionRewards(address)", custodyValidator),
			runClaimTransaction(transition, custodyOwner, 1, "claim_commission_zero_repeat", "claimCommissionRewards(address)", custodyValidator))
	} else {
		transactions = append(transactions,
			runClaimTransaction(transition, custodyDelegator, 0, "claim_rewards_nonzero", "claimRewards(address)", custodyValidator),
			runClaimTransaction(transition, custodyDelegator, 1, "claim_rewards_zero_repeat", "claimRewards(address)", custodyValidator))
	}
	finishCustodyBlock(transition)
	return claimScenario{Name: name, Before: before, AfterReward: afterReward, RewardMinted: minted.ToBig().String(), RewardWrites: rewardWrites, RewardEndBlockWrites: rewardEndBlockWrites, AfterClaims: committedClaimState(database, 2), Transactions: transactions}
}

func runClaimErrors() []custodyTransaction {
	cfg := cancelRewardConfig()
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	advanceCustodyBlock(transition, 1)
	transactions := []custodyTransaction{
		runClaimTransaction(transition, custodyDelegator, 0, "claim_rewards_missing_delegation", "claimRewards(address)", missingClaimValidator),
		runClaimTransaction(transition, custodyDelegator, 1, "claim_commission_wrong_owner", "claimCommissionRewards(address)", custodyValidator),
		runClaimTransaction(transition, custodyOwner, 0, "claim_commission_missing_validator", "claimCommissionRewards(address)", missingClaimValidator),
	}
	finishCustodyBlock(transition)
	return transactions
}

func main() {
	document := map[string]any{"schema": 1, "selectors": map[string]string{"claim_rewards": "ef5cfb8c", "claim_commission_rewards": "d0eebfe2"}, "action_gas": map[string]uint64{"claim_rewards": 40_000, "claim_commission_rewards": 20_000}, "configuration": map[string]any{"yield_percentage": 20, "blocks_per_year": 1, "commission": 100, "magnolia": 0, "ficus": 0, "cornus": 0, "cacti": "disabled"}, "scenarios": []claimScenario{runClaimScenario("delegator_accrued", false), runClaimScenario("commission_accrued", true)}, "errors": runClaimErrors()}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}
