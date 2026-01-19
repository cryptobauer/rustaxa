use anyhow::Result;
use std::cmp::Ordering;
use std::path::PathBuf;

use crate::StorageError;

/// Configuration for the database connection.
#[derive(Debug, Clone)]
pub struct Config {
    pub db_path: PathBuf,
    pub state_path: PathBuf,

    pub create_if_missing: bool,
    pub create_missing_column_families: bool,
    pub compression: rocksdb::DBCompressionType,
    pub max_total_wal_size: u64,
    pub db_write_buffer_size: usize,
    pub max_open_files: i32,

    pub column_families: Vec<Column>,
}

impl Config {
    /// Creates a new storage configuration with the specified information.
    pub fn new(base_path: PathBuf) -> Self {
        Config {
            db_path: base_path.join("db"),
            state_path: base_path.join("state"),
            // TODO: make configurable via toml.
            create_if_missing: true,
            create_missing_column_families: true,
            compression: rocksdb::DBCompressionType::Lz4,
            max_total_wal_size: 1024 * 1024 * 1024, // 1GB
            db_write_buffer_size: 2 * 1024 * 1024 * 1024, // 2GB
            max_open_files: 256,
            column_families: Column::all().to_vec(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    DefaultColumn,
    Migrations,
    PeriodData,
    Genesis,
    DagBlocks,
    DagBlocksLevel,
    Transactions,
    TrxPeriod,
    Status,
    PbftMgrRoundStep,
    PbftMgrStatus,
    CertVotedBlockInRound,
    ProposedPbftBlocks,
    PbftHead,
    LatestRoundOwnVotes,
    LatestRoundTwoTPlusOneVotes,
    ExtraRewardVotes,
    PbftBlockPeriod,
    DagBlockPeriod,
    ProposalPeriodLevelsMap,
    FinalChainMeta,
    FinalChainBlkByNumber,
    FinalChainBlkHashByNumber,
    FinalChainBlkNumberByHash,
    FinalChainReceiptByTrxHash,
    FinalChainLogBloomsIndex,
    SortitionParamsChange,
    BlockRewardsStats,
    PillarBlock,
    CurrentPillarBlockData,
    CurrentPillarBlockOwnVote,
    SystemTransaction,
    PeriodSystemTransactions,
    FinalChainReceiptByPeriod,
    PeriodLambda,
    RoundsCountDynamicLambda,
}

type ComparatorFn = Box<dyn Fn(&[u8], &[u8]) -> Ordering>;

impl Column {
    /// Returns the column family name.
    pub fn name(&self) -> &'static str {
        match self {
            Column::DefaultColumn => "default_column",
            Column::Migrations => "migrations",
            Column::PeriodData => "period_data",
            Column::Genesis => "genesis",
            Column::DagBlocks => "dag_blocks",
            Column::DagBlocksLevel => "dag_blocks_level",
            Column::Transactions => "transactions",
            Column::TrxPeriod => "trx_period",
            Column::Status => "status",
            Column::PbftMgrRoundStep => "pbft_mgr_round_step",
            Column::PbftMgrStatus => "pbft_mgr_status",
            Column::CertVotedBlockInRound => "cert_voted_block_in_round",
            Column::ProposedPbftBlocks => "proposed_pbft_blocks",
            Column::PbftHead => "pbft_head",
            Column::LatestRoundOwnVotes => "latest_round_own_votes",
            Column::LatestRoundTwoTPlusOneVotes => "latest_round_two_t_plus_one_votes",
            Column::ExtraRewardVotes => "extra_reward_votes",
            Column::PbftBlockPeriod => "pbft_block_period",
            Column::DagBlockPeriod => "dag_block_period",
            Column::ProposalPeriodLevelsMap => "proposal_period_levels_map",
            Column::FinalChainMeta => "final_chain_meta",
            Column::FinalChainBlkByNumber => "final_chain_blk_by_number",
            Column::FinalChainBlkHashByNumber => "final_chain_blk_hash_by_number",
            Column::FinalChainBlkNumberByHash => "final_chain_blk_number_by_hash",
            Column::FinalChainReceiptByTrxHash => "final_chain_receipt_by_trx_hash",
            Column::FinalChainLogBloomsIndex => "final_chain_log_blooms_index",
            Column::SortitionParamsChange => "sortition_params_change",
            Column::BlockRewardsStats => "block_rewards_stats",
            Column::PillarBlock => "pillar_block",
            Column::CurrentPillarBlockData => "current_pillar_block_data",
            Column::CurrentPillarBlockOwnVote => "current_pillar_block_own_vote",
            Column::SystemTransaction => "system_transaction",
            Column::PeriodSystemTransactions => "period_system_transactions",
            Column::FinalChainReceiptByPeriod => "final_chain_receipt_by_period",
            Column::PeriodLambda => "period_lambda",
            Column::RoundsCountDynamicLambda => "rounds_count_dynamic_lambda",
        }
    }

    /// Returns all columns in order.
    pub fn all() -> &'static [Column] {
        &[
            Column::DefaultColumn,
            Column::Migrations,
            Column::PeriodData,
            Column::Genesis,
            Column::DagBlocks,
            Column::DagBlocksLevel,
            Column::Transactions,
            Column::TrxPeriod,
            Column::Status,
            Column::PbftMgrRoundStep,
            Column::PbftMgrStatus,
            Column::CertVotedBlockInRound,
            Column::ProposedPbftBlocks,
            Column::PbftHead,
            Column::LatestRoundOwnVotes,
            Column::LatestRoundTwoTPlusOneVotes,
            Column::ExtraRewardVotes,
            Column::PbftBlockPeriod,
            Column::DagBlockPeriod,
            Column::ProposalPeriodLevelsMap,
            Column::FinalChainMeta,
            Column::FinalChainBlkByNumber,
            Column::FinalChainBlkHashByNumber,
            Column::FinalChainBlkNumberByHash,
            Column::FinalChainReceiptByTrxHash,
            Column::FinalChainLogBloomsIndex,
            Column::SortitionParamsChange,
            Column::BlockRewardsStats,
            Column::PillarBlock,
            Column::CurrentPillarBlockData,
            Column::CurrentPillarBlockOwnVote,
            Column::SystemTransaction,
            Column::PeriodSystemTransactions,
            Column::FinalChainReceiptByPeriod,
            Column::PeriodLambda,
            Column::RoundsCountDynamicLambda,
        ]
    }

    /// Parses a column from its name string. Return an `Error` if the name does not match any known column.
    pub fn from_name(name: &str) -> Result<Column> {
        match name {
            "default_column" => Ok(Column::DefaultColumn),
            "migrations" => Ok(Column::Migrations),
            "period_data" => Ok(Column::PeriodData),
            "genesis" => Ok(Column::Genesis),
            "dag_blocks" => Ok(Column::DagBlocks),
            "dag_blocks_level" => Ok(Column::DagBlocksLevel),
            "transactions" => Ok(Column::Transactions),
            "trx_period" => Ok(Column::TrxPeriod),
            "status" => Ok(Column::Status),
            "pbft_mgr_round_step" => Ok(Column::PbftMgrRoundStep),
            "pbft_mgr_status" => Ok(Column::PbftMgrStatus),
            "cert_voted_block_in_round" => Ok(Column::CertVotedBlockInRound),
            "proposed_pbft_blocks" => Ok(Column::ProposedPbftBlocks),
            "pbft_head" => Ok(Column::PbftHead),
            "latest_round_own_votes" => Ok(Column::LatestRoundOwnVotes),
            "latest_round_two_t_plus_one_votes" => Ok(Column::LatestRoundTwoTPlusOneVotes),
            "extra_reward_votes" => Ok(Column::ExtraRewardVotes),
            "pbft_block_period" => Ok(Column::PbftBlockPeriod),
            "dag_block_period" => Ok(Column::DagBlockPeriod),
            "proposal_period_levels_map" => Ok(Column::ProposalPeriodLevelsMap),
            "final_chain_meta" => Ok(Column::FinalChainMeta),
            "final_chain_blk_by_number" => Ok(Column::FinalChainBlkByNumber),
            "final_chain_blk_hash_by_number" => Ok(Column::FinalChainBlkHashByNumber),
            "final_chain_blk_number_by_hash" => Ok(Column::FinalChainBlkNumberByHash),
            "final_chain_receipt_by_trx_hash" => Ok(Column::FinalChainReceiptByTrxHash),
            "final_chain_log_blooms_index" => Ok(Column::FinalChainLogBloomsIndex),
            "sortition_params_change" => Ok(Column::SortitionParamsChange),
            "block_rewards_stats" => Ok(Column::BlockRewardsStats),
            "pillar_block" => Ok(Column::PillarBlock),
            "current_pillar_block_data" => Ok(Column::CurrentPillarBlockData),
            "current_pillar_block_own_vote" => Ok(Column::CurrentPillarBlockOwnVote),
            "system_transaction" => Ok(Column::SystemTransaction),
            "period_system_transactions" => Ok(Column::PeriodSystemTransactions),
            "final_chain_receipt_by_period" => Ok(Column::FinalChainReceiptByPeriod),
            "period_lambda" => Ok(Column::PeriodLambda),
            "rounds_count_dynamic_lambda" => Ok(Column::RoundsCountDynamicLambda),
            _ => Err(StorageError::Config(format!("Unknown column name: {}", name)).into()),
        }
    }

    /// Returns whether the column uses a custom uint64 comparator.
    pub fn uses_uint64_comparator(&self) -> bool {
        matches!(
            self,
            Column::PeriodData
                | Column::DagBlocksLevel
                | Column::ProposalPeriodLevelsMap
                | Column::SortitionParamsChange
                | Column::BlockRewardsStats
                | Column::PillarBlock
                | Column::FinalChainReceiptByPeriod
                | Column::PeriodLambda
        )
    }

    /// Creates a ColumnFamilyDescriptor for this column.
    pub fn descriptor(&self, opts: &rocksdb::Options) -> rocksdb::ColumnFamilyDescriptor {
        let mut opts = opts.clone();
        if self.uses_uint64_comparator() {
            opts.set_comparator("taraxa.UintComparator", Self::uint64_comparator());
        }
        rocksdb::ColumnFamilyDescriptor::new(self.name(), opts)
    }

    /// Returns a comparator for uint64-based keys, matching C++ UintComparator<u64> behavior.
    /// C++ implementation uses `memcpy` to interpret bytes, which results in Little Endian
    /// interpretation on x64/ARM64. To match this sort order, we must decode as LE.
    fn uint64_comparator() -> ComparatorFn {
        Box::new(|a, b| {
            let a_val = u64::from_le_bytes(a.try_into().unwrap());
            let b_val = u64::from_le_bytes(b.try_into().unwrap());
            a_val.cmp(&b_val)
        })
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusField {
    ExecutedBlkCount,
    ExecutedTrxCount,
    TrxCount,
    DagBlkCount,
    DagEdgeCount,
    DbMajorVersion,
    DbMinorVersion,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_creation() {
        let base_path = PathBuf::from("/tmp/test_db");
        let config = Config::new(base_path);

        assert_eq!(config.db_path, PathBuf::from("/tmp/test_db/db"));
        assert_eq!(config.state_path, PathBuf::from("/tmp/test_db/state"));
        assert!(config.create_if_missing);
        assert!(config.create_missing_column_families);
        assert_eq!(config.max_total_wal_size, 1024 * 1024 * 1024);
        assert_eq!(config.db_write_buffer_size, 2 * 1024 * 1024 * 1024);
        assert_eq!(config.max_open_files, 256);
    }

    #[test]
    fn test_column_name_mapping() {
        // Test a few key columns
        assert_eq!(Column::DefaultColumn.name(), "default_column");
        assert_eq!(Column::PeriodData.name(), "period_data");
        assert_eq!(Column::DagBlocks.name(), "dag_blocks");
        assert_eq!(Column::Status.name(), "status");
        assert_eq!(Column::PbftHead.name(), "pbft_head");
    }

    #[test]
    fn test_column_from_name_valid() {
        assert_eq!(
            Column::from_name("default_column").unwrap(),
            Column::DefaultColumn
        );
        assert_eq!(
            Column::from_name("period_data").unwrap(),
            Column::PeriodData
        );
        assert_eq!(Column::from_name("dag_blocks").unwrap(), Column::DagBlocks);
        assert_eq!(Column::from_name("migrations").unwrap(), Column::Migrations);
        assert_eq!(Column::from_name("pbft_head").unwrap(), Column::PbftHead);
        assert_eq!(
            Column::from_name("final_chain_receipt_by_period").unwrap(),
            Column::FinalChainReceiptByPeriod
        );
    }

    #[test]
    fn test_column_from_name_invalid() {
        assert!(Column::from_name("invalid_column").is_err());
        assert!(Column::from_name("").is_err());
        assert!(Column::from_name("unknown").is_err());
    }

    #[test]
    fn test_all_columns_round_trip() {
        // Verify that all columns can be converted to name and back
        for column in Column::all() {
            let name = column.name();
            let recovered =
                Column::from_name(name).expect(&format!("Failed to parse column: {}", name));
            assert_eq!(*column, recovered);
        }
    }

    #[test]
    fn test_uint64_comparator_columns() {
        let uint64_columns = vec![
            Column::DagBlocksLevel,
            Column::ProposalPeriodLevelsMap,
            Column::BlockRewardsStats,
        ];

        for column in uint64_columns {
            assert!(
                column.uses_uint64_comparator(),
                "{:?} should use uint64 comparator",
                column
            );
        }
    }

    #[test]
    fn test_all_columns_have_names() {
        for column in Column::all() {
            let name = column.name();
            assert!(!name.is_empty(), "Column {:?} has empty name", column);
        }
    }
}
