# Storage Database Overview

This document gives a high-level view of how storage is structured today in the C++ implementation and how that maps to the Rust rewrite.

It is intentionally not exhaustive. The goal is to make the system easier to reason about before we port more of it.

See also: `doc/rewrite/storage_schema_diagrams.md` for visual schema and lifecycle diagrams.

## Big Picture

The storage layer is centered around `DbStorage`, which acts as the node's durable data facade.

At a high level, the system stores three different kinds of data:

1. Non-finalized consensus data.
   Examples: pending DAG blocks, pending transactions, proposed PBFT blocks, current vote state.
2. Finalized period data.
   Once a PBFT period is finalized, its PBFT block, cert votes, DAG blocks, transactions, and pillar votes are packed into `period_data`.
3. Final-chain execution data.
   Examples: final-chain block indexes, receipts, log blooms, and metadata about the latest executed block.

The design is mostly append-and-index:
- large finalized payloads are stored once in `period_data`
- smaller index columns point into that data by period, block hash, transaction hash, or level
- mutable runtime state is kept in small dedicated columns

## Physical Layout

Under the node base path, the storage module works with two persistent directories:

- `db/`: the main RocksDB database managed by `DbStorage`
- `state_db/`: the final-chain state database used by the EVM/state layer

Important nuance:
- `DbStorage` directly opens and manages `db/`
- `state_db/` is not opened as another column family inside `DbStorage`; it is used by the final-chain state API
- snapshot loading and recovery logic treat `db/` and `state_db/` as a pair, so both are moved together during revert operations

When Rust storage is enabled today, the Rust side opens the main DB as a RocksDB secondary under:

- `.rustaxa/storage_secondary`

That allows Rust read access without conflicting with the C++ primary RocksDB owner.

## Core Design Pattern

The database is organized as a single RocksDB instance with many column families.

That gives the project a few useful properties:
- separate logical domains without creating many independent databases
- per-column comparator control for integer-keyed data
- efficient point lookups for hot paths
- iteration over one logical domain without scanning unrelated data

The schema is declared centrally in `DbStorage::Columns` on the C++ side and mirrored in `rustaxa-storage/src/config.rs` on the Rust side.

## Column Family Groups

### 1. Schema and Metadata

- `default`
- `migrations`
- `status`
- `genesis`

Purpose:
- hold versioning and migration support
- store counters and node-global status fields
- store the genesis hash

Typical data shape:
- very small key-value entries
- often keyed by enum values or a constant key such as `0`

Examples:
- `status` stores counters like executed blocks, executed transactions, DAG block count, DAG edge count, and DB version numbers
- `genesis` stores the genesis hash under key `0`

### 2. Finalized Period Data

- `period_data`
- `pbft_block_period`
- `dag_block_period`

Purpose:
- `period_data` is the main finalized bundle for a PBFT period
- the other columns are reverse indexes that map a PBFT block hash or DAG block hash back to the containing period

This is one of the most important design choices in the system.

Instead of storing finalized DAG blocks and finalized transactions as standalone primary records forever, the node stores the finalized period as one bundled object and then uses small indexes to find pieces inside that bundle.

`period_data` contains:
- PBFT block
- cert votes
- DAG blocks
- transactions
- pillar votes

This means many read APIs are actually two-stage reads:

1. use a small index to find the period or position
2. load `period_data` and decode the needed item from RLP

### 3. Non-Finalized DAG and Transaction Data

- `dag_blocks`
- `dag_blocks_level`
- `transactions`
- `trx_period`

Purpose:
- hold pending DAG blocks and transactions before finalization
- maintain indexes needed for fast lookup and traversal

The general flow is:

1. a new DAG block is inserted into `dag_blocks`
2. its level index is updated in `dag_blocks_level`
3. a new transaction is inserted into `transactions`
4. once a PBFT period finalizes, `savePeriodData` removes the now-finalized DAG blocks and transactions from their pending columns
5. reverse indexes are written into `dag_block_period` and `trx_period`
6. the full finalized payload is written once into `period_data`

This split gives the node one model for in-flight consensus data and another for finalized historical data.

### 4. PBFT Runtime and Vote State

- `pbft_mgr_round_step`
- `pbft_mgr_status`
- `cert_voted_block_in_round`
- `proposed_pbft_blocks`
- `pbft_head`
- `latest_round_own_votes`
- `latest_round_two_t_plus_one_votes`
- `extra_reward_votes`

Purpose:
- store small pieces of mutable consensus state
- persist the node's own latest round data and proposed blocks
- keep vote bundles needed across restarts

These columns are mostly not historical archives. They are operational state for the current or recent consensus rounds.

### 5. Proposal and Sortition Configuration

- `proposal_period_levels_map`
- `sortition_params_change`
- `period_lambda`
- `rounds_count_dynamic_lambda`
- `block_rewards_stats`

Purpose:
- map DAG levels to proposal periods
- store configuration changes over time
- store dynamic lambda-related state
- store reward statistics by period

Several of these columns are keyed by integer-like values and use a custom comparator so RocksDB iteration order matches numeric ordering.

That is important because some reads do nearest-previous lookups, such as:
- latest sortition params up to a period
- nearest lambda value at or before a period

### 6. Pillar Chain State

- `pillar_block`
- `current_pillar_block_data`
- `current_pillar_block_own_vote`

Purpose:
- store finalized pillar blocks by period
- store current in-progress pillar voting state

This is structurally similar to the PBFT runtime pattern: one historical column and a few small mutable state records.

### 7. System Transactions

- `system_transaction`
- `period_system_transactions`

Purpose:
- store system transactions that are not part of the normal user transaction flow
- map finalized periods to the set of included system transaction hashes

When reading all transactions for a finalized period, the code combines:
- regular transactions from `period_data`
- system transactions resolved through `period_system_transactions`

### 8. Final Chain Indexes and Receipts

- `final_chain_meta`
- `final_chain_blk_by_number`
- `final_chain_blk_hash_by_number`
- `final_chain_blk_number_by_hash`
- `final_chain_receipt_by_trx_hash`
- `final_chain_receipt_by_period`
- `final_chain_log_blooms_index`

Purpose:
- support final-chain block lookup by number and hash
- store receipt indexes
- store metadata such as the last finalized block number
- support log bloom queries

Important ownership detail:
- these columns are part of the same RocksDB schema
- but much of the read/write logic is owned by the final-chain subsystem rather than by `DbStorage` methods themselves

So from an architecture point of view, `DbStorage` is both:
- a domain storage facade for consensus data
- a shared persistence substrate used directly by higher-level modules like final-chain

## Key Encodings and Access Patterns

The design uses a mix of simple fixed-width keys and RLP-encoded values.

Common patterns:
- hashes as keys for direct lookup
- period numbers as keys for finalized objects
- small enum or integer keys for singleton state
- RLP bundles for complex values

Examples:
- `genesis[0] -> genesis_hash`
- `pillar_block[period] -> pillar_block_rlp`
- `pbft_block_period[pbft_block_hash] -> period`
- `dag_block_period[dag_block_hash] -> rlp(period, position)`
- `trx_period[trx_hash] -> rlp(period, position, is_system?)`
- `period_data[period] -> full period bundle`

This makes the database compact, but it also means many reads are decode-heavy rather than pure key-value retrieval.

## Pending vs Finalized Lifecycle

This is the most important mental model for the database.

### Before Finalization

- DAG blocks live in `dag_blocks`
- level membership lives in `dag_blocks_level`
- normal transactions live in `transactions`
- system transactions live in `system_transaction`
- consensus runtime state lives in the PBFT and vote columns

### At Finalization

`savePeriodData` performs the transition.

It:
- writes `pbft_block_period`
- removes finalized DAG blocks from `dag_blocks`
- writes `dag_block_period` for each finalized DAG block
- removes finalized transactions from `transactions`
- writes `trx_period` for each finalized transaction
- writes the full finalized bundle into `period_data`

This is the handoff point from pending storage to historical storage.

### After Finalization

Historical reads typically no longer depend on pending columns.

Instead they use:
- a period lookup
- then `period_data`
- plus a few final-chain indexes and receipt columns

This is why `period_data` is such a high-value target for the Rust read rewrite.

## Snapshots, Recovery, and Migrations

The storage system also has lifecycle support beyond normal reads and writes.

### Snapshots

Snapshots are directory-level artifacts tracked by period:
- `db<period>` is created by the RocksDB checkpoint path in `DbStorage::createSnapshot`
- `state_db<period>` is expected by the load/recover/delete logic, even though it is not created by `DbStorage::createSnapshot` itself

So the code currently behaves as if DB snapshots are paired, but only the main `db` checkpoint is created directly by this class.

Tracked snapshots can be pruned when the configured snapshot count is exceeded.

### Recovery

Recovery swaps the live `db/` and `state_db/` directories with the snapshot pair for a target period, then removes newer snapshots.

### Migrations

The schema can rebuild or drop old column families and can copy/replace columns during migrations.

This is useful operationally, but most of it is not part of the first Rust storage rewrite wave.

## State DB Relationship

`state_db/` is easy to confuse with the main RocksDB schema, but it is separate.

What is true today:
- `DbStorage` tracks the path to `state_db/`
- snapshot/recovery logic manages it together with `db/`
- final-chain opens and uses it through the state API

What is not true today:
- `state_db/` is not another column family in `DbStorage`
- the current Rust storage bridge work does not replace the state DB layer

## Rust Rewrite Implications

From a rewrite perspective, the design naturally breaks into layers.

### Current Rust Read Slice

The implemented Rust read shim coverage now includes:
- DAG read/index APIs (`dag_blocks`, `dag_blocks_level`, `dag_block_period`, `proposal_period_levels_map`)
- period-data primitives (`getPeriodDataRaw`, `getPeriodFromPbftHash`)
- PBFT block-hash presence checks and PBFT manager/vote reads (`pbftBlockInDb`, `getPbftMgrField`,
  `getPbftMgrStatus`, `getCertVotedBlockInRound`, `getProposedPbftBlocks`, `getPbftHead`,
  `getOwnVerifiedVotes`, `getAllTwoTPlusOneVotes`, `getRewardVotes`)
- transaction read primitives and retrieval paths (`transactionInDb`, `transactionFinalized`,
  `transactionsFinalized`, `getTransactionLocation`, `getTransaction`, `getTransaction(period, position)`,
  `getSystemTransaction`, `getTransactionCount`, `getAllNonfinalizedTransactions`,
  `getAllTransactionPeriod`, `getPeriodSystemTransactionsHashes`)

Current Rust-side repository split in `rustaxa-storage`:
- `DagRepository` for DAG reads and DAG indexes
- `PeriodRepository` for period bundle and period/PBFT-hash lookups
- `PbftRepository` for PBFT hash presence checks and PBFT manager/vote reads
- `TransactionRepository` for transaction presence/location and transaction read paths

The Rust `DbReader::exist` path now mirrors the C++ `exist()` pattern (`KeyMayExist` pre-check + real read to avoid false positives).

### Good first Rust targets

- DAG reads and indexes
- `period_data` reads
- transaction retrieval, location, and presence checks
- small metadata reads such as genesis and status

These are mostly local to `DbStorage` and do not require redesigning the public interface.

### Good second-wave targets

- PBFT runtime state
- pillar state
- dynamic config and statistics columns

These are still mostly standard key-value or iterator reads.

### Harder targets

- write batching across the C++ and Rust boundary
- snapshot/migration/admin APIs
- final-chain code paths that use low-level `DbStorage` helpers directly

The main difficulty is not the schema itself. The harder part is that some higher-level subsystems bypass narrow repository methods and operate directly on the underlying DB facade.

## Recommended Mental Model

If you want the shortest useful model of this database, think of it like this:

- `DbStorage` is the main RocksDB facade for node persistence
- the schema is split by domain using column families
- pending DAG and transaction data are stored separately from finalized historical data
- `period_data` is the central historical bundle for finalized consensus data
- small reverse-index columns map hashes and levels back to periods and positions
- final-chain uses the same RocksDB schema for block indexes and receipts
- `state_db/` is a sibling database used by final-chain state execution, not a column family inside `db/`

That model is enough to navigate most of the current implementation and to reason about the next Rust storage batches.
