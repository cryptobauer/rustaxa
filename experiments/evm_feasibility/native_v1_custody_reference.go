// Pinned V1 undelegation custody oracle. The harness pairs this exporter with
// the versioned in-memory StateTransition support from native_simulation_reference.go
// and an archive-only observer at the raw-storage write boundary.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

var (
	custodyDelegator = common.HexToAddress("0x00000000000000000000000000000000000000d1")
	custodyOwner     = common.HexToAddress("0x00000000000000000000000000000000000000a1")
	custodyValidator = common.HexToAddress("0x0000000000000000000000000000000000000031")
)

type custodyRawWrite struct {
	Address string `json:"address"`
	Key     string `json:"key"`
	Value   string `json:"value"`
}

var custodyWrites struct {
	sync.Mutex
	values []custodyRawWrite
}

func beginCustodyWrites() {
	custodyWrites.Lock()
	custodyWrites.values = make([]custodyRawWrite, 0)
	custodyWrites.Unlock()
	state_evm.SetNativeV1CustodyRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		custodyWrites.Lock()
		custodyWrites.values = append(custodyWrites.values, custodyRawWrite{
			Address: hex.EncodeToString(address[:]), Key: hex.EncodeToString(key[:]), Value: hex.EncodeToString(value),
		})
		custodyWrites.Unlock()
	})
}

func finishCustodyWrites() []custodyRawWrite {
	state_evm.SetNativeV1CustodyRawWriteObserver(nil)
	custodyWrites.Lock()
	defer custodyWrites.Unlock()
	result := append([]custodyRawWrite(nil), custodyWrites.values...)
	custodyWrites.values = nil
	return result
}

type custodyLog struct {
	Address string   `json:"address"`
	Topics  []string `json:"topics"`
	Data    string   `json:"data"`
}

type custodyState struct {
	Period           uint64 `json:"period"`
	DelegatorBalance string `json:"delegator_balance"`
	ContractBalance  string `json:"contract_balance"`
	ValidatorStake   string `json:"validator_stake"`
	TotalDelegated   string `json:"total_delegated"`
	ValidatorExists  bool   `json:"validator_exists"`
}

type custodyTransaction struct {
	Name           string            `json:"name"`
	Selector       string            `json:"selector"`
	Nonce          string            `json:"nonce"`
	GasUsed        uint64            `json:"gas_used"`
	ConsensusError string            `json:"consensus_error"`
	ExecutionError string            `json:"execution_error"`
	Output         string            `json:"output"`
	Logs           []custodyLog      `json:"logs"`
	RawWrites      []custodyRawWrite `json:"ordered_raw_writes"`
}

type custodyScenario struct {
	Name            string               `json:"name"`
	Amount          string               `json:"amount"`
	Magnolia        bool                 `json:"magnolia"`
	Ficus           bool                 `json:"ficus"`
	CornusLock      uint32               `json:"cornus_lock"`
	Cacti           bool                 `json:"cacti"`
	CactiLock       uint32               `json:"cacti_lock"`
	UnlockPeriod    uint64               `json:"unlock_period"`
	ObjectKey       string               `json:"v1_object_key"`
	IterablePrefix  string               `json:"v1_iterable_prefix"`
	Before          custodyState         `json:"before"`
	AfterUndelegate custodyState         `json:"after_undelegate_block"`
	AfterConfirm    custodyState         `json:"after_confirm_block"`
	Transactions    []custodyTransaction `json:"transactions"`
	CommittedRoot   string               `json:"committed_root"`
}

func custodyConfig(magnolia, ficus, cacti bool) chain_config.ChainConfig {
	balances := core.BalanceMap{custodyDelegator: big.NewInt(2_000), custodyOwner: big.NewInt(1_000)}
	maxSupply := big.NewInt(3_000)
	activation := func(enabled bool) types.BlockNum {
		if enabled {
			return 0
		}
		return types.BlockNumberNIL
	}
	return chain_config.ChainConfig{
		EVMChainConfig: params.ChainConfig{ChainId: 666}, GenesisBalances: balances,
		DPOS: chain_config.DPOSConfig{
			EligibilityBalanceThreshold: big.NewInt(100), VoteEligibilityBalanceStep: big.NewInt(10),
			ValidatorMaximumStake: big.NewInt(1_000_000), MinimumDeposit: big.NewInt(1),
			DelegationDelay: 1, DelegationLockingPeriod: 2, BlocksPerYear: 1, YieldPercentage: 0,
			InitialValidators: []chain_config.GenesisValidator{{
				Address: custodyValidator, Owner: custodyOwner, VrfKey: bytes.Repeat([]byte{0x44}, 32),
				Commission: 100, Delegations: core.BalanceMap{custodyDelegator: big.NewInt(1_000)},
			}},
		},
		Hardforks: chain_config.HardforksConfig{
			FixRedelegateBlockNum: 0, FixClaimAllBlockNum: 0,
			RewardsDistributionFrequency: map[uint64]uint32{0: 1},
			MagnoliaHf:                   chain_config.MagnoliaHfConfig{BlockNum: activation(magnolia), JailTime: 1},
			AspenHf: chain_config.AspenHfConfig{
				BlockNumPartOne: 0, BlockNumPartTwo: types.BlockNumberNIL,
				MaxSupply: maxSupply, GeneratedRewards: new(big.Int),
			},
			FicusHf: chain_config.FicusHfConfig{BlockNum: activation(ficus), PillarBlocksInterval: 1_000},
			CornusHf: chain_config.CornusHfConfig{
				BlockNum: 0, DelegationLockingPeriod: 3, DagGasLimit: 1_000_000, PbftGasLimit: 1_000_000,
			},
			SoleiroliaHf: chain_config.SoleiroliaHfConfig{BlockNum: types.BlockNumberNIL},
			CactiHf: chain_config.CactiHfConfig{
				BlockNum: activation(cacti), DelegationLockingPeriod: 7, JailTime: 1,
			},
		},
	}
}

func custodyCommittedState(database *nativeSimulationDB, cfg chain_config.ChainConfig, period uint64) custodyState {
	storageFactory := func(selected types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: database.GetBlockStateReader(selected)}
	}
	reader := new(dpos.API).Init(cfg).NewReader(types.BlockNum(period), storageFactory)
	accountReader := state_db.ExtendedReader{Reader: database.GetBlockStateReader(types.BlockNum(period))}
	balance := func(address common.Address) string {
		value := new(big.Int)
		accountReader.GetRawAccount(&address, func(encoded []byte) {
			value = state_db.DecodeAccountFromTrie(encoded).Balance
		})
		return value.String()
	}
	exists := len(reader.GetVrfKey(&custodyValidator)) != 0
	stake := new(big.Int)
	if exists {
		stake = reader.GetStakingBalance(&custodyValidator)
	}
	dposAddress := *dpos.ContractAddress()
	return custodyState{
		Period: period, DelegatorBalance: balance(custodyDelegator), ContractBalance: balance(dposAddress),
		ValidatorStake: stake.String(), TotalDelegated: reader.TotalAmountDelegated().String(), ValidatorExists: exists,
	}
}

func custodyInput(signature string, amount *big.Int) []byte {
	input := append(nativeSimulationSelector(signature), make([]byte, 12)...)
	input = append(input, custodyValidator[:]...)
	if amount != nil {
		word := make([]byte, 32)
		amount.FillBytes(word)
		input = append(input, word...)
	}
	return input
}

func runCustodyTransaction(transition *state_transition.StateTransition, nonce uint64, name, signature string, amount *big.Int) custodyTransaction {
	dposAddress := *dpos.ContractAddress()
	input := custodyInput(signature, amount)
	beginCustodyWrites()
	result := transition.ExecuteTransaction(&vm.Transaction{
		From: custodyDelegator, To: &dposAddress, Nonce: new(big.Int).SetUint64(nonce),
		GasPrice: new(big.Int), Gas: 200_000, Value: new(big.Int), Input: input,
	})
	writes := finishCustodyWrites()
	logs := make([]custodyLog, len(result.Logs))
	for index, log := range result.Logs {
		topics := make([]string, len(log.Topics))
		for topicIndex, topic := range log.Topics {
			topics[topicIndex] = hex.EncodeToString(topic[:])
		}
		logs[index] = custodyLog{Address: hex.EncodeToString(log.Address[:]), Topics: topics, Data: hex.EncodeToString(log.Data)}
	}
	return custodyTransaction{
		Name: name, Selector: hex.EncodeToString(input[:4]), Nonce: new(big.Int).SetUint64(nonce).String(), GasUsed: result.GasUsed,
		ConsensusError: string(result.ConsensusErr), ExecutionError: string(result.ExecutionErr),
		Output: hex.EncodeToString(result.CodeRetval), Logs: logs, RawWrites: writes,
	}
}

func advanceCustodyBlock(transition *state_transition.StateTransition, period uint64) {
	transition.BeginBlock(&vm.BlockInfo{Author: custodyValidator, GasLimit: 1_000_000, Difficulty: new(big.Int)})
	if uint64(transition.BlockNumber()) != period {
		panic("unexpected custody period")
	}
}

func finishCustodyBlock(transition *state_transition.StateTransition) {
	transition.EndBlock()
	transition.Commit()
}

func runCustodyScenario(name string, amount int64, magnolia, ficus, cacti bool) custodyScenario {
	cfg := custodyConfig(magnolia, ficus, cacti)
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	unlock := uint64(4)
	if cacti {
		unlock = 8
	}
	objectKey := contract_storage.Stor_k_1([]byte{3, 0}, custodyValidator[:], custodyDelegator[:])
	iterablePrefix := append([]byte{3, 1}, custodyDelegator[:]...)
	transactions := make([]custodyTransaction, 0, 5)

	advanceCustodyBlock(transition, 1)
	before := custodyCommittedState(database, cfg, 0)
	transactions = append(transactions,
		runCustodyTransaction(transition, 0, "undelegate", "undelegate(address,uint256)", big.NewInt(amount)),
		runCustodyTransaction(transition, 1, "duplicate_v1", "undelegate(address,uint256)", big.NewInt(1)),
		runCustodyTransaction(transition, 2, "early_confirm", "confirmUndelegate(address)", nil),
	)
	finishCustodyBlock(transition)
	afterUndelegate := custodyCommittedState(database, cfg, 1)
	for period := uint64(2); period < unlock; period++ {
		advanceCustodyBlock(transition, period)
		finishCustodyBlock(transition)
	}
	advanceCustodyBlock(transition, unlock)
	transactions = append(transactions,
		runCustodyTransaction(transition, 3, "mature_confirm", "confirmUndelegate(address)", nil),
		runCustodyTransaction(transition, 4, "missing_confirm", "confirmUndelegate(address)", nil),
	)
	finishCustodyBlock(transition)
	afterConfirm := custodyCommittedState(database, cfg, unlock)
	return custodyScenario{
		Name: name, Amount: big.NewInt(amount).String(), Magnolia: magnolia, Ficus: ficus,
		CornusLock: 3, Cacti: cacti, CactiLock: 7, UnlockPeriod: unlock,
		ObjectKey: hex.EncodeToString(objectKey[:]), IterablePrefix: hex.EncodeToString(iterablePrefix),
		Before: before, AfterUndelegate: afterUndelegate, AfterConfirm: afterConfirm, Transactions: transactions,
		CommittedRoot: hex.EncodeToString(database.descriptor.StateRoot[:]),
	}
}

func main() {
	document := map[string]any{
		"schema":     1,
		"selectors":  map[string]string{"undelegate": "4d99dd16", "confirm_undelegate": "45a02561"},
		"action_gas": map[string]uint64{"undelegate": 60_000, "confirm_undelegate": 20_000},
		"scenarios": []custodyScenario{
			runCustodyScenario("magnolia_ficus_partial_cornus_lock", 300, true, true, false),
			runCustodyScenario("magnolia_ficus_terminal", 1_000, true, true, false),
			runCustodyScenario("magnolia_pre_ficus_partial", 300, true, false, false),
			runCustodyScenario("pre_magnolia_terminal", 1_000, false, true, false),
			runCustodyScenario("cacti_lock_priority", 300, true, true, true),
		},
	}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}
