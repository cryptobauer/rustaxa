//! Terminal staged rewards and end-block execution for the mixed-period profile.
//!
//! The first rewards adapter is deliberately bounded to the declared
//! post-Magnolia, pre-Aspen-part-two workload with one validator and one
//! distribution row. It reuses FinalChain's reward planner as semantic
//! authority, reconstructs the pinned Go helper's intermediate write order,
//! and checks that reconstruction reaches the planner's complete DPoS result.
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
}

impl FinalChainNativeSession<'_> {
    /// Finishes one bound session through rewards and deferred DPoS end-block writes.
    ///
    /// The opaque plan must belong to this exact request and pending period. The
    /// current adapter accepts the M1 profile: one distribution row, one live
    /// validator, Magnolia active, nonzero configured yield, Aspen part two
    /// inactive, no jailed-validator cleanup, and no redelegation correction at
    /// this height. Any state-read, planner, reconstruction, or raw-integrity
    /// error aborts the session and exposes no result. Successful completion
    /// consumes the session phase and cannot be repeated.
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
            || self.final_chain.rewards_config.yield_percentage == 0
            || self.final_chain.aspen_part_two_active(self.pending_period)
            || self.pending_period == self.final_chain.rewards_config.fix_redelegate_block_num
            || !self.dpos_state.slashing_jailed_validators.is_empty()
        {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        let distributions = decode_rewards_block_distributions(&plan.distribution_stats)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if distributions.len() != 1 || self.dpos_state.validator_order.len() != 1 {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        let stats = &distributions[0];
        if stats.validators_stats.len() != 1
            || !stats
                .validators_stats
                .contains_key(&self.dpos_state.validator_order[0])
        {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }

        let mut trace = FinalChainNativeRawTrace::new(state);
        self.validate_deferred_origins(&mut trace)?;
        let mut next = self.dpos_state.clone();
        let mut accounts = StagedDposAccountPort::new([(
            DPOS_CONTRACT_ADDRESS,
            state.account(DPOS_CONTRACT_ADDRESS)?,
        )])
        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;

        let planned = self
            .final_chain
            .plan_minted_rewards(self.pending_period, &distributions, &self.dpos_state)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let reconstruction =
            self.reconstruct_selected_rewards(stats, &mut next, &mut trace, &mut accounts)?;
        if reconstruction != planned.total_minted_reward {
            return Err(FinalChainNativeSessionError::Domain(
                "selected reward reconstruction disagrees with FinalChain minted total".to_owned(),
            ));
        }

        next.aspen_supply_state = planned.supply_after.clone();
        self.serialize_supply(&next, &mut trace)?;
        self.serialize_deferred_end_block(&next, &mut trace)?;

        let mut expected = self.dpos_state.clone();
        let mut expected_rewards = planned.dpos_rewards;
        let mut fees = fee_rewards_from_distribution_stats(&distributions)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        retain_existing_validator_fee_rewards(&mut fees, &expected);
        merge_reward_map(&mut expected_rewards.commission_rewards, &fees)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        self.final_chain
            .apply_dpos_reward_deltas(&mut expected, expected_rewards, planned.supply_after)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
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
        })
    }

    fn reconstruct_selected_rewards(
        &self,
        stats: &RewardsBlockDistribution,
        snapshot: &mut DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
        accounts: &mut StagedDposAccountPort,
    ) -> std::result::Result<DposTokenAmount, FinalChainNativeSessionError> {
        let reward_context = self
            .final_chain
            .minted_block_reward(self.pending_period, stats, snapshot, None)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if reward_context.total_supply_before.is_some() {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        }
        let block_reward = reward_context.block_reward.as_u256();
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

        let (validator, validator_stats) = stats
            .validators_stats
            .first_key_value()
            .ok_or(FinalChainNativeSessionError::RewardsScopeUnsupported)?;
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
                        .ok_or_else(|| domain("validator vote reward multiplication overflow"))?
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

    fn serialize_supply(
        &self,
        snapshot: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        if self.pending_period < self.final_chain.rewards_config.aspen_part_one_period {
            return Ok(());
        }
        let AspenSupplyState::Unmigrated { minted_tokens } = &snapshot.aspen_supply_state else {
            return Err(FinalChainNativeSessionError::RewardsScopeUnsupported);
        };
        let key = ConcreteStorageKey(concrete_storage_key(&[&[6]]));
        let current = trace.current(DPOS_CONTRACT_ADDRESS, key)?;
        let before = self
            .dpos_state
            .aspen_supply_state
            .minted_tokens()
            .map(StoredDposTokenAmount::amount)
            .unwrap_or_default();
        validate_raw_value(
            &current,
            &concrete_u256_bytes(before.as_u256()),
            "minted rewards",
        )?;
        trace.put(
            DPOS_CONTRACT_ADDRESS,
            key,
            concrete_u256_bytes(minted_tokens.as_u256()),
        )
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
