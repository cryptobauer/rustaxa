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

        fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>>;
        fn get_period_from_pbft_hash(&self, hash: &[u8; 32]) -> Result<PeriodLookup>;

        fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool>;
        fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool>;
        fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>>;
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

    fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.catch_up()?;
        self.0.pbft().pbft_block_in_db(H256::from(*hash))
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
}
