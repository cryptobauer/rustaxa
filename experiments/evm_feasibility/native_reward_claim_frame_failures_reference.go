// Actual finite claim failure frames over canonical reward-only P1. Disposable code overlay only.
package main

import (
	"bytes"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"math/big"
	"os"
	"reflect"
)

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
	state        *state_evm.BlockState
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
func frameAccounts(state *state_evm.BlockState) map[string]any {
	out := map[string]any{}
	for _, address := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, *dpos.ContractAddress()} {
		a := state.GetAccountConcrete(&address)
		out[infoHex(address[:])] = map[string]string{"nonce": a.GetNonce().String(), "balance": a.GetBalance().String()}
	}
	return out
}

// Dirty state is live here: no checkpoint/commit/cache clear is permitted.
func main() {
	state_evm.SetRewardBalanceObserver(func(a common.Address, operation string, amount, balance *big.Int) {
		rewardBalances = append(rewardBalances, map[string]string{"address": infoHex(a[:]), "operation": operation, "amount": amount.String(), "balance": balance.String()})
	})
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0x31}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationSender = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	x, y = secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0xa1}, 32))
	pub = append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationDelegator = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	before := currentSignedSnapshot(seed, api)
	control := os.Getenv("REWARD_FRAME_CONTROL") == "1"
	definitions := []struct {
		name                                        string
		commission, static, revert, low, nonpayable bool
	}{
		{name: "commission_low", commission: true, low: true}, {name: "commission_nonpayable", commission: true, nonpayable: true},
		{name: "delegator_low", low: true}, {name: "delegator_nonpayable", nonpayable: true},
	}
	execute := func(definition struct {
		name                                        string
		commission, static, revert, low, nonpayable bool
	}) map[string]any {
		wrapper, sender := nativeSimulationSender, nativeSimulationDelegator
		selector := "ef5cfb8c"
		nativeGas := uint64(40000)
		if definition.commission {
			wrapper, sender = nativeSimulationDelegator, nativeSimulationSender
			selector = "d0eebfe2"
			nativeGas = 20000
		}
		if definition.low {
			nativeGas--
		}
		value := int64(0)
		if definition.nonpayable {
			value = 1
		}
		input := common.FromHex(selector + "0000000000000000000000000000000000000000000000000000000000000031")
		code := redelegateWrapper(definition.static, definition.revert, nativeGas)
		// BlockState commits are no-ops. The underlying canonical database is immutable.
		state := state_evm.GetBlockState(seed.database, 1, 1)
		state.GetAccountConcrete(&wrapper).SetCode(code)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), seed.config.EVMChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: 2, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, Time: 1700000002, GasLimit: 1000000, Difficulty: new(big.Int)}}, seed.config.Hardforks.Rules(2))
		backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: state}, reads: []string{}, writes: []map[string]string{}}
		native := new(dpos.Contract)
		if control {
			native.Init(seed.config, &storage.EVMStateStorage{EVMStateFace: state}, dpos.Reader{}, &e)
		} else {
			native.Init(seed.config, backend, dpos.Reader{}, &e)
		}
		observer := &destinationObserver{infoObserver: &infoObserver{PrecompiledContract: native, backend: backend}, state: state, calls: []map[string]any{}, routeStatic: definition.static}
		native.Register(func(a *common.Address, actual vm.PrecompiledContract) {
			if control {
				e.RegisterPrecompiledContract(a, actual)
			} else {
				e.RegisterPrecompiledContract(a, observer)
			}
		})
		accountsBefore := frameAccounts(state)
		rewardBalances = []map[string]string{}
		result, err := e.Main(&vm.Transaction{From: sender, To: &wrapper, Nonce: new(big.Int), GasPrice: new(big.Int), Gas: 200000, Value: big.NewInt(value), Input: input})
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
		row := map[string]any{"name": definition.name, "commission": definition.commission, "static": definition.static, "parent_revert": definition.revert, "code": infoHex(code), "input": infoHex(input), "from": infoHex(sender[:]), "to": infoHex(wrapper[:]), "nonce": 0, "gas": 200000, "gas_price": "0", "value": value, "transaction_gas_used": result.GasUsed, "parent_output": infoHex(result.CodeRetval), "parent_error": parentError, "consensus_error": consensusError, "execution_error": executionError, "logs": frameLogs(result.Logs), "accounts_before": accountsBefore, "accounts": frameAccounts(state)}
		// Fixed key scope measured in the first dual-pin capture; same reads in controls.
		selected := map[common.Hash]bool{}
		for _, key := range []string{
			"18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c",
			"4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a",
			"5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b",
			"5d96f6f3efe766445e7b883b8d3187bb07f51e4ac6f5933e0f3ad34cc7176f23",
			"753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3",
			"9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e",
			"b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5",
			"cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1",
			"d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873",
			"dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795",
			"ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc",
			"f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393",
		} {
			selected[common.HexToHash(key)] = true
		}
		prior, after := map[string]string{}, map[string]string{}
		address := *dpos.ContractAddress()
		reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(1)}
		for key := range selected {
			value := []byte{}
			reader.GetAccountStorage(&address, &key, func(v []byte) { value = common.CopyBytes(v) })
			prior[infoHex(key[:])] = infoHex(value)
			value = []byte{}
			state.GetAccountConcrete(&address).GetRawState(&key, func(v []byte) { value = common.CopyBytes(v) })
			after[infoHex(key[:])] = infoHex(value)
		}
		row["prior_raw"] = prior
		row["after_raw"] = after
		if !control {
			for _, call := range observer.calls {
				for _, field := range []string{"setup_reads", "ordered_reads"} {
					for _, key := range call[field].([]string) {
						if !selected[common.HexToHash(key)] {
							panic("unmeasured native read")
						}
					}
				}
			}
			for _, write := range backend.writes {
				if !selected[common.HexToHash(write["key"])] {
					panic("unmeasured native write")
				}
			}
			row["calls"] = observer.calls
			row["ordered_raw_writes"] = backend.writes
			row["ordered_balance_effects"] = append([]map[string]string{}, rewardBalances...)
		}
		return row
	}
	rows := []map[string]any{}
	for _, definition := range definitions {
		row := execute(definition)
		if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("frame mutatedcanonicalP1")
		}
		repeated := execute(definition)
		if !reflect.DeepEqual(row, repeated) || !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
			panic("frame repetition/P1 mismatch")
		}
		rows = append(rows, row)
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": currentSignedSnapshot(seed, api), "period": 2, "cases": rows, "scope": "actual finite lowgas/value1 commission/delegator CALL frames on disposable BlockState over canonical92rowP1; wrapper code on existing nativecaller only; no committed code/state mutation, mixed publication order, root parity or production acceptance"}); err != nil {
		panic(err)
	}
}
