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

/// Complete output of one bound session's terminal rewards/end-block phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRewardsOutcome {
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
    /// current adapter accepts Magnolia reward periods and ordered zero-or-more
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
        match result {
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
        }
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

        if self
            .final_chain
            .pre_magnolia_fee_reward_period(self.pending_period)
            || self.pending_period == self.final_chain.rewards_config.fix_redelegate_block_num
        {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        self.validate_committed_slashing_cleanup_scope()?;
        let distributions = decode_rewards_block_distributions(&plan.distribution_stats)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let rewards_enabled = self.final_chain.rewards_config.yield_percentage != 0;

        let mut trace = FinalChainNativeRawTrace::new(state);
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
        };

        let planned = self
            .final_chain
            .plan_minted_rewards(self.pending_period, &distributions, &self.dpos_state)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let reconstruction = if rewards_enabled {
            self.reconstruct_rewards(&distributions, &mut next, &mut trace, &mut accounts)?
        } else {
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
        self.serialize_deferred_end_block(&next, &mut trace)?;
        let (cleanup_ran, scheduler_successor) =
            self.serialize_slashing_cleanup(&mut next, &mut trace)?;

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
    ) -> std::result::Result<DposTokenAmount, FinalChainNativeSessionError> {
        let mut total_reward = DposTokenAmount::zero();
        let mut dynamic_total_supply = snapshot
            .aspen_supply_state
            .total_supply()
            .map(StoredDposTokenAmount::amount);

        for stats in distributions {
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

        for (validator, validator_stats) in &stats.validators_stats {
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
            if snapshot.total_stakes.contains_key(validator) {
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
        }
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
        let current = trace.current(DPOS_CONTRACT_ADDRESS, key)?;
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
        let total_current = trace.current(DPOS_CONTRACT_ADDRESS, total_key)?;
        match &snapshot.aspen_supply_state {
            AspenSupplyState::Unmigrated { minted_tokens } => {
                validate_raw_value(&total_current, &[], "Aspen total supply")?;
                trace.put(
                    DPOS_CONTRACT_ADDRESS,
                    total_key,
                    concrete_u256_bytes(total_supply_before.as_u256()),
                )?;
                let minted_key = ConcreteStorageKey(concrete_storage_key(&[&[6]]));
                let minted_current = trace.current(DPOS_CONTRACT_ADDRESS, minted_key)?;
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
                    &trace.current(DPOS_CONTRACT_ADDRESS, yield_key)?,
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
                &trace.current(DPOS_CONTRACT_ADDRESS, yield_key)?,
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
            &trace.current(DPOS_CONTRACT_ADDRESS, key)?,
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
            &trace.current(DPOS_CONTRACT_ADDRESS, key)?,
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
        validate_raw_value(
            &trace.current(DPOS_CONTRACT_ADDRESS, vote_key)?,
            &concrete_compact_u64(self.period_start_total_vote_count),
            "eligible vote count",
        )?;
        let amount_key = ConcreteStorageKey(concrete_storage_key(&[&[5]]));
        validate_raw_value(
            &trace.current(DPOS_CONTRACT_ADDRESS, amount_key)?,
            &concrete_u256_bytes(self.period_start_amount_delegated),
            "amount delegated",
        )
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
        let scheduled = self.reward_scheduler_basis.map(|basis| {
            basis.plan_cleanup(
                &snapshot.slashing_jailed_validators,
                &snapshot.slashing_jail_blocks,
            )
        });
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
        validate_raw_value(
            &trace.current(SLASHING_CONTRACT_ADDRESS, key)?,
            &before,
            "slashing jailed validators",
        )?;
        let mut expires = false;
        for validator in &snapshot.slashing_jailed_validators {
            let jail_key = ConcreteStorageKey(concrete_storage_key(&[&[0], validator]));
            let jail_block = snapshot.slashing_jail_blocks.get(validator).copied();
            let expected = jail_block.map(concrete_rlp_u64).unwrap_or_default();
            validate_raw_value(
                &trace.current(SLASHING_CONTRACT_ADDRESS, jail_key)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H160;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_types::GenesisValidatorMetadata;
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

    #[derive(Default)]
    struct RewardState {
        rows: RefCell<RewardRows>,
        account_reads: Cell<usize>,
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
            Ok(self
                .rows
                .borrow()
                .get(&(address, key.0))
                .cloned()
                .unwrap_or(ConcreteRead::Absent))
        }
    }

    fn temp_db_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa-current-rewards-{}-{nanos}",
            std::process::id()
        ))
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
        let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
            storage.clone(),
            1_000_000.into(),
            0,
            Vec::new(),
            vec![
                validator(VALIDATOR_ONE, DELEGATOR_ONE, 1_000, 100),
                validator(VALIDATOR_TWO, DELEGATOR_TWO, 2_000, 2_500),
            ],
            GenesisDposConfig {
                eligibility_balance_threshold: U256::from(100).into(),
                vote_eligibility_balance_step: U256::from(10).into(),
                validator_maximum_stake: U256::from(1_000_000).into(),
                minimum_deposit: U256::one().into(),
                ..Default::default()
            },
            FinalChainRewardsConfig {
                committee_size: 10,
                magnolia_period: FinalChainBlockNumber::GENESIS,
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
                rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
                ..Default::default()
            },
            FinalChainBlockNumber::GENESIS,
        )
        .unwrap();
        test(&chain);
        drop(chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
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

    #[test]
    fn empty_distribution_stream_is_a_zero_effect_nonboundary_phase() {
        with_current_reward_chain(|chain| {
            let reward_plan = plan(Vec::new());
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
            let basis = chain
                .reward_scheduler_basis(
                    reward_plan.request_id,
                    17,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            session.reward_scheduler_basis = Some(basis);
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
            let basis = chain
                .reward_scheduler_basis(
                    reward_plan.request_id,
                    17,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            let mut session = chain
                .begin_native_session_bound(
                    reward_plan.request_id,
                    1.into(),
                    FinalChainBlockNumber::GENESIS,
                )
                .unwrap();
            session.reward_scheduler_basis = Some(basis);
            let state = RewardState::from_snapshot(&session.dpos_state);

            let outcome = session.finish_rewards(&reward_plan, &state).unwrap();
            assert!(outcome.dpos_snapshot.slashing_jailed_validators.is_empty());
            assert_eq!(outcome.raw_mutations.len(), 1);
            assert!(outcome.scheduler_successor.is_some());
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
