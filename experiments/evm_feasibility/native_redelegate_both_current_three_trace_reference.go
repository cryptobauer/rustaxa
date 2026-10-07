// Actual count3-H1 retained-owner direct TraceRunner with real partial prefix/full target in period2.
package main

import (
	"bytes"
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
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

// Supplemental actual StateTransition execution is separate from default trace
// JSON. Passive copied native cache over committed H1 preserves deletions.
func retainedOwnerLiveWitness(seed nativeSimulationSeed, transactions []vm.Transaction) map[string]any {
	transition := nativeSimulationTransition(seed.database, &seed.config)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)})
	baseline := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(1)}
	capture := func() map[string]any {
		frozen := signedH1Frozen{baseline, transition.CurrentSourceCacheSnapshot()}
		raw := signedH1Raw(frozen)
		contract := *dpos.ContractAddress()
		for _, last := range []byte{0x31, 0x32} {
			validator := common.BytesToAddress([]byte{last})
			key := contract_storage.Stor_k_1([]byte{1}, validator[:], []byte{2})
			present, value := false, []byte{}
			frozen.GetAccountStorage(&contract, key, func(v []byte) {
				if len(v) > 0 {
					present = true
					value = common.CopyBytes(v)
				}
			})
			raw[common.Bytes2Hex(key[:])] = map[string]any{"present": present, "value": common.Bytes2Hex(value)}
		}
		accounts := map[string]any{}
		for _, actor := range []common.Address{nativeSimulationSender, nativeSimulationDelegator, contract} {
			account := transition.GetEvmState().GetAccount(&actor)
			accounts[common.Bytes2Hex(actor[:])] = map[string]string{"nonce": account.GetNonce().String(), "balance": account.GetBalance().String()}
		}
		return map[string]any{"raw": raw, "facts": signedH1Facts(frozen), "accounts": accounts}
	}
	nodeKeys := map[string]string{}
	for _, last := range []byte{0x31, 0x32} {
		validator := common.BytesToAddress([]byte{last})
		for _, block := range []byte{1, 2} {
			key := contract_storage.Stor_k_1([]byte{1}, validator[:], []byte{block})
			nodeKeys[common.Bytes2Hex([]byte{last, block})] = common.Bytes2Hex(key[:])
		}
	}
	before := capture()
	steps := []map[string]any{}
	for _, transaction := range transactions {
		request := traceRequest(transaction)
		result := transition.ExecuteTransaction(&transaction)
		if result.ConsensusErr != "" || result.ExecutionErr != "" {
			panic("live witness execution failed")
		}
		logs := []nativeSimulationLog{}
		for _, log := range result.Logs {
			topics := []string{}
			for _, topic := range log.Topics {
				topics = append(topics, common.Bytes2Hex(topic[:]))
			}
			logs = append(logs, nativeSimulationLog{Address: common.Bytes2Hex(log.Address[:]), Topics: topics, Data: common.Bytes2Hex(log.Data)})
		}
		steps = append(steps, map[string]any{"request": request, "gas_used": result.GasUsed, "logs": logs, "after": capture()})
	}
	return map[string]any{"period": 2, "node_keys": nodeKeys, "before": before, "steps": steps, "scope": "actual separate two-ExecuteTransaction live H2; passive cachecopy/H1fallback; no EndBlock/Commit or default-trace native-write claim"}
}

func main() {
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0x31}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationSender = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	x, y = secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0xa1}, 32))
	pub = append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationDelegator = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	seed, _ := seedCurrentSignedH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.TraceRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: nativeSimulationPeriod + 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000002, Difficulty: new(big.Int)}}
	before := currentSignedSnapshot(seed, api)
	address := *dpos.ContractAddress()
	tx := func(input []byte) vm.Transaction {
		return vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Lsh(big.NewInt(1), 512), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: input}
	}
	prefixTx := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 300))
	targetTx := tx(redelegateSimulationABI(common.BytesToAddress([]byte{0x31}), common.BytesToAddress([]byte{0x32}), 400))
	targetTx.Nonce = new(big.Int).Add(prefixTx.Nonce, big.NewInt(1))
	definitions := []struct {
		name            string
		prefix, targets []vm.Transaction
	}{
		{name: "partial_prefix_then_full_both_current_three", prefix: []vm.Transaction{prefixTx}, targets: []vm.Transaction{targetTx}},
	}
	cases := []map[string]any{}
	for _, definition := range definitions {
		result, diagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		repeated, repeatedDiagnostics := redelegateTrace(runner, block, definition.prefix, definition.targets)
		if !bytes.Equal(result, repeated) || diagnostics != repeatedDiagnostics {
			panic("trace did not repeat")
		}
		prefix, targets := []map[string]any{}, []map[string]any{}
		for _, transaction := range definition.prefix {
			prefix = append(prefix, traceRequest(transaction))
		}
		for _, transaction := range definition.targets {
			targets = append(targets, traceRequest(transaction))
		}
		cases = append(cases, map[string]any{"name": definition.name, "prefix": prefix, "targets": targets, "result": json.RawMessage(result), "diagnostics": diagnostics})
	}
	witnessSeed, _ := seedCurrentSignedH1()
	witnessSeedBefore := currentSignedSnapshot(witnessSeed, new(dpos.API).Init(witnessSeed.config))
	if !reflect.DeepEqual(before, witnessSeedBefore) {
		panic("separate live witness H1 identity differs")
	}
	witness := retainedOwnerLiveWitness(witnessSeed, []vm.Transaction{prefixTx, targetTx})
	witness["seed_root"] = witnessSeedBefore["root"]
	witness["seed_row_count"] = len(witnessSeedBefore["seed_rows"].([]nativeSimulationSeedRow))
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("trace changed committed seed")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "period": uint64(block.Number), "cases": cases, "live_witness": witness, "scope": "actual one direct retained-owner count3-H1 structured TraceRunner; complete canonical two-signer108-row H1; real partial300 prefix and full400 target share period2 state; no delayed eligibility, nested target, OE or RPC claim"}); err != nil {
		panic(err)
	}
}
