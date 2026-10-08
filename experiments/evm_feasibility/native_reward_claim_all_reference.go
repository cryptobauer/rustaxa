// Actual selected positive noarg claim-all direct call over canonical reward-only P1. Disposable code overlay only.
package main

import (
	"bytes"
	"encoding/binary"
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
		name                       string
		commission, static, revert bool
	}{
		{name: "claim_all_positive"},
	}
	execute := func(definition struct {
		name                       string
		commission, static, revert bool
	}) map[string]any {
		wrapper, sender := *dpos.ContractAddress(), nativeSimulationSender
		input := common.FromHex("0b83a727")
		code := []byte{}
		state := state_evm.GetBlockState(seed.database, 1, 1)
		indexReader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(1)}
		prefix := append([]byte{2, 1}, sender[:]...)
		countKey := common.BytesToHash(crypto.Keccak256(append(common.CopyBytes(prefix), 1)))
		countBytes := []byte{}
		indexReader.GetAccountStorage(&wrapper, &countKey, func(v []byte) { countBytes = common.CopyBytes(v) })
		if len(countBytes) != 4 {
			panic("index count physical shape")
		}
		items := []map[string]any{}
		for position := uint32(1); position <= binary.LittleEndian.Uint32(countBytes); position++ {
			suffix := make([]byte, 5)
			suffix[0] = 2 // Effective pinned alias-backed item namespace, not nominal 0.
			binary.LittleEndian.PutUint32(suffix[1:], position)
			key := common.BytesToHash(crypto.Keccak256(append(common.CopyBytes(prefix), suffix...)))
			value := []byte{}
			indexReader.GetAccountStorage(&wrapper, &key, func(v []byte) { value = common.CopyBytes(v) })
			if len(value) != 20 {
				panic("index item physical shape")
			}
			items = append(items, map[string]any{"position": position, "key": infoHex(key[:]), "value": infoHex(value)})
		}
		indexBefore := map[string]any{"count_key": infoHex(countKey[:]), "count_value": infoHex(countBytes), "items": items}

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
		result, err := e.Main(&vm.Transaction{From: sender, To: &wrapper, Nonce: new(big.Int), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: input})
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
		row := map[string]any{"name": definition.name, "commission": definition.commission, "static": definition.static, "parent_revert": definition.revert, "code": infoHex(code), "input": infoHex(input), "from": infoHex(sender[:]), "to": infoHex(wrapper[:]), "nonce": 0, "gas": 200000, "gas_price": "0", "value": 0, "transaction_gas_used": result.GasUsed, "parent_output": infoHex(result.CodeRetval), "parent_error": parentError, "consensus_error": consensusError, "execution_error": executionError, "logs": frameLogs(result.Logs), "accounts_before": accountsBefore, "accounts": frameAccounts(state), "index_before": indexBefore}
		// Fixed observed-key union from the first dual-pin capture, also in controls.
		selected := map[common.Hash]bool{}
		for _, key := range []string{
			"0588cd32df1b7d74eac56d18e1d7f8eed9fddd59d9c99ae677f4e6dda4d49ebe",
			"0b0e706ca82a38030addf6404e56ea0efe335a7359c8cb6935b79757e91f6b60",
			"1854cf1c6f2e51a4f3d63254acfe4730bbe70dc15b77679101a6c85fe4e468c7",
			"18d1b9e8964b17ead9b5678f9df74c0c0f28aa2785ae296434c73a3f53469e0c",
			"4c7a777396972eccb784d1842f015e33042408e1eab6b5d2d412c5dfbdd22c05",
			"4ea47a5db4abbbddf0d3f15bf4c5c706e9c788fa5d2b06349c6084aeddb0de92",
			"4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a",
			"5a18b753dd0b15c2716303fc7a79783012cf46840ac66cd7f245d6fcc7cd747b",
			"753a31f794e347eb628f7c87fad5ce1812c431b9be3155ae4788e93d2acedad3",
			"754536fdc3837580b0c148ccad50cac1c2d84dc9a1f8df2790b6f964f4af3180",
			"9628d16514f79a703e4482fabee9a299dff013617c786d3b9c36342714d0812e",
			"9ae80875c0edd2aff0704ec46223edf3402222a7287dd2a517bab4d37cee378a",
			"a61c74a5d06d67459f2b34279269fc29b4f38e3812d313333f4564bd5cdd43e9",
			"b5137228d1309411928257c9edb4d6e6fe4b0cd8e3cc6ade0279502971cfc7e5",
			"cb2211b4efe37f26312eb4c959d610946316aad4e81eca5a5b50f2cadb83b7a1",
			"d0591206d9e81e07f4defc5327957173572bcd1bca7838caa7be39b0c12b1873",
			"dbb8d0f4c497851a5043c6363657698cb1387682cac2f786c731f8936109d795",
			"e8595be39863dce96c5e29d86669ffb30f4e4d19ef14907f331391288d7eaab6",
			"ee2a4bc7db81da2b7164e56b3649b1e2a09c58c455b15dabddd9146c7582cebc",
			"f343681465b9efe82c933c3e8748c70cb8aa06539c361de20f72eac04e766393",
			"fd2a0d75c38edd1628e6f077003f7949d6405ec53bd36fc560972a296914beb0",
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
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": currentSignedSnapshot(seed, api), "period": 2, "cases": rows, "scope": "actual selected positive noarg claimAllRewards directCALLD on disposable BlockState over canonical92rowP1/native2; currentcallerindex copied from detached physicalreader; no codeoverlay, committed mutation, root parity or production acceptance"}); err != nil {
		panic(err)
	}
}
