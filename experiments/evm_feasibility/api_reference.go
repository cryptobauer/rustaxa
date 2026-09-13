// Pinned public-API simulation oracle. api_reference.py copies this file into
// disposable public/local taraxa-evm source archives and runs the real
// state_dry_runner.DryRunner.Apply path against a complete synthetic trie.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"reflect"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

const apiPeriod types.BlockNum = 7

var (
	apiSender = common.HexToAddress("0x00000000000000000000000000000000000000aa")
	apiTarget = common.HexToAddress("0x00000000000000000000000000000000000000bb")
	apiRevert = common.HexToAddress("0x00000000000000000000000000000000000000cc")
	apiEmpty  = common.HexToAddress("0x00000000000000000000000000000000000000dd")
	apiSlot   = common.Hash{}
)

func apiHex(bytes []byte) string { return hex.EncodeToString(bytes) }

func apiInteger(value string) *big.Int {
	integer, ok := new(big.Int).SetString(value, 0)
	if !ok {
		panic("invalid integer: " + value)
	}
	return integer
}

// apiMemory is a complete, immutable-after-seeding state_db.DB. TrieSink writes
// canonical account/code/slot rows into it once; DryRunner receives only its
// historical Reader surface. Any attempt to start or commit a pending block is
// an oracle bug and panics.
type apiMemory struct {
	mu         sync.Mutex
	columns    map[byte]map[common.Hash][]byte
	descriptor state_db.StateDescriptor
}

func newAPIMemory() *apiMemory {
	return &apiMemory{columns: map[byte]map[common.Hash][]byte{}}
}

func (memory *apiMemory) Get(column byte, key *common.Hash, callback func([]byte)) {
	memory.mu.Lock()
	value, present := memory.columns[column][*key]
	copyValue := append([]byte(nil), value...)
	memory.mu.Unlock()
	if present {
		callback(copyValue)
	}
}

func (memory *apiMemory) Put(column byte, key *common.Hash, value []byte) {
	memory.mu.Lock()
	defer memory.mu.Unlock()
	if memory.columns[column] == nil {
		memory.columns[column] = map[common.Hash][]byte{}
	}
	memory.columns[column][*key] = append([]byte(nil), value...)
}

func (memory *apiMemory) GetBlockStateReader(block types.BlockNum) state_db.Reader {
	if block != apiPeriod {
		panic("unexpected historical block")
	}
	return memory
}

func (memory *apiMemory) GetLatestState() state_db.LatestState { return memory }

func (memory *apiMemory) GetCommittedDescriptor() state_db.StateDescriptor {
	return memory.descriptor
}

func (*apiMemory) BeginPendingBlock() state_db.PendingBlockState {
	panic("DryRunner attempted a pending-state write")
}

func (*apiMemory) Commit(common.Hash) error {
	panic("DryRunner attempted a committed-state write")
}

func apiCallCode() []byte {
	// slot[0] += 1; return slot[0], ADDRESS.balance after value transfer, and
	// ORIGIN.balance after the gas-cap debit and value transfer.
	return common.FromHex("0x600054600101806000556000523031602052323160405260606000f3")
}

func apiRevertCode() []byte {
	// ABI Error("oracle boom"), built in memory and returned with REVERT.
	return common.FromHex(
		"0x7f08c379a000000000000000000000000000000000000000000000000000000000600052" +
			"7f0000000000000000000000000000000000000000000000000000000000000020600452" +
			"7f000000000000000000000000000000000000000000000000000000000000000b602452" +
			"7f6f7261636c6520626f6f6d000000000000000000000000000000000000000000604452" +
			"60646000fd")
}

func apiInitCode() []byte {
	// Return the ten-byte runtime `PUSH1 0x2a; MSTORE; RETURN(0, 32)`.
	return common.FromHex("0x7f602a60005260206000f3000000000000000000000000000000000000000000600052600a6000f3")
}

func apiSeed() *apiMemory {
	memory := newAPIMemory()
	sink := new(state_transition.TrieSink).Init(nil, state_transition.TrieSinkOpts{})
	defer sink.Close()
	sink.SetIO(memory)

	wideNonce := new(big.Int).Add(new(big.Int).Lsh(big.NewInt(1), 264), big.NewInt(5))
	seed := func(address *common.Address, change state_evm.AccountChange) {
		mutation := sink.StartMutation(address)
		mutation.Update(change)
		mutation.Commit()
	}
	seed(&apiSender, state_evm.AccountChange{Account: state_db.Account{
		Nonce: wideNonce, Balance: apiInteger("10000000000000000000000000000000000000000"),
	}})
	callCode := apiCallCode()
	callHash := crypto.Keccak256Hash(callCode)
	seed(&apiTarget, state_evm.AccountChange{
		Account:         state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(50), CodeHash: &callHash, CodeSize: uint64(len(callCode))},
		RawStorageDirty: state_evm.RawStorage{apiSlot: []byte{7}}, Code: callCode, CodeDirty: true,
	})
	revertCode := apiRevertCode()
	revertHash := crypto.Keccak256Hash(revertCode)
	seed(&apiRevert, state_evm.AccountChange{
		Account: state_db.Account{Nonce: big.NewInt(1), Balance: big.NewInt(11), CodeHash: &revertHash, CodeSize: uint64(len(revertCode))},
		Code:    revertCode, CodeDirty: true,
	})
	memory.descriptor = state_db.StateDescriptor{BlockNum: apiPeriod, StateRoot: sink.Commit()}
	return memory
}

type apiAccountObservation struct {
	Address     string `json:"address"`
	Exists      bool   `json:"exists"`
	Nonce       string `json:"nonce,omitempty"`
	Balance     string `json:"balance,omitempty"`
	StorageRoot string `json:"storage_root,omitempty"`
	CodeHash    string `json:"code_hash,omitempty"`
	CodeSize    uint64 `json:"code_size,omitempty"`
	Encoded     string `json:"encoded,omitempty"`
	Code        string `json:"code,omitempty"`
}

type apiStateObservation struct {
	Period   uint64                  `json:"period"`
	Root     string                  `json:"root"`
	Accounts []apiAccountObservation `json:"accounts"`
	Slot     struct {
		Address string `json:"address"`
		Key     string `json:"key"`
		Present bool   `json:"present"`
		Value   string `json:"value"`
	} `json:"slot"`
}

func apiObserve(memory *apiMemory) apiStateObservation {
	reader := state_db.ExtendedReader{Reader: memory}
	observation := apiStateObservation{Period: uint64(memory.descriptor.BlockNum), Root: apiHex(memory.descriptor.StateRoot[:])}
	for _, address := range []common.Address{apiSender, apiTarget, apiRevert, common.ZeroAddress, apiEmpty} {
		row := apiAccountObservation{Address: apiHex(address[:])}
		reader.GetRawAccount(&address, func(encoded []byte) {
			account := state_db.DecodeAccountFromTrie(encoded)
			row.Exists = true
			row.Nonce = account.Nonce.String()
			row.Balance = account.Balance.String()
			row.CodeSize = account.CodeSize
			row.Encoded = apiHex(encoded)
			if account.StorageRootHash != nil {
				row.StorageRoot = apiHex(account.StorageRootHash[:])
			}
			if account.CodeHash != nil {
				row.CodeHash = apiHex(account.CodeHash[:])
				row.Code = apiHex(reader.GetCode(account.CodeHash))
			}
		})
		observation.Accounts = append(observation.Accounts, row)
	}
	observation.Slot.Address = apiHex(apiTarget[:])
	observation.Slot.Key = apiHex(apiSlot[:])
	reader.GetAccountStorage(&apiTarget, &apiSlot, func(value []byte) {
		observation.Slot.Present = true
		observation.Slot.Value = apiHex(value)
	})
	return observation
}

type apiTransactionInput struct {
	From          string `json:"from"`
	To            string `json:"to,omitempty"`
	SuppliedNonce string `json:"supplied_nonce"`
	GasPrice      string `json:"gas_price"`
	Gas           uint64 `json:"gas"`
	Value         string `json:"value"`
	Input         string `json:"input"`
}

type apiExecutionOutput struct {
	EffectiveNonce string   `json:"effective_nonce"`
	GasUsed        uint64   `json:"gas_used"`
	ConsensusError string   `json:"consensus_error"`
	ExecutionError string   `json:"execution_error"`
	Return         string   `json:"return"`
	Created        string   `json:"created"`
	Logs           []string `json:"logs"`
}

type apiCase struct {
	Name   string              `json:"name"`
	Input  apiTransactionInput `json:"input"`
	Output apiExecutionOutput  `json:"output"`
}

func apiRun(runner *state_dry_runner.DryRunner, block *vm.Block, name string, transaction vm.Transaction) apiCase {
	input := apiTransactionInput{
		From: apiHex(transaction.From[:]), SuppliedNonce: transaction.Nonce.String(),
		GasPrice: transaction.GasPrice.String(), Gas: transaction.Gas,
		Value: transaction.Value.String(), Input: apiHex(transaction.Input),
	}
	if transaction.To != nil {
		input.To = apiHex(transaction.To[:])
	}
	result := runner.Apply(block, &transaction)
	logs := make([]string, len(result.Logs))
	for index, log := range result.Logs {
		encoded, err := json.Marshal(log)
		if err != nil {
			panic(err)
		}
		logs[index] = string(encoded)
	}
	return apiCase{Name: name, Input: input, Output: apiExecutionOutput{
		EffectiveNonce: transaction.Nonce.String(), GasUsed: result.GasUsed,
		ConsensusError: string(result.ConsensusErr), ExecutionError: string(result.ExecutionErr),
		Return: apiHex(result.CodeRetval), Created: apiHex(result.NewContractAddr[:]), Logs: logs,
	}}
}

func main() {
	memory := apiSeed()
	before := apiObserve(memory)
	config := &chain_config.ChainConfig{EVMChainConfig: params.TestChainConfig}
	config.Hardforks.MagnoliaHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartOne = types.BlockNumberNIL
	config.Hardforks.AspenHf.BlockNumPartTwo = types.BlockNumberNIL
	config.Hardforks.FicusHf.BlockNum = types.BlockNumberNIL
	config.Hardforks.CornusHf.BlockNum = 0
	config.Hardforks.CactiHf.BlockNum = types.BlockNumberNIL
	block := &vm.Block{Number: apiPeriod, BlockInfo: vm.BlockInfo{GasLimit: 1_000_000, Time: 1_700_000_007, Difficulty: big.NewInt(0)}}
	runner := new(state_dry_runner.DryRunner).Init(memory, func(types.BlockNum) *big.Int { return new(big.Int) }, nil, nil, config)

	wideSupplied := new(big.Int).Lsh(big.NewInt(1), 512)
	transactions := []struct {
		name        string
		transaction vm.Transaction
	}{
		{"call_stale_nonce", vm.Transaction{From: apiSender, To: &apiTarget, Nonce: big.NewInt(0), GasPrice: big.NewInt(2), Gas: 100_000, Value: big.NewInt(123)}},
		{"call_large_nonce", vm.Transaction{From: apiSender, To: &apiTarget, Nonce: wideSupplied, GasPrice: big.NewInt(2), Gas: 100_000, Value: big.NewInt(123)}},
		{"call_revert_reason", vm.Transaction{From: apiSender, To: &apiRevert, Nonce: big.NewInt(3), GasPrice: big.NewInt(3), Gas: 100_000, Value: big.NewInt(17)}},
		{"create_large_nonce", vm.Transaction{From: apiSender, Nonce: wideSupplied, GasPrice: big.NewInt(1), Gas: 150_000, Value: big.NewInt(25), Input: apiInitCode()}},
		{"zero_sender_fee_exempt", vm.Transaction{From: common.ZeroAddress, To: &apiEmpty, Nonce: wideSupplied, GasPrice: big.NewInt(999), Gas: 30_000, Value: big.NewInt(0)}},
	}
	cases := make([]apiCase, 0, len(transactions))
	for _, item := range transactions {
		cases = append(cases, apiRun(runner, block, item.name, item.transaction))
	}
	repeatOne := apiRun(runner, block, "repeat_one", transactions[0].transaction)
	repeatTwo := apiRun(runner, block, "repeat_two", transactions[0].transaction)
	after := apiObserve(memory)

	result := map[string]any{
		"schema": 1,
		"reference_config": map[string]any{
			"chain_id": params.TestChainConfig.ChainId, "period": uint64(apiPeriod),
			"block_gas_limit": block.GasLimit, "timestamp": block.Time, "difficulty": block.Difficulty.String(),
			"rules":           map[string]bool{"magnolia": false, "aspen_part_one": false, "aspen_part_two": false, "ficus": false, "cornus": true, "cacti": false},
			"dpos_registered": false, "block_hash": "0",
		},
		"semantics": map[string]any{
			"entrypoint": "state_dry_runner.DryRunner.Apply", "nonce": "supplied nonce is replaced with persisted sender nonce + 1",
			"state": "each Apply uses a new non-committing BlockState over the same committed historical reader",
		},
		"state_before": before, "cases": cases,
		"repeated":    map[string]any{"first": repeatOne.Output, "second": repeatTwo.Output, "identical": reflect.DeepEqual(repeatOne.Output, repeatTwo.Output)},
		"state_after": after, "committed_state_unchanged": reflect.DeepEqual(before, after),
	}
	if err := json.NewEncoder(os.Stdout).Encode(result); err != nil {
		panic(err)
	}
}
