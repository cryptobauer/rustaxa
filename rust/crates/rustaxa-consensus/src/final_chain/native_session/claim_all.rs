//! Finite positive noarg claim-all execution over the canonical P1 topology.
//!
//! Gas preparation authenticates only the caller's LE32 count. Invocation binds
//! that observation and materializes ordered item rows before executing two
//! scalar claims on shared private account, semantic and raw views. The first
//! pays a positive reward; the second pays zero but still writes its checkpoint.
//! Broader topologies and business failures are admission errors, never receipts
//! with a silently discarded irreversible prefix. No effect escapes on error.

use super::account::{DposAccountPort, StagedDposAccountPort};
use super::custody::{
    delegator_validators_prefix, iterable_count_key, iterable_item_key, map_kernel_error,
};
use super::raw::FinalChainNativeRawTrace;
use super::*;

/// Private gas authority and ordered semantic traversal retained by preparation.
/// Item rows are intentionally not read until invocation. Exact request/quote
/// identity remains owned by the enclosing prepared call.
#[derive(Clone)]
pub(super) struct PreparedClaimAll {
    caller: [u8; 20],
    validators: [[u8; 20]; 2],
    count_key: ConcreteStorageKey,
    count_read: ConcreteRead<Vec<u8>>,
}

fn scope<T>() -> Result<T, FinalChainNativeSessionError> {
    Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
}

impl FinalChainNativeSession<'_> {
    /// Binds the measured direct, first-in-session, funded noarg request.
    /// Reads only the exact present count; excluded shapes poison preparation
    /// through the caller, while unavailable/mismatched rows remain hard errors.
    pub(super) fn prepare_claim_all(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        if self.origin != NativeSessionOrigin::PendingPeriod
            || self.pending_period != 2.into()
            || self.next_sequence != 0
            || request.input != DPOS_CLAIM_ALL_REWARDS_SELECTOR
            || request.depth != 0
            || request.kind != FinalChainNativeCallKind::Call
            || request.is_static
            || request.value.value() != &BigUint::default()
            || request.supplied_gas.as_u64() < 90_000
            || self.pending_period < self.final_chain.rewards_config.fix_claim_all_block_num
            || self.pending_period < self.final_chain.rewards_config.aspen_part_one_period
            || !self.final_chain.magnolia_active(self.pending_period)
            || !self.final_chain.ficus_active_at(self.pending_period)
            || self.final_chain.aspen_part_two_active(self.pending_period)
            || self.final_chain.dpos_validator_maximum_stake.as_u256() != U256::from(1_000_000)
        {
            return scope();
        }
        let Some(rows) = self.dpos_state.delegator_validators.get(&request.caller) else {
            return scope();
        };
        let Ok(validators) = <[[u8; 20]; 2]>::try_from(rows.as_slice()) else {
            return scope();
        };
        if validators[0] == validators[1] || rows != &self.dpos_state.validator_order {
            return scope();
        }
        let count_key = iterable_count_key(&delegator_validators_prefix(request.caller));
        let count_read = state.raw_storage(DPOS_CONTRACT_ADDRESS, &count_key)?;
        if count_read != ConcreteRead::Present(2u32.to_le_bytes().to_vec()) {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "claim-all count".into(),
            ));
        }
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: 90_000.into(),
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind: PreparedKind::ClaimAll(PreparedClaimAll {
                caller: request.caller,
                validators,
                count_key,
                count_read,
            }),
        });
        Ok(quote)
    }

    /// Qualifies the measured role/order/graph shape without extra raw reads.
    /// Address renaming is allowed; numeric stake, reward and node topology are
    /// the canonical witness. Missing provenance and graph inconsistencies fail
    /// closed. Ordinary account existence and affordability are checked below.
    fn qualify_claim_all(
        &self,
        prepared: &PreparedClaimAll,
    ) -> Result<(), FinalChainNativeSessionError> {
        let s = &self.dpos_state;
        let [first, second] = prepared.validators;
        let caller = prepared.caller;
        let Some(metadata) = s.validator_metadata.get(&first) else {
            return scope();
        };
        let owner = metadata.owner;
        if caller == DPOS_CONTRACT_ADDRESS
            || [first, second, owner].contains(&caller)
            || [first, second, DPOS_CONTRACT_ADDRESS].contains(&owner)
            || s.validator_order != prepared.validators
            || s.total_stakes.len() != 2
            || s.validator_metadata.len() != 2
            || s.vrf_keys.len() != 2
            || s.delegations.len() != 2
            || s.delegator_validators.len() != 2
            || !s.validator_reward_per_stake.is_empty()
            || s.delegation_reward_cursors.len() != 2
            || !s.delegation_ledger_history_complete
            || !s.redelegate_same_validator_history_complete
            || !s.redelegate_same_validator_corruption.is_empty()
            || s.delegator_rewards
                .keys()
                .chain(s.commission_rewards.keys())
                .any(|v| !prepared.validators.contains(v))
        {
            return scope();
        }
        validate_dpos_principal_ledger(s).map_err(map_kernel_error)?;
        let graph = &s.reward_reference_graph;
        let graph_error =
            |e: DposRewardGraphError| FinalChainNativeSessionError::Domain(e.to_string());
        if graph.current_block().map_err(graph_error)? != 2
            || graph.live_nodes().map_err(graph_error)?.len() != 2
        {
            return scope();
        }
        // Canonical genesis mirrors are absent. The graph carries the zero
        // indices. Its older head is not the exceptional legacy stale flag.
        let zero = DposRewardIndex::zero();
        for (index, validator) in prepared.validators.into_iter().enumerate() {
            if s.validator_metadata
                .get(&validator)
                .is_none_or(|m| m.owner != owner)
                || !s.vrf_keys.contains_key(&validator)
                || s.total_stakes
                    .get(&validator)
                    .is_none_or(|v| v.as_u256() != U256::from(2000))
                || s.delegations.get(&validator).is_none_or(|rows| {
                    rows.len() != 2
                        || [caller, owner]
                            .iter()
                            .any(|a| rows.get(a).is_none_or(|v| v.as_u256() != U256::from(1000)))
                })
                || [caller, owner].iter().any(|a| {
                    s.delegator_validators.get(a).map(Vec::as_slice)
                        != Some(prepared.validators.as_slice())
                })
                || s.delegation_reward_cursors
                    .get(&validator)
                    .is_none_or(|rows| {
                        rows.len() != 2
                            || [caller, owner]
                                .iter()
                                .any(|a| rows.get(a).is_none_or(|v| v.index != zero))
                    })
                || s.validator_reward_per_stake
                    .get(&validator)
                    .is_some_and(|v| v.index != zero)
                || s.delegator_rewards
                    .get(&validator)
                    .map(StoredDposTokenAmount::as_u256)
                    .unwrap_or_default()
                    != U256::from(if index == 0 { 792 } else { 0 })
                || s.commission_rewards
                    .get(&validator)
                    .map(StoredDposTokenAmount::as_u256)
                    .unwrap_or_default()
                    != U256::from(if index == 0 { 8 } else { 0 })
                || graph.read_validator_head(&validator).map_err(graph_error)? != 0
                || graph.is_stale_head(&validator).map_err(graph_error)?
                || [caller, owner]
                    .iter()
                    .any(|a| graph.read_cursor(&validator, a).ok() != Some(0))
            {
                return scope();
            }
            let node = graph
                .load_node(&NodeKey {
                    validator,
                    block: 0,
                })
                .map_err(graph_error)?;
            if node.count != 3 || node.reward_per_stake != zero {
                return scope();
            }
        }
        Ok(())
    }

    /// Runs the two snapshotted scalar steps on one private evolving overlay.
    /// Count and item disagreement, infrastructure failure, or an unexpected
    /// kernel result returns an error; the caller aborts the complete session.
    /// Only a fully qualified success publishes one outcome and semantic view.
    pub(super) fn invoke_claim_all(
        &mut self,
        prepared: PreparedClaimAll,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if state.raw_storage(DPOS_CONTRACT_ADDRESS, &prepared.count_key)? != prepared.count_read
            || self
                .dpos_state
                .delegator_validators
                .get(&prepared.caller)
                .map(Vec::as_slice)
                != Some(prepared.validators.as_slice())
        {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "claim-all bound count/order".into(),
            ));
        }
        let prefix = delegator_validators_prefix(prepared.caller);
        for (index, validator) in prepared.validators.into_iter().enumerate() {
            let key = iterable_item_key(&prefix, index as u32 + 1);
            if state.raw_storage(DPOS_CONTRACT_ADDRESS, &key)?
                != ConcreteRead::Present(validator.to_vec())
            {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "claim-all ordered item".into(),
                ));
            }
        }
        self.qualify_claim_all(&prepared)?;
        let mut accounts = StagedDposAccountPort::from_state(state);
        if !accounts
            .account(prepared.caller)
            .map_err(map_kernel_error)?
            .exists
            || !accounts
                .account(DPOS_CONTRACT_ADDRESS)
                .map_err(map_kernel_error)?
                .exists
        {
            return scope();
        }
        let mut next = self.dpos_state.clone();
        let mut trace = FinalChainNativeRawTrace::new(state);
        let mut logs = Vec::new();
        for (index, validator) in prepared.validators.into_iter().enumerate() {
            let before = next.clone();
            let outcome = self
                .final_chain
                .apply_dpos_delegator_reward_claim_for_contract(
                    &mut next,
                    &mut accounts,
                    validator,
                    prepared.caller,
                )
                .map_err(map_kernel_error)?;
            // These log roles also bind the selected positive/zero arithmetic.
            // No unsupported business result may discard a prior raw prefix.
            if outcome.status_code != 1
                || !outcome.code_retval.is_empty()
                || outcome.logs.len() != if index == 0 { 1 } else { 0 }
            {
                return scope();
            }
            self.serialize_delegator_claim(prepared.caller, validator, &before, &next, &mut trace)?;
            logs.extend(outcome.logs.into_iter().map(|log| FinalChainCallLog {
                address: log.address,
                topics: log.topics,
                data: log.data,
            }));
        }
        self.dpos_state = next;
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status: FinalChainNativeStatus::Success,
                gas_used: quote.required_gas,
                output: Vec::new(),
                account_mutations: accounts.into_mutations(),
                raw_mutations: trace.finish(),
                logs,
            },
        ))
    }
}
