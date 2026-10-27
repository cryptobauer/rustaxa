// Synthetic retained-validator seed and actual pinned Go commission execution.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"reflect"
	"sort"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var retainedKeys = map[common.Hash]bool{}
var retainedWrites = []custodyRawWrite{}
var retainedBalances = []map[string]string{}

// Copy the actual child result before its enclosing EVM frame reverts.
type retainedObserver struct {
	vm.PrecompiledContract
	state       *state_evm.BlockState
	observation map[string]any
}

func (o *retainedObserver) Run(ctx vm.CallFrame, e *vm.EVM) ([]byte, error) {
	out, err := o.PrecompiledContract.Run(ctx, e)
	message := ""
	if err != nil {
		message = err.Error()
	}
	o.observation["error"] = message
	o.observation["output"] = hex.EncodeToString(out)
	o.observation["logs"] = retainedLogs(o.state.GetLogs())
	o.observation["caller"] = hex.EncodeToString(ctx.CallerAccount.Address().Bytes())
	return out, err
}
func retainedLogs(logs []vm.LogRecord) []nativeSimulationLog {
	out := []nativeSimulationLog{}
	for _, l := range logs {
		topics := []string{}
		for _, t := range l.Topics {
			topics = append(topics, hex.EncodeToString(t[:]))
		}
		out = append(out, nativeSimulationLog{Address: hex.EncodeToString(l.Address[:]), Topics: topics, Data: hex.EncodeToString(l.Data)})
	}
	return out
}
func retainedFrame(seed nativeSimulationSeed, wide bool) map[string]any {
	// Explicit disposable code overlay, never deployed or committed.
	code := common.FromHex("366000600037600060003660003460fe62004e20f160005260206000fd")
	state := state_evm.GetBlockState(seed.database, 2, 2)
	if wide {
		code[len(code)-1] = 0xf3
		state.GetAccountConcrete(dpos.ContractAddress()).AddBalance(new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 600), state.GetAccountConcrete(dpos.ContractAddress()).GetBalance()))
		state.GetAccountConcrete(&custodyOwner).SubBalance(new(big.Int).Add(big.NewInt(17), state.GetAccountConcrete(&custodyOwner).GetBalance()))
	}
	state.GetAccountConcrete(&custodyOwner).SetCode(code)
	var e vm.EVM
	e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), seed.config.EVMChainConfig, vm.Config{})
	e.SetBlock(&vm.Block{Number: 3, BlockInfo: vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000003, Difficulty: new(big.Int)}}, seed.config.Hardforks.Rules(3))
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
		for _, a := range []common.Address{custodyOwner, custodyDelegator, *dpos.ContractAddress()} {
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
	result, err := e.Main(&vm.Transaction{From: custodyDelegator, To: &custodyOwner, Nonce: big.NewInt(1), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: custodyInput("claimCommissionRewards(address)", nil)})
	message := ""
	if err != nil {
		message = err.Error()
	}
	return map[string]any{"code_overlay": hex.EncodeToString(code), "input": hex.EncodeToString(custodyInput("claimCommissionRewards(address)", nil)), "sender": hex.EncodeToString(custodyDelegator[:]), "receiver": hex.EncodeToString(custodyOwner[:]), "gas_used": result.GasUsed, "parent_error": message, "output": hex.EncodeToString(result.CodeRetval), "logs": retainedLogs(result.Logs), "accounts_before": priorAccounts, "accounts_after": accounts(), "raw_before": priorRaw, "raw_after": raw(), "ordered_raw_writes": retainedWrites, "ordered_balance_effects": retainedBalances, "native_observation": observation}
}

func retainedIdentity(key byte) common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{key}, 32))
	pub := append(x.FillBytes(make([]byte, 32)), y.FillBytes(make([]byte, 32))...)
	return common.BytesToAddress(crypto.Keccak256(pub)[12:])
}

func retainedSnapshot(seed nativeSimulationSeed) map[string]any {
	period := seed.database.descriptor.BlockNum
	reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	a := *dpos.ContractAddress()
	wrapper := new(storage.StorageWrapper)
	wrapper.StorageReaderWrapper.Init(&a, reader)
	validators := new(dpos.Validators)
	validators.Init(wrapper, []byte{0})
	v := validators.GetValidator(&custodyValidator)
	if v == nil {
		panic("retained validator missing")
	}
	pool := validators.GetValidatorRewards(&custodyValidator)
	queue := new(dpos.Undelegations)
	queue.Init(wrapper, []byte{3})
	pending := queue.GetUndelegationV1(&custodyDelegator, &custodyValidator)
	queueCount := queue.GetUndelegationsV1Count(&custodyDelegator)
	raw := map[string]string{}
	for k := range retainedKeys {
		value := []byte{}
		reader.GetAccountStorage(&a, &k, func(b []byte) { value = common.CopyBytes(b) })
		if len(value) > 0 {
			raw[hex.EncodeToString(k[:])] = hex.EncodeToString(value)
		}
	}
	accounts := []map[string]any{}
	for _, address := range []common.Address{custodyDelegator, custodyOwner, a} {
		row := map[string]any{"address": hex.EncodeToString(address[:]), "exists": false}
		reader.GetRawAccount(&address, func(b []byte) {
			acc := state_db.DecodeAccountFromTrie(b)
			row["exists"] = true
			row["encoded"] = hex.EncodeToString(b)
			row["nonce"] = acc.Nonce.String()
			row["balance"] = acc.Balance.String()
		})
		accounts = append(accounts, row)
	}
	return map[string]any{"period": uint64(period), "root": hex.EncodeToString(seed.database.descriptor.StateRoot[:]), "seed_rows": nativeSimulationSeedRows(seed.database), "accounts": accounts, "native_raw": raw, "validator": v, "stake": v.TotalStake.String(), "pending_count": v.UndelegationsCount, "pending": pending, "queue_count": queueCount, "commission": pool.CommissionRewardsPool.String(), "rewards": pool.RewardsPool.String()}
}

func main() {
	if inventory := os.Getenv("RETAINED_RAW_KEYS"); inventory != "" {
		var keys []string
		if err := json.Unmarshal([]byte(inventory), &keys); err != nil {
			panic(err)
		}
		for _, k := range keys {
			retainedKeys[common.HexToHash(k)] = true
		}
	}
	custodyDelegator = retainedIdentity(0x31)
	custodyOwner = retainedIdentity(0xa1)
	nativeSimulationSender = custodyDelegator
	nativeSimulationDelegator = custodyOwner
	state_evm.SetNativeV1CustodyRawWriteObserver(func(a common.Address, k common.Hash, v []byte) {
		retainedKeys[k] = true
		retainedWrites = append(retainedWrites, custodyRawWrite{Address: hex.EncodeToString(a[:]), Key: hex.EncodeToString(k[:]), Value: hex.EncodeToString(v)})
	})
	state_evm.SetRewardBalanceObserver(func(a common.Address, op string, amount, balance *big.Int) {
		retainedBalances = append(retainedBalances, map[string]string{"address": hex.EncodeToString(a[:]), "operation": op, "amount": amount.String(), "balance": balance.String()})
	})
	cfg := cancelRewardConfig()
	cfg.Hardforks.PhalaenopsisHfBlockNum = 0
	db := newNativeSimulationDB()
	transition := nativeSimulationTransition(db, &cfg)
	defer transition.Close()
	seed := nativeSimulationSeed{database: db, config: cfg, wrapper: custodyOwner}
	provenance := []map[string]any{{"operation": "genesis", "state": retainedSnapshot(seed)}}
	transition.BeginBlock(&vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000001, Difficulty: new(big.Int)})
	stats := rewards_stats.RewardsStats{BlockAuthor: custodyValidator, BlocksPerYear: 1, ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{custodyValidator: {VoteWeight: 1, FeesRewards: new(big.Int)}}, TotalVotesWeight: 1, MaxVotesWeight: 1}
	minted := transition.DistributeRewards(&stats)
	if minted.String() != "200" {
		panic("mint changed")
	}
	transition.EndBlock()
	transition.Commit()
	provenance = append(provenance, map[string]any{"operation": "P1 reward", "minted": minted.String(), "state": retainedSnapshot(seed)})
	transition.BeginBlock(&vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000002, Difficulty: new(big.Int)})
	a := *dpos.ContractAddress()
	input := custodyInput("undelegate(address,uint256)", big.NewInt(1000))
	setup := transition.ExecuteTransaction(&vm.Transaction{From: custodyDelegator, To: &a, Nonce: new(big.Int), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: input})
	if setup.ConsensusErr != "" || setup.ExecutionErr != "" {
		panic("full undelegation failed")
	}
	transition.EndBlock()
	transition.Commit()
	before := retainedSnapshot(seed)
	if before["stake"] != "0" || before["pending_count"] != uint16(1) || before["commission"] != "2" {
		panic("seed not qualified")
	}
	provenance = append(provenance, map[string]any{"operation": "P2 full V1 undelegation", "sender": hex.EncodeToString(custodyDelegator[:]), "input": hex.EncodeToString(input), "nonce": "0", "gas_used": setup.GasUsed, "state": before})
	frame := retainedFrame(seed, false)
	wideEffects := retainedFrame(seed, true)
	if !reflect.DeepEqual(before, retainedSnapshot(seed)) {
		panic("frame mutated committed P2")
	}
	api := new(dpos.API).Init(cfg)
	factory := func(p types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: db.GetBlockStateReader(p)}
	}
	runner := new(state_dry_runner.DryRunner).Init(db, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &cfg)
	block := &vm.Block{Number: 2, BlockInfo: vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000002, Difficulty: new(big.Int)}}
	simulation := []nativeSimulationCase{}
	for _, c := range []struct {
		name string
		from common.Address
	}{{"positive", custodyOwner}, {"wrong_owner", custodyDelegator}} {
		tx := vm.Transaction{From: c.from, To: &a, Nonce: new(big.Int).Lsh(big.NewInt(1), 512), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: custodyInput("claimCommissionRewards(address)", nil)}
		first := runNativeSimulationCaptured(runner, block, c.name, tx)
		tx.Nonce = new(big.Int).Lsh(big.NewInt(1), 512)
		second := runNativeSimulationCaptured(runner, block, c.name, tx)
		if !reflect.DeepEqual(first, second) || !reflect.DeepEqual(before, retainedSnapshot(seed)) {
			panic("simulation leaked or differed")
		}
		simulation = append(simulation, first)
	}
	transition.BeginBlock(&vm.BlockInfo{Author: custodyValidator, GasLimit: 1000000, Time: 1700000003, Difficulty: new(big.Int)})
	cases := []map[string]any{}
	for _, c := range []struct {
		name  string
		from  common.Address
		nonce int64
	}{{"wrong_owner", custodyDelegator, 1}, {"positive", custodyOwner, 0}, {"zero_repeat", custodyOwner, 1}} {
		retainedWrites = []custodyRawWrite{}
		retainedBalances = []map[string]string{}
		tx := vm.Transaction{From: c.from, To: &a, Nonce: big.NewInt(c.nonce), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: custodyInput("claimCommissionRewards(address)", nil)}
		result := transition.ExecuteTransaction(&tx)
		logs := []nativeSimulationLog{}
		for _, l := range result.Logs {
			topics := []string{}
			for _, t := range l.Topics {
				topics = append(topics, hex.EncodeToString(t[:]))
			}
			logs = append(logs, nativeSimulationLog{Address: hex.EncodeToString(l.Address[:]), Topics: topics, Data: hex.EncodeToString(l.Data)})
		}
		cases = append(cases, map[string]any{"name": c.name, "sender": hex.EncodeToString(c.from[:]), "nonce": c.nonce, "input": hex.EncodeToString(tx.Input), "gas_used": result.GasUsed, "consensus_error": result.ConsensusErr, "execution_error": result.ExecutionErr, "output": hex.EncodeToString(result.CodeRetval), "logs": logs, "ordered_raw_writes": retainedWrites, "ordered_balance_effects": retainedBalances})
	}
	transition.EndBlock()
	transition.Commit()
	after := retainedSnapshot(seed)
	keys := []string{}
	for k := range retainedKeys {
		keys = append(keys, hex.EncodeToString(k[:]))
	}
	sort.Strings(keys)
	document := map[string]any{"schema": 1, "configuration": cfg, "delegator": hex.EncodeToString(custodyDelegator[:]), "owner": hex.EncodeToString(custodyOwner[:]), "key_provenance": "secp256k1 repeated32 keys31/A1; genesis principal1000 delegated by31; P1 mint200; P2 full V1 undelegation1000 unlock5; no synthetic pending rows", "provenance": provenance, "state_before": before, "state_after": after, "cases": cases, "simulation": simulation, "frame": frame, "wide_effects": wideEffects, "raw_key_inventory": keys, "scope": "synthetic retained-validator profile only; no N4 or producer identity"}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}
