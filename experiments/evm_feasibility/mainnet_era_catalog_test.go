package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/holiman/uint256"
	"math/big"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

type eraCatalogTestInput struct{}

func (eraCatalogTestInput) GetCode(*common.Hash) []byte { return nil }
func (eraCatalogTestInput) GetAccount(_ *common.Address, cb func(state_db.Account)) {
	cb(state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(1)})
}
func (eraCatalogTestInput) GetAccountStorage(_ *common.Address, _ *common.Hash, _ func([]byte)) {}

type eraCatalogTestOutput struct{}
type eraCatalogTestMutation struct{}

func (eraCatalogTestOutput) StartMutation(*common.Address) state_evm.AccountMutation {
	return eraCatalogTestMutation{}
}
func (eraCatalogTestOutput) Delete(*common.Address)           {}
func (eraCatalogTestMutation) Update(state_evm.AccountChange) {}
func (eraCatalogTestMutation) Commit()                        {}

func eraCatalogTestState() *state_evm.TransitionState {
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{NumTransactionsToBuffer: 1})
	state.SetInput(eraCatalogTestInput{})
	return state
}
func eraCatalogTestFlush(address common.Address, key int64) {
	state := eraCatalogTestState()
	state.GetAccountConcrete(&address).SetState(big.NewInt(key), big.NewInt(1))
	state.CommitTransaction(eraCatalogTestOutput{})
	state.Commit()
}

func TestEraCatalogActualFlushAndRevertedStorage(t *testing.T) {
	ids := catalogIdentities{}
	restore := eraCatalogScope(&ids)
	defer restore()
	address := common.HexToAddress("1234")
	state := eraCatalogTestState()
	account := state.GetAccountConcrete(&address)
	account.SetState(big.NewInt(0), big.NewInt(1))
	account.SetState(big.NewInt(0), big.NewInt(2))
	if len(ids.Slots) != 0 {
		t.Fatal("an unflushed setter polluted committed identities")
	}
	state.CommitTransaction(eraCatalogTestOutput{})
	state.Commit()
	want := catalogSlot{Address: address, Key: common.BytesToHash(big.NewInt(0).Bytes())}
	if !reflect.DeepEqual(ids.Slots, []catalogSlot{want}) || !reflect.DeepEqual(ids.Accounts, []common.Address{address}) {
		t.Fatal("actual committed key identity missing or duplicated", ids)
	}
	state = eraCatalogTestState()
	snapshot := state.Snapshot()
	state.GetAccountConcrete(&address).SetState(big.NewInt(7), big.NewInt(1))
	state.RevertToSnapshot(snapshot)
	state.CommitTransaction(eraCatalogTestOutput{})
	state.Commit()
	if !reflect.DeepEqual(ids.Slots, []catalogSlot{want}) {
		t.Fatal("reverted SSTORE became committed catalog identity", ids)
	}
}

func TestEraCatalogNestedDonorQueryAndPanicRestoreHandlers(t *testing.T) {
	outerCalls := 0
	previous := state_evm.SwapEraCommittedStorageKeyObserver(func(common.Address, common.Hash) { outerCalls++ })
	defer state_evm.SwapEraCommittedStorageKeyObserver(previous)
	parent, child := catalogIdentities{}, catalogIdentities{}
	parentAddress, childAddress, queryAddress := common.HexToAddress("1111"), common.HexToAddress("2222"), common.HexToAddress("3333")
	func() {
		restoreParent := eraCatalogScope(&parent)
		defer restoreParent()
		eraCatalogTestFlush(parentAddress, 1)
		func() {
			restoreCaller := eraCatalogScope(nil)
			defer restoreCaller()
			restoreChild := eraCatalogScope(&child)
			defer restoreChild()
			eraCatalogTestFlush(childAddress, 2)
			eraQueriesWithoutCatalog(func() []map[string]any { eraCatalogTestFlush(queryAddress, 3); return nil })
			eraCatalogTestFlush(childAddress, 4)
		}()
		func() {
			defer func() {
				if recover() == nil {
					t.Error("expected isolated query panic")
				}
			}()
			eraQueriesWithoutCatalog(func() []map[string]any { eraCatalogTestFlush(queryAddress, 5); panic("query failure") })
		}()
		// A catalog replacement must not leave the callback targeting a stale copy.
		parent = eraWritten(parent, nil)
		eraCatalogTestFlush(parentAddress, 6)
	}()
	eraCatalogTestFlush(queryAddress, 7)
	if outerCalls != 1 || len(parent.Slots) != 2 || len(child.Slots) != 2 {
		t.Fatal("scope restoration or run isolation failed", outerCalls, parent, child)
	}
	for _, slot := range parent.Slots {
		if slot.Address != parentAddress {
			t.Fatal("child/query polluted parent")
		}
	}
	for _, slot := range child.Slots {
		if slot.Address != childAddress {
			t.Fatal("query polluted child")
		}
	}
}

func TestEraCatalogSuccessfulCreateMembershipKeepsEventDuplicates(t *testing.T) {
	initial, created := common.HexToAddress("aaaa"), common.HexToAddress("bbbb")
	slot := catalogSlot{Address: initial, Key: common.HexToHash("01")}
	ids := catalogIdentities{Accounts: []common.Address{initial}, Slots: []catalogSlot{slot}, Invocations: "unchanged"}
	addresses := []common.Address{initial, initial}
	success := vm.ExecutionResult{NewContractAddr: created}
	eraRecordCreated(&ids, &addresses, vm.Transaction{}, success)
	eraRecordCreated(&ids, &addresses, vm.Transaction{}, success)
	if !reflect.DeepEqual(ids.Accounts, []common.Address{initial, created}) || !reflect.DeepEqual(addresses, []common.Address{initial, initial, created, created}) {
		t.Fatal("CREATE identity/event order or dedup mismatch")
	}
	eraRecordCreated(&ids, &addresses, vm.Transaction{}, vm.ExecutionResult{NewContractAddr: common.HexToAddress("cccc"), ExecutionErr: "execution reverted"})
	eraRecordCreated(&ids, &addresses, vm.Transaction{}, vm.ExecutionResult{NewContractAddr: common.HexToAddress("dddd"), ConsensusErr: "rejected"})
	receiver := initial
	eraRecordCreated(&ids, &addresses, vm.Transaction{To: &receiver}, success)
	if len(ids.Accounts) != 2 || len(addresses) != 4 || !reflect.DeepEqual(ids.Slots, []catalogSlot{slot}) || ids.Invocations != "unchanged" {
		t.Fatal("non-CREATE event or identity-only collector changed unrelated catalog")
	}
}

// Uses the actual C-composition helper and real Go ordinary account semantics;
// the fake boundary records calls, not expected reward or account bytes.
type eraFeeTestState struct {
	state  *state_evm.TransitionState
	cfg    chain_config.ChainConfig
	period types.BlockNum
	calls  int
}

func (s *eraFeeTestState) BlockNumber() types.BlockNum               { return s.period }
func (s *eraFeeTestState) GetChainConfig() *chain_config.ChainConfig { return &s.cfg }
func (s *eraFeeTestState) AddTxFeeToBalance(author *common.Address, fee *uint256.Int) {
	s.calls++
	s.state.GetAccount(author).AddBalance(fee.ToBig())
}

type eraFeeAbsentInput struct{}

func (eraFeeAbsentInput) GetCode(*common.Hash) []byte                                   { return nil }
func (eraFeeAbsentInput) GetAccount(*common.Address, func(state_db.Account))            {}
func (eraFeeAbsentInput) GetAccountStorage(*common.Address, *common.Hash, func([]byte)) {}
func TestEraCComposedFeeReturnedFailuresAndBoundary(t *testing.T) {
	author := common.HexToAddress("aabb")
	for _, failure := range []vm.ExecutionResult{{GasUsed: 7}, {GasUsed: 7, ExecutionErr: "execution reverted"}, {GasUsed: 7, ConsensusErr: "gas charged failure"}} {
		for _, period := range []types.BlockNum{9, 10, 11} {
			s := &eraFeeTestState{state: eraCatalogTestState(), period: period}
			s.cfg.Hardforks.MagnoliaHf.BlockNum = 10
			before := new(big.Int).Set(s.state.GetAccount(&author).GetBalance())
			eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: big.NewInt(3)}, failure)
			want := new(big.Int).Set(before)
			calls := 0
			if period < 10 {
				want.Add(want, big.NewInt(21))
				calls = 1
			}
			if s.calls != calls || s.state.GetAccount(&author).GetBalance().Cmp(want) != 0 {
				t.Fatal("boundary/status changed C fee composition", period, failure, s.calls, want)
			}
		}
	}
}
func TestEraCComposedFeeUint256ProductAndZeroAccountLifecycle(t *testing.T) {
	author := common.HexToAddress("aabb")
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{NumTransactionsToBuffer: 1})
	state.SetInput(eraFeeAbsentInput{})
	s := &eraFeeTestState{state: state, period: 1}
	s.cfg.Hardforks.MagnoliaHf.BlockNum = 2
	if !state.GetAccountConcrete(&author).IsNIL() {
		t.Fatal("fixture author must start absent")
	}
	eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: big.NewInt(0)}, vm.ExecutionResult{GasUsed: 7})
	if s.calls != 1 || state.GetAccountConcrete(&author).IsNIL() || state.GetAccount(&author).GetBalance().Sign() != 0 || state.GetAccount(&author).GetNonce().Sign() != 0 {
		t.Fatal("zero fee touch/create was omitted")
	}
	price := new(big.Int).Lsh(big.NewInt(1), 255)
	eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: price}, vm.ExecutionResult{GasUsed: 3})
	if state.GetAccount(&author).GetBalance().Cmp(price) != 0 {
		t.Fatal("C uint256 multiplication must wrap, not overflow or use unbounded product")
	}
	// Ordinary account addition is big.Int, unlike the fee product.
	eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: price}, vm.ExecutionResult{GasUsed: 1})
	if state.GetAccount(&author).GetBalance().Cmp(new(big.Int).Lsh(big.NewInt(1), 256)) != 0 {
		t.Fatal("ordinary fee balance addition was narrowed")
	}
}

func TestEraCComposedFeeCatalogTracksActualCreditsIncludingZeroTouch(t *testing.T) {
	author := common.HexToAddress("aabb")
	other := common.HexToAddress("bbcc")
	ids := catalogIdentities{}
	addresses := []common.Address{}
	s := &eraFeeTestState{state: eraCatalogTestState(), period: 1}
	s.cfg.Hardforks.MagnoliaHf.BlockNum = 2
	if eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: big.NewInt(0)}, vm.ExecutionResult{GasUsed: 7}) {
		eraRecordFeeCredit(&ids, &addresses, author)
	}
	if eraCreditCComposedTransactionFee(s, author, vm.Transaction{GasPrice: big.NewInt(3)}, vm.ExecutionResult{GasUsed: 7, ExecutionErr: "execution reverted"}) {
		eraRecordFeeCredit(&ids, &addresses, author)
	}
	s.period = 2
	if eraCreditCComposedTransactionFee(s, other, vm.Transaction{GasPrice: big.NewInt(3)}, vm.ExecutionResult{GasUsed: 7}) {
		eraRecordFeeCredit(&ids, &addresses, other)
	}
	if !reflect.DeepEqual(ids.Accounts, []common.Address{author}) || !reflect.DeepEqual(addresses, []common.Address{author}) || len(ids.Slots) != 0 {
		t.Fatal("credit event catalog membership/order or native raw separation changed", ids, addresses)
	}
}

// Component only: zero-price calls are not mainnet admission evidence. This
// intentionally changes design01's registered-validator fixture to no validators,
// while retaining the real nonnil DPoS object and its EndBlock checkpoint.
func eraZeroFeeRealTransitionCase(t *testing.T, name string, credit, observer bool) map[string]any {
	sender := testSender()
	author := common.HexToAddress("fa1c")
	recipient := common.HexToAddress("0777")
	cfg := witnessConfig(sender)
	cfg.GenesisBalances = core.BalanceMap{sender: big.NewInt(1000000)}
	cfg.DPOS.InitialValidators = nil
	cfg.DPOS.YieldPercentage = 0
	cfg.Hardforks.MagnoliaHf.BlockNum = 10
	cfg.Hardforks.CornusHf.BlockNum = maxPeriod
	cfg.Hardforks.AspenHf.BlockNumPartOne = maxPeriod
	cfg.Hardforks.AspenHf.BlockNumPartTwo = maxPeriod
	cfg.Hardforks.FicusHf.BlockNum = maxPeriod
	cfg.Hardforks.CactiHf.BlockNum = maxPeriod
	latest := newMemoryLatest()
	st := newStateTransition(latest, &cfg)
	defer st.Close()
	addresses := []common.Address{sender, author, recipient, *dpos.ContractAddress()}
	physical := func(period uint64) []map[string]any {
		return observeAddressSet(state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(period))}, addresses)
	}
	current := func() map[string]any {
		a := st.GetEvmState().GetAccountConcrete(&author)
		return map[string]any{"present": !a.IsNIL(), "nonce": a.GetNonce().String(), "balance": a.GetBalance().String()}
	}
	genesis := latest.GetCommittedDescriptor()
	genesisAccounts := physical(0)
	for _, row := range genesisAccounts {
		if row["address"] == hex.EncodeToString(author[:]) && row["present"] != false {
			t.Fatal("declared author is not physically absent at genesis")
		}
	}
	block := vm.BlockInfo{Author: author, GasLimit: blockGas, Difficulty: new(big.Int)}
	st.BeginBlock(&block)
	events := catalogIdentities{}
	observedAddresses := []common.Address{}
	transactions := []map[string]any{}
	count := 1
	if name == "second_value_to_author" {
		count = 2
	}
	for i := 0; i < count; i++ {
		target := recipient
		if i == 1 {
			target = author
		}
		gas := uint64(21000)
		price := big.NewInt(0)
		value := big.NewInt(1)
		if name == "zero_gas_intrinsic_failure" {
			gas = 0
			price = big.NewInt(1)
			value = new(big.Int)
		}
		tx := vm.Transaction{From: sender, To: &target, Nonce: big.NewInt(int64(i)), GasPrice: price, Gas: gas, Value: value}
		result := st.ExecuteTransaction(&tx)
		if name == "zero_gas_intrinsic_failure" {
			if result.GasUsed != 0 || result.ConsensusErr == "" || result.ExecutionErr != "" {
				t.Fatal("actual intrinsic-failure result changed", result)
			}
		} else if result.GasUsed != 21000 || result.ConsensusErr != "" || result.ExecutionErr != "" {
			t.Fatal("actual ordinary zero-price call failed", result)
		}
		afterExecute := current()
		applied := false
		if credit {
			applied = eraCreditCComposedTransactionFee(st, author, tx, result)
			if applied {
				eraRecordFeeCredit(&events, &observedAddresses, author)
			}
		}
		afterCredit := current()
		if applied && result.GasUsed*price.Uint64() != 0 {
			t.Fatal("zero-fee fixture accidentally acquired positive fee")
		}
		row := map[string]any{"index": i, "gas_used": result.GasUsed, "execution_error": string(result.ExecutionErr), "consensus_error": string(result.ConsensusErr), "after_execute_checkpoint": afterExecute, "after_c_credit": afterCredit, "c_credit_called": applied}
		if observer {
			root, ids, reader := observerFinalizeTransaction(st, uint64(i))
			row["observer_root"] = hex.EncodeToString(root[:])
			row["observer_catalog"] = jsonValue(ids)
			row["observer_accounts"] = observeAddressSet(reader, addresses)
			afterObserver := current()
			row["after_observer_checkpoint"] = afterObserver
			if i == 0 && afterObserver["present"] != false {
				t.Fatal("actual observer checkpoint did not prune zero author", afterObserver)
			}
		}
		transactions = append(transactions, row)
	}
	beginRawWriteTrace()
	st.EndBlock()
	end := finishRawWriteTrace()
	prepared := st.PrepareCommit()
	committed := st.Commit()
	if prepared != committed {
		t.Fatal("prepared/committed root mismatch")
	}
	final := physical(1)
	for _, row := range final {
		if row["address"] == hex.EncodeToString(author[:]) {
			if name == "second_value_to_author" {
				if row["present"] != true || row["balance"] != "1" || row["nonce"] != "0" {
					t.Fatal("real second transfer to author mismatch", row)
				}
			}
			if name != "second_value_to_author" && row["present"] != false {
				t.Fatal("zero touch survived actual EndBlock checkpoint", row)
			}
		}
	}
	// Reopen an independently allocated physical memory reader after commit; this
	// is not a filesystem DB reopen, which remains the Rust typed-writer gate.
	reopened := state_db.ExtendedReader{Reader: latest.readerAt(1)}
	return map[string]any{"component": "zero-fee-checkpoint-v1", "case": name, "credit": credit, "observer": observer,
		"fixture_override": "no validators, nonnil DPoS, yield0; ordinary absent author; no mainnet admission/liveness claim",
		"declared":         map[string]any{"sender": hex.EncodeToString(sender[:]), "author": hex.EncodeToString(author[:]), "recipient": hex.EncodeToString(recipient[:]), "sender_balance": "1000000", "magnolia": 10, "dpos_nonce": "1", "validators": 0, "yield": 0},
		"genesis_root":     hex.EncodeToString(genesis.StateRoot[:]), "genesis_accounts": genesisAccounts, "transactions": transactions,
		"end_block_raw_writes": end, "final_root": hex.EncodeToString(committed[:]), "final_accounts": final, "reopened_accounts": observeAddressSet(reopened, addresses), "credit_event_catalog": jsonValue(events)}
}

func TestEraCComposedFeeRealTransitionZeroPruning(t *testing.T) {
	for _, name := range []string{"zero_price_call", "second_value_to_author", "zero_gas_intrinsic_failure"} {
		var normalRoot any
		for _, observer := range []bool{false, true} {
			if observer && !observerAvailable() {
				continue
			}
			credited := eraZeroFeeRealTransitionCase(t, name, true, observer)
			control := eraZeroFeeRealTransitionCase(t, name, false, observer)
			if credited["genesis_root"] != control["genesis_root"] || credited["final_root"] != control["final_root"] {
				t.Fatal("zero-credit control root invariant failed", name, observer, credited, control)
			}
			if !observer {
				normalRoot = credited["final_root"]
			} else if credited["final_root"] != normalRoot {
				t.Fatal("observer changed zero-fee component final root", name, credited, normalRoot)
			}
			raw, err := json.Marshal(map[string]any{"credited": credited, "omitted_control": control})
			if err != nil {
				t.Fatal(err)
			}
			t.Log("ZERO_FEE_GO_EVIDENCE " + string(raw))
		}
	}
}

// Unsigned transition component only: these exact configured addresses do not
// have signing keys in this harness. No signed/mainnet/N4 provenance is claimed.
func eraCorrectionSeedConfig(t *testing.T, missingDelegation bool) (chain_config.ChainConfig, common.Address, common.Address, *big.Int) {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("cmd", "mainnet_era", "input.json"))
	if err != nil {
		t.Fatal(err)
	}
	var input eraInput
	if err = json.Unmarshal(raw, &input); err != nil {
		t.Fatal(err)
	}
	cfg := input.Configuration
	if cfg.Hardforks.FixRedelegateBlockNum != 3091000 || len(cfg.Hardforks.Redelegations) != 1 {
		t.Fatal("original correction configuration required")
	}
	correction := cfg.Hardforks.Redelegations[0]
	validator := common.HexToAddress("6671df8b597c0a14a9361b4b98f97af1360e4352")
	delegator := common.HexToAddress("af5a5cd6da86838efac0ea91a712857d6aca53ac")
	if correction.Validator != validator || correction.Delegator != delegator || correction.Amount.Cmp(big.NewInt(1000)) != 0 {
		t.Fatal("correction identity/amount changed")
	}
	// Declared stake meets the real profile's minimum remainder and maximum.
	stake := new(big.Int).Add(new(big.Int).Mul(cfg.DPOS.MinimumDeposit, big.NewInt(2)), big.NewInt(2000))
	if stake.Cmp(big.NewInt(1000)) < 0 || new(big.Int).Add(stake, big.NewInt(1000)).Cmp(cfg.DPOS.ValidatorMaximumStake) > 0 {
		t.Fatal("profile cannot admit bounded seed stake")
	}
	owner := testSender()
	initialDelegator := delegator
	if missingDelegation {
		initialDelegator = owner
	}
	allocation := new(big.Int).Mul(stake, big.NewInt(4))
	cfg.GenesisBalances = core.BalanceMap{delegator: new(big.Int).Set(allocation), owner: new(big.Int).Set(allocation)}
	cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{{Address: validator, Owner: owner, VrfKey: bytes.Repeat([]byte{0x44}, 32), Commission: 100, Description: "unsigned correction component", Delegations: core.BalanceMap{initialDelegator: new(big.Int).Set(stake)}}}
	return cfg, validator, delegator, stake
}

// Same bounded projection/copy mechanism as eraSeed, with a complete actual
// genesis event catalog (including the newly declared ordinary addresses).
// Only actual Go genesis rows are copied; no post-transaction row is edited.
func eraCorrectionSeedGenesis(t *testing.T, cfg chain_config.ChainConfig, at uint64) (*memoryLatest, catalogIdentities, map[string]any) {
	t.Helper()
	source := newMemoryLatest()
	ids := catalogIdentities{}
	restore := eraCatalogScope(&ids)
	beginRawWriteTrace()
	genesis := newStateTransition(source, &cfg)
	writes := finishRawWriteTrace()
	genesis.Close()
	restore()
	ids = eraWritten(ids, writes)
	for address := range cfg.GenesisBalances {
		eraAddAccountIdentity(&ids, address)
	}
	eraAddAccountIdentity(&ids, *dpos.ContractAddress())
	ids = catalogIdentityClosure(mergeCatalogs(catalogIdentities{}, ids))
	reader := state_db.ExtendedReader{Reader: source.readerAt(0)}
	declared := captureNativeCatalog(reader, ids)
	latest := newMemoryLatest()
	// Retain every delayed reader needed by B-1/B/B+1. Graph dates stay0.
	first := at - uint64(cfg.DPOS.DelegationDelay) - 1
	latest.descriptor.BlockNum = types.BlockNum(first - 1)
	pending := latest.BeginPendingBlock()
	sink := new(state_transition.TrieSink).Init(nil, state_transition.TrieSinkOpts{})
	sink.SetIO(pending)
	for _, entry := range ids.Accounts {
		address := entry
		reader.GetAccount(&address, func(account state_db.Account) {
			change := state_evm.AccountChange{Account: account, RawStorageDirty: state_evm.RawStorage{}}
			change.StorageRootHash = nil
			if account.CodeHash != nil {
				change.Code = reader.GetCode(account.CodeHash)
				change.CodeDirty = true
			}
			for _, slot := range ids.Slots {
				if slot.Address == address {
					key := slot.Key
					reader.GetAccountStorage(&address, &key, func(value []byte) { change.RawStorageDirty[key] = common.CopyBytes(value) })
				}
			}
			mutation := sink.StartMutation(&address)
			mutation.Update(change)
			mutation.Commit()
		})
	}
	root := sink.Commit()
	sink.Close()
	must(latest.Commit(root))
	if root != source.descriptor.StateRoot {
		t.Fatal("complete declared genesis catalog failed exact root reconstruction")
	}
	history := []map[string]any{{"period": first, "root": hex.EncodeToString(root[:])}}
	for period := first + 1; period <= at; period++ {
		pending := latest.BeginPendingBlock()
		writer := new(state_transition.TrieSink).Init(&root, state_transition.TrieSinkOpts{})
		writer.SetIO(pending)
		root = writer.Commit()
		writer.Close()
		must(latest.Commit(root))
		if root != source.descriptor.StateRoot {
			t.Fatal("unchanged genesis copy changed root")
		}
		history = append(history, map[string]any{"period": period, "root": hex.EncodeToString(root[:])})
	}
	return latest, ids, map[string]any{"actual_genesis_root": hex.EncodeToString(source.descriptor.StateRoot[:]), "actual_genesis_catalog": declared, "genesis_raw_writes": writes, "synthetic_unchanged_history": history, "actual_genesis_configuration": configurationJSON(cfg, 1000)}
}

func eraCorrectionRaw(st *state_transition.StateTransition, key common.Hash) []byte {
	var result []byte
	st.GetEvmState().GetAccountConcrete(dpos.ContractAddress()).GetRawState(&key, func(value []byte) { result = common.CopyBytes(value) })
	return result
}
func eraCorrectionFacts(t *testing.T, st *state_transition.StateTransition, validator, delegator common.Address) map[string]any {
	t.Helper()
	validatorKey := *contract_storage.Stor_k_1([]byte{0, 0}, validator[:])
	delegationKey := contract_storage.Stor_k_2([]byte{2, 0}, validator[:], delegator[:])
	validatorRaw := eraCorrectionRaw(st, validatorKey)
	delegationRaw := eraCorrectionRaw(st, delegationKey)
	var value dpos.ValidatorV1
	var delegation dpos.Delegation
	if err := rlp.DecodeBytes(validatorRaw, &value); err != nil {
		t.Fatal("actual Legacy validator decode", err)
	}
	if err := rlp.DecodeBytes(delegationRaw, &delegation); err != nil {
		t.Fatal("actual delegation decode", err)
	}
	headKey := contract_storage.Stor_k_2([]byte{1}, validator[:], dpos.BlockToBytes(value.LastUpdated))
	cursorKey := contract_storage.Stor_k_2([]byte{1}, validator[:], dpos.BlockToBytes(delegation.LastUpdated))
	rewardsKey := *contract_storage.Stor_k_1([]byte{0, 2}, validator[:])
	cursorRaw := eraCorrectionRaw(st, cursorKey)
	var cursorState dpos.State
	if len(cursorRaw) > 0 {
		if err := rlp.DecodeBytes(cursorRaw, &cursorState); err != nil {
			t.Fatal("actual cursor node decode", err)
		}
	}
	cursorRewardIndex := "absent"
	if cursorState.RewardsPer1Stake != nil {
		cursorRewardIndex = cursorState.RewardsPer1Stake.String()
	}

	return map[string]any{"validator_key": hex.EncodeToString(validatorKey[:]), "validator_raw": hex.EncodeToString(validatorRaw), "stake": value.TotalStake.String(), "head": uint64(value.LastUpdated), "commission": value.Commission, "last_commission_change": uint64(value.LastCommissionChange), "delegation_key": hex.EncodeToString(delegationKey[:]), "delegation_raw": hex.EncodeToString(delegationRaw), "principal": delegation.Stake.String(), "cursor": uint64(delegation.LastUpdated), "head_node_key": hex.EncodeToString(headKey[:]), "head_node_raw": hex.EncodeToString(eraCorrectionRaw(st, headKey)), "cursor_node_key": hex.EncodeToString(cursorKey[:]), "cursor_node_raw": hex.EncodeToString(cursorRaw), "cursor_node_count": cursorState.Count, "cursor_reward_index": cursorRewardIndex, "rewards_raw": hex.EncodeToString(eraCorrectionRaw(st, rewardsKey)), "eligible_raw": hex.EncodeToString(eraCorrectionRaw(st, *contract_storage.Stor_k_1([]byte{4}))), "amount_delegated_raw": hex.EncodeToString(eraCorrectionRaw(st, *contract_storage.Stor_k_1([]byte{5}))), "validator_row_shape": "Legacy4", "undelegations_count": 0}
}
func eraCorrectionLog(t *testing.T, value any) {
	t.Helper()
	raw, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	t.Log("CORRECTION_SEED_GO_EVIDENCE " + string(raw))
}

func eraCorrectionPositive(t *testing.T, observer bool) common.Hash {
	t.Helper()
	cfg, validator, delegator, stake := eraCorrectionSeedConfig(t, false)
	b := cfg.Hardforks.FixRedelegateBlockNum
	latest, ids, seed := eraCorrectionSeedGenesis(t, cfg, b-2)
	st := newStateTransition(latest, &cfg)
	defer st.Close()
	restore := eraCatalogScope(&ids)
	defer restore()
	author := testSender()
	addresses := []common.Address{author, delegator, validator, *dpos.ContractAddress()}
	periods := []map[string]any{}
	for period := b - 1; period <= b+1; period++ {
		prior := latest.GetCommittedDescriptor()
		st.BeginBlock(&vm.BlockInfo{Author: author, GasLimit: blockGas, Difficulty: new(big.Int)})
		var txEvidence any
		if period == b-1 {
			tx := vm.Transaction{From: delegator, To: dpos.ContractAddress(), Nonce: big.NewInt(0), GasPrice: big.NewInt(1000000000), Gas: 500000, Value: new(big.Int), Input: integrationPack(false, "reDelegate", validator, validator, big.NewInt(1000))}
			declaredInput := common.CopyBytes(tx.Input)
			beginRawWriteTrace()
			beginGasRefundTrace()
			result := st.ExecuteTransaction(&tx)
			refund := finishGasRefundTrace()
			writes := finishRawWriteTrace()
			if result.ConsensusErr != "" || result.ExecutionErr != "" || result.GasUsed == 0 {
				t.Fatal("actual unsigned precursor did not succeed", result)
			}
			if !eraCreditCComposedTransactionFee(st, author, tx, result) {
				t.Fatal("preMagnolia C fee composition not applied")
			}
			eraRecordFeeCredit(&ids, &addresses, author)
			ids = eraWritten(ids, writes)
			txEvidence = map[string]any{"unsigned_component": true, "from": hex.EncodeToString(delegator[:]), "to": hex.EncodeToString((*tx.To)[:]), "nonce": "0", "value": "0", "declared_input": hex.EncodeToString(declaredInput), "executed_input": hex.EncodeToString(tx.Input), "gas_limit": tx.Gas, "gas_price": tx.GasPrice.String(), "result": jsonValue(result), "gas_refund": refund, "raw_writes": writes}
			if observer {
				root, catalog, reader := observerFinalizeTransaction(st, 0)
				ids = mergeCatalogs(ids, catalogIdentityClosure(catalog))
				txEvidence.(map[string]any)["observer_root"] = hex.EncodeToString(root[:])
				txEvidence.(map[string]any)["observer_catalog"] = captureCatalog(reader, catalog)
			}
		}
		before := eraCorrectionFacts(t, st, validator, delegator)
		if period == b-1 || period == b {
			if before["head_node_raw"] != "" || before["cursor_node_raw"] == "" || before["cursor"] != b-1 || before["head"] == before["cursor"] {
				t.Fatal("actual precursor failed required missing-head/live-cursor topology", before)
			}
			inflated := new(big.Int).Add(stake, big.NewInt(1000)).String()
			if before["stake"] != inflated || before["principal"] != stake.String() {
				t.Fatal("actual precursor did not produce exact configured correction amount", before)
			}
		}
		beginRawWriteTrace()
		st.EndBlock()
		end := finishRawWriteTrace()
		ids = eraWritten(ids, end)
		prepared := st.PrepareCommit()
		committed := st.Commit()
		if prepared != committed {
			t.Fatal("prepared/committed root mismatch")
		}
		// Read current committed rows through a fresh transition; do not replace
		// the running owner whose process-local cache is part of this component.
		readOwner := newStateTransition(latest, &cfg)
		readOwner.GetEvmState().SetInput(state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(period))})
		after := eraCorrectionFacts(t, readOwner, validator, delegator)
		readOwner.Close()
		if period == b-1 {
			if after["stake"] != before["stake"] || after["head"] != before["head"] {
				t.Fatal("correction ran before B")
			}
		} else {
			if after["stake"] != stake.String() || after["head"] != after["cursor"] || after["cursor"] != b-1 || after["head_node_raw"] == "" {
				t.Fatal("actual correction result mismatch", after)
			}
		}
		if after["cursor_node_raw"] != before["cursor_node_raw"] || after["delegation_raw"] != before["delegation_raw"] || after["rewards_raw"] != before["rewards_raw"] || after["eligible_raw"] != before["eligible_raw"] || after["amount_delegated_raw"] != before["amount_delegated_raw"] || after["commission"] != before["commission"] || after["last_commission_change"] != before["last_commission_change"] {
			t.Fatal("correction changed unrelated principal/rewards/scalar/metadata", before, after)
		}
		if period == b {
			if len(end) != 1 || end[0].Key != before["validator_key"] {
				t.Fatal("correction must persist exactly actual Legacy validator row", end)
			}
		}
		if period == b+1 && (len(end) != 0 || committed != prior.StateRoot) {
			t.Fatal("correction repeated after B", end)
		}
		physical := state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(period))}
		periods = append(periods, map[string]any{"number": period, "prior_root": hex.EncodeToString(prior.StateRoot[:]), "transaction": txEvidence, "before_end_block": before, "end_block_raw_writes": end, "after_end_block": after, "root": hex.EncodeToString(committed[:]), "accounts": observeAddressSet(physical, addresses), "catalog": captureNativeCatalog(physical, ids)})
	}
	eraCorrectionLog(t, map[string]any{"scope": "UNSIGNED_SYNTHETIC_COMPONENT_NOT_SIGNED_MAINNET_N4", "observer": observer, "seed": seed, "periods": periods, "final_physical_rows": latest.rows.exportPhysical()})
	return latest.descriptor.StateRoot
}
func TestEraCorrectionSeedActualUnsignedLifecycle(t *testing.T) {
	root := eraCorrectionPositive(t, false)
	if observerAvailable() && eraCorrectionPositive(t, true) != root {
		t.Fatal("local observer changed actual component final root")
	}
}
func TestEraCorrectionSeedRejectsUncorruptedAndMissingDelegation(t *testing.T) {
	for _, missing := range []bool{false, true} {
		t.Run(fmt.Sprint(missing), func(t *testing.T) {
			cfg, validator, delegator, _ := eraCorrectionSeedConfig(t, missing)
			b := cfg.Hardforks.FixRedelegateBlockNum
			latest, ids, seed := eraCorrectionSeedGenesis(t, cfg, b-1)
			st := newStateTransition(latest, &cfg)
			defer st.Close()
			prior := latest.GetCommittedDescriptor()
			priorRows := latest.rows.exportPhysical()
			st.BeginBlock(&vm.BlockInfo{Author: testSender(), GasLimit: blockGas, Difficulty: new(big.Int)})
			// A real recognized getter initializes the actual native contract without
			// fabricating/mutating the malformed correction preconditions.
			tx := vm.Transaction{From: testSender(), To: dpos.ContractAddress(), Nonce: big.NewInt(0), GasPrice: new(big.Int), Gas: 500000, Value: new(big.Int), Input: integrationPack(false, "getValidator", validator)}
			result := st.ExecuteTransaction(&tx)
			if result.ConsensusErr != "" || result.ExecutionErr != "" {
				t.Fatal("actual native initializer failed", result)
			}
			if !eraCreditCComposedTransactionFee(st, testSender(), tx, result) {
				t.Fatal("negative preMagnolia C fee composition not applied")
			}
			// Executing the getter changes its sender nonce only in the unpublished
			// transaction checkpoint; this is included in the prior physical snapshot.
			priorRows = latest.rows.exportPhysical()
			beginRawWriteTrace()
			var failure any
			func() { defer func() { failure = recover() }(); st.EndBlock() }()
			writes := finishRawWriteTrace()
			if failure == nil {
				t.Fatal("invalid correction preconditions were accepted")
			}
			if !missing && fmt.Sprint(failure) != "HF on wrong account" {
				t.Fatal("uncorrupted correction wrong failure", failure)
			}
			if len(writes) != 0 || latest.GetCommittedDescriptor() != prior || !reflect.DeepEqual(priorRows, latest.rows.exportPhysical()) {
				t.Fatal("failed correction published or changed physical rows", writes)
			}
			eraCorrectionLog(t, map[string]any{"scope": "UNSIGNED_SYNTHETIC_NEGATIVE_NO_COMMIT", "missing_delegation": missing, "validator": hex.EncodeToString(validator[:]), "delegator": hex.EncodeToString(delegator[:]), "seed": seed, "actual_panic": fmt.Sprint(failure), "raw_writes": writes, "committed_root": hex.EncodeToString(prior.StateRoot[:]), "catalog": jsonValue(ids)})
		})
	}
	// Fresh independent positive construction/commit proves a failed owner was
	// not reused or repaired. No expectation bytes are supplied to that run.
	eraCorrectionPositive(t, false)
}
