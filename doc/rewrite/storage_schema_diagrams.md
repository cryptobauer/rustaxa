# Storage Schema Diagrams

This document visualizes the storage layout described in `doc/rewrite/storage_database_overview.md`.

These diagrams are meant to be architectural aids, not byte-accurate schema specifications. They show how the database is organized, how data flows through it, and how higher-level subsystems depend on it.

## Diagram 1: Physical Storage Layout

```mermaid
flowchart LR
    NodeBase[Node base path]

    subgraph MainStorage[Main storage files]
        DB[(db/ RocksDB primary)]
        StateDB[(state_db/ sibling state DB)]
        RustSecondary[(.rustaxa/storage_secondary)]
    end

    subgraph Owners[Runtime owners]
        CppDb[DbStorage C++ facade]
        FinalChain[FinalChain and State API]
        RustBridge[Rust storage bridge]
    end

    NodeBase --> DB
    NodeBase --> StateDB
    NodeBase --> RustSecondary

    CppDb --> DB
    FinalChain --> StateDB
    FinalChain --> DB
    RustBridge --> RustSecondary
    RustSecondary -. secondary read view .-> DB

    CppDb -. path only .-> StateDB
```

## Diagram 2: Full Column Family Map

```mermaid
flowchart TB
    DB[(db RocksDB)]

    subgraph Metadata[Schema and metadata]
        default_cf[default]
        migrations_cf[migrations]
        genesis_cf[genesis]
        status_cf[status]
    end

    subgraph Finalized[Finalized period data]
        period_data_cf[period_data]
        pbft_block_period_cf[pbft_block_period]
        dag_block_period_cf[dag_block_period]
    end

    subgraph Pending[Non-finalized DAG and transactions]
        dag_blocks_cf[dag_blocks]
        dag_blocks_level_cf[dag_blocks_level]
        transactions_cf[transactions]
        trx_period_cf[trx_period]
    end

    subgraph PbftRuntime[PBFT runtime and votes]
        pbft_mgr_round_step_cf[pbft_mgr_round_step]
        pbft_mgr_status_cf[pbft_mgr_status]
        cert_voted_block_in_round_cf[cert_voted_block_in_round]
        proposed_pbft_blocks_cf[proposed_pbft_blocks]
        pbft_head_cf[pbft_head]
        latest_round_own_votes_cf[latest_round_own_votes]
        latest_round_two_t_plus_one_votes_cf[latest_round_two_t_plus_one_votes]
        extra_reward_votes_cf[extra_reward_votes]
    end

    subgraph ConfigStats[Proposal, sortition, lambda, rewards]
        proposal_period_levels_map_cf[proposal_period_levels_map]
        sortition_params_change_cf[sortition_params_change]
        period_lambda_cf[period_lambda]
        rounds_count_dynamic_lambda_cf[rounds_count_dynamic_lambda]
        block_rewards_stats_cf[block_rewards_stats]
    end

    subgraph Pillar[Pillar chain]
        pillar_block_cf[pillar_block]
        current_pillar_block_data_cf[current_pillar_block_data]
        current_pillar_block_own_vote_cf[current_pillar_block_own_vote]
    end

    subgraph SystemTx[System transactions]
        system_transaction_cf[system_transaction]
        period_system_transactions_cf[period_system_transactions]
    end

    subgraph FinalChain[Final-chain indexes and receipts]
        final_chain_meta_cf[final_chain_meta]
        final_chain_blk_by_number_cf[final_chain_blk_by_number]
        final_chain_blk_hash_by_number_cf[final_chain_blk_hash_by_number]
        final_chain_blk_number_by_hash_cf[final_chain_blk_number_by_hash]
        final_chain_receipt_by_trx_hash_cf[final_chain_receipt_by_trx_hash]
        final_chain_receipt_by_period_cf[final_chain_receipt_by_period]
        final_chain_log_blooms_index_cf[final_chain_log_blooms_index]
    end

    DB --> Metadata
    DB --> Finalized
    DB --> Pending
    DB --> PbftRuntime
    DB --> ConfigStats
    DB --> Pillar
    DB --> SystemTx
    DB --> FinalChain
```

## Diagram 3: Core Index Relationships

```mermaid
flowchart LR
    DagHash[dag block hash]
    PbftHash[pbft block hash]
    TrxHash[transaction hash]
    Level[level]
    Period[period]

    DagHash --> dag_blocks_cf[dag_blocks]
    DagHash --> dag_block_period_cf[dag_block_period]
    PbftHash --> pbft_block_period_cf[pbft_block_period]
    TrxHash --> transactions_cf[transactions]
    TrxHash --> trx_period_cf[trx_period]
    Level --> dag_blocks_level_cf[dag_blocks_level]
    Level --> proposal_period_levels_map_cf[proposal_period_levels_map]
    Period --> period_data_cf[period_data]
    Period --> pillar_block_cf[pillar_block]
    Period --> period_system_transactions_cf[period_system_transactions]
    Period --> final_chain_receipt_by_period_cf[final_chain_receipt_by_period]
    Period --> period_lambda_cf[period_lambda]
    Period --> sortition_params_change_cf[sortition_params_change]
    Period --> block_rewards_stats_cf[block_rewards_stats]

    pbft_block_period_cf --> period_data_cf
    dag_block_period_cf --> period_data_cf
    trx_period_cf --> period_data_cf
    period_system_transactions_cf --> system_transaction_cf
    dag_blocks_level_cf --> dag_blocks_cf
    proposal_period_levels_map_cf --> period_data_cf
```

## Diagram 4: Pending to Finalized Lifecycle

```mermaid
flowchart TB
    subgraph BeforeFinalization[Before finalization]
        dag_blocks_cf[dag_blocks]
        dag_blocks_level_cf[dag_blocks_level]
        transactions_cf[transactions]
        system_transaction_cf[system_transaction]
        period_system_transactions_cf[period_system_transactions]
    end

    SavePeriodData[savePeriodData period batch]

    subgraph AfterFinalization[After finalization]
        period_data_cf[period_data]
        pbft_block_period_cf[pbft_block_period]
        dag_block_period_cf[dag_block_period]
        trx_period_cf[trx_period]
    end

    dag_blocks_cf --> SavePeriodData
    dag_blocks_level_cf --> SavePeriodData
    transactions_cf --> SavePeriodData
    system_transaction_cf --> SavePeriodData
    period_system_transactions_cf --> SavePeriodData

    SavePeriodData -->|write full finalized bundle| period_data_cf
    SavePeriodData -->|index PBFT hash to period| pbft_block_period_cf
    SavePeriodData -->|index DAG hash to period and position| dag_block_period_cf
    SavePeriodData -->|index transaction hash to period and position| trx_period_cf

    SavePeriodData -. remove finalized pending DAG blocks .-> dag_blocks_cf
    SavePeriodData -. remove finalized pending transactions .-> transactions_cf
```

## Diagram 5: Typical Read Paths

```mermaid
flowchart LR
    DagLookup[getDagBlock by hash] --> DagPeriod{present in dag_blocks?}
    DagPeriod -->|yes| dag_blocks_cf[dag_blocks]
    DagPeriod -->|no| dag_block_period_cf[dag_block_period]
    dag_block_period_cf --> period_data_cf[period_data]
    period_data_cf --> DecodeDag[decode DAG block from bundle]

    TrxLookup[getTransaction by hash] --> TrxPending{present in transactions?}
    TrxPending -->|yes| transactions_cf[transactions]
    TrxPending -->|no| trx_period_cf[trx_period]
    trx_period_cf --> TrxSystem{system transaction?}
    TrxSystem -->|no| period_data_cf
    TrxSystem -->|yes| system_transaction_cf[system_transaction]
    period_data_cf --> DecodeTrx[decode transaction from bundle]

    PbftLookup[getPbftBlock by hash] --> pbft_block_period_cf[pbft_block_period]
    pbft_block_period_cf --> period_data_cf
    period_data_cf --> DecodePbft[decode PBFT block from bundle]
```

## Diagram 6: Final-Chain Relationship to DbStorage

```mermaid
flowchart LR
    subgraph DbStorageDomain[DbStorage-managed schema]
        period_data_cf[period_data]
        status_cf[status]
        final_chain_meta_cf[final_chain_meta]
        final_chain_blk_by_number_cf[final_chain_blk_by_number]
        final_chain_blk_hash_by_number_cf[final_chain_blk_hash_by_number]
        final_chain_blk_number_by_hash_cf[final_chain_blk_number_by_hash]
        final_chain_receipt_by_trx_hash_cf[final_chain_receipt_by_trx_hash]
        final_chain_receipt_by_period_cf[final_chain_receipt_by_period]
        final_chain_log_blooms_index_cf[final_chain_log_blooms_index]
    end

    FinalChain[FinalChain subsystem]
    StateAPI[State API]
    StateDB[(state_db)]

    FinalChain --> period_data_cf
    FinalChain --> status_cf
    FinalChain --> final_chain_meta_cf
    FinalChain --> final_chain_blk_by_number_cf
    FinalChain --> final_chain_blk_hash_by_number_cf
    FinalChain --> final_chain_blk_number_by_hash_cf
    FinalChain --> final_chain_receipt_by_trx_hash_cf
    FinalChain --> final_chain_receipt_by_period_cf
    FinalChain --> final_chain_log_blooms_index_cf

    FinalChain --> StateAPI
    StateAPI --> StateDB
```

## Diagram 7: Snapshot and Recovery Model

```mermaid
flowchart TB
    subgraph Live[Live directories]
        live_db[(db/)]
        live_state[(state_db/)]
    end

    subgraph SnapshotSet[Snapshot artifacts by period]
        snap_db[(dbN)]
        snap_state[(state_dbN)]
    end

    CreateSnapshot[createSnapshot for period]
    LoadSnapshots[loadSnapshots]
    Recover[recoverToPeriod]
    DeleteSnapshot[deleteSnapshot]

    live_db --> CreateSnapshot --> snap_db
    snap_db --> LoadSnapshots
    snap_state --> LoadSnapshots

    snap_db --> Recover --> live_db
    snap_state --> Recover --> live_state

    Recover --> DeleteSnapshot
    DeleteSnapshot --> snap_db
    DeleteSnapshot --> snap_state
```

## Diagram 8: Rust Rewrite Insertion Point

```mermaid
flowchart LR
    Callers[Consensus, PBFT, DAG, FinalChain, RPC]
    DbStorageCpp[DbStorage public C++ API]
    LegacyDb[legacy C++ RocksDB reads via db_]
    RustShim[rust_storage_ bridge]
    RustRepos[Rust repositories over RocksDB secondary]
    MainDb[(db/ primary)]

    Callers --> DbStorageCpp
    DbStorageCpp --> LegacyDb
    DbStorageCpp --> RustShim
    LegacyDb --> MainDb
    RustShim --> RustRepos
    RustRepos -. secondary catch-up and reads .-> MainDb
```

Current implemented Rust repositories behind this insertion point:
- `DagRepository`
- `PeriodRepository`
- `PbftRepository` (currently used for `pbftBlockInDb` checks)

## Reading Order Suggestion

If you are trying to understand the storage module from scratch, the diagrams are easiest to consume in this order:

1. Physical storage layout
2. Full column family map
3. Pending to finalized lifecycle
4. Typical read paths
5. Final-chain relationship
6. Snapshot and recovery model
7. Rust rewrite insertion point

That sequence matches how the code is structured: first the database exists, then data is grouped, then it moves through the system, then higher-level subsystems consume it.
