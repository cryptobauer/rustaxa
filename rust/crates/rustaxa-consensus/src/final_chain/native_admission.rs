//! Shared quote/admission ordering for the bounded native-session surface.
//!
//! This module does no state mutation or value transfer. It keeps arbitrary-width
//! call value intact until payability classification and returns the reference's
//! terminal failure before any kernel/account conversion. The staged session and
//! independent concrete replay use the same historical ordering.

use super::*;

/// A normal native termination before business execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeAdmissionFailure {
    InsufficientGas,
    NestedBeforeFix,
    NonPayable,
}

impl NativeAdmissionFailure {
    /// Exact Go error text; none of these failures is ABI output.
    pub(super) const fn message(self) -> &'static str {
        match self {
            Self::InsufficientGas => "out of gas",
            Self::NestedBeforeFix => "only top-level calls are allowed",
            Self::NonPayable => "Method is not payable",
        }
    }
}

/// Immutable quote and optional pre-kernel termination for one decoded call.
pub(super) struct NativeAdmission {
    pub(super) required_gas: FinalChainGas,
    pub(super) failure: Option<NativeAdmissionFailure>,
}

impl FinalChain {
    /// Quotes a recognized DPoS operation and applies funding/depth/payability order.
    ///
    /// The caller supplies the appropriate current or historical gas snapshot.
    /// Call value is never projected into a machine word. RequiredGas precedes
    /// child funding; funded Run applies the pre-fix depth restriction before
    /// the post-Cornus nonpayability error. Lazy cache initialization remains the
    /// surrounding period session's responsibility. Unknown/malformed ABI paths
    /// require their own decoder-error ordering and must not use this helper.
    pub(super) fn native_invocation_admission(
        &self,
        transaction: &DposTransaction,
        period: FinalChainBlockNumber,
        depth: u16,
        value: &BigUint,
        supplied_gas: FinalChainGas,
        gas_snapshot: Option<&DposSnapshot>,
    ) -> Result<NativeAdmission> {
        let nonpayable = period >= self.dpos_cornus_period
            && value != &BigUint::default()
            && !dpos_transaction_is_payable(transaction);
        let required_gas = if nonpayable {
            FinalChainGas::ZERO
        } else {
            dpos_transaction_required_gas(
                transaction,
                period,
                self.rewards_config.fix_claim_all_block_num,
                self.dpos_cornus_period,
                gas_snapshot,
            )?
        };
        let failure = if supplied_gas < required_gas {
            Some(NativeAdmissionFailure::InsufficientGas)
        } else if period < self.rewards_config.fix_redelegate_block_num && depth != 0 {
            Some(NativeAdmissionFailure::NestedBeforeFix)
        } else if nonpayable {
            Some(NativeAdmissionFailure::NonPayable)
        } else {
            None
        };
        Ok(NativeAdmission {
            required_gas,
            failure,
        })
    }
}

/// Operations whose complete decoder/admission/error path is covered by this slice.
pub(super) fn supports_shared_admission(transaction: &DposTransaction) -> bool {
    matches!(
        transaction,
        DposTransaction::SetCommission { .. }
            | DposTransaction::Delegate { .. }
            | DposTransaction::UndelegateV2 { .. }
            | DposTransaction::ConfirmUndelegateV2 { .. }
    )
}
