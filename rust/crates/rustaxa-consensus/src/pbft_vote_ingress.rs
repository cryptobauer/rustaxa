//! PBFT vote ingress planning for packet-adjacent consensus routing.
//!
//! This module owns deterministic ingress gating for PBFT vote packets before
//! the authoritative vote admission runtime mutates verified-vote state. It
//! receives compact vote facts plus scalar local PBFT/network-window context
//! and returns a side-effect-free decision. Callers still decode packets,
//! maintain peer timing, send sync requests, disconnect peers, admit votes, and
//! execute gossip or proposed-block effects at the boundary.

use crate::verified_votes::PbftVoteType;

/// Compact PBFT vote facts required for ingress relevance checks.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteIngressFact {
    /// PBFT period carried by the vote.
    pub period: u64,
    /// PBFT round carried by the vote.
    pub round: u64,
    /// PBFT step carried by the vote.
    pub step: u64,
    /// PBFT vote type derived from the vote step.
    pub vote_type: PbftVoteType,
}

/// Scalar local state and network-window policy for one ingress decision.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteIngressContext {
    /// Current local PBFT period.
    pub current_period: u64,
    /// Current local PBFT round.
    pub current_round: u64,
    /// Current local PBFT step.
    pub current_step: u64,
    /// Accepted future-period window from network DDoS protection settings.
    /// A value of zero disables the upper-period check, matching legacy C++.
    pub max_future_period_delta: u64,
    /// Accepted future-round window. A value of zero disables the upper-round check.
    pub max_future_round_delta: u64,
    /// Accepted future-step window. A value of zero disables the upper-step check.
    pub max_future_step_delta: u64,
    /// Whether this route should enforce max round/step bounds.
    pub validate_max_round_step: bool,
    /// Whether the vote came from the same peer id as its recovered voter.
    pub source_peer_is_voter: bool,
    /// Whether the caller may emit a PBFT chain sync request now.
    pub can_request_pbft_sync: bool,
    /// Whether the caller may emit a next-votes sync request now.
    pub can_request_next_votes_sync: bool,
}

/// Stable PBFT vote ingress status for bridge payloads and tests.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteIngressStatus {
    /// The vote passed ingress gating and may enter admission.
    Accepted,
    /// The vote is outside the PBFT relevance window and should be dropped.
    Irrelevant,
    /// The vote period is below the accepted lower bound.
    InvalidPeriodTooSmall,
    /// The vote period is above the accepted future-period window.
    InvalidPeriodTooBig,
    /// The vote round is below the accepted lower bound.
    InvalidRoundTooSmall,
    /// The vote round is above the accepted future-round window.
    InvalidRoundTooBig,
    /// The vote step is above the accepted future-step window.
    InvalidStepTooBig,
    /// Vote bundles do not support propose votes.
    UnsupportedBundleProposeVote,
    /// A bundled vote does not match the bundle reference identity.
    BundleVoteMismatch,
}

impl PbftVoteIngressStatus {
    /// Stable numeric status for CXX bridge payloads.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::Irrelevant => 1,
            Self::InvalidPeriodTooSmall => 2,
            Self::InvalidPeriodTooBig => 3,
            Self::InvalidRoundTooSmall => 4,
            Self::InvalidRoundTooBig => 5,
            Self::InvalidStepTooBig => 6,
            Self::UnsupportedBundleProposeVote => 7,
            Self::BundleVoteMismatch => 8,
        }
    }
}

/// Side-effect-free PBFT vote ingress decision.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteIngressPlan {
    /// Primary status.
    pub status: PbftVoteIngressStatus,
    /// Whether the vote should continue to validation/admission.
    pub accepted: bool,
    /// Whether the vote is relevant to the local PBFT state.
    pub relevant: bool,
    /// Whether the C++ boundary should request PBFT chain sync.
    pub request_pbft_sync: bool,
    /// Whether the C++ boundary should request next-votes sync for the current round.
    pub request_next_votes_sync: bool,
    /// Round baseline used by the legacy-compatible round checks.
    pub checking_round: u64,
    /// Step baseline used by the legacy-compatible step checks.
    pub checking_step: u64,
}

impl PbftVoteIngressPlan {
    const fn terminal(
        status: PbftVoteIngressStatus,
        relevant: bool,
        request_pbft_sync: bool,
        request_next_votes_sync: bool,
        checking_round: u64,
        checking_step: u64,
    ) -> Self {
        Self {
            status,
            accepted: matches!(status, PbftVoteIngressStatus::Accepted),
            relevant,
            request_pbft_sync,
            request_next_votes_sync,
            checking_round,
            checking_step,
        }
    }
}

/// Plans ingress gating for one PBFT vote packet or bundle item.
#[must_use]
pub fn plan_pbft_vote_ingress(
    fact: PbftVoteIngressFact,
    context: PbftVoteIngressContext,
) -> PbftVoteIngressPlan {
    let relevant = is_relevant(fact, context);
    let checking_round = if context.current_period == fact.period {
        context.current_round
    } else {
        1
    };
    let checking_step =
        if context.current_period == fact.period && context.current_round == fact.round {
            context.current_step
        } else {
            1
        };

    if !relevant {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::Irrelevant,
            false,
            false,
            false,
            checking_round,
            checking_step,
        );
    }

    if fact.period == 0
        || fact.period < context.current_period.saturating_sub(1)
        || (fact.period == context.current_period.saturating_sub(1)
            && fact.vote_type != PbftVoteType::Cert)
    {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::InvalidPeriodTooSmall,
            relevant,
            false,
            false,
            checking_round,
            checking_step,
        );
    }

    if context.max_future_period_delta != 0
        && fact.period.saturating_sub(1)
            > context
                .current_period
                .saturating_add(context.max_future_period_delta)
    {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::InvalidPeriodTooBig,
            relevant,
            context.source_peer_is_voter && context.can_request_pbft_sync,
            false,
            checking_round,
            checking_step,
        );
    }

    if fact.round < checking_round.saturating_sub(1)
        || (fact.round == checking_round.saturating_sub(1) && fact.vote_type != PbftVoteType::Next)
    {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::InvalidRoundTooSmall,
            relevant,
            false,
            false,
            checking_round,
            checking_step,
        );
    }

    if context.validate_max_round_step
        && context.max_future_round_delta != 0
        && fact.round >= checking_round.saturating_add(context.max_future_round_delta)
    {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::InvalidRoundTooBig,
            relevant,
            false,
            context.current_period == fact.period
                && context.source_peer_is_voter
                && context.can_request_next_votes_sync,
            checking_round,
            checking_step,
        );
    }

    if context.validate_max_round_step
        && context.max_future_step_delta != 0
        && fact.step >= checking_step.saturating_add(context.max_future_step_delta)
    {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::InvalidStepTooBig,
            relevant,
            false,
            false,
            checking_round,
            checking_step,
        );
    }

    PbftVoteIngressPlan::terminal(
        PbftVoteIngressStatus::Accepted,
        relevant,
        false,
        false,
        checking_round,
        checking_step,
    )
}

/// Plans bundle-level PBFT vote ingress rules for one item against the first
/// vote's identity.
#[must_use]
pub fn plan_pbft_vote_bundle_ingress(
    reference: PbftVoteIngressFact,
    vote: PbftVoteIngressFact,
    context: PbftVoteIngressContext,
) -> PbftVoteIngressPlan {
    let base = plan_pbft_vote_ingress(reference, context);
    if !base.accepted {
        return base;
    }

    if reference.vote_type == PbftVoteType::Propose {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::UnsupportedBundleProposeVote,
            true,
            false,
            false,
            base.checking_round,
            base.checking_step,
        );
    }

    if reference != vote {
        return PbftVoteIngressPlan::terminal(
            PbftVoteIngressStatus::BundleVoteMismatch,
            true,
            false,
            false,
            base.checking_round,
            base.checking_step,
        );
    }

    base
}

fn is_relevant(fact: PbftVoteIngressFact, context: PbftVoteIngressContext) -> bool {
    if fact.period >= context.current_period && fact.round >= context.current_round {
        return true;
    }
    if fact.period == context.current_period
        && fact.round == context.current_round.saturating_sub(1)
        && fact.vote_type == PbftVoteType::Next
    {
        return true;
    }
    fact.period == context.current_period.saturating_sub(1) && fact.vote_type == PbftVoteType::Cert
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn fact(
        period: u64,
        round: u64,
        step: u64,
        vote_type: PbftVoteType,
    ) -> PbftVoteIngressFact {
        PbftVoteIngressFact {
            period,
            round,
            step,
            vote_type,
        }
    }

    const fn context() -> PbftVoteIngressContext {
        PbftVoteIngressContext {
            current_period: 10,
            current_round: 3,
            current_step: 2,
            max_future_period_delta: 2,
            max_future_round_delta: 3,
            max_future_step_delta: 4,
            validate_max_round_step: true,
            source_peer_is_voter: true,
            can_request_pbft_sync: true,
            can_request_next_votes_sync: true,
        }
    }

    #[test]
    fn accepts_current_and_previous_round_next_vote() {
        assert_eq!(
            plan_pbft_vote_ingress(fact(10, 3, 2, PbftVoteType::Soft), context()).status,
            PbftVoteIngressStatus::Accepted
        );
        assert_eq!(
            plan_pbft_vote_ingress(fact(10, 2, 4, PbftVoteType::Next), context()).status,
            PbftVoteIngressStatus::Accepted
        );
    }

    #[test]
    fn rejects_irrelevant_or_too_old_votes() {
        assert_eq!(
            plan_pbft_vote_ingress(fact(9, 3, 3, PbftVoteType::Soft), context()).status,
            PbftVoteIngressStatus::Irrelevant
        );
        assert_eq!(
            plan_pbft_vote_ingress(fact(8, 3, 3, PbftVoteType::Cert), context()).status,
            PbftVoteIngressStatus::Irrelevant
        );
    }

    #[test]
    fn plans_sync_requests_for_future_period_or_round_when_allowed() {
        let period_plan = plan_pbft_vote_ingress(fact(14, 3, 1, PbftVoteType::Soft), context());
        assert_eq!(
            period_plan.status,
            PbftVoteIngressStatus::InvalidPeriodTooBig
        );
        assert!(period_plan.request_pbft_sync);

        let round_plan = plan_pbft_vote_ingress(fact(10, 6, 1, PbftVoteType::Soft), context());
        assert_eq!(round_plan.status, PbftVoteIngressStatus::InvalidRoundTooBig);
        assert!(round_plan.request_next_votes_sync);
    }

    #[test]
    fn bundle_plan_rejects_propose_and_mismatched_items() {
        assert_eq!(
            plan_pbft_vote_bundle_ingress(
                fact(10, 3, 1, PbftVoteType::Propose),
                fact(10, 3, 1, PbftVoteType::Propose),
                context()
            )
            .status,
            PbftVoteIngressStatus::UnsupportedBundleProposeVote
        );
        assert_eq!(
            plan_pbft_vote_bundle_ingress(
                fact(10, 3, 2, PbftVoteType::Soft),
                fact(10, 3, 3, PbftVoteType::Soft),
                context()
            )
            .status,
            PbftVoteIngressStatus::BundleVoteMismatch
        );
    }
}
