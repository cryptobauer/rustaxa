// Actual pinned EVM metadata calls. The observer wraps the native interface
// without changing quotes, return values, errors or storage operations.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

func infoHex(value []byte) string { return hex.EncodeToString(value) }

type infoInput struct {
	accounts map[common.Address]state_db.Account
	codes    map[common.Hash][]byte
	raw      map[common.Hash][]byte
}

func (i infoInput) GetAccount(a *common.Address, cb func(state_db.Account)) {
	if account, ok := i.accounts[*a]; ok {
		account.Nonce = new(big.Int).Set(account.Nonce)
		account.Balance = new(big.Int).Set(account.Balance)
		cb(account)
	}
}
func (i infoInput) GetCode(h *common.Hash) []byte {
	code, ok := i.codes[*h]
	if !ok {
		panic("unexpected code read")
	}
	return code
}
func (i infoInput) GetAccountStorage(a *common.Address, k *common.Hash, cb func([]byte)) {
	if *a != *dpos.ContractAddress() {
		panic("unexpected raw address")
	}
	if value, ok := i.raw[*k]; ok {
		cb(common.CopyBytes(value))
	}
}

type infoBackend struct {
	storage.EVMStateStorage
	reads  []string
	writes []map[string]string
}

func (b *infoBackend) GetAccountStorage(a *common.Address, k *common.Hash, cb func([]byte)) {
	b.reads = append(b.reads, infoHex(k[:]))
	b.EVMStateStorage.GetAccountStorage(a, k, cb)
}
func (b *infoBackend) Put(a *common.Address, k *common.Hash, value []byte) {
	b.writes = append(b.writes, map[string]string{"address": infoHex(a[:]), "key": infoHex(k[:]), "value": infoHex(value)})
	b.EVMStateStorage.Put(a, k, value)
}

type infoObserver struct {
	vm.PrecompiledContract
	quote      uint64
	funding    uint64
	called     bool
	output     []byte
	err        string
	depth      uint16
	caller     common.Address
	backend    *infoBackend
	setupReads []string
}

func (o *infoObserver) RequiredGas(ctx vm.CallFrame, e *vm.EVM) uint64 {
	o.funding = ctx.Gas
	o.depth = e.GetDepth()
	o.caller = *ctx.CallerAccount.Address()
	o.quote = o.PrecompiledContract.RequiredGas(ctx, e)
	o.setupReads = append([]string{}, o.backend.reads...)
	o.backend.reads = []string{}
	return o.quote
}
func (o *infoObserver) Run(ctx vm.CallFrame, e *vm.EVM) ([]byte, error) {
	o.called = true
	o.depth = e.GetDepth()
	o.caller = *ctx.CallerAccount.Address()
	out, err := o.PrecompiledContract.Run(ctx, e)
	o.output = common.CopyBytes(out)
	if err != nil {
		o.err = err.Error()
	}
	return out, err
}
func infoABI(validator common.Address, description, endpoint []byte) []byte {
	word := func(value int) []byte {
		out := make([]byte, 32)
		new(big.Int).SetInt64(int64(value)).FillBytes(out)
		return out
	}
	tail := func(value []byte) []byte {
		out := append(word(len(value)), value...)
		return append(out, make([]byte, (32-len(value)%32)%32)...)
	}
	desc := tail(description)
	input := append(crypto.Keccak256([]byte("setValidatorInfo(address,string,string)"))[:4], make([]byte, 12)...)
	input = append(input, validator[:]...)
	input = append(input, word(96)...)
	input = append(input, word(96+len(desc))...)
	input = append(input, desc...)
	return append(input, tail(endpoint)...)
}
func infoWrapper(static, revert bool, gas uint16) []byte {
	code := []byte{0x36, 0x60, 0, 0x60, 0, 0x37, 0x60, 0, 0x60, 0, 0x36, 0x60, 0}
	if !static {
		code = append(code, 0x60, 0)
	}
	code = append(code, 0x60, 0xfe, 0x61, byte(gas>>8), byte(gas))
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
	const fix = 3091000
	const cornus = 15610000
	validator := common.BytesToAddress([]byte{0x31})
	owner := common.BytesToAddress([]byte{0xa1})
	wrong := common.BytesToAddress([]byte{0xaa})
	missing := common.BytesToAddress([]byte{0x99})
	contract := *dpos.ContractAddress()
	cases := []struct {
		name                           string
		description, endpoint          []byte
		wrong, missing, static, revert bool
		period                         uint64
		value                          int64
		gas                            uint16
	}{
		{name: "replace", description: []byte{0xff, 0, 0x61}, endpoint: []byte("endpoint"), period: cornus, gas: 20000},
		{name: "empty", period: cornus, gas: 20000},
		{name: "maximum", description: bytes.Repeat([]byte{0x64}, 100), endpoint: bytes.Repeat([]byte{0x65}, 50), period: cornus, gas: 20000},
		{name: "both_too_long", description: bytes.Repeat([]byte{0x64}, 101), endpoint: bytes.Repeat([]byte{0x65}, 51), period: cornus, gas: 20000},
		{name: "description_too_long", description: bytes.Repeat([]byte{0x64}, 101), period: cornus, gas: 20000},
		{name: "wrong_owner", wrong: true, period: cornus, gas: 20000},
		{name: "missing_validator", missing: true, period: cornus, gas: 20000},
		{name: "static", static: true, description: []byte("static"), period: cornus, gas: 20000},
		{name: "parent_revert", revert: true, description: []byte("irreversible"), period: cornus, gas: 20000},
		{name: "insufficient_gas", period: cornus, gas: 19999},
		{name: "before_fix", period: fix - 1, gas: 20000},
		{name: "at_fix", period: fix, gas: 20000},
		{name: "before_cornus_value", period: cornus - 1, value: 1, gas: 20000},
		{name: "at_cornus_value", period: cornus, value: 1, gas: 20000},
	}
	rows := []map[string]any{}
	for _, c := range cases {
		selected := validator
		if c.missing {
			selected = missing
		}
		caller := owner
		if c.wrong {
			caller = wrong
		}
		// Value admission uses a direct call; other cases use real nested frames.
		code := infoWrapper(c.static, c.revert, c.gas)
		hash := crypto.Keccak256Hash(code)
		key := func(prefix []byte) common.Hash { return *storage.Stor_k_1(prefix, validator[:]) }
		raw := map[common.Hash][]byte{key([]byte{0, 3}): owner[:], key([]byte{0, 1}): rlp.MustEncodeToBytes([][]byte{{}, {}}), key([]byte{0, 5, 2}): {1, 0, 0, 0}, *storage.Stor_k_1([]byte{4}): {10}, *storage.Stor_k_1([]byte{5}): {0x27, 0x10}}
		input := infoInput{accounts: map[common.Address]state_db.Account{owner: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &hash, CodeSize: uint64(len(code))}, wrong: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)}, contract: {Nonce: big.NewInt(1), Balance: big.NewInt(10000)}}, codes: map[common.Hash][]byte{hash: code}, raw: raw}
		// Wrong-owner wrapper must itself be the wrong caller.
		if c.wrong {
			a := input.accounts[wrong]
			a.CodeHash = &hash
			a.CodeSize = uint64(len(code))
			input.accounts[wrong] = a
		}
		var state state_evm.TransitionState
		state.Init(state_evm.Opts{})
		state.SetInput(input)
		var e vm.EVM
		e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &state, vm.DefaultOpts(), params.TestChainConfig, vm.Config{})
		e.SetBlock(&vm.Block{Number: types.BlockNum(c.period), BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: new(big.Int)}}, vm.Rules{IsCornus: c.period >= cornus, IsMagnolia: true})
		cfg := chain_config.ChainConfig{}
		cfg.Hardforks.FixRedelegateBlockNum = fix
		cfg.Hardforks.CornusHf.BlockNum = cornus
		cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
		cfg.DPOS.BlocksPerYear = 1
		backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &state}, reads: []string{}, writes: []map[string]string{}}
		native := new(dpos.Contract).Init(cfg, backend, dpos.Reader{}, &e)
		observer := &infoObserver{PrecompiledContract: native, backend: backend}
		native.Register(func(address *common.Address, _ vm.PrecompiledContract) {
			e.RegisterPrecompiledContract(address, observer)
		})
		abi := infoABI(selected, c.description, c.endpoint)
		to := caller
		from := wrong
		if c.value != 0 {
			to = contract
			from = owner
		}
		result, err := e.Main(&vm.Transaction{From: from, To: &to, Nonce: big.NewInt(1), Value: big.NewInt(c.value), GasPrice: big.NewInt(0), Gas: 200000, Input: abi})
		logs := []map[string]any{}
		for _, log := range result.Logs {
			topics := []string{}
			for _, topic := range log.Topics {
				topics = append(topics, infoHex(topic[:]))
			}
			logs = append(logs, map[string]any{"address": infoHex(log.Address[:]), "topics": topics, "data": infoHex(log.Data)})
		}
		after := []byte{}
		state.GetAccountConcrete(&contract).GetRawState(&[]common.Hash{key([]byte{0, 1})}[0], func(value []byte) { after = common.CopyBytes(value) })
		parentError := ""
		if err != nil {
			parentError = err.Error()
		}
		rows = append(rows, map[string]any{"name": c.name, "input": infoHex(abi), "description": infoHex(c.description), "endpoint": infoHex(c.endpoint), "validator": infoHex(selected[:]), "caller": infoHex(caller[:]), "period": c.period, "value": c.value, "static": c.static, "parent_revert": c.revert, "supplied_native_gas": observer.funding, "required_gas": observer.quote, "native_called": observer.called, "native_error": observer.err, "native_output": infoHex(observer.output), "depth": observer.depth, "setup_reads": observer.setupReads, "ordered_reads": backend.reads, "ordered_raw_writes": backend.writes, "logs": logs, "after_info": infoHex(after), "parent_error": parentError, "parent_output": infoHex(result.CodeRetval), "transaction_gas_used": result.GasUsed})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "fix_redelegate": fix, "cornus": cornus, "scope": "synthetic actual EVM metadata frames; no publication or historical replay", "cases": rows}); err != nil {
		panic(err)
	}
}
