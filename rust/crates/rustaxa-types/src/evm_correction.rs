//! Explicit isolated execution corrections without network activation.

/// Immutable correction set, independent of the instruction phase.
///
/// Corrections have no network activation mapping. Existing constructors select
/// historical rules. Each additional stage must name its complete rule set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CorrectionStage {
    /// Preserve the accepted historical execution rules.
    #[default]
    Historical,
    /// Revert transient writes with their frame; retain all other historical rules.
    TransientRollback,
    /// Add one SELFDESTRUCT refund per account to transient rollback.
    SingleSelfDestructRefund,
    /// Add checked RETURNDATACOPY and its frame failures to the refund stage.
    CheckedReturnDataCopy,
    /// Add bounded Falcon ABI parsing to the checked-copy stage.
    BoundedFalconAbi,
    /// Add hard nested slashing RLP errors and admission guards to bounded Falcon.
    SlashingHardError,
}

impl CorrectionStage {
    /// Whether frame revert restores transient writes under this fixed set.
    pub const fn transient_rollback(self) -> bool {
        matches!(
            self,
            Self::TransientRollback
                | Self::SingleSelfDestructRefund
                | Self::CheckedReturnDataCopy
                | Self::BoundedFalconAbi
                | Self::SlashingHardError
        )
    }

    /// Whether SELFDESTRUCT grants at most one refund per account.
    pub const fn single_selfdestruct_refund(self) -> bool {
        matches!(
            self,
            Self::SingleSelfDestructRefund
                | Self::CheckedReturnDataCopy
                | Self::BoundedFalconAbi
                | Self::SlashingHardError
        )
    }

    /// Whether RETURNDATACOPY uses checked destination arithmetic and fault traces.
    pub const fn checked_return_data_copy(self) -> bool {
        matches!(
            self,
            Self::CheckedReturnDataCopy | Self::BoundedFalconAbi | Self::SlashingHardError
        )
    }
    /// Whether Falcon uses supplied-body bounds without declared-length allocation.
    pub const fn bounded_falcon_abi(self) -> bool {
        matches!(self, Self::BoundedFalconAbi | Self::SlashingHardError)
    }

    /// Whether nested slashing RLP errors abort the pending attempt.
    pub const fn slashing_hard_error(self) -> bool {
        matches!(self, Self::SlashingHardError)
    }
}
