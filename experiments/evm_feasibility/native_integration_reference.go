// Current-profile signed native workload. Uses the existing mixed-period owners.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"math/big"
	"runtime/debug"
	"sort"
	"strings"

	"github.com/Taraxa-project/taraxa-evm/accounts/abi"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	dpos_sol "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/solidity"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
	slash_sol "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/solidity"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

var integrationKeys = [][]byte{bytes.Repeat([]byte{1}, 32), bytes.Repeat([]byte{2}, 32), bytes.Repeat([]byte{3}, 32), bytes.Repeat([]byte{4}, 32), bytes.Repeat([]byte{5}, 32)}
var integrationValidators = []common.Address{addressForPrivateKey(bytes.Repeat([]byte{9}, 32)), addressForPrivateKey(bytes.Repeat([]byte{10}, 32)), addressForPrivateKey(bytes.Repeat([]byte{11}, 32))}
var integrationValidatorKeys = [][]byte{bytes.Repeat([]byte{9}, 32), bytes.Repeat([]byte{10}, 32), bytes.Repeat([]byte{11}, 32)}
var integrationAPIMode bool

func integrationConfig(bracket bool) chain_config.ChainConfig {
	c := witnessConfig(addressForPrivateKey(integrationKeys[0]))
	c.GenesisBalances = core.BalanceMap{}
	for _, key := range integrationKeys {
		c.GenesisBalances[addressForPrivateKey(key)] = big.NewInt(1_000_000_000)
	}
	c.DPOS.InitialValidators = nil
	c.DPOS.BlocksPerYear = 100
	c.DPOS.YieldPercentage = 1
	c.DPOS.CommissionChangeDelta = 100
	c.DPOS.CommissionChangeFrequency = 2
	c.DPOS.DelegationLockingPeriod = 3
	c.Hardforks.AspenHf.BlockNumPartTwo = 0
	c.Hardforks.AspenHf.MaxSupply = big.NewInt(10_000_000_000)
	c.Hardforks.CornusHf.DelegationLockingPeriod = 3
	c.Hardforks.CornusHf.PbftGasLimit = fullBlockGas
	c.Hardforks.CornusHf.DagGasLimit = fullBlockGas
	c.Hardforks.SoleiroliaHf.BlockNum = 0
	c.Hardforks.SoleiroliaHf.TrxMinGasPrice = 1
	c.Hardforks.SoleiroliaHf.TrxMaxGasLimit = fullBlockGas
	c.Hardforks.MagnoliaHf.JailTime = 4
	c.Hardforks.CactiHf.BlockNum = 0
	if bracket {
		c.Hardforks.CactiHf.BlockNum = 4
	}
	c.Hardforks.CactiHf.JailTime = 2
	c.Hardforks.CactiHf.DelegationLockingPeriod = 2
	return c
}

func integrationABI(slashingContract bool) abi.ABI {
	metadata := dpos_sol.TaraxaDposClientMetaData
	if slashingContract {
		metadata = slash_sol.TaraxaSlashingClientMetaData
	}
	a, err := abi.JSON(strings.NewReader(metadata))
	must(err)
	return a
}
func integrationPack(slash bool, name string, args ...any) []byte {
	a := integrationABI(slash)
	b, err := a.Pack(name, args...)
	must(err)
	return b
}
func integrationProof(index int) []byte {
	h := crypto.Keccak256(integrationValidators[index][:])
	sig, err := secp256k1.Sign(h, integrationValidatorKeys[index])
	must(err)
	sig[64] += 27
	return sig
}
func integrationVote(index int, hash byte) []byte {
	v := slashing.Vote{BlockHash: common.Hash{hash}, VrfSortition: slashing.VrfPbftSortition{Period: 3, Round: 2, Step: 4}}
	v.VrfSortitionBytes = rlp.MustEncodeToBytes(v.VrfSortition)
	sig, err := secp256k1.Sign(v.GetHash().Bytes(), integrationValidatorKeys[index])
	must(err)
	copy(v.Signature[:], sig)
	return rlp.MustEncodeToBytes(v)
}

func integrationInputs(bracket bool) ([]fullPeriodInput, map[string]any, []common.Address) {
	ps := make([]fullPeriodInput, 12)
	for i := range ps {
		ps[i].number = uint64(i + 1)
	}
	nonces := make([]uint64, len(integrationKeys))
	add := func(p int, who int, name string, value uint64, slash bool, input []byte, expected string) {
		address := dpos.ContractAddress()
		if slash {
			address = slashing.ContractAddress()
		}
		spec := fullSpec(name, nonces[who], addressForPrivateKey(integrationKeys[who]), integrationKeys[who], address, value, input)
		spec.ExpectedError = expected
		nonces[who]++
		ps[p-1].transactions = append(ps[p-1].transactions, spec)
	}
	a, b, c := integrationValidators[0], integrationValidators[1], integrationValidators[2]
	for i, v := range integrationValidators {
		add(1, i, fmt.Sprintf("register_%d", i), 1000, false, integrationPack(false, "registerValidator", v, integrationProof(i), bytes.Repeat([]byte{byte(0x44 + i)}, 32), uint16(100), "validator", "endpoint"), "")
	}
	for _, who := range []int{3, 4} {
		for _, v := range []common.Address{a, b} {
			add(2, who, "delegate", 200, false, integrationPack(false, "delegate", v), "")
		}
	}
	add(2, 4, "delegate_c", 200, false, integrationPack(false, "delegate", c), "")
	add(3, 0, "metadata", 0, false, integrationPack(false, "setValidatorInfo", a, "updated", "https://fixture.invalid"), "")
	add(3, 0, "commission", 0, false, integrationPack(false, "setCommission", a, uint16(150)), "")
	add(3, 3, "claim", 0, false, integrationPack(false, "claimRewards", a), "")
	add(4, 3, "redelegate_existing", 0, false, integrationPack(false, "reDelegate", a, b, big.NewInt(50)), "")
	add(4, 3, "redelegate_new", 0, false, integrationPack(false, "reDelegate", a, c, big.NewInt(25)), "")
	proof := integrationPack(true, "commitDoubleVotingProof", integrationVote(0, 1), integrationVote(0, 2))
	add(4, 4, "slash_a", 0, true, proof, "")
	add(4, 4, "slash_duplicate", 0, true, proof, "Existing double voting proof")
	add(4, 4, "delayed_jail", 0, true, integrationPack(true, "getJailBlock", a), "")
	add(4, 4, "delayed_jailed", 0, true, integrationPack(true, "getJailedValidators"), "")
	add(5, 3, "claim_all", 0, false, integrationPack(false, "claimAllRewards"), "")
	add(5, 3, "withdraw_v1", 0, false, integrationPack(false, "undelegate", b, big.NewInt(10)), "")
	add(5, 3, "cancel_v1", 0, false, integrationPack(false, "cancelUndelegate", b), "")
	add(5, 3, "withdraw_v2", 0, false, integrationPack(false, "undelegateV2", b, big.NewInt(10)), "")
	add(5, 3, "cancel_v2", 0, false, integrationPack(false, "cancelUndelegateV2", b, uint64(1)), "")
	add(6, 3, "redelegate_full", 0, false, integrationPack(false, "reDelegate", a, b, big.NewInt(125)), "")
	add(6, 4, "survivor_claim", 0, false, integrationPack(false, "claimRewards", a), "")
	add(7, 0, "commission_claim", 0, false, integrationPack(false, "claimCommissionRewards", a), "")
	// All C principal is withdrawn; B remains the reward author/survivor.
	add(8, 2, "withdraw_owner_c", 0, false, integrationPack(false, "undelegate", c, big.NewInt(1000)), "")
	add(8, 3, "withdraw_new_c", 0, false, integrationPack(false, "undelegateV2", c, big.NewInt(25)), "")
	add(8, 4, "withdraw_other_c", 0, false, integrationPack(false, "undelegateV2", c, big.NewInt(200)), "")
	add(8, 2, "clear_commission_c", 0, false, integrationPack(false, "claimCommissionRewards", c), "")
	add(9, 2, "locked_c", 0, false, integrationPack(false, "confirmUndelegate", c), "Undelegation is not yet ready to be withdrawn")
	add(10, 2, "confirm_owner_c", 0, false, integrationPack(false, "confirmUndelegate", c), "")
	add(10, 3, "confirm_new_c", 0, false, integrationPack(false, "confirmUndelegateV2", c, uint64(2)), "")
	add(10, 4, "confirm_last_c", 0, false, integrationPack(false, "confirmUndelegateV2", c, uint64(1)), "")
	for _, p := range []int{7, 11, 12} {
		who := 4
		d := addressForPrivateKey(integrationKeys[who])
		queries := [][]byte{
			integrationPack(false, "getValidator", a), integrationPack(false, "getValidators", uint32(0)), integrationPack(false, "getValidatorsFor", addressForPrivateKey(integrationKeys[0]), uint32(0)),
			integrationPack(false, "getTotalDelegation", d), integrationPack(false, "getDelegations", d, uint32(0)), integrationPack(false, "getUndelegations", d, uint32(0)), integrationPack(false, "getUndelegationsV2", d, uint32(0)),
			integrationPack(false, "isValidatorEligible", a), integrationPack(false, "getValidatorEligibleVotesCount", a), integrationPack(false, "getTotalEligibleVotesCount"),
		}
		for _, q := range queries {
			add(p, who, "query", 0, false, q, "")
		}
		add(p, who, "jail_query", 0, true, integrationPack(true, "getJailBlock", a), "")
		add(p, who, "list_query", 0, true, integrationPack(true, "getJailedValidators"), "")
	}
	// Read the live V2 record while it exists and exercise escrow's exact selector.
	add(9, 4, "get_v2", 0, false, integrationPack(false, "getUndelegationV2", addressForPrivateKey(integrationKeys[4]), c, uint64(1)), "")
	add(12, 4, "escrow", 1, false, common.FromHex("44df8e70"), "")
	owner := addressForPrivateKey(integrationKeys[0])
	ordinary := crypto.CreateAddress(&owner, big.NewInt(1))
	wrapper := crypto.CreateAddress(&owner, big.NewInt(2))
	runtime := common.FromHex("6001600055600160006000a160006000f3")
	parentRuntime := common.FromHex("36600060003760006000366000600060ee620186a0f160006000fd")
	deploy := fullSpec("deploy_storage_log", 0, owner, integrationKeys[0], nil, 7, initRuntime(runtime))
	deploy.ExpectedContract = &ordinary
	ps[0].transactions = append(ps[0].transactions, deploy)
	deployParent := fullSpec("deploy_parent", 0, owner, integrationKeys[0], nil, 0, initRuntime(parentRuntime))
	deployParent.ExpectedContract = &wrapper
	ps[0].transactions = append(ps[0].transactions, deployParent)
	ordinaryCall := fullSpec("ordinary_storage_value_log", 0, owner, integrationKeys[0], &ordinary, 3, nil)
	ps[1].transactions = append(ps[1].transactions, ordinaryCall)
	parent := fullSpec("parent_revert_with_native_slash", 0, addressForPrivateKey(integrationKeys[4]), integrationKeys[4], &wrapper, 0, integrationPack(true, "commitDoubleVotingProof", integrationVote(0, 3), integrationVote(0, 4)))
	parent.ExpectedError = "execution reverted"
	ps[4].transactions = append(ps[4].transactions, parent)
	if bracket {
		add(3, 0, "pre_cacti_withdraw", 0, false, integrationPack(false, "undelegate", a, big.NewInt(1)), "")
		add(3, 0, "pre_cacti_cancel", 0, false, integrationPack(false, "cancelUndelegate", a), "")
		add(3, 4, "pre_cacti_slash", 0, true, integrationPack(true, "commitDoubleVotingProof", integrationVote(0, 5), integrationVote(0, 6)), "")
		add(4, 0, "at_cacti_withdraw", 0, false, integrationPack(false, "undelegateV2", a, big.NewInt(1)), "")
		add(4, 0, "at_cacti_cancel", 0, false, integrationPack(false, "cancelUndelegateV2", a, uint64(1)), "")
	}
	addresses := []common.Address{a, b, c, *dpos.ContractAddress(), *slashing.ContractAddress(), ordinary, wrapper}
	allocations := []map[string]string{}
	for _, key := range integrationKeys {
		v := addressForPrivateKey(key)
		addresses = append(addresses, v)
		allocations = append(allocations, map[string]string{"address": hex.EncodeToString(v[:]), "balance": "1000000000"})
	}
	inputs := map[string]any{"genesis_allocations": allocations, "initial_validators": []any{}, "initial_validator": map[string]any{"address": hex.EncodeToString(a[:]), "owner": hex.EncodeToString(owner[:]), "vrf_key": strings.Repeat("44", 32), "commission": 100, "delegations": []any{}}}
	chronologicalNonces := map[common.Address]uint64{}
	for i := range ps {
		for j := range ps[i].transactions {
			spec := &ps[i].transactions[j]
			sender := addressForPrivateKey(spec.PrivateKey)
			spec.Nonce = chronologicalNonces[sender]
			chronologicalNonces[sender]++
		}
	}
	return ps, inputs, addresses
}

func integrationPeriod(st *state_transition.StateTransition, latest *memoryLatest, cfg chain_config.ChainConfig, observer bool, input fullPeriodInput, identities *catalogIdentities, addresses []common.Address) map[string]any {
	author := integrationValidators[1]
	beginRawWriteTrace()
	st.BeginBlock(&vm.BlockInfo{Author: author, GasLimit: fullBlockGas, Difficulty: new(big.Int)})
	begin := finishRawWriteTrace()
	rows := []map[string]any{}
	hashes := []string{}
	fees := new(big.Int)
	var gas uint64
	for index, spec := range input.transactions {
		sender := addressForPrivateKey(spec.PrivateKey)
		signed := signTransaction(spec.transactionSpec, sender, spec.PrivateKey)
		tx := transactionFromSigned(spec.transactionSpec, signed, sender)
		beginRawWriteTrace()
		beginGasRefundTrace()
		result := st.ExecuteTransaction(&tx)
		refund := finishGasRefundTrace()
		writes := finishRawWriteTrace()
		gas += result.GasUsed
		expectedInput := common.CopyBytes(signed.Decoded.Input)
		registrationABI := integrationABI(false)
		if len(expectedInput) >= 4 && bytes.Equal(expectedInput[:4], registrationABI.Methods["registerValidator"].Id()) {
			offset := new(big.Int).SetBytes(expectedInput[36:68]).Uint64()
			expectedInput[4+offset+32+64] -= 27
		}
		if !bytes.Equal(tx.Input, expectedInput) {
			panic("unexplained reference working-input mutation")
		}
		original := transactionFromSigned(spec.transactionSpec, signed, sender)
		row := fullTransactionRow(spec, signed, original, result, gas)
		row["executed_input_after"] = hex.EncodeToString(tx.Input)
		row["index"] = index
		attachRawWrites(row, writes)
		fee := new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed), tx.GasPrice)
		row["fee"] = fee.String()
		fees.Add(fees, fee)
		hashes = append(hashes, hex.EncodeToString(signed.Hash[:]))
		if rawWriteTraceAvailable() {
			row["gas_refund"] = refund
		}
		if observer {
			root, ids, reader := observerFinalizeTransaction(st, uint64(index))
			*identities = mergeCatalogs(*identities, catalogIdentityClosure(ids))
			row["observer"] = map[string]any{"root": hex.EncodeToString(root[:]), "catalog": captureCatalog(reader, ids), "catalog_identities": jsonValue(ids), "accounts": observeAddressSet(reader, addresses)}
		}
		rows = append(rows, row)
	}
	factory := func(p types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(p)}
	}
	prior := new(dpos.API).Init(cfg).NewReader(types.BlockNum(input.number-1), factory)
	eligible := prior.TotalEligibleVoteCount()
	votes := prior.GetEligibleVoteCount(&author)
	stats := rewards_stats.RewardsStats{BlockAuthor: author, BlocksPerYear: 100, ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{}}
	facts := []map[string]any{}
	certs := []map[string]any{}
	dags := []map[string]any{}
	// Every transaction is attributed once to B's minimum-difficulty DAG block.
	// Certificates use the synthetic committee facts; no votes exist at genesis.
	for _, v := range []common.Address{author, integrationValidators[0], integrationValidators[2]} {
		weight := uint64(0)
		if input.number > 1 && prior.GetEligibleVoteCount(&v) > 0 {
			weight = 1
		}
		if v != author && weight == 0 {
			continue
		}
		if input.number > 1 && v == author && weight == 0 {
			panic("survivor B is not eligible")
		}
		fee := new(big.Int)
		txs := []string{}
		if v == author {
			fee.Set(fees)
			txs = hashes
		}
		stats.ValidatorsStats[v] = rewards_stats.ValidatorStats{DagBlocksCount: 1, VoteWeight: weight, FeesRewards: fee}
		stats.TotalDagBlocksCount++
		stats.TotalVotesWeight += weight
		facts = append(facts, map[string]any{"validator": hex.EncodeToString(v[:]), "dag_blocks_count": 1, "vote_weight": weight, "fees_reward": fee.String(), "reward_key": hex.EncodeToString(crypto.Keccak256([]byte{0, 2}, v[:]))})
		dags = append(dags, map[string]any{"author": hex.EncodeToString(v[:]), "difficulty": 1, "transaction_hashes": txs})
		if weight > 0 {
			certs = append(certs, map[string]any{"validator": hex.EncodeToString(v[:]), "period": input.number, "weight": weight})
		}
	}
	if input.number > 1 {
		stats.MaxVotesWeight = 1
	}
	beginRawWriteTrace()
	minted := st.DistributeRewards(&stats)
	rewardWrites := finishRawWriteTrace()
	if input.number > 1 && minted.IsZero() {
		panic("expected positive minted reward")
	}
	beginRawWriteTrace()
	st.EndBlock()
	end := finishRawWriteTrace()
	if observer {
		observerRecordRewards(st)
	}
	root := st.PrepareCommit()
	var idsJSON, finalCatalog any
	if observer {
		ids, reader := observerPeriodCatalog(st)
		*identities = mergeCatalogs(*identities, catalogIdentityClosure(ids))
		idsJSON = jsonValue(ids)
		closure := *identities
		closure.Invocations = ids.Invocations
		finalCatalog = captureNativeCatalog(reader, closure)
	}
	committed := st.Commit()
	if root != committed {
		panic("prepared/committed root mismatch")
	}
	reader := state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(input.number))}
	return map[string]any{"number": input.number, "transactions": rows, "planner_facts": map[string]any{"dag_blocks": dags, "certificate_votes": certs, "total_eligible_vote_count": eligible, "validator_eligible_vote_count": votes, "committee_size": 1, "pillar_interval": 1000, "is_pillar_boundary": false},
		"reward_input":  map[string]any{"block_author": hex.EncodeToString(author[:]), "blocks_per_year": 100, "total_dag_blocks_count": stats.TotalDagBlocksCount, "total_votes_weight": stats.TotalVotesWeight, "max_votes_weight": stats.MaxVotesWeight, "validators": facts},
		"reward_output": map[string]string{"actual_transaction_fees": fees.String(), "minted_reward": minted.ToBig().String()}, "period_catalog_identities": idsJSON,
		"ordered_raw_writes": map[string]any{"begin_block": begin, "rewards": rewardWrites, "end_block": end},
		"final":              map[string]any{"root": hex.EncodeToString(root[:]), "committed_root": hex.EncodeToString(committed[:]), "rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(), "accounts": observeAddressSet(reader, addresses), "native_storage_by_hashed_path": nativeStorageByPath(reader), "native_catalog": finalCatalog}}
}

func integrationRun(mode string, bracket bool) map[string]any {
	observer := mode == "observer"
	if observer && !observerAvailable() {
		panic("observer unavailable")
	}
	cfg := integrationConfig(bracket)
	periods, inputs, addresses := integrationInputs(bracket)
	latest := newMemoryLatest()
	beginRawWriteTrace()
	st := newStateTransition(latest, &cfg)
	writes := finishRawWriteTrace()
	ids := catalogIdentities{}
	descriptor := latest.GetCommittedDescriptor()
	reader := state_db.ExtendedReader{Reader: latest.readerAt(0)}
	genesis := map[string]any{"period": 0, "root": hex.EncodeToString(descriptor.StateRoot[:]), "rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(), "accounts": observeAddressSet(reader, addresses), "native_storage_by_hashed_path": nativeStorageByPath(reader)}
	attachRawWrites(genesis, writes)
	if observer {
		g, r := observerGenesisCatalog(st)
		ids = mergeCatalogs(ids, catalogIdentityClosure(g))
		genesis["native_catalog"] = captureNativeCatalog(r, ids)
	}
	out := []map[string]any{}
	for _, p := range periods {
		out = append(out, integrationPeriod(st, latest, cfg, observer, p, &ids, addresses))
		if p.number == 9 {
			st.Close()
			st = newStateTransition(latest, &cfg)
		}
	}
	st.Close()
	registry := []map[string]string{}
	for _, slash := range []bool{false, true} {
		a := integrationABI(slash)
		contract := "dpos"
		if slash {
			contract = "slashing"
		}
		for _, m := range a.Methods {
			registry = append(registry, map[string]string{"contract": contract, "signature": m.Sig(), "selector": hex.EncodeToString(m.Id())})
		}
	}
	registry = append(registry, map[string]string{"contract": "dpos", "signature": "transferIntoDPoSContract()", "selector": "44df8e70"})
	sort.Slice(registry, func(i, j int) bool { return registry[i]["signature"] < registry[j]["signature"] })
	config := configurationJSON(cfg, 1)
	config["block_gas_limit"] = fullBlockGas
	config["transaction_gas_limit"] = fullTransactionGas
	config["active_rules"] = []string{"Magnolia", "AspenPartOne", "AspenPartTwo", "Ficus", "Cornus", "Soleirolia", "Cacti"}
	if integrationAPIMode {
		roots := map[string]string{"0": hex.EncodeToString(descriptor.StateRoot[:])}
		for _, period := range out {
			roots[fmt.Sprint(period["number"])] = period["final"].(map[string]any)["root"].(string)
		}
		return map[string]any{"schema": 1, "kind": "historical_api", "profile": "synthetic_current", "configuration": config, "registry": registry,
			"seed_roots": roots, "cases": integrationAPIs(latest, cfg, periods, ids, addresses), "interacting_traces": integrationInteractingTraces(latest, cfg, ids, addresses)}
	}
	return map[string]any{"schema": 1, "execution_mode": mode, "profile": "synthetic_current", "cacti_bracket": bracket, "configuration": config, "inputs": inputs, "genesis": genesis, "periods": out, "registry": registry, "reopen": map[string]any{"close_reopen_after_periods": []uint64{9}, "continued_after_each_reopen": true}, "period_bound": map[string]uint64{"last_custody": 8, "locking": 2, "delay": 1, "continuation": 1, "end": 12}}
}
func main() {
	mode := flag.String("mode", "batched", "reference lifecycle")
	bracket := flag.Bool("bracket", false, "Cacti activation bracket")
	api := flag.Bool("api", false, "historical API corpus")
	flag.Parse()
	integrationAPIMode = *api
	b, err := json.Marshal(integrationRun(*mode, *bracket))
	must(err)
	fmt.Println(string(b))
}

// Existing row-model owner also supplies the public dry/trace reader interface.
func (m *memoryLatest) GetBlockStateReader(period types.BlockNum) state_db.Reader {
	return m.readerAt(period)
}
func (m *memoryLatest) GetLatestState() state_db.LatestState { return m }

type integrationAPIRequest struct {
	name   string
	from   common.Address
	to     common.Address
	value  uint64
	input  []byte
	create bool
}

func (r integrationAPIRequest) transaction(gas uint64, sequence uint64) vm.Transaction {
	target := &r.to
	if r.create {
		target = nil
	}
	return vm.Transaction{From: r.from, To: target, Nonce: new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 512), new(big.Int).SetUint64(sequence)),
		GasPrice: big.NewInt(1), Gas: gas, Value: new(big.Int).SetUint64(r.value), Input: common.CopyBytes(r.input)}
}
func integrationAPIResult(result vm.ExecutionResult) map[string]any {
	logs := make([]map[string]any, 0, len(result.Logs))
	for _, log := range result.Logs {
		topics := make([]string, 0, len(log.Topics))
		for _, topic := range log.Topics {
			topics = append(topics, hex.EncodeToString(topic[:]))
		}
		logs = append(logs, map[string]any{"address": hex.EncodeToString(log.Address[:]), "topics": topics, "data": hex.EncodeToString(log.Data)})
	}
	return map[string]any{"gas_used": result.GasUsed, "consensus_error": string(result.ConsensusErr), "execution_error": string(result.ExecutionErr),
		"output": hex.EncodeToString(result.CodeRetval), "logs": logs, "execution_result_rlp": hex.EncodeToString(rlp.MustEncodeToBytes(&result))}
}
func integrationAPIInput(r integrationAPIRequest) map[string]any {
	var target any = hex.EncodeToString(r.to[:])
	if r.create {
		target = nil
	}
	return map[string]any{"name": r.name, "sender": hex.EncodeToString(r.from[:]), "to": target, "value": fmt.Sprint(r.value), "gas_price": "1", "input": hex.EncodeToString(r.input), "supplied_nonce": new(big.Int).Lsh(big.NewInt(1), 512).String()}
}
func integrationAPIBlock(period uint64) *vm.Block {
	return &vm.Block{Number: types.BlockNum(period), BlockInfo: vm.BlockInfo{Author: integrationValidators[1], GasLimit: fullBlockGas, Difficulty: new(big.Int)}}
}
func integrationLive(state *state_evm.BlockState, ids catalogIdentities, addresses []common.Address, writes []rawWrite) map[string]any {
	slots := map[catalogSlot]bool{}
	for _, slot := range ids.Slots {
		slots[slot] = true
	}
	for _, write := range writes {
		slots[catalogSlot{Address: common.HexToAddress(write.Address), Key: common.HexToHash(write.Key)}] = true
	}
	raw := []map[string]any{}
	for slot := range slots {
		value := []byte{}
		callback := false
		state.GetAccountConcrete(&slot.Address).GetRawState(&slot.Key, func(b []byte) { callback = true; value = common.CopyBytes(b) })
		raw = append(raw, map[string]any{"address": hex.EncodeToString(slot.Address[:]), "key": hex.EncodeToString(slot.Key[:]), "callback": callback, "value": hex.EncodeToString(value)})
	}
	sort.Slice(raw, func(i, j int) bool {
		return raw[i]["address"].(string)+raw[i]["key"].(string) < raw[j]["address"].(string)+raw[j]["key"].(string)
	})
	accounts := []map[string]any{}
	for _, address := range addresses {
		account := state.GetAccountConcrete(&address)
		accounts = append(accounts, map[string]any{"address": hex.EncodeToString(address[:]), "nonce": account.GetNonce().String(), "balance": account.GetBalance().String()})
	}
	return map[string]any{"slots": raw, "accounts": accounts}
}
func integrationTrace(latest *memoryLatest, cfg chain_config.ChainConfig, period uint64, prefix []integrationAPIRequest, target integrationAPIRequest, ids catalogIdentities, addresses []common.Address) (output map[string]any) {
	output = map[string]any{"calls": []map[string]any{}, "prefix": []map[string]any{}, "request": integrationAPIInput(target)}
	prefixRows := []map[string]any{}
	transactions := []vm.Transaction{}
	for i, r := range prefix {
		tx := r.transaction(fullTransactionGas, uint64(i))
		transactions = append(transactions, tx)
		row := integrationAPIInput(r)
		row["supplied_nonce"] = tx.Nonce.String()
		prefixRows = append(prefixRows, row)
	}
	output["prefix"] = prefixRows
	targetTx := target.transaction(fullTransactionGas, uint64(len(prefix)))
	targets := []vm.Transaction{targetTx}
	output["request"].(map[string]any)["supplied_nonce"] = targetTx.Nonce.String()
	calls := []map[string]any{}
	var current map[string]any
	state_dry_runner.SetRetainedTraceObserver(func(stage string, after bool, state *state_evm.BlockState, tx *vm.Transaction, result *vm.ExecutionResult, err error) {
		if !after {
			beginRawWriteTrace()
			current = map[string]any{"stage": stage, "before": integrationLive(state, ids, addresses, nil)}
			return
		}
		writes := finishRawWriteTrace()
		current["after"] = integrationLive(state, ids, addresses, writes)
		current["ordered_raw_writes"] = writes
		current["result"] = integrationAPIResult(*result)
		message := ""
		if err != nil {
			message = err.Error()
		}
		current["error"] = message
		calls = append(calls, current)
		current = nil
		for _, write := range writes {
			ids.Slots = append(append([]catalogSlot{}, ids.Slots...), catalogSlot{Address: common.HexToAddress(write.Address), Key: common.HexToHash(write.Key)})
		}
	})
	defer state_dry_runner.SetRetainedTraceObserver(nil)
	defer func() {
		if recovered := recover(); recovered != nil {
			if current != nil {
				current["interrupted_raw_writes"] = finishRawWriteTrace()
				output["interrupted_call"] = current
			}
			output["panic"] = fmt.Sprint(recovered)
			output["panic_stack"] = string(debug.Stack())
			output["calls"] = calls
		}
	}()
	factory := func(p types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(p)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(latest, func(types.BlockNum) *big.Int { return new(big.Int) }, new(dpos.API).Init(cfg), factory, &cfg)
	// TraceRunner opens period-1 and shares its mutable block state for all calls.
	var result any
	result = runner.Trace(integrationAPIBlock(period), &transactions, &targets, nil)
	output["result"] = json.RawMessage(result.([]byte))
	output["calls"] = calls
	return output
}
func integrationAPIs(latest *memoryLatest, cfg chain_config.ChainConfig, periods []fullPeriodInput, ids catalogIdentities, addresses []common.Address) []map[string]any {
	requests := []integrationAPIRequest{}
	seen := map[string]bool{}
	for _, period := range periods {
		for _, spec := range period.transactions {
			if spec.To == nil || (*spec.To != *dpos.ContractAddress() && *spec.To != *slashing.ContractAddress()) || len(spec.Input) < 4 {
				continue
			}
			selector := hex.EncodeToString(spec.Input[:4])
			if seen[selector] {
				continue
			}
			seen[selector] = true
			requests = append(requests, integrationAPIRequest{name: spec.Name, from: addressForPrivateKey(spec.PrivateKey), to: *spec.To, value: spec.Value, input: common.CopyBytes(spec.Input)})
		}
	}
	key := bytes.Repeat([]byte{12}, 32)
	newValidator := addressForPrivateKey(key)
	sig, err := secp256k1.Sign(crypto.Keccak256(newValidator[:]), key)
	must(err)
	sig[64] += 27
	requests[0] = integrationAPIRequest{name: "registration_new", from: addressForPrivateKey(integrationKeys[0]), to: *dpos.ContractAddress(), value: 1000,
		input: integrationPack(false, "registerValidator", newValidator, sig, bytes.Repeat([]byte{0x77}, 32), uint16(100), "new", "endpoint")}
	registration := func(name string, proof, vrf []byte, commission uint16, description, endpoint string, value uint64) {
		requests = append(requests, integrationAPIRequest{name: name, from: addressForPrivateKey(integrationKeys[0]), to: *dpos.ContractAddress(), value: value,
			input: integrationPack(false, "registerValidator", newValidator, proof, vrf, commission, description, endpoint)})
	}
	vrf := bytes.Repeat([]byte{0x77}, 32)
	registration("registration_short_proof", sig[:64], vrf, 100, "new", "endpoint", 1000)
	registration("registration_wrong_proof", integrationProof(0), vrf, 100, "new", "endpoint", 1000)
	registration("registration_short_vrf", sig, vrf[:31], 100, "new", "endpoint", 1000)
	registration("registration_long_vrf", sig, append(common.CopyBytes(vrf), 1), 100, "new", "endpoint", 1000)
	registration("registration_long_description", sig, vrf, 100, strings.Repeat("d", 257), "endpoint", 1000)
	registration("registration_long_endpoint", sig, vrf, 100, "new", strings.Repeat("e", 257), 1000)
	registration("registration_commission_overflow", sig, vrf, 10001, "new", "endpoint", 1000)
	registration("registration_zero_stake", sig, vrf, 100, "new", "endpoint", 0)
	registration("registration_max_stake", sig, vrf, 100, "new", "endpoint", 1_000_001)
	dirty := common.CopyBytes(requests[0].input)
	dirty[4+96] = 0xff
	requests = append(requests, integrationAPIRequest{name: "registration_dirty_uint16", from: requests[0].from, to: requests[0].to, value: 1000, input: dirty})
	for _, def := range []struct {
		name        string
		left, right []byte
	}{
		{"slash_identical", integrationVote(0, 1), integrationVote(0, 1)},
		{"slash_different_validators", integrationVote(0, 7), integrationVote(1, 8)},
		{"slash_reversed_duplicate", integrationVote(0, 2), integrationVote(0, 1)},
	} {
		requests = append(requests, integrationAPIRequest{name: def.name, from: addressForPrivateKey(integrationKeys[4]), to: *slashing.ContractAddress(), input: integrationPack(true, "commitDoubleVotingProof", def.left, def.right)})
	}
	factory := func(p types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(p)}
	}
	seedBefore, _ := json.Marshal(latest.rows.exportPhysical())
	cases := []map[string]any{}
	for _, head := range []uint64{0, 3, 4, 7, 9, 11} {
		selectedRequests := append([]integrationAPIRequest{}, requests...)
		if head == 3 {
			for _, request := range requests[:29] {
				paid := request
				paid.name = "boundary/value/" + hex.EncodeToString(request.input[:4])
				paid.value = 1
				selectedRequests = append(selectedRequests, paid)
				if len(request.input) > 4 {
					malformed := request
					malformed.name = "boundary/selector_only/" + hex.EncodeToString(request.input[:4])
					malformed.input = common.CopyBytes(request.input[:4])
					malformed.value = 0
					selectedRequests = append(selectedRequests, malformed)
				}
			}
		}
		if head == 3 || head == 7 {
			voteA := slashing.Vote{BlockHash: common.Hash{1}, VrfSortition: slashing.VrfPbftSortition{Period: 3, Round: 2, Step: 4}}
			voteA.VrfSortitionBytes = rlp.MustEncodeToBytes(voteA.VrfSortition)
			voteA.Signature = [65]byte{}
			selectedRequests = append(selectedRequests, integrationAPIRequest{name: "boundary/slash_bad_signature_or_duplicate", from: addressForPrivateKey(integrationKeys[4]), to: *slashing.ContractAddress(), input: integrationPack(true, "commitDoubleVotingProof", rlp.MustEncodeToBytes(voteA), integrationVote(0, 2))})
		}
		if head == 3 {
			proofRequest := func(name string, left, right []byte) {
				selectedRequests = append(selectedRequests, integrationAPIRequest{name: "boundary/" + name, from: addressForPrivateKey(integrationKeys[4]), to: *slashing.ContractAddress(), input: integrationPack(true, "commitDoubleVotingProof", left, right)})
			}
			vote := func(hash common.Hash, period uint64, step uint32, key []byte) []byte {
				v := slashing.Vote{VrfSortition: slashing.VrfPbftSortition{Period: 3, Round: 2, Step: 4}}
				v.BlockHash = hash
				v.VrfSortition.Period = period
				v.VrfSortition.Step = step
				v.VrfSortitionBytes = rlp.MustEncodeToBytes(v.VrfSortition)
				signature, err := secp256k1.Sign(v.GetHash().Bytes(), key)
				must(err)
				copy(v.Signature[:], signature)
				return rlp.MustEncodeToBytes(v)
			}
			proofRequest("slash_slot_mismatch", integrationVote(0, 1), vote(common.Hash{2}, 4, 4, integrationValidatorKeys[0]))
			proofRequest("slash_next_mixed_null", vote(common.Hash{1}, 3, 5, integrationValidatorKeys[0]), vote(common.Hash{}, 3, 5, integrationValidatorKeys[0]))
			proofRequest("slash_non_validator", vote(common.Hash{7}, 3, 4, bytes.Repeat([]byte{12}, 32)), vote(common.Hash{8}, 3, 4, bytes.Repeat([]byte{12}, 32)))
		}
		for _, request := range selectedRequests {
			probe := func(gas uint64) map[string]any {
				runner := new(state_dry_runner.DryRunner).Init(latest, func(types.BlockNum) *big.Int { return new(big.Int) }, new(dpos.API).Init(cfg), factory, &cfg)
				tx := request.transaction(gas, 0)
				beginRawWriteTrace()
				result := runner.Apply(integrationAPIBlock(head), &tx)
				writes := finishRawWriteTrace()
				return map[string]any{"gas": gas, "effective_nonce": tx.Nonce.String(), "output": integrationAPIResult(result), "ordered_raw_writes": writes, "executed_input_after": hex.EncodeToString(tx.Input)}
			}
			call := probe(fullTransactionGas)
			estimates := []map[string]any{}
			used := call["output"].(map[string]any)["gas_used"].(uint64)
			lower := uint64(0)
			if used > 0 {
				lower = used - 1
			}
			for _, cap := range []uint64{fullTransactionGas, used, lower} {
				first := probe(cap)
				probes := []map[string]any{first}
				out := first["output"].(map[string]any)
				if out["consensus_error"] == "" && out["execution_error"] == "" {
					lo, hi := out["gas_used"].(uint64), cap
					for hi-lo > hi/20 {
						mid := lo + (hi-lo)/2
						r := probe(mid)
						probes = append(probes, r)
						out = r["output"].(map[string]any)
						if out["consensus_error"] != "" {
							break
						}
						if out["execution_error"] == "" {
							hi = mid
						} else {
							lo = mid
						}
					}
				}
				estimates = append(estimates, map[string]any{"name": fmt.Sprintf("%d/%s/%d", head, request.name, cap), "cap": cap, "probes": probes})
			}
			prefix := []integrationAPIRequest{request}
			trace := integrationTrace(latest, cfg, head+1, nil, request, ids, addresses)
			sequence := integrationTrace(latest, cfg, head+1, prefix, request, ids, addresses)
			cases = append(cases, map[string]any{"name": fmt.Sprintf("%d/%s", head, request.name), "head": head, "request": integrationAPIInput(request), "call": call, "estimates": estimates, "trace": trace, "trace_sequence": sequence})
		}
	}
	seedAfter, _ := json.Marshal(latest.rows.exportPhysical())
	if !bytes.Equal(seedBefore, seedAfter) {
		panic("historical API changed committed source rows")
	}
	return cases
}

func integrationInteractingTraces(latest *memoryLatest, cfg chain_config.ChainConfig, ids catalogIdentities, addresses []common.Address) []map[string]any {
	owner := addressForPrivateKey(integrationKeys[0])
	delegator := addressForPrivateKey(integrationKeys[3])
	a, b := integrationValidators[0], integrationValidators[1]
	key := bytes.Repeat([]byte{12}, 32)
	fresh := addressForPrivateKey(key)
	signature, err := secp256k1.Sign(crypto.Keccak256(fresh[:]), key)
	must(err)
	signature[64] += 27
	addresses = append(append([]common.Address{}, addresses...), fresh)
	registration := integrationAPIRequest{name: "register_source", from: owner, to: *dpos.ContractAddress(), value: 1000, input: integrationPack(false, "registerValidator", fresh, signature, bytes.Repeat([]byte{0x77}, 32), uint16(0), "source", "endpoint")}
	dposRequest := func(name string, from common.Address, method string, args ...any) integrationAPIRequest {
		return integrationAPIRequest{name: name, from: from, to: *dpos.ContractAddress(), input: integrationPack(false, method, args...)}
	}
	slash := integrationAPIRequest{name: "slash", from: addressForPrivateKey(integrationKeys[4]), to: *slashing.ContractAddress(), input: integrationPack(true, "commitDoubleVotingProof", integrationVote(0, 1), integrationVote(0, 2))}
	jailed := integrationAPIRequest{name: "jailed", from: owner, to: *slashing.ContractAddress(), input: integrationPack(true, "getJailedValidators")}
	eligible := dposRequest("eligible", owner, "isValidatorEligible", a)
	total := dposRequest("total", owner, "getTotalEligibleVotesCount")
	definitions := []struct {
		name   string
		prefix []integrationAPIRequest
		target integrationAPIRequest
	}{
		{"rewarded_current_source_full", []integrationAPIRequest{dposRequest("partial", delegator, "reDelegate", a, b, big.NewInt(10))}, dposRequest("full", delegator, "reDelegate", a, b, big.NewInt(190))},
		{"current_source_validator_delete", []integrationAPIRequest{registration}, dposRequest("full", owner, "reDelegate", fresh, b, big.NewInt(1000))},
		{"current_source_retained_v1_custody", []integrationAPIRequest{registration, dposRequest("withdraw", owner, "undelegate", fresh, big.NewInt(10))}, dposRequest("full", owner, "reDelegate", fresh, b, big.NewInt(990))},
		{"current_source_retained_v2_custody", []integrationAPIRequest{registration, dposRequest("withdraw", owner, "undelegateV2", fresh, big.NewInt(10))}, dposRequest("full", owner, "reDelegate", fresh, b, big.NewInt(990))},
		{"cached_jail_list_read_slash_read", []integrationAPIRequest{jailed, slash}, jailed},
		{"cached_eligibility_read_slash_read", []integrationAPIRequest{eligible, slash}, eligible},
		{"cached_total_read_slash_read", []integrationAPIRequest{total, slash}, total},
	}
	output := []map[string]any{}
	for _, def := range definitions {
		output = append(output, map[string]any{"name": def.name, "head": uint64(3), "trace": integrationTrace(latest, cfg, 4, def.prefix, def.target, ids, addresses)})
	}
	periods, _, _ := integrationInputs(false)
	seen := map[string]bool{}
	for _, period := range periods {
		for _, spec := range period.transactions {
			if spec.To == nil || (*spec.To != *dpos.ContractAddress() && *spec.To != *slashing.ContractAddress()) || len(spec.Input) < 4 {
				continue
			}
			selector := hex.EncodeToString(spec.Input[:4])
			if seen[selector] {
				continue
			}
			seen[selector] = true
			sender := addressForPrivateKey(spec.PrivateKey)
			runtime := append(common.FromHex("366000600037600060003660003473"), spec.To[:]...)
			runtime = append(runtime, common.FromHex("62061a80f15060006000fd")...)
			deploy := integrationAPIRequest{name: "deploy_parent", from: sender, create: true, input: initRuntime(runtime)}
			wrapper := crypto.CreateAddress(&sender, new(big.Int).Lsh(big.NewInt(1), 512))
			target := integrationAPIRequest{name: "nested_" + selector, from: sender, to: wrapper, value: spec.Value, input: common.CopyBytes(spec.Input)}
			if selector == hex.EncodeToString(integrationABI(false).Methods["registerValidator"].Id()) {
				target.input = common.CopyBytes(registration.input)
			}
			trace := integrationTrace(latest, cfg, 4, []integrationAPIRequest{deploy}, target, ids, append(append([]common.Address{}, addresses...), wrapper))
			output = append(output, map[string]any{"name": "nested_parent_revert/" + selector, "head": uint64(3), "trace": trace, "native_selector": selector, "creation_prefix": true})
		}
	}
	// Registration borrows CALL memory. Exercise nonzero input offset,
	// overlapping empty return copy, repeated proof-byte mutation and revert.
	for _, repeat := range []bool{false, true} {
		inputOffset := byte(32)
		proofOffset := new(big.Int).SetBytes(registration.input[36:68]).Uint64() + 4 + 32 + 64 + uint64(inputOffset)
		runtime := common.FromHex("366000602037")
		call := func(valueOpcode []byte) []byte {
			code := []byte{0x60, 0x20, 0x61, byte(proofOffset >> 8), byte(proofOffset), 0x36, 0x60, inputOffset}
			code = append(code, valueOpcode...)
			code = append(code, 0x73)
			target := dpos.ContractAddress()
			code = append(code, target[:]...)
			return append(code, common.FromHex("62061a80f150")...)
		}
		runtime = append(runtime, call([]byte{0x34})...)
		count := 1
		if repeat {
			runtime = append(runtime, call([]byte{0x60, 0})...)
			count = 2
		}
		runtime = append(runtime, common.FromHex("60006000fd")...)
		deploy := integrationAPIRequest{name: "deploy_alias_parent", from: owner, create: true, input: initRuntime(runtime)}
		wrapper := crypto.CreateAddress(&owner, new(big.Int).Lsh(big.NewInt(1), 512))
		target := integrationAPIRequest{name: "borrowed_proof", from: owner, to: wrapper, value: 1000, input: common.CopyBytes(registration.input)}
		trace := integrationTrace(latest, cfg, 4, []integrationAPIRequest{deploy}, target, ids, append(append([]common.Address{}, addresses...), wrapper))
		output = append(output, map[string]any{"name": fmt.Sprintf("registration_input_alias/repeat=%t", repeat), "head": uint64(3), "trace": trace, "creation_prefix": true, "native_invocations": count, "untraced": true})
	}

	return output
}
