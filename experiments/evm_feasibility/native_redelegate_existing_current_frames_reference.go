// Actual pinned Go two-transaction existing-destination current-node full removal frame observations.
package main

import (
	_ "embed"
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
	state       *state_evm.TransitionState
	calls       []map[string]any
	firstWrite  int
	firstLog    int
	routeStatic bool
}

func (o *destinationObserver) RequiredGas(ctx vm.CallFrame, e *vm.EVM) uint64 {
	o.called = false
	o.err = ""
	o.output = nil
	o.firstWrite = len(o.backend.writes)
	o.firstLog = len(o.state.GetLogs())
	quote := o.infoObserver.RequiredGas(ctx, e)
	o.calls = append(o.calls, map[string]any{"required_gas": quote, "supplied_native_gas": o.funding,
		"depth": o.depth, "caller": infoHex(o.caller[:]), "input": infoHex(ctx.Input), "value": ctx.Value.String(), "native_called": false,
		"native_error": "", "native_output": "", "route_staticcall": o.routeStatic, "setup_reads": append([]string{}, o.setupReads...), "ordered_reads": []string{}, "logs": []map[string]any{}, "ordered_raw_writes": []map[string]string{}})
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
	for _, last := range []byte{0xaa, 0xd1, 0xfe} {
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
func main() {
	var seed struct {
		Cases []struct {
			Attempts []struct {
				Input     string
				RawBefore map[string]struct {
					Present bool
					Value   string
				} `json:"raw_before"`
			}
		}
	}
	if err := json.Unmarshal(redelegateSeed, &seed); err != nil {
		panic(err)
	}
	prefixInput := common.Hex2Bytes(seed.Cases[0].Attempts[0].Input)
	targetInput := common.Hex2Bytes(seed.Cases[0].Attempts[1].Input)
	sender, wrapper := common.BytesToAddress([]byte{0xaa}), common.BytesToAddress([]byte{0xd1})
	contract := *dpos.ContractAddress()
	control := os.Getenv("EXISTING_CURRENT_FRAME_CONTROL") == "1"
	definitions := []struct {
		name                        string
		direct, static, revert, two bool
		gas                         uint64
		value                       int64
	}{
		{name: "direct_existing_current", direct: true, gas: 80000}, {name: "nested_existing_current", gas: 80000},
		{name: "static_existing_current", static: true, gas: 80000}, {name: "parent_revert_existing_current", revert: true, gas: 80000},
		{name: "two_existing_current_calls_parent_revert", two: true, revert: true, gas: 80000},
		{name: "nested_nonpayable", gas: 80000, value: 1}, {name: "nested_underfunded", gas: 79999},
	}
	rows := []map[string]any{}
	for _, c := range definitions {
		code := redelegateWrapper(c.static, c.revert, c.gas)
		if c.two {
			first := redelegateWrapper(false, false, c.gas)
			first = append(first[:len(first)-8], 0x50)
			code = append(first, code...)
		}
		hash := crypto.Keccak256Hash(code)
		raw := map[common.Hash][]byte{}
		selected := map[common.Hash]bool{}
		for key, read := range seed.Cases[0].Attempts[0].RawBefore {
			k := common.HexToHash(key)
			selected[k] = true
			if read.Present {
				raw[k] = common.Hex2Bytes(read.Value)
			}
		}
		for field, value := range map[byte][]byte{4: {1, 0x90}, 5: {0x0f, 0xa0}} {
			k := *storage.Stor_k_1([]byte{field})
			if old, ok := raw[k]; ok && !reflect.DeepEqual(old, value) {
				panic("seed global conflict")
			}
			raw[k] = common.CopyBytes(value)
			selected[k] = true
		}
		inputState := infoInput{accounts: map[common.Address]state_db.Account{
			sender: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)}, wrapper: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &hash, CodeSize: uint64(len(code))}, contract: {Nonce: big.NewInt(1), Balance: big.NewInt(4000)},
		}, codes: map[common.Hash][]byte{hash: code}, raw: raw}
		var state state_evm.TransitionState
		state.Init(state_evm.Opts{})
		state.SetInput(inputState)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: 1, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: new(big.Int)}}, vm.Rules{IsMagnolia: true, IsCornus: true})
		cfg := chain_config.ChainConfig{}
		cfg.Hardforks.FixRedelegateBlockNum = 0
		cfg.Hardforks.CornusHf.BlockNum = 0
		cfg.Hardforks.MagnoliaHf.BlockNum = 0
		cfg.Hardforks.FicusHf.BlockNum = 0
		cfg.Hardforks.AspenHf.BlockNumPartOne = ^uint64(0)
		cfg.Hardforks.AspenHf.BlockNumPartTwo = ^uint64(0)
		cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
		cfg.DPOS.BlocksPerYear = 1
		cfg.DPOS.MinimumDeposit = big.NewInt(100)
		cfg.DPOS.ValidatorMaximumStake = big.NewInt(1000000)
		cfg.DPOS.EligibilityBalanceThreshold = big.NewInt(100)
		cfg.DPOS.VoteEligibilityBalanceStep = big.NewInt(10)
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
			}
			return row
		}
		observer.routeStatic = false // Direct prefix route; not native read_only observation.
		prefix := execute(wrapper, contract, 1, 0, prefixInput)
		if prefix["parent_error"] != "" || prefix["consensus_error"] != "" || prefix["execution_error"] != "" || len(prefix["logs"].([]map[string]any)) != 1 {
			panic("prefix failed")
		}
		to, from, nonce := wrapper, sender, int64(1)
		if c.direct {
			to = contract
			from = wrapper
			nonce = 2
		}
		observer.routeStatic = c.static // Wrapper opcode route; Go dispatch has no native read_only flag.
		target := execute(from, to, nonce, c.value, targetInput)
		if !reflect.DeepEqual(prefix["after_raw"], target["prior_raw"]) || !reflect.DeepEqual(prefix["accounts"], target["accounts_before"]) {
			panic("transaction boundary changed")
		}
		target["name"] = c.name
		target["prefix"] = prefix
		target["code"] = infoHex(code)
		target["direct"] = c.direct
		target["static"] = c.static
		target["parent_revert"] = c.revert
		target["two_calls"] = c.two
		target["fix"] = uint64(0)
		target["aspen_zero"] = false
		sourceKey := storage.Stor_k_1([]byte{2, 0}, targetInput[16:36], wrapper[:])
		target["source_delegation_key"] = infoHex(sourceKey[:])
		target["source_current_key"] = "b43e743da4ade83344c4edf921144d283b142466c18d45ca4840151c70461fa9"
		if target["prior_raw"].(map[string]string)[target["source_current_key"].(string)] != "c28002" {
			panic("prefix source node absent")
		}
		rows = append(rows, target)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": rows, "scope": "actual pinned Go two-Main existing-destination current-node frames; initial frozen42-row seed; live dirty raw, cumulative log and transaction suffix authority; no complete physical checkpoint or production acceptance"}); err != nil {
		panic(err)
	}
}
