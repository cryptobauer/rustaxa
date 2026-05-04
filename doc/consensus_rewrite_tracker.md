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
| DAG graph | `dag/dag.hpp`, `dag/dag.cpp` | 424 lines | `rust-backed` | Rust domain behind C++ API | C++ `Dag`/`PivotTree` graph operations route to Rust under `RUSTAXA_ENABLE`. Legacy Boost graph remains the pure C++ fallback. |
| DAG manager | `dag/dag_manager.hpp`, `dag/dag_manager.cpp` | 1048 lines | partial | C++ shim plus Rust domain/infra ports | Manager orchestration remains C++; its `Dag`/`PivotTree` graph objects are Rust-backed in Rust mode. Depends on transaction manager, PBFT chain, storage, network, key manager, FinalChain. |
| DAG proposer | `dag/dag_block_proposer.hpp`, `dag/dag_block_proposer.cpp` | 576 lines | `cpp-owned` initially | C++ orchestration, later Rust proposer policy | Threaded, networked, VDF/DPoS-heavy. Keep orchestration in C++ early. |
| Sortition params | `dag/sortition_params_manager.hpp`, `dag/sortition_params_manager.cpp` | 331 lines | `not-started` | Rust domain plus storage port | Deterministic calculations and RLP/storage compatibility are good Rust candidates after DAG graph. |
| PBFT chain | `pbft/pbft_chain.hpp`, `pbft/pbft_chain.cpp` | 259 lines | `not-started` | Rust-backed infra/domain | Relatively bounded persisted head/chain state. Good early PBFT slice. |
| Proposed blocks | `pbft/proposed_blocks.hpp`, `pbft/proposed_blocks.cpp` | 178 lines | `not-started` | Rust domain/infra | Period/hash keyed cache plus DB persistence. |
| Period data queue | `pbft/period_data_queue.hpp`, `pbft/period_data_queue.cpp` | 168 lines | `not-started` | Rust domain, C++ sync wiring | Queue behavior is bounded; peer `NodeID` keeps C++ bridge concerns. |
| PBFT manager | `pbft/pbft_manager.hpp`, `pbft/pbft_manager.cpp` | 3267 lines | `not-started` | Split Rust services behind C++ daemon shell | Highest complexity: state machine, finalization, gossip, threading, storage, DAG, votes, pillar, FinalChain. |
| Verified votes | `vote_manager/verified_votes.hpp`, `vote_manager/verified_votes.cpp` | 384 lines | `not-started` | Rust domain | Good candidate for deterministic vote aggregation and threshold tests. |
| Vote manager | `vote_manager/vote_manager.hpp`, `vote_manager/vote_manager.cpp` | 1145 lines | `not-started` | Rust domain for validation/aggregation; C++ network/storage shell | Depends on FinalChain DPoS, VRF, slashing, storage, network. |
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
| `SortitionParamsManager` | params lookup, DAG efficiency, interval recalculation, cleanup | `DbStorage`, config, `PeriodData`, VDF params | `sortition_test`, `full_node_test` lambda tests | Rust deterministic calculations with storage port |

### PBFT

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `PbftChain` | head/hash/size reads, block lookup, update head, block validation | `DbStorage`, `PbftBlock` | `pbft_chain_test`, `pbft_manager_test`, `full_node_test` | Early Rust-backed PBFT state slice |
| `ProposedBlocks` | push, mark valid, lookup, presence, cleanup, old-block checks | `DbStorage`, `PbftBlock` | `pbft_manager_test` proposed-block cases | Rust domain plus storage adapter |
| `PeriodDataQueue` | push/pop/clear/size/period/last block/old-data cleanup | `PeriodData`, `PbftVote`, peer `NodeID` | `pbft_manager_test`, full-node sync tests | Rust queue if peer identity bridge stays simple; otherwise defer |
| `PbftManager` | lifecycle, state machine, proposal generation, period/round/step, DPoS counts, sync queue, block validation, gossip, finalization, dynamic lambda | nearly every consensus subsystem | `pbft_manager_test`, `vote_test`, `pillar_chain_test`, `full_node_test`, Python integration | Split into Rust services after lower-level ports are stable |

### Votes and Eligibility

| Class | Public API groups | Dependencies | Tests | Target |
| --- | --- | --- | --- | --- |
| `VerifiedVotes` | vote insertion, unique voter tracking, step/round/period lookup, 2t+1 voted blocks, cleanup | `PbftVote` | `vote_test`, `pbft_manager_test` | Rust domain vote aggregation |
| `VoteManager` | vote validation, generation, reward votes, two_t_plus_one thresholds, VRF sortition, current period/round | `FinalChain`, `PbftChain`, `KeyManager`, `SlashingManager`, `DbStorage`, `Network`, VRF | `vote_test`, `pbft_manager_test` | Port validation/aggregation after DPoS ports exist; keep gossip shell in C++ |
| FinalChain DPoS ports | `dposIsEligible`, eligible vote count, total vote count, validators eligible vote counts, VRF key | FinalChain/state API/EVM | `pbft_manager_test`, `state_api_test`, proposer tests | Required before replacing temporary consensus eligibility behavior |

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
orchestration remains C++, but its graph objects now use Rust in Rust-enabled builds.

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
- `DagManager` remains in C++ during this slice; production graph routing is active through the `Dag`/`PivotTree`
  wrappers under `RUSTAXA_ENABLE`.

Required tests:

- Rust unit tests for graph insertion, leaves, reachability, ghost path, and deterministic order. Landed.
- CXX bridge fixture tests for Rust graph behavior. Landed under `rust_consensus_tests`.
- C++ public API regression/parity tests through the Rust-backed `Dag` wrapper. Landed through `dag_test`.
- Existing C++ tests: `dag_test`, `dag_block_test`, and ordering cases in `full_node_test`.

Open questions:

- Whether `computeOrder` must preserve every Boost traversal tie-breaker or only the externally visible block order.
- Whether direct C++ legacy `Dag` linking can be added to `rust_consensus_tests` without duplicate dependency symbols, or
  whether parity should stay fixture/transcript based.

## Validation Matrix

| Change area | Minimum validation |
| --- | --- |
| Rust consensus domain only | `cargo fmt --manifest-path rust/Cargo.toml`, `cargo clippy --manifest-path rust/Cargo.toml`, `cargo test --manifest-path rust/Cargo.toml` |
| DAG graph routing | Rust validation plus `rust_consensus_tests`, `dag_test`, and `dag_block_test` |
| PBFT chain/proposed-block/queue routing | Rust validation plus `pbft_chain_test` and relevant `pbft_manager_test` cases |
| Vote aggregation/eligibility | Rust validation plus `vote_test`, relevant `pbft_manager_test`, and DPoS/state API coverage |
| Transaction queue behavior | Rust validation plus `transaction_test` and affected DAG/PBFT tests |
| Pillar/reward behavior | Rust validation plus `pillar_chain_test`, `rewards_stats_test`, and affected full-node tests |
| PBFT manager state machine | Targeted PBFT/vote/DAG tests plus full-node smoke and Python integration coverage as needed |

## Current Open Items

| Item | Status | Owner decision needed |
| --- | --- | --- |
| Replace temporary `dposIsEligible` behavior | `shim-stubbed` | Needs real FinalChain/state DPoS port before consensus can rely on it. |
| Create Rust DAG graph module | `rust-backed` | Landed as standalone Rust domain plus bridge tests and C++ `Dag` production routing under `RUSTAXA_ENABLE`. |
| Define consensus storage ports | `not-started` | Needed before Rust services depend on storage. |
| Decide CXX bridge shape for consensus hashes and vectors | `rust-backed` for DAG graph | DAG bridge uses fixed bytes and explicit boundary conversion; revisit if PBFT/vote bridges need richer payloads. |
| Add C++/Rust DAG parity fixture | `rust-backed` | Rust bridge fixture tests and C++ public API regression tests landed. Direct in-process legacy-vs-Rust comparison remains optional if duplicate dependency symbols are resolved. |
