// Actual EVM escrow-entry calls, with call value forwarded by the real wrapper.
package main

import (
	"encoding/json"
	"math/big"
	"os"

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
)

func main() {
	const phala = 6943000
	const cornus = 15610000
	const fix = 3091000
	sender, wrapper := common.BytesToAddress([]byte{0xaa}), common.BytesToAddress([]byte{0xa1})
	contract := *dpos.ContractAddress()
	definitions := []struct {
		name                             string
		period, phala, fix               uint64
		value                            int64
		gas                              uint16
		direct, static, revert, trailing bool
	}{
		{name: "direct_zero", period: cornus, phala: phala, fix: fix, direct: true, gas: 1000},
		{name: "direct_value", period: cornus, phala: phala, fix: fix, direct: true, value: 7, gas: 1000},
		{name: "nested_value", period: cornus, phala: phala, fix: fix, value: 7, gas: 1000},
		{name: "static_zero", period: cornus, phala: phala, fix: fix, static: true, gas: 1000},
		{name: "parent_revert", period: cornus, phala: phala, fix: fix, value: 7, gas: 1000, revert: true},
		{name: "insufficient", period: cornus, phala: phala, fix: fix, value: 7, gas: 999},
		{name: "before_phala", period: phala - 1, phala: phala, fix: fix, direct: true, value: 7, gas: 1000},
		{name: "at_phala", period: phala, phala: phala, fix: fix, direct: true, value: 7, gas: 1000},
		{name: "trailing", period: cornus, phala: phala, fix: fix, direct: true, value: 7, gas: 1000, trailing: true},
		{name: "nested_before_fix", period: 1, phala: 0, fix: 2, value: 7, gas: 1000},
		{name: "unfunded_before_fix", period: 1, phala: 0, fix: 2, value: 7, gas: 999},
		{name: "inactive_after_cornus", period: cornus, phala: cornus + 1, fix: fix, direct: true, value: 7, gas: 1000},
	}
	rows := []map[string]any{}
	for _, c := range definitions {
		code := infoWrapper(c.static, c.revert, c.gas)
		if !c.static {
			// Replace the metadata wrapper's PUSH1 0 call value with CALLVALUE.
			code = append(append(append([]byte{}, code[:13]...), 0x34), code[15:]...)
		}
		codeHash := crypto.Keccak256Hash(code)
		raw := map[common.Hash][]byte{*storage.Stor_k_1([]byte{4}): {10}, *storage.Stor_k_1([]byte{5}): {0x27, 0x10}}
		inputState := infoInput{accounts: map[common.Address]state_db.Account{
			sender:   {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)},
			wrapper:  {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &codeHash, CodeSize: uint64(len(code))},
			contract: {Nonce: big.NewInt(1), Balance: big.NewInt(10000)},
		}, codes: map[common.Hash][]byte{codeHash: code}, raw: raw}
		var state state_evm.TransitionState
		state.Init(state_evm.Opts{})
		state.SetInput(inputState)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: types.BlockNum(c.period), BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: new(big.Int)}}, vm.Rules{IsMagnolia: true, IsCornus: c.period >= cornus})
		cfg := chain_config.ChainConfig{}
		cfg.Hardforks.FixRedelegateBlockNum = c.fix
		cfg.Hardforks.PhalaenopsisHfBlockNum = c.phala
		cfg.Hardforks.CornusHf.BlockNum = cornus
		cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
		cfg.DPOS.BlocksPerYear = 1
		backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &state}, reads: []string{}, writes: []map[string]string{}}
		native := new(dpos.Contract).Init(cfg, backend, dpos.Reader{}, &e)
		observer := &infoObserver{PrecompiledContract: native, backend: backend}
		native.Register(func(address *common.Address, _ vm.PrecompiledContract) {
			e.RegisterPrecompiledContract(address, observer)
		})
		calldata := common.Hex2Bytes("44df8e70")
		if c.trailing {
			calldata = append(calldata, 0)
		}
		to := wrapper
		if c.direct {
			to = contract
		}
		result, err := e.Main(&vm.Transaction{From: sender, To: &to, Nonce: big.NewInt(1), GasPrice: new(big.Int), Gas: 200000, Value: big.NewInt(c.value), Input: calldata})
		parentError := ""
		if err != nil {
			parentError = err.Error()
		}
		accounts := map[string]any{}
		for _, address := range []common.Address{sender, wrapper, contract} {
			a := state.GetAccountConcrete(&address)
			accounts[infoHex(address[:])] = map[string]string{"nonce": a.GetNonce().String(), "balance": a.GetBalance().String()}
		}
		prior := map[string]string{}
		after := map[string]string{}
		for key, value := range raw {
			prior[infoHex(key[:])] = infoHex(value)
			state.GetAccountConcrete(&contract).GetRawState(&key, func(value []byte) { after[infoHex(key[:])] = infoHex(value) })
		}
		rows = append(rows, map[string]any{"name": c.name, "period": c.period, "phala": c.phala, "fix": c.fix, "value": c.value, "input": infoHex(calldata), "code": infoHex(code), "direct": c.direct, "static": c.static, "parent_revert": c.revert, "supplied_native_gas": observer.funding, "required_gas": observer.quote, "native_called": observer.called, "native_error": observer.err, "native_output": infoHex(observer.output), "depth": observer.depth, "caller": infoHex(observer.caller[:]), "setup_reads": observer.setupReads, "ordered_reads": backend.reads, "ordered_raw_writes": backend.writes, "parent_error": parentError, "parent_output": infoHex(result.CodeRetval), "transaction_gas_used": result.GasUsed, "logs_count": len(result.Logs), "accounts": accounts, "prior_raw": prior, "after_raw": after})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "cases": rows, "scope": "actual EVM escrow selector frames; mainnet activation neighbors plus explicit synthetic pre-fix ordering; no production route"}); err != nil {
		panic(err)
	}
}
