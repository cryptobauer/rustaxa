// Actual pinned Go one-member full source removal/new destination frame observations.
package main

import (
	_ "embed"
	"encoding/hex"
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
	state      *state_evm.TransitionState
	calls      []map[string]any
	firstWrite int
	firstLog   int
}

func (o *destinationObserver) RequiredGas(ctx vm.CallFrame, e *vm.EVM) uint64 {
	o.called = false
	o.err = ""
	o.output = nil
	o.firstWrite = len(o.backend.writes)
	o.firstLog = len(o.state.GetLogs())
	quote := o.infoObserver.RequiredGas(ctx, e)
	o.calls = append(o.calls, map[string]any{"required_gas": quote, "supplied_native_gas": o.funding,
		"depth": o.depth, "caller": infoHex(o.caller[:]), "native_called": false,
		"native_error": "", "native_output": "", "setup_reads": append([]string{}, o.setupReads...), "ordered_reads": []string{}, "logs": []map[string]any{}, "ordered_raw_writes": []map[string]string{}})
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

type fullFrameRead struct {
	Key, Value string
	Present    bool
}

func main() {
	var seed struct {
		Cases []struct {
			Input    string
			SeedRaw  []fullFrameRead `json:"seed_raw"`
			Attempts []struct {
				OrderedReads []fullFrameRead `json:"ordered_reads"`
			}
		}
	}
	if err := json.Unmarshal(redelegateSeed, &seed); err != nil {
		panic(err)
	}
	sender, wrapper := common.BytesToAddress([]byte{0xaa}), common.BytesToAddress([]byte{0xd1})
	contract := *dpos.ContractAddress()
	definitions := []struct {
		name                                                string
		direct, static, revert, two                         bool
		gas                                                 uint64
		value, amount                                       int64
		length                                              int
		dirty, trailing, beforeFix, atFixMissing, aspenZero bool
		seedIndex                                           int
	}{
		{name: "direct_full_new", direct: true, gas: 80000, length: 96},
		{name: "nested_full_new", gas: 80000, length: 96},
		{name: "static_full_new", static: true, gas: 80000, length: 96},
		{name: "parent_revert_full_new", revert: true, gas: 80000, length: 96},
		{name: "two_full_new_calls_parent_revert", two: true, revert: true, gas: 80000, length: 96},
		{name: "nested_nonpayable", gas: 80000, value: 1, length: 96},
		{name: "nested_underfunded", gas: 79999, length: 96},
	}
	rows := []map[string]any{}
	for _, c := range definitions {
		code := redelegateWrapper(c.static, c.revert, c.gas)
		if c.two {
			first := redelegateWrapper(false, false, c.gas)
			first = append(first[:len(first)-8], 0x50) // POP exactly the CALL result.
			code = append(first, code...)
		}
		hash := crypto.Keccak256Hash(code)
		raw := map[common.Hash][]byte{}
		for _, read := range append(seed.Cases[c.seedIndex].Attempts[0].OrderedReads, seed.Cases[c.seedIndex].SeedRaw...) {
			if read.Present {
				raw[common.HexToHash(read.Key)] = common.Hex2Bytes(read.Value)
			}
		}
		raw[*storage.Stor_k_1([]byte{4})] = []byte{1, 0x2c}
		raw[*storage.Stor_k_1([]byte{5})] = []byte{0x0b, 0xb8}
		inputState := infoInput{accounts: map[common.Address]state_db.Account{
			sender:   {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)},
			wrapper:  {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &hash, CodeSize: uint64(len(code))},
			contract: {Nonce: big.NewInt(1), Balance: big.NewInt(3000)},
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
		if c.beforeFix {
			cfg.Hardforks.FixRedelegateBlockNum = 2
		}
		if c.atFixMissing {
			cfg.Hardforks.FixRedelegateBlockNum = 1
		}
		cfg.Hardforks.AspenHf.BlockNumPartOne = ^uint64(0)
		cfg.Hardforks.AspenHf.BlockNumPartTwo = ^uint64(0)
		if c.aspenZero {
			cfg.Hardforks.AspenHf.BlockNumPartTwo = 1
		}
		cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
		cfg.DPOS.BlocksPerYear = 1
		cfg.DPOS.MinimumDeposit = big.NewInt(100)
		cfg.DPOS.ValidatorMaximumStake = big.NewInt(1000000)
		cfg.DPOS.EligibilityBalanceThreshold = big.NewInt(100)
		cfg.DPOS.VoteEligibilityBalanceStep = big.NewInt(10)
		backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &state}, reads: []string{}, writes: []map[string]string{}}
		native := new(dpos.Contract).Init(cfg, backend, dpos.Reader{}, &e)
		observer := &destinationObserver{infoObserver: &infoObserver{PrecompiledContract: native, backend: backend}, state: &state, calls: []map[string]any{}}
		native.Register(func(address *common.Address, _ vm.PrecompiledContract) {
			e.RegisterPrecompiledContract(address, observer)
		})
		base, _ := hex.DecodeString(seed.Cases[c.seedIndex].Input)
		calldata := common.CopyBytes(base[:4+c.length])
		if c.amount != 0 {
			big.NewInt(c.amount).FillBytes(calldata[68:100])
		}
		if c.dirty {
			for i := 4; i < 16; i++ {
				calldata[i] = 0xff
			}
			for i := 36; i < 48; i++ {
				calldata[i] = 0xff
			}
		}
		if c.trailing {
			calldata = append(calldata, 0xff)
		}
		if c.atFixMissing {
			calldata[35] = 0x99
		}
		if c.aspenZero {
			for i := 68; i < 100; i++ {
				calldata[i] = 0
			}
		}
		to, from := wrapper, sender
		if c.direct {
			to = contract
			from = wrapper
		}
		result, err := e.Main(&vm.Transaction{From: from, To: &to, Nonce: big.NewInt(1), GasPrice: new(big.Int), Gas: 200000, Value: big.NewInt(c.value), Input: calldata})
		parentError := ""
		if err != nil {
			parentError = err.Error()
		}
		logs := []map[string]any{}
		for _, log := range result.Logs {
			topics := []string{}
			for _, topic := range log.Topics {
				topics = append(topics, infoHex(topic[:]))
			}
			logs = append(logs, map[string]any{"address": infoHex(log.Address[:]), "topics": topics, "data": infoHex(log.Data)})
		}
		accounts := map[string]any{}
		for _, address := range []common.Address{sender, wrapper, contract} {
			a := state.GetAccountConcrete(&address)
			accounts[infoHex(address[:])] = map[string]string{"nonce": a.GetNonce().String(), "balance": a.GetBalance().String()}
		}
		prior, after := map[string]string{}, map[string]string{}
		keys := map[common.Hash]bool{}
		for key, value := range raw {
			prior[infoHex(key[:])] = infoHex(value)
			keys[key] = true
		}
		for _, write := range backend.writes {
			keys[common.HexToHash(write["key"])] = true
		}
		for key := range keys {
			value := []byte{}
			state.GetAccountConcrete(&contract).GetRawState(&key, func(v []byte) { value = common.CopyBytes(v) })
			after[infoHex(key[:])] = infoHex(value)
		}
		executionReads := []string{}
		for _, call := range observer.calls {
			executionReads = append(executionReads, call["ordered_reads"].([]string)...)
		}
		sourceDelegationKey := storage.Stor_k_1([]byte{2, 0}, calldata[16:36], wrapper[:])
		rows = append(rows, map[string]any{"name": c.name, "source_delegation_key": infoHex(sourceDelegationKey[:]), "calls": observer.calls, "two_calls": c.two, "input": infoHex(calldata), "code": infoHex(code), "direct": c.direct, "static": c.static, "parent_revert": c.revert, "value": c.value, "fix": uint64(cfg.Hardforks.FixRedelegateBlockNum), "aspen_zero": c.aspenZero, "caller": infoHex(observer.caller[:]), "depth": observer.depth, "required_gas": observer.quote, "supplied_native_gas": observer.funding, "native_called": observer.called, "native_error": observer.err, "native_output": infoHex(observer.output), "setup_reads": observer.setupReads, "ordered_reads": executionReads, "ordered_raw_writes": backend.writes, "parent_error": parentError, "parent_output": infoHex(result.CodeRetval), "transaction_gas_used": result.GasUsed, "logs": logs, "accounts": accounts, "prior_raw": prior, "after_raw": after})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": rows, "scope": "synthetic touched-row one-member full source/new destination retained-validator frame seed; actual pinned Go; no complete physical state or production acceptance"}); err != nil {
		panic(err)
	}
}
