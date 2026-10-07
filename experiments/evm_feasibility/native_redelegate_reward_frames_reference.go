// Actual pinned Go positive-reward partial redelegation frame observations.
package main

import (
	_ "embed"
	"encoding/binary"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"
)

//go:embed seed.json
var redelegateSeed []byte

func redelegateWrapper(static, revert bool, gas uint64) []byte {
	code := []byte{0x36, 0x60, 0, 0x60, 0, 0x37, 0x60, 0, 0x60, 0, 0x36, 0x60, 0}
	if !static {
		code = append(code, 0x34)
	}
	code = append(code, 0x60, 0xfe, 0x62, byte(gas>>16), byte(gas>>8), byte(gas))
	if static {
		code = append(code, 0xfa)
	} else {
		code = append(code, 0xf1)
	}
	code = append(code, 0x60, 0, 0x52, 0x60, 32, 0x60, 0)
	if revert {
		return append(code, 0xfd)
	}
	return append(code, 0xf3)
}

// Copy-only per-call observations, retained before enclosing frame rollback.
type destinationObserver struct {
	*infoObserver
	state        *state_evm.TransitionState
	calls        []map[string]any
	firstWrite   int
	firstLog     int
	firstBalance int
	routeStatic  bool
}

func (o *destinationObserver) RequiredGas(ctx vm.CallFrame, e *vm.EVM) uint64 {
	o.called = false
	o.err = ""
	o.output = nil
	o.firstWrite = len(o.backend.writes)
	o.firstBalance = len(rewardBalances)
	o.firstLog = len(o.state.GetLogs())
	quote := o.infoObserver.RequiredGas(ctx, e)
	o.calls = append(o.calls, map[string]any{"required_gas": quote, "supplied_native_gas": o.funding,
		"depth": o.depth, "caller": infoHex(o.caller[:]), "input": infoHex(ctx.Input), "value": ctx.Value.String(), "native_called": false,
		"native_error": "", "native_output": "", "route_staticcall": o.routeStatic, "setup_reads": append([]string{}, o.setupReads...), "ordered_reads": []string{}, "logs": []map[string]any{}, "ordered_raw_writes": []map[string]string{}, "ordered_balance_effects": []map[string]string{}})
	return quote
}
func (o *destinationObserver) Run(ctx vm.CallFrame, e *vm.EVM) ([]byte, error) {
	out, err := o.infoObserver.Run(ctx, e)
	row := o.calls[len(o.calls)-1]
	row["native_called"] = true
	row["native_error"] = o.err
	row["native_output"] = infoHex(out)
	writes := []map[string]string{}
	for _, original := range o.backend.writes[o.firstWrite:] {
		copy := map[string]string{}
		for key, value := range original {
			copy[key] = value
		}
		writes = append(writes, copy)
	}
	logs := []map[string]any{}
	for _, log := range o.state.GetLogs()[o.firstLog:] {
		topics := []string{}
		for _, topic := range log.Topics {
			topics = append(topics, infoHex(topic[:]))
		}
		logs = append(logs, map[string]any{"address": infoHex(log.Address[:]), "topics": topics, "data": infoHex(log.Data)})
	}
	row["ordered_reads"] = append([]string{}, o.backend.reads...)
	o.backend.reads = []string{} // Isolate next RequiredGas setup from this Run.
	row["ordered_raw_writes"] = writes
	row["logs"] = logs
	row["ordered_balance_effects"] = append([]map[string]string{}, rewardBalances[o.firstBalance:]...)
	return out, err
}

// Copy-only log vectors; EVM.Main retains cumulative state logs between calls.
func frameLogs(logs []vm.LogRecord) []map[string]any {
	out := []map[string]any{}
	for _, log := range logs {
		topics := []string{}
		for _, topic := range log.Topics {
			topics = append(topics, infoHex(topic[:]))
		}
		out = append(out, map[string]any{"address": infoHex(log.Address[:]), "topics": topics, "data": infoHex(log.Data)})
	}
	return out
}
func frameAccounts(state *state_evm.TransitionState) map[string]any {
	out := map[string]any{}
	for _, last := range []byte{0xd1, 0xa1, 0xfe} {
		address := common.BytesToAddress([]byte{last})
		a := state.GetAccountConcrete(&address)
		out[infoHex(address[:])] = map[string]string{"nonce": a.GetNonce().String(), "balance": a.GetBalance().String()}
	}
	return out
}

// Dirty state is live here: no checkpoint/commit/cache clear is permitted.
func frameRaw(state *state_evm.TransitionState, selected map[common.Hash]bool) map[string]string {
	address := *dpos.ContractAddress()
	account := state.GetAccountConcrete(&address)
	keys := map[common.Hash]bool{}
	for key := range selected {
		keys[key] = true
	}
	for key := range account.RawStorageDirty {
		keys[key] = true
	}
	out := map[string]string{}
	for key := range keys {
		value := []byte{}
		account.GetRawState(&key, func(v []byte) { value = common.CopyBytes(v) })
		out[infoHex(key[:])] = infoHex(value)
	}
	return out
}

var rewardBalances = []map[string]string{}

func main() {
	state_evm.SetRewardBalanceObserver(func(address common.Address, operation string, amount, balance *big.Int) {
		rewardBalances = append(rewardBalances, map[string]string{"address": infoHex(address[:]), "operation": operation, "amount": amount.String(), "balance": balance.String()})
	})
	var seed struct {
		Cases []struct {
			Input   string
			SeedRaw []struct {
				Key     string
				Present bool
				Value   string
			} `json:"seed_raw"`
			Rows []struct {
				Column int
				Key    string
				Value  string
			} `json:"reward_committed_rows"`
			Accounts map[string]struct {
				Nonce   string
				Balance string
			} `json:"reward_committed_accounts"`
			Configuration struct {
				ChainConfig chain_config.ChainConfig `json:"chain_config"`
			}
		}
	}
	if err := json.Unmarshal(redelegateSeed, &seed); err != nil {
		panic(err)
	}
	targetInput := common.Hex2Bytes(seed.Cases[0].Input)
	wrapper := common.BytesToAddress([]byte{0xd1})
	sender := common.BytesToAddress([]byte{0xa1})
	contract := *dpos.ContractAddress()
	control := os.Getenv("REWARD_FRAME_CONTROL") == "1"
	definitions := []struct {
		name                        string
		direct, static, revert, two bool
		gas                         uint64
		value                       int64
	}{
		{name: "direct_rewarded_partial", direct: true, gas: 80000}, {name: "nested_rewarded_partial", gas: 80000},
		{name: "static_rewarded_partial", static: true, gas: 80000}, {name: "parent_revert_rewarded_partial", revert: true, gas: 80000},
		{name: "two_rewarded_partial_calls_parent_revert", two: true, revert: true, gas: 80000},
		{name: "nested_nonpayable", gas: 80000, value: 1}, {name: "nested_underfunded", gas: 79999},
	}
	rows := []map[string]any{}
	for _, c := range definitions {
		code := redelegateWrapper(c.static, c.revert, c.gas)
		if c.two {
			first := redelegateWrapper(false, false, c.gas)
			first = append(first[:len(first)-8], 0x50)
			second := code
			// Fresh calldata copy, then replace amount word at byte68 with701.
			second = append(append(append([]byte{}, second[:6]...), 0x61, 0x02, 0xbd, 0x60, 0x44, 0x52), second[6:]...)
			code = append(first, second...)
		}
		hash := crypto.Keccak256Hash(code)
		raw := map[common.Hash][]byte{}
		selected := map[common.Hash]bool{}
		// Resolve logical keys forward from complete actual committedP1 rows.
		resolve := func(k common.Hash) []byte {
			prefix := crypto.Keccak256Hash(contract[:], crypto.Keccak256(k[:]))
			var result []byte
			var latest uint64
			found := false
			for _, row := range seed.Cases[0].Rows {
				key := common.Hex2Bytes(row.Key)
				if row.Column != 4 || len(key) != 40 || !reflect.DeepEqual(key[:32], prefix[:]) {
					continue
				}
				period := binary.BigEndian.Uint64(key[32:])
				if period <= 1 && (!found || period > latest) {
					found = true
					latest = period
					result = common.Hex2Bytes(row.Value)
				}
			}
			return result
		}
		for _, read := range seed.Cases[0].SeedRaw {
			k := common.HexToHash(read.Key)
			selected[k] = true
			value := resolve(k)
			if (len(value) > 0) != read.Present || infoHex(value) != read.Value {
				panic("selected seed differs from physicalP1")
			}
			if len(value) > 0 {
				raw[k] = common.CopyBytes(value)
			}
		}
		for _, field := range []byte{4, 5, 6, 7} {
			k := *storage.Stor_k_1([]byte{field})
			value := resolve(k)
			if field <= 5 && len(value) == 0 {
				panic("missing physical global")
			}
			if len(value) > 0 {
				raw[k] = common.CopyBytes(value)
			}
			selected[k] = true
		}
		inputState := infoInput{accounts: map[common.Address]state_db.Account{}, codes: map[common.Hash][]byte{hash: code}, raw: raw}
		for key, fact := range seed.Cases[0].Accounts {
			nonce, ok := new(big.Int).SetString(fact.Nonce, 10)
			if !ok {
				panic("nonce")
			}
			balance, ok := new(big.Int).SetString(fact.Balance, 10)
			if !ok {
				panic("balance")
			}
			inputState.accounts[common.HexToAddress(key)] = state_db.Account{Nonce: nonce, Balance: balance}
		}
		// Only code fields on the existing caller change for this frame experiment.
		a := inputState.accounts[wrapper]
		a.CodeHash = &hash
		a.CodeSize = uint64(len(code))
		inputState.accounts[wrapper] = a
		var state state_evm.TransitionState
		state.Init(state_evm.Opts{})
		state.SetInput(inputState)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: 2, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: new(big.Int)}}, vm.Rules{IsMagnolia: true, IsCornus: true})
		cfg := seed.Cases[0].Configuration.ChainConfig
		backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &state}, reads: []string{}, writes: []map[string]string{}}
		native := new(dpos.Contract)
		if control {
			native.Init(cfg, &storage.EVMStateStorage{EVMStateFace: &state}, dpos.Reader{}, &e)
		} else {
			native.Init(cfg, backend, dpos.Reader{}, &e)
		}
		observer := &destinationObserver{infoObserver: &infoObserver{PrecompiledContract: native, backend: backend}, state: &state, calls: []map[string]any{}}
		native.Register(func(address *common.Address, actual vm.PrecompiledContract) {
			if control {
				e.RegisterPrecompiledContract(address, actual)
			} else {
				e.RegisterPrecompiledContract(address, observer)
			}
		})
		execute := func(from, to common.Address, nonce, value int64, input []byte) map[string]any {
			beforeRaw := frameRaw(&state, selected)
			beforeAccounts := frameAccounts(&state)
			beforeLogs := frameLogs(state.GetLogs())
			beforeRefund := state.GetRefund()
			backend.reads = []string{}
			backend.writes = []map[string]string{}
			observer.calls = []map[string]any{}
			rewardBalances = []map[string]string{}
			result, err := e.Main(&vm.Transaction{From: from, To: &to, Nonce: big.NewInt(nonce), GasPrice: new(big.Int), Gas: 200000, Value: big.NewInt(value), Input: common.CopyBytes(input)})
			parentError := ""
			if err != nil {
				parentError = err.Error()
			}
			consensusError := ""
			if result.ConsensusErr != "" {
				consensusError = result.ConsensusErr.Error()
			}
			executionError := ""
			if result.ExecutionErr != "" {
				executionError = result.ExecutionErr.Error()
			}
			cumulative := frameLogs(result.Logs)
			live := frameLogs(state.GetLogs())
			if !reflect.DeepEqual(cumulative, live) || len(cumulative) < len(beforeLogs) || !reflect.DeepEqual(cumulative[:len(beforeLogs)], beforeLogs) {
				panic("cumulative log prefix changed")
			}
			if beforeRefund != 0 || state.GetRefund() != 0 {
				panic("nonzero cumulative refund")
			}
			localLogs := append([]map[string]any{}, cumulative[len(beforeLogs):]...)
			row := map[string]any{"input": infoHex(input), "from": infoHex(from[:]), "to": infoHex(to[:]), "nonce": nonce, "value": value, "gas": 200000, "gas_price": "0", "log_count_before": len(beforeLogs), "refund_before": beforeRefund, "refund_after": state.GetRefund(), "cumulative_logs": cumulative, "logs": localLogs, "parent_error": parentError, "consensus_error": consensusError, "execution_error": executionError, "parent_output": infoHex(result.CodeRetval), "transaction_gas_used": result.GasUsed, "accounts_before": beforeAccounts, "accounts": frameAccounts(&state), "prior_raw": beforeRaw, "after_raw": frameRaw(&state, selected)}
			if !control {
				reads := []string{}
				for _, call := range observer.calls {
					reads = append(reads, call["ordered_reads"].([]string)...)
				}
				row["calls"] = append([]map[string]any{}, observer.calls...)
				row["ordered_raw_writes"] = append([]map[string]string{}, backend.writes...)
				row["ordered_reads"] = reads
				row["ordered_balance_effects"] = append([]map[string]string{}, rewardBalances...)
				nativeBalances := []map[string]string{}
				for _, call := range observer.calls {
					if effects, ok := call["ordered_balance_effects"].([]map[string]string); ok {
						nativeBalances = append(nativeBalances, effects...)
					}
				}
				row["native_ordered_balance_effects"] = nativeBalances
			}
			return row
		}
		to, from, nonce := wrapper, sender, int64(0)
		if c.direct {
			to = contract
			from = wrapper
		}
		observer.routeStatic = c.static // Wrapper opcode route; Go dispatch has no native read_only flag.
		target := execute(from, to, nonce, c.value, targetInput)
		target["name"] = c.name
		target["code"] = infoHex(code)
		target["direct"] = c.direct
		target["static"] = c.static
		target["parent_revert"] = c.revert
		target["two_calls"] = c.two
		target["fix"] = uint64(0)
		target["aspen_zero"] = false
		sourceKey := storage.Stor_k_1([]byte{2, 0}, targetInput[16:36], wrapper[:])
		target["source_delegation_key"] = infoHex(sourceKey[:])
		rows = append(rows, target)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": rows, "scope": "actual pinned Go positive-reward partial frame experiment; actual committedP1 physical rows and account facts with caller code overlay only; no public lifecycle or production acceptance"}); err != nil {
		panic(err)
	}
}
