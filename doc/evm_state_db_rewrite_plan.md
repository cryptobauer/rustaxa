# EVM and state database modernization proposal

Status: evaluation and proposed work only; no implementation or production routing is authorized by this document.

Evaluated on 2026-09-12, branch `feat/rust/evm-state-db`, repository revision
`922dd662502a0f932c033c7de142737b472af9a1`, EVM submodule revision `bb0ab67c8`.
This proposal supplements [PLAN.md](../PLAN.md); it does not change its current ownership boundaries.

## Recommendation

Pursue a modern Rust execution stack with REVM as the preferred engine, subject to a compatibility feasibility gate.
Retain native Rust consensus/FinalChain orchestration and extend the existing Rust storage infrastructure to own
`state_db/`. Keep RocksDB initially. Adopt Ethereum ecosystem components selectively rather than replacing the node
with Reth or translating the entire Go interpreter into Rust.

Treat three decisions separately:

1. Replace the implementation while preserving Taraxa execution behavior and canonical roots.
2. Replace the physical state storage representation while preserving those same roots and recovery guarantees.
3. Introduce new Ethereum protocol features through separately specified Taraxa network upgrades.

The task owner confirmed existing-network compatibility first. The initial migration must preserve historical
execution and currently valid behavior on the supported Taraxa networks, including canonical roots, receipts, hashes,
and unusual execution semantics. Changing protocol rules to accommodate a library is outside this migration's scope.
Ethereum feature upgrades remain a separate, later decision requiring explicit authorization and network activation.
Modern library code does not require activating its newest Ethereum rules. Conversely, replacing an implementation
does not make existing Taraxa rules equivalent to a named Ethereum fork.

The DAG is not the main engine-integration obstacle: the existing Rust application supplies ordered execution inputs.
The difficult coupling is in Taraxa transaction semantics, native contracts, state representation, and historical fork
behavior. REVM is promising because its framework exposes handlers, instructions, precompiles, journals, and database
interfaces, but the audit below identifies cases that may exceed convenient extension points.
See [REVM architecture](https://bluealloy.github.io/revm/architecture.html).

## What the repository actually contains

| Area | Current ownership and consequence |
| --- | --- |
| DAG/PBFT and FinalChain lifecycle | Rust orders execution, plans system transactions/rewards, validates results, approves state commits, recovers pending publication, and publishes canonical indexes. Reuse this work. |
| Concrete execution | `ExternalEvmStateOwner` owns C++ `StateAPI`, which calls the modified Go submodule. It executes every finalized period, including periods consisting of native operations. Native semantic coverage alone does not mean the Go execution path is removable. |
| Canonical state root | The concrete post-rewards root is authoritative. Rust account/DPoS snapshots are not a replacement for the complete arbitrary-contract state trie. |
| Native contracts | Rust already implements substantial DPoS/slashing/reward semantics and independently replays concrete invocation transcripts. The concrete Go contracts still write the root-producing storage representation. |
| Persistence | Rust owns the main `db/`, including FinalChain rows and publication markers. Go owns the separate sibling `state_db/`, its versioned values, trie/code data, descriptors, and concrete provenance. |
| Go dependencies | `go.mod` declares Go 1.22 and go-ethereum 1.13.10, but execution uses the submodule's own `core/vm`. Updating the external geth dependency alone does not modernize that interpreter. |
| Queries and operations | Arbitrary calls, gas estimates, code/storage reads, tracing, pruning, snapshots, and reopen behavior remain part of the replacement scope. Consensus execution success alone is insufficient. |

Primary local entry points:

- [Application execution port](../rust/crates/rustaxa-consensus/src/consensus_application_runtime.rs),
  `ConsensusExecutionPort`, and [execution lifecycle](../rust/crates/rustaxa-consensus/src/final_chain_execution.rs).
- [FinalChain native execution and concrete replay](../rust/crates/rustaxa-consensus/src/final_chain.rs),
  especially `external_evm_concrete_projection` and the existing native mutation kernels.
- [Concrete projection/provenance codecs](../rust/crates/rustaxa-consensus/src/concrete_state_projection.rs).
- [Concrete state owner](../libraries/core_libs/consensus/src/application/external_evm_state_owner.cpp) and
  [StateAPI](../libraries/core_libs/consensus/include/final_chain/state_api.hpp).
- [Go execution](../submodules/taraxa-evm/core/vm/evm.go),
  [state transitions](../submodules/taraxa-evm/taraxa/state/state_transition/state_transition.go), and
  [state database](../submodules/taraxa-evm/taraxa/state/state_db_rocksdb/db.go).

## Compatibility issues that determine feasibility

### Transaction envelope and integer widths

`EVM.Main` charges gas before checking a low nonce, permits nonce skipping, and handles transactions unable to afford
their gas cap. Cornus changes nonce advancement on several failure paths. Contract creation derives addresses using
the submitted nonce, and the zero-address system sender has special treatment. These are consensus rules, not ordinary
Ethereum mempool validation. A default Ethereum transaction rejection can therefore disagree on balance, nonce, gas,
receipt, and state root simultaneously. Merely disabling REVM nonce validation is insufficient.

Taraxa uses arbitrary-width account nonces; [FinalChainNonce](../rust/crates/rustaxa-types/src/final_chain.rs) explicitly
preserves values above U256. REVM's documented [AccountInfo](https://docs.rs/revm-state/latest/revm_state/struct.AccountInfo.html)
uses `u64` for nonce and U256 for balance. This is a hard compatibility mismatch, including account emptiness,
creation collision checks, CREATE address derivation, internal creation, and nonce increment/rollback.

The usual argument that nobody can increment a nonce that far is inapplicable when nonce skipping is allowed.
[Ethereum's nonce cap](https://eips.ethereum.org/EIPS/eip-2681) cannot silently become a Taraxa validation rule.
Preserving a wide nonce in a side map is only a candidate solution: every engine read and mutation must use it
correctly. Clamping, truncation, and rejecting otherwise-valid Taraxa transactions are unacceptable shortcuts.
Also audit Go big-integer balance arithmetic against protocol-reachable bounds before assuming U256 conversion is safe.

### Taraxa forks are combinations of rules

The checked-in engine selects Californicum/Ficus/Cacti instruction and precompile tables while retaining
`GasTableCalifornicum`. It contains PUSH0, adds MCOPY at Ficus, and adds standard transient-storage opcode addresses
at Cacti. The old `0xb3`/`0xb4` transient opcodes remain inherited in that table. Execution still caps refunds at
half of gas consumed. These facts rule out selecting Cancun or another complete Ethereum spec as a parity shortcut.

Build an explicit period-to-rules matrix covering instruction availability, gas schedules, refunds, SSTORE,
SELFDESTRUCT, code/initcode restrictions, block environment, precompile registration, and system code installation.
Use [instruction tables](../submodules/taraxa-evm/core/vm/jump_table.go),
[gas rules](../submodules/taraxa-evm/core/vm/gas.go), and
[hardfork mutations](../submodules/taraxa-evm/taraxa/state/state_transition/state_hardforks.go).
Record actual configured activation periods separately for each target network; this evaluation did not establish live
network activation status.

### Native contracts and rollback

The [native contract storage adapter](../submodules/taraxa-evm/taraxa/state/contracts/storage/evm_state_storage_adapter.go)
calls `SetStateRawIrreversibly`. The Rust concrete transcript explicitly distinguishes own-frame and parent-frame
reverts where those writes survive. A conventional journal that rolls back every write would change consensus behavior.
Balances, nonces, logs, ordinary storage, and native raw storage must each follow their observed rollback rules.

Integrate the existing Rust DPoS/slashing kernels into actual EVM call frames, including calls from arbitrary bytecode,
CALLCODE/DELEGATECALL/STATICCALL context, gas, delayed eligibility snapshots, same-block mutations, and parent reverts.
Top-level transaction dispatch alone is insufficient. Shared state must expose native effects to subsequent bytecode
and transactions in the required order; post-block reconciliation is too late.

An additional audit concern: `TransitionState.SetTransientState` says it journals changes, but its implementation
directly updates the transient map without registering a revert. Add differential cases for nested transient writes
and reverts before assuming standard EIP-1153 behavior. Treat this as an observed code discrepancy needing a fixture,
not a newly authorized fix to the reference implementation.

The [precompile tables](../submodules/taraxa-evm/core/vm/contracts.go) also change BLS address mappings between Ficus
and Cacti and include P-256 and Falcon verification in Cacti. Inventory addresses, input acceptance, errors, output,
and gas independently of library algorithm names. Prefer maintained crypto implementations after exact vector parity;
the Falcon implementation choice remains open. Preserve installed DPoS and OP-related bytecode exactly at fork events.

### State commitments and physical storage

The [account codec](../submodules/taraxa-evm/taraxa/state/state_db/main_trie.go) stores a five-field account record,
including code size, but hashes a four-field account leaf with normalized empty storage/code hashes. The
[storage codec](../submodules/taraxa-evm/taraxa/state/state_db/account_trie.go) hashes RLP-wrapped raw bytes.
Native contract storage permits byte arrays, not just ordinary EVM U256 slots. The
[trie writer](../submodules/taraxa-evm/taraxa/state/state_transition/trie_sink.go) handles both paths.

Preserve exact key hashing, leaf bytes, deletion behavior, empty-root constants, embedded-node rules, and roots.
The physical database representation can change independently. Do not serialize the Rust DPoS snapshot directly into
the state trie or pass native raw values through U256 codecs.

The current database has numbered column families, versioned account/storage values keyed by hash plus big-endian
period, latest-value views, code/node data, configuration changes, and lifecycle metadata. Historical reads use the
latest version at or before the requested period. A simple address-to-account key/value store would lose required
history and pruning behavior. Migration must not assume address or storage-key preimages can be recovered from hashes.

## Options

| Option | Benefits | Costs and limits | Assessment |
| --- | --- | --- | --- |
| Maintain/upgrade the Go fork | Smallest immediate integration change; preserves a useful reference | Continues Go/CGO/C++ boundaries and bespoke interpreter maintenance; upstream geth upgrade is still a semantic port | Contingency if Rust engine feasibility fails |
| Translate the Go EVM and state stack into Rust | Maximum control over historical semantics | Rustaxa becomes responsible for interpreter correctness, optimization, and future Ethereum changes | Last resort for bounded legacy components, not preferred overall strategy |
| REVM plus Taraxa execution policy and Rust state storage | Reuses a maintained Rust engine and existing Rust consensus/storage; separates future Ethereum changes | Wide nonces, mixed fork rules, raw native storage, and rollback may require substantial customization | Preferred, conditional on the feasibility gate |
| Reth SDK/full execution stack | Reusable node, execution, provider, and RPC components | Adopting broad provider/block/pool architecture expands scope and overlaps already-migrated Taraxa responsibilities | Select individual components only where they simplify a demonstrated requirement |
| evmone through EVMC with a Rust host | Independent maintained interpreter; host separation is useful | Interpreter is C++; host/state work remains; custom gas/opcode behavior still needs assessment | Credible second engine candidate, especially if REVM state assumptions prove too restrictive |

REVM describes both Ethereum execution and variant-framework use and is used by Reth and Foundry:
[REVM project](https://github.com/bluealloy/revm). Reth supports customizable execution components:
[Reth SDK](https://reth.rs/sdk/), [EVM component](https://reth.rs/sdk/node-components/evm/).
evmone is a standalone C++ EVM implementing EVMC:
[evmone project](https://github.com/ipsilon/evmone).
These establish architectural options, not measured Taraxa performance or proven compatibility.

Pin a mutually compatible release set and Rust toolchain during the feasibility work. Do not select moving `main`
dependencies or promise that the newest release is the best baseline. REVM documents that its MSRV can change.
Keep any required patch set small, documented, and exercised against subsequent upstream releases. If preserving
Taraxa requires maintaining a second interpreter inside REVM, reevaluate this option rather than calling it a thin adapter.

## Proposed ownership and dependency shape

```text
Existing Rust consensus application / FinalChain
  ordering, fork policy, system/rewards planning, publication, recovery
                         |
             existing typed execution boundary
                         |
                 Rust execution adapter
                  /                 \
       REVM integration         existing Rust native kernels
                  \                 /
             staged execution state and effects
                         |
          Rust state database + commitment codec
                         |
             RocksDB state_db/ persistence
```

- Introduce a focused `rustaxa-evm` crate for engine integration, Taraxa transaction handling, rule selection,
  frame/native-contract integration, and inspectors. Give it narrow state/native-contract ports; keep REVM types
  internal and retain existing canonical transaction bytes and domain newtypes at application boundaries.
- Extend `rustaxa-storage` with the concrete state database and codec infrastructure, reusing its RocksDB dependency
  and infrastructure conventions. Keep `db/` and `state_db/` distinct initially; this does not imply combining their
  schemas or pretending a cross-database write is one atomic batch.
- Reuse existing Rust DPoS, reward graph, slashing, and execution kernels. Expose the minimum staged-state operations
  needed for nested frames. The existing crate dependencies already run from consensus to storage: define engine
  ports so that wiring does not create a consensus/engine dependency cycle. Extract shared domain units only where
  necessary; do not copy the kernels or have EVM calls reenter the locked application root.
- Model ordinary rollback and Taraxa irreversible native effects explicitly. Preserve transient-state quirks until
  a separately activated protocol change. One coherent execution view need not mean one uniform rollback rule.
- Keep result identity, intermediate roots, post-rewards root, markers, commit approval, and head-last publication.
  Applying REVM's returned state to an in-memory overlay is not the durable state commit.
- Preserve exact public query behavior through bounded Rust query adapters. At cutover, retire the concrete C++/Go
  leaves and their last callers; do not rebuild a C++ FinalChain facade or add new C++ orchestration.

For MPT work, evaluate the existing `triehash` dependency as a small reference calculator and `alloy-trie` for reusable
root/proof machinery. [Alloy trie](https://github.com/alloy-rs/trie) is not a complete historical state database.
Its [raw-leaf hash builder](https://raw.githubusercontent.com/alloy-rs/trie/main/src/hash_builder/mod.rs) is a candidate
for custom encoded leaves; convenience Ethereum account/slot types must not narrow Taraxa values. Demonstrate root
parity before adopting it, and separately design incremental updates so execution does not rebuild the entire trie
after every transaction. Consider [alloy-evm](https://github.com/alloy-rs/evm) only if its abstraction reduces adapter
code after the Taraxa handler is understood. A repository-wide migration from `ethereum-types` to Alloy is unnecessary.

## Delivery sequence and decision gates

Each stage below describes future implementation work. None was performed as part of this planning task.

| Stage | Deliverable | Exit criterion |
| --- | --- | --- |
| 0. Behavioral contract | Versioned fork/opcode/gas/precompile matrix, exact source/config pins, state codec specification, reference execution transcript format | Every known customization has an owner, source location, and planned differential fixture; target networks and historical retention requirements are explicit |
| 1. Engine feasibility | Isolated REVM compatibility experiments over test state, plus root-builder vectors | Wide nonce/CREATE, failure charging, mixed gas/opcodes, native raw state, nested rollback, and representative roots match; required upstream patches are enumerated and judged maintainable |
| 2. Rust state database | Read views, raw/native and EVM codecs, trie updates, staged writes, descriptors/provenance, commit/discard, history and pruning | Go-vs-Rust read/root/update/reopen conformance passes on copied fixtures; no canonical production writer is changed |
| 3. Complete Rust executor | Taraxa envelopes, all applicable instruction/precompile profiles, existing native kernels, genesis/fork hooks, system calls/rewards | Transaction-by-transaction and period-by-period execution matches reference, including mixed bytecode/native work and intermediate roots |
| 4. Queries and operations | Historical account/code/storage, call/estimate/trace, snapshot/prune, startup and recovery integration | Public result/error parity, no mutation from simulations, supported historical reads and retained-root behavior verified |
| 5. Replay and shadow validation | Independent Rust and Go databases run identical canonical inputs; restart/crash/prune and bounded performance campaigns | Zero unexplained deterministic mismatches; replay covers all supported fork transitions and adversarial cases; operational migration is rehearsed |
| 6. Production authority cutover | Application composes native Rust execution directly; obsolete Rust-mode CXX/Go routes removed | Required Tier 3 gates pass, rollback/rebuild procedure is proved, pure-C++ reference remains functional |
| 7. Ethereum feature upgrades | Separately approved Taraxa fork proposals and ecosystem compatibility tests | Network-level activation and mixed-version rules are specified; old-period replay remains deterministic |

Stages 1 and 2 can progress independently once Stage 0 specifies the codecs and reference harness. Join them at Stage 3.
Keep one production composition under `RUSTAXA_ENABLE`; reference selection and engine comparisons belong in explicit
test/replay harnesses, not silent runtime fallback. No live Go/Rust dual writer should open the same database directory.

Stage 1 must fail its gate if valid wide-nonce behavior cannot be preserved, the customization substantially forks
REVM internals, raw native state cannot coexist correctly with EVM execution, or canonical roots disagree. The response
is a recorded engine/design decision: evaluate evmone or a bounded compatibility implementation that preserves
existing-network behavior. A coordinated fork is not an acceptable workaround within this migration.
An unavailable Rust behavior must not silently delegate to legacy production execution.

Use stage exits to estimate the remaining work. A credible delivery date requires measuring adapter complexity,
state size, replay throughput, and available engineering capacity; repository inspection alone cannot supply it.

## Database migration and deployment

Prefer offline migration into a new state directory or deterministic replay/full resync for the first deployment.
Decide which is operationally acceptable after measuring actual database sizes and replay rates. This permits modern
internal storage without requiring the new writer to preserve every Go trie-node storage detail.

- A converter must start from a consistent stopped/checkpointed `db/` + `state_db/` pair and verify chain identity,
  period, root, native snapshots, and pending-state status. Specify any new identity/provenance linkage deliberately;
  it cannot be invented independently of the main database's paired markers.
- Preserve account and raw storage leaf bytes, code, native contract history, configuration events, and the declared
  historical retention range. A migration preserving only the current root is not an archive migration.
- Build the destination independently, verify the source and destination root at the selected head and retained
  checkpoints, and prove the next blocks execute identically. Resume interrupted conversion through explicit progress
  records without mutating the source.
- Switch the matching database pair only after verification and retain the original pair as a backup. Once the new
  backend has advanced, rollback means restoring/replaying a consistent pair or using a separately proved reverse
  converter, not opening its new schema with the old binary.
- Current Rust concrete-root provenance and complete native snapshots remain required. Existing markerless or
  incomplete histories do not become supported merely because the EVM was replaced. Preserve the established
  backup/full-resync path until a dedicated importer proves a wider migration contract.

Opening the existing Go RocksDB format directly from Rust is an alternative worth measuring, especially for faster
operator upgrades, but only after verifying column families, codecs, compression, locking, and RocksDB compatibility.
Do not combine an initial engine migration with a switch to MDBX, a new commitment scheme, or database unification.

## Validation and performance evidence

Follow [rewrite_validation_strategy.md](rewrite_validation_strategy.md). Preserve existing tests and reference behavior.
Current-source C++/Go comparison protects rewrite parity; pinned network history and an established reference client
are also needed to substantiate existing-network compatibility. The current branch's own parity tests alone cannot
prove agreement with every historical mainnet period.

The new differential harness should feed both implementations identical chain configuration, prior state, block
context, ordered canonical transactions, system actions, and reward inputs. Compare execution status/errors, return
data, CREATE addresses, per-transaction gas, balances/nonces, ordinary and native storage, logs/blooms, receipts,
intermediate roots, final roots, header bytes/hashes, and delayed validator/reward facts. Minimize and retain each
mismatch as a regression fixture. Database failures must remain infrastructure errors, not fabricated failed receipts.

Required adversarial coverage includes:

- Nonce gaps, stale nonces, u64/U256 boundaries, maximum transaction nonce followed by execution, internal CREATE,
  CREATE2, collisions, failed creation, and zero-address/system envelopes.
- Insufficient gas funds/value, intrinsic gas failures, pre/post-Cornus effects, refunds, SELFDESTRUCT, empty-account
  deletion, SSTORE transitions, and bytecode validity limits.
- All relevant fork boundaries at the preceding, activation, and following periods, plus genesis activation and
  disabled-fork sentinels; old/new transient opcodes and nested transient reverts.
- Native calls from contracts, static/delegate/callcode context, native failures, caught and enclosing reverts,
  irreversible raw writes, logs, same-block rewards/delegations, and delayed eligibility.
- BLS mapping changes and malformed cryptographic inputs; P-256/Falcon exact formats and gas; installed system code.
- Root-preserving inserts/updates/deletes, arbitrary raw values, absent/empty distinctions, historical reads,
  pruning with retained roots, corrupted records, failed writes, discard/retry, and reopen.
- Crashes before/after pending markers, concrete commit, and publication; rewards retries; idempotent recovery;
  simulations concurrent with finalized reads; multi-node sync and finalization agreement.

Run affected Rust package formatting/lint/tests, the relevant Rust consensus/bridge and StateAPI/RPC tests, and
`rust_storage_tests` for each storage slice. CMake builds use `--parallel 12`. Audit configured feature selection and
required tests actually executed. The existing Ethereum smoke in `state_api_test.cpp` is disabled and cannot be
reported as coverage. Add direct Rust executor coverage as StateAPI is retired without weakening behavioral tests.

Before production cutover, require the relevant Tier 3 FinalChain parity, storage differential, concrete-root
recovery/full-node gate (`rewrite-validate-e02`), CTest/Python integration, and both feature-on and pure-C++ builds
when build/source selection changes. Coordinate expensive gates under the repository's authorization rules when
implementation begins; this evaluation runs none of them.

Use upstream Ethereum cases for the rules Taraxa actually shares, plus explicit Taraxa differential fixtures for
differences. Ethereum's maintained [execution specs and tests](https://github.com/ethereum/execution-specs) provide
the upstream corpus; passing standard Ethereum tests does not prove Taraxa parity.

Benchmark representative transfer, contract storage, contract creation, native-contract, cryptographic, mixed-block,
and historical RPC workloads on matching hardware and durability settings. Measure execution, root construction,
commit/fsync, reopen, memory, database growth, pruning, and end-to-end catch-up separately. Compare tail finalization
latency and peak memory as well as throughput. Agree numeric regression budgets before rollout. No speedup is claimed
by this proposal; the custom state/commit path may dominate interpreter time.

## Getting closer to Ethereum after parity

First modernize dependency maintenance, test intake, execution interfaces, and tooling while retaining Taraxa's rules.
Then define an explicit supported-EIP profile and a repeatable upgrade process. Keep canonical DAG/PBFT ordering and
the existing Rust consensus architecture; execution modernization does not imply adopting Ethereum consensus.

Potential later proposals:

- A coherent newer opcode/gas/precompile profile, with activation rules for deployed contracts and an explicit decision
  on legacy transient opcodes and rollback quirks. Modern compiler compatibility must be tested against a named profile.
- Typed transaction support under [EIP-2718](https://eips.ethereum.org/EIPS/eip-2718), including signing/hash codecs,
  admission, gossip, DAG packing, persistence, receipts, and RPC. It is not only an EVM parser change.
- An [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559) fee-market proposal, if desired, that specifies base-fee
  progression across finalized periods, admission before final ordering, effective fees, burning, and interaction
  with Taraxa validator rewards and supply accounting. Accepting type-2 syntax alone is not full 1559 semantics.
- Nonce bounds or ordering changes, if desired, with explicit handling of preexisting wide nonces and DAG-reordered
  transactions. A new cap cannot retroactively remove replay support.
- Wallet/account-abstraction improvements after transaction and nonce rules are settled.

[EIP-4844](https://eips.ethereum.org/EIPS/eip-4844) introduces blob transactions and data outside ordinary EVM
execution. It requires a separate availability/transport/validation design for Taraxa; installing a KZG precompile
does not provide blob support. It should have its own use case and proposal.

Speculative parallel execution should also be a later performance project. DAG-independent transactions can still
touch shared EVM storage or native contracts. Any parallel executor must preserve canonical sequential outcomes and
the unusual rollback rules, with conflict detection and replay validated independently.

## Decisions still open

1. Identify the specific supported networks and required historical replay/archive retention. Existing-network
   compatibility first is confirmed; this is no longer an open architectural choice.
2. Establish whether REVM customization remains maintainable after the wide-nonce and native rollback experiments.
3. Choose offline conversion versus full replay using measured operator costs; direct-format compatibility is optional.
4. Select exact engine/trie/crypto releases, toolchain, and acceptable patch ownership after the feasibility gate.
5. Agree performance/resource budgets and the production rollout evidence window.
6. Decide which Ethereum feature changes warrant coordinated network proposals after implementation parity.

This evaluation inspected source and current upstream documentation. It did not implement a backend, build a
prototype, run parity/replay tests, benchmark engines, or verify live network state. REVM is the preferred candidate,
not yet a proven drop-in replacement.
