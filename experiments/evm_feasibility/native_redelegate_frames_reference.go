// Actual pinned Go redelegation ABI/admission and EVM frame observations.
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

func main() {
	var seed struct {
		Cases []struct {
			Input    string
			Attempts []struct {
				OrderedReads []struct {
					Key, Value string
					Present    bool
				} `json:"ordered_reads"`
			}
		}
	}
	if err := json.Unmarshal(redelegateSeed, &seed); err != nil {
		panic(err)
	}
	base, _ := hex.DecodeString(seed.Cases[0].Input)
	sender, wrapper := common.BytesToAddress([]byte{0xaa}), common.BytesToAddress([]byte{0xd1})
	contract := *dpos.ContractAddress()
	definitions := []struct {
		name                                                string
		direct, static, revert                              bool
		gas                                                 uint64
		value                                               int64
		length                                              int
		dirty, trailing, beforeFix, atFixMissing, aspenZero bool
	}{
		{name: "direct_partial", direct: true, gas: 80000, length: 96},
		{name: "nested_partial", gas: 80000, length: 96},
		{name: "static_partial", static: true, gas: 80000, length: 96},
		{name: "parent_revert", revert: true, gas: 80000, length: 96},
		{name: "nested_underfunded", gas: 79999, length: 96},
		{name: "direct_nonpayable", direct: true, gas: 80000, value: 1, length: 96},
		{name: "nested_nonpayable", gas: 80000, value: 1, length: 96},
		{name: "dirty_addresses", direct: true, gas: 80000, length: 96, dirty: true},
		{name: "trailing", direct: true, gas: 80000, length: 96, trailing: true},
		{name: "short_0", direct: true, gas: 80000, length: 0},
		{name: "short_31", direct: true, gas: 80000, length: 31},
		{name: "short_32", direct: true, gas: 80000, length: 32},
		{name: "short_63", direct: true, gas: 80000, length: 63},
		{name: "short_64", direct: true, gas: 80000, length: 64},
		{name: "short_95", direct: true, gas: 80000, length: 95},
		{name: "malformed_underfunded", gas: 79999, length: 0},
		{name: "malformed_nonpayable", gas: 80000, value: 1, length: 0},
		{name: "nested_before_fix_malformed", gas: 80000, length: 0, beforeFix: true},
		{name: "nested_before_fix_nonpayable", gas: 80000, value: 1, length: 0, beforeFix: true},
		{name: "at_fix_missing_source", gas: 80000, length: 96, atFixMissing: true},
		{name: "aspen_zero", direct: true, gas: 80000, length: 96, aspenZero: true},
	}
	rows := []map[string]any{}
	for _, c := range definitions {
		code := redelegateWrapper(c.static, c.revert, c.gas)
		hash := crypto.Keccak256Hash(code)
		raw := map[common.Hash][]byte{}
		for _, read := range seed.Cases[0].Attempts[0].OrderedReads {
			if read.Present {
				raw[common.HexToHash(read.Key)] = common.Hex2Bytes(read.Value)
			}
		}
		raw[*storage.Stor_k_1([]byte{4})] = []byte{200}
		raw[*storage.Stor_k_1([]byte{5})] = []byte{7, 0xd0}
		inputState := infoInput{accounts: map[common.Address]state_db.Account{
			sender:   {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)},
			wrapper:  {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &hash, CodeSize: uint64(len(code))},
			contract: {Nonce: big.NewInt(1), Balance: big.NewInt(2000)},
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
		observer := &infoObserver{PrecompiledContract: native, backend: backend}
		native.Register(func(address *common.Address, _ vm.PrecompiledContract) {
			e.RegisterPrecompiledContract(address, observer)
		})
		calldata := common.CopyBytes(base[:4+c.length])
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
		rows = append(rows, map[string]any{"name": c.name, "input": infoHex(calldata), "code": infoHex(code), "direct": c.direct, "static": c.static, "parent_revert": c.revert, "value": c.value, "fix": uint64(cfg.Hardforks.FixRedelegateBlockNum), "aspen_zero": c.aspenZero, "caller": infoHex(observer.caller[:]), "depth": observer.depth, "required_gas": observer.quote, "supplied_native_gas": observer.funding, "native_called": observer.called, "native_error": observer.err, "native_output": infoHex(observer.output), "setup_reads": observer.setupReads, "ordered_reads": backend.reads, "ordered_raw_writes": backend.writes, "parent_error": parentError, "parent_output": infoHex(result.CodeRetval), "transaction_gas_used": result.GasUsed, "logs": logs, "accounts": accounts, "prior_raw": prior, "after_raw": after})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": rows, "scope": "synthetic two-validator post-Cornus zero-reward ABI/frame corpus; actual pinned Go; not production acceptance"}); err != nil {
		panic(err)
	}
}
