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
use crate::ffi::{BridgeRewardsStatsRuntime, BridgeStorage};
use anyhow::{anyhow, Result};
use ethereum_types::{H160, H256, U256};
#[cfg(test)]
use rustaxa_consensus::RewardsStatsRuntime;
use rustaxa_consensus::{
    apply_rewards_stats_storage_writes as domain_apply_rewards_stats_storage_writes,
    rewards_stats_runtime_from_storage, FinalizedRewardsPeriodFact, RewardCertVoteFact,
    RewardDagBlockFact, RewardTransactionFact, RewardsFrequencyRule, RewardsStatsApplyStatus,
    RewardsStatsConfig, RewardsStatsPeriodRlp, RewardsStatsProcessPlan, RewardsStatsStatus,
    RewardsStatsStorageApplyResult,
};

const REWARDS_STATS_APPLY_STATUS_APPLIED: u8 = 0;
const REWARDS_STATS_APPLY_STATUS_REJECTED: u8 = 1;

/// Creates a Rust rewards-stat runtime seeded from existing Rust storage rows.
///
/// `last_block_number` mirrors the legacy constructor recovery behavior for
/// in-memory state: when it falls on a distribution boundary, cached rows are
/// not loaded into the runtime. The bridge does not delete storage rows during
/// construction; callers should use the explicit append/clear path for
/// finalization-atomic mutations.
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
    Ok(Box::new(BridgeRewardsStatsRuntime(
        rewards_stats_runtime_from_storage(&storage.0, config, frequency_rules, last_block_number)?,
    )))
}

impl BridgeRewardsStatsRuntime {
    /// Processes one finalized period through this Rust rewards-stat runtime.
    pub fn process_finalized_period_rewards_stats(
        &mut self,
        fact: RewardsStatsProcessFact,
    ) -> RewardsStatsProcessResult {
        process_finalized_period_rewards_stats(self, fact)
    }

    /// Clears this runtime's cache after the caller commits a distribution
    /// boundary period.
    pub fn rewards_stats_runtime_clear_committed(&mut self, current_period: u64) {
        rewards_stats_runtime_clear_committed(self, current_period);
    }
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
        Ok(fact) => runtime.0.process_period(fact).into(),
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

/// Clears the runtime cache after the caller has committed a distribution
/// boundary period.
pub fn rewards_stats_runtime_clear_committed(
    runtime: &mut BridgeRewardsStatsRuntime,
    current_period: u64,
) {
    runtime.0.clear_committed(current_period);
}

/// Applies reward-stat cache writes or clears through a Rust-owned storage batch.
///
/// Inputs:
/// - `storage`: shared Rust storage handle.
/// - `plan` is the successful result from `process_finalized_period_rewards_stats`.
/// - `sync`: commit sync flag.
///
/// Outputs:
/// - `status` is `0` when the requested writes were committed and `1` when
///   the plan was rejected or internally inconsistent.
pub fn apply_rewards_stats_storage_writes(
    storage: &BridgeStorage,
    plan: &RewardsStatsProcessResult,
    sync: bool,
) -> Result<RewardsStatsApplyResult> {
    let plan = rewards_stats_process_plan_from_ffi(plan);
    Ok(domain_apply_rewards_stats_storage_writes(&storage.0, &plan, sync)?.into())
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

    #[test]
    fn bridge_processes_period_and_returns_rlp() {
        let mut runtime = BridgeRewardsStatsRuntime(
            RewardsStatsRuntime::new(config().into(), Vec::new(), Vec::new()).unwrap(),
        );

        let result = process_finalized_period_rewards_stats(&mut runtime, fact(1));

        assert_eq!(result.status, 0);
        assert_eq!(result.distribution_stats.len(), 1);
        let rlp = Rlp::new(&result.current_block_stats_rlp);
        assert_eq!(rlp.val_at::<u32>(1).unwrap(), 99);
        assert_eq!(rlp.val_at::<u64>(4).unwrap(), 11);
    }

    #[test]
    fn bridge_rejects_oversized_gas_price() {
        let mut runtime = BridgeRewardsStatsRuntime(
            RewardsStatsRuntime::new(config().into(), Vec::new(), Vec::new()).unwrap(),
        );
        let mut bad = fact(1);
        bad.transactions[0].gas_price_be = vec![1; 33];

        let result = process_finalized_period_rewards_stats(&mut runtime, bad);

        assert_eq!(result.status, 1);
        assert_eq!(result.error_code, "REWARDS_STATS_GAS_PRICE_TOO_WIDE");
    }
}
