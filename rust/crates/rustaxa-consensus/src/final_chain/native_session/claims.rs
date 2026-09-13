//! Ordered staged execution for `claimRewards` and commission claims.
//!
//! Both adapters call the existing FinalChain reward kernels through the
//! full-width staged account port. Successful calls then reproduce the native
//! contract's irreversible raw-storage call order from the semantic snapshots
//! on either side of the kernel. Ordinary balance changes and logs remain in
//! the enclosing EVM frame. Infrastructure failures poison the unpublished
//! session and expose no partial effect.

use super::account::StagedDposAccountPort;
use super::custody::{
    ExpectedRaw, NodeTrace, checked_put, delegation_key, encode_delegation, map_kernel_error,
    put_rewards,
};
use super::raw::FinalChainNativeRawTrace;
use super::*;

/// Returns whether a decoded selected mutation belongs to this claims adapter.
pub(super) fn is_claim(transaction: &DposTransaction) -> bool {
    matches!(
        transaction,
        DposTransaction::ClaimRewards { .. } | DposTransaction::ClaimCommissionRewards { .. }
    )
}

impl FinalChainNativeSession<'_> {
    /// Runs one prepared reward claim and returns its ordered semantic effects.
    pub(super) fn invoke_selected_claim(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if !self.final_chain.magnolia_active(self.pending_period) {
            return Err(FinalChainNativeSessionError::ClaimsScopeUnsupported);
        }
        if let DposTransaction::ClaimCommissionRewards { owner, validator } = &transaction {
            // Preserve the contract's owner-check precedence. Once ownership is
            // established, zero-stake commission claims enter fork-sensitive
            // validator deletion/retention paths outside this first corpus.
            if self
                .dpos_state
                .validator_metadata
                .get(validator)
                .is_some_and(|metadata| metadata.owner == *owner)
                && self
                    .dpos_state
                    .total_stakes
                    .get(validator)
                    .is_some_and(StoredDposTokenAmount::is_zero)
            {
                return Err(FinalChainNativeSessionError::ClaimsScopeUnsupported);
            }
        }

        let before = self.dpos_state.clone();
        let mut next = before.clone();
        let mut accounts = StagedDposAccountPort::from_state(state);
        let outcome = match transaction.clone() {
            DposTransaction::ClaimRewards {
                delegator,
                validator,
            } => self
                .final_chain
                .apply_dpos_delegator_reward_claim_for_contract(
                    &mut next,
                    &mut accounts,
                    validator,
                    delegator,
                ),
            DposTransaction::ClaimCommissionRewards { owner, validator } => {
                self.final_chain.apply_dpos_commission_reward_claim(
                    &mut next,
                    &mut accounts,
                    owner,
                    validator,
                    self.pending_period,
                )
            }
            _ => return Err(FinalChainNativeSessionError::UnsupportedOperation),
        }
        .map_err(map_kernel_error)?;

        let status = if outcome.status_code == 1 {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: outcome
                    .contract_error
                    .as_ref()
                    .map(DposContractError::legacy_message)
                    .unwrap_or_default(),
            }
        };
        let (account_mutations, raw_mutations) = if outcome.status_code == 1 {
            let mut trace = FinalChainNativeRawTrace::new(state);
            self.serialize_selected_claim(&transaction, &before, &next, &mut trace)?;
            self.dpos_state = next;
            (accounts.into_mutations(), trace.finish())
        } else {
            (Vec::new(), Vec::new())
        };

        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status,
                gas_used: quote.required_gas,
                output: outcome.code_retval,
                account_mutations,
                raw_mutations,
                logs: outcome
                    .logs
                    .into_iter()
                    .map(|log| FinalChainCallLog {
                        address: log.address,
                        topics: log.topics,
                        data: log.data,
                    })
                    .collect(),
            },
        ))
    }

    fn serialize_selected_claim(
        &self,
        transaction: &DposTransaction,
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        match transaction {
            DposTransaction::ClaimRewards {
                delegator,
                validator,
            } => self.serialize_delegator_claim(*delegator, *validator, before, after, trace),
            DposTransaction::ClaimCommissionRewards { validator, .. } => {
                self.serialize_commission_claim(*validator, before, after, trace)
            }
            _ => Err(FinalChainNativeSessionError::UnsupportedOperation),
        }
    }

    fn serialize_delegator_claim(
        &self,
        delegator: [u8; 20],
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let current_block = before
            .reward_reference_graph
            .current_block()
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let mut nodes = NodeTrace::new(before)?;
        if !nodes.contains(validator, current_block) {
            let head = before
                .reward_reference_graph
                .read_validator_head(&validator)
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            nodes.decrement_and_write(validator, head, trace)?;
            self.put_validator(before, after, validator, trace)?;
            put_rewards(before, after, validator, trace)?;
        }

        let prior_cursor = before
            .reward_reference_graph
            .read_cursor(&validator, &delegator)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        nodes.decrement_and_write(validator, prior_cursor, trace)?;
        checked_put(
            trace,
            delegation_key(validator, delegator),
            ExpectedRaw::Exact(encode_delegation(before, validator, delegator)?),
            encode_delegation(after, validator, delegator)?,
            "reward-claim delegation",
        )?;
        nodes.write_final(validator, current_block, after, trace)
    }

    fn serialize_commission_claim(
        &self,
        validator: [u8; 20],
        before: &DposSnapshot,
        after: &DposSnapshot,
        trace: &mut FinalChainNativeRawTrace<'_>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        if !after.total_stakes.contains_key(&validator) {
            return Err(FinalChainNativeSessionError::ClaimsScopeUnsupported);
        }
        put_rewards(before, after, validator, trace)
    }
}
