use ethereum_types::H256;
use rustaxa_storage::Config;
use rustaxa_storage::Storage as InnerStorage;
use std::path::PathBuf;

pub struct Storage(#[allow(dead_code)] InnerStorage);

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
        fn get_proposal_period_for_dag_level(&self, level: u64) -> Result<u64>;
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

        fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>>;
        fn get_period_from_pbft_hash(&self, hash: &[u8; 32]) -> Result<PeriodLookup>;
        fn get_block_receipt(&self, period: u64) -> Result<Vec<u8>>;
        fn get_pillar_block(&self, period: u64) -> Result<Vec<u8>>;
        fn get_latest_pillar_block(&self) -> Result<Vec<u8>>;
        fn get_own_pillar_block_vote(&self) -> Result<Vec<u8>>;
        fn get_current_pillar_block_data(&self) -> Result<Vec<u8>>;
        fn get_genesis_hash(&self) -> Result<Vec<u8>>;
        fn get_last_sortition_params(&self, count: u64) -> Result<Vec<BlockRlp>>;
        fn get_params_change_for_period(&self, period: u64) -> Result<Vec<u8>>;
        fn get_status_field(&self, field: u8) -> Result<u64>;
        fn get_period_lambda(&self, period: u64, find_closest: bool) -> Result<PeriodLambda>;
        fn get_rounds_count_dynamic_lambda(&self) -> Result<u32>;
        fn get_blocks_rewards_stats(&self) -> Result<Vec<PeriodRlp>>;

        fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn get_pbft_mgr_field(&self, field: u8) -> Result<u32>;
        fn get_pbft_mgr_status(&self, field: u8) -> Result<bool>;
        fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>>;
        fn get_proposed_pbft_blocks(&self) -> Result<Vec<BlockRlp>>;
        fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
        fn get_own_verified_votes(&self) -> Result<Vec<VoteRlp>>;
        fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<VoteRlp>>;
        fn get_reward_votes(&self) -> Result<Vec<VoteRlp>>;

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
    }
}

pub fn create_storage(path: &str) -> Result<Box<Storage>, anyhow::Error> {
    let path_buf = PathBuf::from(path);
    let config = Config::new(path_buf);
    let storage = InnerStorage::new(config)?;
    Ok(Box::new(Storage(storage)))
}

impl Storage {
    fn dag_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .dag()
            .dag_block_in_db(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_dag_block(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .dag()
            .dag_block_rlp(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_dag_block_period(&self, hash: &[u8; 32]) -> Result<ffi::BlockPeriod, anyhow::Error> {
        self.0.catch_up()?;
        let (period, position) = self
            .0
            .dag()
            .dag_block_period(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(ffi::BlockPeriod { period, position })
    }

    fn get_last_blocks_level(&self) -> Result<u64, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .dag()
            .last_blocks_level()
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_blocks_by_level(&self, level: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        let hashes = self
            .0
            .dag()
            .blocks_by_level(level)
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
        self.0.catch_up()?;
        let rlps = self
            .0
            .dag()
            .dag_blocks_at_level_rlp(level, number_of_levels)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rlps
            .into_iter()
            .map(|data| ffi::BlockRlp { data })
            .collect())
    }

    fn get_nonfinalized_dag_blocks(&self) -> Result<Vec<ffi::LevelBlocks>, anyhow::Error> {
        self.0.catch_up()?;
        let map = self
            .0
            .dag()
            .nonfinalized_dag_blocks_rlp()
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

    fn get_proposal_period_for_dag_level(&self, level: u64) -> Result<u64, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .dag()
            .proposal_period_for_dag_level(level)
            .map(|opt| opt.unwrap_or(0))
            .map_err(|e| anyhow::anyhow!(e))
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
            .save_dag_block(H256::from(*hash), level, tips_count, &block_rlp)
    }

    fn update_dag_block_counter(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .update_dag_block_counter(H256::from(*hash), level, tips_count)
    }

    fn remove_dag_block(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.dag().remove_dag_block(H256::from(*hash))
    }

    fn save_proposal_period_dag_levels_map(
        &self,
        level: u64,
        period: u64,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .save_proposal_period_dag_levels_map(level, period)
    }

    fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .period()
            .period_data_raw(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_period_from_pbft_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<ffi::PeriodLookup, anyhow::Error> {
        self.0.catch_up()?;
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
        self.0.catch_up()?;
        self.0
            .period()
            .block_receipt(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    fn get_pillar_block(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pillar()
            .pillar_block_rlp(period)?
            .unwrap_or_default())
    }

    fn get_latest_pillar_block(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pillar()
            .latest_pillar_block_rlp()?
            .unwrap_or_default())
    }

    fn get_own_pillar_block_vote(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pillar()
            .own_pillar_block_vote_rlp()?
            .unwrap_or_default())
    }

    fn get_current_pillar_block_data(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pillar()
            .current_pillar_block_data_rlp()?
            .unwrap_or_default())
    }

    fn get_genesis_hash(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self.0.metadata().genesis_hash_bytes()?.unwrap_or_default())
    }

    fn get_last_sortition_params(&self, count: u64) -> Result<Vec<ffi::BlockRlp>, anyhow::Error> {
        self.0.catch_up()?;

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
        self.0.catch_up()?;
        Ok(self
            .0
            .metadata()
            .params_change_for_period_rlp(period)?
            .unwrap_or_default())
    }

    fn get_status_field(&self, field: u8) -> Result<u64, anyhow::Error> {
        self.0.catch_up()?;
        self.0.metadata().status_field(field)
    }

    fn get_period_lambda(
        &self,
        period: u64,
        find_closest: bool,
    ) -> Result<ffi::PeriodLambda, anyhow::Error> {
        self.0.catch_up()?;
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
        self.0.catch_up()?;
        self.0.metadata().rounds_count_dynamic_lambda()
    }

    fn get_blocks_rewards_stats(&self) -> Result<Vec<ffi::PeriodRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let stats = self.0.metadata().block_rewards_stats_rlp()?;
        Ok(stats
            .into_iter()
            .map(|(period, data)| ffi::PeriodRlp { period, data })
            .collect())
    }

    fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        self.0.pbft().pbft_block_in_db(H256::from(*hash))
    }

    fn get_pbft_mgr_field(&self, field: u8) -> Result<u32, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self.0.pbft().pbft_mgr_field(field)?.unwrap_or(1))
    }

    fn get_pbft_mgr_status(&self, field: u8) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self.0.pbft().pbft_mgr_status(field)?.unwrap_or(false))
    }

    fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pbft()
            .cert_voted_block_in_round_rlp()?
            .unwrap_or_default())
    }

    fn get_proposed_pbft_blocks(&self) -> Result<Vec<ffi::BlockRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let blocks = self.0.pbft().proposed_pbft_blocks_rlp()?;
        Ok(blocks
            .into_iter()
            .map(|data| ffi::BlockRlp { data })
            .collect())
    }

    fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .pbft()
            .pbft_head(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_own_verified_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let votes = self.0.pbft().own_verified_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let votes = self.0.pbft().all_two_t_plus_one_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn get_reward_votes(&self) -> Result<Vec<ffi::VoteRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let votes = self.0.pbft().reward_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| ffi::VoteRlp { data })
            .collect())
    }

    fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        self.0.transaction().transaction_in_db(H256::from(*hash))
    }

    fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        self.0
            .transaction()
            .transaction_finalized(H256::from(*hash))
    }

    fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .transaction()
            .transaction_location_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
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
        self.0.catch_up()?;
        Ok(self
            .0
            .transaction()
            .transaction_by_period_position_rlp(period, position)?
            .unwrap_or_default())
    }

    fn get_transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.0.catch_up()?;
        self.0.transaction().transaction_count(period)
    }

    fn get_system_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        self.0.catch_up()?;
        Ok(self
            .0
            .transaction()
            .system_transaction_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    fn get_all_nonfinalized_transactions(&self) -> Result<Vec<ffi::TxRlp>, anyhow::Error> {
        self.0.catch_up()?;
        let trxs = self.0.transaction().all_nonfinalized_transactions_rlp()?;
        Ok(trxs.into_iter().map(|data| ffi::TxRlp { data }).collect())
    }

    fn get_all_transaction_period(&self) -> Result<Vec<ffi::HashPeriod>, anyhow::Error> {
        self.0.catch_up()?;
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
        self.0.catch_up()?;
        self.0
            .transaction()
            .period_system_transactions_hashes_rlp(period)
    }
}
