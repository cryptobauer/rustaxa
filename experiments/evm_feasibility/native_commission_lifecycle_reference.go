// Synthetic dual-fork lifecycle seeds. All native state comes from real transitions.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"
	"reflect"
	"sort"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

var lifecycleRows = map[string]nativeSimulationSeedRow{}

func lifecycleSeedRows(db *nativeSimulationDB) []string {
	ids := []string{}
	for _, row := range nativeSimulationSeedRows(db) {
		b, _ := json.Marshal(row)
		h := sha256.Sum256(b)
		id := hex.EncodeToString(h[:])
		lifecycleRows[id] = row
		ids = append(ids, id)
	}
	return ids
}

type lifecycleOperation struct {
	Name   string         `json:"name"`
	Sender common.Address `json:"sender"`
	Input  []byte         `json:"input"`
}

func lifecycleOp(name, signature string, sender common.Address, amount *big.Int) lifecycleOperation {
	return lifecycleOperation{name, sender, custodyInput(signature, amount)}
}
func lifecycleSnapshot(seed nativeSimulationSeed) map[string]any {
	p := seed.database.descriptor.BlockNum
	reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(p)}
	a := *dpos.ContractAddress()
	w := new(storage.StorageWrapper)
	w.StorageReaderWrapper.Init(&a, reader)
	validators := new(dpos.Validators)
	validators.Init(w, []byte{0})
	addresses, _ := validators.GetValidatorsAddresses(0, 100)
	rows := []map[string]any{}
	for _, v := range []common.Address{common.HexToAddress("31"), common.HexToAddress("32"), common.HexToAddress("33")} {
		row := map[string]any{"address": hex.EncodeToString(v[:]), "validator": validators.GetValidator(&v), "owner": validators.GetValidatorOwner(&v)}
		if row["validator"] != (*dpos.Validator)(nil) {
			row["rewards"] = validators.GetValidatorRewards(&v)
		}
		rows = append(rows, row)
	}
	queue := new(dpos.Undelegations)
	queue.Init(w, []byte{3})
	queues := []map[string]any{}
	for _, d := range []common.Address{custodyDelegator, retainedIdentity(0x32)} {
		queues = append(queues, map[string]any{"delegator": hex.EncodeToString(d[:]), "v1": queue.GetUndelegationV1(&d, &custodyValidator), "v1_count": queue.GetUndelegationsV1Count(&d), "v2_count": queue.GetUndelegationsV2Count(&d)})
	}
	raw := map[string]string{}
	for k := range retainedKeys {
		reader.GetAccountStorage(&a, &k, func(b []byte) { raw[hex.EncodeToString(k[:])] = hex.EncodeToString(b) })
	}
	accounts := []map[string]any{}
	for _, v := range []common.Address{custodyDelegator, retainedIdentity(0x32), custodyOwner, a} {
		row := map[string]any{"address": hex.EncodeToString(v[:]), "exists": false}
		reader.GetRawAccount(&v, func(b []byte) {
			acc := state_db.DecodeAccountFromTrie(b)
			row["exists"] = true
			row["encoded"] = hex.EncodeToString(b)
			row["nonce"] = acc.Nonce.String()
			row["balance"] = acc.Balance.String()
		})
		accounts = append(accounts, row)
	}
	return map[string]any{"period": uint64(p), "root": hex.EncodeToString(seed.database.descriptor.StateRoot[:]), "seed_rows": lifecycleSeedRows(seed.database), "native_raw": raw, "accounts": accounts, "validators": rows, "validator_order": addresses, "queues": queues}
}
func lifecycleTx(op lifecycleOperation, nonce *big.Int, gas uint64) vm.Transaction {
	a := *dpos.ContractAddress()
	return vm.Transaction{From: op.Sender, To: &a, Nonce: nonce, GasPrice: new(big.Int), Gas: gas, Value: new(big.Int), Input: op.Input}
}
func lifecycleResult(op lifecycleOperation, tx vm.Transaction, result vm.ExecutionResult) map[string]any {
	return map[string]any{"name": op.Name, "sender": hex.EncodeToString(tx.From[:]), "to": hex.EncodeToString(tx.To[:]), "input": hex.EncodeToString(tx.Input), "nonce": tx.Nonce.String(), "gas": tx.Gas, "gas_price": "0", "value": "0", "gas_used": result.GasUsed, "consensus_error": string(result.ConsensusErr), "execution_error": string(result.ExecutionErr), "output": hex.EncodeToString(result.CodeRetval), "logs": retainedLogs(result.Logs), "ordered_raw_writes": retainedWrites, "ordered_balance_effects": retainedBalances}
}
func lifecycleExecute(t *state_transition.StateTransition, op lifecycleOperation, nonces map[common.Address]uint64) map[string]any {
	tx := lifecycleTx(op, new(big.Int).SetUint64(nonces[op.Sender]), 200000)
	nonces[op.Sender]++
	retainedWrites = []custodyRawWrite{}
	retainedBalances = []map[string]string{}
	r := t.ExecuteTransaction(&tx)
	return lifecycleResult(op, tx, r)
}
func lifecycleBlock(p uint64) *vm.Block {
	return &vm.Block{Number: types.BlockNum(p), BlockInfo: vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000000 + p, Difficulty: new(big.Int)}}
}
func lifecycleFrame(seed nativeSimulationSeed, wide bool) map[string]any {
	// Explicit disposable code overlay, never deployed or committed.
	code := common.FromHex("366000600037600060003660003460fe62004e20f160005260206000fd")
	state := state_evm.GetBlockState(seed.database, seed.database.descriptor.BlockNum, 2)
	if wide {
		code[len(code)-1] = 0xf3
		state.GetAccountConcrete(dpos.ContractAddress()).AddBalance(new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 600), state.GetAccountConcrete(dpos.ContractAddress()).GetBalance()))
		state.GetAccountConcrete(&custodyOwner).SubBalance(new(big.Int).Add(big.NewInt(17), state.GetAccountConcrete(&custodyOwner).GetBalance()))
	}
	state.GetAccountConcrete(&custodyOwner).SetCode(code)
	var e vm.EVM
	e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), seed.config.EVMChainConfig, vm.Config{})
	e.SetBlock(lifecycleBlock(uint64(seed.database.descriptor.BlockNum)+1), seed.config.Hardforks.Rules(seed.database.descriptor.BlockNum+1))
	native := new(dpos.Contract)
	native.Init(seed.config, &storage.EVMStateStorage{EVMStateFace: state}, dpos.Reader{}, &e)
	observation := map[string]any{}
	native.Register(func(a *common.Address, actual vm.PrecompiledContract) {
		if os.Getenv("RETAINED_CONTROL") == "1" {
			e.RegisterPrecompiledContract(a, actual)
		} else {
			e.RegisterPrecompiledContract(a, &retainedObserver{PrecompiledContract: actual, state: state, observation: observation})
		}
	})
	accounts := func() map[string]any {
		out := map[string]any{}
		for _, a := range []common.Address{custodyOwner, custodyDelegator, retainedIdentity(0x32), *dpos.ContractAddress()} {
			acc := state.GetAccountConcrete(&a)
			out[hex.EncodeToString(a[:])] = map[string]string{"nonce": acc.GetNonce().String(), "balance": acc.GetBalance().String()}
		}
		return out
	}
	raw := func() map[string]string {
		out := map[string]string{}
		for k := range retainedKeys {
			v := []byte{}
			state.GetAccountConcrete(dpos.ContractAddress()).GetRawState(&k, func(b []byte) { v = common.CopyBytes(b) })
			if len(v) > 0 {
				out[hex.EncodeToString(k[:])] = hex.EncodeToString(v)
			}
		}
		return out
	}
	priorAccounts, priorRaw := accounts(), raw()
	retainedWrites = []custodyRawWrite{}
	retainedBalances = []map[string]string{}
	result, err := e.Main(&vm.Transaction{From: custodyDelegator, To: &custodyOwner, Nonce: new(big.Int).Set(state.GetAccountConcrete(&custodyDelegator).GetNonce()), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: custodyInput("claimCommissionRewards(address)", nil)})
	message := ""
	if err != nil {
		message = err.Error()
	}
	return map[string]any{"code_overlay": hex.EncodeToString(code), "input": hex.EncodeToString(custodyInput("claimCommissionRewards(address)", nil)), "sender": hex.EncodeToString(custodyDelegator[:]), "receiver": hex.EncodeToString(custodyOwner[:]), "gas_used": result.GasUsed, "parent_error": message, "output": hex.EncodeToString(result.CodeRetval), "logs": retainedLogs(result.Logs), "accounts_before": priorAccounts, "accounts_after": accounts(), "raw_before": priorRaw, "raw_after": raw(), "ordered_raw_writes": retainedWrites, "ordered_balance_effects": retainedBalances, "native_observation": observation}
}

func lifecycleRequest(tx vm.Transaction) map[string]any {
	return map[string]any{"sender": hex.EncodeToString(tx.From[:]), "to": hex.EncodeToString(tx.To[:]), "nonce": tx.Nonce.String(), "gas": tx.Gas, "gas_price": tx.GasPrice.String(), "value": tx.Value.String(), "input": hex.EncodeToString(tx.Input)}
}
func lifecycleLive(state *state_evm.BlockState) map[string]any {
	raw := map[string]string{}
	for k := range retainedKeys {
		state.GetAccountConcrete(dpos.ContractAddress()).GetRawState(&k, func(b []byte) { raw[hex.EncodeToString(k[:])] = hex.EncodeToString(b) })
	}
	accounts := map[string]any{}
	for _, a := range []common.Address{custodyOwner, custodyDelegator, retainedIdentity(0x32), *dpos.ContractAddress()} {
		acc := state.GetAccountConcrete(&a)
		accounts[hex.EncodeToString(a[:])] = map[string]any{"exists": !acc.IsNIL(), "balance": acc.GetBalance().String(), "nonce": acc.GetNonce().String()}
	}
	return map[string]any{"raw": raw, "accounts": accounts}
}

func lifecycleConsumers(seed nativeSimulationSeed, op lifecycleOperation) map[string]any {
	before := lifecycleSnapshot(seed)
	p := uint64(seed.database.descriptor.BlockNum)
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	probe := func(gas uint64) map[string]any {
		runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, new(dpos.API).Init(seed.config), factory, &seed.config)
		tx := lifecycleTx(op, new(big.Int).Lsh(big.NewInt(1), 512), gas)
		retainedWrites = []custodyRawWrite{}
		retainedBalances = []map[string]string{}
		result := runner.Apply(lifecycleBlock(p), &tx)
		row := lifecycleResult(op, tx, result)
		row["supplied_nonce"] = new(big.Int).Lsh(big.NewInt(1), 512).String()
		return row
	}
	call := probe(200000)
	estimates := []map[string]any{}
	// Measure adjacent gas caps from actual execution, each with fresh probes.
	for _, cap := range []uint64{200000, call["gas_used"].(uint64), call["gas_used"].(uint64) - 1} {
		first := probe(cap)
		probes := []map[string]any{first}
		if first["consensus_error"] == "" && first["execution_error"] == "" {
			lo, hi := first["gas_used"].(uint64), cap
			for hi-lo > hi/20 {
				mid := lo + (hi-lo)/2
				r := probe(mid)
				probes = append(probes, r)
				if r["consensus_error"] != "" {
					break
				}
				if r["execution_error"] == "" {
					hi = mid
				} else {
					lo = mid
				}
			}
		}
		estimates = append(estimates, map[string]any{"cap": cap, "probes": probes})
	}
	if !reflect.DeepEqual(call, probe(200000)) {
		panic("fresh call differs")
	}
	trace := func(sequence bool) map[string]any {
		runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, new(dpos.API).Init(seed.config), factory, &seed.config)
		prefix := []vm.Transaction{}
		targets := []vm.Transaction{lifecycleTx(op, new(big.Int).Lsh(big.NewInt(1), 512), 200000)}
		if sequence {
			prefix = append(prefix, targets[0])
			targets[0].Nonce = new(big.Int).Add(targets[0].Nonce, big.NewInt(1))
		}
		calls := []map[string]any{}
		var current map[string]any
		state_dry_runner.SetRetainedTraceObserver(func(stage string, after bool, state *state_evm.BlockState, request *vm.Transaction, result *vm.ExecutionResult, err error) {
			if !after {
				retainedWrites = []custodyRawWrite{}
				retainedBalances = []map[string]string{}
				current = map[string]any{"stage": stage, "request": lifecycleRequest(*request), "before": lifecycleLive(state)}
				return
			}
			message := ""
			if err != nil {
				message = err.Error()
			}
			current["error"] = message
			current["after"] = lifecycleLive(state)
			current["result"] = lifecycleResult(op, *request, *result)
			calls = append(calls, current)
		})
		r, w, err := os.Pipe()
		if err != nil {
			panic(err)
		}
		stdout := os.Stdout
		os.Stdout = w
		result := runner.Trace(lifecycleBlock(p+1), &prefix, &targets, nil)
		os.Stdout = stdout
		w.Close()
		var diagnostics []byte
		buf := make([]byte, 4096)
		for {
			n, e := r.Read(buf)
			diagnostics = append(diagnostics, buf[:n]...)
			if e != nil {
				break
			}
		}
		r.Close()
		requests := []map[string]any{}
		for _, tx := range append(prefix, targets...) {
			requests = append(requests, lifecycleRequest(tx))
		}
		return map[string]any{"result": json.RawMessage(result), "diagnostics": string(diagnostics), "requests": requests, "observation_witness": calls}
	}
	direct, sequence := trace(false), trace(true)
	if !reflect.DeepEqual(before, lifecycleSnapshot(seed)) {
		panic("consumer mutated committed seed")
	}
	return map[string]any{"call": call, "estimates": estimates, "trace": direct, "trace_sequence": sequence, "selected_period": p, "trace_execution_period": p + 1}
}
func lifecycleScenario(cacti bool, queue, phase, topology string) map[string]any {
	cfg := cancelRewardConfig()
	cfg.Hardforks.PhalaenopsisHfBlockNum = 0
	if cacti {
		cfg.Hardforks.CactiHf.BlockNum = 0
	}
	// Hold all other configuration values fixed, including the Cacti lock.
	other := retainedIdentity(0x32)
	cfg.GenesisBalances[other] = big.NewInt(1000)
	cfg.GenesisBalances[custodyDelegator] = big.NewInt(5000)
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(10000)
	base := cfg.DPOS.InitialValidators[0]
	base.Delegations = core.BalanceMap{custodyDelegator: big.NewInt(500), other: big.NewInt(500)}
	cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{base}
	if topology != "singleton" {
		left, right := base, base
		left.Address = common.HexToAddress("32")
		right.Address = common.HexToAddress("33")
		left.VrfKey = bytes.Repeat([]byte{0x55}, 32)
		right.VrfKey = bytes.Repeat([]byte{0x66}, 32)
		left.Delegations = core.BalanceMap{custodyDelegator: big.NewInt(1000)}
		right.Delegations = core.BalanceMap{custodyDelegator: big.NewInt(1000)}
		switch topology {
		case "first":
			cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{base, left, right}
		case "middle":
			cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{left, base, right}
		case "last":
			cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{left, right, base}
		}
	}
	db := newNativeSimulationDB()
	t := nativeSimulationTransition(db, &cfg)
	defer t.Close()
	seed := nativeSimulationSeed{database: db, config: cfg, wrapper: custodyOwner}
	nonces := map[common.Address]uint64{}
	provenance := []map[string]any{{"operation": "genesis", "state": lifecycleSnapshot(seed)}}
	t.BeginBlock(&lifecycleBlock(1).BlockInfo)
	stats := accruedCancelRewardStats()
	minted := t.DistributeRewards(&stats)
	t.EndBlock()
	t.Commit()
	provenance = append(provenance, map[string]any{"operation": "reward", "stats": stats, "minted": minted.String(), "state": lifecycleSnapshot(seed)})
	sigs := []string{"undelegate(address,uint256)", "undelegate(address,uint256)"}
	conf := []lifecycleOperation{lifecycleOp("confirm_first", "confirmUndelegate(address)", custodyDelegator, nil), lifecycleOp("confirm_last", "confirmUndelegate(address)", other, nil)}
	if queue == "v2" || queue == "mixed" {
		sigs[1] = "undelegateV2(address,uint256)"
		conf[1] = lifecycleOp("confirm_last", "confirmUndelegateV2(address,uint64)", other, big.NewInt(1))
	}
	if queue == "v2" {
		sigs[0] = "undelegateV2(address,uint256)"
		conf[0] = lifecycleOp("confirm_first", "confirmUndelegateV2(address,uint64)", custodyDelegator, big.NewInt(1))
	}
	if queue == "mixed_v1_last" {
		sigs[0] = "undelegateV2(address,uint256)"
		conf[0] = lifecycleOp("confirm_first", "confirmUndelegateV2(address,uint64)", custodyDelegator, big.NewInt(1))
	}
	t.BeginBlock(&lifecycleBlock(2).BlockInfo)
	setup := []map[string]any{}
	for i, d := range []common.Address{custodyDelegator, other} {
		row := lifecycleExecute(t, lifecycleOp("full_undelegation", sigs[i], d, big.NewInt(500)), nonces)
		if row["consensus_error"] != "" || row["execution_error"] != "" {
			panic(row)
		}
		setup = append(setup, row)
	}
	t.EndBlock()
	t.Commit()
	provenance = append(provenance, map[string]any{"operation": "full_undelegations", "transactions": setup, "state": lifecycleSnapshot(seed)})
	period := uint64(3)
	if phase != "pending" {
		mature := uint64(6)
		if cacti {
			mature = 10
		}
		for period < mature {
			t.BeginBlock(&lifecycleBlock(period).BlockInfo)
			t.EndBlock()
			t.Commit()
			period++
		}
		if phase == "partial" || phase == "final" {
			t.BeginBlock(&lifecycleBlock(period).BlockInfo)
			setup = []map[string]any{}
			count := 1
			if phase == "final" {
				count = 2
			}
			for _, op := range conf[:count] {
				row := lifecycleExecute(t, op, nonces)
				if row["consensus_error"] != "" || row["execution_error"] != "" {
					panic(row)
				}
				setup = append(setup, row)
			}
			t.EndBlock()
			t.Commit()
			provenance = append(provenance, map[string]any{"operation": "matured_confirmations", "transactions": setup, "state": lifecycleSnapshot(seed)})
			period++
		}
	}
	before := lifecycleSnapshot(seed)
	ops := []lifecycleOperation{lifecycleOp("wrong_owner", "claimCommissionRewards(address)", custodyDelegator, nil), lifecycleOp("positive", "claimCommissionRewards(address)", custodyOwner, nil), lifecycleOp("repeat", "claimCommissionRewards(address)", custodyOwner, nil)}
	if phase == "claim_before_final" {
		ops = append(ops, conf...)
		ops = append(ops, lifecycleOp("after_deletion", "claimCommissionRewards(address)", custodyOwner, nil))
	}
	// Each API request receives the same committed seed as its native operation.
	consumers := lifecycleConsumers(seed, ops[1])
	frame := lifecycleFrame(seed, false)
	cases := []map[string]any{}
	for _, op := range ops {
		prior := lifecycleSnapshot(seed)
		apis := lifecycleConsumers(seed, op)
		t.BeginBlock(&lifecycleBlock(period).BlockInfo)
		row := lifecycleExecute(t, op, nonces)
		t.EndBlock()
		t.Commit()
		row["state_before"] = prior
		row["state_after"] = lifecycleSnapshot(seed)
		row["consumers"] = apis
		row["period"] = period
		cases = append(cases, row)
		period++
	}
	return map[string]any{"name": fmt.Sprintf("cacti_%t/%s/%s/%s", cacti, queue, phase, topology), "cacti": cacti, "queue": queue, "phase": phase, "topology": topology, "configuration": cfg, "provenance": provenance, "state_before": before, "state_after": lifecycleSnapshot(seed), "execution_period": period, "cases": cases, "consumers": consumers, "frame": frame}
}
func main() {
	custodyDelegator = retainedIdentity(0x31)
	custodyOwner = retainedIdentity(0xa1)
	if inventory := os.Getenv("RETAINED_RAW_KEYS"); inventory != "" {
		var keys []string
		if err := json.Unmarshal([]byte(inventory), &keys); err != nil {
			panic(err)
		}
		for _, k := range keys {
			retainedKeys[common.HexToHash(k)] = true
		}
	}
	state_evm.SetNativeV1CustodyRawWriteObserver(func(a common.Address, k common.Hash, v []byte) {
		retainedKeys[k] = true
		retainedWrites = append(retainedWrites, custodyRawWrite{Address: hex.EncodeToString(a[:]), Key: hex.EncodeToString(k[:]), Value: hex.EncodeToString(v)})
	})
	state_evm.SetRewardBalanceObserver(func(a common.Address, op string, amount, balance *big.Int) {
		retainedBalances = append(retainedBalances, map[string]string{"address": hex.EncodeToString(a[:]), "operation": op, "amount": amount.String(), "balance": balance.String()})
	})
	scenarios := []map[string]any{}
	for _, cacti := range []bool{false, true} {
		for _, queue := range []string{"v1", "v2", "mixed", "mixed_v1_last"} {
			for _, phase := range []string{"pending", "partial", "claim_before_final", "final"} {
				topologies := []string{"singleton"}
				if phase == "final" {
					topologies = []string{"singleton", "first", "middle", "last"}
				}
				for _, topology := range topologies {
					scenarios = append(scenarios, lifecycleScenario(cacti, queue, phase, topology))
				}
			}
		}
	}
	keys := []string{}
	for k := range retainedKeys {
		keys = append(keys, hex.EncodeToString(k[:]))
	}
	sort.Strings(keys)
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "scenarios": scenarios, "raw_key_inventory": keys, "concrete_rows": lifecycleRows}); err != nil {
		panic(err)
	}
}
