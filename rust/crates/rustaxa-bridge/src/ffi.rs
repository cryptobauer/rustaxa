use crate::final_chain::*;
use crate::storage::*;
use crate::vdf::*;
use rustaxa_consensus::FinalChain;
use rustaxa_storage::Storage as InnerStorage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::sync::Mutex;

pub struct Storage(
    pub Arc<InnerStorage>,
    pub Mutex<HashMap<u64, StorageWriteBatch>>,
    pub AtomicU64,
);

pub struct BridgeFinalChain(pub FinalChain);

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

        // Storage

        type Storage;

        pub fn create_storage(path: &str) -> Result<Box<Storage>>;
        pub fn create_write_batch(self: &Storage) -> Result<u64>;
        pub fn batch_put(
            self: &Storage,
            batch_id: u64,
            column: u8,
            key: Vec<u8>,
            value: Vec<u8>,
        ) -> Result<()>;
        pub fn batch_delete(self: &Storage, batch_id: u64, column: u8, key: Vec<u8>) -> Result<()>;
        pub fn commit_write_batch(self: &Storage, batch_id: u64, sync: bool) -> Result<()>;
        pub fn drop_write_batch(self: &Storage, batch_id: u64) -> Result<()>;

        pub fn dag_block_in_db(self: &Storage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_dag_block(self: &Storage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_dag_block_period(self: &Storage, hash: &[u8; 32]) -> Result<BlockPeriod>;
        pub fn get_dag_block_period_lookup(
            self: &Storage,
            hash: &[u8; 32],
        ) -> Result<BlockPeriodLookup>;
        pub fn get_last_blocks_level(self: &Storage) -> Result<u64>;
        pub fn get_blocks_by_level(self: &Storage, level: u64) -> Result<Vec<u8>>;
        pub fn get_dag_blocks_at_level(
            self: &Storage,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        pub fn get_nonfinalized_dag_blocks(self: &Storage) -> Result<Vec<LevelBlocks>>;
        pub fn get_proposal_period_for_dag_level(
            self: &Storage,
            level: u64,
        ) -> Result<PeriodLookup>;
        pub fn save_dag_block(
            self: &Storage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn update_dag_block_counter(
            self: &Storage,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
        ) -> Result<()>;
        pub fn remove_dag_block(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_proposal_period_dag_levels_map(
            self: &Storage,
            level: u64,
            period: u64,
        ) -> Result<()>;
        pub fn save_dag_block_period(
            self: &Storage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
        ) -> Result<()>;

        pub fn get_period_data_raw(self: &Storage, period: u64) -> Result<Vec<u8>>;
        pub fn get_period_from_pbft_hash(self: &Storage, hash: &[u8; 32]) -> Result<PeriodLookup>;
        pub fn get_block_receipt(self: &Storage, period: u64) -> Result<Vec<u8>>;
        pub fn get_final_chain_meta_value(self: &Storage, key: u32) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_header(self: &Storage, block_number: u64) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_hash_by_number(
            self: &Storage,
            block_number: u64,
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_block_number_by_hash(
            self: &Storage,
            hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_log_blooms_chunk(
            self: &Storage,
            chunk_id: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn get_final_chain_receipt_by_trx_hash(
            self: &Storage,
            trx_hash: &[u8; 32],
        ) -> Result<Vec<u8>>;
        pub fn save_period_data(
            self: &Storage,
            period: u64,
            period_data_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_block_period(self: &Storage, hash: &[u8; 32], period: u64) -> Result<()>;
        pub fn get_pillar_block(self: &Storage, period: u64) -> Result<Vec<u8>>;
        pub fn get_latest_pillar_block(self: &Storage) -> Result<Vec<u8>>;
        pub fn get_own_pillar_block_vote(self: &Storage) -> Result<Vec<u8>>;
        pub fn get_current_pillar_block_data(self: &Storage) -> Result<Vec<u8>>;
        pub fn save_pillar_block(
            self: &Storage,
            period: u64,
            pillar_block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_own_pillar_block_vote(self: &Storage, vote_rlp: Vec<u8>) -> Result<()>;
        pub fn save_current_pillar_block_data(self: &Storage, data_rlp: Vec<u8>) -> Result<()>;
        pub fn get_genesis_hash(self: &Storage) -> Result<Vec<u8>>;
        pub fn set_genesis_hash(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn get_last_sortition_params(self: &Storage, count: u64) -> Result<Vec<BlockRlp>>;
        pub fn get_params_change_for_period(self: &Storage, period: u64) -> Result<Vec<u8>>;
        pub fn get_status_field(self: &Storage, field: u8) -> Result<u64>;
        pub fn save_status_field(self: &Storage, field: u8, value: u64) -> Result<()>;
        pub fn save_sortition_params_change(
            self: &Storage,
            period: u64,
            params_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn get_period_lambda(
            self: &Storage,
            period: u64,
            find_closest: bool,
        ) -> Result<PeriodLambda>;
        pub fn save_period_lambda(self: &Storage, period: u64, period_lambda: u32) -> Result<()>;
        pub fn get_rounds_count_dynamic_lambda(self: &Storage) -> Result<u32>;
        pub fn save_rounds_count_dynamic_lambda(self: &Storage, rounds_count: u32) -> Result<()>;
        pub fn get_blocks_rewards_stats(self: &Storage) -> Result<Vec<PeriodRlp>>;
        pub fn save_block_rewards_stats(
            self: &Storage,
            period: u64,
            stats_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn clear_block_rewards_stats(self: &Storage) -> Result<()>;

        pub fn pbft_block_in_db(self: &Storage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_pbft_mgr_field(self: &Storage, field: u8) -> Result<u32>;
        pub fn get_pbft_mgr_status(self: &Storage, field: u8) -> Result<bool>;
        pub fn get_cert_voted_block_in_round(self: &Storage) -> Result<Vec<u8>>;
        pub fn get_proposed_pbft_blocks(self: &Storage) -> Result<Vec<BlockRlp>>;
        pub fn get_pbft_head(self: &Storage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_own_verified_votes(self: &Storage) -> Result<Vec<VoteRlp>>;
        pub fn get_all_two_t_plus_one_votes(self: &Storage) -> Result<Vec<VoteRlp>>;
        pub fn get_reward_votes(self: &Storage) -> Result<Vec<VoteRlp>>;
        pub fn save_cert_voted_block_in_round(
            self: &Storage,
            round: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_proposed_pbft_block(
            self: &Storage,
            hash: &[u8; 32],
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_pbft_mgr_field(self: &Storage, field: u8, value: u32) -> Result<()>;
        pub fn save_pbft_mgr_status(self: &Storage, field: u8, value: bool) -> Result<()>;
        pub fn save_pbft_head(self: &Storage, hash: &[u8; 32], head: Vec<u8>) -> Result<()>;
        pub fn save_own_verified_vote(
            self: &Storage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn remove_cert_voted_block_in_round(self: &Storage) -> Result<()>;
        pub fn remove_proposed_pbft_block(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_own_verified_vote(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn remove_extra_reward_vote(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn replace_two_t_plus_one_votes(
            self: &Storage,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_extra_reward_vote(
            self: &Storage,
            hash: &[u8; 32],
            vote_rlp: Vec<u8>,
        ) -> Result<()>;

        pub fn transaction_in_db(self: &Storage, hash: &[u8; 32]) -> Result<bool>;
        pub fn transaction_finalized(self: &Storage, hash: &[u8; 32]) -> Result<bool>;
        pub fn get_transaction_location(self: &Storage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction(self: &Storage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_transaction_by_period_position(
            self: &Storage,
            period: u64,
            position: u32,
        ) -> Result<Vec<u8>>;
        pub fn get_transaction_count(self: &Storage, period: u64) -> Result<u64>;
        pub fn get_system_transaction(self: &Storage, hash: &[u8; 32]) -> Result<Vec<u8>>;
        pub fn get_all_nonfinalized_transactions(self: &Storage) -> Result<Vec<TxRlp>>;
        pub fn get_all_transaction_period(self: &Storage) -> Result<Vec<HashPeriod>>;
        pub fn get_period_system_transactions_hashes(
            self: &Storage,
            period: u64,
        ) -> Result<Vec<u8>>;
        pub fn save_transaction(self: &Storage, hash: &[u8; 32], trx_rlp: Vec<u8>) -> Result<()>;
        pub fn remove_transaction(self: &Storage, hash: &[u8; 32]) -> Result<()>;
        pub fn save_transaction_location(
            self: &Storage,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        pub fn save_system_transaction(
            self: &Storage,
            hash: &[u8; 32],
            trx_rlp: Vec<u8>,
        ) -> Result<()>;
        pub fn save_period_system_transactions_hashes(
            self: &Storage,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;

        // FinalChain

        type BridgeFinalChain;

        pub fn create_final_chain(storage: &Storage) -> Result<Box<BridgeFinalChain>>;

        pub fn get_block_hash(self: &BridgeFinalChain, num: u64) -> Result<Vec<u8>>;
    }
}
