//! Terminal staged rewards and end-block execution for the mixed-period profile.
//!
//! The adapter covers disabled-yield end-block processing and ordered
//! fixed-yield and Aspen-part-two distributions with any number of live
//! validators. It reuses FinalChain's reward planner as
//! semantic authority, reconstructs the pinned Go helper's intermediate write
//! order, and checks that reconstruction reaches the planner's complete DPoS
//! result. Go iterates each distribution's validator map in runtime map order,
//! while Rust chooses decoded address order as one valid execution order.
//! Per-validator reward rows are disjoint, non-negative account additions and
//! minted totals commute, and yield/supply writes surround the whole map loop;
//! the final logical state and root are therefore order-independent even though
//! intermediate raw and account mutations are not canonical. End-block
//! processing also rewrites a previously committed slashing jailed list when
//! at least one member expires, after the DPoS deferred rows and with every
//! source jail-block row authenticated. Scheduler-bound sessions reproduce
//! all-future skips and decreasing jail-duration behavior from the private live
//! timer. Standalone sessions retain the narrower authenticated-state guards.
//! Ordinary custody effects remain full-width and raw effects retain repeated
//! writes to the same validator-rewards row.

use super::account::{DposAccountPort, StagedDposAccountPort};
use super::raw::FinalChainNativeRawTrace;
use super::*;
use crate::native_projection_context::{
    FinalChainNativeRewardsBoundary, FinalChainNativeRewardsPhase,
};

/// Complete output of one bound session's terminal rewards/end-block phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRewardsOutcome {
    /// Actual terminal phase boundaries, in execution order.
    pub boundaries: Vec<FinalChainNativeRewardsBoundary>,
    /// Minted reward encoded into the FinalChain block header.
    pub total_reward: DposTokenAmount,
    /// Ordered ordinary custody effects for the execution journal.
    pub account_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Ordered raw rewards and deferred end-block operations.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
    /// Complete unpublished semantic state used by independent consensus replay.
    pub(in crate::final_chain) dpos_snapshot: DposSnapshot,
    /// Process-local cleanup timer derived from the FinalChain-issued basis.
    /// It remains absent for standalone sessions and cannot publish by itself.
    pub(in crate::final_chain) scheduler_successor:
        Option<FinalChainPreparedRewardSchedulerSuccessor>,
}

impl FinalChainNativeSession<'_> {
    /// Finishes one bound session through rewards and deferred DPoS end-block writes.
    ///
    /// The opaque plan must belong to this exact request and pending period. The
    /// current adapter accepts pre-Magnolia periods only with Complete owned
    /// validator shapes and proven source-period fee exclusion, and accepts
    /// Magnolia reward periods and ordered zero-or-more
    /// distribution rows with any number of validators and previously committed
    /// jailed-validator lists whose membership changes at this period. An
    /// all-future nonempty list, a decreasing Magnolia-to-Cacti jail duration,
    /// any same-period slashing mutation, and redelegation correction at the
    /// exact fix height remain unsupported. The parent snapshot rejects newly
    /// staged slashing, while the scheduler guards avoid guessing Go's
    /// process-local `nextCleanUpBlock`. A zero configured
    /// yield skips distributions exactly as
    /// `StateTransition::DistributeRewards` does, while retaining deferred
    /// end-block and slashing-list writes. Enabled rewards emit validator rows in
    /// Rust address order, one of the Go map loop's valid permutations; callers
    /// must treat intermediate order as noncanonical. Fixed-yield and Aspen
    /// part-two supply transitions are reconstructed. Any state-read, planner,
    /// reconstruction, or raw-integrity error aborts the session and exposes no
    /// result. Successful completion consumes the session phase and cannot be
    /// repeated.
    pub fn finish_rewards(
        &mut self,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeRewardsOutcome, FinalChainNativeSessionError> {
        let result = self.finish_rewards_inner(plan, state);
        #[cfg(feature = "synthetic-fixtures")]
        if self.pending_period.as_u64() == 5 {
            cleanup_diagnostic::record(format_args!(
                "finish_before pending={:?} outcome={} aborted={} finished={}",
                self.pending_period,
                if result.is_ok() { "OK" } else { "ERROR" },
                self.aborted,
                self.finished_rewards
            ));
        }
        let returned = match result {
            Ok(outcome) => {
                self.finished_rewards = true;
                self.dpos_state = outcome.dpos_snapshot.clone();
                Ok(outcome)
            }
            Err(
                error @ (FinalChainNativeSessionError::UnboundRewards
                | FinalChainNativeSessionError::RewardsPlanMismatch
                | FinalChainNativeSessionError::RewardsAlreadyFinished
                | FinalChainNativeSessionError::RewardsScopeUnsupported
                | FinalChainNativeSessionError::QuoteOutstanding),
            ) => Err(error),
            Err(error) => {
                self.aborted = true;
                Err(error)
            }
        };
        #[cfg(feature = "synthetic-fixtures")]
        if self.pending_period.as_u64() == 5 {
            cleanup_diagnostic::record(format_args!(
                "finish_after pending={:?} outcome={} aborted={} finished={} publication=NOT_OBSERVED",
                self.pending_period,
                if returned.is_ok() { "OK" } else { "ERROR" },
                self.aborted,
                self.finished_rewards
            ));
        }
        returned
    }

    fn finish_rewards_inner(
        &self,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeRewardsOutcome, FinalChainNativeSessionError> {
        if self.aborted {
            return Err(FinalChainNativeSessionError::Aborted);
        }
        if self.finished_rewards {
            return Err(FinalChainNativeSessionError::RewardsAlreadyFinished);
        }
        if self.prepared.is_some() {
            return Err(FinalChainNativeSessionError::QuoteOutstanding);
        }
        let Some(request_id) = self.request_id else {
            return Err(FinalChainNativeSessionError::UnboundRewards);
        };
        if plan.request_id != request_id || plan.period != self.pending_period {
            return Err(FinalChainNativeSessionError::RewardsPlanMismatch);
        }
        self.final_chain
            .validate_external_evm_rewards_stats_plan(plan)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;

        if (self
            .final_chain
            .pre_magnolia_fee_reward_period(self.pending_period)
            && !self.dpos_state.has_complete_validator_shapes())
            || self.pending_period == self.final_chain.rewards_config.fix_redelegate_block_num
        {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        self.validate_committed_slashing_cleanup_scope()?;
        let distributions = decode_rewards_block_distributions(&plan.distribution_stats)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        // decode_rewards_block_distributions carries each source period from
        // the authenticated opaque plan, not the current payout coordinate.
        validate_source_period_fee_exclusion(
            self.final_chain.rewards_config.magnolia_period,
            &distributions,
        )?;
        validate_cached_source_period_fee_exclusion(
            self.final_chain.rewards_config.magnolia_period,
            plan,
        )?;
        let rewards_enabled = self.final_chain.rewards_config.yield_percentage != 0;

        let mut trace = FinalChainNativeRawTrace::new(state);
        trace.reward_phase(0, self.pending_period.as_u64(), 0, true);
        self.validate_deferred_origins(&mut trace)?;
        let mut next = self.dpos_state.clone();
        let mut accounts = if !rewards_enabled || distributions.is_empty() {
            StagedDposAccountPort::from_state(state)
        } else {
            StagedDposAccountPort::new([(
                DPOS_CONTRACT_ADDRESS,
                state.account(DPOS_CONTRACT_ADDRESS)?,
            )])
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?
            .with_observer(state)
        };

        trace.reward_phase(0, self.pending_period.as_u64(), 0, false);
        let planned = self
            .final_chain
            .plan_minted_rewards(self.pending_period, &distributions, &self.dpos_state)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if trace.mutation_count() != 0 || accounts.mutation_count() != 0 {
            return Err(domain("reward preparation emitted mutations"));
        }
        let mut boundaries = Vec::new();
        let reconstruction = if rewards_enabled {
            self.reconstruct_rewards(
                &distributions,
                &mut next,
                &mut trace,
                &mut accounts,
                &mut boundaries,
            )?
        } else {
            let sources = distributions
                .iter()
                .map(|stats| stats.period)
                .collect::<Vec<_>>();
            state.observe_stage(FinalChainNativeStageEvent::RewardDisabled {
                period: self.pending_period.as_u64(),
                sources: &sources,
            });
            for (ordinal, stats) in distributions.iter().enumerate() {
                boundaries.push(FinalChainNativeRewardsBoundary {
                    phase: FinalChainNativeRewardsPhase::Distribution {
                        source_period: stats.period,
                        ordinal,
                        executed: false,
                    },
                    raw_start: trace.mutation_count(),
                    raw_end: trace.mutation_count(),
                    ordinary_start: accounts.mutation_count(),
                    ordinary_end: accounts.mutation_count(),
                });
            }
            DposTokenAmount::zero()
        };
        if reconstruction != planned.total_minted_reward {
            return Err(FinalChainNativeSessionError::Domain(
                "selected reward reconstruction disagrees with FinalChain minted total".to_owned(),
            ));
        }

        if next.aspen_supply_state != planned.supply_after {
            return Err(FinalChainNativeSessionError::Domain(
                "selected reward supply reconstruction disagrees with FinalChain planner"
                    .to_owned(),
            ));
        }
        let raw_start = trace.mutation_count();
        let ordinary_start = accounts.mutation_count();
        trace.reward_phase(2, self.pending_period.as_u64(), 0, true);
        self.serialize_deferred_end_block(&next, &mut trace)?;
        trace.reward_phase(2, self.pending_period.as_u64(), 0, false);
        boundaries.push(FinalChainNativeRewardsBoundary {
            phase: FinalChainNativeRewardsPhase::DeferredEndBlock,
            raw_start,
            raw_end: trace.mutation_count(),
            ordinary_start,
            ordinary_end: accounts.mutation_count(),
        });
        let raw_start = trace.mutation_count();
        let ordinary_start = accounts.mutation_count();
        trace.reward_phase(3, self.pending_period.as_u64(), 0, true);
        let (cleanup_ran, scheduler_successor) =
            self.serialize_slashing_cleanup(&mut next, &mut trace)?;
        trace.reward_phase(3, self.pending_period.as_u64(), 0, false);
        boundaries.push(FinalChainNativeRewardsBoundary {
            phase: FinalChainNativeRewardsPhase::SlashingCleanup,
            raw_start,
            raw_end: trace.mutation_count(),
            ordinary_start,
            ordinary_end: accounts.mutation_count(),
        });

        let mut expected = self.dpos_state.clone();
        let mut expected_rewards = planned.dpos_rewards;
        let mut fees = if rewards_enabled {
            fee_rewards_from_distribution_stats(&distributions)
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?
        } else {
            BTreeMap::new()
        };
        retain_existing_validator_fee_rewards(&mut fees, &expected);
        merge_reward_map(&mut expected_rewards.commission_rewards, &fees)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        self.final_chain
            .apply_dpos_reward_deltas(&mut expected, expected_rewards, planned.supply_after)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if cleanup_ran {
            cleanup_slashing_jailed_validators(&mut expected, self.pending_period);
        }
        if next != expected {
            return Err(FinalChainNativeSessionError::Domain(
                "selected reward write reconstruction disagrees with FinalChain semantic kernel"
                    .to_owned(),
            ));
        }

        Ok(FinalChainNativeRewardsOutcome {
            boundaries,
            total_reward: reconstruction,
            account_mutations: accounts.into_mutations(),
            raw_mutations: trace.finish(),
            dpos_snapshot: next,
            scheduler_successor,
        })
    }

    fn reconstruct_rewards(
        &self,
        distributions: &[RewardsBlockDistribution],
        snapshot: &mut DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
        accounts: &mut StagedDposAccountPort,
        boundaries: &mut Vec<FinalChainNativeRewardsBoundary>,
    ) -> std::result::Result<DposTokenAmount, FinalChainNativeSessionError> {
        let mut total_reward = DposTokenAmount::zero();
        let mut dynamic_total_supply = snapshot
            .aspen_supply_state
            .total_supply()
            .map(StoredDposTokenAmount::amount);

        for (ordinal, stats) in distributions.iter().enumerate() {
            let raw_start = trace.mutation_count();
            let ordinary_start = accounts.mutation_count();
            trace.reward_phase(
                1,
                stats.period,
                u64::try_from(ordinal).map_err(|_| domain("reward ordinal overflow"))?,
                true,
            );
            let reward_context = self
                .final_chain
                .minted_block_reward(self.pending_period, stats, snapshot, dynamic_total_supply)
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            if let Some(total_supply_before) = reward_context.total_supply_before {
                self.serialize_aspen_distribution_start(
                    snapshot,
                    total_supply_before,
                    reward_context.current_yield,
                    trace,
                )?;
            }
            let minted = self.reconstruct_distribution_rewards(
                stats,
                reward_context.block_reward,
                snapshot,
                trace,
                accounts,
            )?;
            total_reward = total_reward
                .checked_add(minted)
                .ok_or_else(|| domain("total minted reward overflow"))?;

            if let Some(total_supply_before) = reward_context.total_supply_before {
                let total_supply_after = total_supply_before
                    .checked_add(minted)
                    .ok_or_else(|| domain("Aspen total supply overflow"))?;
                if total_supply_after.as_u256()
                    > self.final_chain.rewards_config.aspen_max_supply.as_u256()
                {
                    return Err(domain("Aspen total supply exceeds maximum supply"));
                }
                self.serialize_aspen_distribution_end(
                    snapshot,
                    total_supply_before,
                    total_supply_after,
                    reward_context.current_yield,
                    trace,
                )?;
                dynamic_total_supply = Some(total_supply_after);
            } else if self.pending_period >= self.final_chain.rewards_config.aspen_part_one_period {
                self.serialize_legacy_minted_distribution(snapshot, minted, trace)?;
            }
            trace.reward_phase(
                1,
                stats.period,
                u64::try_from(ordinal).map_err(|_| domain("reward ordinal overflow"))?,
                false,
            );
            boundaries.push(FinalChainNativeRewardsBoundary {
                phase: FinalChainNativeRewardsPhase::Distribution {
                    source_period: stats.period,
                    ordinal,
                    executed: true,
                },
                raw_start,
                raw_end: trace.mutation_count(),
                ordinary_start,
                ordinary_end: accounts.mutation_count(),
            });
        }
        Ok(total_reward)
    }

    fn reconstruct_distribution_rewards(
        &self,
        stats: &RewardsBlockDistribution,
        block_reward: DposTokenAmount,
        snapshot: &mut DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
        accounts: &mut StagedDposAccountPort,
    ) -> std::result::Result<DposTokenAmount, FinalChainNativeSessionError> {
        let block_reward = block_reward.as_u256();
        let mut dag_reward = block_reward;
        let mut vote_reward = U256::zero();
        let mut author_reward = U256::zero();
        if stats.total_votes_weight > 0 {
            dag_reward = percent_of(
                block_reward,
                self.final_chain.rewards_config.dag_proposers_reward_percent,
                "DAG proposer reward",
            )
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            vote_reward = block_reward
                .checked_sub(dag_reward)
                .ok_or_else(|| domain("vote reward subtraction underflow"))?;
            let bonus = percent_of(
                block_reward,
                self.final_chain
                    .rewards_config
                    .max_block_author_reward_percent,
                "block author reward",
            )
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            vote_reward = vote_reward
                .checked_sub(bonus)
                .ok_or_else(|| domain("block author reward exceeds vote reward"))?;
            let maximum = stats.max_votes_weight.max(stats.total_votes_weight);
            author_reward = if maximum == stats.total_votes_weight {
                bonus
            } else {
                let threshold = maximum
                    .checked_mul(2)
                    .and_then(|value| value.checked_div(3))
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| domain("reward max vote weight overflow"))?;
                let denominator = maximum
                    .checked_sub(threshold)
                    .ok_or_else(|| domain("reward max vote weight denominator underflow"))?;
                if denominator == 0 {
                    U256::zero()
                } else {
                    bonus
                        .checked_mul(U256::from(
                            stats.total_votes_weight.saturating_sub(threshold),
                        ))
                        .ok_or_else(|| domain("block author reward multiplication overflow"))?
                        / U256::from(denominator)
                }
            };
        }

        let mut total = U256::zero();
        if !author_reward.is_zero() && snapshot.total_stakes.contains_key(&stats.block_author.0) {
            let before = encode_reward_row(snapshot, stats.block_author.0);
            self.add_reward_row(snapshot, stats.block_author.0, author_reward, U256::zero())?;
            self.put_reward_row(snapshot, stats.block_author.0, &before, trace)?;
            total = total
                .checked_add(author_reward)
                .ok_or_else(|| domain("total minted reward overflow"))?;
        }

        trace.reward_map(true, true, [0; 20], 0);
        for (ordinal, (validator, validator_stats)) in stats.validators_stats.iter().enumerate() {
            let ordinal =
                u64::try_from(ordinal).map_err(|_| domain("reward validator ordinal overflow"))?;
            trace.reward_map(true, false, *validator, ordinal);
            trace.reward_fee_source(
                *validator,
                validator_stats.fees_rewards.as_u256().to_big_endian(),
            );
            let mut validator_reward = U256::zero();
            if validator_stats.dag_blocks_count > 0 {
                if stats.total_dag_blocks_count == 0 {
                    return Err(domain("reward DAG block count is zero"));
                }
                validator_reward = dag_reward
                    .checked_mul(U256::from(validator_stats.dag_blocks_count))
                    .ok_or_else(|| domain("validator DAG reward multiplication overflow"))?
                    / U256::from(stats.total_dag_blocks_count);
            }
            if validator_stats.vote_weight > 0 {
                if stats.total_votes_weight == 0 {
                    return Err(domain("reward total vote weight is zero"));
                }
                validator_reward = validator_reward
                    .checked_add(
                        vote_reward
                            .checked_mul(U256::from(validator_stats.vote_weight))
                            .ok_or_else(|| {
                                domain("validator vote reward multiplication overflow")
                            })?
                            / U256::from(stats.total_votes_weight),
                    )
                    .ok_or_else(|| domain("validator reward overflow"))?;
            }
            let validator_exists = snapshot.total_stakes.contains_key(validator);
            trace.reward_validator_exists(*validator, validator_exists);
            if validator_exists {
                let before = encode_reward_row(snapshot, *validator);
                let fee = validator_stats.fees_rewards.as_u256();
                if !fee.is_zero() {
                    accounts
                        .add_balance(
                            DPOS_CONTRACT_ADDRESS,
                            &BigUint::from_bytes_be(&u256_to_big_endian(fee)),
                        )
                        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
                }
                self.add_reward_row(snapshot, *validator, validator_reward, fee)?;
                // Go AddValidatorRewards writes this row for every existing validator,
                // including an arithmetically unchanged zero-reward row.
                self.put_reward_row(snapshot, *validator, &before, trace)?;
                total = total
                    .checked_add(validator_reward)
                    .ok_or_else(|| domain("total minted reward overflow"))?;
            }
            trace.reward_map(false, false, *validator, ordinal);
        }
        trace.reward_map(
            false,
            true,
            [0; 20],
            u64::try_from(stats.validators_stats.len())
                .map_err(|_| domain("reward validator count overflow"))?,
        );
        // Go performs this AddBalance even when total is zero; the staged port
        // therefore emits Touch rather than dropping the lifecycle operation.
        accounts
            .add_balance(
                DPOS_CONTRACT_ADDRESS,
                &BigUint::from_bytes_be(&u256_to_big_endian(total)),
            )
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        Ok(DposTokenAmount::from(total))
    }

    fn add_reward_row(
        &self,
        snapshot: &mut DposSnapshot,
        validator: [u8; 20],
        minted: U256,
        fee: U256,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let metadata = snapshot
            .validator_metadata
            .get(&validator)
            .ok_or_else(|| domain("selected reward validator metadata is absent"))?;
        let commission = percent_of_max_commission(minted, metadata.commission)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?
            .checked_add(fee)
            .ok_or_else(|| domain("selected validator commission reward overflow"))?;
        let delegator = minted
            .checked_sub(
                percent_of_max_commission(minted, metadata.commission)
                    .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?,
            )
            .ok_or_else(|| domain("selected delegator reward subtraction underflow"))?;
        add_reward_pool(&mut snapshot.commission_rewards, validator, commission)?;
        add_reward_pool(&mut snapshot.delegator_rewards, validator, delegator)?;
        Ok(())
    }

    fn put_reward_row(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
        expected: &[u8],
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let key = ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &validator]));
        let current = trace.expectation(DPOS_CONTRACT_ADDRESS, key)?;
        validate_raw_value(&current, expected, "validator rewards")?;
        trace.put(
            DPOS_CONTRACT_ADDRESS,
            key,
            encode_reward_row(snapshot, validator),
        )
    }

    fn serialize_aspen_distribution_start(
        &self,
        snapshot: &mut DposSnapshot,
        total_supply_before: DposTokenAmount,
        current_yield: AspenYield,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let total_key = ConcreteStorageKey(concrete_storage_key(&[&[7]]));
        let total_current = trace.expectation(DPOS_CONTRACT_ADDRESS, total_key)?;
        match &snapshot.aspen_supply_state {
            AspenSupplyState::Unmigrated { minted_tokens } => {
                validate_raw_value(&total_current, &[], "Aspen total supply")?;
                trace.put(
                    DPOS_CONTRACT_ADDRESS,
                    total_key,
                    concrete_u256_bytes(total_supply_before.as_u256()),
                )?;
                let minted_key = ConcreteStorageKey(concrete_storage_key(&[&[6]]));
                let minted_current = trace.expectation(DPOS_CONTRACT_ADDRESS, minted_key)?;
                validate_raw_value(
                    &minted_current,
                    &concrete_u256_bytes(minted_tokens.as_u256()),
                    "minted rewards",
                )?;
                trace.put(DPOS_CONTRACT_ADDRESS, minted_key, Vec::new())?;
            }
            AspenSupplyState::Migrated {
                total_supply,
                current_yield: previous_yield,
            } => {
                if total_supply.amount() != total_supply_before {
                    return Err(domain(
                        "Aspen planner supply disagrees with reconstructed supply",
                    ));
                }
                validate_raw_value(
                    &total_current,
                    &concrete_u256_bytes(total_supply_before.as_u256()),
                    "Aspen total supply",
                )?;
                let yield_key = ConcreteStorageKey(concrete_storage_key(&[&[8]]));
                validate_raw_value(
                    &trace.expectation(DPOS_CONTRACT_ADDRESS, yield_key)?,
                    &concrete_rlp_u64(previous_yield.as_u64()),
                    "Aspen current yield",
                )?;
            }
        }
        let yield_key = ConcreteStorageKey(concrete_storage_key(&[&[8]]));
        if matches!(
            snapshot.aspen_supply_state,
            AspenSupplyState::Unmigrated { .. }
        ) {
            validate_raw_value(
                &trace.expectation(DPOS_CONTRACT_ADDRESS, yield_key)?,
                &[],
                "Aspen current yield",
            )?;
        }
        trace.put(
            DPOS_CONTRACT_ADDRESS,
            yield_key,
            concrete_rlp_u64(current_yield.as_u64()),
        )?;
        snapshot.aspen_supply_state = AspenSupplyState::Migrated {
            total_supply: StoredDposTokenAmount::canonical_after_mutation(total_supply_before),
            current_yield,
        };
        Ok(())
    }

    fn serialize_aspen_distribution_end(
        &self,
        snapshot: &mut DposSnapshot,
        total_supply_before: DposTokenAmount,
        total_supply_after: DposTokenAmount,
        current_yield: AspenYield,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let key = ConcreteStorageKey(concrete_storage_key(&[&[7]]));
        validate_raw_value(
            &trace.expectation(DPOS_CONTRACT_ADDRESS, key)?,
            &concrete_u256_bytes(total_supply_before.as_u256()),
            "Aspen total supply",
        )?;
        trace.put(
            DPOS_CONTRACT_ADDRESS,
            key,
            concrete_u256_bytes(total_supply_after.as_u256()),
        )?;
        snapshot.aspen_supply_state = AspenSupplyState::Migrated {
            total_supply: StoredDposTokenAmount::canonical_after_mutation(total_supply_after),
            current_yield,
        };
        Ok(())
    }

    fn serialize_legacy_minted_distribution(
        &self,
        snapshot: &mut DposSnapshot,
        minted: DposTokenAmount,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let AspenSupplyState::Unmigrated { minted_tokens } = &snapshot.aspen_supply_state else {
            return Err(domain(
                "legacy minted counter is unavailable after Aspen migration",
            ));
        };
        let before = minted_tokens.amount();
        let after = before
            .checked_add(minted)
            .ok_or_else(|| domain("Aspen minted-token counter overflow"))?;
        let key = ConcreteStorageKey(concrete_storage_key(&[&[6]]));
        validate_raw_value(
            &trace.expectation(DPOS_CONTRACT_ADDRESS, key)?,
            &concrete_u256_bytes(before.as_u256()),
            "minted rewards",
        )?;
        trace.put(
            DPOS_CONTRACT_ADDRESS,
            key,
            concrete_u256_bytes(after.as_u256()),
        )?;
        snapshot.aspen_supply_state = AspenSupplyState::Unmigrated {
            minted_tokens: StoredDposTokenAmount::canonical_after_mutation(after),
        };
        Ok(())
    }

    fn validate_deferred_origins(
        &self,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let vote_key = ConcreteStorageKey(concrete_storage_key(&[&[4]]));
        // Go leaves this scalar absent when genesis registers no validators.
        // Later zero values still require their normal committed encoding.
        let empty_genesis = self.pending_period.as_u64() == 1
            && self.period_start_total_vote_count == 0
            && self
                .final_chain
                .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
                .map_err(custody::map_kernel_error)?
                .validator_order
                .is_empty();
        validate_raw_value(
            &trace.expectation(DPOS_CONTRACT_ADDRESS, vote_key)?,
            &if empty_genesis {
                Vec::new()
            } else {
                concrete_compact_u64(self.period_start_total_vote_count)
            },
            "eligible vote count",
        )?;
        let amount_key = ConcreteStorageKey(concrete_storage_key(&[&[5]]));
        validate_raw_value(
            &trace.expectation(DPOS_CONTRACT_ADDRESS, amount_key)?,
            &concrete_u256_bytes(self.period_start_amount_delegated),
            "amount delegated",
        )
    }

    #[cfg(test)]
    pub(super) fn serialize_deferred_end_block_for_test(
        &self,
        snapshot: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        self.serialize_deferred_end_block(snapshot, trace)
    }

    fn serialize_deferred_end_block(
        &self,
        snapshot: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        if snapshot.total_vote_count != self.period_start_total_vote_count {
            trace.put(
                DPOS_CONTRACT_ADDRESS,
                ConcreteStorageKey(concrete_storage_key(&[&[4]])),
                concrete_compact_u64(snapshot.total_vote_count),
            )?;
        }
        let amount = total_staked_amount(snapshot)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if amount != self.period_start_amount_delegated {
            trace.put(
                DPOS_CONTRACT_ADDRESS,
                ConcreteStorageKey(concrete_storage_key(&[&[5]])),
                concrete_u256_bytes(amount),
            )?;
        }
        Ok(())
    }

    /// Serializes the legacy slashing list cleanup after DPoS EndBlock writes.
    ///
    /// Go reads a previously committed nonempty list in stored order, retains
    /// entries whose jail block is strictly greater than the current block, and
    /// puts the resulting RLP list when its process-local scheduler runs.
    /// Per-validator jail-block rows are authenticated and remain untouched,
    /// including rows for validators removed from the list. An empty input list
    /// returns without a raw write. An all-future list is rejected because the
    /// authenticated state does not contain the scheduler needed to decide
    /// whether Go rewrites or skips it. A Cacti jail duration below Magnolia is
    /// also rejected: Go never lowers a previously cached Magnolia expiry when
    /// a later Cacti transaction overwrites a jail row with an earlier block.
    fn serialize_slashing_cleanup(
        &self,
        snapshot: &mut DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<
        (bool, Option<FinalChainPreparedRewardSchedulerSuccessor>),
        FinalChainNativeSessionError,
    > {
        let parent = self
            .pending_period
            .as_u64()
            .checked_sub(1)
            .ok_or(FinalChainNativeSessionError::PeriodMismatch)?;
        let batched_parent = self
            .final_chain
            .reward_cleanup_visible_snapshot(parent.into(), snapshot, self.reward_scheduler_basis)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let visible = batched_parent.as_ref().unwrap_or(snapshot);
        let scheduled = self.reward_scheduler_basis.map(|basis| {
            basis.plan_cleanup(
                &visible.slashing_jailed_validators,
                &visible.slashing_jail_blocks,
            )
        });
        #[cfg(feature = "synthetic-fixtures")]
        if self.pending_period.as_u64() == 5 && cleanup_diagnostic::is_active() {
            let states = [
                ("snapshot", &*snapshot),
                ("visible", visible),
                ("eligibility", &self.eligibility_state),
                ("session", &self.dpos_state),
            ];
            if states.iter().any(|(_, state)| {
                state.slashing_jailed_validators.len() > 64 || state.slashing_jail_blocks.len() > 64
            }) || scheduled
                .as_ref()
                .is_some_and(|plan| plan.retained_validators.len() > 64)
            {
                cleanup_diagnostic::overflow();
            } else {
                cleanup_diagnostic::record(format_args!(
                    "decision pending={:?} parent={} request={:?} origin={:?} eligibility_period={:?} visible_batched={}",
                    self.pending_period,
                    parent,
                    self.request_id,
                    self.origin,
                    self.eligibility_period,
                    batched_parent.is_some()
                ));
                cleanup_diagnostic::record(format_args!(
                    "scheduler basis={:?}",
                    self.reward_scheduler_basis
                ));
                match &scheduled {
                    Some(plan) => {
                        cleanup_diagnostic::record(format_args!(
                            "plan present=true ran={} retained_count={} successor={:?}",
                            plan.ran,
                            plan.retained_validators.len(),
                            plan.successor
                        ));
                        for (index, address) in plan.retained_validators.iter().enumerate() {
                            cleanup_diagnostic::record(format_args!(
                                "retained index={} address={:?}",
                                index, address
                            ));
                        }
                    }
                    None => cleanup_diagnostic::record(format_args!("plan present=false")),
                }
                for (role, state) in states {
                    cleanup_diagnostic::record(format_args!(
                        "state_counts role={} list={} blocks={}",
                        role,
                        state.slashing_jailed_validators.len(),
                        state.slashing_jail_blocks.len()
                    ));
                    for (index, address) in state.slashing_jailed_validators.iter().enumerate() {
                        cleanup_diagnostic::record(format_args!(
                            "state_list role={} index={} address={:?}",
                            role, index, address
                        ));
                    }
                    for (address, block) in &state.slashing_jail_blocks {
                        cleanup_diagnostic::record(format_args!(
                            "state_block role={} address={:?} block={}",
                            role, address, block
                        ));
                    }
                }
            }
        }
        if let Some(plan) = &scheduled {
            if !plan.ran {
                return Ok((false, Some(plan.successor)));
            }
        } else {
            if snapshot.slashing_jailed_validators.is_empty() {
                return Ok((false, None));
            }
            // Without a FinalChain-issued process-local basis, fixed or
            // nondecreasing jail duration plus an expiring member is the only
            // state-derived subset whose scheduler decision is determined.
            if self.final_chain.cacti_active(self.pending_period)
                && self.final_chain.rewards_config.cacti_jail_time
                    < self.final_chain.rewards_config.magnolia_jail_time
            {
                return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
            }
        }
        let key = ConcreteStorageKey(slashing_jailed_validators_key());
        let before = encode_slashing_jailed_list(snapshot);
        let actual = trace.current(SLASHING_CONTRACT_ADDRESS, key);
        #[cfg(feature = "synthetic-fixtures")]
        if self.pending_period.as_u64() == 5 {
            if before.len() > 1024
                || matches!(&actual, Ok(ConcreteRead::Present(value)) if value.len() > 1024)
            {
                cleanup_diagnostic::overflow();
            } else {
                cleanup_diagnostic::record(format_args!(
                    "compare pending={:?} address={:?} key={:?} expected={:?} actual={:?} trace_cache=UNKNOWN authority=UNKNOWN",
                    self.pending_period, SLASHING_CONTRACT_ADDRESS, key, before, actual
                ));
            }
        }
        let actual = actual?;
        if batched_parent.is_none()
            && matches!(actual, ConcreteRead::Absent)
            && scheduled.is_some()
            && !trace.account(SLASHING_CONTRACT_ADDRESS)?.exists
        {
            let basis = self
                .reward_scheduler_basis
                .expect("scheduled cleanup has a basis");
            let empty = basis.plan_cleanup(&[], &Default::default());
            debug_assert!(!empty.ran);
            #[cfg(feature = "synthetic-fixtures")]
            if cleanup_diagnostic::is_active() {
                cleanup_diagnostic::record(format_args!(
                    "cleanup_final ran=false authority=account_absent successor={:?}",
                    empty.successor
                ));
            }
            return Ok((false, Some(empty.successor)));
        }
        let validation = validate_raw_value(&actual, &before, "slashing jailed validators");
        #[cfg(feature = "synthetic-fixtures")]
        if self.pending_period.as_u64() == 5 {
            cleanup_diagnostic::record(format_args!("validation result={:?}", validation));
        }
        validation?;
        let mut expires = false;
        for validator in &snapshot.slashing_jailed_validators {
            let jail_key = ConcreteStorageKey(concrete_storage_key(&[&[0], validator]));
            let jail_block = snapshot.slashing_jail_blocks.get(validator).copied();
            let expected = jail_block.map(concrete_rlp_u64).unwrap_or_default();
            validate_raw_value(
                &trace.expectation(SLASHING_CONTRACT_ADDRESS, jail_key)?,
                &expected,
                "slashing validator jail block",
            )?;
            expires |= jail_block
                .map(|block| block <= self.pending_period.as_u64())
                .unwrap_or(true);
        }
        if scheduled.is_none() && !expires {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        if let Some(plan) = &scheduled {
            snapshot.slashing_jailed_validators = plan.retained_validators.clone();
        } else {
            cleanup_slashing_jailed_validators(snapshot, self.pending_period);
        }
        trace.put(
            SLASHING_CONTRACT_ADDRESS,
            key,
            encode_slashing_jailed_list(snapshot),
        )?;
        Ok((true, scheduled.map(|plan| plan.successor)))
    }

    /// Rejects newly staged slashing rows until their same-period Go visibility
    /// boundary has a dedicated native-session representation.
    fn validate_committed_slashing_cleanup_scope(
        &self,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let parent = self
            .pending_period
            .checked_sub_distance(1)
            .ok_or(FinalChainNativeSessionError::RewardsScopeUnsupported)?;
        let committed = self
            .final_chain
            .dpos_snapshot_at_finalized_block(parent)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if let Some(verified) = &self.verified_slashing {
            return if verified.matches(&self.dpos_state) {
                Ok(())
            } else {
                Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
            };
        }
        if self.dpos_state.slashing_jail_blocks != committed.slashing_jail_blocks
            || self.dpos_state.slashing_jailed_validators != committed.slashing_jailed_validators
            || self.dpos_state.slashing_double_voting_proofs
                != committed.slashing_double_voting_proofs
        {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        Ok(())
    }
}

fn slashing_jailed_validators_key() -> [u8; 32] {
    let mut key = [0_u8; 32];
    key[31] = 2;
    key
}

fn encode_slashing_jailed_list(snapshot: &DposSnapshot) -> Vec<u8> {
    let mut list = rlp::RlpStream::new_list(snapshot.slashing_jailed_validators.len());
    for validator in &snapshot.slashing_jailed_validators {
        list.append(&validator.as_slice());
    }
    list.out().to_vec()
}

fn add_reward_pool(
    pools: &mut DposRewardPools,
    validator: [u8; 20],
    amount: U256,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let current = pools
        .get(&validator)
        .map(StoredDposTokenAmount::as_u256)
        .unwrap_or_default();
    pools.insert(
        validator,
        StoredDposTokenAmount::canonical_u256_after_mutation(
            current
                .checked_add(amount)
                .ok_or_else(|| domain("selected validator reward pool overflow"))?,
        ),
    );
    Ok(())
}

fn encode_reward_row(snapshot: &DposSnapshot, validator: [u8; 20]) -> Vec<u8> {
    let mut row = rlp::RlpStream::new_list(2);
    row.append(
        &snapshot
            .delegator_rewards
            .get(&validator)
            .map(StoredDposTokenAmount::as_u256)
            .unwrap_or_default(),
    );
    row.append(
        &snapshot
            .commission_rewards
            .get(&validator)
            .map(StoredDposTokenAmount::as_u256)
            .unwrap_or_default(),
    );
    row.out().to_vec()
}

fn validate_raw_value(
    observed: &ConcreteRead<Vec<u8>>,
    expected: &[u8],
    label: &str,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    let matches = match observed {
        ConcreteRead::Present(value) => value == expected,
        ConcreteRead::Absent | ConcreteRead::Tombstone => expected.is_empty(),
    };
    if !matches {
        return Err(FinalChainNativeSessionError::RawIntegrity(format!(
            "{label} raw/domain facts disagree"
        )));
    }
    Ok(())
}

fn domain(message: &'static str) -> FinalChainNativeSessionError {
    FinalChainNativeSessionError::Domain(message.to_owned())
}

impl FinalChain {
    pub(in crate::final_chain) fn reward_cleanup_visible_snapshot(
        &self,
        parent: FinalChainBlockNumber,
        current: &DposSnapshot,
        basis: Option<reward_scheduler::FinalChainRewardSchedulerBasis>,
    ) -> Result<Option<DposSnapshot>, anyhow::Error> {
        #[cfg(feature = "synthetic-fixtures")]
        if self.synthetic_batched_lifecycle {
            // The authenticated process-local timer makes this a no-read
            // path. Keep transaction changes; publication still binds the
            // unchanged successor to the admitted generation.
            if basis
                .is_some_and(reward_scheduler::FinalChainRewardSchedulerBasis::skips_before_current)
            {
                return Ok(None);
            }
            let committed = self.dpos_snapshot_at_finalized_block(parent)?;
            let changed = current.slashing_jail_blocks != committed.slashing_jail_blocks
                || current.slashing_jailed_validators != committed.slashing_jailed_validators;
            anyhow::ensure!(
                !changed || committed.slashing_jailed_validators.is_empty(),
                "SYNTHETIC_BATCHED_EXISTING_JAIL_LIST_CHANGE_UNQUALIFIED"
            );
            return Ok(Some(committed));
        }
        let _ = (parent, current, basis);
        Ok(None)
    }
}

fn validate_cached_source_period_fee_exclusion(
    magnolia: FinalChainBlockNumber,
    plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
) -> std::result::Result<(), FinalChainNativeSessionError> {
    if plan.storage_update.cache_current_period {
        if plan.storage_update.current_period != plan.period
            || plan.storage_update.current_block_stats_rlp.is_empty()
        {
            return Err(FinalChainNativeSessionError::RewardsPlanMismatch);
        }
        let source = RewardsStatsPeriodRlp {
            period: plan.storage_update.current_period.as_u64(),
            data: plan.storage_update.current_block_stats_rlp.clone(),
        };
        let decoded = decode_rewards_block_distributions(&[source])
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        validate_source_period_fee_exclusion(magnolia, &decoded)?;
    }
    Ok(())
}

fn validate_source_period_fee_exclusion(
    magnolia: FinalChainBlockNumber,
    distributions: &[RewardsBlockDistribution],
) -> std::result::Result<(), FinalChainNativeSessionError> {
    for source in distributions {
        if source.period < magnolia.as_u64()
            && source
                .validators_stats
                .values()
                .any(|validator| !validator.fees_rewards.is_zero())
        {
            return Err(FinalChainNativeSessionError::Domain(
                "pre-Magnolia source statistics contain unexplained native fee rewards".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H160;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_types::concrete_state::ConcreteStateIdentity;
    use rustaxa_types::{FinalChainAccountBalance, GenesisValidatorMetadata};
    use serde_json::Value;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    const VALIDATOR_ONE: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31,
    ];
    const DELEGATOR_ONE: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x32,
    ];
    const VALIDATOR_TWO: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x41,
    ];
    const DELEGATOR_TWO: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x42,
    ];
    const ZERO_YIELD_DELEGATOR: [u8; 20] = [
        0x1a, 0x64, 0x2f, 0x0e, 0x3c, 0x3a, 0xf5, 0x45, 0xe7, 0xac, 0xbd, 0x38, 0xb0, 0x72, 0x51,
        0xb3, 0x99, 0x09, 0x14, 0xf1,
    ];
    const MISSING_AUTHOR: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x51,
    ];
    type RewardRows = BTreeMap<([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>>;

    type RewardRead = (([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>);

    #[derive(Default)]
    struct RewardState {
        rows: RefCell<RewardRows>,
        account_reads: Cell<usize>,
        raw_reads: RefCell<Vec<RewardRead>>,
        dpos_balance: RefCell<BigInt>,
    }

    impl RewardState {
        fn from_snapshot(snapshot: &DposSnapshot) -> Self {
            let mut rows: RewardRows = canonical_concrete_precompile_storage(snapshot, true)
                .unwrap()
                .into_iter()
                .map(|(identity, values)| {
                    (
                        identity,
                        ConcreteRead::Present(
                            values
                                .into_iter()
                                .next()
                                .expect("canonical reward row has one encoding"),
                        ),
                    )
                })
                .collect();
            // The Go genesis fixture has not materialized the three Aspen
            // lifecycle keys. Preserve absence instead of normalizing it to an
            // empty value so the staged expectations remain exact.
            for suffix in [6_u8, 7, 8] {
                rows.remove(&(DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[suffix]])));
            }
            Self {
                rows: RefCell::new(rows),
                account_reads: Cell::new(0),
                raw_reads: RefCell::new(Vec::new()),
                dpos_balance: RefCell::new(BigInt::from(3_000_u64)),
            }
        }

        fn apply(&self, mutation: &FinalChainNativeRawMutation) {
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    ConcreteRead::Present(value.as_bytes().to_vec())
                }
                FinalChainNativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            };
            self.rows
                .borrow_mut()
                .insert((mutation.address, mutation.key.0), value);
        }

        fn set_dpos_balance(&self, balance: u64) {
            *self.dpos_balance.borrow_mut() = BigInt::from(balance);
        }
    }

    impl FinalChainNativeStateRead for RewardState {
        fn account(
            &self,
            address: [u8; 20],
        ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
            self.account_reads.set(self.account_reads.get() + 1);
            if address != DPOS_CONTRACT_ADDRESS {
                return Err(FinalChainNativeStateReadError::Invariant(
                    "unexpected reward account read".to_owned(),
                ));
            }
            Ok(FinalChainNativeAccount {
                exists: true,
                nonce: FinalChainNonce::from_u64(1),
                balance: self.dpos_balance.borrow().clone(),
            })
        }

        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            let value = self
                .rows
                .borrow()
                .get(&(address, key.0))
                .cloned()
                .unwrap_or(ConcreteRead::Absent);
            self.raw_reads
                .borrow_mut()
                .push(((address, key.0), value.clone()));
            Ok(value)
        }
    }

    fn temp_db_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!(
                "rustaxa-current-rewards-{}-{nanos}",
                std::process::id()
            )),
            true,
        )
    }

    fn with_reward_chain(yield_percentage: u16, test: impl FnOnce(&FinalChain)) {
        with_reward_chain_and_jail_policy(yield_percentage, FinalChainBlockNumber::MAX, 0, 0, test);
    }

    fn with_reward_chain_and_jail_policy(
        yield_percentage: u16,
        cacti_period: FinalChainBlockNumber,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        test: impl FnOnce(&FinalChain),
    ) {
        with_reward_chain_setup(
            yield_percentage,
            cacti_period,
            magnolia_jail_time,
            cacti_jail_time,
            1,
            test,
        );
    }

    // Frequency-two uses the same complete owner fixture and post-delegation
    // ordinary balances as the independent synthetic Go genesis setup.
    fn with_reward_chain_setup(
        yield_percentage: u16,
        cacti_period: FinalChainBlockNumber,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        distribution_frequency: u32,
        test: impl FnOnce(&FinalChain),
    ) {
        with_reward_chain_setup_for_magnolia(
            yield_percentage,
            cacti_period,
            magnolia_jail_time,
            cacti_jail_time,
            distribution_frequency,
            FinalChainBlockNumber::GENESIS,
            test,
        );
    }
    fn with_reward_chain_setup_for_magnolia(
        yield_percentage: u16,
        cacti_period: FinalChainBlockNumber,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        distribution_frequency: u32,
        magnolia_period: FinalChainBlockNumber,
        test: impl FnOnce(&FinalChain),
    ) {
        with_owned_reward_chain_setup(
            yield_percentage,
            cacti_period,
            magnolia_jail_time,
            cacti_jail_time,
            distribution_frequency,
            magnolia_period,
            |chain| test(&chain),
        );
    }

    fn with_owned_reward_chain_setup(
        yield_percentage: u16,
        cacti_period: FinalChainBlockNumber,
        magnolia_jail_time: u64,
        cacti_jail_time: u64,
        distribution_frequency: u32,
        magnolia_period: FinalChainBlockNumber,
        test: impl FnOnce(Arc<FinalChain>),
    ) {
        let path = temp_db_path();
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let validator = |address, delegator, stake: u64, commission| GenesisValidator {
            address,
            vrf_key: [0; 32],
            total_stake: U256::from(stake).to_big_endian().to_vec(),
            delegations: vec![(delegator, U256::from(stake).to_big_endian().to_vec())],
            metadata: GenesisValidatorMetadata {
                owner: delegator,
                commission,
                ..Default::default()
            },
        };
        let chain = Arc::new(
            FinalChain::new_with_rewards_config_and_ficus_activation(
                storage.clone(),
                1_000_000.into(),
                0,
                crate::FinalChainGenesisAllocation {
                    accounts: if distribution_frequency == 2 {
                        [(DELEGATOR_ONE, 1_000_u64), (DELEGATOR_TWO, 1_000)]
                            .into_iter()
                            .map(|(address, balance)| GenesisAccount {
                                address,
                                balance: FinalChainAccountBalance::from_cpp_genesis_bytes(
                                    &U256::from(balance).to_big_endian(),
                                )
                                .unwrap(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    },
                    validators: vec![
                        validator(VALIDATOR_ONE, DELEGATOR_ONE, 1_000, 100),
                        validator(VALIDATOR_TWO, DELEGATOR_TWO, 2_000, 2_500),
                    ],
                },
                GenesisDposConfig {
                    eligibility_balance_threshold: U256::from(100).into(),
                    vote_eligibility_balance_step: U256::from(10).into(),
                    validator_maximum_stake: U256::from(1_000_000).into(),
                    minimum_deposit: U256::one().into(),
                    ..Default::default()
                },
                FinalChainRewardsConfig {
                    committee_size: 10,
                    magnolia_period,
                    aspen_part_one_period: FinalChainBlockNumber::GENESIS,
                    aspen_part_two_period: 1.into(),
                    fix_redelegate_block_num: FinalChainBlockNumber::MAX,
                    max_block_author_reward_percent: 10,
                    dag_proposers_reward_percent: 50,
                    yield_percentage,
                    dpos_blocks_per_year: 10,
                    cornus_period: FinalChainBlockNumber::GENESIS,
                    genesis_balance_sum: Some(DposTokenAmount::from(U256::from(5_000))),
                    aspen_max_supply: DposTokenAmount::from(U256::from(6_000)),
                    aspen_generated_rewards: DposTokenAmount::zero(),
                    cacti_period,
                    magnolia_jail_time,
                    cacti_jail_time,
                    rewards_distribution_frequency: vec![(
                        FinalChainBlockNumber::GENESIS,
                        distribution_frequency,
                    )],
                    ..Default::default()
                },
                FinalChainBlockNumber::GENESIS,
            )
            .unwrap(),
        );
        test(chain);
        drop(storage);
        let _ = crate::closeout_test_scratch::dispose(path);
    }

    fn with_current_reward_chain(test: impl FnOnce(&FinalChain)) {
        with_reward_chain(1, test);
    }

    fn seed_committed_jail(chain: &FinalChain, jail_block: u64) {
        let mut snapshots = chain.dpos_snapshots.lock().unwrap();
        let snapshot = snapshots
            .get_mut(&FinalChainBlockNumber::GENESIS)
            .expect("reward test chain has a genesis DPoS snapshot");
        snapshot
            .slashing_jail_blocks
            .insert(ZERO_YIELD_DELEGATOR, jail_block);
        snapshot
            .slashing_jailed_validators
            .push(ZERO_YIELD_DELEGATOR);
    }

    fn seed_committed_jail_list_without_block(chain: &FinalChain) {
        chain
            .dpos_snapshots
            .lock()
            .unwrap()
            .get_mut(&FinalChainBlockNumber::GENESIS)
            .expect("reward test chain has a genesis DPoS snapshot")
            .slashing_jailed_validators
            .push(ZERO_YIELD_DELEGATOR);
    }

    fn distribution_rlp(
        period: u64,
        author: [u8; 20],
        validator: [u8; 20],
        fee: u64,
    ) -> RewardsStatsPeriodRlp {
        distributions_rlp(period, author, &[(validator, fee)], 1, 10)
    }

    fn distributions_rlp(
        period: u64,
        author: [u8; 20],
        validator_fees: &[([u8; 20], u64)],
        total_dag_blocks_count: u32,
        total_votes_weight: u64,
    ) -> RewardsStatsPeriodRlp {
        let mut validators = rlp::RlpStream::new_list(validator_fees.len());
        for (validator, fee) in validator_fees {
            let mut validator_stats = rlp::RlpStream::new_list(3);
            validator_stats.append(&1_u32);
            validator_stats.append(&10_u64);
            validator_stats.append(&U256::from(*fee));
            let mut validator_row = rlp::RlpStream::new_list(2);
            validator_row.append(&H160::from(*validator));
            validator_row.append_raw(&validator_stats.out(), 1);
            validators.append_raw(&validator_row.out(), 1);
        }
        let mut block = rlp::RlpStream::new_list(6);
        block.append(&H160::from(author));
        block.append(&10_u32);
        block.append_raw(&validators.out(), 1);
        block.append(&total_dag_blocks_count);
        block.append(&total_votes_weight);
        block.append(&total_votes_weight);
        RewardsStatsPeriodRlp {
            period,
            data: block.out().to_vec(),
        }
    }

    fn plan(rows: Vec<RewardsStatsPeriodRlp>) -> FinalChainPreparedExternalEvmRewardsStatsPlan {
        FinalChainPreparedExternalEvmRewardsStatsPlan {
            request_id: [0x72; 32],
            period: 1.into(),
            expected_prior_head: FinalChainBlockNumber::GENESIS,
            expected_runtime_generation: 0,
            distribution_stats: rows,
            storage_update: FinalChainExternalEvmRewardsStatsUpdate {
                current_period: 1.into(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn pre_magnolia_source_period_fees_are_classified_from_each_bound_record() {
        let accepted = decode_rewards_block_distributions(&[
            distribution_rlp(9, VALIDATOR_ONE, VALIDATOR_ONE, 0),
            distribution_rlp(10, VALIDATOR_TWO, VALIDATOR_TWO, 17),
        ])
        .unwrap();
        assert_eq!(
            accepted.iter().map(|row| row.period).collect::<Vec<_>>(),
            vec![9, 10]
        );
        validate_source_period_fee_exclusion(10.into(), &accepted).unwrap();
        let invalid = decode_rewards_block_distributions(&[
            distribution_rlp(9, VALIDATOR_ONE, VALIDATOR_ONE, 11),
            distribution_rlp(10, VALIDATOR_TWO, VALIDATOR_TWO, 17),
        ])
        .unwrap();
        assert!(validate_source_period_fee_exclusion(10.into(), &invalid).is_err());
        validate_source_period_fee_exclusion(0.into(), &invalid).unwrap();
    }

    #[test]
    fn pre_magnolia_cached_source_fees_require_bound_source_period_and_zero_fee() {
        let mut prepared = plan(Vec::new());
        prepared.period = 9.into();
        prepared.expected_prior_head = 8.into();
        prepared.storage_update.cache_current_period = true;
        prepared.storage_update.current_period = 9.into();
        prepared.storage_update.current_block_stats_rlp =
            distribution_rlp(9, VALIDATOR_ONE, VALIDATOR_ONE, 11).data;
        assert!(validate_cached_source_period_fee_exclusion(10.into(), &prepared).is_err());
        prepared.storage_update.current_block_stats_rlp =
            distribution_rlp(9, VALIDATOR_ONE, VALIDATOR_ONE, 0).data;
        validate_cached_source_period_fee_exclusion(10.into(), &prepared).unwrap();
        prepared.storage_update.current_period = 10.into();
        assert_eq!(
            validate_cached_source_period_fee_exclusion(10.into(), &prepared),
            Err(FinalChainNativeSessionError::RewardsPlanMismatch)
        );
        prepared.period = 10.into();
        prepared.storage_update.current_block_stats_rlp =
            distribution_rlp(10, VALIDATOR_ONE, VALIDATOR_ONE, 11).data;
        validate_cached_source_period_fee_exclusion(10.into(), &prepared).unwrap();
    }

    struct PreMagnoliaRewardFault<'a> {
        state: &'a RewardState,
        key: ConcreteStorageKey,
        mode: u8,
    }
    impl FinalChainNativeStateRead for PreMagnoliaRewardFault<'_> {
        fn account(
            &self,
            address: [u8; 20],
        ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
            self.state.account(address)
        }
        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            if address == DPOS_CONTRACT_ADDRESS && *key == self.key {
                return match self.mode {
                    0 => Err(FinalChainNativeStateReadError::State(
                        ConcreteReadError::HistoryUnavailable(ConcreteStateIdentity {
                            period: 0.into(),
                            state_root: [0; 32],
                        }),
                    )),
                    1 => Ok(ConcreteRead::Present(vec![0xff])),
                    _ => Ok(ConcreteRead::Absent),
                };
            }
            self.state.raw_storage(address, key)
        }
    }

    #[test]
    fn pre_magnolia_terminal_rewards_keep_nonzero_mint_no_pbft_credit_and_fault_isolation() {
        with_reward_chain_setup_for_magnolia(
            20,
            FinalChainBlockNumber::MAX,
            0,
            0,
            1,
            10.into(),
            |chain| {
                let reward_plan = plan(vec![distribution_rlp(1, VALIDATOR_ONE, VALIDATOR_ONE, 0)]);
                let mut probe = chain
                    .begin_native_session_bound(reward_plan.request_id, 1.into(), 0.into())
                    .unwrap();
                let before = probe.dpos_state.clone();
                assert!(before.has_complete_validator_shapes());
                let state = RewardState::from_snapshot(&before);
                let successful = probe.finish_rewards(&reward_plan, &state).unwrap();
                assert!(!successful.total_reward.is_zero());
                assert!(!before.commission_rewards.contains_key(&VALIDATOR_ONE));
                assert!(
                    successful
                        .dpos_snapshot
                        .commission_rewards
                        .get(&VALIDATOR_ONE)
                        .unwrap()
                        .is_zero()
                );
                assert_eq!(
                    successful.dpos_snapshot.commission_rewards[&VALIDATOR_ONE].persisted_len(),
                    0
                );
                assert_eq!(
                    successful.dpos_snapshot.validator_row_authority,
                    before.validator_row_authority
                );
                assert!(
                    successful
                        .account_mutations
                        .iter()
                        .all(|mutation| match mutation {
                            FinalChainNativeOrdinaryMutation::BalanceReplace {
                                address, ..
                            }
                            | FinalChainNativeOrdinaryMutation::Touch { address, .. }
                            | FinalChainNativeOrdinaryMutation::EnsureExists { address, .. }
                            | FinalChainNativeOrdinaryMutation::NonceReplace { address, .. } =>
                                *address == DPOS_CONTRACT_ADDRESS,
                        })
                );
                assert_eq!(
                    probe.finish_rewards(&reward_plan, &state),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                );
                let key = ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &VALIDATOR_ONE]));
                assert!(
                    state
                        .raw_reads
                        .borrow()
                        .iter()
                        .any(|((address, k), _)| *address == DPOS_CONTRACT_ADDRESS && *k == key.0)
                );
                for mode in 0..3 {
                    let mut session = chain
                        .begin_native_session_bound(reward_plan.request_id, 1.into(), 0.into())
                        .unwrap();
                    let state = RewardState::from_snapshot(&before);
                    let rows = state.rows.borrow().clone();
                    let balance = state.dpos_balance.borrow().clone();
                    let fault = PreMagnoliaRewardFault {
                        state: &state,
                        key,
                        mode,
                    };
                    let error = session.finish_rewards(&reward_plan, &fault).unwrap_err();
                    assert!(
                        if mode == 0 {
                            matches!(error, FinalChainNativeSessionError::StateRead(_))
                        } else {
                            matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                        },
                        "{error:?}"
                    );
                    assert!(session.aborted);
                    assert_eq!(session.dpos_state, before);
                    assert_eq!(*state.rows.borrow(), rows);
                    assert_eq!(*state.dpos_balance.borrow(), balance);
                    assert_eq!(
                        session.finish_rewards(&reward_plan, &state),
                        Err(FinalChainNativeSessionError::Aborted)
                    );
                    let mut retry = chain
                        .begin_native_session_bound(reward_plan.request_id, 1.into(), 0.into())
                        .unwrap();
                    assert_eq!(
                        retry.finish_rewards(&reward_plan, &state).unwrap(),
                        successful
                    );
                }
                let mut incomplete = chain
                    .begin_native_session_bound(reward_plan.request_id, 1.into(), 0.into())
                    .unwrap();
                incomplete.dpos_state.validator_row_authority =
                    ValidatorRowAuthority::CompatibilityOnly;
                assert_eq!(
                    incomplete.finish_rewards(&reward_plan, &state),
                    Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
                );
                let bad_plan = plan(vec![distribution_rlp(1, VALIDATOR_ONE, VALIDATOR_ONE, 11)]);
                let mut bad = chain
                    .begin_native_session_bound(bad_plan.request_id, 1.into(), 0.into())
                    .unwrap();
                assert!(matches!(
                    bad.finish_rewards(&bad_plan, &state),
                    Err(FinalChainNativeSessionError::Domain(_))
                ));
                assert!(bad.aborted);
                assert_eq!(bad.dpos_state, before);
            },
        );
    }

    fn mutation_value(mutation: &FinalChainNativeRawMutation) -> Vec<u8> {
        match &mutation.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
            FinalChainNativeRawOperation::Delete => Vec::new(),
        }
    }

    fn delegate_request() -> FinalChainNativeRequest {
        let mut input = DPOS_DELEGATE_SELECTOR.to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&VALIDATOR_ONE);
        FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: FinalChainTransactionPosition::new(0),
                sequence: 0,
            },
            period: 1.into(),
            depth: 0,
            kind: FinalChainNativeCallKind::Call,
            is_static: false,
            caller: ZERO_YIELD_DELEGATOR,
            contract: DPOS_CONTRACT_ADDRESS,
            state_address: DPOS_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::new(BigUint::from(100_u64)),
            input,
            supplied_gas: DPOS_DELEGATE_GAS.into(),
        }
    }

    fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
        bytes
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn hex_decode(value: &str) -> Vec<u8> {
        let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
        assert!(remainder.is_empty());
        pairs
            .iter()
            .map(|chunk| {
                u8::from_str_radix(std::str::from_utf8(chunk).unwrap(), 16)
                    .expect("fixture contains hexadecimal bytes")
            })
            .collect()
    }

    fn fixed_hex<const N: usize>(value: &str) -> [u8; N] {
        hex_decode(value)
            .try_into()
            .unwrap_or_else(|_| panic!("fixture hexadecimal value is not {N} bytes"))
    }

    #[test]
    fn aspen_two_ordered_distributions_match_both_pinned_go_references() {
        let public: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let local: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_local.json"
        )))
        .unwrap();
        assert_eq!(public, local);

        with_current_reward_chain(|chain| {
            let reward_plan = plan(vec![
                distribution_rlp(1, VALIDATOR_ONE, VALIDATOR_ONE, 11),
                distribution_rlp(1, VALIDATOR_TWO, VALIDATOR_TWO, 17),
            ]);
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();

            assert_eq!(
                outcome.total_reward.as_u256().to_string(),
                public["total_minted"].as_str().unwrap()
            );
            let mut expected_writes = public["distributions"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|distribution| {
                    distribution["ordered_raw_writes"]
                        .as_array()
                        .unwrap()
                        .iter()
                })
                .collect::<Vec<_>>();
            expected_writes.extend(public["end_block_ordered_raw_writes"].as_array().unwrap());
            assert_eq!(outcome.raw_mutations.len(), expected_writes.len());
            for (actual, expected) in outcome.raw_mutations.iter().zip(expected_writes) {
                assert_eq!(hex_bytes(actual.address), expected["address"]);
                assert_eq!(hex_bytes(actual.key.0), expected["key"]);
                assert_eq!(hex_bytes(mutation_value(actual)), expected["value"]);
            }
            assert_eq!(
                outcome
                    .raw_mutations
                    .iter()
                    .map(|mutation| mutation.expected.clone())
                    .collect::<Vec<_>>(),
                vec![
                    ConcreteRead::Absent,
                    ConcreteRead::Absent,
                    ConcreteRead::Absent,
                    ConcreteRead::Present(vec![0xc2, 0x80, 0x80]),
                    ConcreteRead::Present(vec![0xc2, 0x06, 0x80]),
                    ConcreteRead::Present(vec![0x13, 0x88]),
                    ConcreteRead::Present(vec![0x83, 0x03, 0x0d, 0x40]),
                    ConcreteRead::Present(vec![0xc2, 0x80, 0x80]),
                    ConcreteRead::Present(vec![0xc2, 0x04, 0x01]),
                    ConcreteRead::Present(vec![0x13, 0xc4]),
                ]
            );
            assert_eq!(state.account_reads.get(), 1);
            let balances = outcome
                .account_mutations
                .iter()
                .map(|mutation| match mutation {
                    FinalChainNativeOrdinaryMutation::BalanceReplace { replacement, .. } => {
                        replacement.clone()
                    }
                    mutation => panic!("unexpected Aspen reward account mutation: {mutation:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(
                balances,
                vec![3_011_u64, 3_071, 3_088, 3_143]
                    .into_iter()
                    .map(BigInt::from)
                    .collect::<Vec<_>>()
            );
            let AspenSupplyState::Migrated {
                total_supply,
                current_yield,
            } = outcome.dpos_snapshot.aspen_supply_state
            else {
                panic!("Aspen reward phase must migrate supply")
            };
            assert_eq!(total_supply.as_u256(), U256::from(5_115));
            assert_eq!(current_yield.as_u64(), 185_770);
            assert_eq!(
                outcome.dpos_snapshot.delegator_rewards[&VALIDATOR_ONE].as_u256(),
                U256::from(60)
            );
            assert_eq!(
                outcome.dpos_snapshot.commission_rewards[&VALIDATOR_ONE].as_u256(),
                U256::from(11)
            );
            assert_eq!(
                outcome.dpos_snapshot.delegator_rewards[&VALIDATOR_TWO].as_u256(),
                U256::from(42)
            );
            assert_eq!(
                outcome.dpos_snapshot.commission_rewards[&VALIDATOR_TWO].as_u256(),
                U256::from(30)
            );
        });
    }

    #[test]
    fn committed_state_reopen_resets_go_cleanup_scheduler() {
        let public: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let local: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_local.json"
        )))
        .unwrap();
        assert_eq!(public, local);

        let witness = &public["jailed_validator_cleanup"]["reopened_scheduler"];
        assert_eq!(
            witness["period_2_live_cleanup"]["ordered_raw_writes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            witness["period_3_after_reopen"]["ordered_raw_writes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            witness["period_4_live_cleanup"]["ordered_raw_writes"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            witness["period_2_live_cleanup"]["after"]["descriptor"]["root"],
            witness["period_3_after_reopen"]["after"]["descriptor"]["root"]
        );
    }

    #[test]
    fn multi_validator_map_uses_one_observed_go_order_and_same_final_state() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let permutations = &fixture["multi_validator_permutations"];
        assert_eq!(permutations["same_final_logical_state_and_root"], true);
        let variants = permutations["observed_variants"].as_array().unwrap();
        assert_eq!(variants.len(), 2);
        assert_ne!(
            variants[0]["validator_iteration_order"],
            variants[1]["validator_iteration_order"]
        );
        assert_eq!(
            variants[0]["after"]["descriptor"]["root"],
            variants[1]["after"]["descriptor"]["root"]
        );
        assert_eq!(
            variants[0]["after"]["dpos_account"]["balance"],
            variants[1]["after"]["dpos_account"]["balance"]
        );
        assert_eq!(permutations["physical_rows_equal"], true);
        assert!(
            permutations["physical_row_variance"]
                .as_array()
                .unwrap()
                .iter()
                .all(|column| column["only_left"].as_array().unwrap().is_empty()
                    && column["only_right"].as_array().unwrap().is_empty()
                    && column["different_values"].as_array().unwrap().is_empty())
        );

        with_current_reward_chain(|chain| {
            let reward_plan = plan(vec![distributions_rlp(
                1,
                MISSING_AUTHOR,
                &[(VALIDATOR_ONE, 11), (VALIDATOR_TWO, 17)],
                2,
                20,
            )]);
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();

            let rust_order = variants
                .iter()
                .find(|variant| {
                    variant["validator_iteration_order"]
                        == serde_json::json!([hex_bytes(VALIDATOR_ONE), hex_bytes(VALIDATOR_TWO)])
                })
                .expect("Go runs observed Rust address order");
            let expected_writes = rust_order["ordered_raw_writes"].as_array().unwrap();
            assert_eq!(outcome.raw_mutations.len(), expected_writes.len());
            for (actual, expected) in outcome.raw_mutations.iter().zip(expected_writes) {
                assert_eq!(hex_bytes(actual.address), expected["address"]);
                assert_eq!(hex_bytes(actual.key.0), expected["key"]);
                assert_eq!(hex_bytes(mutation_value(actual)), expected["value"]);
            }
            assert_eq!(
                outcome.total_reward.as_u256().to_string(),
                rust_order["total_minted"]
            );
            let final_balances = outcome
                .account_mutations
                .iter()
                .map(|mutation| match mutation {
                    FinalChainNativeOrdinaryMutation::BalanceReplace { replacement, .. } => {
                        replacement.clone()
                    }
                    mutation => panic!("unexpected multi-validator mutation: {mutation:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(
                final_balances,
                [3_011_u64, 3_028, 3_082]
                    .into_iter()
                    .map(BigInt::from)
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                final_balances.last().unwrap().to_string(),
                rust_order["after"]["dpos_account"]["balance"]
            );
            let AspenSupplyState::Migrated {
                total_supply,
                current_yield,
            } = &outcome.dpos_snapshot.aspen_supply_state
            else {
                panic!("multi-validator Aspen rewards migrate supply")
            };
            assert_eq!(total_supply.as_u256(), U256::from(5_054));
            assert_eq!(current_yield.as_u64(), 200_000);
            assert_eq!(
                outcome.dpos_snapshot.delegator_rewards[&VALIDATOR_ONE].as_u256(),
                U256::from(27)
            );
            assert_eq!(
                outcome.dpos_snapshot.commission_rewards[&VALIDATOR_ONE].as_u256(),
                U256::from(11)
            );
            assert_eq!(
                outcome.dpos_snapshot.delegator_rewards[&VALIDATOR_TWO].as_u256(),
                U256::from(21)
            );
            assert_eq!(
                outcome.dpos_snapshot.commission_rewards[&VALIDATOR_TWO].as_u256(),
                U256::from(23)
            );

            let canonical =
                canonical_concrete_precompile_storage(&outcome.dpos_snapshot, true).unwrap();
            for slot in rust_order["after"]["slots"].as_array().unwrap() {
                let key = fixed_hex::<32>(slot["key"].as_str().unwrap());
                let expected = hex_decode(slot["value"].as_str().unwrap());
                let actual = canonical
                    .get(&(DPOS_CONTRACT_ADDRESS, key))
                    .and_then(|values| values.first())
                    .cloned()
                    .unwrap_or_default();
                assert_eq!(actual, expected, "{}", slot["name"]);
            }
        });
    }

    fn hex_bytes_with_prefix(bytes: [u8; 20]) -> String {
        format!("0x{}", hex_bytes(bytes))
    }

    // Exact input contract for this bounded fixture, independent of observed effects.
    // Compare complete JSON values: missing/extra keys, types and array lengths
    // must match before any owner state or lifecycle is constructed.
    const NONBOUNDARY_INPUT_CONTRACT: &str = r#"{
  "schema": 1,
  "synthetic": true,
  "period": 1,
  "parent_period": 0,
  "parent_root_label": "0x1234",
  "block_author": "0x0000000000000000000000000000000000000051",
  "timestamp": 0,
  "gas_limit": 1000000,
  "config": {
    "EVMChainConfig": {
      "chainId": 1
    },
    "GenesisBalances": {
      "0x0000000000000000000000000000000000000032": 2000,
      "0x0000000000000000000000000000000000000042": 3000
    },
    "DPOS": {
      "EligibilityBalanceThreshold": 100,
      "VoteEligibilityBalanceStep": 10,
      "ValidatorMaximumStake": 1000000,
      "MinimumDeposit": 1,
      "MaxBlockAuthorReward": 10,
      "DagProposersReward": 50,
      "CommissionChangeDelta": 0,
      "CommissionChangeFrequency": 0,
      "DelegationDelay": 0,
      "DelegationLockingPeriod": 0,
      "BlocksPerYear": 10,
      "YieldPercentage": 1,
      "InitialValidators": [
        {
          "Address": "0x0000000000000000000000000000000000000031",
          "Owner": "0x0000000000000000000000000000000000000032",
          "VrfKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          "Commission": 100,
          "Endpoint": "",
          "Description": "",
          "Delegations": {
            "0x0000000000000000000000000000000000000032": 1000
          }
        },
        {
          "Address": "0x0000000000000000000000000000000000000041",
          "Owner": "0x0000000000000000000000000000000000000042",
          "VrfKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
          "Commission": 2500,
          "Endpoint": "",
          "Description": "",
          "Delegations": {
            "0x0000000000000000000000000000000000000042": 2000
          }
        }
      ]
    },
    "Hardforks": {
      "FixRedelegateBlockNum": 18446744073709551615,
      "Redelegations": null,
      "RewardsDistributionFrequency": {
        "0": 2
      },
      "MagnoliaHf": {
        "BlockNum": 0,
        "JailTime": 0
      },
      "PhalaenopsisHfBlockNum": 0,
      "FixClaimAllBlockNum": 0,
      "AspenHf": {
        "BlockNumPartOne": 0,
        "BlockNumPartTwo": 1,
        "MaxSupply": 6000,
        "GeneratedRewards": 0
      },
      "FicusHf": {
        "BlockNum": 0,
        "PillarBlocksInterval": 0,
        "BridgeContractAddress": "0x0000000000000000000000000000000000000000"
      },
      "CornusHf": {
        "BlockNum": 0,
        "DelegationLockingPeriod": 0,
        "DagGasLimit": 0,
        "PbftGasLimit": 0
      },
      "SoleiroliaHf": {
        "BlockNum": 18446744073709551615,
        "TrxMinGasPrice": 0,
        "TrxMaxGasLimit": 0
      },
      "CactiHf": {
        "BlockNum": 18446744073709551615,
        "LambdaMin": 0,
        "LambdaMax": 0,
        "LambdaDefault": 0,
        "LambdaChangeInterval": 0,
        "LambdaChange": 0,
        "BlockPropagationMin": 0,
        "BlockPropagationMax": 0,
        "ConsensusDelay": 0,
        "DelegationLockingPeriod": 0,
        "JailTime": 0
      }
    }
  },
  "rewards_fact": {
    "period": 1,
    "block_author": "0x0000000000000000000000000000000000000051",
    "blocks_per_year": 10,
    "caller_eligible_vote_count": 0,
    "planner_eligible_vote_count": 10,
    "transactions": [],
    "dag_blocks": [],
    "cert_votes": []
  },
  "rust_genesis_accounts_after_delegation": {
    "0000000000000000000000000000000000000032": 1000,
    "0000000000000000000000000000000000000042": 1000
  },
  "go_setup": "ApplyGenesis, then pinned genesis Cornus DPoS bytecode and op_stack.OpPrecompiles through value-only account/raw collector; no trie/root",
  "synthetic_root_authenticated": false,
  "producer_qualified": false,
  "real_window_gate_closed": false
}"#;

    fn validate_nonboundary_input(fixture: &Value) -> Result<(), &'static str> {
        let expected: Value = serde_json::from_str(NONBOUNDARY_INPUT_CONTRACT).unwrap();
        if fixture != &expected {
            return Err("synthetic input contract drift");
        }
        Ok(())
    }

    fn nonboundary_input() -> Value {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/synthetic_native_transition_input.json"
        )))
        .unwrap();
        validate_nonboundary_input(&fixture).unwrap();
        fixture
    }

    #[test]
    fn nonboundary_manifest_rejects_drift_at_every_field() {
        fn visit(root: &Value, node: &Value, path: &str, cases: &mut usize) {
            let mut changed = root.clone();
            *changed.pointer_mut(path).unwrap() = if node.is_null() {
                serde_json::json!(false)
            } else {
                Value::Null
            };
            assert!(
                validate_nonboundary_input(&changed).is_err(),
                "changed {path}"
            );
            *cases += 1;
            match node {
                Value::Object(fields) => {
                    let mut unknown = root.clone();
                    unknown
                        .pointer_mut(path)
                        .unwrap()
                        .as_object_mut()
                        .unwrap()
                        .insert("unknown_input".into(), serde_json::json!(0));
                    assert!(
                        validate_nonboundary_input(&unknown).is_err(),
                        "unknown {path}"
                    );
                    *cases += 1;
                    for (key, value) in fields {
                        let mut missing = root.clone();
                        missing
                            .pointer_mut(path)
                            .unwrap()
                            .as_object_mut()
                            .unwrap()
                            .remove(key);
                        assert!(
                            validate_nonboundary_input(&missing).is_err(),
                            "missing {path}/{key}"
                        );
                        *cases += 1;
                        let escaped = key.replace('~', "~0").replace('/', "~1");
                        visit(root, value, &format!("{path}/{escaped}"), cases);
                    }
                }
                Value::Array(items) => {
                    let mut extra = root.clone();
                    extra
                        .pointer_mut(path)
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .push(Value::Null);
                    assert!(validate_nonboundary_input(&extra).is_err(), "extra {path}");
                    *cases += 1;
                    for (index, item) in items.iter().enumerate() {
                        visit(root, item, &format!("{path}/{index}"), cases);
                    }
                }
                _ => {
                    let replacement = match node {
                        Value::Bool(value) => serde_json::json!(!value),
                        Value::Number(value) => {
                            serde_json::json!(value.as_u64().unwrap().wrapping_add(1))
                        }
                        Value::String(value) => serde_json::json!(format!("{value}drift")),
                        Value::Null => serde_json::json!(0),
                        _ => unreachable!(),
                    };
                    let mut changed = root.clone();
                    *changed.pointer_mut(path).unwrap() = replacement;
                    assert!(
                        validate_nonboundary_input(&changed).is_err(),
                        "value {path}"
                    );
                    *cases += 1;
                }
            }
        }
        let fixture = nonboundary_input();
        let mut cases = 0;
        visit(&fixture, &fixture, "", &mut cases);
        println!("nonboundary manifest drift controls: {cases}");
    }

    fn with_nonboundary_reward_chain(test: impl FnOnce(&FinalChain)) {
        let fixture = nonboundary_input();
        assert_eq!(
            fixture["config"]["Hardforks"]["RewardsDistributionFrequency"]["0"],
            2
        );
        assert_eq!(
            fixture["config"]["GenesisBalances"],
            serde_json::json!({hex_bytes_with_prefix(DELEGATOR_ONE):2000, hex_bytes_with_prefix(DELEGATOR_TWO):3000})
        );
        let validators = fixture["config"]["DPOS"]["InitialValidators"]
            .as_array()
            .unwrap();
        for (row, (address, owner, stake, commission)) in validators.iter().zip([
            (VALIDATOR_ONE, DELEGATOR_ONE, 1000, 100),
            (VALIDATOR_TWO, DELEGATOR_TWO, 2000, 2500),
        ]) {
            assert_eq!(row["Address"], hex_bytes_with_prefix(address));
            assert_eq!(row["Owner"], hex_bytes_with_prefix(owner));
            assert_eq!(row["Commission"], commission);
            assert_eq!(row["Delegations"][hex_bytes_with_prefix(owner)], stake);
        }
        assert_eq!(validators.len(), 2);
        assert_eq!(fixture["rewards_fact"]["planner_eligible_vote_count"], 10);
        for (field, value) in [
            ("EligibilityBalanceThreshold", 100),
            ("VoteEligibilityBalanceStep", 10),
            ("ValidatorMaximumStake", 1_000_000),
            ("MinimumDeposit", 1),
            ("MaxBlockAuthorReward", 10),
            ("DagProposersReward", 50),
            ("BlocksPerYear", 10),
            ("YieldPercentage", 1),
            ("DelegationDelay", 0),
            ("DelegationLockingPeriod", 0),
        ] {
            assert_eq!(fixture["config"]["DPOS"][field], value);
        }
        let forks = &fixture["config"]["Hardforks"];
        assert_eq!(forks["FixRedelegateBlockNum"], u64::MAX);
        for field in ["MagnoliaHf", "FicusHf", "CornusHf"] {
            assert_eq!(forks[field]["BlockNum"], 0);
        }
        for field in ["CactiHf", "SoleiroliaHf"] {
            assert_eq!(forks[field]["BlockNum"], u64::MAX);
        }
        assert_eq!(
            forks["AspenHf"],
            serde_json::json!({"BlockNumPartOne":0,"BlockNumPartTwo":1,"MaxSupply":6000,"GeneratedRewards":0})
        );
        with_reward_chain_setup(1, FinalChainBlockNumber::MAX, 0, 0, 2, test);
    }

    fn nonboundary_plan(chain: &FinalChain) -> FinalChainPreparedExternalEvmRewardsStatsPlan {
        let _ = nonboundary_input();
        let plan = chain
            .plan_external_evm_rewards_stats(
                [0x74; 32],
                FinalizedRewardsPeriodFact {
                    period: 1,
                    block_author: H160::from(MISSING_AUTHOR),
                    blocks_per_year: 10,
                    dpos_eligible_total_vote_count: 0,
                    transactions: Vec::new(),
                    dag_blocks: Vec::new(),
                    cert_votes: Vec::new(),
                },
            )
            .unwrap();
        assert!(plan.distribution_stats.is_empty());
        assert!(plan.storage_update.cache_current_period);
        assert!(!plan.storage_update.clear_cached_stats);
        assert!(!plan.storage_update.current_block_stats_rlp.is_empty());
        plan
    }

    #[test]
    fn owned_epoch_session_matches_borrowed_and_retains_owner() {
        with_owned_reward_chain_setup(
            1,
            FinalChainBlockNumber::MAX,
            0,
            0,
            2,
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let epoch = 73;
                let parent = FinalChainBlockNumber::GENESIS;
                chain
                    .authorize_reward_scheduler_joint_startup(epoch, parent)
                    .unwrap();
                chain.complete_reward_scheduler_recovery(epoch).unwrap();
                let plan = nonboundary_plan(&chain);
                let mut borrowed = chain
                    .begin_native_session_bound_at_state_api_epoch(
                        plan.request_id,
                        epoch,
                        1.into(),
                        parent,
                    )
                    .unwrap();
                let mut owned = chain
                    .begin_owned_native_session_bound_at_state_api_epoch(
                        plan.request_id,
                        epoch,
                        1.into(),
                        parent,
                    )
                    .unwrap();
                assert_eq!(borrowed.dpos_state, owned.dpos_state);
                assert_eq!(borrowed.eligibility_state, owned.eligibility_state);
                assert_eq!(borrowed.eligibility_period, owned.eligibility_period);
                assert_eq!(borrowed.next_sequence, owned.next_sequence);
                assert_eq!(
                    borrowed.period_start_total_vote_count,
                    owned.period_start_total_vote_count
                );
                assert_eq!(
                    borrowed.period_start_amount_delegated,
                    owned.period_start_amount_delegated
                );
                let state_a = RewardState::from_snapshot(&borrowed.dpos_state);
                let state_b = RewardState::from_snapshot(&owned.dpos_state);
                let mut wrong_plan = plan.clone();
                wrong_plan.request_id[0] ^= 1;
                assert!(matches!(
                    borrowed.finish_rewards(&wrong_plan, &state_a),
                    Err(FinalChainNativeSessionError::RewardsPlanMismatch)
                ));
                assert!(matches!(
                    owned.finish_rewards(&wrong_plan, &state_b),
                    Err(FinalChainNativeSessionError::RewardsPlanMismatch)
                ));
                let request = delegate_request();
                let qa = borrowed.prepare(&request, &state_a).unwrap();
                let qb = owned.prepare(&request, &state_b).unwrap();
                assert_eq!(qa, qb);
                assert!(matches!(
                    borrowed.finish_rewards(&plan, &state_a),
                    Err(FinalChainNativeSessionError::QuoteOutstanding)
                ));
                assert!(matches!(
                    owned.finish_rewards(&plan, &state_b),
                    Err(FinalChainNativeSessionError::QuoteOutstanding)
                ));
                let a = borrowed.invoke(&request, qa, &state_a).unwrap();
                let b = owned.invoke(&request, qb, &state_b).unwrap();
                assert_eq!(a, b);
                let FinalChainNativeInvocationResult::Completed(outcome) = a else {
                    panic!("delegate gas")
                };
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                for mutation in &outcome.raw_mutations {
                    state_a.apply(mutation);
                    state_b.apply(mutation);
                }
                state_a.set_dpos_balance(3100);
                state_b.set_dpos_balance(3100);
                let ra = borrowed.finish_rewards(&plan, &state_a).unwrap();
                let rb = owned.finish_rewards(&plan, &state_b).unwrap();
                assert_eq!(ra, rb);
                assert_eq!(*state_a.raw_reads.borrow(), *state_b.raw_reads.borrow());
                assert!(matches!(
                    borrowed.finish_rewards(&plan, &state_a),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                ));
                assert!(matches!(
                    owned.finish_rewards(&plan, &state_b),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                ));
                drop(borrowed);
                drop(owned);
                let mut lifetime_session = chain
                    .begin_owned_native_session_bound_at_state_api_epoch(
                        plan.request_id,
                        epoch,
                        1.into(),
                        parent,
                    )
                    .unwrap();
                let lifetime_state = RewardState::from_snapshot(&lifetime_session.dpos_state);
                let weak = Arc::downgrade(&chain);
                drop(chain);
                assert!(weak.upgrade().is_some());
                let quote = lifetime_session.prepare(&request, &lifetime_state).unwrap();
                let result = lifetime_session
                    .invoke(&request, quote, &lifetime_state)
                    .unwrap();
                assert!(matches!(
                    result,
                    FinalChainNativeInvocationResult::Completed(_)
                ));
                drop(lifetime_session);
                assert!(weak.upgrade().is_none());
            },
        );
    }

    #[test]
    fn owned_epoch_session_preserves_constructor_rejections() {
        with_owned_reward_chain_setup(
            1,
            FinalChainBlockNumber::MAX,
            0,
            0,
            2,
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let parent = FinalChainBlockNumber::GENESIS;
                chain
                    .authorize_reward_scheduler_joint_startup(73, parent)
                    .unwrap();
                chain.complete_reward_scheduler_recovery(73).unwrap();
                for (epoch, pending, expected_parent) in [(74, 1, 0), (73, 2, 0), (73, 2, 1)] {
                    let a = chain
                        .begin_native_session_bound_at_state_api_epoch(
                            [0x74; 32],
                            epoch,
                            pending.into(),
                            expected_parent.into(),
                        )
                        .err()
                        .unwrap();
                    let b = chain
                        .begin_owned_native_session_bound_at_state_api_epoch(
                            [0x74; 32],
                            epoch,
                            pending.into(),
                            expected_parent.into(),
                        )
                        .err()
                        .unwrap();
                    assert_eq!(a, b);
                }
                assert_eq!(chain.last_block_number_typed().unwrap(), parent);
            },
        );
    }

    #[test]
    fn owned_epoch_session_abort_isolated_from_fresh_retry() {
        with_owned_reward_chain_setup(
            1,
            FinalChainBlockNumber::MAX,
            0,
            0,
            2,
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let parent = FinalChainBlockNumber::GENESIS;
                chain
                    .authorize_reward_scheduler_joint_startup(73, parent)
                    .unwrap();
                chain.complete_reward_scheduler_recovery(73).unwrap();
                let plan = nonboundary_plan(&chain);
                let mut failed = chain
                    .begin_owned_native_session_bound_at_state_api_epoch(
                        plan.request_id,
                        73,
                        1.into(),
                        parent,
                    )
                    .unwrap();
                let state = RewardState::from_snapshot(&failed.dpos_state);
                state
                    .rows
                    .borrow_mut()
                    .remove(&(DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[4]])));
                assert!(matches!(
                    failed.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::RawIntegrity(_))
                ));
                assert!(matches!(
                    failed.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                ));
                let mut retry = chain
                    .begin_owned_native_session_bound_at_state_api_epoch(
                        plan.request_id,
                        73,
                        1.into(),
                        parent,
                    )
                    .unwrap();
                let fresh = RewardState::from_snapshot(&retry.dpos_state);
                retry.finish_rewards(&plan, &fresh).unwrap();
                assert_eq!(chain.last_block_number_typed().unwrap(), parent);
            },
        );
    }

    #[test]
    fn empty_distribution_stream_is_a_zero_effect_nonboundary_phase() {
        with_nonboundary_reward_chain(|chain| {
            let reward_plan = nonboundary_plan(chain);
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let before = session.dpos_state.clone();
            let state = RewardState::from_snapshot(&before);
            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert_eq!(outcome.total_reward, DposTokenAmount::zero());
            assert!(outcome.account_mutations.is_empty());
            assert!(outcome.raw_mutations.is_empty());
            assert_eq!(outcome.dpos_snapshot, before);
            assert_eq!(state.account_reads.get(), 0);
            let expected = vec![
                (
                    (DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[4]])),
                    ConcreteRead::Present(concrete_compact_u64(300)),
                ),
                (
                    (DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[5]])),
                    ConcreteRead::Present(concrete_u256_bytes(U256::from(3_000))),
                ),
            ];
            assert_eq!(*state.raw_reads.borrow(), expected);
            let go: Value = serde_json::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../experiments/evm_feasibility/fixtures/synthetic_native_transition_go_hardened.json"
            )))
            .unwrap();
            let witness = &go["witness"];
            assert_eq!(witness["nonboundary"], true);
            assert_eq!(witness["ordered_raw_writes"], serde_json::json!([]));
            assert_eq!(go["manifest_binding"]["positive_accepted"], true);
            for counter in [
                "backend_put_attempts",
                "commit_attempts",
                "trie_mutation_attempts",
            ] {
                assert_eq!(witness[counter], 0);
            }
            let canonical = canonical_concrete_precompile_storage(&before, true).unwrap();
            let go_rows = witness["genesis_raw_rows"].as_array().unwrap();
            assert_eq!(go_rows.len(), 27);
            for row in go_rows {
                let address = fixed_hex::<20>(row["address"].as_str().unwrap());
                let key = fixed_hex::<32>(row["key"].as_str().unwrap());
                assert_eq!(
                    canonical[&(address, key)][0],
                    hex_decode(row["value"].as_str().unwrap())
                );
            }
            assert_eq!(canonical.len(), 29);
            // Go leaves the empty DPoS minted-token row and slashing jail-list absent.
            // These two representational rows are not exact raw snapshot parity.
            let rust_only = canonical
                .iter()
                .filter(|((address, key), _)| {
                    !go_rows.iter().any(|row| {
                        row["address"] == hex_bytes(address) && row["key"] == hex_bytes(key)
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(rust_only.len(), 2);
            assert!(
                rust_only
                    .iter()
                    .all(|((address, _), values)| (*address == DPOS_CONTRACT_ADDRESS
                        && values[0].is_empty())
                        || (*address == SLASHING_CONTRACT_ADDRESS && values[0] == vec![0xc0]))
            );
            // Optional evidence export is derived from actual reads and effects.
            // It never supplies planner or executor inputs.
            if let Ok(path) = std::env::var("RUSTAXA_SYNTHETIC_REWARDS_EVIDENCE") {
                let rows = canonical_concrete_precompile_storage(&before, true).unwrap();
                let report = serde_json::json!({
                    "synthetic": true, "period": 1, "distribution_frequency": 2,
                    "cache_current_period": reward_plan.storage_update.cache_current_period,
                    "distribution_count": reward_plan.distribution_stats.len(),
                    "reads": state.raw_reads.borrow().iter().map(|((address,key),value)| {
                        let ConcreteRead::Present(value) = value else { panic!("origin absent") };
                        serde_json::json!({"address":hex_bytes(address),"key":hex_bytes(key),"value":hex_bytes(value)})
                    }).collect::<Vec<_>>(),
                    "genesis_raw_rows": rows.into_iter().map(|((address,key),values)| serde_json::json!({"address":hex_bytes(address),"key":hex_bytes(key),"value":hex_bytes(&values[0])})).collect::<Vec<_>>(),
                    "account_mutations": outcome.account_mutations.len(), "raw_mutations": outcome.raw_mutations.len(),
                    "total_reward": outcome.total_reward.as_u256().to_string(), "snapshot_unchanged": outcome.dpos_snapshot == before,
                    "account_reads": state.account_reads.get(),
                });
                std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
            }
        });
    }

    #[test]
    fn nonboundary_rewards_reject_missing_and_malformed_origins() {
        with_nonboundary_reward_chain(|chain| {
            let plan = nonboundary_plan(chain);
            for suffix in [4_u8, 5] {
                for malformed in [false, true] {
                    let mut session = chain
                        .begin_native_session_bound(
                            plan.request_id,
                            1.into(),
                            FinalChainBlockNumber::GENESIS,
                        )
                        .unwrap();
                    let before = session.dpos_state.clone();
                    let state = RewardState::from_snapshot(&before);
                    let identity = (DPOS_CONTRACT_ADDRESS, concrete_storage_key(&[&[suffix]]));
                    if malformed {
                        state
                            .rows
                            .borrow_mut()
                            .insert(identity, ConcreteRead::Present(vec![0xff]));
                    } else {
                        state.rows.borrow_mut().remove(&identity);
                    }
                    assert!(matches!(
                        session.finish_rewards(&plan, &state),
                        Err(FinalChainNativeSessionError::RawIntegrity(_))
                    ));
                    assert_eq!(session.dpos_state, before);
                    assert_eq!(state.account_reads.get(), 0);
                    assert_eq!(
                        session.finish_rewards(&plan, &state),
                        Err(FinalChainNativeSessionError::Aborted)
                    );
                }
            }
        });
    }

    #[test]
    fn nonboundary_rewards_reject_request_period_and_authority_before_reads() {
        with_nonboundary_reward_chain(|chain| {
            let plan = nonboundary_plan(chain);
            assert!(matches!(
                chain.begin_native_session_bound(
                    plan.request_id,
                    2.into(),
                    FinalChainBlockNumber::GENESIS
                ),
                Err(FinalChainNativeSessionError::PendingPeriodMismatch)
            ));
            {
                let request = [0x75; 32];
                let period = 1_u64;
                let mut session = chain
                    .begin_native_session_bound(
                        request,
                        period.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                let before = session.dpos_state.clone();
                let state = RewardState::from_snapshot(&before);
                assert_eq!(
                    session.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::RewardsPlanMismatch)
                );
                assert_eq!(session.dpos_state, before);
                assert!(state.raw_reads.borrow().is_empty());
                assert_eq!(state.account_reads.get(), 0);
            }
            let mut session = chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            assert_eq!(
                session.finish_rewards(&plan, &state),
                Err(FinalChainNativeSessionError::UnboundRewards)
            );
            assert!(state.raw_reads.borrow().is_empty());
            let mut session = chain
                .begin_native_session_bound(
                    plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            chain.rewards_stats_runtime.lock().unwrap().generation += 1;
            assert!(
                matches!(session.finish_rewards(&plan, &state), Err(FinalChainNativeSessionError::Domain(message)) if message.contains("RUNTIME_GENERATION_MISMATCH"))
            );
            assert!(state.raw_reads.borrow().is_empty());
            assert_eq!(state.account_reads.get(), 0);
        });
    }

    #[test]
    fn zero_yield_skips_distribution_but_flushes_deferred_end_block_rows() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let fixture = &fixture["zero_yield_end_block"];
        assert!(fixture["distribution_return"].is_null());
        assert!(
            fixture["distribution_ordered_raw_writes"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        with_reward_chain(0, |chain| {
            let reward_plan = plan(vec![distribution_rlp(1, VALIDATOR_ONE, VALIDATOR_ONE, 11)]);
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let delegate = delegate_request();
            let quote = session.prepare(&delegate, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(delegate_outcome) =
                session.invoke(&delegate, quote, &state).unwrap()
            else {
                panic!("delegate fixture has sufficient gas")
            };
            assert_eq!(delegate_outcome.status, FinalChainNativeStatus::Success);
            for mutation in &delegate_outcome.raw_mutations {
                state.apply(mutation);
            }
            // The surrounding ordinary EVM frame has already transferred the
            // delegate value before terminal rewards execute.
            state.set_dpos_balance(3_100);

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert_eq!(outcome.total_reward, DposTokenAmount::zero());
            assert!(outcome.account_mutations.is_empty());
            assert_eq!(outcome.raw_mutations.len(), 2);
            assert!(matches!(
                outcome.boundaries.last().unwrap().phase,
                FinalChainNativeRewardsPhase::SlashingCleanup
            ));
            let mut raw = 0;
            let mut ordinary = 0;
            for boundary in &outcome.boundaries {
                assert_eq!(boundary.raw_start, raw);
                assert_eq!(boundary.ordinary_start, ordinary);
                assert!(boundary.raw_end >= raw && boundary.ordinary_end >= ordinary);
                raw = boundary.raw_end;
                ordinary = boundary.ordinary_end;
            }
            assert_eq!(raw, outcome.raw_mutations.len());
            assert_eq!(ordinary, outcome.account_mutations.len());
            let expected = fixture["end_block_ordered_raw_writes"].as_array().unwrap();
            for (actual, expected) in outcome.raw_mutations.iter().zip(expected) {
                assert_eq!(hex_bytes(actual.address), expected["address"]);
                assert_eq!(hex_bytes(actual.key.0), expected["key"]);
                assert_eq!(hex_bytes(mutation_value(actual)), expected["value"]);
            }
            assert_eq!(
                outcome
                    .raw_mutations
                    .iter()
                    .map(|mutation| mutation.expected.clone())
                    .collect::<Vec<_>>(),
                vec![
                    ConcreteRead::Present(vec![0x01, 0x2c]),
                    ConcreteRead::Present(vec![0x0b, 0xb8]),
                ]
            );
            assert_eq!(
                outcome.dpos_snapshot.total_stakes[&VALIDATOR_ONE].as_u256(),
                U256::from(1_100)
            );
            assert_eq!(outcome.dpos_snapshot.total_vote_count, 310);
            assert!(matches!(
                outcome.dpos_snapshot.aspen_supply_state,
                AspenSupplyState::Unmigrated { .. }
            ));
        });
    }

    #[test]
    fn expiring_jail_cleanup_follows_deferred_dpos_rows() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let zero_yield = &fixture["zero_yield_end_block"];
        let jail_expiry = fixture["jailed_validator_cleanup"]["periods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|period| period["period"] == 5)
            .unwrap();

        with_reward_chain(0, |chain| {
            seed_committed_jail(chain, 1);
            let reward_plan = plan(vec![distribution_rlp(1, VALIDATOR_ONE, VALIDATOR_ONE, 11)]);
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let delegate = delegate_request();
            let quote = session.prepare(&delegate, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(delegate_outcome) =
                session.invoke(&delegate, quote, &state).unwrap()
            else {
                panic!("delegate fixture has sufficient gas")
            };
            for mutation in &delegate_outcome.raw_mutations {
                state.apply(mutation);
            }
            state.set_dpos_balance(3_100);

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert_eq!(outcome.raw_mutations.len(), 3);
            let expected_dpos = zero_yield["end_block_ordered_raw_writes"]
                .as_array()
                .unwrap();
            for (actual, expected) in outcome.raw_mutations[..2].iter().zip(expected_dpos) {
                assert_eq!(hex_bytes(actual.address), expected["address"]);
                assert_eq!(hex_bytes(actual.key.0), expected["key"]);
                assert_eq!(hex_bytes(mutation_value(actual)), expected["value"]);
            }
            let expected_slashing = &jail_expiry["ordered_raw_writes"][0];
            let actual_slashing = &outcome.raw_mutations[2];
            assert_eq!(
                hex_bytes(actual_slashing.address),
                expected_slashing["address"]
            );
            assert_eq!(hex_bytes(actual_slashing.key.0), expected_slashing["key"]);
            assert_eq!(
                hex_bytes(mutation_value(actual_slashing)),
                expected_slashing["value"]
            );
            assert_eq!(
                actual_slashing.expected,
                ConcreteRead::Present(hex_decode(
                    jail_expiry["before_end_block"][0]["value"]
                        .as_str()
                        .unwrap()
                ))
            );
            assert_eq!(
                outcome.dpos_snapshot.slashing_jailed_validators,
                Vec::<[u8; 20]>::new()
            );
            assert_eq!(
                outcome.dpos_snapshot.slashing_jail_blocks[&ZERO_YIELD_DELEGATOR],
                1
            );
        });
    }

    #[test]
    fn jailed_cleanup_expires_equality_but_preserves_jail_block_row() {
        let fixture: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/current_rewards_public.json"
        )))
        .unwrap();
        let jail_expiry = fixture["jailed_validator_cleanup"]["periods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|period| period["period"] == 5)
            .unwrap();

        with_current_reward_chain(|chain| {
            seed_committed_jail(chain, 1);
            let reward_plan = plan(Vec::new());
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();

            assert_eq!(outcome.raw_mutations.len(), 1);
            let expected = &jail_expiry["ordered_raw_writes"][0];
            let mutation = &outcome.raw_mutations[0];
            assert_eq!(hex_bytes(mutation.address), expected["address"]);
            assert_eq!(hex_bytes(mutation.key.0), expected["key"]);
            assert_eq!(hex_bytes(mutation_value(mutation)), expected["value"]);
            assert_eq!(
                mutation.expected,
                ConcreteRead::Present(hex_decode(
                    jail_expiry["before_end_block"][0]["value"]
                        .as_str()
                        .unwrap()
                ))
            );
            assert!(outcome.dpos_snapshot.slashing_jailed_validators.is_empty());
            assert_eq!(
                outcome.dpos_snapshot.slashing_jail_blocks[&ZERO_YIELD_DELEGATOR],
                1
            );
            let canonical =
                canonical_concrete_precompile_storage(&outcome.dpos_snapshot, true).unwrap();
            let list_slot = jail_expiry["after"]["slots"]
                .as_array()
                .unwrap()
                .iter()
                .find(|slot| slot["name"] == "jailed_validators")
                .unwrap();
            let key = fixed_hex::<32>(list_slot["key"].as_str().unwrap());
            let actual = canonical
                .get(&(SLASHING_CONTRACT_ADDRESS, key))
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_default();
            assert_eq!(actual, hex_decode(list_slot["value"].as_str().unwrap()));
        });
    }

    #[test]
    fn newly_staged_slashing_rows_remain_out_of_terminal_scope() {
        with_current_reward_chain(|chain| {
            let reward_plan = plan(Vec::new());
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            session
                .dpos_state
                .slashing_jail_blocks
                .insert(ZERO_YIELD_DELEGATOR, 3);
            session
                .dpos_state
                .slashing_jailed_validators
                .push(ZERO_YIELD_DELEGATOR);
            let state = RewardState::from_snapshot(&session.dpos_state);

            assert!(matches!(
                session.finish_rewards(&reward_plan, &state),
                Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
            ));
            assert!(!session.finished_rewards);
            assert!(!session.aborted);
        });
    }

    #[test]
    fn all_future_committed_jail_list_requires_scheduler_provenance() {
        with_current_reward_chain(|chain| {
            seed_committed_jail(chain, 3);
            let reward_plan = plan(Vec::new());
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);

            assert!(matches!(
                session.finish_rewards(&reward_plan, &state),
                Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
            ));
            assert!(!session.finished_rewards);
            assert!(!session.aborted);
        });
    }

    #[test]
    fn scheduler_bound_all_future_list_runs_first_scan_and_prepares_timer() {
        with_current_reward_chain(|chain| {
            seed_committed_jail(chain, 3);
            let reward_plan = plan(Vec::new());
            chain
                .authorize_reward_scheduler_joint_startup(17, FinalChainBlockNumber::GENESIS)
                .unwrap();
            chain.complete_reward_scheduler_recovery(17).unwrap();
            let mut session = chain
                .begin_native_session_bound_at_state_api_epoch(
                    reward_plan.request_id,
                    17,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert_eq!(outcome.raw_mutations.len(), 1);
            assert_eq!(
                mutation_value(&outcome.raw_mutations[0]),
                encode_slashing_jailed_list(&outcome.dpos_snapshot)
            );
            assert_eq!(
                outcome.dpos_snapshot.slashing_jailed_validators,
                vec![ZERO_YIELD_DELEGATOR]
            );
            assert!(outcome.scheduler_successor.is_some());
        });
    }

    #[test]
    fn decreasing_cacti_jail_duration_requires_scheduler_provenance() {
        with_reward_chain_and_jail_policy(1, FinalChainBlockNumber::GENESIS, 100, 1, |chain| {
            seed_committed_jail(chain, 1);
            let reward_plan = plan(Vec::new());
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);

            assert!(matches!(
                session.finish_rewards(&reward_plan, &state),
                Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
            ));
            assert!(!session.finished_rewards);
            assert!(!session.aborted);
        });
    }

    #[test]
    fn scheduler_bound_decreasing_jail_duration_uses_live_timer_authority() {
        with_reward_chain_and_jail_policy(1, FinalChainBlockNumber::GENESIS, 100, 1, |chain| {
            seed_committed_jail(chain, 1);
            let reward_plan = plan(Vec::new());
            chain
                .authorize_reward_scheduler_joint_startup(17, FinalChainBlockNumber::GENESIS)
                .unwrap();
            chain.complete_reward_scheduler_recovery(17).unwrap();
            let mut session = chain
                .begin_native_session_bound_at_state_api_epoch(
                    reward_plan.request_id,
                    17,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert!(outcome.dpos_snapshot.slashing_jailed_validators.is_empty());
            assert_eq!(outcome.raw_mutations.len(), 1);
            assert!(outcome.scheduler_successor.is_some());
        });
    }

    struct CleanupAccountRead<'a> {
        state: &'a RewardState,
        mode: u8,
    }

    impl FinalChainNativeStateRead for CleanupAccountRead<'_> {
        fn account(
            &self,
            address: [u8; 20],
        ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
            if address != SLASHING_CONTRACT_ADDRESS {
                return self.state.account(address);
            }
            match self.mode {
                0 => Ok(FinalChainNativeAccount {
                    exists: false,
                    nonce: FinalChainNonce::zero(),
                    balance: BigInt::default(),
                }),
                1 => Ok(FinalChainNativeAccount {
                    exists: true,
                    nonce: FinalChainNonce::zero(),
                    balance: BigInt::default(),
                }),
                _ => Err(FinalChainNativeStateReadError::State(
                    ConcreteReadError::IdentityMismatch {
                        expected: ConcreteStateIdentity {
                            period: 1.into(),
                            state_root: [1; 32],
                        },
                        observed: ConcreteStateIdentity {
                            period: 1.into(),
                            state_root: [2; 32],
                        },
                    },
                )),
            }
        }

        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            if self.mode == 3
                && address == SLASHING_CONTRACT_ADDRESS
                && key.0 == slashing_jailed_validators_key()
            {
                return Err(FinalChainNativeStateReadError::State(
                    ConcreteReadError::HistoryUnavailable(ConcreteStateIdentity {
                        period: 1.into(),
                        state_root: [1; 32],
                    }),
                ));
            }
            self.state.raw_storage(address, key)
        }
    }

    #[cfg(feature = "synthetic-fixtures")]
    #[test]
    fn batched_cleanup_keeps_committed_visibility_policy() {
        with_owned_reward_chain_setup(
            1,
            FinalChainBlockNumber::MAX,
            0,
            0,
            1,
            FinalChainBlockNumber::GENESIS,
            |mut chain| {
                Arc::get_mut(&mut chain)
                    .unwrap()
                    .configure_synthetic_batched_lifecycle();
                seed_committed_jail(&chain, 3);
                let committed = chain
                    .dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let visible = chain
                    .reward_cleanup_visible_snapshot(
                        FinalChainBlockNumber::GENESIS,
                        &committed,
                        None,
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    visible.slashing_jailed_validators,
                    committed.slashing_jailed_validators
                );
                assert_eq!(visible.slashing_jail_blocks, committed.slashing_jail_blocks);
                let mut changed = committed.clone();
                changed.slashing_jailed_validators.clear();
                assert!(
                    chain
                        .reward_cleanup_visible_snapshot(
                            FinalChainBlockNumber::GENESIS,
                            &changed,
                            None
                        )
                        .is_err()
                );
            },
        );
    }

    #[test]
    fn due_cleanup_uses_only_account_gated_absence_for_empty_source() {
        with_current_reward_chain(|chain| {
            seed_committed_jail(chain, 3);
            let reward_plan = plan(Vec::new());
            chain
                .authorize_reward_scheduler_joint_startup(17, FinalChainBlockNumber::GENESIS)
                .unwrap();
            chain.complete_reward_scheduler_recovery(17).unwrap();
            for mode in 0..4 {
                let mut session = chain
                    .begin_native_session_bound_at_state_api_epoch(
                        reward_plan.request_id,
                        17,
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                let original = session.dpos_state.slashing_jailed_validators.clone();
                let state = RewardState::from_snapshot(&session.dpos_state);
                state
                    .rows
                    .borrow_mut()
                    .remove(&(SLASHING_CONTRACT_ADDRESS, slashing_jailed_validators_key()));
                let result = session.finish_rewards(
                    &reward_plan,
                    &CleanupAccountRead {
                        state: &state,
                        mode,
                    },
                );
                match mode {
                    0 => {
                        let result = result.unwrap();
                        assert_eq!(result.dpos_snapshot.slashing_jailed_validators, original);
                        assert!(
                            result
                                .raw_mutations
                                .iter()
                                .all(|row| row.address != SLASHING_CONTRACT_ADDRESS)
                        );
                        assert_eq!(
                            result.scheduler_successor,
                            session
                                .reward_scheduler_basis
                                .map(|basis| basis.successor(0))
                        );
                    }
                    1 => assert!(matches!(
                        result,
                        Err(FinalChainNativeSessionError::RawIntegrity(_))
                    )),
                    2 => assert!(matches!(
                        result,
                        Err(FinalChainNativeSessionError::StateRead(
                            FinalChainNativeStateReadError::State(
                                ConcreteReadError::IdentityMismatch { .. }
                            )
                        ))
                    )),
                    _ => assert!(matches!(
                        result,
                        Err(FinalChainNativeSessionError::StateRead(
                            FinalChainNativeStateReadError::State(
                                ConcreteReadError::HistoryUnavailable(_)
                            )
                        ))
                    )),
                }
            }
        });
    }

    #[test]
    fn jailed_cleanup_rejects_missing_or_mismatched_jail_block_row() {
        with_current_reward_chain(|chain| {
            seed_committed_jail(chain, 1);
            let reward_plan = plan(Vec::new());
            let jail_key = concrete_storage_key(&[&[0], &ZERO_YIELD_DELEGATOR]);
            for observed in [ConcreteRead::Absent, ConcreteRead::Present(vec![2])] {
                let mut session = chain
                    .begin_native_session_bound(
                        reward_plan.request_id,
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                let state = RewardState::from_snapshot(&session.dpos_state);
                state
                    .rows
                    .borrow_mut()
                    .insert((SLASHING_CONTRACT_ADDRESS, jail_key), observed);

                assert!(matches!(
                    session.finish_rewards(&reward_plan, &state),
                    Err(FinalChainNativeSessionError::RawIntegrity(message))
                        if message == "slashing validator jail block raw/domain facts disagree"
                ));
                assert!(session.aborted);
            }
        });
    }

    #[test]
    fn jailed_cleanup_treats_authenticated_absent_jail_block_as_zero() {
        with_current_reward_chain(|chain| {
            seed_committed_jail_list_without_block(chain);
            let reward_plan = plan(Vec::new());
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let state = RewardState::from_snapshot(&session.dpos_state);
            let jail_key = ConcreteStorageKey(concrete_storage_key(&[&[0], &ZERO_YIELD_DELEGATOR]));
            assert_eq!(
                state
                    .raw_storage(SLASHING_CONTRACT_ADDRESS, &jail_key)
                    .unwrap(),
                ConcreteRead::Absent
            );

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert_eq!(outcome.raw_mutations.len(), 1);
            assert_eq!(mutation_value(&outcome.raw_mutations[0]), vec![0xc0]);
            assert!(outcome.dpos_snapshot.slashing_jailed_validators.is_empty());
            assert!(outcome.dpos_snapshot.slashing_jail_blocks.is_empty());
        });
    }
}
