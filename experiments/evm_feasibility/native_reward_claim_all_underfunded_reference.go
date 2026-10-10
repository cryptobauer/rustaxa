// Actual H1 DryRunner and independent P2 EVM.Main claim-all gas boundaries.
// Each attempt uses a fresh disposable owner over the same committed P1.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"
	"reflect"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

func boundaryHex(data []byte) string { return hex.EncodeToString(data) }

func boundaryIdentity(key byte) common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{key}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	return common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
}

func main() {
	nativeSimulationSender = boundaryIdentity(0x31)
	nativeSimulationDelegator = boundaryIdentity(0xa1)
	seed := seedRewardH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	before := currentSignedSnapshot(seed, api)
	priorRaw := h1RawCopy(state_evm.GetBlockState(seed.database, 1, 1))
	contract := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	control := os.Getenv("CLAIM_ALL_BOUNDARY_CONTROL") == "1"
	var quotes, runs, reads []map[string]any
	var writes, balances []map[string]string
	stage := ""
	vm.SetClaimAllBoundaryObserver(func(kind string, start bool, frame vm.CallFrame, e *vm.EVM, required uint64, output []byte, err error) {
		if start {
			stage = kind
			if kind == "run" {
				runs = append(runs, map[string]any{"supplied_native_gas": frame.Gas})
			}
			return
		}
		if kind == "quote" {
			caller := frame.CallerAccount.Address()
			quotes = append(quotes, map[string]any{"required_gas": required, "supplied_native_gas": frame.Gas,
				"caller": boundaryHex(caller[:]), "input": boundaryHex(frame.Input), "depth": e.GetDepth(), "value": frame.Value.String()})
		} else {
			row := runs[len(runs)-1]
			row["output"] = boundaryHex(output)
			row["error"] = ""
			if err != nil {
				row["error"] = err.Error()
			}
		}
		stage = ""
	})
	storage.SetClaimAllBoundaryReadObserver(func(kind string, address *common.Address, key *common.Hash) {
		if !control && stage != "" {
			reads = append(reads, map[string]any{"stage": stage, "kind": kind, "address": boundaryHex(address[:]), "key": boundaryHex(key[:])})
		}
	})
	state_evm.SetNativeV1CustodyRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		if !control && stage != "" {
			writes = append(writes, map[string]string{"address": boundaryHex(address[:]), "key": boundaryHex(key[:]), "value": boundaryHex(value)})
		}
	})
	state_evm.SetRewardBalanceObserver(func(address common.Address, operation string, amount, balance *big.Int) {
		if !control && stage != "" {
			balances = append(balances, map[string]string{"address": boundaryHex(address[:]), "operation": operation, "amount": amount.String(), "balance": balance.String()})
		}
	})
	accounts := map[string]any{}
	afterRaw := map[string]string{}
	post := func(state *state_evm.BlockState) {
		accounts = map[string]any{}
		for _, address := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, contract} {
			account := state.GetAccountConcrete(&address)
			accounts[boundaryHex(address[:])] = map[string]string{"nonce": account.GetNonce().String(), "balance": account.GetBalance().String()}
		}
		afterRaw = h1RawCopy(state)
	}
	state_dry_runner.SetRewardH1PostObserver(post)
	cases := []map[string]any{}
	for _, period := range []types.BlockNum{1, 2} {
		block := &vm.Block{Number: period, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000000 + uint64(period), Difficulty: new(big.Int)}}
		for _, cap := range []uint64{21271, 21272, 21273, 90000, 111271, 111272, 111273} {
			attempts := []map[string]any{}
			for repeat := 0; repeat < 2; repeat++ {
				quotes, runs, reads = []map[string]any{}, []map[string]any{}, []map[string]any{}
				writes, balances = []map[string]string{}, []map[string]string{}
				stage = ""
				tx := vm.Transaction{From: nativeSimulationSender, To: &contract, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: cap, Value: new(big.Int), Input: common.FromHex("0b83a727")}
				suppliedNonce := new(big.Int).Set(tx.Nonce)
				var result vm.ExecutionResult
				var parentError any // DryRunner returns no separate parent error lane.
				owner := "DryRunner.Apply"
				if period == 1 {
					result = runner.Apply(block, &tx)
				} else {
					// Independent pending execution: keep concrete reader1, execute at2.
					// This owner uses the explicit current nonce, not DryRunner's override.
					owner = "EVM.Main"
					parentError = ""
					state := state_evm.GetBlockState(seed.database, 1, 1)
					tx.Nonce = new(big.Int).Set(state.GetAccount(&tx.From).GetNonce())
					suppliedNonce.Set(tx.Nonce)
					var e vm.EVM
					e.Init(func(types.BlockNum) *big.Int { return new(big.Int) }, state, vm.DefaultOpts(), seed.config.EVMChainConfig, vm.Config{})
					e.SetBlock(block, seed.config.Hardforks.Rules(period))
					api.InitAndRegisterAllContracts(storage.EVMStateStorage{EVMStateFace: state}, period, factory, &e, e.RegisterPrecompiledContract)
					var err error
					result, err = e.Main(&tx)
					if err != nil {
						parentError = err.Error()
					}
					post(state)
				}
				// Exact output comes from the returned execution result, never a prediction.
				logs := []map[string]any{}
				for _, log := range result.Logs {
					topics := []string{}
					for _, topic := range log.Topics {
						topics = append(topics, boundaryHex(topic[:]))
					}
					logs = append(logs, map[string]any{"address": boundaryHex(log.Address[:]), "topics": topics, "data": boundaryHex(log.Data)})
				}
				var nativeUsed, nativeLeft any
				if len(quotes) != 0 {
					// Direct first native call only: derive from actual Main gas used.
					nativeUsed = result.GasUsed - 21272
					nativeLeft = cap - result.GasUsed
				}
				row := map[string]any{"period": period, "owner": owner, "supplied_nonce": suppliedNonce.String(), "effective_nonce": tx.Nonce.String(),
					"gas": cap, "gas_price": "0", "value": "0", "input": boundaryHex(tx.Input), "to": boundaryHex(contract[:]),
					"gas_used": result.GasUsed, "consensus_error": string(result.ConsensusErr), "execution_error": string(result.ExecutionErr),
					"parent_error": parentError, "return": boundaryHex(result.CodeRetval), "logs": logs, "quotes": quotes, "runs": runs,
					"quote_called": len(quotes) != 0, "run_called": len(runs) != 0,
					"intrinsic_gas": 21272, "native_gas_used": nativeUsed, "native_gas_left": nativeLeft,
					"ordered_reads": reads, "ordered_raw_writes": writes, "ordered_balance_effects": balances,
					"accounts": accounts, "prior_raw": priorRaw, "after_raw": afterRaw}
				if !reflect.DeepEqual(before, currentSignedSnapshot(seed, api)) {
					panic("boundary attempt mutated committed P1")
				}
				attempts = append(attempts, row)
			}
			if !reflect.DeepEqual(attempts[0], attempts[1]) {
				panic("boundary repeat differs")
			}
			cases = append(cases, attempts[0])
		}
	}
	after := currentSignedSnapshot(seed, api)
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "caller": boundaryHex(nativeSimulationSender[:]), "owner": boundaryHex(nativeSimulationDelegator[:]),
		"state_before": before, "state_after": after, "config": seed.config, "cases": cases, "repeat_identical": true, "state_unchanged_after_each": true,
		"scope": "canonical reward-only P1; literal H1 DryRunner and independent pending2 over reader1; no production/root/recovery acceptance"}); err != nil {
		panic(err)
	}
}
