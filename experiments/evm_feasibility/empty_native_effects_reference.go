// Executes only a cold immutable-Go Init/BeginBlock/EndBlock/Close witness.
// The fixture port carries previously authenticated bytes; it is not a database,
// proof verifier, complete state snapshot, transaction replay or root executor.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"math/big"
	"os"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/Taraxa-project/taraxa-evm/taraxa/util/keccak256"
)

const head uint64 = 25706949

type input struct {
	ParentPeriod    uint64                   `json:"parent_period"`
	ParentRoot      string                   `json:"parent_root"`
	DposPhysicalRLP string                   `json:"dpos_physical_rlp"`
	JailPhysicalRLP string                   `json:"jail_physical_rlp"`
	Config          chain_config.ChainConfig `json:"config"`
	Author          string                   `json:"author"`
	Timestamp       uint64                   `json:"timestamp"`
	GasLimit        uint64                   `json:"gas_limit"`
}

type read struct {
	Column byte   `json:"column"`
	Key    string `json:"key_hex"`
	Bytes  int    `json:"bytes"`
}

type fixturePort struct {
	descriptor                                     state_db.StateDescriptor
	accountKey                                     common.Hash
	jailKey                                        common.Hash
	account                                        []byte
	reads                                          []read
	accountReads, jailReads, begins, puts, commits int
}

// Get permits precisely one read of each supplied parent target. Every other
// key/column, repeat, or missing value is an explicit fixture contract failure.
func (p *fixturePort) Get(column state_db.Column, key *common.Hash, apply func([]byte)) {
	if key == nil {
		panic("fixture Get nil key")
	}
	var value []byte
	switch {
	case column == state_db.COL_main_trie_value && *key == p.accountKey:
		p.accountReads++
		if p.accountReads != 1 {
			panic("repeated parent DPoS read")
		}
		value = p.account
	case column == state_db.COL_acc_trie_value && *key == p.jailKey:
		p.jailReads++
		if p.jailReads != 1 {
			panic("repeated parent jail read")
		}
		value = []byte{0xc0}
	default:
		panic(fmt.Sprintf("forbidden fixture Get column=%d key=%x", column, *key))
	}
	if len(p.reads) >= 2 {
		panic("fixture read cap exceeded")
	}
	p.reads = append(p.reads, read{Column: column, Key: hex.EncodeToString(key[:]), Bytes: len(value)})
	apply(common.CopyBytes(value))
}
func (p *fixturePort) Put(state_db.Column, *common.Hash, []byte) {
	p.puts++
	panic("fixture forbids all Put")
}
func (p *fixturePort) Commit(common.Hash) error                         { p.commits++; panic("fixture forbids Commit") }
func (p *fixturePort) GetNumber() types.BlockNum                        { return head }
func (p *fixturePort) GetCommittedDescriptor() state_db.StateDescriptor { return p.descriptor }
func (p *fixturePort) BeginPendingBlock() state_db.PendingBlockState {
	p.begins++
	if p.begins != 1 {
		panic("fixture permits one cold block only")
	}
	return p
}

// Historical reader construction is permitted as a label; every attempted
// delayed/history read is denied rather than substituting parent fixture bytes.
type deniedHistoryReader struct{ period types.BlockNum }

func (r deniedHistoryReader) Get(state_db.Column, *common.Hash, func([]byte)) {
	panic(fmt.Sprintf("history read forbidden at period %d", r.period))
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}

func witness(in input) map[string]any {
	if in.ParentPeriod != head-1 || in.ParentRoot != "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2" || in.JailPhysicalRLP != "c0" {
		panic("fixture parent identity/value drift")
	}
	account, err := hex.DecodeString(in.DposPhysicalRLP)
	must(err)
	if len(account) == 0 {
		panic("missing fixture parent account")
	}
	dposAddress := common.HexToAddress("0xfe")
	slashingAddress := common.HexToAddress("0xee")
	key := common.HexToHash("0x02")
	p := &fixturePort{descriptor: state_db.StateDescriptor{BlockNum: head - 1, StateRoot: common.HexToHash(in.ParentRoot)}, accountKey: *keccak256.Hash(dposAddress[:]), jailKey: *keccak256.Hash(slashingAddress[:], keccak256.Hash(key[:])[:]), account: account, reads: make([]read, 0)}
	api := new(dpos.API).Init(in.Config)
	readerPeriods := make([]types.BlockNum, 0)
	factory := func(period types.BlockNum) contract_storage.StorageReader {
		if period > head-1 {
			panic("future fixture reader selection")
		}
		// Reader creation carries a period label only. Any actual unsupported
		// delayed/history key read is denied by the exact two-target port.
		readerPeriods = append(readerPeriods, period)
		return state_db.ExtendedReader{Reader: deniedHistoryReader{period: period}}
	}
	transition := new(state_transition.StateTransition).Init(p, func(types.BlockNum) *big.Int { panic("block hash callback forbidden") }, api,
		func(period types.BlockNum) dpos.Reader { return api.NewDelayedReader(period, factory) },
		func(period types.BlockNum) slashing.Reader { return api.NewSlashingReader(period, factory) },
		&in.Config, state_transition.Opts{EVMState: state_evm.Opts{NumTransactionsToBuffer: 1}, Trie: state_transition.TrieSinkOpts{}})
	transition.BeginBlock(&vm.BlockInfo{Author: common.HexToAddress(in.Author), Time: in.Timestamp, GasLimit: in.GasLimit, Difficulty: new(big.Int)})
	transition.EndBlock()
	transition.Close() // Drains all queues before any success claim.
	if p.accountReads != 1 || p.jailReads != 1 || p.begins != 1 || p.puts != 0 || p.commits != 0 || state_transition.EmptyEffectsMutationAttempts() != 0 {
		panic("cold fixture operation contract failed")
	}
	return map[string]any{"period": head, "parent_period_label": in.ParentPeriod, "parent_root_label": in.ParentRoot, "construction": "one cold StateTransition", "operations": []string{"Init", "BeginBlock", "EndBlock", "Close"}, "reads": p.reads, "read_count": len(p.reads), "historical_reader_labels": readerPeriods, "historical_reads_allowed": false, "backend_put_attempts": p.puts, "commit_attempts": p.commits, "trie_mutation_attempts": state_transition.EmptyEffectsMutationAttempts(), "ordinary_transactions_executed": 0, "reward_distributions_executed": 0, "prepare_commit_called": false, "root_derived": false, "fixture_port_authenticated": false, "complete_snapshot_claim": false, "warm_constructor_claim": false, "producer_qualified": false}
}

// Negative mutation controls run in separate processes and never contaminate
// the successful witness's cold constructor or operation counters.
func control(kind string) (result map[string]any) {
	defer func() {
		recovered := recover()
		if recovered == nil || fmt.Sprint(recovered) != "empty-effects observer rejects TrieSink mutation attempt" || state_transition.EmptyEffectsMutationAttempts() != 1 {
			panic("mutation observer negative control did not reject")
		}
		result = map[string]any{"control": kind, "rejected": true, "mutation_attempts": 1, "reason": fmt.Sprint(recovered)}
	}()
	sink := new(state_transition.TrieSink)
	address := common.HexToAddress("0xfe")
	switch kind {
	case "start":
		sink.StartMutation(&address)
	case "delete":
		sink.Delete(&address)
	default:
		panic("unknown mutation control")
	}
	return nil
}

func main() {
	file := flag.String("input", "", "validated fixture JSON")
	mode := flag.String("control", "", "isolated start/delete negative control")
	flag.Parse()
	var output map[string]any
	if *mode != "" {
		output = control(*mode)
	} else {
		raw, err := os.ReadFile(*file)
		must(err)
		var in input
		decoder := json.NewDecoder(bytes.NewReader(raw))
		decoder.DisallowUnknownFields()
		must(decoder.Decode(&in))
		output = witness(in)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(output))
}
