use crate::dag::*;
use crate::final_chain::*;
use crate::storage::*;
use crate::vdf::*;
use rustaxa_consensus::dag::DagGraph;
use rustaxa_consensus::FinalChain;
use rustaxa_storage::Storage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex;

pub struct BridgeStorage(
    pub Arc<Storage>,
    pub Mutex<HashMap<u64, StorageWriteBatch>>,
    pub AtomicU64,
);

pub struct BridgeFinalChain(pub FinalChain);

pub struct BridgeDagGraph(pub DagGraph);

#[cxx::bridge(namespace = "rustaxa")]
pub mod rustaxa_ffi {
    struct BlockPeriod {
        period: u64,
        position: u32,
    }

    struct BlockPeriodLookup {
        found: bool,
        period: u64,
        position: u32,
    }

    struct BlockRlp {
        data: Vec<u8>,
    }

    struct LevelBlocks {
        level: u64,
        blocks: Vec<BlockRlp>,
    }

    struct PeriodLookup {
        found: bool,
        period: u64,
    }

    struct PeriodLambda {
        found: bool,
        value: u32,
    }

    struct PeriodRlp {
        period: u64,
        data: Vec<u8>,
    }

    struct TxRlp {
        data: Vec<u8>,
    }

    struct HashPeriod {
        hash: [u8; 32],
        period: u64,
    }

    struct VoteRlp {
        data: Vec<u8>,
    }

    struct FinalChainBlockNumberLookup {
        found: bool,
        value: u64,
    }

    struct GenesisAccount {
        address: [u8; 20],
        balance: Vec<u8>,
    }

    struct GenesisValidator {
        address: [u8; 20],
        vrf_key: [u8; 32],
        total_stake: Vec<u8>,
    }

    struct GenesisDposConfig {
        eligibility_balance_threshold: Vec<u8>,
        vote_eligibility_balance_step: Vec<u8>,
        validator_maximum_stake: Vec<u8>,
    }

    struct AccountLookup {
        found: bool,
        nonce: u64,
        balance: Vec<u8>,
        storage_root_hash: [u8; 32],
        code_hash: [u8; 32],
        code_size: u64,
    }

    struct FinalizationOutcome {
        block_header_rlp: Vec<u8>,
        receipts: Vec<ReceiptRlp>,
    }

    struct FinalizationTransaction {
        hash: [u8; 32],
        sender: [u8; 20],
        receiver_found: bool,
        receiver: [u8; 20],
        nonce: u64,
        value: Vec<u8>,
        gas_price: Vec<u8>,
        gas_limit: u64,
        data: Vec<u8>,
        rlp: Vec<u8>,
    }

    struct ReceiptRlp {
        data: Vec<u8>,
    }

    struct DagHash {
        hash: [u8; 32],
    }

    struct DagLevelHashes {
        level: u64,
        hashes: Vec<DagHash>,
    }

    struct DagOrder {
        found: bool,
        hashes: Vec<DagHash>,
    }

    extern "Rust" {
        type WesolowskiVdf;
        type CancellationToken;
        type Solution;

        pub fn make_vdf(
            lambda: u32,
            time_bits: u32,
            input: &[u8],
            modulus: &[u8],
        ) -> Box<WesolowskiVdf>;

        pub fn make_solution(proof: &[u8], output: &[u8]) -> Box<Solution>;

        pub fn make_cancellation_token() -> Box<CancellationToken>;
        pub unsafe fn make_cancellation_token_with_atomic(
            atomic_ptr: *const bool,
        ) -> Box<CancellationToken>;
        pub fn cancellation_token_cancel(token: &CancellationToken);

        pub fn prove(vdf: &WesolowskiVdf, cancelled: &CancellationToken) -> Box<Solution>;
        pub fn verify(vdf: &WesolowskiVdf, solution: &Solution) -> bool;

        pub fn solution_get_proof(solution: &Solution) -> &[u8];
        pub fn solution_get_output(solution: &Solution) -> &[u8];

        // Consensus DAG

        type BridgeDagGraph;

        pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph>;
        pub fn dag_vertex_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_edge_count(self: &BridgeDagGraph) -> usize;
        pub fn dag_has_vertex(self: &BridgeDagGraph, vertex: &[u8; 32]) -> bool;
        pub fn dag_add_vertex_edges(
            self: &mut BridgeDagGraph,
            new_vertex: &[u8; 32],
            pivot: &[u8; 32],
            tips: Vec<DagHash>,
        ) -> bool;
        pub fn dag_leaves(self: &BridgeDagGraph) -> Vec<DagHash>;
        pub fn dag_ghost_path(self: &BridgeDagGraph, root: &[u8; 32]) -> Vec<DagHash>;
        pub fn dag_compute_order(
            self: &BridgeDagGraph,
            anchor: &[u8; 32],
            non_finalized_blocks: Vec<DagLevelHashes>,
        ) -> DagOrder;
        pub fn dag_clear(self: &mut BridgeDagGraph);
        pub fn dag_graphviz_dot(self: &BridgeDagGraph) -> String;

        // Storage

        type BridgeStorage;

        pub fn create_storage(path: &str) -> Result<Box<BridgeStorage>>;
        pub fn create_write_batch(self: &BridgeStorage) -> Result<u64>;
        pub fn batch_put(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
            value: Vec<u8>,
        ) -> Result<()>;
        pub fn batch_delete(
            self: &BridgeStorage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
        ) -> Result<()>;
        pub fn commit_write_batch(self: &BridgeStorage, batch_id: u64, sync: bool) -> Result<()>;
        pub fn drop_write_batch(self: &BridgeStorage, batch_id: u64) -> Result<()>;

        pub fn dag_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period(self: &BridgeStorage, hash: &[u8; 32]) -> Result<BlockPeriod>;
        pub fn get_dag_block_period_lookup(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &BridgeStorage) -> Result<u64>;
        pub fn get_blocks_by_level(self: &BridgeStorage, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &BridgeStorage,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_nonfinalized_dag_blocks(self: &BridgeStorage) -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &BridgeStorage,
            level: u64,
        ) -> Result<PeriodLookup>;
        pub fn save_dag_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn update_dag_block_counter(
            self: &BridgeStorage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
        ) -> Result<()>;
        pub fn remove_dag_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_proposal_period_dag_levels_map(
            self: &BridgeStorage,
            level: u64,
            period: u64,
        ) -> Result<()>;
        pub fn save_dag_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
        ) -> Result<()>;

        pub fn get_period_data_raw(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_period_from_pbft_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<PeriodLookup>;
        pub fn get_block_receipt(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(self: &BridgeStorage, key: u32) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &BridgeStorage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &BridgeStorage,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &BridgeStorage,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &BridgeStorage,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn save_period_data(
            self: &BridgeStorage,
            period: u64,
            period_data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_block_period(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
        ) -> Result<()>;
        pub fn get_pillar_block(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_latest_pillar_block(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_own_pillar_block_vote(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_current_pillar_block_data(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn save_pillar_block(
            self: &BridgeStorage,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_own_pillar_block_vote(self: &BridgeStorage, vote_rlp: Vec<u8>) -> Result<()>;
        pub fn save_current_pillar_block_data(
            self: &BridgeStorage,
            data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_genesis_hash(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn set_genesis_hash(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn get_last_sortition_params(self: &BridgeStorage, count: u64)
            -> Result<Vec<BlockRlp>>;
        pub fn get_params_change_for_period(self: &BridgeStorage, period: u64) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &BridgeStorage, field: u8) -> Result<u64>;
        pub fn save_status_field(self: &BridgeStorage, field: u8, value: u64) -> Result<()>;
        pub fn save_sortition_params_change(
            self: &BridgeStorage,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_period_lambda(
            self: &BridgeStorage,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn save_period_lambda(
            self: &BridgeStorage,
            period: u64,
            period_lambda: u32,
        ) -> Result<()>;
        pub fn get_rounds_count_dynamic_lambda(self: &BridgeStorage) -> Result<u32>;
        pub fn save_rounds_count_dynamic_lambda(
            self: &BridgeStorage,
            rounds_count: u32,
        ) -> Result<()>;
        pub fn get_blocks_rewards_stats(self: &BridgeStorage) -> Result<Vec<PeriodRlp>>;
        pub fn save_block_rewards_stats(
            self: &BridgeStorage,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn clear_block_rewards_stats(self: &BridgeStorage) -> Result<()>;

        pub fn pbft_block_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &BridgeStorage, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &BridgeStorage, field: u8) -> Result<bool>;
        pub fn get_cert_voted_block_in_round(self: &BridgeStorage) -> Result<Vec<u8>>;
        pub fn get_proposed_pbft_blocks(self: &BridgeStorage) -> Result<Vec<BlockRlp>>;
        pub fn get_pbft_head(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_own_verified_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_all_two_t_plus_one_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn get_reward_votes(self: &BridgeStorage) -> Result<Vec<VoteRlp>>;
        pub fn save_cert_voted_block_in_round(
            self: &BridgeStorage,
            round: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_proposed_pbft_block(
            self: &BridgeStorage,
            hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_mgr_field(self: &BridgeStorage, field: u8, value: u32) -> Result<()>;
        pub fn save_pbft_mgr_status(self: &BridgeStorage, field: u8, value: bool) -> Result<()>;
        pub fn save_pbft_head(self: &BridgeStorage, hash: &[u8; 32], head: Vec<u8>) -> Result<()>;
        pub fn save_own_verified_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_cert_voted_block_in_round(self: &BridgeStorage) -> Result<()>;
        pub fn remove_proposed_pbft_block(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_own_verified_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_extra_reward_vote(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn replace_two_t_plus_one_votes(
            self: &BridgeStorage,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_extra_reward_vote(
            self: &BridgeStorage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;

        pub fn transaction_in_db(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn transaction_finalized(self: &BridgeStorage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_transaction_location(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &BridgeStorage,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeStorage, period: u64) -> Result<u64>;
        pub fn get_system_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(self: &BridgeStorage) -> Result<Vec<TxRlp>>;
        pub fn get_all_transaction_period(self: &BridgeStorage) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn save_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_transaction(self: &BridgeStorage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_transaction_location(
            self: &BridgeStorage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        pub fn save_system_transaction(
            self: &BridgeStorage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_period_system_transactions_hashes(
            self: &BridgeStorage,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;

        // FinalChain

        type BridgeFinalChain;

        pub fn create_final_chain(
            storage: &BridgeStorage,
            block_gas_limit: u64,
            genesis_timestamp: u64,
            genesis_accounts: Vec<GenesisAccount>,
            genesis_validators: Vec<GenesisValidator>,
            genesis_dpos_config: GenesisDposConfig,
        ) -> Result<Box<BridgeFinalChain>>;

        pub fn get_last_block_number(self: &BridgeFinalChain) -> Result<u64>;
        pub fn get_block_number(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<FinalChainBlockNumberLookup>;
        pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_block_header(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
        pub fn get_transaction_location(
            self: &BridgeFinalChain,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &BridgeFinalChain, period: u64) -> Result<u64>;
        pub fn get_account(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<AccountLookup>;
        pub fn get_dpos_eligible_vote_count(
            self: &BridgeFinalChain,
            address: &[u8; 20],
        ) -> Result<u64>;
        pub fn get_dpos_eligible_total_vote_count(self: &BridgeFinalChain) -> Result<u64>;
        pub fn get_dpos_is_eligible(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<bool>;
        pub fn get_vrf_key(self: &BridgeFinalChain, address: &[u8; 20]) -> Result<Vec<u8>>;
        pub fn estimate_call_gas(self: &BridgeFinalChain, gas_limit: u64) -> Result<u64>;
        pub fn finalize_block(
            self: &BridgeFinalChain,
            pbft_block_rlp: Vec<u8>,
            transactions: Vec<FinalizationTransaction>,
        ) -> Result<FinalizationOutcome>;
        pub fn get_transaction_rlps(self: &BridgeFinalChain, period: u64) -> Result<Vec<TxRlp>>;
        pub fn get_transaction_receipt(
            self: &BridgeFinalChain,
            period: u64,
            position: u64,
        ) -> Result<Vec<u8>>;
    }
}
