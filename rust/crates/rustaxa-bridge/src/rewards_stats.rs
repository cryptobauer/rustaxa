//! CXX bridge for Rust rewards-statistics planning.
//!
//! The bridge exposes a Rust-owned rewards-stat runtime to C++ shims using
//! plain facts and legacy-compatible `BlockStats` RLP bytes. Rewards-stat cache
//! persistence and clearing are applied through Rust-owned storage batches in
//! `rustaxa-consensus`; the bridge only maps CXX DTOs.

use crate::ffi::rustaxa_ffi::{
    PeriodRlp as FfiPeriodRlp, RewardsCertVoteFact as FfiRewardsCertVoteFact,
    RewardsDagBlockFact as FfiRewardsDagBlockFact, RewardsFrequencyRule as FfiRewardsFrequencyRule,
    RewardsStatsApplyResult, RewardsStatsConfig as FfiRewardsStatsConfig, RewardsStatsProcessFact,
    RewardsStatsProcessResult, RewardsTransactionFact as FfiRewardsTransactionFact,
};
use crate::ffi::{BridgeRewardsStatsRuntime, BridgeStorage, BridgeStorageBatch};
use anyhow::{anyhow, Result};
use ethereum_types::{H160, H256, U256};
#[cfg(test)]
use rustaxa_consensus::apply_rewards_stats_storage_writes as domain_apply_rewards_stats_storage_writes;
#[cfg(test)]
use rustaxa_consensus::RewardsStatsRuntime;
use rustaxa_consensus::{
    append_rewards_stats_storage_writes_to_batch as domain_append_rewards_stats_storage_writes_to_batch,
    clear_rewards_stats_storage as domain_clear_rewards_stats_storage,
    rewards_stats_runtime_from_storage, FinalizedRewardsPeriodFact, RewardCertVoteFact,
    RewardDagBlockFact, RewardTransactionFact, RewardsFrequencyRule, RewardsStatsApplyStatus,
    RewardsStatsConfig, RewardsStatsPeriodRlp, RewardsStatsProcessPlan, RewardsStatsStatus,
    RewardsStatsStorageApplyResult,
};

const REWARDS_STATS_APPLY_STATUS_APPLIED: u8 = 0;
const REWARDS_STATS_APPLY_STATUS_REJECTED: u8 = 1;

/// Creates a Rust rewards-stat runtime seeded from existing Rust storage rows.
///
/// `last_block_number` mirrors the legacy constructor recovery behavior: when
/// it falls on a distribution boundary, the consensus domain clears stale cache
/// rows through `rustaxa-storage` before returning an empty runtime cache.
pub fn create_rewards_stats_runtime(
    storage: &BridgeStorage,
    config: FfiRewardsStatsConfig,
    frequency_rules: Vec<FfiRewardsFrequencyRule>,
    last_block_number: u64,
) -> Result<Box<BridgeRewardsStatsRuntime>> {
    let config = RewardsStatsConfig::from(config);
    let frequency_rules = frequency_rules
        .into_iter()
        .map(RewardsFrequencyRule::from)
        .collect::<Vec<_>>();
    Ok(Box::new(BridgeRewardsStatsRuntime {
        state: rewards_stats_runtime_from_storage(
            &storage.0,
            config,
            frequency_rules,
            last_block_number,
        )?,
        storage: storage.0.clone(),
    }))
}

impl BridgeRewardsStatsRuntime {
    /// Processes one finalized period through this Rust rewards-stat runtime.
    pub fn process_finalized_period_rewards_stats(
        &mut self,
        fact: RewardsStatsProcessFact,
    ) -> RewardsStatsProcessResult {
        process_finalized_period_rewards_stats(self, fact)
    }

    /// Previews rewards-stat processing on a cloned runtime state.
    pub fn preview_finalized_period_rewards_stats(
        &self,
        fact: RewardsStatsProcessFact,
    ) -> RewardsStatsProcessResult {
        preview_finalized_period_rewards_stats(self, fact)
    }

    /// Commits a previously previewed rewards-stat process result to this runtime.
    pub fn rewards_stats_runtime_commit_process_result(
        &mut self,
        plan: &RewardsStatsProcessResult,
    ) -> Result<RewardsStatsApplyResult> {
        rewards_stats_runtime_commit_process_result(self, plan)
    }

    /// Clears this runtime's cache after the caller commits a distribution
    /// boundary period.
    pub fn rewards_stats_runtime_clear_committed(&mut self, current_period: u64) {
        rewards_stats_runtime_clear_committed(self, current_period);
    }

    /// Returns this runtime's cached reward-stat rows as legacy RLP DTOs.
    pub fn rewards_stats_runtime_cached_stats(&self) -> Vec<FfiPeriodRlp> {
        rewards_stats_runtime_cached_stats(self)
    }

    /// Clears persisted reward-stat rows and in-memory state after a committed
    /// distribution-boundary period.
    pub fn rewards_stats_runtime_clear_storage_and_state(
        &mut self,
        current_period: u64,
        sync: bool,
    ) -> Result<RewardsStatsApplyResult> {
        rewards_stats_runtime_clear_storage_and_state(self, current_period, sync)
    }
}

fn rewards_stats_batch_mut(
    batch: &mut BridgeStorageBatch,
) -> Result<&mut rustaxa_storage::StorageWriteBatch> {
    batch
        .batch
        .as_mut()
        .ok_or_else(|| anyhow!("REWARDS_STATS_BATCH_ALREADY_COMMITTED"))
}

/// Processes one finalized period through the Rust rewards-stat runtime.
///
/// Conversion failures, such as oversized gas-price bytes, return a rejected
/// result with a stable error code instead of panicking or falling back to C++.
pub fn process_finalized_period_rewards_stats(
    runtime: &mut BridgeRewardsStatsRuntime,
    fact: RewardsStatsProcessFact,
) -> RewardsStatsProcessResult {
    let current_period = fact.period;
    match finalized_fact_from_ffi(fact) {
        Ok(fact) => runtime.state.process_period(fact).into(),
        Err(error) => RewardsStatsProcessResult {
            status: RewardsStatsStatus::Rejected.as_u8(),
            error_code: error.to_string(),
            current_period,
            cache_current_period: false,
            clear_cached_stats: false,
            current_block_stats_rlp: Vec::new(),
            distribution_stats: Vec::new(),
        },
    }
}

/// Processes one finalized period on a cloned runtime without mutating live state.
///
/// This is used by FinalChain publication paths that must first distribute
/// rewards and commit storage before advancing the in-memory rewards cache.
pub fn preview_finalized_period_rewards_stats(
    runtime: &BridgeRewardsStatsRuntime,
    fact: RewardsStatsProcessFact,
) -> RewardsStatsProcessResult {
    let current_period = fact.period;
    match finalized_fact_from_ffi(fact) {
        Ok(fact) => runtime.state.clone().process_period(fact).into(),
        Err(error) => RewardsStatsProcessResult {
            status: RewardsStatsStatus::Rejected.as_u8(),
            error_code: error.to_string(),
            current_period,
            cache_current_period: false,
            clear_cached_stats: false,
            current_block_stats_rlp: Vec::new(),
            distribution_stats: Vec::new(),
        },
    }
}

/// Applies a previously previewed rewards-stat process result to live state.
pub fn rewards_stats_runtime_commit_process_result(
    runtime: &mut BridgeRewardsStatsRuntime,
    plan: &RewardsStatsProcessResult,
) -> Result<RewardsStatsApplyResult> {
    let plan = rewards_stats_process_plan_from_ffi(plan);
    if let Err(error) = runtime.state.apply_process_plan(&plan) {
        return Ok(RewardsStatsStorageApplyResult {
            status: RewardsStatsApplyStatus::Rejected,
            current_period: plan.current_period,
            wrote_current_period: false,
            cleared_cached_stats: false,
            error_code: error.to_string(),
        }
        .into());
    }
    Ok(RewardsStatsStorageApplyResult {
        status: RewardsStatsApplyStatus::Applied,
        current_period: plan.current_period,
        wrote_current_period: plan.cache_current_period,
        cleared_cached_stats: plan.clear_cached_stats,
        error_code: String::new(),
    }
    .into())
}

/// Clears the runtime cache after the caller has committed a distribution
/// boundary period.
pub fn rewards_stats_runtime_clear_committed(
    runtime: &mut BridgeRewardsStatsRuntime,
    current_period: u64,
) {
    runtime.state.clear_committed(current_period);
}

/// Returns the runtime cache as CXX-safe RLP rows.
pub fn rewards_stats_runtime_cached_stats(
    runtime: &BridgeRewardsStatsRuntime,
) -> Vec<FfiPeriodRlp> {
    runtime
        .state
        .cached_stats_rlp()
        .into_iter()
        .map(FfiPeriodRlp::from)
        .collect()
}

/// Applies reward-stat writes through the storage handle owned by the runtime.
#[cfg(test)]
pub fn rewards_stats_runtime_apply_storage_writes(
    runtime: &BridgeRewardsStatsRuntime,
    plan: &RewardsStatsProcessResult,
    sync: bool,
) -> Result<RewardsStatsApplyResult> {
    let plan = rewards_stats_process_plan_from_ffi(plan);
    Ok(domain_apply_rewards_stats_storage_writes(&runtime.storage, &plan, sync)?.into())
}

/// Clears persisted reward-stat rows and updates the runtime cache after the
/// storage commit succeeds.
pub fn rewards_stats_runtime_clear_storage_and_state(
    runtime: &mut BridgeRewardsStatsRuntime,
    current_period: u64,
    sync: bool,
) -> Result<RewardsStatsApplyResult> {
    let result = domain_clear_rewards_stats_storage(&runtime.storage, current_period, sync)?;
    if result.status == RewardsStatsApplyStatus::Applied {
        runtime.state.clear_committed(current_period);
    }
    Ok(result.into())
}

/// Appends reward-stat cache writes to an existing Rust storage shim batch.
///
/// This is the task-specific replacement for routing rewards-stat production
/// writes through generic `storage_shim_save_block_rewards_stats`. The legacy
/// C++ `Batch&` remains only as an opaque carrier for the Rust-owned batch while
/// rewards-stat validation and column/key selection stay in the rewards module.
pub fn rewards_stats_append_storage_writes_to_batch(
    batch: &mut BridgeStorageBatch,
    plan: &RewardsStatsProcessResult,
) -> Result<RewardsStatsApplyResult> {
    let plan = rewards_stats_process_plan_from_ffi(plan);
    let storage = batch.storage.clone();
    Ok(domain_append_rewards_stats_storage_writes_to_batch(
        &storage,
        rewards_stats_batch_mut(batch)?,
        &plan,
    )?
    .into())
}

fn finalized_fact_from_ffi(value: RewardsStatsProcessFact) -> Result<FinalizedRewardsPeriodFact> {
    Ok(FinalizedRewardsPeriodFact {
        period: value.period,
        block_author: H160::from(value.block_author),
        blocks_per_year: value.blocks_per_year,
        dpos_eligible_total_vote_count: value.dpos_eligible_total_vote_count,
        transactions: value
            .transactions
            .into_iter()
            .map(transaction_fact_from_ffi)
            .collect::<Result<Vec<_>>>()?,
        dag_blocks: value
            .dag_blocks
            .into_iter()
            .map(RewardDagBlockFact::from)
            .collect(),
        cert_votes: value
            .cert_votes
            .into_iter()
            .map(RewardCertVoteFact::from)
            .collect(),
    })
}

fn transaction_fact_from_ffi(value: FfiRewardsTransactionFact) -> Result<RewardTransactionFact> {
    if value.gas_price_be.len() > 32 {
        return Err(anyhow!("REWARDS_STATS_GAS_PRICE_TOO_WIDE"));
    }
    Ok(RewardTransactionFact {
        hash: H256::from(value.hash),
        gas_price: U256::from_big_endian(&value.gas_price_be),
        gas_used: value.gas_used,
    })
}

fn rewards_stats_process_plan_from_ffi(
    value: &RewardsStatsProcessResult,
) -> RewardsStatsProcessPlan {
    RewardsStatsProcessPlan {
        status: if value.status == RewardsStatsStatus::Applied.as_u8() {
            RewardsStatsStatus::Applied
        } else {
            RewardsStatsStatus::Rejected
        },
        error_code: value.error_code.clone(),
        current_period: value.current_period,
        cache_current_period: value.cache_current_period,
        clear_cached_stats: value.clear_cached_stats,
        current_block_stats_rlp: value.current_block_stats_rlp.clone(),
        distribution_stats: value
            .distribution_stats
            .iter()
            .map(|entry| RewardsStatsPeriodRlp {
                period: entry.period,
                data: entry.data.clone(),
            })
            .collect(),
    }
}

impl From<FfiRewardsStatsConfig> for RewardsStatsConfig {
    fn from(value: FfiRewardsStatsConfig) -> Self {
        Self {
            committee_size: value.committee_size,
            magnolia_period: value.magnolia_period,
            aspen_part_one_period: value.aspen_part_one_period,
        }
    }
}

impl From<FfiRewardsFrequencyRule> for RewardsFrequencyRule {
    fn from(value: FfiRewardsFrequencyRule) -> Self {
        Self {
            from_period: value.from_period,
            frequency: value.frequency,
        }
    }
}

impl From<FfiRewardsDagBlockFact> for RewardDagBlockFact {
    fn from(value: FfiRewardsDagBlockFact) -> Self {
        Self {
            author: H160::from(value.author),
            difficulty: value.difficulty,
            transaction_hashes: value
                .transaction_hashes
                .into_iter()
                .map(|entry| H256::from(entry.hash))
                .collect(),
        }
    }
}

impl From<FfiRewardsCertVoteFact> for RewardCertVoteFact {
    fn from(value: FfiRewardsCertVoteFact) -> Self {
        Self {
            voter: H160::from(value.voter),
            weight: value.weight,
            period: value.period,
        }
    }
}

impl From<RewardsStatsProcessPlan> for RewardsStatsProcessResult {
    fn from(value: RewardsStatsProcessPlan) -> Self {
        Self {
            status: value.status.as_u8(),
            error_code: value.error_code,
            current_period: value.current_period,
            cache_current_period: value.cache_current_period,
            clear_cached_stats: value.clear_cached_stats,
            current_block_stats_rlp: value.current_block_stats_rlp,
            distribution_stats: value
                .distribution_stats
                .into_iter()
                .map(FfiPeriodRlp::from)
                .collect(),
        }
    }
}

impl From<RewardsStatsStorageApplyResult> for RewardsStatsApplyResult {
    fn from(value: RewardsStatsStorageApplyResult) -> Self {
        Self {
            status: match value.status {
                RewardsStatsApplyStatus::Applied => REWARDS_STATS_APPLY_STATUS_APPLIED,
                RewardsStatsApplyStatus::Rejected => REWARDS_STATS_APPLY_STATUS_REJECTED,
            },
            current_period: value.current_period,
            wrote_current_period: value.wrote_current_period,
            cleared_cached_stats: value.cleared_cached_stats,
            error_code: value.error_code,
        }
    }
}

impl From<RewardsStatsPeriodRlp> for FfiPeriodRlp {
    fn from(value: RewardsStatsPeriodRlp) -> Self {
        Self {
            period: value.period,
            data: value.data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::Rlp;
    use rustaxa_storage::{Config, Storage};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn hash(value: u8) -> [u8; 32] {
        [value; 32]
    }

    fn config() -> FfiRewardsStatsConfig {
        FfiRewardsStatsConfig {
            committee_size: 100,
            magnolia_period: 1,
            aspen_part_one_period: 10,
        }
    }

    fn fact(period: u64) -> RewardsStatsProcessFact {
        RewardsStatsProcessFact {
            period,
            block_author: [1; 20],
            blocks_per_year: 99,
            dpos_eligible_total_vote_count: 80,
            transactions: vec![FfiRewardsTransactionFact {
                hash: hash(7),
                gas_price_be: vec![2],
                gas_used: 5,
            }],
            dag_blocks: vec![FfiRewardsDagBlockFact {
                author: [2; 20],
                difficulty: 3,
                transaction_hashes: vec![crate::ffi::rustaxa_ffi::RewardsHash { hash: hash(7) }],
            }],
            cert_votes: vec![FfiRewardsCertVoteFact {
                voter: [3; 20],
                weight: 11,
                period: 4,
            }],
        }
    }

    fn temp_storage(name: &str) -> Arc<Storage> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = PathBuf::from(format!(
            "/tmp/rustaxa_bridge_rewards_stats_{name}_{}_{}",
            std::process::id(),
            nonce
        ));
        Arc::new(Storage::new(Config::new(dir)).unwrap())
    }

    fn runtime_for_tests() -> BridgeRewardsStatsRuntime {
        BridgeRewardsStatsRuntime {
            state: RewardsStatsRuntime::new(config().into(), Vec::new(), Vec::new()).unwrap(),
            storage: temp_storage("runtime"),
        }
    }

    #[test]
    fn bridge_processes_period_and_returns_rlp() {
        let mut runtime = runtime_for_tests();

        let result = process_finalized_period_rewards_stats(&mut runtime, fact(1));

        assert_eq!(result.status, 0);
        assert_eq!(result.distribution_stats.len(), 1);
        let rlp = Rlp::new(&result.current_block_stats_rlp);
        assert_eq!(rlp.val_at::<u32>(1).unwrap(), 99);
        assert_eq!(rlp.val_at::<u64>(4).unwrap(), 11);
    }

    #[test]
    fn bridge_rejects_oversized_gas_price() {
        let mut runtime = runtime_for_tests();
        let mut bad = fact(1);
        bad.transactions[0].gas_price_be = vec![1; 33];

        let result = process_finalized_period_rewards_stats(&mut runtime, bad);

        assert_eq!(result.status, 1);
        assert_eq!(result.error_code, "REWARDS_STATS_GAS_PRICE_TOO_WIDE");
    }

    #[test]
    fn runtime_apply_writes_and_clear_use_owned_storage() {
        let storage = temp_storage("owned_apply");
        let mut runtime = BridgeRewardsStatsRuntime {
            state: RewardsStatsRuntime::new(
                config().into(),
                vec![RewardsFrequencyRule {
                    from_period: 0,
                    frequency: 3,
                }],
                Vec::new(),
            )
            .unwrap(),
            storage: storage.clone(),
        };

        let cache_plan = process_finalized_period_rewards_stats(&mut runtime, fact(1));
        assert!(cache_plan.cache_current_period);
        let apply =
            rewards_stats_runtime_apply_storage_writes(&runtime, &cache_plan, false).unwrap();
        assert_eq!(apply.status, REWARDS_STATS_APPLY_STATUS_APPLIED);
        assert!(apply.wrote_current_period);
        assert_eq!(
            storage.metadata().block_rewards_stats_rlp().unwrap().len(),
            1
        );
        assert_eq!(rewards_stats_runtime_cached_stats(&runtime).len(), 1);

        let clear = rewards_stats_runtime_clear_storage_and_state(&mut runtime, 3, false).unwrap();
        assert_eq!(clear.status, REWARDS_STATS_APPLY_STATUS_APPLIED);
        assert!(clear.cleared_cached_stats);
        assert!(storage
            .metadata()
            .block_rewards_stats_rlp()
            .unwrap()
            .is_empty());
        assert!(rewards_stats_runtime_cached_stats(&runtime).is_empty());
    }

    #[test]
    fn bridge_appends_rewards_stats_to_existing_storage_batch() {
        let storage = temp_storage("batch_append");
        let mut runtime = BridgeRewardsStatsRuntime {
            state: RewardsStatsRuntime::new(
                config().into(),
                vec![RewardsFrequencyRule {
                    from_period: 0,
                    frequency: 3,
                }],
                Vec::new(),
            )
            .unwrap(),
            storage: storage.clone(),
        };

        let cache_plan = process_finalized_period_rewards_stats(&mut runtime, fact(1));
        let mut batch = BridgeStorageBatch {
            storage: storage.clone(),
            batch: Some(storage.create_write_batch()),
        };

        let apply = rewards_stats_append_storage_writes_to_batch(&mut batch, &cache_plan).unwrap();
        assert_eq!(apply.status, REWARDS_STATS_APPLY_STATUS_APPLIED);
        assert!(apply.wrote_current_period);
        assert!(storage
            .metadata()
            .block_rewards_stats_rlp()
            .unwrap()
            .is_empty());

        storage
            .commit_write_batch_with_sync(batch.batch.take().unwrap(), false)
            .unwrap();
        assert_eq!(
            storage.metadata().block_rewards_stats_rlp().unwrap().len(),
            1
        );
    }

    #[test]
    fn preview_does_not_advance_runtime_until_commit() {
        let mut runtime = BridgeRewardsStatsRuntime {
            state: RewardsStatsRuntime::new(
                config().into(),
                vec![RewardsFrequencyRule {
                    from_period: 0,
                    frequency: 3,
                }],
                Vec::new(),
            )
            .unwrap(),
            storage: temp_storage("preview_commit"),
        };

        let preview = preview_finalized_period_rewards_stats(&runtime, fact(1));
        assert_eq!(preview.status, 0);
        assert!(preview.cache_current_period);
        assert!(rewards_stats_runtime_cached_stats(&runtime).is_empty());

        let commit = rewards_stats_runtime_commit_process_result(&mut runtime, &preview)
            .expect("preview commit should succeed");
        assert_eq!(commit.status, REWARDS_STATS_APPLY_STATUS_APPLIED);
        assert!(commit.wrote_current_period);
        assert_eq!(rewards_stats_runtime_cached_stats(&runtime).len(), 1);
    }
}
