//! Read-only DPoS calls over staged and delayed native-session snapshots.
//!
//! Current-state reads borrow the session's evolving DPoS snapshot, so they see
//! successful earlier native mutations even when an enclosing ordinary EVM
//! frame rolls back. Eligibility reads instead borrow the immutable delayed
//! snapshot captured when the session began. Query execution emits no account,
//! raw-storage or log mutations and cannot publish either snapshot.

use super::*;

/// Reports whether a decoded DPoS call is a read supported by staged sessions.
///
/// Both successfully decoded and malformed instances are classified so the
/// existing admission policy can charge gas and return the contract's normal
/// ABI failure without consulting raw state.
pub(super) fn is_query(transaction: &DposTransaction) -> bool {
    matches!(
        transaction,
        DposTransaction::IsValidatorEligible(_)
            | DposTransaction::GetTotalEligibleVotesCount
            | DposTransaction::GetValidatorEligibleVotesCount(_)
            | DposTransaction::GetValidator(_)
            | DposTransaction::GetTotalDelegation(_)
            | DposTransaction::GetDelegations(_)
            | DposTransaction::GetValidators(_)
            | DposTransaction::GetValidatorsFor(_)
            | DposTransaction::GetUndelegations(_)
            | DposTransaction::GetUndelegationsV2(_)
            | DposTransaction::GetUndelegationV2(_)
    )
}

impl FinalChainNativeSession<'_> {
    /// Executes one admitted query against its selected immutable or evolving view.
    ///
    /// `abi_data_len` is the exact calldata length after the selector and is
    /// used only to reproduce legacy Go ABI errors. Kernel/invariant errors
    /// abort the session; normal decode and missing-record errors produce a
    /// mutation-free contract failure.
    pub(super) fn invoke_query(
        &self,
        transaction: DposTransaction,
        abi_data_len: usize,
        quote: FinalChainNativeGasQuote,
    ) -> std::result::Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let diagnostic = query_failure_diagnostic(&transaction, abi_data_len);
        let outcome = match transaction {
            eligibility @ (DposTransaction::IsValidatorEligible(_)
            | DposTransaction::GetTotalEligibleVotesCount
            | DposTransaction::GetValidatorEligibleVotesCount(_)) => {
                self.final_chain.apply_dpos_eligibility_read(
                    &self.eligibility_state,
                    self.eligibility_period,
                    eligibility,
                )
            }
            DposTransaction::GetTotalDelegation(read) => self
                .final_chain
                .apply_dpos_total_delegation_read(&self.dpos_state, read),
            page @ DposTransaction::GetDelegations(_) => self
                .final_chain
                .apply_dpos_delegation_page_read(&self.dpos_state, page),
            singleton @ (DposTransaction::GetValidator(_)
            | DposTransaction::GetUndelegationV2(_)) => self
                .final_chain
                .apply_dpos_fixed_singleton_read(&self.dpos_state, self.pending_period, singleton),
            page @ (DposTransaction::GetValidators(_) | DposTransaction::GetValidatorsFor(_)) => {
                self.final_chain.apply_dpos_validator_page_read(
                    &self.dpos_state,
                    self.pending_period,
                    page,
                )
            }
            page @ (DposTransaction::GetUndelegations(_)
            | DposTransaction::GetUndelegationsV2(_)) => self
                .final_chain
                .apply_dpos_undelegation_page_read(&self.dpos_state, self.pending_period, page),
            _ => unreachable!("query classifier routed a mutation"),
        }
        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;

        let status = if outcome.status_code == 1 {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: outcome
                    .contract_error
                    .map(|error| error.legacy_message())
                    .or(diagnostic)
                    .unwrap_or_default(),
            }
        };
        debug_assert!(outcome.logs.is_empty(), "DPoS queries never emit logs");
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status,
                gas_used: quote.required_gas,
                output: outcome.code_retval,
                account_mutations: Vec::new(),
                raw_mutations: Vec::new(),
                logs: Vec::new(),
            },
        ))
    }
}

fn query_failure_diagnostic(transaction: &DposTransaction, abi_data_len: usize) -> Option<String> {
    match transaction {
        DposTransaction::IsValidatorEligible(Err(_))
        | DposTransaction::GetValidatorEligibleVotesCount(Err(_))
        | DposTransaction::GetValidator(Err(_))
        | DposTransaction::GetTotalDelegation(Err(_))
        | DposTransaction::GetValidators(Err(_)) => Some(go_abi_length_error(abi_data_len, 1)),
        DposTransaction::GetDelegations(Err(_))
        | DposTransaction::GetValidatorsFor(Err(_))
        | DposTransaction::GetUndelegations(Err(_))
        | DposTransaction::GetUndelegationsV2(Err(_)) => Some(go_abi_length_error(abi_data_len, 2)),
        DposTransaction::GetUndelegationV2(Err(_)) => Some(go_abi_length_error(abi_data_len, 3)),
        DposTransaction::GetValidator(Ok(_)) => Some("Validator does not exist".to_owned()),
        DposTransaction::GetUndelegationV2(Ok(_)) => Some("Undelegation does not exist".to_owned()),
        _ => None,
    }
}

fn go_abi_length_error(data_len: usize, words: usize) -> String {
    // Taraxa's pinned `accounts/abi/unpack.go::toGoType` visits fixed ABI words
    // in order and reports the end offset of the first unavailable word.
    let required = (1..=words)
        .map(|word| word * 32)
        .find(|required| *required > data_len)
        .unwrap_or(words * 32);
    format!("abi: cannot marshal in to go type: length insufficient {data_len} require {required}")
}
