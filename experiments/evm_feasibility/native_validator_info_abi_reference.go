// Metadata ABI/admission evidence through the actual pinned EVM. Shared support
// is copied unchanged; its main is renamed only in the disposable archive.
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
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

func runInfoABI(name string, abi []byte, gas uint16, value int64) map[string]any {
	owner := common.BytesToAddress([]byte{0xa1})
	sender := common.BytesToAddress([]byte{0xaa})
	validator := common.BytesToAddress([]byte{0x31})
	contract := *dpos.ContractAddress()
	code := infoWrapper(false, false, gas)
	hash := crypto.Keccak256Hash(code)
	key := func(prefix []byte) common.Hash { return *storage.Stor_k_1(prefix, validator[:]) }
	raw := map[common.Hash][]byte{key([]byte{0, 3}): owner[:], key([]byte{0, 1}): rlp.MustEncodeToBytes([][]byte{{}, {}}), key([]byte{0, 5, 2}): {1, 0, 0, 0}, *storage.Stor_k_1([]byte{4}): {10}, *storage.Stor_k_1([]byte{5}): {0x27, 0x10}}
	input := infoInput{accounts: map[common.Address]state_db.Account{owner: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000), CodeHash: &hash, CodeSize: uint64(len(code))}, sender: {Nonce: big.NewInt(1), Balance: big.NewInt(1000000)}, contract: {Nonce: big.NewInt(1), Balance: big.NewInt(10000)}}, codes: map[common.Hash][]byte{hash: code}, raw: raw}
	var state state_evm.TransitionState
	state.Init(state_evm.Opts{})
	state.SetInput(input)
	var e vm.EVM
	e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, &state, vm.DefaultOpts(), params.ChainConfig{ChainId: 841}, vm.Config{})
	e.SetBlock(&vm.Block{Number: 25706949, BlockInfo: vm.BlockInfo{GasLimit: 1000000, Difficulty: new(big.Int)}}, vm.Rules{IsCornus: true, IsMagnolia: true, IsFicus: true, IsCacti: true})
	cfg := chain_config.ChainConfig{}
	cfg.EVMChainConfig = params.ChainConfig{ChainId: 841}
	cfg.Hardforks.FixRedelegateBlockNum = 3091000
	cfg.Hardforks.MagnoliaHf.BlockNum = 5730000
	cfg.Hardforks.CornusHf.BlockNum = 15610000
	cfg.Hardforks.FicusHf.BlockNum = 11616000
	cfg.Hardforks.CactiHf.BlockNum = 24350801
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(1000000000)
	cfg.DPOS.BlocksPerYear = 1
	backend := &infoBackend{EVMStateStorage: storage.EVMStateStorage{EVMStateFace: &state}, reads: []string{}, writes: []map[string]string{}}
	native := new(dpos.Contract).Init(cfg, backend, dpos.Reader{}, &e)
	observer := &infoObserver{PrecompiledContract: native, backend: backend}
	native.Register(func(address *common.Address, _ vm.PrecompiledContract) {
		e.RegisterPrecompiledContract(address, observer)
	})
	to := owner
	from := sender
	if value != 0 {
		to = contract
		from = owner
	}
	result, err := e.Main(&vm.Transaction{From: from, To: &to, Nonce: big.NewInt(1), Value: big.NewInt(value), GasPrice: big.NewInt(0), Gas: 200000, Input: abi})
	logs := []map[string]any{}
	for _, log := range result.Logs {
		topics := []string{}
		for _, topic := range log.Topics {
			topics = append(topics, infoHex(topic[:]))
		}
		logs = append(logs, map[string]any{"address": infoHex(log.Address[:]), "topics": topics, "data": infoHex(log.Data)})
	}
	after := []byte{}
	infoKey := key([]byte{0, 1})
	state.GetAccountConcrete(&contract).GetRawState(&infoKey, func(value []byte) { after = common.CopyBytes(value) })
	parentError := ""
	if err != nil {
		parentError = err.Error()
	}
	return map[string]any{"name": name, "input": infoHex(abi), "caller": infoHex(observer.caller[:]), "value": value, "period": 25706949, "depth": observer.depth, "supplied_native_gas": observer.funding, "required_gas": observer.quote, "native_called": observer.called, "native_error": observer.err, "native_output": infoHex(observer.output), "ordered_reads": backend.reads, "ordered_raw_writes": backend.writes, "logs": logs, "after_info": infoHex(after), "parent_error": parentError, "parent_output": infoHex(result.CodeRetval)}
}

func main() {
	validator := common.BytesToAddress([]byte{0x31})
	base := infoABI(validator, []byte{0xff, 0, 0x61}, []byte("endpoint"))
	clone := func() []byte { return common.CopyBytes(base) }
	word := func(data []byte, offset int, value *big.Int) []byte {
		value.FillBytes(data[offset : offset+32])
		return data
	}
	rows := []map[string]any{}
	add := func(name string, input []byte) { rows = append(rows, runInfoABI(name, input, 20000, 0)) }
	add("cacti_canonical", clone())
	for _, length := range []int{4, 5, 35, 36, 67, 68, 99} {
		add(new(big.Int).SetInt64(int64(length)).String()+"_bytes", clone()[:length])
	}
	add("description_offset_huge", word(clone(), 36, new(big.Int).Lsh(big.NewInt(1), 255)))
	add("endpoint_offset_huge", word(clone(), 68, new(big.Int).Lsh(big.NewInt(1), 255)))
	add("description_length_huge", word(clone(), 100, new(big.Int).Lsh(big.NewInt(1), 255)))
	add("endpoint_length_huge", word(clone(), 164, new(big.Int).Lsh(big.NewInt(1), 255)))
	add("description_length_missing_bytes", word(clone(), 100, big.NewInt(10000)))
	add("description_precedes_missing_endpoint_head", word(clone(), 36, big.NewInt(10000))[:68])
	dirty := clone()
	for index := 4; index < 16; index++ {
		dirty[index] = 0xff
	}
	add("dirty_address_and_trailing", append(dirty, 0xff))
	add("overlapping_tails", word(clone(), 68, big.NewInt(96)))
	add("unpadded_endpoint", clone()[:204])
	unaligned := append(clone()[:100], append([]byte{0}, clone()[100:]...)...)
	word(unaligned, 36, big.NewInt(97))
	word(unaligned, 68, big.NewInt(161))
	add("unaligned_tails", unaligned)
	add("head_alias_empty_strings", word(word(clone(), 36, big.NewInt(64)), 68, big.NewInt(0)))
	rows = append(rows, runInfoABI("short_head_insufficient_gas", clone()[:4], 19999, 0), runInfoABI("short_head_nonpayable", clone()[:4], 20000, 1))
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "chain_id": 841, "cacti": 24350801, "period": 25706949, "cases": rows}); err != nil {
		panic(err)
	}
}
