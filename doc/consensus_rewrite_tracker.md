# Consensus Rewrite Tracker

This tracker expands the consensus section in `PLAN.md`. Keep it current as consensus code moves from C++ into Rust.
Required test coverage and parity gates for the Rust consensus model are defined in
`doc/consensus_model_test_plan.md`.

## Status Legend

| Status | Meaning |
| --- | --- |
| `not-started` | No Rust port exists yet. |
| `rust-domain` | Pure Rust domain model or deterministic algorithm exists without C++ runtime wiring. |
| `rust-backed` | C++ public API is backed by Rust for this behavior. |
| `partial` | Some Rust-backed behavior exists, but documented gaps remain before the API can replace C++ semantics. |
| `shim-stubbed` | Rust mode has an explicit temporary default/no-op. Must not delegate to legacy C++. |
| `cpp-owned` | Intentionally left in C++ for now. |
| `deferred` | Out of current consensus rewrite scope. |

## Hard Rules

- Do not delegate Rust shim behavior back to legacy C++ implementation methods.
- Keep public C++ APIs stable unless a task explicitly changes the API.
- Main-only files referenced from upstream-owned C++ files must be included only behind `RUSTAXA_ENABLE=1` or a narrower Rustaxa feature guard.
- Treat DPoS eligibility and vote-count behavior as required consensus functionality. Temporary defaults must stay visible in this tracker.
- Keep network callbacks, daemon threads, peer gossip, and full-node orchestration in C++ until the Rust domain services are stable.

## Current Rust Starting Point

| Area | Rust location | Status | Notes |
| --- | --- | --- | --- |
| FinalChain read/index helper | `rust/crates/rustaxa-consensus/src/final_chain.rs` | `rust-backed` for selected FinalChain reads | Exists because FinalChain work started before consensus. It is not a PBFT/DAG port yet. |
| Consensus crate root | `rust/crates/rustaxa-consensus/src/lib.rs` | `rust-backed` for DAG graph | Exports the DAG graph model used by the C++ `Dag` wrapper and selected FinalChain read helpers. |
| Shared DAG/PBFT types | `rust/crates/rustaxa-types/src/{dag.rs,pbft.rs}` and codec modules | partial | Useful for future consensus domain types, but not yet a full consensus model. |
| Storage ports | `rust/crates/rustaxa-storage/src/{dag.rs,pbft.rs,pillar.rs}` | partial infra | Use through narrow domain-facing ports; do not let consensus logic depend on broad storage APIs. |

## Module Inventory

| Module | Primary files | Approx size | Status | Proposed ownership | Notes |
| --- | --- | ---: | --- | --- | --- |
| DAG graph | `dag/dag.hpp`, `dag/dag.cpp`, `dag_shim/*` | 424 lines | `rust-backed` | Rust domain behind C++ overlay shim | C++ `Dag`/`PivotTree` graph operations route to Rust under `RUSTAXA_ENABLE` through a full overlay shim. Legacy Boost graph remains pure C++ fallback/reference code and is compiled as `DagOld`/`PivotTreeOld` only in Rust-enabled builds. |
| DAG manager | `dag/dag_manager.hpp`, `dag/dag_manager.cpp` | 1048 lines | partial | C++ shim plus Rust domain/infra ports | Manager orchestration remains C++; its `Dag`/`PivotTree` graph objects are Rust-backed in Rust mode. Depends on transaction manager, PBFT chain, storage, network, key manager, FinalChain. |
| DAG proposer | `dag/dag_block_proposer.hpp`, `dag/dag_block_proposer.cpp` | 576 lines | `cpp-owned` initially | C++ orchestration, later Rust proposer policy | Threaded, networked, VDF/DPoS-heavy. Keep orchestration in C++ early. |
| Sortition params | `dag/sortition_params_manager.hpp`, `dag/sortition_params_manager.cpp` | 331 lines | `rust-backed` | Rust domain behind C++ overlay shim | Deterministic efficiency/threshold runtime state routes to `rustaxa-consensus::sortition` under `RUSTAXA_ENABLE_SORTITION_PARAMS`. C++ still owns storage reads/writes and batch atomicity; the legacy implementation is compiled as `SortitionParamsManagerOld` only for pure C++ reference builds. |
| PBFT chain | `pbft/pbft_chain.hpp`, `pbft/pbft_chain.cpp`, `pbft_chain_shim/*` | 259 lines | `rust-backed` | Rust domain behind C++ overlay shim | In-memory head updates, legacy JSON-head preview, and next-block validation route to Rust under `RUSTAXA_ENABLE_PBFT_CHAIN`. C++ shim preserves `DbStorage` lookup/persistence ownership and JsonCpp formatting; legacy implementation is compiled as `PbftChainOld` only for pure C++ reference builds. |
| Proposed blocks | `pbft/proposed_blocks.hpp`, `pbft/proposed_blocks.cpp`, `proposed_blocks_shim/*` | 178 lines | `rust-backed` | Rust domain behind C++ overlay shim | Proposed block membership, cached validation flags, RLP payload snapshots, old-block diagnostics, and cleanup planning route to Rust under `RUSTAXA_ENABLE_PROPOSED_BLOCKS`. C++ shim preserves `DbStorage` persistence/removal and `PbftBlock` materialization; legacy implementation is compiled as `ProposedBlocksOld` only for pure C++ reference builds. |
| Period data queue | `pbft/period_data_queue.hpp`, `pbft/period_data_queue.cpp`, `period_data_queue_shim/*` | 168 lines | `rust-backed` | Rust domain metadata behind C++ overlay shim | Admission rules, effective processable size, period tracking, pop vote-source decisions, cleanup planning, and clear semantics route to Rust under `RUSTAXA_ENABLE_PERIOD_DATA_QUEUE`. C++ keeps live `PeriodData`, `PbftVote`, and peer `NodeID` ownership. |
| PBFT manager | `pbft/pbft_manager.hpp`, `pbft/pbft_manager.cpp` | 3267 lines | `not-started` | Split Rust services behind C++ daemon shell | Highest complexity: state machine, finalization, gossip, threading, storage, DAG, votes, pillar, FinalChain. |
| Verified votes | `vote_manager/verified_votes.hpp`, `vote_manager/verified_votes.cpp`, `verified_votes_shim/*` | 384 lines | `rust-backed` | Rust domain behind C++ overlay shim | Unique-voter checks, voted-value weight aggregation, 2t+1 block mappings, period cleanup, and round t+1 markers route to `rustaxa-consensus::verified_votes` through `RUSTAXA_ENABLE_VERIFIED_VOTES`. C++ shim owns live `PbftVote` objects and reconstructs legacy snapshot types for `VoteManager` callers. 2t+1 mapping insertion in Rust mode is first-writer-wins, and `VoteManager` consumes insertion outcome to avoid replacing already-mapped bundles. Legacy implementation is compiled as `VerifiedVotesOld` for pure C++ reference builds. |
| Vote manager | `vote_manager/vote_manager.hpp`, `vote_manager/vote_manager.cpp` | 1145 lines | `partial` | Rust domain for validation/aggregation; C++ network/storage shell | `addVerifiedVote` now uses Rust-backed atomic verified-vote insertion under `RUSTAXA_ENABLE_VERIFIED_VOTES` while the rest of VoteManager remains C++ orchestration. Depends on FinalChain DPoS, VRF, slashing, storage, network. |
| Transaction queue | `transaction/transaction_queue.hpp`, `transaction/transaction_queue.cpp` | 501 lines | `not-started` | Rust domain | Port before `TransactionManager`; deterministic ordering and pool limits need parity tests. |
| Transaction manager | `transaction/transaction_manager.hpp`, `transaction/transaction_manager.cpp` | 837 lines | `cpp-owned` initially | C++ shell plus Rust queue/validation helpers | FinalChain/state API, storage, async estimation, DAG status transitions. |
| Gas pricer | `transaction/gas_pricer.hpp`, `transaction/gas_pricer.cpp` | 171 lines | `not-started` | Rust domain/infra | Bounded dependency on storage and transaction manager. |
| Pillar block/votes | `pillar_chain/pillar_block.hpp`, `pillar_chain/pillar_votes.hpp`, matching `.cpp` files | 627 lines | `not-started` | Rust domain and codecs | RLP and Solidity encoding compatibility are critical. |
| Pillar manager | `pillar_chain/pillar_chain_manager.hpp`, `pillar_chain/pillar_chain_manager.cpp` | 629 lines | `deferred` | C++ orchestration initially | Depends on FinalChain validator vote counts, network, storage, key manager. |
| Rewards stats | `rewards/block_stats.*`, `rewards/rewards_stats.*` | 407 lines | `not-started` | Rust deterministic domain | Port after PeriodData, votes, and DPoS vote-count ports are real. |
| Slashing manager | `slashing_manager/slashing_manager.*` | 102 lines | `deferred` | C++ initially | Depends on FinalChain and transaction submission. |
| Key manager | `key_manager/key_manager.*` | 55 lines | `cpp-owned` | C++ initially | Small wallet/secret wrapper; not on critical rewrite path. |

## Public API Tracker

### DAG

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `Dag` / `PivotTree` | vertex/edge counts, `hasVertex`, `addVEEs`, leaves, ghost path, deterministic order, graph clearing | hashes, Boost graph today | `dag_test`, `full_node_test` ordering cases | Rust domain graph with byte/hash-compatible ordering |
| `DagManager` | block known/get/verify/add, pivot/tip availability, ordering, frontier, non-finalized blocks, anchors, expiry, VDF message | `DbStorage`, `TransactionManager`, `PbftChain`, `FinalChain`, `Network`, `KeyManager`, config | `dag_test`, `dag_block_test`, `pbft_manager_test`, `full_node_test` | C++ shim delegates pure graph/order logic to Rust first |
| `DagBlockProposer` | proposer lifecycle, propose block, select tips, proposer eligibility | `DagManager`, `TransactionManager`, `FinalChain`, `DbStorage`, `KeyManager`, `Network`, VDF | `dag_block_test`, `pbft_manager_test`, `sortition_test`, full-node tests | Keep C++ thread/network shell; port deterministic selection/policy later |
| `SortitionParamsManager` | params lookup, DAG efficiency, interval recalculation, cleanup | `DbStorage`, config, `PeriodData`, VDF params | `sortition_test`, `rust_consensus_tests`, `sortition_params_manager_shim_test`, full-node lambda tests | Rust deterministic calculations and runtime state; C++ storage/batch shell |

### PBFT

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PbftChain` | head/hash/size reads, block lookup, update head, block validation | `DbStorage`, `PbftBlock` | `pbft_chain_test`, `pbft_manager_test`, `full_node_test` | Early Rust-backed PBFT state slice |
| `ProposedBlocks` | push, mark valid, lookup, presence, cleanup, old-block checks | `DbStorage`, `PbftBlock` | `pbft_manager_test` proposed-block cases | Rust domain plus storage adapter |
| `PeriodDataQueue` | push/pop/clear/size/period/last block/old-data cleanup | `PeriodData`, `PbftVote`, peer `NodeID` | `rust_consensus_tests`, `period_data_queue_shim_test`, `pbft_manager_test`, full-node sync tests | Rust metadata queue plus C++ live payload shim |
| `PbftManager` | lifecycle, state machine, proposal generation, period/round/step, DPoS counts, sync queue, block validation, gossip, finalization, dynamic lambda | nearly every consensus subsystem | `pbft_manager_test`, `vote_test`, `pillar_chain_test`, `full_node_test`, Python integration | Split into Rust services after lower-level ports are stable |

### Votes and Eligibility

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `VerifiedVotes` | vote insertion, unique voter tracking, step/round/period lookup, 2t+1 voted blocks, cleanup | `PbftVote` | `vote_test`, `pbft_manager_test` | Rust domain vote aggregation |
| `VoteManager` | vote validation, generation, reward votes, two_t_plus_one thresholds, VRF sortition, current period/round | `FinalChain`, `PbftChain`, `KeyManager`, `SlashingManager`, `DbStorage`, `Network`, VRF | `vote_test`, `pbft_manager_test` | Port validation/aggregation after DPoS ports exist; keep gossip shell in C++ |
| FinalChain DPoS ports | `dposIsEligible`, eligible vote count, total vote count, validators eligible vote counts, validators total stakes, VRF key, selected DPoS precompile reads | FinalChain/state API/EVM | `rust_consensus_tests`, `final_chain_test`, `rpc_test`, `pbft_manager_test`, `state_api_test`, proposer tests | Partial: genesis snapshot is Rust-backed and block numbers are preserved through the shim/bridge. Rust finalization now appends snapshots for native-transfer blocks and records post-Magnolia transaction-fee commission rewards by finalized DAG block author. Missing historical snapshots and unsupported state/EVM DPoS transitions still throw instead of falling back. |

### Transactions

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `TransactionQueue` | insert/erase/get/order/group/contains/size/purge/known tx/min gas price | `FinalChain`, transactions | `transaction_test`, `full_node_test` pool cases | Rust domain queue before manager port |
| `TransactionManager` | verify/insert/pack/get/finalize status/non-finalized recovery/gas estimation | `DbStorage`, `FinalChain`, thread pool, `DagBlock`, state API | `transaction_test`, `dag_block_test`, `pbft_manager_test`, `full_node_test` | C++ shell initially; port queue and deterministic helpers first |
| `GasPricer` | gas price reads/calculation | `DbStorage`, `TransactionManager` | `transaction_test`, full-node transaction tests | Rust after queue semantics are stable |

### Pillar, Rewards, Slashing

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PillarBlock` / `PillarBlockData` | RLP, hash, JSON, Solidity encode/decode, validator vote-count deltas | hashes, state API data | `pillar_chain_test` encoding/finalization cases | Rust domain and codec parity |
| `PillarVotes` | vote uniqueness, threshold accumulation, above-threshold selection, cleanup | `PillarVote` | `pillar_chain_test` | Rust domain after vote-count port |
| `PillarChainManager` | create block, validate/generate/finalize votes, current/finalized block state | `FinalChain`, `DbStorage`, `Network`, `KeyManager` | `pillar_chain_test`, `full_node_test` | Defer orchestration until FinalChain DPoS/state ports are real |
| `rewards::BlockStats` / `rewards::Stats` | per-block stats, interval recovery/processing/cleanup | `PeriodData`, PBFT votes, DPoS total vote count, storage | `rewards_stats_test`, full-node reward paths | Rust deterministic domain after PeriodData and vote models |
| `SlashingManager` | double-voting proof submission | `FinalChain`, `TransactionManager`, `GasPricer` | limited direct coverage | Deferred |

## First Slice: Rust DAG Graph

Status: `rust-backed` landed for C++ `Dag`/`PivotTree` graph operations under `RUSTAXA_ENABLE`. `DagManager`
orchestration remains C++, but Rust-enabled builds now keep a Rust-owned `DagManagerState` for deterministic in-memory
state. Frontier, ghost path, ordering, graphviz output, counters, anchors, period, expiry level, non-finalized indexes,
minimum difficulty, pivot/tip availability metadata, storage-backed persistence, and the first deterministic
`verifyBlock` reject decisions route through that state/runtime. The Rust-mode `DagManager` shim now owns the
`verifyBlock` flow directly for prechecks, transaction materialization, DAG VDF payload/difficulty/proof verification,
VDF/DPoS authorization decision ordering, and Rust-backed gas policy decisions instead of forwarding the method
wholesale to `DagManagerOld`.

Target behavior:

- `Dag::hasVertex`
- Rust `DagGraph::add_vertex_edges` parity with C++ `Dag::addVEEs`
- leaf collection
- `PivotTree::getGhostPath`
- `Dag::computeOrder`
- vertex/edge counters and clear behavior

Rust design sketch:

- `rustaxa-consensus` has a `dag` module with explicit hash-keyed graph state instead of mirrored Boost graph types.
- Ordering is deterministic and covered by Rust unit tests and CXX bridge fixture tests.
- The bridge uses fixed hash bytes and explicit conversion at the boundary.
- `DagManager` remains in C++ during this slice; command-side DB writes, transaction handling, events, and networking
  still stay with the C++ side while deterministic in-memory state and storage-backed prechecks move through
  `DagManagerState`/`BridgeDagManagerRuntime` under `RUSTAXA_ENABLE`.

Required tests:

- Rust unit tests for graph insertion, leaves, reachability, ghost path, and deterministic order. Landed.
- CXX bridge fixture tests for Rust graph behavior. Landed under `rust_consensus_tests`.
- C++ public API regression/parity tests through the Rust-backed `Dag` wrapper. Landed through `dag_test`.
- Existing C++ tests: `dag_test`, `dag_block_test`, and ordering cases in `full_node_test`.
- Rust `verifyBlock` coverage for tip count/uniqueness, missing proposal-period mapping, expired block, transaction
  availability, VDF/DPoS authorization decision ordering, and gas-policy decisions. Landed in `rustaxa-consensus` and
  `rustaxa-bridge`; the shim now passes an explicit status-coded Rust VDF/DPoS fact envelope instead of encoding separate
  authorization branches in C++. DPoS/VRF facts are collected through a Rust FinalChain bridge bundle. Rust now decodes
  the DAG VDF payload, verifies the embedded VRF proof, calculates sortition difficulty, and verifies the Wesolowski
  proof against the exact legacy ASCII-hex modulus bytes used by C++ `VdfSortition`. The path no longer requires a
  `DagManagerOld::verifyBlock` method forward, and it no longer derives VRF output through the C++ VRF wrapper.

Open questions:

- Whether `computeOrder` must preserve every Boost traversal tie-breaker or only the externally visible block order.
- Whether direct C++ legacy `Dag` linking can be added to `rust_consensus_tests` without duplicate dependency symbols, or
  whether parity should stay fixture/transcript based.

## Validation Matrix

| Change area | Minimum validation |
| --- | --- |
| Rust consensus domain only | `cargo fmt --manifest-path rust/Cargo.toml`, `cargo clippy --manifest-path rust/Cargo.toml`, `cargo test --manifest-path rust/Cargo.toml` |
| DAG graph routing | Rust validation plus `rust_consensus_tests`, `dag_test`, `dag_block_test`, and `dag_shim_test` |
| Sortition params routing | Rust validation plus `rust_consensus_tests`, `sortition_test`, and `sortition_params_manager_shim_test` |
| PBFT chain/proposed-block/queue routing | Rust validation plus `rust_consensus_tests`, `pbft_chain_test`, `pbft_chain_shim_test`, `proposed_blocks_shim_test`, `period_data_queue_shim_test`, and relevant `pbft_manager_test` cases |
| Vote aggregation/eligibility | Rust validation plus `rust_consensus_tests`, `verified_votes_shim_test`, `vote_test`, relevant `pbft_manager_test`, and DPoS/state API coverage |
| Transaction queue behavior | Rust validation plus `transaction_test` and affected DAG/PBFT tests |
| Pillar/reward behavior | Rust validation plus `pillar_chain_test`, `rewards_stats_test`, and affected full-node tests |
| PBFT manager state machine | Targeted PBFT/vote/DAG tests plus full-node smoke and Python integration coverage as needed |

## Current Open Items

| Item | Status | Owner decision needed |
| --- | --- | --- |
| Replace temporary DPoS query behavior | `partial` | Genesis DPoS vote-count, eligibility, validator total stake, and validator eligible vote-count queries are Rust-backed. Rust-finalized native-transfer blocks now carry forward snapshots and post-Magnolia fee commission rewards. Remaining gaps: validator owner/metadata, delegation mutations, jailing, slashing, broader rewards distribution, and contract-call state transitions. |
| Create Rust DAG graph module | `rust-backed` | Landed as standalone Rust domain plus bridge tests and C++ `Dag` production routing through a full overlay shim under `RUSTAXA_ENABLE`. |
| Route sortition params through Rust | `rust-backed` | Landed under `RUSTAXA_ENABLE_SORTITION_PARAMS`; storage and write batches intentionally remain C++ owned for this slice. |
| Route verified votes through Rust | `rust-backed` | Landed under `RUSTAXA_ENABLE_VERIFIED_VOTES`; C++ shim preserves live `PbftVote` ownership while Rust owns deterministic index semantics and 2t+1 metadata. VoteManager Rust mode now consumes a single atomic insert outcome in `addVerifiedVote`, removing split unique-voter/voted-value mutation in the Rust-enabled path. |
| Route DagManager verify flow through Rust/shim | `partial` | Tip count/uniqueness, proposal-period availability, expiry, transaction availability, DAG embedded-VRF/VDF payload/difficulty/proof verification, VDF/DPoS authorization ordering, and gas-policy decisions route through Rust. The shim owns live transaction fetching plus temporary VRF input and VDF message construction, while DPoS/VRF facts now come from a Rust FinalChain bridge bundle and feed a single status-coded Rust envelope. Remaining gaps: move VRF input and VDF message construction fully into Rust, then replace temporary historical hardfork vote-ceiling compatibility. |
| Define consensus storage ports | `not-started` | Needed before Rust services depend on storage. |
| Decide CXX bridge shape for consensus hashes and vectors | `rust-backed` for DAG graph | DAG bridge uses fixed bytes and explicit boundary conversion; revisit if PBFT/vote bridges need richer payloads. |
| Add C++/Rust DAG parity fixture | `rust-backed` | Rust bridge fixture tests and C++ public API regression tests landed. Direct in-process legacy-vs-Rust comparison remains optional if duplicate dependency symbols are resolved. |
| Vote packet duplicate-with-block delivery gap | `deferred` | Reproduced in `PbftManagerTest.propose_block_and_vote_broadcast`: some peers can miss proposed-block insertion when vote paths short-circuit in network packet handlers. Do not patch upstream-owned network C++ in this rewrite stream; track as network-module follow-up and resolve via rewrite-side network shim when network work starts. |
