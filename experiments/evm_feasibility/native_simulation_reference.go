// Pinned native-call simulation oracle. The Python harness runs this exporter
// from disposable public/local taraxa-evm source archives. It seeds a complete
// versioned trie through StateTransition, then invokes the actual
// state_dry_runner.DryRunner.Apply path through an ordinary wrapper contract.
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"reflect"
	"sort"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

const (
	nativeSimulationPeriod    types.BlockNum = 1
	nativeSimulationGas                      = uint64(500_000)
	nativeSimulationMaxPeriod                = ^uint64(0)
)

var (
	nativeSimulationSender    = common.HexToAddress("0x00000000000000000000000000000000000000aa")
	nativeSimulationDelegator = common.HexToAddress("0x00000000000000000000000000000000000000bb")
	nativeSimulationValidator = common.HexToAddress("0x0000000000000000000000000000000000000031")
	nativeSimulationMissing   = common.HexToAddress("0x0000000000000000000000000000000000000099")
)

type nativeSimulationRows struct {
	mu       sync.Mutex
	period   uint64
	latest   [state_db.COL_COUNT]map[common.Hash][]byte
	physical [state_db.COL_COUNT]map[string][]byte
}

func newNativeSimulationRows() *nativeSimulationRows {
	rows := new(nativeSimulationRows)
	for column := range rows.latest {
		rows.latest[column] = make(map[common.Hash][]byte)
		rows.physical[column] = make(map[string][]byte)
	}
	return rows
}

func (rows *nativeSimulationRows) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	rows.mu.Lock()
	value, present := rows.latest[column][*key]
	value = common.CopyBytes(value)
	rows.mu.Unlock()
	if present {
		callback(value)
	}
}

func (rows *nativeSimulationRows) Put(column state_db.Column, key *common.Hash, value []byte) {
	rows.mu.Lock()
	defer rows.mu.Unlock()
	rows.latest[column][*key] = common.CopyBytes(value)
	physicalKey := append([]byte(nil), key[:]...)
	if column == state_db.COL_main_trie_value || column == state_db.COL_acc_trie_value {
		version := make([]byte, 8)
		binary.BigEndian.PutUint64(version, rows.period)
		physicalKey = append(physicalKey, version...)
	}
	rows.physical[column][string(physicalKey)] = common.CopyBytes(value)
}

type nativeSimulationPending struct {
	latest  *nativeSimulationDB
	number  types.BlockNum
	pending [state_db.COL_COUNT]map[common.Hash][]byte
	mu      sync.Mutex
}

func (pending *nativeSimulationPending) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	pending.mu.Lock()
	value, present := pending.pending[column][*key]
	value = common.CopyBytes(value)
	pending.mu.Unlock()
	if present {
		callback(value)
		return
	}
	nativeSimulationHistorical{latest: pending.latest, period: pending.latest.descriptor.BlockNum}.Get(column, key, callback)
}

func (pending *nativeSimulationPending) Put(column state_db.Column, key *common.Hash, value []byte) {
	pending.mu.Lock()
	pending.pending[column][*key] = common.CopyBytes(value)
	pending.mu.Unlock()
	pending.latest.rows.Put(column, key, value)
}

func (pending *nativeSimulationPending) GetNumber() types.BlockNum { return pending.number }

type nativeSimulationDB struct {
	rows       *nativeSimulationRows
	descriptor state_db.StateDescriptor
	pending    *nativeSimulationPending
}

func newNativeSimulationDB() *nativeSimulationDB {
	return &nativeSimulationDB{
		rows: newNativeSimulationRows(),
		descriptor: state_db.StateDescriptor{
			BlockNum: types.BlockNumberNIL, StateRoot: common.ZeroHash,
		},
	}
}

func (database *nativeSimulationDB) GetCommittedDescriptor() state_db.StateDescriptor {
	return database.descriptor
}

func (database *nativeSimulationDB) BeginPendingBlock() state_db.PendingBlockState {
	number := database.descriptor.BlockNum + 1
	database.rows.mu.Lock()
	database.rows.period = uint64(number)
	database.rows.mu.Unlock()
	pending := &nativeSimulationPending{latest: database, number: number}
	for column := range pending.pending {
		pending.pending[column] = make(map[common.Hash][]byte)
	}
	database.pending = pending
	return pending
}

func (database *nativeSimulationDB) Commit(root common.Hash) error {
	if database.pending == nil {
		return fmt.Errorf("native simulation commit has no pending block")
	}
	database.descriptor = state_db.StateDescriptor{BlockNum: database.pending.number, StateRoot: root}
	database.pending = nil
	return nil
}

func (database *nativeSimulationDB) GetBlockStateReader(block types.BlockNum) state_db.Reader {
	return nativeSimulationHistorical{latest: database, period: block}
}

func (database *nativeSimulationDB) GetLatestState() state_db.LatestState { return database }

type nativeSimulationHistorical struct {
	latest *nativeSimulationDB
	period types.BlockNum
}

func (historical nativeSimulationHistorical) Get(column state_db.Column, key *common.Hash, callback func([]byte)) {
	if historical.latest.descriptor.BlockNum == types.BlockNumberNIL {
		return
	}
	if historical.period > historical.latest.descriptor.BlockNum {
		panic(fmt.Sprintf("historical read %d exceeds committed %d", historical.period, historical.latest.descriptor.BlockNum))
	}
	historical.latest.rows.mu.Lock()
	if column != state_db.COL_main_trie_value && column != state_db.COL_acc_trie_value {
		value, present := historical.latest.rows.latest[column][*key]
		value = common.CopyBytes(value)
		historical.latest.rows.mu.Unlock()
		if present && len(value) != 0 {
			callback(value)
		}
		return
	}
	var selected []byte
	var selectedPeriod uint64
	found := false
	for physicalKey, value := range historical.latest.rows.physical[column] {
		encodedKey := []byte(physicalKey)
		if len(encodedKey) != 40 || !bytes.Equal(encodedKey[:32], key[:]) {
			continue
		}
		period := binary.BigEndian.Uint64(encodedKey[32:])
		if period <= uint64(historical.period) && (!found || period > selectedPeriod) {
			selected = common.CopyBytes(value)
			selectedPeriod = period
			found = true
		}
	}
	historical.latest.rows.mu.Unlock()
	if found && len(selected) != 0 {
		callback(selected)
	}
}

func nativeSimulationConfig() chain_config.ChainConfig {
	senderBalance := new(big.Int).Exp(big.NewInt(10), big.NewInt(40), nil)
	delegatorBalance := big.NewInt(1_000_000)
	maxSupply := new(big.Int).Add(new(big.Int).Set(senderBalance), delegatorBalance)
	return chain_config.ChainConfig{
		EVMChainConfig: params.ChainConfig{ChainId: 666},
		GenesisBalances: core.BalanceMap{
			nativeSimulationSender: senderBalance, nativeSimulationDelegator: delegatorBalance,
		},
		DPOS: chain_config.DPOSConfig{
			EligibilityBalanceThreshold: big.NewInt(100), VoteEligibilityBalanceStep: big.NewInt(10),
			ValidatorMaximumStake: big.NewInt(1_000_000), MinimumDeposit: big.NewInt(1),
			MaxBlockAuthorReward: 10, DagProposersReward: 50,
			CommissionChangeDelta: 0, CommissionChangeFrequency: 0,
			DelegationDelay: 1, DelegationLockingPeriod: 1, BlocksPerYear: 1, YieldPercentage: 0,
			InitialValidators: []chain_config.GenesisValidator{{
				Address: nativeSimulationValidator, Owner: nativeSimulationSender,
				VrfKey: bytes.Repeat([]byte{0x44}, 32), Commission: 100,
				Delegations: core.BalanceMap{nativeSimulationDelegator: big.NewInt(90)},
			}},
		},
		Hardforks: chain_config.HardforksConfig{
			FixRedelegateBlockNum: 0, FixClaimAllBlockNum: 0, PhalaenopsisHfBlockNum: 0,
			RewardsDistributionFrequency: map[uint64]uint32{0: 1},
			MagnoliaHf:                   chain_config.MagnoliaHfConfig{BlockNum: 0, JailTime: 1},
			AspenHf: chain_config.AspenHfConfig{
				BlockNumPartOne: 0, BlockNumPartTwo: nativeSimulationMaxPeriod,
				MaxSupply: maxSupply, GeneratedRewards: new(big.Int),
			},
			FicusHf: chain_config.FicusHfConfig{BlockNum: 0, PillarBlocksInterval: 1_000},
			CornusHf: chain_config.CornusHfConfig{
				BlockNum: 0, DelegationLockingPeriod: 1,
				DagGasLimit: nativeSimulationGas, PbftGasLimit: nativeSimulationGas,
			},
			SoleiroliaHf: chain_config.SoleiroliaHfConfig{BlockNum: nativeSimulationMaxPeriod},
			CactiHf: chain_config.CactiHfConfig{
				BlockNum: nativeSimulationMaxPeriod, DelegationLockingPeriod: 1, JailTime: 1,
			},
		},
	}
}

func nativeSimulationTransition(database *nativeSimulationDB, config *chain_config.ChainConfig) *state_transition.StateTransition {
	api := new(dpos.API).Init(*config)
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: database.GetBlockStateReader(period)}
	}
	return new(state_transition.StateTransition).Init(
		database,
		func(types.BlockNum) *big.Int { return new(big.Int) },
		api,
		func(period types.BlockNum) dpos.Reader { return api.NewDelayedReader(period, storageFactory) },
		func(period types.BlockNum) slashing.Reader { return api.NewSlashingReader(period, storageFactory) },
		config,
		state_transition.Opts{EVMState: state_evm.Opts{NumTransactionsToBuffer: 1}},
	)
}

func nativeSimulationSelector(signature string) []byte {
	return crypto.Keccak256([]byte(signature))[:4]
}

func nativeSimulationWord(selector []byte) []byte {
	word := make([]byte, 32)
	copy(word, selector)
	return word
}

func nativeSimulationWrapperRuntime() []byte {
	var code []byte
	mstoreSelector := func(signature string) {
		code = append(code, 0x7f)
		code = append(code, nativeSimulationWord(nativeSimulationSelector(signature))...)
		code = append(code, 0x60, 0x00, 0x52)
	}
	mstoreValidator := func() {
		code = append(code, 0x73)
		code = append(code, nativeSimulationValidator[:]...)
		code = append(code, 0x60, 0x04, 0x52)
	}
	call := func(valueOpcode byte, outputOffset byte) {
		code = append(code,
			0x60, 0x20, 0x60, outputOffset, 0x60, 0x24, 0x60, 0x00,
			valueOpcode, 0x60, 0xfe, 0x5a, 0xf1, 0x50,
		)
	}

	mstoreSelector("delegate(address)")
	mstoreValidator()
	// Discard the delegate return area. CALLVALUE is forwarded to DPoS.
	code = append(code, 0x60, 0x00, 0x60, 0x00, 0x60, 0x24, 0x60, 0x00, 0x34, 0x60, 0xfe, 0x5a, 0xf1, 0x50)

	mstoreSelector("getTotalDelegation(address)")
	// ADDRESS becomes the delegator argument.
	code = append(code, 0x30, 0x60, 0x04, 0x52)
	call(0x5f, 0x40) // PUSH0 value, current staged query result at memory[0x40].

	mstoreSelector("getValidatorEligibleVotesCount(address)")
	mstoreValidator()
	call(0x5f, 0x60) // Delayed query result at memory[0x60].
	code = append(code, 0x60, 0x40, 0x60, 0x40, 0xf3)
	return code
}

func nativeSimulationInitCode(runtime []byte) []byte {
	if len(runtime) > 255 {
		panic("native simulation wrapper runtime exceeds PUSH1 length")
	}
	length := byte(len(runtime))
	init := []byte{0x60, length, 0x60, 0x0c, 0x60, 0x00, 0x39, 0x60, length, 0x60, 0x00, 0xf3}
	return append(init, runtime...)
}

type nativeSimulationSeed struct {
	database *nativeSimulationDB
	config   chain_config.ChainConfig
	wrapper  common.Address
}

func seedNativeSimulation() nativeSimulationSeed {
	database := newNativeSimulationDB()
	config := nativeSimulationConfig()
	transition := nativeSimulationTransition(database, &config)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Difficulty: new(big.Int)})

	wideNonce := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(5))
	transition.GetEvmState().GetAccount(&nativeSimulationSender).SetNonce(wideNonce)
	create := vm.Transaction{
		From: nativeSimulationSender, Nonce: new(big.Int).Set(wideNonce), GasPrice: big.NewInt(1),
		Gas: nativeSimulationGas, Value: new(big.Int), Input: nativeSimulationInitCode(nativeSimulationWrapperRuntime()),
	}
	created := transition.ExecuteTransaction(&create)
	if created.ConsensusErr != "" || created.ExecutionErr != "" {
		panic(fmt.Sprintf("wrapper deployment failed: consensus=%q execution=%q", created.ConsensusErr, created.ExecutionErr))
	}

	dposAddress := *dpos.ContractAddress()
	delegateInput := append(nativeSimulationSelector("delegate(address)"), make([]byte, 12)...)
	delegateInput = append(delegateInput, nativeSimulationValidator[:]...)
	delegate := vm.Transaction{
		From: nativeSimulationDelegator, To: &dposAddress, Nonce: new(big.Int), GasPrice: big.NewInt(1),
		Gas: 200_000, Value: big.NewInt(20), Input: delegateInput,
	}
	delegated := transition.ExecuteTransaction(&delegate)
	if delegated.ConsensusErr != "" || delegated.ExecutionErr != "" {
		panic(fmt.Sprintf("committed delegate failed: consensus=%q execution=%q", delegated.ConsensusErr, delegated.ExecutionErr))
	}
	transition.EndBlock()
	transition.Commit()
	if database.descriptor.BlockNum != nativeSimulationPeriod {
		panic(fmt.Sprintf("seed committed period %d", database.descriptor.BlockNum))
	}
	return nativeSimulationSeed{database: database, config: config, wrapper: created.NewContractAddr}
}

type nativeSimulationLog struct {
	Address string   `json:"address"`
	Topics  []string `json:"topics"`
	Data    string   `json:"data"`
}

type nativeSimulationOutput struct {
	EffectiveNonce string                `json:"effective_nonce"`
	GasUsed        uint64                `json:"gas_used"`
	ConsensusError string                `json:"consensus_error"`
	ExecutionError string                `json:"execution_error"`
	Return         string                `json:"return"`
	Logs           []nativeSimulationLog `json:"logs"`
}

type nativeSimulationCase struct {
	Name          string                 `json:"name"`
	To            string                 `json:"to"`
	SuppliedNonce string                 `json:"supplied_nonce"`
	GasPrice      string                 `json:"gas_price"`
	Gas           uint64                 `json:"gas"`
	Value         string                 `json:"value"`
	Input         string                 `json:"input"`
	ReferenceLog  string                 `json:"reference_stdout,omitempty"`
	Output        nativeSimulationOutput `json:"output"`
}

func runNativeSimulation(runner *state_dry_runner.DryRunner, block *vm.Block, name string, transaction vm.Transaction) nativeSimulationCase {
	suppliedNonce := new(big.Int).Set(transaction.Nonce)
	result := runner.Apply(block, &transaction)
	logs := make([]nativeSimulationLog, len(result.Logs))
	for index, log := range result.Logs {
		topics := make([]string, len(log.Topics))
		for topicIndex, topic := range log.Topics {
			topics[topicIndex] = hex.EncodeToString(topic[:])
		}
		logs[index] = nativeSimulationLog{
			Address: hex.EncodeToString(log.Address[:]),
			Topics:  topics,
			Data:    hex.EncodeToString(log.Data),
		}
	}
	return nativeSimulationCase{
		Name: name, To: hex.EncodeToString(transaction.To[:]), SuppliedNonce: suppliedNonce.String(),
		GasPrice: transaction.GasPrice.String(), Gas: transaction.Gas, Value: transaction.Value.String(),
		Input: hex.EncodeToString(transaction.Input),
		Output: nativeSimulationOutput{
			EffectiveNonce: transaction.Nonce.String(), GasUsed: result.GasUsed,
			ConsensusError: string(result.ConsensusErr), ExecutionError: string(result.ExecutionErr),
			Return: hex.EncodeToString(result.CodeRetval), Logs: logs,
		},
	}
}

func runNativeSimulationCaptured(runner *state_dry_runner.DryRunner, block *vm.Block, name string, transaction vm.Transaction) nativeSimulationCase {
	read, write, err := os.Pipe()
	if err != nil {
		panic(err)
	}
	stdout := os.Stdout
	os.Stdout = write
	result := runNativeSimulation(runner, block, name, transaction)
	os.Stdout = stdout
	if err := write.Close(); err != nil {
		panic(err)
	}
	printed := new(bytes.Buffer)
	if _, err := printed.ReadFrom(read); err != nil {
		panic(err)
	}
	if err := read.Close(); err != nil {
		panic(err)
	}
	result.ReferenceLog = printed.String()
	return result
}

type nativeSimulationSeedRow struct {
	Column byte   `json:"column"`
	Key    string `json:"key"`
	Value  string `json:"value"`
}

func nativeSimulationSeedRows(database *nativeSimulationDB) []nativeSimulationSeedRow {
	database.rows.mu.Lock()
	defer database.rows.mu.Unlock()
	var result []nativeSimulationSeedRow
	for column, rows := range database.rows.physical {
		for key, value := range rows {
			result = append(result, nativeSimulationSeedRow{
				Column: byte(column), Key: hex.EncodeToString([]byte(key)), Value: hex.EncodeToString(value),
			})
		}
	}
	sort.Slice(result, func(i, j int) bool {
		if result[i].Column != result[j].Column {
			return result[i].Column < result[j].Column
		}
		return result[i].Key < result[j].Key
	})
	return result
}

func nativeSimulationSnapshot(seed nativeSimulationSeed, api *dpos.API) map[string]any {
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	current := api.NewReader(nativeSimulationPeriod, storageFactory)
	delayed := api.NewDelayedReader(nativeSimulationPeriod, storageFactory)
	reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(nativeSimulationPeriod)}
	accounts := make([]map[string]any, 0, 3)
	for _, address := range []common.Address{nativeSimulationSender, seed.wrapper, *dpos.ContractAddress()} {
		row := map[string]any{"address": hex.EncodeToString(address[:]), "exists": false}
		reader.GetRawAccount(&address, func(encoded []byte) {
			account := state_db.DecodeAccountFromTrie(encoded)
			row["exists"] = true
			row["encoded"] = hex.EncodeToString(encoded)
			row["nonce"] = account.Nonce.String()
			row["balance"] = account.Balance.String()
			if account.CodeHash != nil {
				row["code_hash"] = hex.EncodeToString(account.CodeHash[:])
				row["code"] = hex.EncodeToString(reader.GetCode(account.CodeHash))
			}
		})
		accounts = append(accounts, row)
	}
	return map[string]any{
		"period":    uint64(seed.database.descriptor.BlockNum),
		"root":      hex.EncodeToString(seed.database.descriptor.StateRoot[:]),
		"seed_rows": nativeSimulationSeedRows(seed.database), "accounts": accounts,
		"current": map[string]any{
			"eligible":                 current.IsEligible(&nativeSimulationValidator),
			"validator_eligible_votes": current.GetEligibleVoteCount(&nativeSimulationValidator),
			"total_eligible_votes":     current.TotalEligibleVoteCount(),
		},
		"delayed": map[string]any{
			"effective_period":         0,
			"eligible":                 delayed.IsEligible(&nativeSimulationValidator),
			"validator_eligible_votes": delayed.GetEligibleVoteCount(&nativeSimulationValidator),
			"total_eligible_votes":     delayed.TotalEligibleVoteCount(),
		},
	}
}

func nativeSimulationABIInput(signature string, address common.Address) []byte {
	input := append(nativeSimulationSelector(signature), make([]byte, 12)...)
	return append(input, address[:]...)
}

func nativeSimulationHighAddressInput(signature string, address common.Address) []byte {
	input := append(nativeSimulationSelector(signature), bytes.Repeat([]byte{0xff}, 12)...)
	return append(input, address[:]...)
}

func main() {
	seed := seedNativeSimulation()
	api := new(dpos.API).Init(seed.config)
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(
		seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, storageFactory, &seed.config,
	)
	block := &vm.Block{
		Number:    nativeSimulationPeriod,
		BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1_700_000_001, Difficulty: new(big.Int)},
	}
	before := nativeSimulationSnapshot(seed, api)
	wideSupplied := new(big.Int).Lsh(big.NewInt(1), 512)
	ordinary := vm.Transaction{
		From: nativeSimulationSender, To: &seed.wrapper, Nonce: wideSupplied,
		GasPrice: big.NewInt(2), Gas: nativeSimulationGas, Value: big.NewInt(25),
	}
	first := runNativeSimulation(runner, block, "delegate_then_current_and_delayed_queries", ordinary)
	second := runNativeSimulation(runner, block, "repeat_delegate_then_queries", ordinary)
	dposAddress := *dpos.ContractAddress()
	malformed := runNativeSimulationCaptured(runner, block, "malformed_get_total_delegation", vm.Transaction{
		From: nativeSimulationSender, To: &dposAddress, Nonce: big.NewInt(0), GasPrice: big.NewInt(1),
		Gas: 100_000, Value: new(big.Int), Input: nativeSimulationSelector("getTotalDelegation(address)"),
	})
	missing := runNativeSimulation(runner, block, "missing_get_validator", vm.Transaction{
		From: nativeSimulationSender, To: &dposAddress, Nonce: big.NewInt(7), GasPrice: big.NewInt(1),
		Gas: 100_000, Value: new(big.Int), Input: nativeSimulationABIInput("getValidator(address)", nativeSimulationMissing),
	})
	shortTwoWord := runNativeSimulationCaptured(runner, block, "short_get_delegations", vm.Transaction{
		From: nativeSimulationSender, To: &dposAddress, Nonce: big.NewInt(8), GasPrice: big.NewInt(1),
		Gas: 100_000, Value: new(big.Int), Input: nativeSimulationABIInput("getDelegations(address,uint32)", seed.wrapper),
	})
	highBits := runNativeSimulation(runner, block, "high_bits_get_total_delegation", vm.Transaction{
		From: nativeSimulationSender, To: &dposAddress, Nonce: big.NewInt(9), GasPrice: big.NewInt(1),
		Gas: 100_000, Value: new(big.Int), Input: nativeSimulationHighAddressInput("getTotalDelegation(address)", seed.wrapper),
	})
	after := nativeSimulationSnapshot(seed, api)
	result := map[string]any{
		"schema": 1,
		"configuration": map[string]any{
			"chain_id": 666, "period": uint64(nativeSimulationPeriod), "delegation_delay": 1,
			"eligibility_threshold": "100", "vote_step": "10", "committed_validator_stake": "110",
			"simulation_delegate": "25", "fix_redelegate_block": 0,
		},
		"wrapper": map[string]any{
			"address": hex.EncodeToString(seed.wrapper[:]), "runtime": hex.EncodeToString(nativeSimulationWrapperRuntime()),
			"sequence": []string{"delegate(address) value=25", "getTotalDelegation(address(this))", "getValidatorEligibleVotesCount(validator)"},
		},
		"state_before":              before,
		"cases":                     []nativeSimulationCase{first, second, malformed, missing, shortTwoWord, highBits},
		"repeat_identical":          reflect.DeepEqual(first.Output, second.Output),
		"state_after":               after,
		"committed_state_unchanged": reflect.DeepEqual(before, after),
		"semantics": map[string]any{
			"entrypoint": "state_dry_runner.DryRunner.Apply",
			"nonce":      "each supplied nonce is replaced with committed sender nonce + 1",
			"state":      "each Apply creates a disposable BlockState over historical H; native mutations are visible to later native queries in that call and are discarded after Apply",
		},
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
