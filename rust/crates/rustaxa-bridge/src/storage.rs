use ethereum_types::H256;
use rustaxa_storage::Config;
use rustaxa_storage::Storage as InnerStorage;
use rustaxa_storage::StorageWriteBatch;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub struct Storage(
    #[allow(dead_code)] InnerStorage,
    Mutex<HashMap<u64, StorageWriteBatch>>,
    AtomicU64,
);

#[cxx::bridge(namespace = "rustaxa::storage")]
mod ffi {
    struct BlockPeriod {
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
        type Storage;
        fn create_storage(path: &str) -> Result<Box<Storage>>;
        fn create_write_batch(&self) -> Result<u64>;
        fn batch_put(&self, batch_id: u64, column: u8, key: Vec<u8>, value: Vec<u8>) -> Result<()>;
        fn batch_delete(&self, batch_id: u64, column: u8, key: Vec<u8>) -> Result<()>;
        fn commit_write_batch(&self, batch_id: u64, sync: bool) -> Result<()>;
        fn drop_write_batch(&self, batch_id: u64) -> Result<()>;

        fn dag_block_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn get_dag_block(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_dag_block_period(&self, hash: &[u8; 32]) -> Result<BlockPeriod>;
        fn get_last_blocks_level(&self) -> Result<u64>;
        fn get_blocks_by_level(&self, level: u64) -> Result<Vec<u8>>;
        fn get_dag_blocks_at_level(
            &self,
            level: u64,
            number_of_levels: u32,
        ) -> Result<Vec<BlockRlp>>;
        fn get_nonfinalized_dag_blocks(&self) -> Result<Vec<LevelBlocks>>;
        fn get_proposal_period_for_dag_level(&self, level: u64) -> Result<PeriodLookup>;
        fn save_dag_block(
            &self,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
            block_rlp: Vec<u8>,
        ) -> Result<()>;
        fn update_dag_block_counter(
            &self,
            hash: &[u8; 32],
            level: u64,
            tips_count: u64,
        ) -> Result<()>;
        fn remove_dag_block(&self, hash: &[u8; 32]) -> Result<()>;
        fn save_proposal_period_dag_levels_map(&self, level: u64, period: u64) -> Result<()>;
        fn save_dag_block_period(&self, hash: &[u8; 32], period: u64, position: u32) -> Result<()>;

        fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>>;
        fn get_period_from_pbft_hash(&self, hash: &[u8; 32]) -> Result<PeriodLookup>;
        fn get_block_receipt(&self, period: u64) -> Result<Vec<u8>>;
        fn get_final_chain_meta_value(&self, key: u32) -> Result<Vec<u8>>;
        fn get_final_chain_block_header(&self, block_number: u64) -> Result<Vec<u8>>;
        fn get_final_chain_block_hash_by_number(&self, block_number: u64) -> Result<Vec<u8>>;
        fn get_final_chain_block_number_by_hash(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_final_chain_log_blooms_chunk(&self, chunk_id: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_final_chain_receipt_by_trx_hash(&self, trx_hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn save_period_data(&self, period: u64, period_data_rlp: Vec<u8>) -> Result<()>;
        fn save_pbft_block_period(&self, hash: &[u8; 32], period: u64) -> Result<()>;
        fn get_pillar_block(&self, period: u64) -> Result<Vec<u8>>;
        fn get_latest_pillar_block(&self) -> Result<Vec<u8>>;
        fn get_own_pillar_block_vote(&self) -> Result<Vec<u8>>;
        fn get_current_pillar_block_data(&self) -> Result<Vec<u8>>;
        fn save_pillar_block(&self, period: u64, pillar_block_rlp: Vec<u8>) -> Result<()>;
        fn save_own_pillar_block_vote(&self, vote_rlp: Vec<u8>) -> Result<()>;
        fn save_current_pillar_block_data(&self, data_rlp: Vec<u8>) -> Result<()>;
        fn get_genesis_hash(&self) -> Result<Vec<u8>>;
        fn set_genesis_hash(&self, hash: &[u8; 32]) -> Result<()>;
        fn get_last_sortition_params(&self, count: u64) -> Result<Vec<BlockRlp>>;
        fn get_params_change_for_period(&self, period: u64) -> Result<Vec<u8>>;
        fn get_status_field(&self, field: u8) -> Result<u64>;
        fn save_status_field(&self, field: u8, value: u64) -> Result<()>;
        fn save_sortition_params_change(&self, period: u64, params_rlp: Vec<u8>) -> Result<()>;
        fn get_period_lambda(&self, period: u64, find_closest: bool) -> Result<PeriodLambda>;
        fn save_period_lambda(&self, period: u64, period_lambda: u32) -> Result<()>;
        fn get_rounds_count_dynamic_lambda(&self) -> Result<u32>;
        fn save_rounds_count_dynamic_lambda(&self, rounds_count: u32) -> Result<()>;
        fn get_blocks_rewards_stats(&self) -> Result<Vec<PeriodRlp>>;
        fn save_block_rewards_stats(&self, period: u64, stats_rlp: Vec<u8>) -> Result<()>;
        fn clear_block_rewards_stats(&self) -> Result<()>;

        fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn get_pbft_mgr_field(&self, field: u8) -> Result<u32>;
        fn get_pbft_mgr_status(&self, field: u8) -> Result<bool>;
        fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>>;
        fn get_proposed_pbft_blocks(&self) -> Result<Vec<BlockRlp>>;
        fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_own_verified_votes(&self) -> Result<Vec<VoteRlp>>;
        fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<VoteRlp>>;
        fn get_reward_votes(&self) -> Result<Vec<VoteRlp>>;
        fn save_cert_voted_block_in_round(&self, round: u64, block_rlp: Vec<u8>) -> Result<()>;
        fn save_proposed_pbft_block(&self, hash: &[u8; 32], block_rlp: Vec<u8>) -> Result<()>;
        fn save_pbft_mgr_field(&self, field: u8, value: u32) -> Result<()>;
        fn save_pbft_mgr_status(&self, field: u8, value: bool) -> Result<()>;
        fn save_pbft_head(&self, hash: &[u8; 32], head: Vec<u8>) -> Result<()>;
        fn save_own_verified_vote(&self, hash: &[u8; 32], vote_rlp: Vec<u8>) -> Result<()>;
        fn remove_cert_voted_block_in_round(&self) -> Result<()>;
        fn remove_proposed_pbft_block(&self, hash: &[u8; 32]) -> Result<()>;
        fn remove_own_verified_vote(&self, hash: &[u8; 32]) -> Result<()>;
        fn remove_extra_reward_vote(&self, hash: &[u8; 32]) -> Result<()>;
        fn replace_two_t_plus_one_votes(
            &self,
            vote_type: u8,
            votes_bundle_rlp: Vec<u8>,
        ) -> Result<()>;
        fn save_extra_reward_vote(&self, hash: &[u8; 32], vote_rlp: Vec<u8>) -> Result<()>;

        fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool>;
        fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_transaction_by_period_position(&self, period: u64, position: u32)
            -> Result<Vec<u8>>;
        fn get_transaction_count(&self, period: u64) -> Result<u64>;
        fn get_system_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_all_nonfinalized_transactions(&self) -> Result<Vec<TxRlp>>;
        fn get_all_transaction_period(&self) -> Result<Vec<HashPeriod>>;
        fn get_period_system_transactions_hashes(&self, period: u64) -> Result<Vec<u8>>;
        fn save_transaction(&self, hash: &[u8; 32], trx_rlp: Vec<u8>) -> Result<()>;
        fn remove_transaction(&self, hash: &[u8; 32]) -> Result<()>;
        fn save_transaction_location(
            &self,
            hash: &[u8; 32],
            period: u64,
            position: u32,
            is_system: bool,
        ) -> Result<()>;
        fn save_system_transaction(&self, hash: &[u8; 32], trx_rlp: Vec<u8>) -> Result<()>;
        fn save_period_system_transactions_hashes(
            &self,
            period: u64,
            hashes_rlp: Vec<u8>,
        ) -> Result<()>;
    }
}

pub fn create_storage(path: &str) -> Result<Box<Storage>, anyhow::Error> {
    let path_buf = PathBuf::from(path);
    let config = Config::new(path_buf);
    let storage = InnerStorage::new(config)?;
    Ok(Box::new(Storage(
        storage,
        Mutex::new(HashMap::new()),
        AtomicU64::new(1),
    )))
}

impl Storage {
    fn create_write_batch(&self) -> Result<u64, anyhow::Error> {
        let batch_id = self.2.fetch_add(1, Ordering::Relaxed);
        let mut batches = self
            .1
            .lock()
            .map_err(|_| anyhow::anyhow!("batch registry lock poisoned"))?;
        batches.insert(batch_id, self.0.create_write_batch());
        Ok(batch_id)
    }

    fn batch_put(
        &self,
        batch_id: u64,
        column: u8,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        let column = rustaxa_storage::Column::from_index(column)?;
        let mut batches = self
            .1
            .lock()
            .map_err(|_| anyhow::anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow::anyhow!("unknown batch id: {}", batch_id))?;
        self.0.batch_put_raw(batch, column, &key, &value)
    }

    fn batch_delete(&self, batch_id: u64, column: u8, key: Vec<u8>) -> Result<(), anyhow::Error> {
        let column = rustaxa_storage::Column::from_index(column)?;
        let mut batches = self
            .1
            .lock()
            .map_err(|_| anyhow::anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow::anyhow!("unknown batch id: {}", batch_id))?;
        self.0.batch_delete_raw(batch, column, &key)
    }

    fn commit_write_batch(&self, batch_id: u64, sync: bool) -> Result<(), anyhow::Error> {
        let batch = {
            let mut batches = self
                .1
                .lock()
                .map_err(|_| anyhow::anyhow!("batch registry lock poisoned"))?;
            batches
                .remove(&batch_id)
                .ok_or_else(|| anyhow::anyhow!("unknown batch id: {}", batch_id))?
        };
        self.0.commit_write_batch_with_sync(batch, sync)
    }

    fn drop_write_batch(&self, batch_id: u64) -> Result<(), anyhow::Error> {
        let mut batches = self
            .1
            .lock()
            .map_err(|_| anyhow::anyhow!("batch registry lock poisoned"))?;
        batches.remove(&batch_id);
        Ok(())
    }

    fn dag_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0
            .dag()
            .exists(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_dag_block(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .dag()
            .by_hash_rlp(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_dag_block_period(&self, hash: &[u8; 32]) -> Result<ffi::BlockPeriod, anyhow::Error> {
        let (period, position) = self
            .0
            .dag()
            .period(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(ffi::BlockPeriod { period, position })
    }

    fn get_last_blocks_level(&self) -> Result<u64, anyhow::Error> {
        self.0.dag().last_level().map_err(|e| anyhow::anyhow!(e))
    }

    fn get_blocks_by_level(&self, level: u64) -> Result<Vec<u8>, anyhow::Error> {
        let hashes = self
            .0
            .dag()
            .hashes_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        let mut bytes = Vec::with_capacity(hashes.len() * 32);
        for h in hashes {
            bytes.extend_from_slice(h.as_bytes());
        }
        Ok(bytes)
    }

    fn get_dag_blocks_at_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<ffi::BlockRlp>, anyhow::Error> {
        let rlps = self
            .0
            .dag()
            .at_level_range(level, number_of_levels)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rlps
            .into_iter()
            .map(|data| ffi::BlockRlp { data })
            .collect())
    }

    fn get_nonfinalized_dag_blocks(&self) -> Result<Vec<ffi::LevelBlocks>, anyhow::Error> {
        let map = self
            .0
            .dag()
            .non_finalized()
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(map
            .into_iter()
            .map(|(level, blocks)| ffi::LevelBlocks {
                level,
                blocks: blocks
                    .into_iter()
                    .map(|data| ffi::BlockRlp { data })
                    .collect(),
            })
            .collect())
    }

    fn get_proposal_period_for_dag_level(
        &self,
        level: u64,
    ) -> Result<ffi::PeriodLookup, anyhow::Error> {
        let period = self
            .0
            .dag()
            .proposal_period_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(match period {
            Some(period) => ffi::PeriodLookup {
                found: true,
                period,
            },
            None => ffi::PeriodLookup {
                found: false,
                period: 0,
            },
        })
    }

    fn save_dag_block(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .write(H256::from(*hash), level, tips_count, &block_rlp)
    }

    fn update_dag_block_counter(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .update_counter(H256::from(*hash), level, tips_count)
    }

    fn remove_dag_block(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.dag().remove(H256::from(*hash))
    }

    fn save_proposal_period_dag_levels_map(
        &self,
        level: u64,
        period: u64,
    ) -> Result<(), anyhow::Error> {
        self.0.dag().write_proposal_period_at_level(level, period)
    }

    fn save_dag_block_period(
        &self,
        hash: &[u8; 32],
        period: u64,
        position: u32,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .write_period(H256::from(*hash), period, position)
    }

    fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .period()
            .period_data_raw(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_period_from_pbft_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<ffi::PeriodLookup, anyhow::Error> {
        let lookup = self
            .0
            .period()
            .period_from_pbft_hash(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;

        match lookup {
            Some(period) => Ok(ffi::PeriodLookup {
                found: true,
                period,
            }),
            None => Ok(ffi::PeriodLookup {
                found: false,
                period: 0,
            }),
        }
    }

    fn get_block_receipt(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .period()
            .block_receipt(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_final_chain_meta_value(&self, key: u32) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.final_chain().meta_value(key)?.unwrap_or_default())
    }

    fn get_final_chain_block_header(&self, block_number: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_header_raw(block_number)?
            .unwrap_or_default())
    }

    fn get_final_chain_block_hash_by_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_hash_by_number(block_number)?
            .unwrap_or_default())
    }

    fn get_final_chain_block_number_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_number_by_hash(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_final_chain_log_blooms_chunk(
        &self,
        chunk_id: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .log_blooms_chunk_raw(H256::from(*chunk_id))?
            .unwrap_or_default())
    }

    fn get_final_chain_receipt_by_trx_hash(
        &self,
        trx_hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .receipt_by_trx_hash(H256::from(*trx_hash))?
            .unwrap_or_default())
    }

    fn save_period_data(&self, period: u64, period_data_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.period().save_period_data(period, &period_data_rlp)
    }

    fn save_pbft_block_period(&self, hash: &[u8; 32], period: u64) -> Result<(), anyhow::Error> {
        self.0
            .period()
            .save_pbft_block_period(H256::from(*hash), period)
    }

    fn get_pillar_block(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pillar()
            .pillar_block_rlp(period)?
            .unwrap_or_default())
    }

    fn get_latest_pillar_block(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pillar()
            .latest_pillar_block_rlp()?
            .unwrap_or_default())
    }

    fn get_own_pillar_block_vote(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pillar()
            .own_pillar_block_vote_rlp()?
            .unwrap_or_default())
    }

    fn get_current_pillar_block_data(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pillar()
            .current_pillar_block_data_rlp()?
            .unwrap_or_default())
    }

    fn save_pillar_block(
        &self,
        period: u64,
        pillar_block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0.pillar().save_pillar_block(period, &pillar_block_rlp)
    }

    fn save_own_pillar_block_vote(&self, vote_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pillar().save_own_pillar_block_vote(&vote_rlp)
    }

    fn save_current_pillar_block_data(&self, data_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pillar().save_current_pillar_block_data(&data_rlp)
    }

    fn get_genesis_hash(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.metadata().genesis_hash()?.unwrap_or_default())
    }

    fn set_genesis_hash(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.metadata().set_genesis_hash_if_empty(hash)
    }

    fn get_last_sortition_params(&self, count: u64) -> Result<Vec<ffi::BlockRlp>, anyhow::Error> {
        // C++ passes size_t across the bridge; on the same architecture, size_t and usize are equal.
        // This conversion should never fail on 32-bit or 64-bit systems.
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        let changes = self.0.metadata().last_sortition_params_changes_rlp(count)?;
        Ok(changes
            .into_iter()
            .map(|data| ffi::BlockRlp { data })
            .collect())
    }

    fn get_params_change_for_period(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .metadata()
            .params_change_for_period_rlp(period)?
            .unwrap_or_default())
    }

    fn get_status_field(&self, field: u8) -> Result<u64, anyhow::Error> {
        self.0.metadata().status_field(field)
    }

    fn save_status_field(&self, field: u8, value: u64) -> Result<(), anyhow::Error> {
        self.0.metadata().save_status_field(field, value)
    }

    fn save_sortition_params_change(
        &self,
        period: u64,
        params_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .save_sortition_params_change(period, &params_rlp)
    }

    fn get_period_lambda(
        &self,
        period: u64,
        find_closest: bool,
    ) -> Result<ffi::PeriodLambda, anyhow::Error> {
        let value = self.0.metadata().period_lambda(period, find_closest)?;
        Ok(match value {
            Some(value) => ffi::PeriodLambda { found: true, value },
            None => ffi::PeriodLambda {
                found: false,
                value: 0,
            },
        })
    }

    fn get_rounds_count_dynamic_lambda(&self) -> Result<u32, anyhow::Error> {
        self.0.metadata().rounds_count_dynamic_lambda()
    }

    fn save_period_lambda(&self, period: u64, period_lambda: u32) -> Result<(), anyhow::Error> {
        self.0.metadata().save_period_lambda(period, period_lambda)
    }

    fn save_rounds_count_dynamic_lambda(&self, rounds_count: u32) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .save_rounds_count_dynamic_lambda(rounds_count)
    }

    fn get_blocks_rewards_stats(&self) -> Result<Vec<ffi::PeriodRlp>, anyhow::Error> {
        let stats = self.0.metadata().block_rewards_stats_rlp()?;
        Ok(stats
            .into_iter()
            .map(|(period, data)| ffi::PeriodRlp { period, data })
            .collect())
    }

    fn save_block_rewards_stats(
        &self,
        period: u64,
        stats_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .save_block_rewards_stats(period, &stats_rlp)
    }

    fn clear_block_rewards_stats(&self) -> Result<(), anyhow::Error> {
        self.0.metadata().clear_block_rewards_stats()
    }

    fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.pbft().pbft_block_in_db(H256::from(*hash))
    }

    fn get_pbft_mgr_field(&self, field: u8) -> Result<u32, anyhow::Error> {
        Ok(self.0.pbft().pbft_mgr_field(field)?.unwrap_or(1))
    }

    fn get_pbft_mgr_status(&self, field: u8) -> Result<bool, anyhow::Error> {
        Ok(self.0.pbft().pbft_mgr_status(field)?.unwrap_or(false))
    }

    fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pbft()
            .cert_voted_block_in_round_rlp()?
            .unwrap_or_default())
    }

    fn get_proposed_pbft_blocks(&self) -> Result<Vec<ffi::BlockRlp>, anyhow::Error> {
        let blocks = self.0.pbft().proposed_pbft_blocks_rlp()?;
        Ok(blocks
            .into_iter()
            .map(|data| ffi::BlockRlp { data })
            .collect())
    }

    fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pbft()
            .pbft_head(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_own_verified_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().own_verified_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().all_two_t_plus_one_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn get_reward_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().reward_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn save_cert_voted_block_in_round(
        &self,
        round: u64,
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .save_cert_voted_block_in_round(round, &block_rlp)
    }

    fn save_proposed_pbft_block(
        &self,
        hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .save_proposed_pbft_block(H256::from(*hash), &block_rlp)
    }

    fn save_pbft_mgr_field(&self, field: u8, value: u32) -> Result<(), anyhow::Error> {
        self.0.pbft().save_pbft_mgr_field(field, value)
    }

    fn save_pbft_mgr_status(&self, field: u8, value: bool) -> Result<(), anyhow::Error> {
        self.0.pbft().save_pbft_mgr_status(field, value)
    }

    fn save_pbft_head(&self, hash: &[u8; 32], head: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pbft().save_pbft_head(H256::from(*hash), &head)
    }

    fn save_own_verified_vote(
        &self,
        hash: &[u8; 32],
        vote_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .save_own_verified_vote(H256::from(*hash), &vote_rlp)
    }

    fn remove_cert_voted_block_in_round(&self) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_cert_voted_block_in_round()
    }

    fn remove_proposed_pbft_block(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_proposed_pbft_block(H256::from(*hash))
    }

    fn remove_own_verified_vote(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_own_verified_vote(H256::from(*hash))
    }

    fn remove_extra_reward_vote(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_extra_reward_vote(H256::from(*hash))
    }

    fn replace_two_t_plus_one_votes(
        &self,
        vote_type: u8,
        votes_bundle_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .replace_two_t_plus_one_votes(vote_type, &votes_bundle_rlp)
    }

    fn save_extra_reward_vote(
        &self,
        hash: &[u8; 32],
        vote_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .save_extra_reward_vote(H256::from(*hash), &vote_rlp)
    }

    fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.transaction().transaction_in_db(H256::from(*hash))
    }

    fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0
            .transaction()
            .transaction_finalized(H256::from(*hash))
    }

    fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .transaction_location_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .transaction_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_transaction_by_period_position(
        &self,
        period: u64,
        position: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .transaction_by_period_position_rlp(period, position)?
            .unwrap_or_default())
    }

    fn get_transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.0.transaction().transaction_count(period)
    }

    fn get_system_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .system_transaction_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_all_nonfinalized_transactions(&self) -> Result<Vec<ffi::TxRlp>, anyhow::Error> {
        let trxs = self.0.transaction().all_nonfinalized_transactions_rlp()?;
        Ok(trxs.into_iter().map(|data| ffi::TxRlp { data }).collect())
    }

    fn get_all_transaction_period(&self) -> Result<Vec<ffi::HashPeriod>, anyhow::Error> {
        let periods = self.0.transaction().all_transaction_period()?;
        Ok(periods
            .into_iter()
            .map(|(hash, period)| {
                let mut h = [0u8; 32];
                h.copy_from_slice(hash.as_bytes());
                ffi::HashPeriod { hash: h, period }
            })
            .collect())
    }

    fn get_period_system_transactions_hashes(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .transaction()
            .period_system_transactions_hashes_rlp(period)
    }

    fn save_transaction(&self, hash: &[u8; 32], trx_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .save_transaction(H256::from(*hash), &trx_rlp)
    }

    fn remove_transaction(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.transaction().remove_transaction(H256::from(*hash))
    }

    fn save_transaction_location(
        &self,
        hash: &[u8; 32],
        period: u64,
        position: u32,
        is_system: bool,
    ) -> Result<(), anyhow::Error> {
        self.0.transaction().save_transaction_location(
            H256::from(*hash),
            period,
            position,
            is_system,
        )
    }

    fn save_system_transaction(
        &self,
        hash: &[u8; 32],
        trx_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .save_system_transaction(H256::from(*hash), &trx_rlp)
    }

    fn save_period_system_transactions_hashes(
        &self,
        period: u64,
        hashes_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .save_period_system_transactions_hashes(period, &hashes_rlp)
    }
}
