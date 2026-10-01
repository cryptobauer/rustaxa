// Synthetic genesis setup and cold period-one EndBlock. No trie/root execution.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	dpos_sol "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/solidity"
	slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition/op_stack"
	"github.com/Taraxa-project/taraxa-evm/taraxa/util/keccak256"
	"io"
	"math/big"
	"os"
	"reflect"
	"sort"
)

var (
	currentValidatorOne  = common.HexToAddress("0x0000000000000000000000000000000000000031")
	currentValidatorTwo  = common.HexToAddress("0x0000000000000000000000000000000000000041")
	currentDelegatorOne  = common.HexToAddress("0x0000000000000000000000000000000000000032")
	currentDelegatorTwo  = common.HexToAddress("0x0000000000000000000000000000000000000042")
	currentMissingAuthor = common.HexToAddress("0x0000000000000000000000000000000000000051")
	currentDpos          = common.HexToAddress("0x00000000000000000000000000000000000000fe")
	currentSlashing      = common.HexToAddress("0x00000000000000000000000000000000000000ee")
)

type observation struct {
	Column  state_db.Column `json:"column"`
	Key     string          `json:"key"`
	Value   string          `json:"value"`
	Present bool            `json:"present"`
}
type rawRow struct {
	Address string `json:"address"`
	Key     string `json:"key"`
	Value   string `json:"value"`
}
type port struct {
	rows                                           map[state_db.Column]map[common.Hash][]byte
	accounts                                       map[common.Address]state_db.Account
	raw                                            map[common.Address]map[common.Hash][]byte
	reads                                          []observation
	measuring                                      bool
	setupAccounts, setupRaw, puts, commits, begins int
}

func newPort() *port {
	return &port{rows: make(map[state_db.Column]map[common.Hash][]byte), accounts: make(map[common.Address]state_db.Account), raw: make(map[common.Address]map[common.Hash][]byte), reads: make([]observation, 0)}
}
func (p *port) seed(c state_db.Column, k common.Hash, v []byte) {
	if p.rows[c] == nil {
		p.rows[c] = make(map[common.Hash][]byte)
	}
	p.rows[c][k] = common.CopyBytes(v)
}
func (p *port) Get(c state_db.Column, k *common.Hash, cb func([]byte)) {
	v, ok := p.rows[c][*k]
	if p.measuring {
		p.reads = append(p.reads, observation{c, hex.EncodeToString(k[:]), hex.EncodeToString(v), ok})
	}
	if ok {
		cb(common.CopyBytes(v))
	}
}
func (p *port) Put(state_db.Column, *common.Hash, []byte) {
	p.puts++
	panic("synthetic lifecycle Put forbidden")
}
func (p *port) Commit(common.Hash) error  { p.commits++; panic("synthetic lifecycle Commit forbidden") }
func (p *port) GetNumber() types.BlockNum { return 1 }
func (p *port) GetCommittedDescriptor() state_db.StateDescriptor {
	return state_db.StateDescriptor{BlockNum: 0, StateRoot: common.HexToHash("0x1234")}
}
func (p *port) BeginPendingBlock() state_db.PendingBlockState { p.begins++; return p }

// Setup output persists account/raw values directly. It has no trie or root.
type setupMutation struct {
	p       *port
	address common.Address
}

func (p *port) StartMutation(a *common.Address) state_evm.AccountMutation {
	return setupMutation{p, *a}
}
func (p *port) Delete(*common.Address) { panic("unexpected setup delete") }
func (m setupMutation) Update(c state_evm.AccountChange) {
	m.p.setupAccounts++
	m.p.accounts[m.address] = c.Account
	encoded, _ := c.Account.EncodeForTrie()
	m.p.seed(state_db.COL_main_trie_value, *keccak256.Hash(m.address[:]), encoded)
	if m.p.raw[m.address] == nil {
		m.p.raw[m.address] = make(map[common.Hash][]byte)
	}
	for k, v := range c.RawStorageDirty {
		m.p.setupRaw++
		m.p.raw[m.address][k] = common.CopyBytes(v)
		m.p.seed(state_db.COL_acc_trie_value, *keccak256.Hash(m.address[:], keccak256.Hash(k[:])[:]), v)
	}
	if len(c.StorageDirty) != 0 {
		panic("unexpected genesis EVM storage")
	}
}
func (m setupMutation) Commit() { panic("setup commit forbidden") }
func (p *port) rawRows() []rawRow {
	r := make([]rawRow, 0)
	for a, rows := range p.raw {
		for k, v := range rows {
			r = append(r, rawRow{hex.EncodeToString(a[:]), hex.EncodeToString(k[:]), hex.EncodeToString(v)})
		}
	}
	sort.Slice(r, func(i, j int) bool { return r[i].Address+r[i].Key < r[j].Address+r[j].Key })
	return r
}
func must(e error) {
	if e != nil {
		panic(e)
	}
}

// Full bounded input contract. It constrains setup and caller orchestration;
// it supplies no observed output or expected effect to the engine.
const syntheticInputContract = `{
  "schema": 1,
  "synthetic": true,
  "period": 1,
  "parent_period": 0,
  "parent_root_label": "0x1234",
  "block_author": "0x0000000000000000000000000000000000000051",
  "timestamp": 0,
  "gas_limit": 1000000,
  "config": {
    "EVMChainConfig": {
      "chainId": 1
    },
    "GenesisBalances": {
      "0x0000000000000000000000000000000000000032": 2000,
      "0x0000000000000000000000000000000000000042": 3000
    },
    "DPOS": {
      "EligibilityBalanceThreshold": 100,
      "VoteEligibilityBalanceStep": 10,
      "ValidatorMaximumStake": 1000000,
      "MinimumDeposit": 1,
      "MaxBlockAuthorReward": 10,
      "DagProposersReward": 50,
      "CommissionChangeDelta": 0,
      "CommissionChangeFrequency": 0,
      "DelegationDelay": 0,
      "DelegationLockingPeriod": 0,
      "BlocksPerYear": 10,
      "YieldPercentage": 1,
      "InitialValidators": [
        {
          "Address": "0x0000000000000000000000000000000000000031",
          "Owner": "0x0000000000000000000000000000000000000032",
          "VrfKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          "Commission": 100,
          "Endpoint": "",
          "Description": "",
          "Delegations": {
            "0x0000000000000000000000000000000000000032": 1000
          }
        },
        {
          "Address": "0x0000000000000000000000000000000000000041",
          "Owner": "0x0000000000000000000000000000000000000042",
          "VrfKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          "Commission": 2500,
          "Endpoint": "",
          "Description": "",
          "Delegations": {
            "0x0000000000000000000000000000000000000042": 2000
          }
        }
      ]
    },
    "Hardforks": {
      "FixRedelegateBlockNum": 18446744073709551615,
      "Redelegations": null,
      "RewardsDistributionFrequency": {
        "0": 2
      },
      "MagnoliaHf": {
        "BlockNum": 0,
        "JailTime": 0
      },
      "PhalaenopsisHfBlockNum": 0,
      "FixClaimAllBlockNum": 0,
      "AspenHf": {
        "BlockNumPartOne": 0,
        "BlockNumPartTwo": 1,
        "MaxSupply": 6000,
        "GeneratedRewards": 0
      },
      "FicusHf": {
        "BlockNum": 0,
        "PillarBlocksInterval": 0,
        "BridgeContractAddress": "0x0000000000000000000000000000000000000000"
      },
      "CornusHf": {
        "BlockNum": 0,
        "DelegationLockingPeriod": 0,
        "DagGasLimit": 0,
        "PbftGasLimit": 0
      },
      "SoleiroliaHf": {
        "BlockNum": 18446744073709551615,
        "TrxMinGasPrice": 0,
        "TrxMaxGasLimit": 0
      },
      "CactiHf": {
        "BlockNum": 18446744073709551615,
        "LambdaMin": 0,
        "LambdaMax": 0,
        "LambdaDefault": 0,
        "LambdaChangeInterval": 0,
        "LambdaChange": 0,
        "BlockPropagationMin": 0,
        "BlockPropagationMax": 0,
        "ConsensusDelay": 0,
        "DelegationLockingPeriod": 0,
        "JailTime": 0
      }
    }
  },
  "rewards_fact": {
    "period": 1,
    "block_author": "0x0000000000000000000000000000000000000051",
    "blocks_per_year": 10,
    "caller_eligible_vote_count": 0,
    "planner_eligible_vote_count": 10,
    "transactions": [],
    "dag_blocks": [],
    "cert_votes": []
  },
  "rust_genesis_accounts_after_delegation": {
    "0000000000000000000000000000000000000032": 1000,
    "0000000000000000000000000000000000000042": 1000
  },
  "go_setup": "ApplyGenesis, then pinned genesis Cornus DPoS bytecode and op_stack.OpPrecompiles through value-only account/raw collector; no trie/root",
  "synthetic_root_authenticated": false,
  "producer_qualified": false,
  "real_window_gate_closed": false
}`

func validateManifest(raw []byte) error {
	decode := func(input []byte) (any, error) {
		decoder := json.NewDecoder(bytes.NewReader(input))
		decoder.UseNumber()
		var value any
		if err := decoder.Decode(&value); err != nil {
			return nil, err
		}
		var trailing any
		if err := decoder.Decode(&trailing); err != io.EOF {
			return nil, fmt.Errorf("trailing manifest data")
		}
		return value, nil
	}
	actual, err := decode(raw)
	if err != nil {
		return err
	}
	expected, err := decode([]byte(syntheticInputContract))
	if err != nil {
		return err
	}
	if !reflect.DeepEqual(actual, expected) {
		return fmt.Errorf("synthetic input contract drift")
	}
	return nil
}

func main() {
	if len(os.Args) != 2 && !(len(os.Args) == 3 && os.Args[2] == "--validate-only") {
		panic("shared synthetic manifest required")
	}
	raw, err := os.ReadFile(os.Args[1])
	must(err)
	must(validateManifest(raw))
	if len(os.Args) == 3 {
		return
	}
	var manifest struct {
		Config       chain_config.ChainConfig `json:"config"`
		Period       uint64                   `json:"period"`
		ParentPeriod uint64                   `json:"parent_period"`
		ParentRoot   string                   `json:"parent_root_label"`
		Author       common.Address           `json:"block_author"`
		Timestamp    uint64                   `json:"timestamp"`
		GasLimit     uint64                   `json:"gas_limit"`
	}
	must(json.Unmarshal(raw, &manifest))
	if manifest.Period != 1 || manifest.ParentPeriod != 0 || manifest.ParentRoot != "0x1234" || manifest.Author != currentMissingAuthor {
		panic("synthetic scope drift")
	}
	cfg := manifest.Config
	p := newPort()
	api := new(dpos.API).Init(cfg)
	factory := func(types.BlockNum) storage.StorageReader { return state_db.ExtendedReader{Reader: p} }
	// Complete independent Go genesis construction, with no StateTransition genesis
	// Commit: direct ApplyGenesis and a value-only setup output.
	state := new(state_evm.TransitionState)
	state.Init(state_evm.Opts{NumTransactionsToBuffer: 1})
	state.SetInput(state_db.ExtendedReader{Reader: p})
	evm := new(vm.EVM)
	evm.Init(func(types.BlockNum) *big.Int { panic("block hash forbidden") }, state, vm.DefaultOpts(), cfg.EVMChainConfig, vm.Config{})
	evm.SetBlock(&vm.Block{Number: 0, BlockInfo: vm.BlockInfo{Difficulty: new(big.Int)}}, cfg.Hardforks.Rules(0))
	for a, b := range cfg.GenesisBalances {
		address := a
		state.GetAccount(&address).AddBalance(b)
	}
	contract := api.NewContract(storage.EVMStateStorage{state}, api.NewReader(0, factory), evm)
	must(contract.ApplyGenesis(state.GetAccount))
	// Genesis hardfork account setup, as in the pinned applyHFChanges path.
	state.GetAccount(&currentDpos).SetCode(dpos_sol.CornusDposImplBytecode)
	for address, code := range op_stack.OpPrecompiles {
		address := address
		state.GetAccount(&address).SetCode(code)
	}
	state.CommitTransaction(p)
	setupRows := p.rawRows()
	setupAccounts := make(map[string]string)
	for a, account := range p.accounts {
		setupAccounts[hex.EncodeToString(a[:])] = account.Balance.String()
	}
	p.measuring = true
	rawWrites := make([]rawRow, 0)
	state_evm.SetMixedPeriodRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		rawWrites = append(rawWrites, rawRow{hex.EncodeToString(address[:]), hex.EncodeToString(key[:]), hex.EncodeToString(value)})
	})
	transition := new(state_transition.StateTransition).Init(p, func(types.BlockNum) *big.Int { panic("block hash forbidden") }, api, func(n types.BlockNum) dpos.Reader { return api.NewDelayedReader(n, factory) }, func(n types.BlockNum) slashing.Reader { return api.NewSlashingReader(n, factory) }, &cfg, state_transition.Opts{EVMState: state_evm.Opts{NumTransactionsToBuffer: 1}})
	transition.BeginBlock(&vm.BlockInfo{Author: manifest.Author, Time: manifest.Timestamp, GasLimit: manifest.GasLimit, Difficulty: new(big.Int)})
	transition.EndBlock()
	transition.Close()
	state_evm.SetMixedPeriodRawWriteObserver(nil)
	if p.puts != 0 || p.commits != 0 || p.begins != 1 || state_transition.EmptyEffectsMutationAttempts() != 0 {
		panic("synthetic lifecycle effect contract failed")
	}
	// Frequency and empty rewards are explicit caller orchestration inputs; Go
	// EndBlock does not itself invoke the C++/Rust rewards-stats planner.
	result := map[string]any{"synthetic": true, "period": manifest.Period, "distribution_frequency": cfg.Hardforks.RewardsDistributionFrequency[0], "nonboundary": manifest.Period%uint64(cfg.Hardforks.RewardsDistributionFrequency[0]) != 0, "reward_distributions_requested": 0, "operations": []string{"Init", "BeginBlock", "EndBlock", "Close"}, "config": cfg, "setup_account_updates": p.setupAccounts, "setup_raw_updates": p.setupRaw, "genesis_accounts_after_delegation": setupAccounts, "genesis_raw_rows": setupRows, "reads": p.reads, "ordered_raw_writes": rawWrites, "backend_put_attempts": p.puts, "commit_attempts": p.commits, "trie_mutation_attempts": state_transition.EmptyEffectsMutationAttempts(), "prepare_commit_called": false, "root_derived": false, "parent_root_label": "1234 (synthetic nonempty label; no authenticated root)", "producer_qualified": false, "real_window_gate_closed": false}
	if fmt.Sprint(result["distribution_frequency"]) != "2" {
		panic("frequency drift")
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(result))
}
