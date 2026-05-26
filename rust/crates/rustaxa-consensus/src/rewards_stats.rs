//! Deterministic rewards-statistics planning for finalized PBFT periods.
//!
//! This module owns the storage-free calculation previously embedded in the C++
//! `rewards::BlockStats` and `rewards::Stats` classes. Callers provide already
//! materialized period facts: PBFT author, finalized transactions with gas
//! usage, finalized DAG block authors/difficulties, cert-vote weights, DPoS
//! total vote count, and hardfork/frequency configuration. The planner returns
//! legacy-compatible `BlockStats` RLP bytes plus explicit cache/clear intents.
//!
//! The module intentionally does not distribute rewards or mutate account state.
//! Callers that own staged final-chain state can decode the legacy distribution
//! rows into typed reward inputs and apply the side effects in their own atomic
//! persistence boundary.

use anyhow::{Context, Result, anyhow, bail};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use std::collections::{BTreeMap, BTreeSet};

/// Hardfork and committee configuration used by reward-stat planning.
///
/// `magnolia_period` gates fee rewards: periods below this value preserve the
/// legacy zero-fee behavior. `aspen_part_one_period` gates DAG-block rewards:
/// periods at or above this value count minimum-difficulty DAG blocks instead
/// of first unique transaction inclusion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsStatsConfig {
    pub committee_size: u32,
    pub magnolia_period: u64,
    pub aspen_part_one_period: u64,
}

/// Rewards distribution frequency rule active from `from_period` onward.
///
/// Rules mirror `HardforksConfig::rewards_distribution_frequency`: if no rule
/// starts at or before a period, the effective frequency is one block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsFrequencyRule {
    pub from_period: u64,
    pub frequency: u32,
}

/// Transaction fee fact for one finalized transaction in a PBFT period.
///
/// `gas_price` is the canonical transaction gas price and `gas_used` is the
/// post-execution gas usage supplied by the still-C++ FinalChain execution path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardTransactionFact {
    pub hash: H256,
    pub gas_price: U256,
    pub gas_used: u64,
}

/// Finalized DAG block fact needed for reward-stat calculation.
///
/// Transaction hashes are in the legacy DAG block order. Hashes that are not
/// part of the finalized PBFT transaction list are ignored by the fee map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardDagBlockFact {
    pub author: H160,
    pub difficulty: u16,
    pub transaction_hashes: Vec<H256>,
}

/// Previous-block cert-vote fact used for validator vote-weight rewards.
///
/// `period` is carried for bridge parity with the C++ vote sidecar; the planner
/// uses the caller-supplied DPoS total vote count and does not perform DPoS
/// queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardCertVoteFact {
    pub voter: H160,
    pub weight: u64,
    pub period: u64,
}

/// Complete rewards-stat input for one finalized PBFT period.
///
/// The fact is intentionally plain and side-effect free. It can be built from
/// C++ sidecars today and from Rust period/final-chain domains later without
/// changing the reward-stat planner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalizedRewardsPeriodFact {
    pub period: u64,
    pub block_author: H160,
    pub blocks_per_year: u32,
    pub dpos_eligible_total_vote_count: u64,
    pub transactions: Vec<RewardTransactionFact>,
    pub dag_blocks: Vec<RewardDagBlockFact>,
    pub cert_votes: Vec<RewardCertVoteFact>,
}

/// Status for one rewards-stat planning operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewardsStatsStatus {
    Applied,
    Rejected,
}

impl RewardsStatsStatus {
    pub fn as_u8(self) -> u8 {
        match self {
            RewardsStatsStatus::Applied => 0,
            RewardsStatsStatus::Rejected => 1,
        }
    }
}

/// One legacy-compatible block-stat payload keyed by PBFT period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsStatsPeriodRlp {
    pub period: u64,
    pub data: Vec<u8>,
}

/// Per-validator facts decoded from legacy `BlockStats` distribution RLP.
///
/// The values are the deterministic activity counters and fee ownership used
/// by reward distribution. They intentionally do not mutate account or DPoS
/// state; FinalChain decides how to split rewards across validator commission,
/// delegator pools, and executable account balance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RewardsValidatorDistribution {
    pub dag_blocks_count: u32,
    pub vote_weight: u64,
    pub fees_rewards: U256,
}

/// One finalized period's reward-distribution inputs decoded from legacy
/// `BlockStats` RLP.
///
/// This is a typed view over the compatibility payload already returned to C++
/// shims. It preserves per-period boundaries because legacy StateAPI reward
/// distribution runs once per cached block stat at distribution time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsBlockDistribution {
    pub period: u64,
    pub block_author: H160,
    pub blocks_per_year: u32,
    pub validators_stats: BTreeMap<[u8; 20], RewardsValidatorDistribution>,
    pub total_dag_blocks_count: u32,
    pub total_votes_weight: u64,
    pub max_votes_weight: u64,
}

/// Plan returned after processing one finalized period.
///
/// `current_block_stats_rlp` is always set on success. When
/// `cache_current_period` is true, callers should persist that RLP under
/// `current_period` in the block-reward stats column. When
/// `clear_cached_stats` is true, callers should clear persisted cached stats
/// after the surrounding finalization batch has committed and should return
/// `distribution_stats` to the existing reward-distribution API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardsStatsProcessPlan {
    pub status: RewardsStatsStatus,
    pub error_code: String,
    pub current_period: u64,
    pub cache_current_period: bool,
    pub clear_cached_stats: bool,
    pub current_block_stats_rlp: Vec<u8>,
    pub distribution_stats: Vec<RewardsStatsPeriodRlp>,
}

/// In-memory rewards-stat runtime holding the current rewards interval cache.
///
/// The runtime accepts legacy persisted `BlockStats` RLP on construction so
/// restart recovery can keep using existing storage rows until Rust owns the
/// full final-chain lifecycle.
#[derive(Clone, Debug)]
pub struct RewardsStatsRuntime {
    config: RewardsStatsConfig,
    frequency_rules: Vec<RewardsFrequencyRule>,
    cached_stats: BTreeMap<u64, Vec<u8>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ValidatorStats {
    dag_blocks_count: u32,
    vote_weight: u64,
    fees_rewards: U256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BlockStats {
    block_author: H160,
    blocks_per_year: u32,
    validators_stats: BTreeMap<H160, ValidatorStats>,
    total_dag_blocks_count: u32,
    total_votes_weight: u64,
    max_votes_weight: u64,
}

impl RewardsStatsRuntime {
    /// Creates a rewards-stat runtime seeded with persisted current-interval
    /// stats.
    ///
    /// Inputs:
    /// - `config`: committee and hardfork periods.
    /// - `frequency_rules`: distribution interval changes.
    /// - `persisted_stats`: legacy `BlockStats` RLP rows keyed by PBFT period.
    ///
    /// The constructor sorts rules and persisted rows by period, preserving a
    /// deterministic Rust cache while accepting storage rows in any order.
    pub fn new(
        config: RewardsStatsConfig,
        mut frequency_rules: Vec<RewardsFrequencyRule>,
        persisted_stats: Vec<RewardsStatsPeriodRlp>,
    ) -> Result<Self> {
        frequency_rules.sort_by_key(|rule| rule.from_period);
        if frequency_rules.iter().any(|rule| rule.frequency == 0) {
            bail!("REWARDS_STATS_ZERO_FREQUENCY");
        }

        let mut cached_stats = BTreeMap::new();
        for stat in persisted_stats {
            if stat.data.is_empty() {
                bail!("REWARDS_STATS_EMPTY_PERSISTED_RLP");
            }
            cached_stats.insert(stat.period, stat.data);
        }

        Ok(Self {
            config,
            frequency_rules,
            cached_stats,
        })
    }

    /// Processes one finalized period and returns the reward-stat cache and
    /// distribution intents for that period.
    ///
    /// The runtime mutates its in-memory interval cache exactly when the legacy
    /// `rewards::Stats::processStats` would mutate `blocks_stats_`. Call
    /// `clear_committed` after the surrounding finalization commit if the plan
    /// requests `clear_cached_stats`.
    pub fn process_period(&mut self, fact: FinalizedRewardsPeriodFact) -> RewardsStatsProcessPlan {
        let current_period = fact.period;
        match self.process_period_result(fact) {
            Ok(plan) => plan,
            Err(error) => RewardsStatsProcessPlan {
                status: RewardsStatsStatus::Rejected,
                error_code: error.to_string(),
                current_period,
                cache_current_period: false,
                clear_cached_stats: false,
                current_block_stats_rlp: Vec::new(),
                distribution_stats: Vec::new(),
            },
        }
    }

    /// Clears the in-memory rewards interval cache when the committed period is
    /// a distribution boundary.
    ///
    /// The method mirrors `rewards::Stats::clear`: frequency-one periods do not
    /// hold cache state, and non-boundary periods leave the cache intact.
    pub fn clear_committed(&mut self, current_period: u64) {
        let frequency = self.rewards_distribution_frequency(current_period);
        if frequency > 1 && current_period.is_multiple_of(u64::from(frequency)) {
            self.cached_stats.clear();
        }
    }

    fn process_period_result(
        &mut self,
        fact: FinalizedRewardsPeriodFact,
    ) -> Result<RewardsStatsProcessPlan> {
        let current_period = fact.period;
        let frequency = self.rewards_distribution_frequency(current_period);
        let block_stats_rlp = self.block_stats(fact)?.to_rlp();

        if frequency == 1 {
            return Ok(RewardsStatsProcessPlan {
                status: RewardsStatsStatus::Applied,
                error_code: String::new(),
                current_period,
                cache_current_period: false,
                clear_cached_stats: false,
                current_block_stats_rlp: block_stats_rlp.clone(),
                distribution_stats: vec![RewardsStatsPeriodRlp {
                    period: current_period,
                    data: block_stats_rlp,
                }],
            });
        }

        self.cached_stats
            .insert(current_period, block_stats_rlp.clone());

        if !current_period.is_multiple_of(u64::from(frequency)) {
            return Ok(RewardsStatsProcessPlan {
                status: RewardsStatsStatus::Applied,
                error_code: String::new(),
                current_period,
                cache_current_period: true,
                clear_cached_stats: false,
                current_block_stats_rlp: block_stats_rlp,
                distribution_stats: Vec::new(),
            });
        }

        let distribution_stats = self
            .cached_stats
            .iter()
            .map(|(period, data)| RewardsStatsPeriodRlp {
                period: *period,
                data: data.clone(),
            })
            .collect();
        Ok(RewardsStatsProcessPlan {
            status: RewardsStatsStatus::Applied,
            error_code: String::new(),
            current_period,
            cache_current_period: false,
            clear_cached_stats: true,
            current_block_stats_rlp: block_stats_rlp,
            distribution_stats,
        })
    }

    fn rewards_distribution_frequency(&self, period: u64) -> u32 {
        self.frequency_rules
            .iter()
            .rev()
            .find(|rule| rule.from_period <= period)
            .map(|rule| rule.frequency)
            .unwrap_or(1)
    }

    fn block_stats(&self, fact: FinalizedRewardsPeriodFact) -> Result<BlockStats> {
        let dpos_vote_count = if fact.cert_votes.is_empty() {
            u64::from(self.config.committee_size)
        } else {
            fact.dpos_eligible_total_vote_count
        };
        let mut stats = BlockStats {
            block_author: fact.block_author,
            blocks_per_year: fact.blocks_per_year,
            validators_stats: BTreeMap::new(),
            total_dag_blocks_count: 0,
            total_votes_weight: 0,
            max_votes_weight: u64::from(self.config.committee_size).min(dpos_vote_count),
        };

        let include_fees = fact.period >= self.config.magnolia_period;
        let mut fee_by_tx = BTreeMap::<H256, U256>::new();
        let mut block_tx_hashes = BTreeSet::<H256>::new();
        for tx in &fact.transactions {
            block_tx_hashes.insert(tx.hash);
            let fee = if include_fees {
                tx.gas_price
                    .checked_mul(U256::from(tx.gas_used))
                    .ok_or_else(|| anyhow!("REWARDS_STATS_FEE_OVERFLOW"))?
            } else {
                U256::zero()
            };
            fee_by_tx.insert(tx.hash, fee);
        }

        if fact.period >= self.config.aspen_part_one_period {
            Self::process_dag_blocks_aspen(&mut stats, &fact.dag_blocks, &mut fee_by_tx)?;
        } else {
            Self::process_dag_blocks(
                &mut stats,
                &fact.dag_blocks,
                &block_tx_hashes,
                &mut fee_by_tx,
            )?;
        }

        for vote in fact.cert_votes {
            Self::add_vote(&mut stats, vote)?;
        }

        Ok(stats)
    }

    fn process_dag_blocks(
        stats: &mut BlockStats,
        dag_blocks: &[RewardDagBlockFact],
        block_tx_hashes: &BTreeSet<H256>,
        fee_by_tx: &mut BTreeMap<H256, U256>,
    ) -> Result<()> {
        for dag_block in dag_blocks {
            let mut has_unique_transactions = false;
            for tx_hash in &dag_block.transaction_hashes {
                if !block_tx_hashes.contains(tx_hash) {
                    continue;
                }
                if Self::add_transaction(stats, *tx_hash, dag_block.author, fee_by_tx)? {
                    has_unique_transactions = true;
                }
            }
            if has_unique_transactions {
                let validator_stats = stats.validators_stats.entry(dag_block.author).or_default();
                validator_stats.dag_blocks_count = validator_stats
                    .dag_blocks_count
                    .checked_add(1)
                    .context("REWARDS_STATS_DAG_BLOCK_COUNT_OVERFLOW")?;
                stats.total_dag_blocks_count = stats
                    .total_dag_blocks_count
                    .checked_add(1)
                    .context("REWARDS_STATS_TOTAL_DAG_BLOCK_COUNT_OVERFLOW")?;
            }
        }
        Ok(())
    }

    fn process_dag_blocks_aspen(
        stats: &mut BlockStats,
        dag_blocks: &[RewardDagBlockFact],
        fee_by_tx: &mut BTreeMap<H256, U256>,
    ) -> Result<()> {
        let min_difficulty = dag_blocks
            .iter()
            .map(|dag_block| dag_block.difficulty)
            .min()
            .unwrap_or(u16::MAX);

        for dag_block in dag_blocks {
            if dag_block.difficulty == min_difficulty {
                let validator_stats = stats.validators_stats.entry(dag_block.author).or_default();
                validator_stats.dag_blocks_count = validator_stats
                    .dag_blocks_count
                    .checked_add(1)
                    .context("REWARDS_STATS_DAG_BLOCK_COUNT_OVERFLOW")?;
                stats.total_dag_blocks_count = stats
                    .total_dag_blocks_count
                    .checked_add(1)
                    .context("REWARDS_STATS_TOTAL_DAG_BLOCK_COUNT_OVERFLOW")?;
            }
            for tx_hash in &dag_block.transaction_hashes {
                Self::add_transaction(stats, *tx_hash, dag_block.author, fee_by_tx)?;
            }
        }
        Ok(())
    }

    fn add_transaction(
        stats: &mut BlockStats,
        tx_hash: H256,
        validator: H160,
        fee_by_tx: &mut BTreeMap<H256, U256>,
    ) -> Result<bool> {
        let Some(fee) = fee_by_tx.remove(&tx_hash) else {
            return Ok(false);
        };
        let validator_stats = stats.validators_stats.entry(validator).or_default();
        validator_stats.fees_rewards = validator_stats
            .fees_rewards
            .checked_add(fee)
            .ok_or_else(|| anyhow!("REWARDS_STATS_VALIDATOR_FEE_OVERFLOW"))?;
        Ok(true)
    }

    fn add_vote(stats: &mut BlockStats, vote: RewardCertVoteFact) -> Result<()> {
        if vote.weight == 0 {
            bail!("REWARDS_STATS_ZERO_VOTE_WEIGHT");
        }
        let validator_stats = stats.validators_stats.entry(vote.voter).or_default();
        if validator_stats.vote_weight != 0 {
            bail!("REWARDS_STATS_DUPLICATE_VOTER");
        }
        stats.total_votes_weight = stats
            .total_votes_weight
            .checked_add(vote.weight)
            .context("REWARDS_STATS_TOTAL_VOTE_WEIGHT_OVERFLOW")?;
        validator_stats.vote_weight = vote.weight;
        Ok(())
    }
}

impl BlockStats {
    fn to_rlp(&self) -> Vec<u8> {
        let mut stream = RlpStream::new_list(6);
        stream.append(&self.block_author);
        stream.append(&self.blocks_per_year);
        stream.begin_list(self.validators_stats.len());
        for (validator, stats) in &self.validators_stats {
            stream.begin_list(2);
            stream.append(validator);
            stats.rlp_append(&mut stream);
        }
        stream.append(&self.total_dag_blocks_count);
        stream.append(&self.total_votes_weight);
        stream.append(&self.max_votes_weight);
        stream.out().to_vec()
    }
}

impl ValidatorStats {
    fn rlp_append(&self, stream: &mut RlpStream) {
        stream.begin_list(3);
        stream.append(&self.dag_blocks_count);
        stream.append(&self.vote_weight);
        stream.append(&self.fees_rewards);
    }
}

/// Decodes legacy distribution-stat RLP rows into typed Rust reward inputs.
///
/// Inputs are the exact `distribution_stats` rows from
/// `RewardsStatsProcessPlan`. Malformed RLP, truncated validator entries, or
/// duplicate validators inside one row are returned as errors so callers do not
/// silently compute rewards from partial facts.
pub fn decode_rewards_block_distributions(
    distribution_stats: &[RewardsStatsPeriodRlp],
) -> Result<Vec<RewardsBlockDistribution>> {
    distribution_stats
        .iter()
        .map(|stats| decode_rewards_block_distribution(stats.period, &stats.data))
        .collect()
}

fn decode_rewards_block_distribution(period: u64, data: &[u8]) -> Result<RewardsBlockDistribution> {
    let block_stats = Rlp::new(data);
    anyhow::ensure!(
        block_stats.item_count()? == 6,
        "REWARDS_STATS_BLOCK_RLP_ITEM_COUNT"
    );
    let validators = block_stats.at(2)?;
    let mut validators_stats = BTreeMap::new();
    for entry in validators.iter() {
        anyhow::ensure!(
            entry.item_count()? == 2,
            "REWARDS_STATS_VALIDATOR_ENTRY_ITEM_COUNT"
        );
        let validator = entry.val_at::<H160>(0)?.0;
        let stats = entry.at(1)?;
        anyhow::ensure!(
            stats.item_count()? == 3,
            "REWARDS_STATS_VALIDATOR_STATS_ITEM_COUNT"
        );
        let replaced = validators_stats.insert(
            validator,
            RewardsValidatorDistribution {
                dag_blocks_count: stats.val_at(0)?,
                vote_weight: stats.val_at(1)?,
                fees_rewards: stats.val_at(2)?,
            },
        );
        anyhow::ensure!(replaced.is_none(), "REWARDS_STATS_DUPLICATE_VALIDATOR_RLP");
    }

    Ok(RewardsBlockDistribution {
        period,
        block_author: block_stats.val_at(0)?,
        blocks_per_year: block_stats.val_at(1)?,
        validators_stats,
        total_dag_blocks_count: block_stats.val_at(3)?,
        total_votes_weight: block_stats.val_at(4)?,
        max_votes_weight: block_stats.val_at(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::Rlp;

    fn addr(byte: u8) -> H160 {
        H160::from([byte; 20])
    }

    fn hash(id: u64) -> H256 {
        H256::from_low_u64_be(id)
    }

    fn tx(id: u64, gas_price: u64, gas_used: u64) -> RewardTransactionFact {
        RewardTransactionFact {
            hash: hash(id),
            gas_price: U256::from(gas_price),
            gas_used,
        }
    }

    fn dag(author: u8, difficulty: u16, txs: &[u64]) -> RewardDagBlockFact {
        RewardDagBlockFact {
            author: addr(author),
            difficulty,
            transaction_hashes: txs.iter().map(|id| hash(*id)).collect(),
        }
    }

    fn vote(voter: u8, weight: u64) -> RewardCertVoteFact {
        RewardCertVoteFact {
            voter: addr(voter),
            weight,
            period: 9,
        }
    }

    fn runtime(frequency: u32) -> RewardsStatsRuntime {
        RewardsStatsRuntime::new(
            RewardsStatsConfig {
                committee_size: 100,
                magnolia_period: 5,
                aspen_part_one_period: 20,
            },
            if frequency == 1 {
                Vec::new()
            } else {
                vec![RewardsFrequencyRule {
                    from_period: 0,
                    frequency,
                }]
            },
            Vec::new(),
        )
        .unwrap()
    }

    fn fact(period: u64) -> FinalizedRewardsPeriodFact {
        FinalizedRewardsPeriodFact {
            period,
            block_author: addr(1),
            blocks_per_year: 1234,
            dpos_eligible_total_vote_count: 90,
            transactions: vec![tx(1, 2, 10), tx(2, 3, 20)],
            dag_blocks: vec![dag(2, 7, &[1]), dag(3, 8, &[1, 2])],
            cert_votes: vec![vote(4, 15), vote(5, 25)],
        }
    }

    fn validator_stats_count(stats_rlp: &[u8]) -> usize {
        Rlp::new(stats_rlp).at(2).unwrap().item_count().unwrap()
    }

    #[test]
    fn default_frequency_distributes_every_period() {
        let mut runtime = runtime(1);
        let plan = runtime.process_period(fact(7));

        assert_eq!(plan.status, RewardsStatsStatus::Applied);
        assert!(!plan.cache_current_period);
        assert!(!plan.clear_cached_stats);
        assert_eq!(plan.distribution_stats.len(), 1);
        assert_eq!(plan.distribution_stats[0].period, 7);
        assert_eq!(validator_stats_count(&plan.current_block_stats_rlp), 4);
    }

    #[test]
    fn interval_caches_then_distributes_ordered_stats() {
        let mut runtime = runtime(3);

        let first = runtime.process_period(fact(1));
        assert!(first.cache_current_period);
        assert!(first.distribution_stats.is_empty());

        let second = runtime.process_period(fact(2));
        assert!(second.cache_current_period);
        assert!(second.distribution_stats.is_empty());

        let third = runtime.process_period(fact(3));
        assert!(!third.cache_current_period);
        assert!(third.clear_cached_stats);
        assert_eq!(
            third
                .distribution_stats
                .iter()
                .map(|entry| entry.period)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        runtime.clear_committed(3);
        let next = runtime.process_period(fact(6));
        assert_eq!(next.distribution_stats.len(), 1);
        assert_eq!(next.distribution_stats[0].period, 6);
    }

    #[test]
    fn decodes_structured_distribution_stats() {
        let mut runtime = runtime(2);

        assert!(
            runtime
                .process_period(fact(5))
                .distribution_stats
                .is_empty()
        );
        let plan = runtime.process_period(fact(6));
        let decoded = decode_rewards_block_distributions(&plan.distribution_stats).unwrap();

        assert_eq!(
            decoded.iter().map(|stats| stats.period).collect::<Vec<_>>(),
            vec![5, 6]
        );
        assert_eq!(decoded[0].block_author, addr(1));
        assert_eq!(decoded[0].blocks_per_year, 1234);
        assert_eq!(decoded[0].total_dag_blocks_count, 2);
        assert_eq!(decoded[0].total_votes_weight, 40);
        assert_eq!(decoded[0].max_votes_weight, 90);
        assert_eq!(
            decoded[0].validators_stats.get(&addr(2).0).unwrap(),
            &RewardsValidatorDistribution {
                dag_blocks_count: 1,
                vote_weight: 0,
                fees_rewards: U256::from(20u64),
            }
        );
    }

    #[test]
    fn pre_magnolia_ignores_transaction_fees() {
        let mut runtime = runtime(1);
        let plan = runtime.process_period(fact(4));
        let validators = Rlp::new(&plan.current_block_stats_rlp).at(2).unwrap();

        for entry in validators.iter() {
            let value = entry.at(1).unwrap();
            let fee = value.val_at::<U256>(2).unwrap();
            assert_eq!(fee, U256::zero());
        }
    }

    #[test]
    fn aspen_counts_minimum_difficulty_blocks_and_preserves_fees() {
        let mut runtime = runtime(1);
        let mut aspen_fact = fact(20);
        aspen_fact.dag_blocks = vec![
            dag(2, 8, &[1]),
            dag(3, 7, &[2]),
            dag(4, 7, &[1]),
            dag(5, 9, &[99]),
        ];
        let plan = runtime.process_period(aspen_fact);
        let rlp = Rlp::new(&plan.current_block_stats_rlp);

        assert_eq!(rlp.val_at::<u32>(3).unwrap(), 2);
        let validators = rlp.at(2).unwrap();
        let mut dag_counts = BTreeMap::new();
        let mut fees = BTreeMap::new();
        for entry in validators.iter() {
            let validator = entry.val_at::<H160>(0).unwrap();
            let value = entry.at(1).unwrap();
            dag_counts.insert(validator, value.val_at::<u32>(0).unwrap());
            fees.insert(validator, value.val_at::<U256>(2).unwrap());
        }
        assert_eq!(dag_counts.get(&addr(3)), Some(&1));
        assert_eq!(dag_counts.get(&addr(4)), Some(&1));
        assert_eq!(fees.get(&addr(2)), Some(&U256::from(20)));
        assert_eq!(fees.get(&addr(3)), Some(&U256::from(60)));
    }

    #[test]
    fn duplicate_cert_voter_is_rejected() {
        let mut runtime = runtime(1);
        let mut bad = fact(7);
        bad.cert_votes = vec![vote(4, 10), vote(4, 20)];

        let plan = runtime.process_period(bad);
        assert_eq!(plan.status, RewardsStatsStatus::Rejected);
        assert_eq!(plan.error_code, "REWARDS_STATS_DUPLICATE_VOTER");
    }
}
