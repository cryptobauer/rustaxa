//! Rust-owned PBFT threshold planning.
//!
//! This module owns the deterministic `2t+1` threshold calculation and the
//! live cache used by VoteManager in Rust rewrite mode. It does not read
//! FinalChain directly; callers supply total eligible DPoS vote counts only
//! when Rust reports that a cache miss needs that external fact.

use std::collections::BTreeMap;

use crate::pbft_vote_validation::pbft_vote_sortition_threshold;
use crate::verified_votes::PbftVoteType;

/// Outcome status for one PBFT `2t+1` threshold planning pass.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftTwoTPlusOneThresholdStatus {
    /// A cached or newly computed threshold is available.
    Available,
    /// The cache missed and the caller must supply total eligible DPoS votes.
    NeedsDposTotal,
    /// FinalChain is behind the requested eligibility period.
    FutureDposState,
    /// The caller hit an unexpected lookup or boundary failure.
    UnknownError,
    /// The PBFT vote type cannot be used for threshold calculation.
    InvalidVoteType,
    /// Arithmetic overflowed while deriving the `2t+1` threshold.
    CalculationOverflow,
}

impl PbftTwoTPlusOneThresholdStatus {
    /// Stable numeric status used by CXX bridge payloads.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Available => 0,
            Self::NeedsDposTotal => 1,
            Self::FutureDposState => 2,
            Self::UnknownError => 3,
            Self::InvalidVoteType => 4,
            Self::CalculationOverflow => 5,
        }
    }
}

/// Caller facts for one PBFT `2t+1` threshold request.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftTwoTPlusOneThresholdFact {
    /// Period for which the threshold is requested.
    pub pbft_period: u64,
    /// Vote type whose sortition family controls the threshold.
    pub vote_type: PbftVoteType,
    /// Current PBFT chain size. Only this period is cacheable, matching legacy behavior.
    pub current_pbft_chain_size: u64,
    /// PBFT committee size used by soft/cert/next votes.
    pub committee_size: u64,
    /// Proposal committee size used by proposal votes.
    pub number_of_proposers: u64,
    /// Whether `total_dpos_votes_count` contains a fresh FinalChain fact.
    pub has_total_dpos_votes_count: bool,
    /// Total eligible DPoS votes for `pbft_period` when supplied by the caller.
    pub total_dpos_votes_count: u64,
    /// True when FinalChain reported state behind the requested period.
    pub future_dpos_state: bool,
    /// True when a non-future lookup or bridge invariant failed.
    pub unknown_error: bool,
}

/// Rust threshold plan returned to the C++ VoteManager shim.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftTwoTPlusOneThresholdPlan {
    /// Planning status.
    pub status: PbftTwoTPlusOneThresholdStatus,
    /// Stable error code for bridge/log consumers.
    pub error_code: &'static str,
    /// Whether `threshold` is authoritative for this request.
    pub has_threshold: bool,
    /// PBFT `2t+1` threshold when available.
    pub threshold: u64,
    /// PBFT sortition threshold used to derive `threshold` when available.
    pub sortition_threshold: u64,
    /// Whether the caller must fetch total eligible DPoS votes and retry.
    pub needs_total_dpos_votes: bool,
    /// Whether the threshold came from Rust's live cache.
    pub cache_hit: bool,
    /// Whether this planning pass inserted or refreshed the Rust cache.
    pub cached: bool,
}

impl PbftTwoTPlusOneThresholdPlan {
    fn available_with_sortition(
        threshold: u64,
        sortition_threshold: u64,
        cache_hit: bool,
        cached: bool,
    ) -> Self {
        Self {
            status: PbftTwoTPlusOneThresholdStatus::Available,
            error_code: "",
            has_threshold: true,
            threshold,
            sortition_threshold,
            needs_total_dpos_votes: false,
            cache_hit,
            cached,
        }
    }

    fn missing(status: PbftTwoTPlusOneThresholdStatus, error_code: &'static str) -> Self {
        Self {
            status,
            error_code,
            has_threshold: false,
            threshold: 0,
            sortition_threshold: 0,
            needs_total_dpos_votes: matches!(
                status,
                PbftTwoTPlusOneThresholdStatus::NeedsDposTotal
            ),
            cache_hit: false,
            cached: false,
        }
    }
}

/// Rust-owned cache and calculator for PBFT `2t+1` thresholds.
///
/// The cache mirrors legacy VoteManager behavior: it stores one current-period
/// threshold per vote type and does not cache historical or future periods.
#[derive(Debug, Clone, Default)]
pub struct PbftTwoTPlusOneThresholdRuntime {
    current_thresholds: BTreeMap<PbftVoteType, (u64, u64, u64)>,
}

impl PbftTwoTPlusOneThresholdRuntime {
    /// Creates an empty threshold runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Plans or computes the threshold for `fact`.
    ///
    /// Edge behavior:
    /// - Cache hits return immediately without requiring a DPoS total fact.
    /// - Cache misses return `NeedsDposTotal` unless the caller supplied the
    ///   total eligible DPoS vote count.
    /// - Only requests for `current_pbft_chain_size` refresh the cache.
    /// - Invalid vote types propagate as errors from the sortition threshold rule.
    pub fn plan_threshold(
        &mut self,
        fact: PbftTwoTPlusOneThresholdFact,
    ) -> PbftTwoTPlusOneThresholdPlan {
        if matches!(fact.vote_type, PbftVoteType::Invalid) {
            return PbftTwoTPlusOneThresholdPlan::missing(
                PbftTwoTPlusOneThresholdStatus::InvalidVoteType,
                "PBFT_TWO_T_PLUS_ONE_INVALID_VOTE_TYPE",
            );
        }

        if fact.future_dpos_state {
            return PbftTwoTPlusOneThresholdPlan::missing(
                PbftTwoTPlusOneThresholdStatus::FutureDposState,
                "PBFT_TWO_T_PLUS_ONE_FUTURE_DPOS_STATE",
            );
        }

        if fact.unknown_error {
            return PbftTwoTPlusOneThresholdPlan::missing(
                PbftTwoTPlusOneThresholdStatus::UnknownError,
                "PBFT_TWO_T_PLUS_ONE_UNKNOWN_ERROR",
            );
        }

        if let Some((cached_period, threshold, sortition_threshold)) =
            self.current_thresholds.get(&fact.vote_type)
            && *cached_period == fact.pbft_period
            && *threshold != 0
        {
            return PbftTwoTPlusOneThresholdPlan::available_with_sortition(
                *threshold,
                *sortition_threshold,
                true,
                false,
            );
        }

        if !fact.has_total_dpos_votes_count {
            return PbftTwoTPlusOneThresholdPlan::missing(
                PbftTwoTPlusOneThresholdStatus::NeedsDposTotal,
                "PBFT_TWO_T_PLUS_ONE_NEEDS_DPOS_TOTAL",
            );
        }

        let sortition_threshold = pbft_vote_sortition_threshold(
            fact.total_dpos_votes_count,
            fact.vote_type,
            fact.committee_size,
            fact.number_of_proposers,
        )
        .expect("invalid PBFT vote type rejected before threshold calculation");
        let Some(two_t_plus_one) = sortition_threshold
            .checked_mul(2)
            .map(|threshold| threshold / 3 + 1)
        else {
            return PbftTwoTPlusOneThresholdPlan::missing(
                PbftTwoTPlusOneThresholdStatus::CalculationOverflow,
                "PBFT_TWO_T_PLUS_ONE_CALCULATION_OVERFLOW",
            );
        };
        let should_cache = fact.pbft_period == fact.current_pbft_chain_size;
        if should_cache {
            self.current_thresholds.insert(
                fact.vote_type,
                (fact.pbft_period, two_t_plus_one, sortition_threshold),
            );
        }

        PbftTwoTPlusOneThresholdPlan::available_with_sortition(
            two_t_plus_one,
            sortition_threshold,
            false,
            should_cache,
        )
    }

    /// Returns the number of vote-type entries retained by the live cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.current_thresholds.len()
    }

    /// Returns true when no threshold is currently cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.current_thresholds.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_cert_fact(has_total: bool, total: u64) -> PbftTwoTPlusOneThresholdFact {
        PbftTwoTPlusOneThresholdFact {
            pbft_period: 7,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: 7,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count: has_total,
            total_dpos_votes_count: total,
            future_dpos_state: false,
            unknown_error: false,
        }
    }

    #[test]
    fn asks_for_dpos_total_on_cache_miss() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let plan = runtime.plan_threshold(current_cert_fact(false, 0));

        assert_eq!(plan.status, PbftTwoTPlusOneThresholdStatus::NeedsDposTotal);
        assert!(plan.needs_total_dpos_votes);
        assert!(!plan.has_threshold);
        assert!(runtime.is_empty());
    }

    #[test]
    fn computes_and_caches_current_period_threshold() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let computed = runtime.plan_threshold(current_cert_fact(true, 90));

        assert_eq!(computed.status, PbftTwoTPlusOneThresholdStatus::Available);
        assert_eq!(computed.threshold, 61);
        assert!(computed.cached);
        assert!(!computed.cache_hit);
        assert_eq!(runtime.len(), 1);

        let cached = runtime.plan_threshold(current_cert_fact(false, 0));
        assert_eq!(cached.threshold, 61);
        assert!(cached.cache_hit);
        assert!(!cached.needs_total_dpos_votes);
    }

    #[test]
    fn does_not_cache_non_current_period_threshold() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let mut fact = current_cert_fact(true, 90);
        fact.pbft_period = 6;

        let computed = runtime.plan_threshold(fact);
        assert_eq!(computed.threshold, 61);
        assert!(!computed.cached);
        assert!(runtime.is_empty());
    }

    #[test]
    fn proposal_threshold_uses_number_of_proposers() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let mut fact = current_cert_fact(true, 100);
        fact.vote_type = PbftVoteType::Propose;

        let computed = runtime.plan_threshold(fact);
        assert_eq!(computed.threshold, 14);
    }

    #[test]
    fn explicit_failure_facts_are_not_cached() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let mut future = current_cert_fact(false, 0);
        future.future_dpos_state = true;
        let future_plan = runtime.plan_threshold(future);
        assert_eq!(
            future_plan.status,
            PbftTwoTPlusOneThresholdStatus::FutureDposState
        );

        let mut unknown = current_cert_fact(false, 0);
        unknown.unknown_error = true;
        let unknown_plan = runtime.plan_threshold(unknown);
        assert_eq!(
            unknown_plan.status,
            PbftTwoTPlusOneThresholdStatus::UnknownError
        );
        assert!(runtime.is_empty());
    }

    #[test]
    fn invalid_vote_type_is_rejected_without_cache_update() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let mut fact = current_cert_fact(true, 100);
        fact.vote_type = PbftVoteType::Invalid;

        let plan = runtime.plan_threshold(fact);
        assert_eq!(plan.status, PbftTwoTPlusOneThresholdStatus::InvalidVoteType);
        assert!(runtime.is_empty());
    }

    #[test]
    fn zero_total_dpos_still_has_legacy_threshold_floor() {
        let mut runtime = PbftTwoTPlusOneThresholdRuntime::new();
        let computed = runtime.plan_threshold(current_cert_fact(true, 0));

        assert_eq!(computed.sortition_threshold, 0);
        assert_eq!(computed.threshold, 1);
    }
}
