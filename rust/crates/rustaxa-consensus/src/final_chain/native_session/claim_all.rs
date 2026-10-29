//! Finite noarg claim-all execution over canonical P1 and H1/P0 trace topologies.
//!
//! Positive P1 admission permits HistoricalSimulation1 and PendingPeriod2,
//! with the reward graph bound to that selected execution block.
//! Gas preparation authenticates only the caller's LE32 count. Invocation binds
//! that observation and materializes ordered item rows before executing two
//! scalar claims on shared private account, semantic and raw views. The first
//! pays a positive reward; the second pays zero but still writes its checkpoint.
//! Broader topologies and business failures are admission errors, never receipts
//! with a silently discarded irreversible prefix. No effect escapes on error.
//! Underfunded first calls qualify the complete preloaded semantic profile and
//! select a read-free gas terminal. Go's lazy counter/supply reads are carried
//! by that authenticated source boundary, not repeated as journal callbacks.
//! HistoricalTrace1 loads exact zero-reward P0, changes only private execution
//! context, and admits native gas178728 at transaction0. Its scalar steps read
//! no ordinary accounts and emit no balances/logs. General trace queries and
//! other caps remain closed.
//! The separate HistoricalTraceBoundary1 origin admits the six measured native
//! gas values0/1/68728/89999/90000/90001 on that same P0 source. Underfunded
//! calls bind an InsufficientGas terminal after one count read; funded calls
//! share the zero-payout scalar path. The178728-only origin stays distinct.
//! HistoricalTraceOrdinaryPrefix binds the composition owner's qualified caller
//! and admits only transaction1/native sequence0 with native gas178728. The EVM
//! owner must prove the exact successful ordinary prefix before construction;
//! this origin shares P0 scalar/source checks without changing the older guards.

//! The separate ClaimAllNonPayable origin qualifies positive P1 at construction
//! and admits only value1 at the six measured caps, with zero quote gas and no
//! native callbacks. Its disposable terminal returns the Cornus nonpayable
//! failure; ordinary transfer rollback remains with the EVM owner.
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

#[derive(Clone)]
pub(super) struct PreparedCurrentClaimAll {
    caller: [u8; 20],
    validators: Vec<[u8; 20]>,
    count_key: ConcreteStorageKey,
    count_read: ConcreteRead<Vec<u8>>,
}

/// Exact legacy page and raw count bound to the enclosing prepared request.
#[derive(Clone)]
pub(super) struct PreparedLegacyClaimAll {
    caller: [u8; 20],
    validators: Vec<[u8; 20]>,
    count_key: ConcreteStorageKey,
    count_read: ConcreteRead<Vec<u8>>,
    start: usize,
    end: usize,
}

fn scope<T>() -> Result<T, FinalChainNativeSessionError> {
    Err(FinalChainNativeSessionError::ClaimsScopeUnsupported)
}

impl FinalChainNativeSession<'_> {
    /// Bounded post-Magnolia old-selector route. Its inputs and membership are
    /// deliberately narrower than the general uint32 ABI.
    pub(super) fn prepare_legacy_claim_all(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        let boundary = self
            .final_chain
            .rewards_config
            .fix_claim_all_block_num
            .as_u64();
        let period = request.period.as_u64();
        if boundary.checked_sub(1).is_none_or(|p| period < p)
            || boundary.checked_add(1).is_none_or(|p| period > p)
            || !self.final_chain.magnolia_active(request.period)
            || request.period >= self.final_chain.dpos_cornus_period
            || request.kind != FinalChainNativeCallKind::Call
            || request.is_static
            || !matches!(
                self.origin,
                NativeSessionOrigin::PendingPeriod
                    | NativeSessionOrigin::HistoricalSimulation
                    | NativeSessionOrigin::CurrentHistoricalTrace
            )
        {
            return scope();
        }
        let batch = if request.input.len() == 4 {
            None
        } else if request.input.len() == 36
            && request.input[4..35].iter().all(|b| *b == 0)
            && request.input[35] <= 1
        {
            Some(request.input[35] as usize)
        } else {
            return scope();
        };
        let zero = request.value.value() == &BigUint::default();
        let one = request.value.value() == &BigUint::from(1u8);
        if !(request.depth == 0
            && ((batch == Some(0) && (zero || one))
                || (batch == Some(1) && zero)
                || (batch.is_none() && zero))
            || request.depth == 1 && batch == Some(0) && zero)
        {
            return scope();
        }
        let quote_zero = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: FinalChainGas::ZERO,
        };
        if period >= boundary || batch.is_none() {
            let error = if period >= boundary {
                "no method with id: 0x09b72e00"
            } else {
                "abi: cannot marshal in to go type: length insufficient 0 require 32"
            };
            self.prepared = Some(PreparedCall {
                request: request.clone(),
                quote: quote_zero,
                kind: PreparedKind::AbiFailure(error.into()),
            });
            return Ok(quote_zero);
        }
        if !self.dpos_state.has_complete_validator_shapes()
            || !self.dpos_state.delegation_ledger_history_complete
            || !self.dpos_state.redelegate_same_validator_history_complete
        {
            return scope();
        }
        validate_dpos_principal_ledger(&self.dpos_state).map_err(map_kernel_error)?;
        let validators = self
            .dpos_state
            .delegator_validators
            .get(&request.caller)
            .cloned()
            .unwrap_or_default();
        if !matches!(validators.len(), 0 | 2)
            || (request.depth == 0 && validators.len() != 2)
            || (request.depth == 1 && !validators.is_empty())
            || validators.iter().copied().collect::<BTreeSet<_>>().len() != validators.len()
        {
            return scope();
        }
        let count_key = iterable_count_key(&delegator_validators_prefix(request.caller));
        let count_read = state.raw_storage(DPOS_CONTRACT_ADDRESS, &count_key)?;
        let count = validators.len() as u32;
        let count_matches = match &count_read {
            ConcreteRead::Present(bytes) => {
                bytes == &count.to_le_bytes() || (count == 0 && bytes.is_empty())
            }
            ConcreteRead::Absent | ConcreteRead::Tombstone => count == 0,
        };
        if !count_matches {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "legacy claim-all count".into(),
            ));
        }
        let start = batch
            .unwrap()
            .checked_mul(DPOS_CLAIM_ALL_REWARDS_MAX_COUNT)
            .ok_or_else(|| {
                FinalChainNativeSessionError::Domain("legacy claim-all page overflow".into())
            })?;
        let end = start
            .checked_add(DPOS_CLAIM_ALL_REWARDS_MAX_COUNT)
            .ok_or_else(|| {
                FinalChainNativeSessionError::Domain("legacy claim-all page overflow".into())
            })?;
        let selected = end.min(validators.len()).saturating_sub(start);
        let gas = 45_000u64 * (selected.max(1) as u64);
        if request.supplied_gas.as_u64() <= gas + 1 {
            return scope();
        }
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: gas.into(),
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind: PreparedKind::LegacyClaimAll(PreparedLegacyClaimAll {
                caller: request.caller,
                validators,
                count_key,
                count_read,
                start,
                end,
            }),
        });
        Ok(quote)
    }

    pub(super) fn invoke_legacy_claim_all(
        &mut self,
        prepared: PreparedLegacyClaimAll,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if state.raw_storage(DPOS_CONTRACT_ADDRESS, &prepared.count_key)? != prepared.count_read
            || self
                .dpos_state
                .delegator_validators
                .get(&prepared.caller)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                != prepared.validators.as_slice()
        {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "legacy claim-all bound count/order".into(),
            ));
        }
        let prefix = delegator_validators_prefix(prepared.caller);
        for (index, validator) in prepared.validators.iter().enumerate() {
            if state.raw_storage(
                DPOS_CONTRACT_ADDRESS,
                &iterable_item_key(&prefix, index as u32 + 1),
            )? != ConcreteRead::Present(validator.to_vec())
            {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "legacy claim-all ordered item".into(),
                ));
            }
        }
        let mut next = self.dpos_state.clone();
        let mut accounts = StagedDposAccountPort::from_state(state);
        let mut trace = FinalChainNativeRawTrace::new(state);
        let mut logs = Vec::new();
        for validator in prepared
            .validators
            .iter()
            .skip(prepared.start)
            .take(prepared.end.saturating_sub(prepared.start))
        {
            let before = next.clone();
            let outcome = self
                .final_chain
                .apply_dpos_delegator_reward_claim_for_contract(
                    &mut next,
                    &mut accounts,
                    *validator,
                    prepared.caller,
                )
                .map_err(map_kernel_error)?;
            if outcome.status_code != 1 || !outcome.code_retval.is_empty() {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "legacy claim-all semantic membership".into(),
                ));
            }
            self.serialize_delegator_claim(
                prepared.caller,
                *validator,
                &before,
                &next,
                &mut trace,
            )?;
            logs.extend(outcome.logs.into_iter().map(|log| FinalChainCallLog {
                address: log.address,
                topics: log.topics,
                data: log.data,
            }));
        }
        let is_end = prepared.start >= prepared.validators.len()
            || prepared.end >= prepared.validators.len();
        let mut output = vec![0; 32];
        output[31] = u8::from(is_end);
        self.dpos_state = next;
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status: FinalChainNativeStatus::Success,
                gas_used: quote.required_gas,
                output,
                account_mutations: accounts.into_mutations(),
                raw_mutations: trace.finish(),
                logs,
            },
        ))
    }

    /// Binds the measured direct, first-in-session noarg request and count quote.
    /// Origin/period is exactly historical1, pending2 or qualified trace1/P0,
    /// with no cross-pair admission. Reads only the exact present count;
    /// excluded shapes poison preparation
    /// through the caller, while unavailable/mismatched rows remain hard errors.
    pub(super) fn prepare_claim_all(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        if self.final_chain.cacti_active(self.pending_period)
            && self.final_chain.aspen_part_two_active(self.pending_period)
            && !self.origin.is_claim_all_trace()
            && self.origin != NativeSessionOrigin::ClaimAllNonPayable
        {
            return self.prepare_current_claim_all(request, state);
        }
        if self.origin == NativeSessionOrigin::ClaimAllNonPayable {
            return self.prepare_nonpayable_claim_all(request);
        }
        if !matches!(
            (self.origin, self.pending_period.as_u64()),
            (NativeSessionOrigin::PendingPeriod, 2)
                | (NativeSessionOrigin::HistoricalSimulation, 1)
                | (NativeSessionOrigin::HistoricalTrace, 1)
                | (NativeSessionOrigin::HistoricalTraceBoundary, 1)
                | (NativeSessionOrigin::HistoricalTraceOrdinaryPrefix(_), 1)
        ) || self.next_sequence != 0
            || (self.origin == NativeSessionOrigin::HistoricalTrace
                && request.supplied_gas.as_u64() != 178728)
            || (self.origin == NativeSessionOrigin::HistoricalTraceBoundary
                && !matches!(
                    request.supplied_gas.as_u64(),
                    0 | 1 | 68728 | 89999 | 90000 | 90001
                ))
            || (matches!(
                self.origin,
                NativeSessionOrigin::HistoricalTrace | NativeSessionOrigin::HistoricalTraceBoundary
            ) && request.id.transaction != FinalChainTransactionPosition::new(0))
            || matches!(self.origin, NativeSessionOrigin::HistoricalTraceOrdinaryPrefix(caller)
                if request.id.transaction != FinalChainTransactionPosition::new(1)
                || request.caller != caller || request.supplied_gas.as_u64() != 178728)
            || request.input != DPOS_CLAIM_ALL_REWARDS_SELECTOR
            || request.depth != 0
            || request.kind != FinalChainNativeCallKind::Call
            || request.is_static
            || request.value.value() != &BigUint::default()
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
        if self.origin.is_claim_all_trace() {
            self.qualify_claim_all(request.caller, validators, self.pending_period.as_u64())?;
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
        let prepared = PreparedClaimAll {
            caller: request.caller,
            validators,
            count_key,
            count_read,
        };
        let kind = if request.supplied_gas.as_u64() < quote.required_gas.as_u64() {
            self.qualify_underfunded_claim_all(&prepared)?;
            PreparedKind::InsufficientGas
        } else {
            PreparedKind::ClaimAll(prepared)
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind,
        });
        Ok(quote)
    }

    fn prepare_current_claim_all(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        let transaction = DposTransaction::ClaimAllRewards {
            delegator: request.caller,
            batch: None,
        };
        let admission = self
            .final_chain
            .native_invocation_admission(
                &transaction,
                request.period,
                request.depth,
                request.value.value(),
                request.supplied_gas,
                Some(&self.dpos_state),
            )
            .map_err(map_kernel_error)?;
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: admission.required_gas,
        };
        let kind = if matches!(
            admission.failure,
            Some(super::super::native_admission::NativeAdmissionFailure::NonPayable)
        ) {
            PreparedKind::NonPayable
        } else {
            let validators = self
                .dpos_state
                .delegator_validators
                .get(&request.caller)
                .cloned()
                .unwrap_or_default();
            let count_key = iterable_count_key(&delegator_validators_prefix(request.caller));
            let count_read = state.raw_storage(DPOS_CONTRACT_ADDRESS, &count_key)?;
            let count = u32::try_from(validators.len())
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            let matches = match &count_read {
                ConcreteRead::Present(bytes) => {
                    bytes == &count.to_le_bytes() || (count == 0 && bytes.is_empty())
                }
                ConcreteRead::Absent | ConcreteRead::Tombstone => count == 0,
            };
            if !matches {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "current claim-all count".into(),
                ));
            }
            if admission.failure.is_some() {
                PreparedKind::InsufficientGas
            } else {
                PreparedKind::CurrentClaimAll(PreparedCurrentClaimAll {
                    caller: request.caller,
                    validators,
                    count_key,
                    count_read,
                })
            }
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind,
        });
        Ok(quote)
    }

    pub(super) fn invoke_current_claim_all(
        &mut self,
        prepared: PreparedCurrentClaimAll,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if state.raw_storage(DPOS_CONTRACT_ADDRESS, &prepared.count_key)? != prepared.count_read {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "current claim-all bound count".into(),
            ));
        }
        if !self.dpos_state.delegation_ledger_history_complete
            || !self.dpos_state.redelegate_same_validator_history_complete
        {
            return scope();
        }
        validate_dpos_principal_ledger(&self.dpos_state).map_err(map_kernel_error)?;
        let prefix = delegator_validators_prefix(prepared.caller);
        let mut unique = BTreeSet::new();
        for (index, validator) in prepared.validators.iter().enumerate() {
            if !unique.insert(*validator)
                || state.raw_storage(
                    DPOS_CONTRACT_ADDRESS,
                    &iterable_item_key(&prefix, index as u32 + 1),
                )? != ConcreteRead::Present(validator.to_vec())
            {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "current claim-all ordered item".into(),
                ));
            }
        }
        let mut next = self.dpos_state.clone();
        let mut accounts = StagedDposAccountPort::from_state(state);
        let mut trace = FinalChainNativeRawTrace::new(state);
        let mut logs = Vec::new();
        for validator in prepared.validators {
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
            if outcome.status_code != 1 {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "claim-all semantic membership is incomplete".into(),
                ));
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

    /// Admits the measured underfunded P1 and zero-payout P0 trace profiles.
    /// The source must associate the loaded semantics with its raw/account
    /// view. These pure checks map Go's lazy setup values; they do not authenticate
    /// arbitrary substituted bootstrap bytes or reproduce callback fault timing.
    /// Funded calls retain their existing invocation-stage qualification.
    fn qualify_underfunded_claim_all(
        &self,
        prepared: &PreparedClaimAll,
    ) -> Result<(), FinalChainNativeSessionError> {
        self.qualify_claim_all(
            prepared.caller,
            prepared.validators,
            self.pending_period.as_u64(),
        )?;
        if self.origin == NativeSessionOrigin::HistoricalTraceBoundary {
            // The shared qualifier already binds votes400/stake4000/mint0.
            // This origin cannot use the positive P1 minted800 profile below.
            return Ok(());
        }
        self.qualify_positive_claim_all_bootstrap()
    }

    /// Pure numeric lazy-bootstrap mapping shared by qualified positive P1
    /// terminals. This does not construct a prepared claim or read its count.
    fn qualify_positive_claim_all_bootstrap(&self) -> Result<(), FinalChainNativeSessionError> {
        if self.dpos_state.total_vote_count != 400
            || total_staked_amount(&self.dpos_state)
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?
                != U256::from(4000)
            || !matches!(
                &self.dpos_state.aspen_supply_state,
                AspenSupplyState::Unmigrated { minted_tokens }
                    if minted_tokens.as_u256() == U256::from(800)
            )
        {
            return scope();
        }
        Ok(())
    }

    /// Qualifies exact positive P1 before private execution-context adjustment.
    /// Raw bootstrap values/absence and ordinary accounts must be associated
    /// with this semantic history at composition, outside native callbacks.
    pub(super) fn qualify_nonpayable_claim_all_source(
        &self,
        graph_period: u64,
    ) -> Result<(), FinalChainNativeSessionError> {
        if !matches!(self.pending_period.as_u64(), 1 | 2)
            || self.pending_period < self.final_chain.rewards_config.fix_claim_all_block_num
            || self.pending_period < self.final_chain.rewards_config.aspen_part_one_period
            || !self.final_chain.magnolia_active(self.pending_period)
            || !self.final_chain.ficus_active_at(self.pending_period)
            || self.final_chain.aspen_part_two_active(self.pending_period)
            || self.final_chain.dpos_validator_maximum_stake.as_u256() != U256::from(1_000_000)
        {
            return scope();
        }
        let Ok(validators) = <[[u8; 20]; 2]>::try_from(self.dpos_state.validator_order.as_slice())
        else {
            return scope();
        };
        let Some(owner) = self
            .dpos_state
            .validator_metadata
            .get(&validators[0])
            .map(|m| m.owner)
        else {
            return scope();
        };
        let Some(caller) = self
            .dpos_state
            .delegator_validators
            .keys()
            .find(|a| **a != owner)
        else {
            return scope();
        };
        self.qualify_claim_all(*caller, validators, graph_period)?;
        self.qualify_positive_claim_all_bootstrap()?;
        let s = &self.dpos_state;
        if validators.contains(&DPOS_CONTRACT_ADDRESS)
            || s.vote_counts.len() != 2
            || validators.iter().any(|v| {
                s.vote_counts.get(v) != Some(&200)
                    || s.validator_metadata
                        .get(v)
                        .is_none_or(|m| m.commission != 100 || m.last_commission_change != 0)
            })
            || !s.undelegations.is_empty()
            || !s.undelegations_v2.is_empty()
            || !s.undelegation_v2_last_ids.is_empty()
            || !s.slashing_jail_blocks.is_empty()
            || !s.slashing_jailed_validators.is_empty()
            || !s.slashing_double_voting_proofs.is_empty()
        {
            return scope();
        }
        Ok(())
    }

    /// Binds a read-free nonpayable terminal for only the measured value1 caps.
    /// The ordinary owner has already transferred value and advanced the nonce;
    /// failure rollback remains outside this semantic session.
    fn prepare_nonpayable_claim_all(
        &mut self,
        request: &FinalChainNativeRequest,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        if self.next_sequence != 0
            || request.id.transaction != FinalChainTransactionPosition::new(0)
            || request.input != DPOS_CLAIM_ALL_REWARDS_SELECTOR
            || request.depth != 0
            || request.kind != FinalChainNativeCallKind::Call
            || request.is_static
            || request.value.value() != &BigUint::from(1u8)
            || !matches!(
                request.supplied_gas.as_u64(),
                0 | 1 | 68728 | 89999 | 90000 | 90001
            )
        {
            return scope();
        }
        self.qualify_nonpayable_claim_all_source(self.pending_period.as_u64())?;
        if !self
            .dpos_state
            .delegator_validators
            .contains_key(&request.caller)
            || self
                .dpos_state
                .validator_metadata
                .values()
                .any(|m| m.owner == request.caller)
        {
            return scope();
        }
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: 0.into(),
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind: PreparedKind::NonPayable,
        });
        Ok(quote)
    }

    /// Qualifies the measured role/order/graph shape without extra raw reads.
    /// Address renaming is allowed; numeric stake, reward and node topology are
    /// the canonical witness. Missing provenance and graph inconsistencies fail
    /// closed. Ordinary account existence and affordability are checked below.
    pub(super) fn qualify_claim_all(
        &self,
        caller: [u8; 20],
        validators: [[u8; 20]; 2],
        graph_period: u64,
    ) -> Result<(), FinalChainNativeSessionError> {
        let s = &self.dpos_state;
        let [first, second] = validators;
        let trace = self.origin.is_claim_all_trace();
        let Some(metadata) = s.validator_metadata.get(&first) else {
            return scope();
        };
        let owner = metadata.owner;
        if first == second
            || (trace && [first, second].contains(&DPOS_CONTRACT_ADDRESS))
            || caller == DPOS_CONTRACT_ADDRESS
            || [first, second, owner].contains(&caller)
            || [first, second, DPOS_CONTRACT_ADDRESS].contains(&owner)
            || s.validator_order != validators
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
                .any(|v| !validators.contains(v))
        {
            return scope();
        }
        validate_dpos_principal_ledger(s).map_err(map_kernel_error)?;
        let graph = &s.reward_reference_graph;
        let graph_error =
            |e: DposRewardGraphError| FinalChainNativeSessionError::Domain(e.to_string());
        if graph.current_block().map_err(graph_error)? != graph_period
            || graph.live_nodes().map_err(graph_error)?.len() != 2
        {
            return scope();
        }
        // Canonical genesis mirrors are absent. The graph carries the zero
        // indices. Its older head is not the exceptional legacy stale flag.
        let zero = DposRewardIndex::zero();
        for (index, validator) in validators.into_iter().enumerate() {
            if s.validator_metadata.get(&validator).is_none_or(|m| {
                m.owner != owner
                    || (trace && (m.commission != 100 || m.last_commission_change != 0))
            }) || !s.vrf_keys.contains_key(&validator)
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
                    s.delegator_validators.get(a).map(Vec::as_slice) != Some(validators.as_slice())
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
                    != U256::from(if index == 0 && !trace { 792 } else { 0 })
                || s.commission_rewards
                    .get(&validator)
                    .map(StoredDposTokenAmount::as_u256)
                    .unwrap_or_default()
                    != U256::from(if index == 0 && !trace { 8 } else { 0 })
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
        if trace
            && (s.total_vote_count != 400
                || s.vote_counts.len() != 2
                || validators
                    .iter()
                    .any(|v| s.vote_counts.get(v) != Some(&200))
                || !s.undelegations.is_empty()
                || !s.undelegations_v2.is_empty()
                || !s.undelegation_v2_last_ids.is_empty()
                || !s.slashing_jail_blocks.is_empty()
                || !s.slashing_jailed_validators.is_empty()
                || !s.slashing_double_voting_proofs.is_empty()
                || total_staked_amount(s).map_err(map_kernel_error)? != U256::from(4000)
                || !matches!(
                    &s.aspen_supply_state,
                    AspenSupplyState::Unmigrated { minted_tokens }
                        if minted_tokens.as_u256().is_zero()
                ))
        {
            return scope();
        }
        Ok(())
    }

    /// Authenticates the complete genesis topology before private context binding.
    pub(super) fn qualify_trace_claim_all_source(
        &self,
    ) -> Result<(), FinalChainNativeSessionError> {
        let Ok(validators) = <[[u8; 20]; 2]>::try_from(self.dpos_state.validator_order.as_slice())
        else {
            return scope();
        };
        let Some(owner) = self
            .dpos_state
            .validator_metadata
            .get(&validators[0])
            .map(|m| m.owner)
        else {
            return scope();
        };
        let Some(caller) = self
            .dpos_state
            .delegator_validators
            .keys()
            .find(|a| **a != owner)
        else {
            return scope();
        };
        self.qualify_claim_all(*caller, validators, 0)
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
        self.qualify_claim_all(
            prepared.caller,
            prepared.validators,
            self.pending_period.as_u64(),
        )?;
        let zero_trace = self.origin.is_claim_all_trace();
        let mut accounts = StagedDposAccountPort::from_state(state);
        if !zero_trace
            && (!accounts
                .account(prepared.caller)
                .map_err(map_kernel_error)?
                .exists
                || !accounts
                    .account(DPOS_CONTRACT_ADDRESS)
                    .map_err(map_kernel_error)?
                    .exists)
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
                || outcome.logs.len() != if index == 0 && !zero_trace { 1 } else { 0 }
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
        let account_mutations = accounts.into_mutations();
        if zero_trace && !account_mutations.is_empty() {
            return scope();
        }
        self.dpos_state = next;
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status: FinalChainNativeStatus::Success,
                gas_used: quote.required_gas,
                output: Vec::new(),
                account_mutations,
                raw_mutations: trace.finish(),
                logs,
            },
        ))
    }
}
