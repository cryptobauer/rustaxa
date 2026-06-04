//! PBFT vote admission runtime session.
//!
//! This module composes the ingress-adjacent vote event fact boundary with the
//! existing PBFT vote pipeline session. It is the Rust-owned admission
//! orchestration point for one canonical PBFT vote after the caller has
//! supplied validation weight or a validation result. It does not read
//! FinalChain, mutate verified-vote state, write storage, submit slashing
//! proofs, send network messages, or own live C++ `PbftVote` sidecars.

use crate::pbft_vote_event::{
    PbftVoteEventFact, PbftVoteEventFactFlags, PbftVoteEventFactStatus, build_pbft_vote_event_fact,
    build_pbft_vote_event_fact_from_validation,
};
use crate::pbft_vote_pipeline::{
    PbftVotePipelineSession, PbftVotePipelineStatus, PbftVotePipelineStep,
    create_pbft_vote_pipeline_session,
};
use crate::pbft_vote_progress::{
    PbftVoteProgressContext, PbftVoteProgressFact, PbftVoteProgressPlan, PbftVoteProgressStatus,
};
use crate::pbft_vote_validation::PbftCanonicalVoteValidation;
use crate::verified_votes::AddVerifiedVoteOutcome;

/// Stable status for one PBFT vote admission session.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVoteAdmissionStatus {
    /// Event facts were derived and the pipeline session is ready.
    Ready,
    /// Event fact derivation or validation rejected the vote before insertion.
    EventRejected,
    /// The pipeline requested verified-vote insertion and awaits its report.
    AwaitingVerifiedVoteInsert,
    /// The admission session reached a terminal plan.
    Complete,
    /// The caller attempted a stage transition in the wrong order.
    InvalidStage,
}

impl PbftVoteAdmissionStatus {
    /// Stable numeric status used by bridge payloads.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::EventRejected => 1,
            Self::AwaitingVerifiedVoteInsert => 2,
            Self::Complete => 3,
            Self::InvalidStage => 4,
        }
    }
}

/// Pre-mutation admission output for C++ or future pipeline executors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteAdmissionPrecheck {
    /// Admission-stage status after the call.
    pub admission_status: PbftVoteAdmissionStatus,
    /// Canonical validation result when this admission was created from
    /// validation facts instead of a pre-weighted live sidecar.
    pub validation: Option<PbftCanonicalVoteValidation>,
    /// Event fact derivation status.
    pub event_status: PbftVoteEventFactStatus,
    /// Stable event/admission error code for bridge and log consumers.
    pub error_code: &'static str,
    /// Compact progress fact derived from canonical vote bytes or validation.
    pub progress_fact: Option<PbftVoteProgressFact>,
    /// Pipeline precheck output when a pipeline session was created.
    pub pipeline_step: Option<PbftVotePipelineStep>,
    /// Whether the admission session is terminal.
    pub complete: bool,
}

/// Post-mutation admission output for C++ or future pipeline executors.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteAdmissionExecution {
    /// Admission-stage status after the call.
    pub admission_status: PbftVoteAdmissionStatus,
    /// Terminal pipeline output when the report was accepted by the session.
    pub pipeline_step: PbftVotePipelineStep,
    /// Whether the admission session is terminal.
    pub complete: bool,
}

/// Runtime state for one PBFT vote admission.
#[derive(Debug, Clone)]
pub struct PbftVoteAdmissionSession {
    validation: Option<PbftCanonicalVoteValidation>,
    event_fact: PbftVoteEventFact,
    pipeline_session: Option<PbftVotePipelineSession>,
    stage: PbftVoteAdmissionStatus,
}

impl PbftVoteAdmissionSession {
    /// Creates an admission session from canonical vote RLP and an accepted
    /// validation weight.
    ///
    /// Inputs:
    /// - `canonical_vote_rlp`: legacy `PbftVote::rlp(true, false)` bytes.
    /// - `weight`: already-calculated non-zero validation weight.
    /// - `flags`: caller-supplied ingress and validation flags.
    /// - `context`: scalar state view for the vote-progress pipeline.
    ///
    /// Outputs:
    /// - A session that either owns a ready vote pipeline or a terminal event
    ///   rejection. Peer-controlled malformed bytes are represented as
    ///   statuses, not errors.
    pub fn from_canonical_vote(
        canonical_vote_rlp: &[u8],
        weight: u64,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
    ) -> anyhow::Result<Self> {
        let event_fact = build_pbft_vote_event_fact(canonical_vote_rlp, weight, flags)?;
        Ok(Self::from_event_fact(event_fact, context))
    }

    /// Creates an admission session from a canonical validation result.
    ///
    /// Inputs:
    /// - `validation`: Rust canonical vote validation output.
    /// - `flags`: caller-supplied ingress and validation flags.
    /// - `context`: scalar state view for the vote-progress pipeline.
    ///
    /// Outputs:
    /// - A ready session only when validation accepted and returned a non-zero
    ///   calculated weight. Pending/rejected validation becomes a terminal
    ///   event rejection.
    #[must_use]
    pub fn from_validation(
        validation: &PbftCanonicalVoteValidation,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
    ) -> Self {
        let event_fact = build_pbft_vote_event_fact_from_validation(validation, flags);
        Self::from_event_fact_with_validation(Some(validation.clone()), event_fact, context)
    }

    /// Creates an admission session from an already-derived event fact.
    ///
    /// This constructor is useful for tests and for future pipeline stages that
    /// cache event facts in an enrichment arena before admission.
    #[must_use]
    pub fn from_event_fact(
        event_fact: PbftVoteEventFact,
        context: PbftVoteProgressContext,
    ) -> Self {
        Self::from_event_fact_with_validation(None, event_fact, context)
    }

    /// Creates an admission session from an event fact and optional validation
    /// result.
    ///
    /// `validation` is retained only as transition metadata for bridge
    /// prechecks. The progress pipeline still operates on compact progress
    /// facts and remains side-effect-free.
    #[must_use]
    pub fn from_event_fact_with_validation(
        validation: Option<PbftCanonicalVoteValidation>,
        event_fact: PbftVoteEventFact,
        context: PbftVoteProgressContext,
    ) -> Self {
        let pipeline_session = event_fact
            .progress_fact
            .map(|fact| create_pbft_vote_pipeline_session(fact, context));
        let stage = if pipeline_session.is_some() {
            PbftVoteAdmissionStatus::Ready
        } else {
            PbftVoteAdmissionStatus::EventRejected
        };
        Self {
            validation,
            event_fact,
            pipeline_session,
            stage,
        }
    }

    /// Returns the current admission stage.
    #[must_use]
    pub const fn stage(&self) -> PbftVoteAdmissionStatus {
        self.stage
    }

    /// Returns the event fact owned by this admission session.
    #[must_use]
    pub const fn event_fact(&self) -> &PbftVoteEventFact {
        &self.event_fact
    }

    /// Returns the canonical validation result when the session was created
    /// through the validation-backed admission boundary.
    #[must_use]
    pub const fn validation(&self) -> Option<&PbftCanonicalVoteValidation> {
        self.validation.as_ref()
    }

    /// Returns the compact progress fact when event derivation succeeded.
    #[must_use]
    pub const fn progress_fact(&self) -> Option<&PbftVoteProgressFact> {
        self.event_fact.progress_fact.as_ref()
    }

    /// Produces the pre-insert admission plan.
    ///
    /// Outputs:
    /// - `AwaitingVerifiedVoteInsert` when the caller must execute one
    ///   verified-vote insertion mutation and report the outcome.
    /// - `Complete` for terminal reject/known plans.
    /// - `EventRejected` for event or validation rejections that never create
    ///   a pipeline session.
    pub fn precheck(&mut self) -> PbftVoteAdmissionPrecheck {
        if self.stage == PbftVoteAdmissionStatus::EventRejected {
            self.stage = PbftVoteAdmissionStatus::Complete;
            return PbftVoteAdmissionPrecheck {
                admission_status: PbftVoteAdmissionStatus::EventRejected,
                validation: self.validation.clone(),
                event_status: self.event_fact.status,
                error_code: self.event_fact.error_code,
                progress_fact: None,
                pipeline_step: None,
                complete: true,
            };
        }

        if self.stage != PbftVoteAdmissionStatus::Ready {
            return self.invalid_precheck();
        }

        let Some(session) = &mut self.pipeline_session else {
            return self.invalid_precheck();
        };

        let step = session.precheck();
        self.stage = match step.pipeline_status {
            PbftVotePipelineStatus::AwaitingVerifiedVoteInsert => {
                PbftVoteAdmissionStatus::AwaitingVerifiedVoteInsert
            }
            PbftVotePipelineStatus::Complete => PbftVoteAdmissionStatus::Complete,
            PbftVotePipelineStatus::InvalidStage => PbftVoteAdmissionStatus::InvalidStage,
            PbftVotePipelineStatus::Ready => PbftVoteAdmissionStatus::Ready,
        };

        PbftVoteAdmissionPrecheck {
            admission_status: self.stage,
            validation: self.validation.clone(),
            event_status: self.event_fact.status,
            error_code: if step.progress_plan.status == PbftVoteProgressStatus::RejectedInvalidVote
            {
                "PBFT_VOTE_ADMISSION_INVALID_PROGRESS_FACT"
            } else {
                self.event_fact.error_code
            },
            progress_fact: self.event_fact.progress_fact,
            complete: step.complete,
            pipeline_step: Some(step),
        }
    }

    /// Reports the verified-vote insertion outcome and returns the terminal
    /// admission execution plan.
    ///
    /// Reports are accepted only after precheck requested insertion. The
    /// session becomes complete after accepting one report.
    pub fn report_verified_vote_add(
        &mut self,
        add_vote_outcome: AddVerifiedVoteOutcome,
    ) -> PbftVoteAdmissionExecution {
        if self.stage != PbftVoteAdmissionStatus::AwaitingVerifiedVoteInsert {
            return PbftVoteAdmissionExecution {
                admission_status: PbftVoteAdmissionStatus::InvalidStage,
                pipeline_step: invalid_pipeline_step(),
                complete: true,
            };
        }

        let Some(session) = &mut self.pipeline_session else {
            return PbftVoteAdmissionExecution {
                admission_status: PbftVoteAdmissionStatus::InvalidStage,
                pipeline_step: invalid_pipeline_step(),
                complete: true,
            };
        };

        let step = session.report_verified_vote_add(add_vote_outcome);
        self.stage = PbftVoteAdmissionStatus::Complete;
        PbftVoteAdmissionExecution {
            admission_status: PbftVoteAdmissionStatus::Complete,
            complete: step.complete,
            pipeline_step: step,
        }
    }

    fn invalid_precheck(&self) -> PbftVoteAdmissionPrecheck {
        PbftVoteAdmissionPrecheck {
            admission_status: PbftVoteAdmissionStatus::InvalidStage,
            validation: self.validation.clone(),
            event_status: self.event_fact.status,
            error_code: "PBFT_VOTE_ADMISSION_INVALID_STAGE",
            progress_fact: self.event_fact.progress_fact,
            pipeline_step: Some(invalid_pipeline_step()),
            complete: true,
        }
    }
}

fn invalid_pipeline_step() -> PbftVotePipelineStep {
    PbftVotePipelineStep {
        pipeline_status: PbftVotePipelineStatus::InvalidStage,
        progress_plan: PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::RejectedExecutorReport,
            intents: Vec::new(),
            add_vote_outcome: None,
            threshold_decision: None,
            conflicting_vote_hash: None,
        },
        complete: true,
    }
}

/// Creates a PBFT vote admission session from canonical vote bytes and weight.
pub fn create_pbft_vote_admission_session(
    canonical_vote_rlp: &[u8],
    weight: u64,
    flags: PbftVoteEventFactFlags,
    context: PbftVoteProgressContext,
) -> anyhow::Result<PbftVoteAdmissionSession> {
    PbftVoteAdmissionSession::from_canonical_vote(canonical_vote_rlp, weight, flags, context)
}

/// Creates a PBFT vote admission session from a canonical validation result.
///
/// Inputs:
/// - `validation`: authoritative Rust validation output for the canonical vote.
/// - `flags`: ingress and stale-reward facts supplied by the caller.
/// - `context`: scalar state view for vote-progress planning.
///
/// Outputs:
/// - A validation-backed admission session that carries the validation result
///   through precheck and only creates a progress pipeline when validation
///   accepted with a non-zero calculated weight.
#[must_use]
pub fn create_pbft_vote_admission_session_from_validation(
    validation: &PbftCanonicalVoteValidation,
    flags: PbftVoteEventFactFlags,
    context: PbftVoteProgressContext,
) -> PbftVoteAdmissionSession {
    PbftVoteAdmissionSession::from_validation(validation, flags, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_validation::{PbftVoteValidationStatus, validate_canonical_pbft_vote};
    use crate::verified_votes::{PbftVoteType, ThresholdDecisionOutcome};
    use k256::ecdsa::SigningKey;
    use rustaxa_vdf::vrf;
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x51; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn signed_vote_rlp() -> Vec<u8> {
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: [7; 32].into(),
            vote_type: PbftVoteType::Cert,
            period: 12,
            round: 2,
            step: 3,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&NODE_SECRET).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert_eq!(generated.status, crate::PbftVoteGenerationStatus::Generated);
        generated.vote_rlp
    }

    const fn flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    const fn context() -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 12,
            current_round: 1,
            max_future_period_delta: 0,
            two_t_plus_one_threshold: Some(10),
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    const fn add_outcome(
        inserted: bool,
        threshold_decision: Option<ThresholdDecisionOutcome>,
    ) -> AddVerifiedVoteOutcome {
        AddVerifiedVoteOutcome {
            inserted,
            total_weight: 42,
            votes_count: 1,
            conflicting_vote_hash: None,
            used_secondary_slot: false,
            duplicate_vote_hash: false,
            threshold_decision,
        }
    }

    fn validation_facts() -> crate::PbftVoteValidationExternalFacts {
        crate::PbftVoteValidationExternalFacts {
            voter_dpos_ready: true,
            voter_dpos_vote_count: 42,
            total_dpos_ready: true,
            total_dpos_vote_count: 100,
            future_dpos_state: false,
            unknown_error: false,
            vrf_key_ready: true,
            has_vrf_key: true,
            vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            strict_vrf: true,
            committee_size: 100,
            number_of_proposers: 20,
        }
    }

    #[test]
    fn admission_session_derives_fact_then_accepts_executor_report() {
        let vote_rlp = signed_vote_rlp();
        let mut session =
            create_pbft_vote_admission_session(&vote_rlp, 42, flags(), context()).unwrap();

        let precheck = session.precheck();
        assert_eq!(
            precheck.admission_status,
            PbftVoteAdmissionStatus::AwaitingVerifiedVoteInsert
        );
        assert_eq!(precheck.event_status, PbftVoteEventFactStatus::Ready);
        assert!(precheck.progress_fact.is_some());
        assert_eq!(precheck.progress_fact.unwrap().weight, 42);
        assert!(
            precheck
                .pipeline_step
                .unwrap()
                .progress_plan
                .add_vote_outcome
                .is_none()
        );

        let terminal = session.report_verified_vote_add(add_outcome(true, None));
        assert_eq!(terminal.admission_status, PbftVoteAdmissionStatus::Complete);
        assert_eq!(
            terminal.pipeline_step.progress_plan.status,
            PbftVoteProgressStatus::Accepted
        );
    }

    #[test]
    fn validation_backed_admission_uses_calculated_weight() {
        let vote_rlp = signed_vote_rlp();
        let validation = validate_canonical_pbft_vote(&vote_rlp, validation_facts()).unwrap();
        assert_eq!(validation.status, PbftVoteValidationStatus::Valid);
        assert_eq!(validation.calculated_weight, 42);

        let mut session =
            create_pbft_vote_admission_session_from_validation(&validation, flags(), context());
        let precheck = session.precheck();

        assert_eq!(
            precheck.admission_status,
            PbftVoteAdmissionStatus::AwaitingVerifiedVoteInsert
        );
        assert_eq!(
            precheck.validation.as_ref().unwrap().vote_hash,
            validation.vote_hash
        );
        assert_eq!(precheck.event_status, PbftVoteEventFactStatus::Ready);
        assert_eq!(precheck.progress_fact.unwrap().weight, 42);
    }

    #[test]
    fn malformed_vote_completes_without_pipeline_session() {
        let mut session =
            create_pbft_vote_admission_session(&[0x01, 0x02], 42, flags(), context()).unwrap();

        let precheck = session.precheck();
        assert_eq!(
            precheck.admission_status,
            PbftVoteAdmissionStatus::EventRejected
        );
        assert_eq!(precheck.event_status, PbftVoteEventFactStatus::MalformedRlp);
        assert!(precheck.progress_fact.is_none());
        assert!(precheck.pipeline_step.is_none());
        assert!(precheck.complete);
    }

    #[test]
    fn validation_rejection_does_not_create_pipeline_session() {
        let vote_rlp = signed_vote_rlp();
        let validation = validate_canonical_pbft_vote(
            &vote_rlp,
            crate::PbftVoteValidationExternalFacts {
                voter_dpos_ready: false,
                voter_dpos_vote_count: 0,
                total_dpos_ready: true,
                total_dpos_vote_count: 100,
                future_dpos_state: true,
                unknown_error: false,
                vrf_key_ready: false,
                has_vrf_key: false,
                vrf_public_key: [0; 32],
                strict_vrf: true,
                committee_size: 100,
                number_of_proposers: 20,
            },
        )
        .unwrap();
        assert_eq!(validation.status, PbftVoteValidationStatus::FutureDposState);

        let mut session =
            PbftVoteAdmissionSession::from_validation(&validation, flags(), context());
        let precheck = session.precheck();
        assert_eq!(
            precheck.admission_status,
            PbftVoteAdmissionStatus::EventRejected
        );
        assert_eq!(
            precheck.event_status,
            PbftVoteEventFactStatus::ValidationRejected
        );
        assert!(precheck.pipeline_step.is_none());
    }

    #[test]
    fn admission_rejects_out_of_order_executor_report() {
        let vote_rlp = signed_vote_rlp();
        let mut session =
            create_pbft_vote_admission_session(&vote_rlp, 42, flags(), context()).unwrap();

        let terminal = session.report_verified_vote_add(add_outcome(true, None));

        assert_eq!(
            terminal.admission_status,
            PbftVoteAdmissionStatus::InvalidStage
        );
        assert_eq!(
            terminal.pipeline_step.progress_plan.status,
            PbftVoteProgressStatus::RejectedExecutorReport
        );
    }
}
