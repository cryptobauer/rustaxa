package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

type stateKey struct {
	column state_db.Column
	hash   common.Hash
}

type fixtureState map[stateKey][]byte

func (s fixtureState) GetBlockStateReader(number types.BlockNum) state_db.Reader {
	if number != 0 {
		panic(fmt.Sprintf("expected parent block 0, got %d", number))
	}
	return s
}
func (s fixtureState) GetLatestState() state_db.LatestState { return s }
func (s fixtureState) GetCommittedDescriptor() state_db.StateDescriptor {
	return state_db.StateDescriptor{BlockNum: 0}
}
func (s fixtureState) BeginPendingBlock() state_db.PendingBlockState { panic("unexpected state write") }
func (s fixtureState) Commit(common.Hash) error                      { panic("unexpected state write") }
func (s fixtureState) Get(column state_db.Column, hash *common.Hash, receive func([]byte)) {
	if value, exists := s[stateKey{column, *hash}]; exists {
		receive(value)
	}
}

func (s fixtureState) addAccount(address common.Address, balance int64, code []byte) {
	account := state_db.Account{Nonce: big.NewInt(0), Balance: big.NewInt(balance)}
	if len(code) != 0 {
		hash := crypto.Keccak256Hash(code)
		s[stateKey{state_db.COL_code, hash}] = code
		account.CodeHash = &hash
		account.CodeSize = uint64(len(code))
	}
	encoded, _ := account.EncodeForTrie()
	s[stateKey{state_db.COL_main_trie_value, crypto.Keccak256Hash(address[:])}] = encoded
}

func bytes(value string) []byte {
	result, err := hex.DecodeString(value)
	if err != nil {
		panic(err)
	}
	return result
}

func address(last byte) *common.Address {
	result := common.BytesToAddress([]byte{last})
	return &result
}

func transaction(to *common.Address, input string, nonce, value int64) vm.Transaction {
	return vm.Transaction{
		From: *address(0x11), To: to, Input: bytes(input), Nonce: big.NewInt(nonce),
		Value: big.NewInt(value), GasPrice: big.NewInt(0), Gas: 100_000,
	}
}

func transactionWithGas(to *common.Address, gas uint64) vm.Transaction {
	result := transaction(to, "", 0, 0)
	result.Gas = gas
	return result
}

type scenario struct {
	name   string
	prefix []vm.Transaction
	target []vm.Transaction
}

func trace(runner *state_dry_runner.TraceRunner, block *vm.Block, item scenario, config vm.TracingConfig) (result json.RawMessage) {
	defer func() {
		if err := recover(); err != nil {
			panic(fmt.Sprintf("%s %+v: %v", item.name, config, err))
		}
	}()
	return runner.Trace(block, &item.prefix, &item.target, &config)
}

func main() {
	state := fixtureState{}
	state.addAccount(*address(0x11), 1_000_000, nil)
	state.addAccount(*address(0xaa), 0, bytes("602a60005260206000f3"))
	state.addAccount(*address(0xbb), 0, bytes("6020600060006000600060aa61fffff15060206000f3"))
	state.addAccount(*address(0xcc), 0, bytes("602a60005260206000fd"))
	state.addAccount(*address(0xdd), 0, bytes("fe"))
	state.addAccount(*address(0xa1), 0, bytes("6000548060010160005560005260206000f3"))
	state.addAccount(*address(0xa2), 0, bytes("600160005360016000600160006000600461fffff15060016000f3"))

	state.addAccount(*address(0xe1), 0, bytes("600156"))
	state.addAccount(*address(0xe2), 0, bytes("600001"))
	state.addAccount(*address(0xe3), 0, bytes("602a61ffff52"))
	state.addAccount(*address(0xe4), 0, bytes("69600060005360016000f3600052600a60166000f05000"))
	state.addAccount(*address(0xe5), 0, bytes("6020600060006000600060aa61fffff25060206000f3"))
	state.addAccount(*address(0xe6), 0, bytes("602060006000600060aa61fffff45060206000f3"))
	state.addAccount(*address(0xe7), 0, bytes("602060006000600060a161fffffa5060206000f3"))
	state.addAccount(*address(0xe8), 3, bytes("6033ff"))
	state.addAccount(*address(0xe9), 0, bytes("60206000600060006000603361fffff15060206000f3"))
	state.addAccount(*address(0xea), 0, bytes("600101"))
	state.addAccount(*address(0xeb), 0, bytes("6000600060006000600060ec61fffff100"))
	state.addAccount(*address(0xec), 0, bytes("00"))

	state.addAccount(*address(0xc1), 0, bytes("60ff60020a"))
	state.addAccount(*address(0xc2), 0, bytes("6001600055"))
	state.addAccount(*address(0xc3), 0, bytes("6000600060006000600060aa61fffff100"))
	state.addAccount(*address(0xc4), 0, bytes("61ffff6000600037"))
	state.addAccount(*address(0xc5), 0, bytes("6101006000a0"))
	state.addAccount(*address(0xc6), 0, bytes("600001"))
	state.addAccount(*address(0xc7), 0, bytes("602060002000"))
	state.addAccount(*address(0xc8), 0, bytes("6020600020"))
	state.addAccount(*address(0xc9), 0, bytes("6020600060003e"))

	state.addAccount(*address(0xca), 0, bytes("600160006000f0"))
	state.addAccount(*address(0xcb), 0, bytes("6000600160006000f5"))

	state.addAccount(*address(0xd0), 0, bytes("600060006000600060d2600afa00"))
	state.addAccount(*address(0xd1), 0, bytes("600060006000600060d260fffa00"))
	state.addAccount(*address(0xd3), 0, bytes("b3"))
	state.addAccount(*address(0xd4), 0, bytes("b4"))
	state.addAccount(*address(0xd2), 0, bytes("60016000b400"))

	state.addAccount(*address(0xd5), 0, bytes("600060006000600060d660fffa00"))
	state.addAccount(*address(0xd6), 0, bytes("b400"))

	state.addAccount(*address(0xd7), 0, bytes("48"))
	state.addAccount(*address(0xd8), 0, bytes("5d"))

	// Block 1 uses the Ficus and Cornus rules. Cacti starts at block 2.
	schedule := chain_config.ChainConfig{EVMChainConfig: params.TestChainConfig}
	schedule.Hardforks.CactiHf.BlockNum = 2
	block := vm.Block{Number: 1, BlockInfo: vm.BlockInfo{
		Author: *address(0x22), GasLimit: 100_000_000, Time: 10, Difficulty: big.NewInt(0),
	}}
	var runner state_dry_runner.TraceRunner
	runner.Init(state, func(types.BlockNum) *big.Int { return big.NewInt(0) }, nil, nil, &schedule)
	increment := transaction(address(0xa1), "", 0, 0)
	incrementAgain := transaction(address(0xa1), "", 1, 0)
	transfer := transaction(address(0x33), "", 0, 2)
	transfer.Gas = 21_000
	lowGas := transaction(address(0xea), "abcdef", 0, 0)
	lowGas.Gas = 21_000
	memoryOutOfGas := transaction(address(0xe3), "", 0, 0)
	memoryOutOfGas.Gas = 22_000
	scenarios := []scenario{
		{name: "call_memory_return", target: []vm.Transaction{transaction(address(0xaa), "", 0, 0)}},
		{name: "nested_call", target: []vm.Transaction{transaction(address(0xbb), "", 0, 0)}},
		{name: "create", target: []vm.Transaction{transaction(nil, "600060005360016000f3", 0, 0)}},
		{name: "create_revert", target: []vm.Transaction{transaction(nil, "602a60005260206000fd", 0, 0)}},
		{name: "call_revert", target: []vm.Transaction{transaction(address(0xcc), "", 0, 0)}},
		{name: "native_identity", target: []vm.Transaction{transaction(address(4), "1234", 0, 0)}},
		{name: "nested_native_identity", target: []vm.Transaction{transaction(address(0xa2), "", 0, 0)}},
		{name: "invalid_opcode", target: []vm.Transaction{transaction(address(0xdd), "", 0, 0)}},
		{name: "invalid_jump", target: []vm.Transaction{transaction(address(0xe1), "", 0, 0)}},
		{name: "stack_underflow", target: []vm.Transaction{transaction(address(0xe2), "", 0, 0)}},
		{name: "memory_out_of_gas", target: []vm.Transaction{memoryOutOfGas}},
		{name: "nested_create", target: []vm.Transaction{transaction(address(0xe4), "", 0, 0)}},
		{name: "nested_callcode", target: []vm.Transaction{transaction(address(0xe5), "", 0, 0)}},
		{name: "nested_delegate", target: []vm.Transaction{transaction(address(0xe6), "", 0, 2)}},
		{name: "nested_static_error", target: []vm.Transaction{transaction(address(0xe7), "", 0, 0)}},
		{name: "selfdestruct", target: []vm.Transaction{transaction(address(0xe8), "", 0, 0)}},
		{name: "nested_no_code", target: []vm.Transaction{transaction(address(0xe9), "", 0, 0)}},
		{name: "insufficient_intrinsic", target: []vm.Transaction{lowGas}},

		{name: "native_identity_in_initcode", target: []vm.Transaction{transaction(nil, "600160005360016000600160006000600461fffff15060016000f3", 0, 0)}},

		{name: "nested_stop", target: []vm.Transaction{transaction(address(0xeb), "", 0, 0)}},
		{name: "exp_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc1), 21020)}},
		{name: "sstore_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc2), 21012)}},
		{name: "call_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc3), 21300)}},
		{name: "copy_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc4), 22000)}},
		{name: "log_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc5), 22000)}},
		{name: "low_gas_underflow", target: []vm.Transaction{transactionWithGas(address(0xc6), 21003)}},
		{name: "keccak", target: []vm.Transaction{transactionWithGas(address(0xc7), 100000)}},
		{name: "keccak_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc8), 21012)}},
		{name: "returndatacopy_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xc9), 21012)}},
		{name: "create_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xca), 21021)}},
		{name: "create2_out_of_gas", target: []vm.Transaction{transactionWithGas(address(0xcb), 21021)}},
		{name: "static_tstore_low", target: []vm.Transaction{transaction(address(0xd0), "", 0, 0)}},
		{name: "static_tstore_affordable", target: []vm.Transaction{transaction(address(0xd1), "", 0, 0)}},
		{name: "tload_underflow", target: []vm.Transaction{transaction(address(0xd3), "", 0, 0)}},
		{name: "tstore_underflow", target: []vm.Transaction{transaction(address(0xd4), "", 0, 0)}},
		{name: "static_tstore_missing_operands", target: []vm.Transaction{transaction(address(0xd5), "", 0, 0)}},

		{name: "unsupported_basefee_low_gas", target: []vm.Transaction{transactionWithGas(address(0xd7), 21000)}},
		{name: "inactive_tstore_low_gas", target: []vm.Transaction{transactionWithGas(address(0xd8), 21000)}},
		{name: "pure_transfer", target: []vm.Transaction{transfer}},
		{name: "replay_with_prefix", prefix: []vm.Transaction{increment}, target: []vm.Transaction{incrementAgain}},
		{name: "sequential_block", target: []vm.Transaction{increment, incrementAgain}},
	}
	configs := []struct {
		name string
		mode vm.TracingConfig
	}{
		{"trace", vm.TracingConfig{Trace: true}},
		{"vmTrace", vm.TracingConfig{VmTrace: true}},
		{"combined", vm.TracingConfig{Trace: true, VmTrace: true}},
	}
	results := map[string]map[string]json.RawMessage{}
	for _, item := range scenarios {
		modes := map[string]json.RawMessage{}
		for _, cfg := range configs {
			modes[cfg.name] = trace(&runner, &block, item, cfg.mode)
		}
		results[item.name] = modes
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(results); err != nil {
		panic(err)
	}
}
