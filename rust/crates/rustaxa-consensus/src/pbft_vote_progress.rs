//! Deterministic PBFT vote-progress protocol planning.
//!
//! This module models the business-logic boundary for the future PBFT
//! vote-progress pipeline. The planner receives one vote event's compact facts
//! plus an explicit context and returns a protocol plan. It does not own
//! long-lived vote state, mutate [`VerifiedVotes`], write storage, send network
//! messages, or spawn async work. Callers execute returned intents at the
//! boundary and may feed the resulting reports back into the planner.
//!
//! The current planner is intentionally staged because verified-vote insertion
//! is a real state mutation. A first call with no insert report returns a
//! `PendingVerifiedVoteInsert` plan carrying only insert-safe intents. After
//! the executor applies that intent to the Rust-backed verified-vote index, a
//! second call with the [`AddVerifiedVoteOutcome`] turns the mutation report
//! into durable reward-vote, slashing, gossip, and PBFT-progress intents.
//!
//! [`InsertVerifiedVote`]: PbftVoteProgressIntent::InsertVerifiedVote
//! [`VerifiedVotes`]: crate::verified_votes::VerifiedVotes

use ethereum_types::{H160, H256};

use crate::consensus_pipeline::{ConsensusEffect, ConsensusPlan, EventOrigin, PipelineKind};
use crate::verified_votes::{
    AddVerifiedVoteOutcome, PbftVoteType, ThresholdDecisionOutcome, VerifiedVote,
};

/// Stable identity for one PBFT vote.
///
/// This is a compact consensus identity, not a network packet identity. Future
/// ingress stages may derive it from canonical arena bytes without
/// materializing a C++ `PbftVote`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteIdentity {
    /// Canonical hash of the vote.
    pub vote_hash: H256,
    /// Hash of the PBFT block or null block value targeted by the vote.
    pub block_hash: H256,
    /// PBFT period carried by the vote.
    pub period: u64,
    /// PBFT round carried by the vote.
    pub round: u64,
    /// PBFT step carried by the vote.
    pub step: u64,
    /// Recovered voter address.
    pub voter: H160,
}

/// Plain fact bundle for one PBFT vote-progress planning pass.
///
/// All fields are caller-supplied facts. The planner does not recover
/// signatures, read FinalChain, calculate DPoS eligibility, or inspect network
/// packet objects in this slice.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteProgressFact {
    /// Vote identity values.
    pub identity: PbftVoteIdentity,
    /// PBFT vote type.
    pub vote_type: PbftVoteType,
    /// DPoS vote weight supplied by validation.
    pub weight: u64,
    /// Whether the vote hash is already known to the peer/ingress layer.
    pub vote_already_known: bool,
    /// Whether ingress carried a proposed-block sidecar for a proposal vote.
    pub carries_proposed_block: bool,
    /// Whether validation accepted an old vote as an extra reward vote.
    pub valid_stale_reward_vote: bool,
}

/// Borrow-free context required by the PBFT vote-progress planner.
///
/// This is intentionally limited to scalar facts so it remains easy to supply
/// from C++ shims, future ring-buffer stages, or unit tests. Long-lived
/// verified-vote state stays outside the planner.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteProgressContext {
    /// Current local PBFT period.
    pub current_period: u64,
    /// Maximum accepted future period skew for this planning pass.
    pub max_future_period_delta: u64,
    /// Optional 2t+1 threshold. `None` means threshold effects are not planned.
    pub two_t_plus_one_threshold: Option<u64>,
    /// If true, proposal votes must carry proposed-block sidecar bytes.
    pub require_proposed_block_sidecar: bool,
    /// If true, duplicate-vote conflicts should emit a slashing intent.
    pub slashing_enabled: bool,
}

/// Deterministic status for one PBFT vote-progress plan.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteProgressStatus {
    /// Input passed prechecks and the executor should insert the verified vote.
    PendingVerifiedVoteInsert,
    /// Verified-vote insertion report was accepted with no threshold progress.
    Accepted,
    /// Verified-vote insertion report was accepted and threshold progress fired.
    AcceptedWithProgress,
    /// Vote was already known in the ingress or peer-known layer.
    AlreadyKnown,
    /// Verified-vote insertion reported the same vote hash already present.
    DuplicateVerifiedVote,
    /// Vote period is below current period and not a valid extra reward vote.
    RejectedStalePeriod,
    /// Vote period is above the allowed future skew.
    RejectedFuturePeriod,
    /// Input or executor report was invalid.
    RejectedInvalidVote,
    /// Proposal sidecar was required but absent.
    MissingProposedBlockSidecar,
    /// Duplicate voter-slot conflict was reported for slashing.
    ConflictingVote,
    /// Executor report did not match the vote transition being completed.
    RejectedExecutorReport,
}

impl PbftVoteProgressStatus {
    /// Stable numeric status for future bridge payloads and tests.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::PendingVerifiedVoteInsert => 0,
            Self::Accepted => 1,
            Self::AcceptedWithProgress => 2,
            Self::AlreadyKnown => 3,
            Self::DuplicateVerifiedVote => 4,
            Self::RejectedStalePeriod => 5,
            Self::RejectedFuturePeriod => 6,
            Self::RejectedInvalidVote => 7,
            Self::MissingProposedBlockSidecar => 8,
            Self::ConflictingVote => 9,
            Self::RejectedExecutorReport => 10,
        }
    }
}

/// Domain-specific intent emitted by the PBFT vote-progress planner.
///
/// These intents stay typed in Rust so executors receive the identity needed to
/// apply each effect. Future CXX bridge payloads can flatten them into
/// `has_*` booleans and plain hash/address fields.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PbftVoteProgressIntent {
    /// Mark this vote hash as known to the source peer.
    MarkKnown {
        /// Vote hash to mark.
        vote_hash: H256,
    },
    /// Insert one verified vote through the authoritative verified-vote index.
    InsertVerifiedVote {
        /// Vote payload to insert.
        vote: VerifiedVote,
        /// Threshold to apply with insertion, when available.
        two_t_plus_one_threshold: Option<u64>,
    },
    /// Request or retain proposed-block sidecar bytes before admitting a vote.
    RequestProposedBlockSidecar {
        /// PBFT block hash whose sidecar is required.
        block_hash: H256,
        /// Vote period that needs the sidecar.
        period: u64,
    },
    /// Persist this accepted stale cert vote as an extra reward vote.
    PersistExtraRewardVote {
        /// Vote hash to persist in the extra-reward-vote set.
        vote_hash: H256,
    },
    /// Submit duplicate-vote evidence through the slashing pipeline.
    ReportSlashing {
        /// Incoming vote hash that conflicted.
        incoming_vote_hash: H256,
        /// Existing conflicting vote hash selected by verified-vote insertion.
        conflicting_vote_hash: H256,
    },
    /// Gossip or rebroadcast this accepted vote.
    GossipVote {
        /// Vote hash to gossip.
        vote_hash: H256,
    },
    /// Drive PBFT progress checks after t+1 or 2t+1 threshold progress.
    DrivePbftProgress {
        /// PBFT period whose vote state changed.
        period: u64,
        /// PBFT round whose vote state changed.
        round: u64,
    },
}

/// Deterministic plan/output of one PBFT vote-progress planning pass.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteProgressPlan {
    /// Primary status.
    pub status: PbftVoteProgressStatus,
    /// Ordered protocol intents for boundary executors.
    pub intents: Vec<PbftVoteProgressIntent>,
    /// Verified-vote executor report consumed by this plan, when present.
    pub add_vote_outcome: Option<AddVerifiedVoteOutcome>,
    /// Threshold-driven protocol decision from the executor report, when any.
    pub threshold_decision: Option<ThresholdDecisionOutcome>,
    /// Conflicting vote hash for slashing review, when any.
    pub conflicting_vote_hash: Option<H256>,
}

impl PbftVoteProgressPlan {
    fn terminal(status: PbftVoteProgressStatus, intents: Vec<PbftVoteProgressIntent>) -> Self {
        Self {
            status,
            intents,
            add_vote_outcome: None,
            threshold_decision: None,
            conflicting_vote_hash: None,
        }
    }

    /// Projects this domain-specific plan into the provisional generic
    /// consensus pipeline envelope.
    ///
    /// The generic [`ConsensusPlan`] intentionally carries broad effects
    /// without PBFT payloads. Boundary executors should use `intents` for
    /// authoritative payloads and this projection only for pipeline-level
    /// scheduling/tests until production routing proves the final shape.
    #[must_use]
    pub fn to_consensus_plan(&self, origin: EventOrigin) -> ConsensusPlan {
        let effects = self
            .intents
            .iter()
            .map(|intent| match intent {
                PbftVoteProgressIntent::MarkKnown { .. } => ConsensusEffect::MarkKnown,
                PbftVoteProgressIntent::InsertVerifiedVote { .. } => ConsensusEffect::Admit,
                PbftVoteProgressIntent::RequestProposedBlockSidecar { .. } => {
                    ConsensusEffect::RequestSync
                }
                PbftVoteProgressIntent::PersistExtraRewardVote { .. } => ConsensusEffect::Admit,
                PbftVoteProgressIntent::ReportSlashing { .. } => ConsensusEffect::ReportMalicious,
                PbftVoteProgressIntent::GossipVote { .. } => ConsensusEffect::Gossip,
                PbftVoteProgressIntent::DrivePbftProgress { .. } => {
                    ConsensusEffect::DrivePbftProgress
                }
            })
            .collect();

        ConsensusPlan::new(PipelineKind::PbftVoteProgress, origin, effects)
    }

    /// Returns whether this plan contains an intent matching `predicate`.
    #[must_use]
    pub fn contains_intent(&self, predicate: impl Fn(&PbftVoteProgressIntent) -> bool) -> bool {
        self.intents.iter().any(predicate)
    }
}

/// Builds a deterministic, side-effect-free PBFT vote-progress plan.
///
/// Inputs:
/// - `fact`: compact facts for the vote being processed.
/// - `context`: scalar state view for the current planning pass.
/// - `add_vote_report`: optional report from applying a previously returned
///   verified-vote insertion intent.
///
/// Outputs:
/// - A terminal reject/known plan, a pending verified-vote insertion plan, or a
///   post-insertion plan with slashing/reward/gossip/progress intents.
///
/// Invariants and edge behavior:
/// - The planner never mutates the verified-vote index itself.
/// - Zero-weight votes are rejected before an insertion intent is emitted.
/// - Old votes are rejected unless validation marked them as extra reward
///   votes.
/// - Proposal votes can require proposed-block sidecar bytes.
/// - Insert reports that contain conflicts become slashing plans when slashing
///   reporting is enabled in the context.
#[must_use]
pub fn plan_pbft_vote_progress(
    fact: PbftVoteProgressFact,
    context: PbftVoteProgressContext,
    add_vote_report: Option<AddVerifiedVoteOutcome>,
) -> PbftVoteProgressPlan {
    if fact.vote_already_known {
        return PbftVoteProgressPlan::terminal(
            PbftVoteProgressStatus::AlreadyKnown,
            vec![PbftVoteProgressIntent::MarkKnown {
                vote_hash: fact.identity.vote_hash,
            }],
        );
    }

    let is_stale = fact.identity.period < context.current_period;
    if is_stale && !fact.valid_stale_reward_vote {
        return PbftVoteProgressPlan::terminal(
            PbftVoteProgressStatus::RejectedStalePeriod,
            Vec::new(),
        );
    }

    let future_limit = context
        .current_period
        .saturating_add(context.max_future_period_delta);
    if fact.identity.period > future_limit {
        return PbftVoteProgressPlan::terminal(
            PbftVoteProgressStatus::RejectedFuturePeriod,
            Vec::new(),
        );
    }

    if context.require_proposed_block_sidecar
        && fact.vote_type == PbftVoteType::Propose
        && !fact.carries_proposed_block
    {
        return PbftVoteProgressPlan::terminal(
            PbftVoteProgressStatus::MissingProposedBlockSidecar,
            vec![PbftVoteProgressIntent::RequestProposedBlockSidecar {
                block_hash: fact.identity.block_hash,
                period: fact.identity.period,
            }],
        );
    }

    let vote = match VerifiedVote::new(
        fact.identity.vote_hash,
        fact.identity.block_hash,
        fact.identity.voter,
        fact.identity.period,
        fact.identity.round,
        fact.identity.step,
        fact.vote_type,
        fact.weight,
    ) {
        Ok(vote) => vote,
        Err(_) => {
            return PbftVoteProgressPlan::terminal(
                PbftVoteProgressStatus::RejectedInvalidVote,
                Vec::new(),
            );
        }
    };

    let Some(add_vote_outcome) = add_vote_report else {
        let intents = vec![
            PbftVoteProgressIntent::MarkKnown {
                vote_hash: fact.identity.vote_hash,
            },
            PbftVoteProgressIntent::InsertVerifiedVote {
                vote,
                two_t_plus_one_threshold: context.two_t_plus_one_threshold,
            },
        ];
        return PbftVoteProgressPlan::terminal(
            PbftVoteProgressStatus::PendingVerifiedVoteInsert,
            intents,
        );
    };

    plan_from_add_vote_outcome(fact, context, add_vote_outcome)
}

fn plan_from_add_vote_outcome(
    fact: PbftVoteProgressFact,
    context: PbftVoteProgressContext,
    add_vote_outcome: AddVerifiedVoteOutcome,
) -> PbftVoteProgressPlan {
    if let Some(conflicting_vote_hash) = add_vote_outcome.conflicting_vote_hash {
        let mut intents = vec![PbftVoteProgressIntent::MarkKnown {
            vote_hash: fact.identity.vote_hash,
        }];
        if context.slashing_enabled {
            intents.push(PbftVoteProgressIntent::ReportSlashing {
                incoming_vote_hash: fact.identity.vote_hash,
                conflicting_vote_hash,
            });
        }

        return PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::ConflictingVote,
            intents,
            add_vote_outcome: Some(add_vote_outcome),
            threshold_decision: add_vote_outcome.threshold_decision,
            conflicting_vote_hash: Some(conflicting_vote_hash),
        };
    }

    if add_vote_outcome.duplicate_vote_hash {
        return PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::DuplicateVerifiedVote,
            intents: vec![PbftVoteProgressIntent::MarkKnown {
                vote_hash: fact.identity.vote_hash,
            }],
            add_vote_outcome: Some(add_vote_outcome),
            threshold_decision: add_vote_outcome.threshold_decision,
            conflicting_vote_hash: None,
        };
    }

    if !add_vote_outcome.inserted {
        return PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::RejectedInvalidVote,
            intents: Vec::new(),
            add_vote_outcome: Some(add_vote_outcome),
            threshold_decision: add_vote_outcome.threshold_decision,
            conflicting_vote_hash: None,
        };
    }

    let drive_progress = add_vote_outcome
        .threshold_decision
        .is_some_and(|decision| decision.two_t_plus_one_reached || decision.t_plus_one_reached);
    let mut intents = vec![
        PbftVoteProgressIntent::MarkKnown {
            vote_hash: fact.identity.vote_hash,
        },
        PbftVoteProgressIntent::GossipVote {
            vote_hash: fact.identity.vote_hash,
        },
    ];
    if fact.valid_stale_reward_vote {
        intents.push(PbftVoteProgressIntent::PersistExtraRewardVote {
            vote_hash: fact.identity.vote_hash,
        });
    }
    if drive_progress {
        intents.push(PbftVoteProgressIntent::DrivePbftProgress {
            period: fact.identity.period,
            round: fact.identity.round,
        });
    }

    PbftVoteProgressPlan {
        status: if drive_progress {
            PbftVoteProgressStatus::AcceptedWithProgress
        } else {
            PbftVoteProgressStatus::Accepted
        },
        intents,
        add_vote_outcome: Some(add_vote_outcome),
        threshold_decision: add_vote_outcome.threshold_decision,
        conflicting_vote_hash: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus_pipeline::{ConsensusEffect, EventOrigin, IngressPayloadRef};
    use crate::verified_votes::{TwoTPlusOneInsertOutcome, TwoTPlusOneVotedBlockType};

    fn h256(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn h160(v: u64) -> H160 {
        H160::from_low_u64_be(v)
    }

    fn identity(
        vote_hash: u64,
        block_hash: u64,
        period: u64,
        round: u64,
        step: u64,
        voter: u64,
    ) -> PbftVoteIdentity {
        PbftVoteIdentity {
            vote_hash: h256(vote_hash),
            block_hash: h256(block_hash),
            period,
            round,
            step,
            voter: h160(voter),
        }
    }

    fn fact(
        identity: PbftVoteIdentity,
        vote_type: PbftVoteType,
        weight: u64,
    ) -> PbftVoteProgressFact {
        PbftVoteProgressFact {
            identity,
            vote_type,
            weight,
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    const fn context() -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 10,
            max_future_period_delta: 1,
            two_t_plus_one_threshold: Some(2),
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    const fn add_outcome(
        inserted: bool,
        duplicate_vote_hash: bool,
        conflicting_vote_hash: Option<H256>,
        threshold_decision: Option<ThresholdDecisionOutcome>,
    ) -> AddVerifiedVoteOutcome {
        AddVerifiedVoteOutcome {
            inserted,
            total_weight: 1,
            votes_count: 1,
            conflicting_vote_hash,
            used_secondary_slot: false,
            duplicate_vote_hash,
            threshold_decision,
        }
    }

    #[test]
    fn status_codes_are_stable_for_bridge_payloads() {
        assert_eq!(PbftVoteProgressStatus::PendingVerifiedVoteInsert.as_u8(), 0);
        assert_eq!(PbftVoteProgressStatus::Accepted.as_u8(), 1);
        assert_eq!(PbftVoteProgressStatus::AcceptedWithProgress.as_u8(), 2);
        assert_eq!(PbftVoteProgressStatus::AlreadyKnown.as_u8(), 3);
        assert_eq!(PbftVoteProgressStatus::DuplicateVerifiedVote.as_u8(), 4);
        assert_eq!(PbftVoteProgressStatus::RejectedStalePeriod.as_u8(), 5);
        assert_eq!(PbftVoteProgressStatus::RejectedFuturePeriod.as_u8(), 6);
        assert_eq!(PbftVoteProgressStatus::RejectedInvalidVote.as_u8(), 7);
        assert_eq!(
            PbftVoteProgressStatus::MissingProposedBlockSidecar.as_u8(),
            8
        );
        assert_eq!(PbftVoteProgressStatus::ConflictingVote.as_u8(), 9);
        assert_eq!(PbftVoteProgressStatus::RejectedExecutorReport.as_u8(), 10);
    }

    #[test]
    fn stale_vote_is_rejected_by_period() {
        let plan = plan_pbft_vote_progress(
            fact(identity(1, 2, 9, 1, 1, 11), PbftVoteType::Cert, 1),
            context(),
            None,
        );

        assert_eq!(plan.status, PbftVoteProgressStatus::RejectedStalePeriod);
        assert!(plan.intents.is_empty());
    }

    #[test]
    fn validated_stale_reward_vote_precheck_defers_persistence_until_insert_succeeds() {
        let mut facts = fact(identity(2, 3, 9, 1, 3, 11), PbftVoteType::Cert, 1);
        facts.valid_stale_reward_vote = true;

        let plan = plan_pbft_vote_progress(facts, context(), None);

        assert_eq!(
            plan.status,
            PbftVoteProgressStatus::PendingVerifiedVoteInsert
        );
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::InsertVerifiedVote { .. }
        )));
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::PersistExtraRewardVote { .. }
        )));

        let post_insert =
            plan_pbft_vote_progress(facts, context(), Some(add_outcome(true, false, None, None)));
        assert!(post_insert.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::PersistExtraRewardVote { vote_hash }
                if *vote_hash == facts.identity.vote_hash
        )));
    }

    #[test]
    fn future_vote_is_rejected_by_skew_limit() {
        let plan = plan_pbft_vote_progress(
            fact(identity(3, 4, 13, 1, 1, 11), PbftVoteType::Cert, 1),
            context(),
            None,
        );

        assert_eq!(plan.status, PbftVoteProgressStatus::RejectedFuturePeriod);
        assert!(plan.intents.is_empty());
    }

    #[test]
    fn known_vote_short_circuits_plan_without_insert_intent() {
        let mut facts = fact(identity(5, 6, 10, 1, 1, 11), PbftVoteType::Cert, 1);
        facts.vote_already_known = true;

        let plan = plan_pbft_vote_progress(facts, context(), None);

        assert_eq!(plan.status, PbftVoteProgressStatus::AlreadyKnown);
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::MarkKnown { vote_hash }
                if *vote_hash == facts.identity.vote_hash
        )));
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::InsertVerifiedVote { .. }
        )));
    }

    #[test]
    fn validation_rejects_zero_weight_votes_before_insert_intent() {
        let plan = plan_pbft_vote_progress(
            fact(identity(7, 8, 10, 1, 1, 11), PbftVoteType::Cert, 0),
            context(),
            None,
        );

        assert_eq!(plan.status, PbftVoteProgressStatus::RejectedInvalidVote);
        assert!(plan.intents.is_empty());
    }

    #[test]
    fn valid_vote_precheck_returns_verified_vote_insert_intent() {
        let facts = fact(identity(9, 10, 10, 1, 1, 11), PbftVoteType::Cert, 1);

        let plan = plan_pbft_vote_progress(facts, context(), None);

        assert_eq!(
            plan.status,
            PbftVoteProgressStatus::PendingVerifiedVoteInsert
        );
        assert_eq!(plan.intents.len(), 2);
        assert!(matches!(
            &plan.intents[0],
            PbftVoteProgressIntent::MarkKnown { vote_hash }
                if *vote_hash == facts.identity.vote_hash
        ));
        assert!(matches!(
            &plan.intents[1],
            PbftVoteProgressIntent::InsertVerifiedVote { vote, two_t_plus_one_threshold }
                if vote.vote_hash == facts.identity.vote_hash && *two_t_plus_one_threshold == Some(2)
        ));
    }

    #[test]
    fn accepted_insert_report_emits_gossip_without_progress() {
        let facts = fact(identity(10, 11, 10, 1, 1, 11), PbftVoteType::Cert, 1);
        let report = add_outcome(true, false, None, None);

        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::Accepted);
        assert!(
            plan.contains_intent(|intent| matches!(
                intent,
                PbftVoteProgressIntent::MarkKnown { .. }
            ))
        );
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::GossipVote { vote_hash }
                if *vote_hash == facts.identity.vote_hash
        )));
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::DrivePbftProgress { .. }
        )));
    }

    #[test]
    fn missing_proposed_block_sidecar_requests_sidecar_without_marking_known() {
        let context = PbftVoteProgressContext {
            require_proposed_block_sidecar: true,
            ..context()
        };
        let mut facts = fact(identity(11, 12, 10, 1, 1, 11), PbftVoteType::Propose, 1);
        facts.carries_proposed_block = false;

        let plan = plan_pbft_vote_progress(facts, context, None);

        assert_eq!(
            plan.status,
            PbftVoteProgressStatus::MissingProposedBlockSidecar
        );
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::RequestProposedBlockSidecar { block_hash, period }
                if *block_hash == facts.identity.block_hash && *period == facts.identity.period
        )));
        assert!(
            !plan.contains_intent(|intent| matches!(
                intent,
                PbftVoteProgressIntent::MarkKnown { .. }
            ))
        );
        assert!(plan.threshold_decision.is_none());
    }

    #[test]
    fn duplicate_conflicting_vote_report_emits_slashing_plan() {
        let facts = fact(identity(13, 14, 10, 2, 5, 11), PbftVoteType::Cert, 1);
        let report = add_outcome(false, false, Some(h256(15)), None);

        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::ConflictingVote);
        assert_eq!(plan.conflicting_vote_hash, Some(h256(15)));
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::ReportSlashing { incoming_vote_hash, conflicting_vote_hash }
                if *incoming_vote_hash == facts.identity.vote_hash && *conflicting_vote_hash == h256(15)
        )));
    }

    #[test]
    fn conflicting_vote_without_slashing_enabled_keeps_conflict_status_without_report_intent() {
        let context = PbftVoteProgressContext {
            slashing_enabled: false,
            ..context()
        };
        let facts = fact(identity(14, 14, 10, 2, 5, 11), PbftVoteType::Cert, 1);
        let report = add_outcome(false, false, Some(h256(15)), None);

        let plan = plan_pbft_vote_progress(facts, context, Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::ConflictingVote);
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::ReportSlashing { .. }
        )));
    }

    #[test]
    fn duplicate_vote_hash_report_maps_to_duplicate_status() {
        let facts = fact(identity(16, 14, 10, 2, 5, 11), PbftVoteType::Cert, 1);
        let report = add_outcome(false, true, None, None);

        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::DuplicateVerifiedVote);
        assert!(
            plan.contains_intent(|intent| matches!(
                intent,
                PbftVoteProgressIntent::MarkKnown { .. }
            ))
        );
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::ReportSlashing { .. }
        )));
    }

    #[test]
    fn secondary_next_vote_insert_report_is_accepted_without_slashing() {
        let facts = fact(identity(18, 0, 10, 2, 5, 11), PbftVoteType::Next, 1);
        let mut report = add_outcome(true, false, None, None);
        report.used_secondary_slot = true;

        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::Accepted);
        assert!(matches!(
            plan.add_vote_outcome,
            Some(outcome) if outcome.used_secondary_slot
        ));
        assert!(!plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::ReportSlashing { .. }
        )));
    }

    #[test]
    fn threshold_progress_creates_drive_progress_intent() {
        let facts = fact(identity(17, 18, 10, 2, 5, 11), PbftVoteType::Cert, 2);
        let threshold = ThresholdDecisionOutcome {
            t_plus_one_reached: false,
            network_t_plus_one_step_updated: false,
            two_t_plus_one_reached: true,
            two_t_plus_one_kind: Some(TwoTPlusOneVotedBlockType::CertVotedBlock),
            two_t_plus_one_insert_outcome: Some(TwoTPlusOneInsertOutcome {
                round_found: true,
                inserted: true,
            }),
        };
        let report = add_outcome(true, false, None, Some(threshold));

        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        assert_eq!(plan.status, PbftVoteProgressStatus::AcceptedWithProgress);
        assert!(plan.contains_intent(|intent| matches!(
            intent,
            PbftVoteProgressIntent::DrivePbftProgress { period, round }
                if *period == facts.identity.period && *round == facts.identity.round
        )));
        assert!(
            plan.threshold_decision
                .expect("threshold decision")
                .two_t_plus_one_reached
        );
    }

    #[test]
    fn domain_plan_projects_to_generic_consensus_plan_in_intent_order() {
        let facts = fact(identity(19, 20, 10, 2, 5, 11), PbftVoteType::Cert, 1);
        let report = add_outcome(true, false, None, None);
        let plan = plan_pbft_vote_progress(facts, context(), Some(report));

        let consensus_plan =
            plan.to_consensus_plan(EventOrigin::IngressPayload(IngressPayloadRef(77)));

        assert_eq!(consensus_plan.pipeline, PipelineKind::PbftVoteProgress);
        assert_eq!(
            consensus_plan.effects,
            vec![ConsensusEffect::MarkKnown, ConsensusEffect::Gossip]
        );
    }
}
