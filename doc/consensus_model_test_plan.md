# Consensus Model Rewrite Test Plan

This document defines the required tests for the Rust consensus model rewrite. It complements
`doc/consensus_rewrite_tracker.md` and uses the validation tiers in `doc/rewrite_validation_strategy.md`.

## Current Coverage

Rust consensus coverage is not yet consensus-model coverage:

- `rustaxa-consensus` currently exposes selected FinalChain read/index helpers only.
- Existing Rust tests in `rustaxa-consensus` cover FinalChain index/header/transaction lookups.
- `rustaxa-types` has basic DAG and PBFT RLP decoding tests, but no consensus state-machine or graph-model tests.
- `rustaxa-storage` has repository-level unit tests for DAG, PBFT, period, pillar, transaction, config, metadata, and
  final-chain columns.
- There is no Rust DAG graph, PBFT chain, proposed-block cache, period-data queue, vote aggregation, sortition,
  transaction queue, pillar, or rewards consensus model yet.

C++ coverage is broad but mostly not usable as direct Rust parity coverage yet:

- DAG behavior: `dag_test`, `dag_block_test`, selected `full_node_test` ordering/reconstruction cases.
- PBFT chain/manager behavior: `pbft_chain_test`, `pbft_manager_test`, `network_test`, selected `full_node_test` cases.
- Vote behavior: `vote_test`, PBFT manager tests, network vote transfer/broadcast tests.
- Sortition behavior: `sortition_test`.
- Transaction queue/manager behavior: `transaction_test`, DAG/PBFT/full-node transaction cases.
- Pillar/reward behavior: `pillar_chain_test`, `rewards_stats_test`.
- Runtime/RPC/final-chain surfaces: `final_chain_test`, `state_api_test`, `rpc_test`, Python integration tests.

## Gaps

The main gap is that existing C++ tests validate current behavior indirectly, but they do not produce reusable parity
fixtures for Rust model code.

Required missing infrastructure:

- deterministic C++ vs Rust fixtures for graph/order, PBFT chain state, vote aggregation, sortition, transaction
  ordering, pillar encoding, and rewards calculations
- Rust unit tests for pure domain models before adding CXX shims
- Rust bridge tests for every routed C++ API before enabling production Rust behavior
- regression fixtures for malformed/edge inputs that C++ currently accepts, rejects, or normalizes
- runtime smoke/subsystem tests for any path used during startup, sync, consensus, finalization, or RPC handling

Do not route production consensus behavior to Rust until the relevant parity fixture exists and the corresponding
Rust-enabled subsystem or smoke test passes.

## Required Tests By Rewrite Slice

### 1. DAG Graph Model

Rust unit tests required before CXX routing:

- genesis graph initialization creates exactly one vertex and no edges
- repeated vertex insertion does not create duplicate vertices or edges
- pivot edges and tip edges are added only when referenced vertices exist
- isolated vertices are retained and reported as leaves
- leaf collection is deterministic for identical graph contents
- reachability handles self, reachable descendants, missing vertices, and disconnected vertices
- ghost path chooses the heaviest subtree and resolves equal weights by smallest block hash
- `compute_order` returns only non-finalized blocks that can reach the anchor
- `compute_order` is deterministic across insertion orders and hash tie cases
- `clear` empties graph state and allows a fresh graph to be built

C++ vs Rust parity tests required before production routing:

- port the Conflux-paper fixture from `DagTest.compute_epoch` into a shared fixture and compare ordered block hashes
- compare `getGhostPath` and leaves for branching graphs with equal subtree weights
- compare missing-pivot/missing-tip behavior
- compare malformed/missing-anchor behavior for `computeOrder`

Validation target:

```bash
make rewrite-validate-consensus
```

### 2. PBFT Chain Model

Rust unit tests required:

- initializes from an empty/genesis DB state with the same head, size, non-empty size, and last hashes as C++
- `update_pbft_chain` increments total size for every block
- `update_pbft_chain` increments non-empty size only for non-null anchors
- last PBFT hash and last non-null DAG anchor update exactly like C++
- block lookup distinguishes missing block, malformed persisted block, and valid block
- JSON/head encoding used for persisted head state is byte-compatible with C++
- validation rejects wrong period, wrong previous hash, and inconsistent anchor transitions

C++ vs Rust parity tests required:

- replay `pbft_chain_test.pbft_db_test` as a transcript and compare DB head plus block lookup outputs
- compare null-anchor and non-null-anchor chains across multiple periods

### 3. Proposed Blocks And Period Data Queue

Rust unit tests required:

- proposed block insertion is keyed by period and block hash
- duplicate insertion preserves original semantics
- `mark_valid` changes only the target block
- cleanup removes only finalized/old periods
- old-block detection reports the same boundary period as C++
- period-data queue preserves FIFO order
- queue period reflects the last queued or in-flight period exactly like C++
- stale queue cleanup removes periods below the current period and preserves the last cert votes expected by sync

C++ vs Rust parity tests required:

- add C++ fixture coverage for `ProposedBlocks` and `PeriodDataQueue`; current coverage is mostly through
  `pbft_manager_test`
- compare serialized period data plus cert-vote handling for sync queue scenarios

### 4. Vote Aggregation And Eligibility

Rust unit tests required:

- insert vote by period/round/step/block hash and retrieve the same grouping
- duplicate vote from same voter does not increase weight or count
- conflicting vote from same voter is rejected and reports the existing vote
- second next-vote rule allows exactly one null-block and one non-null-block vote at odd finishing steps
- two_t_plus_one block tracking stores soft, cert, next, and next-null voted blocks independently
- t_plus_one network step tracking updates only for the intended period/round
- cleanup removes old periods without disturbing later periods
- vote weight aggregation handles large vote counts without truncation

C++ vs Rust parity tests required:

- replay `vote_test.verified_votes`, `vote_test.two_t_plus_one_votes`, and the second-next-vote case as shared fixtures
- compare vote-count threshold behavior from `vote_test.vote_count_compare`
- compare DPoS eligible vote count and total vote count through a FinalChain/state test fixture before replacing any
  temporary eligibility defaults

Runtime validation required:

- `vote_test`
- relevant `pbft_manager_test` DPoS threshold cases
- `state_api_test` or `final_chain_test` coverage for DPoS vote-count ports

### 5. Sortition Parameters

Rust unit tests required:

- DAG efficiency equals unique transaction count divided by total DAG transaction references
- duplicate transactions across DAG blocks reduce efficiency exactly like C++
- empty/null-anchor periods follow current C++ ignore behavior
- average efficiency uses the same interval window and rounding as C++
- VRF upper range clamps at configured minimum/maximum bounds
- sortition params change RLP is byte-compatible with C++
- cleanup keeps the same number of historical changes as C++

C++ vs Rust parity tests required:

- replay `sortition_test.efficiency_calculation`, `params_change_serialization`, `db_cleanup`, `efficiency_restart`, and
  `params_restart` as fixture transcripts

### 6. Transaction Queue Model

Rust unit tests required:

- insertion classifies proposable and non-proposable transactions exactly like C++
- per-account nonce ordering is stable
- cross-account ordering by gas price and priority is stable
- max pool size, single-account limit, non-proposable limit, and data-size limit evict the same transactions as C++
- low-nonce and insufficient-balance transactions expire at the same finalized-block boundary
- `block_finalized` and `purge` remove the same transactions as C++
- known-transaction cache reports known hashes and expires them under the same conditions
- minimum gas price for block inclusion matches C++ for empty, partial, and full queues

C++ vs Rust parity tests required:

- replay `transaction_test.priority_queue`, `priority_queue_max_size`, `priority_queue_ordering`,
  `priority_queue_ordering_eth_test`, `finalization_ordering`, `zero_gas_price_limit`, and `gas_price_limiting`

### 7. Pillar And Rewards Models

Rust unit tests required:

- pillar block RLP, hash, JSON-facing fields, and Solidity encoding are byte-compatible with C++
- validator vote-count deltas preserve signed values, zero deltas, ordering, and duplicate-validator handling
- pillar vote uniqueness and threshold selection match C++
- rewards stats accumulate block authors, DAG authors, fees, and vote participation exactly like C++
- rewards distribution interval changes and cleanup boundaries match C++

C++ vs Rust parity tests required:

- replay `pillar_chain_test.block_serialization`, `pillar_block_solidity_rlp_encoding`,
  `pillar_vote_solidity_rlp_encoding`, `votes_count_changes`, and `finalize_root_in_pillar_block`
- cover `PillarChainManager::addVerifiedPillarVote` in Rust-enabled mode for successful recovered-voter insertion and
  invalid Rust-inspected signature rejection
- replay `rewards_stats_test.statsProcessing`, `distributionChange`, `feeRewards`, and `dagBlockRewards`

### 8. PBFT Manager State Machine

Do not port `PbftManager` as one large unit. Before routing PBFT manager decisions through Rust, each lower-level model
above must have unit and parity coverage.

Rust service tests required:

- proposal creation inputs produce the same proposed PBFT block metadata as C++
- round/step transitions follow the same soft, cert, next, and timeout cases
- finalization decision uses the same anchor, period data, reward vote, and cert-vote inputs as C++
- sync queue processing accepts/rejects period data identically
- dynamic lambda/backoff inputs produce the same delay decisions

Runtime validation required:

- `make rewrite-validate-consensus`
- `make rewrite-validate-smoke`
- targeted `network_test` PBFT sync/vote cases when gossip or sync behavior changes
- Python integration tests when finalization, node sync, RPC-visible chain state, or transaction execution behavior changes

## Acceptance Rules

- A Rust domain model can merge with Rust unit tests only if it is not production-routed.
- A Rust CXX bridge can merge behind disabled routing with Rust unit tests plus bridge constructor/API smoke tests.
- Production routing requires Rust unit tests, C++ vs Rust parity tests for deterministic outputs, and the narrowest
  Rust-enabled subsystem validation target that covers the runtime path.
- Existing uncovered behavior is not blocked retroactively, but touching that behavior requires adding the missing tests
  for the touched surface.
