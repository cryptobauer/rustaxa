use crate::ffi::rustaxa_ffi::{
    DetermineNewRoundOutcome, PbftFinalizationHash, PbftLeaderCandidateSnapshot,
    PbftLeaderSelectionFinishRequest, PbftLeaderSelectionResult, PbftLeaderSelectionSnapshot,
    PbftNextVotesBundleEgressPlan, PbftOptimizedVoteBundleBuildRequest,
    PbftOptimizedVoteBundleBuildResult, PbftOptimizedVoteBundlePlan,
    PbftRewardVotePayloadSelection as FfiPbftRewardVotePayloadSelection,
    PbftRewardVotesResetRequest as FfiPbftRewardVotesResetRequest,
    PbftTwoTPlusOneThresholdFact as FfiPbftTwoTPlusOneThresholdFact,
    PbftTwoTPlusOneThresholdPlan as FfiPbftTwoTPlusOneThresholdPlan,
    PbftVoteAdmissionRuntimeResult, PbftVoteAdmissionValidationRequest, PbftVoteEventFactFlags,
    PbftVoteProgressContext as FfiPbftVoteProgressContext, PbftVoteRuntimeValidationResult,
    PbftVoteStorageRecord, RewardVoteCursorSnapshot as FfiRewardVoteCursorSnapshot,
    RewardVotePayloadSnapshot as FfiRewardVotePayloadSnapshot, RoundMarkerSnapshot,
    TwoTPlusOneSnapshotEntry, TwoTPlusOneVotePayloadsLookup, TwoTPlusOneVotedBlockLookup,
    VerifiedStepVotePayloadEntry, VerifiedStepVotePayloadsLookup,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
    VerifiedVoteStateSnapshotEntry, VerifiedVotesStateSnapshot,
};
use crate::ffi::{BridgeFinalChain, BridgePbftService};
use crate::pbft_vote_progress::{context_to_domain, execution_plan_to_ffi};

use crate::pbft_vote_validation::threshold_plan_to_ffi;
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::PbftFinalizedPeriodApplyResult;
use rustaxa_consensus::pbft_leader_selection::{
    PbftLeaderCandidateSnapshot as DomainPbftLeaderCandidateSnapshot,
    PbftLeaderCandidateValidation as DomainPbftLeaderCandidateValidation,
    PbftLeaderCandidateValidationStatus as DomainPbftLeaderCandidateValidationStatus,
    PbftLeaderSelectionFinishRequest as DomainPbftLeaderSelectionFinishRequest,
    PbftLeaderSelectionResult as DomainPbftLeaderSelectionResult,
    PbftLeaderSelectionSnapshot as DomainPbftLeaderSelectionSnapshot,
};
use rustaxa_consensus::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdStatus,
};
use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactFlags as DomainPbftVoteEventFactFlags;
use rustaxa_consensus::pbft_vote_validation::{
    PbftCanonicalVoteValidation,
    PbftVoteAdmissionValidationRequest as DomainPbftVoteAdmissionValidationRequest,
};
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome as ConsensusAddVerifiedVoteOutcome,
    DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome, PbftVoteType,
    TwoTPlusOneVotedBlockType, VerifiedVote,
};
use rustaxa_consensus::{
    build_weighted_pbft_vote_payload,
    PbftNextVotesBundleEgressPlan as ConsensusPbftNextVotesBundleEgressPlan,
    PbftOptimizedVoteBundleBuildRequest as ConsensusPbftOptimizedVoteBundleBuildRequest,
    PbftOptimizedVoteBundleBuildResult as ConsensusPbftOptimizedVoteBundleBuildResult,
    PbftOptimizedVoteBundlePlan as ConsensusPbftOptimizedVoteBundlePlan,
    PbftRewardVotePayloadSelection,
    PbftVerifiedVoteProgressBundle as DomainPbftVerifiedVoteProgressBundle,
    PbftVerifiedVoteProgressPersistenceWrite as DomainPbftVoteProgressPersistenceWrite,
    PbftVoteAdmissionTransactionResult, PbftVotePayloadRecord,
    PbftVotePersistenceResult as DomainPbftVotePersistenceResult, PbftVotePersistenceStatus,
    PbftVoteStorageRecord as DomainPbftVoteStorageRecord,
    RewardVoteCursorSnapshot as DomainRewardVoteCursorSnapshot,
    RewardVotePayloadSnapshot as DomainRewardVotePayloadSnapshot, RewardVoteResetApplyRequest,
    VerifiedVotesStateSnapshot as ConsensusVerifiedVotesStateSnapshot,
};

fn threshold_fact_from_request(
    fact: &FfiPbftTwoTPlusOneThresholdFact,
) -> Result<PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan> {
    let vote_type =
        PbftVoteType::try_from(fact.vote_type).map_err(|_| PbftTwoTPlusOneThresholdPlan {
            status: PbftTwoTPlusOneThresholdStatus::InvalidVoteType,
            error_code: "PBFT_TWO_T_PLUS_ONE_INVALID_VOTE_TYPE",
            has_threshold: false,
            threshold: 0,
            sortition_threshold: 0,
            needs_total_dpos_votes: false,
            cache_hit: false,
            cached: false,
        })?;
    Ok(PbftTwoTPlusOneThresholdFact {
        pbft_period: fact.pbft_period,
        vote_type,
        current_pbft_chain_size: 0,
        committee_size: fact.committee_size,
        number_of_proposers: fact.number_of_proposers,
        has_total_dpos_votes_count: false,
        total_dpos_votes_count: 0,
        future_dpos_state: false,
        unknown_error: false,
    })
}

fn pbft_vote_persistence_to_ffi(
    value: DomainPbftVotePersistenceResult,
) -> crate::ffi::rustaxa_ffi::PbftVotePersistenceResult {
    crate::ffi::rustaxa_ffi::PbftVotePersistenceResult {
        status: value.status.as_u8(),
        applied_writes: value.applied_writes,
        error_code: value.error_code,
    }
}

fn pbft_finalization_apply_result_to_ffi(
    value: PbftFinalizedPeriodApplyResult,
) -> crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult {
    crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult {
        status: value.status.as_u8(),
        wrote_pbft_head: value.wrote_pbft_head,
        wrote_period_data: value.wrote_period_data,
        dag_index_writes: value.dag_index_writes,
        transaction_location_writes: value.transaction_location_writes,
        block_period: value.block_period,
        pbft_block_hash: value.pbft_block_hash.0,
        reward_votes_reset_generation: value.reward_votes_reset_generation,
        error_code: value.error_code,
    }
}

fn vote_storage_record_to_ffi(value: DomainPbftVoteStorageRecord) -> PbftVoteStorageRecord {
    PbftVoteStorageRecord {
        hash: value.hash.0,
        vote_rlp: value.vote_rlp,
    }
}

fn vote_payload_record_to_ffi(value: PbftVotePayloadRecord) -> PbftVoteStorageRecord {
    PbftVoteStorageRecord {
        hash: value.hash.0,
        vote_rlp: value.vote_rlp,
    }
}

fn two_t_plus_one_bundle_to_domain(
    kind: u8,
    period: u64,
    round: u64,
    step: u64,
    block_hash: [u8; 32],
) -> Result<DomainPbftVerifiedVoteProgressBundle, DomainPbftVotePersistenceResult> {
    let kind =
        TwoTPlusOneVotedBlockType::try_from(kind).map_err(|_| DomainPbftVotePersistenceResult {
            status: PbftVotePersistenceStatus::Rejected,
            applied_writes: 0,
            error_code: "PBFT_VOTE_PERSIST_INVALID_TWO_T_PLUS_ONE_KIND".to_owned(),
        })?;
    Ok(DomainPbftVerifiedVoteProgressBundle {
        kind,
        period,
        round,
        step,
        block_hash: H256::from(block_hash),
    })
}

fn vote_progress_write_to_domain(
    value: crate::ffi::rustaxa_ffi::PbftVoteProgressPersistenceWrite,
) -> Result<DomainPbftVoteProgressPersistenceWrite, DomainPbftVotePersistenceResult> {
    Ok(DomainPbftVoteProgressPersistenceWrite {
        extra_reward_vote_hash: value
            .has_extra_reward_vote
            .then(|| H256::from(value.extra_reward_vote_hash)),
        two_t_plus_one_bundle: if value.has_two_t_plus_one_bundle {
            Some(two_t_plus_one_bundle_to_domain(
                value.two_t_plus_one_kind,
                value.two_t_plus_one_period,
                value.two_t_plus_one_round,
                value.two_t_plus_one_step,
                value.two_t_plus_one_block_hash,
            )?)
        } else {
            None
        },
    })
}

#[allow(clippy::too_many_arguments)]
impl BridgePbftService {
    fn publish_vote_validation(
        &self,
        validation: PbftCanonicalVoteValidation,
        replay: rustaxa_consensus::pbft_vote_runtime::PbftVoteRuntimeReplayOutcome,
        weighted_vote_rlp: Vec<u8>,
    ) -> Result<PbftVoteRuntimeValidationResult, anyhow::Error> {
        Ok(PbftVoteRuntimeValidationResult {
            status: validation.status.as_u8(),
            error_code: validation.error_code.to_owned(),
            accepted: validation.accepted,
            rejected: validation.rejected,
            validation: validation.into(),
            replay_should_mark: replay.should_mark,
            replay_inserted: replay.inserted,
            replay_already_present: replay.already_present,
            has_weighted_vote: !weighted_vote_rlp.is_empty(),
            weighted_vote_rlp,
        })
    }

    /// Validates one canonical PBFT vote against live Rust FinalChain state.
    ///
    /// The operation preserves legacy lookup order: voter DPoS stake first,
    /// then the validator VRF key at the vote's DPoS block with prior/next
    /// fallback, followed by total DPoS stake. Signature, VRF proof, threshold,
    /// and weight validation execute without holding the verified-votes mutex;
    /// that mutex is acquired only once to publish the resulting replay mark.
    /// Future FinalChain state retains the existing non-replayable status,
    /// while corrupt/missing ready state is reported as the existing unknown
    /// error status.
    pub fn pbft_service_verified_votes_validate_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        canonical_vote_rlp: &[u8],
        strict_vrf: bool,
        committee_size: u64,
        number_of_proposers: u64,
    ) -> Result<PbftVoteRuntimeValidationResult, anyhow::Error> {
        let request = admission_validation_request_to_domain(PbftVoteAdmissionValidationRequest {
            strict_vrf,
            committee_size,
            number_of_proposers,
            has_preverified_weight: false,
            preverified_weight: 0,
        });
        let (validation, replay, weighted_vote_rlp) = self
            .0
            .validate_verified_vote_with_final_chain(&final_chain.0, canonical_vote_rlp, request)?;
        self.publish_vote_validation(validation, replay, weighted_vote_rlp.unwrap_or_default())
    }

    /// Validates and persists one canonical PBFT vote against FinalChain state.
    ///
    /// The call preserves admission replay and checkpoint semantics used by
    /// existing shim wiring: validation runs before write planning and all
    /// persistence writes are wrapped in one transactional Rust admission session.
    pub fn pbft_service_verified_votes_admit_and_persist_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        canonical_vote_rlp: &[u8],
        validation_request: PbftVoteAdmissionValidationRequest,
        flags: PbftVoteEventFactFlags,
        context: FfiPbftVoteProgressContext,
    ) -> Result<PbftVoteAdmissionRuntimeResult, anyhow::Error> {
        let request = admission_validation_request_to_domain(validation_request);
        let (validation, transaction) = self.0.admit_and_persist_verified_vote_with_final_chain(
            &final_chain.0,
            canonical_vote_rlp,
            request,
            flags_to_domain(flags),
            context_to_domain(&context),
        )?;
        let weighted_vote_rlp = if validation.accepted && validation.weight_calculated {
            build_weighted_pbft_vote_payload(canonical_vote_rlp, validation.calculated_weight)?
                .vote_rlp
        } else {
            Vec::new()
        };
        Ok(runtime_outcome_to_ffi(
            validation,
            transaction,
            weighted_vote_rlp,
        ))
    }

    /// Plans or resolves one PBFT `2t+1` threshold with Rust FinalChain composition.
    ///
    /// The caller supplies only the requested period, vote type, and committee
    /// configuration. The CXX request has no PBFT-chain or DPoS state fields;
    /// Rust derives the live PBFT chain size, then probes the verified-vote
    /// cache under its mutex. Only a cache miss
    /// that explicitly requests the total releases that mutex, borrows
    /// `final_chain` synchronously for the exact requested period, refreshes the
    /// chain size, and re-enters the planner to publish the result. Missing
    /// future state and infrastructure errors become the planner's existing
    /// fail-closed statuses; the FinalChain handle is never retained.
    pub fn pbft_service_verified_votes_two_t_plus_one_threshold_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        fact: FfiPbftTwoTPlusOneThresholdFact,
    ) -> Result<FfiPbftTwoTPlusOneThresholdPlan, anyhow::Error> {
        let request = match threshold_fact_from_request(&fact) {
            Ok(request) => request,
            Err(plan) => return Ok(threshold_plan_to_ffi(plan)),
        };
        let plan = self
            .0
            .verified_votes_two_t_plus_one_threshold_with_final_chain(&final_chain.0, request)?;
        Ok(threshold_plan_to_ffi(plan))
    }

    /// Prepares an owned native leader-selection snapshot for CXX validation.
    ///
    /// `period` and `round` select proposal votes at step one. The native
    /// service serializes finalized membership and sibling state, returns
    /// candidates in deterministic vote-hash order, and releases every Rust
    /// lock before this method returns. Status zero means a snapshot is ready;
    /// status one means no candidates. Manager-lock poison follows the native
    /// manager service's invariant panic policy; sibling-lock poison, missing
    /// retained vote payloads, and storage lookup failures are returned as
    /// operational errors.
    pub fn pbft_service_prepare_leader_selection(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftLeaderSelectionSnapshot, anyhow::Error> {
        self.0
            .prepare_leader_selection(period, round)
            .map(leader_selection_snapshot_to_ffi)
    }

    /// Revalidates and finishes one prepared native leader-selection snapshot.
    ///
    /// The request must echo the prepared period, round, and fingerprint and
    /// contain exactly one accepted/rejected report for every candidate that
    /// requested external validation. Unknown report codes are mapped to an
    /// invalid report. Native state is fully revalidated before any proposed
    /// block is marked valid; stale or malformed requests return typed
    /// non-selected results without mutation. Operational lock/codec/planner
    /// contract failures are returned as errors.
    pub fn pbft_service_finish_leader_selection(
        &self,
        request: PbftLeaderSelectionFinishRequest,
    ) -> Result<PbftLeaderSelectionResult, anyhow::Error> {
        self.0
            .finish_leader_selection(leader_selection_finish_request_to_domain(request))
            .map(leader_selection_result_to_ffi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::{PbftOptimizedVoteBundlePlanEntry, VerifiedVoteOptimizedBundleStatus};

    #[test]
    fn optimized_bundle_statuses_keep_stable_cxx_codes() {
        let statuses = [
            VerifiedVoteOptimizedBundleStatus::Ready,
            VerifiedVoteOptimizedBundleStatus::NotFound,
            VerifiedVoteOptimizedBundleStatus::EmptyRequest,
            VerifiedVoteOptimizedBundleStatus::UnsupportedKind,
            VerifiedVoteOptimizedBundleStatus::MappingMismatch,
            VerifiedVoteOptimizedBundleStatus::HashNotInPlan,
            VerifiedVoteOptimizedBundleStatus::OrderMismatch,
            VerifiedVoteOptimizedBundleStatus::MissingPayload,
            VerifiedVoteOptimizedBundleStatus::PayloadDecodeError,
            VerifiedVoteOptimizedBundleStatus::PayloadMetadataMismatch,
        ];
        for (expected, status) in statuses.into_iter().enumerate() {
            let ffi =
                optimize_bundle_build_result_to_ffi(ConsensusPbftOptimizedVoteBundleBuildResult {
                    status,
                    vote_hashes: vec![H256::from([expected as u8; 32])],
                    votes_bundle_rlp: vec![expected as u8],
                });
            assert_eq!(ffi.status, expected as u8);
            assert_eq!(ffi.error_code, status.legacy_error_code());
            assert_eq!(ffi.vote_hashes[0].hash, [expected as u8; 32]);
        }
    }

    #[test]
    fn optimized_bundle_plan_projects_option_and_order() {
        let hashes = vec![H256::from([0x11; 32]), H256::from([0x22; 32])];
        let ffi = optimize_bundle_plan_to_ffi(ConsensusPbftOptimizedVoteBundlePlan {
            status: VerifiedVoteOptimizedBundleStatus::Ready,
            kind: TwoTPlusOneVotedBlockType::NextVotedBlock,
            period: 7,
            round: 8,
            plan: Some(PbftOptimizedVoteBundlePlanEntry {
                block_hash: H256::from([0x33; 32]),
                period: 7,
                round: 8,
                step: 9,
                vote_hashes: hashes,
            }),
        });
        assert!(ffi.found);
        assert_eq!(
            ffi.kind,
            u8::from(TwoTPlusOneVotedBlockType::NextVotedBlock)
        );
        assert_eq!(ffi.block_hash, [0x33; 32]);
        assert_eq!(ffi.vote_hashes[0].hash, [0x11; 32]);
        assert_eq!(ffi.vote_hashes[1].hash, [0x22; 32]);

        let missing = optimize_bundle_plan_to_ffi(ConsensusPbftOptimizedVoteBundlePlan {
            status: VerifiedVoteOptimizedBundleStatus::NotFound,
            kind: TwoTPlusOneVotedBlockType::NextVotedNullBlock,
            period: 12,
            round: 13,
            plan: None,
        });
        assert!(!missing.found);
        assert_eq!(missing.block_hash, [0; 32]);
        assert_eq!(missing.period, 12);
        assert_eq!(missing.round, 13);
        assert!(missing.vote_hashes.is_empty());
    }

    #[test]
    fn raw_invalid_bundle_kind_stays_bridge_only() {
        let request = PbftOptimizedVoteBundleBuildRequest {
            kind: u8::MAX,
            block_hash: [0; 32],
            period: 0,
            round: 0,
            step: 0,
            vote_hashes: Vec::new(),
        };
        assert!(optimize_bundle_request_to_domain(request).is_none());
        assert!(TwoTPlusOneVotedBlockType::try_from(u8::MAX).is_err());
        assert_eq!(
            VerifiedVoteOptimizedBundleStatus::UnsupportedKind.as_u8(),
            3
        );
        assert_eq!(
            VerifiedVoteOptimizedBundleStatus::UnsupportedKind.legacy_error_code(),
            "PBFT_OPTIMIZED_VOTE_BUNDLE_UNSUPPORTED_KIND"
        );
    }

    #[test]
    fn raw_invalid_persistence_kind_stays_typed_rejection() {
        let result = two_t_plus_one_bundle_to_domain(u8::MAX, 1, 2, 3, [0x44; 32]).unwrap_err();
        assert_eq!(result.status, PbftVotePersistenceStatus::Rejected);
        assert_eq!(result.applied_writes, 0);
        assert_eq!(
            result.error_code,
            "PBFT_VOTE_PERSIST_INVALID_TWO_T_PLUS_ONE_KIND"
        );
    }

    #[test]
    fn leader_and_reward_results_keep_stable_boundary_statuses() {
        for (status, expected) in [
            (rustaxa_consensus::PbftLeaderSelectionStatus::Selected, 0),
            (
                rustaxa_consensus::PbftLeaderSelectionStatus::NoCandidates,
                1,
            ),
            (rustaxa_consensus::PbftLeaderSelectionStatus::NoEligible, 2),
            (
                rustaxa_consensus::PbftLeaderSelectionStatus::StaleSnapshot,
                3,
            ),
            (
                rustaxa_consensus::PbftLeaderSelectionStatus::InvalidValidationReport,
                4,
            ),
        ] {
            let result = leader_selection_result_to_ffi(DomainPbftLeaderSelectionResult {
                status,
                error_code: "status".to_owned(),
                selected: status == rustaxa_consensus::PbftLeaderSelectionStatus::Selected,
                selected_vote: DomainPbftVoteStorageRecord {
                    hash: H256::from([0x51; 32]),
                    vote_rlp: vec![0x52],
                },
                selected_block_rlp: vec![0x53],
            });
            assert_eq!(result.status, expected);
            assert_eq!(result.selected_vote.hash, [0x51; 32]);
            assert_eq!(result.selected_block_rlp, vec![0x53]);
        }

        let rejected_write_set =
            rustaxa_consensus::pbft_finalize::PbftFinalizedPeriodApplyStatus::RejectedWriteSet;
        let mapped = pbft_finalization_apply_result_to_ffi(PbftFinalizedPeriodApplyResult {
            status: rejected_write_set,
            wrote_pbft_head: false,
            wrote_period_data: false,
            dag_index_writes: 0,
            transaction_location_writes: 0,
            block_period: 12,
            pbft_block_hash: H256::from([0x61; 32]),
            reward_votes_reset_generation: 0,
            error_code: "PBFT_FINALIZE_REJECTED_WRITE_SET".to_owned(),
        });
        assert_eq!(mapped.status, rejected_write_set.as_u8());
        assert_eq!(mapped.pbft_block_hash, [0x61; 32]);
        assert_eq!(mapped.error_code, "PBFT_FINALIZE_REJECTED_WRITE_SET");
    }

    #[test]
    fn runtime_result_flattens_executor_intents() {
        use ethereum_types::H160;
        use rustaxa_consensus::{
            PbftVoteAdmissionExecution, PbftVoteAdmissionPersistenceStatus,
            PbftVoteAdmissionPrecheck, PbftVoteAdmissionStatus, PbftVoteEventFactStatus,
            PbftVoteIdentity, PbftVotePipelineStatus, PbftVotePipelineStep, PbftVoteProgressFact,
            PbftVoteProgressIntent, PbftVoteProgressPlan, PbftVoteProgressStatus,
            PbftVoteRuntimeAdmissionOutcome, PbftVoteValidationStatus,
        };

        let vote_hash = H256::from([0x71; 32]);
        let block_hash = H256::from([0x72; 32]);
        let voter = H160::from([0x73; 20]);
        let validation = PbftCanonicalVoteValidation {
            status: PbftVoteValidationStatus::Valid,
            error_code: "",
            accepted: true,
            rejected: false,
            mark_validated_replay: true,
            vote_hash,
            signing_hash: H256::from([0x74; 32]),
            block_hash,
            period: 3,
            round: 2,
            step: 4,
            vote_type: PbftVoteType::Next,
            recovered_voter: voter,
            recovered_public_key: [0; 64],
            signature_valid: true,
            vrf_valid: true,
            has_sortition_threshold: true,
            sortition_threshold: 5,
            weight_calculated: true,
            calculated_weight: 5,
            vrf_output: [0; 64],
        };
        let progress_fact = PbftVoteProgressFact {
            identity: PbftVoteIdentity {
                vote_hash,
                block_hash,
                period: 3,
                round: 2,
                step: 4,
                voter,
            },
            vote_type: PbftVoteType::Next,
            weight: 5,
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        };
        let progress_plan = PbftVoteProgressPlan {
            status: PbftVoteProgressStatus::Accepted,
            intents: vec![
                PbftVoteProgressIntent::MarkKnown { vote_hash },
                PbftVoteProgressIntent::GossipVote { vote_hash },
                PbftVoteProgressIntent::DrivePbftProgress {
                    period: 3,
                    round: 2,
                },
            ],
            add_vote_outcome: None,
            threshold_decision: None,
            conflicting_vote_hash: None,
        };
        let outcome = PbftVoteRuntimeAdmissionOutcome {
            precheck: PbftVoteAdmissionPrecheck {
                admission_status: PbftVoteAdmissionStatus::AwaitingVerifiedVoteInsert,
                validation: Some(validation.clone()),
                event_status: PbftVoteEventFactStatus::Ready,
                error_code: "",
                progress_fact: Some(progress_fact),
                pipeline_step: None,
                complete: false,
            },
            replay: rustaxa_consensus::pbft_vote_runtime::PbftVoteRuntimeReplayOutcome {
                should_mark: true,
                inserted: true,
                already_present: false,
            },
            execution: Some(PbftVoteAdmissionExecution {
                admission_status: PbftVoteAdmissionStatus::Complete,
                pipeline_step: PbftVotePipelineStep {
                    pipeline_status: PbftVotePipelineStatus::Complete,
                    progress_plan,
                    complete: true,
                },
                complete: true,
            }),
            add_outcome: None,
            storage_vote: None,
            two_t_plus_one_bundle: None,
            slashing_payloads: None,
        };
        let result = runtime_outcome_to_ffi(
            validation,
            PbftVoteAdmissionTransactionResult {
                outcome,
                persistence_required: false,
                persistence_status: PbftVoteAdmissionPersistenceStatus::NotRequired,
                persistence_applied_writes: 0,
                transition_published: true,
                persistence_error_code: String::new(),
            },
            Vec::new(),
        );
        assert!(result.accepted);
        assert!(result.mark_vote_known);
        assert_eq!(result.mark_vote_known_hash, [0x71; 32]);
        assert!(result.gossip_vote);
        assert_eq!(result.gossip_vote_hash, [0x71; 32]);
        assert!(result.drive_pbft_progress);
        assert_eq!((result.progress_period, result.progress_round), (3, 2));
    }
}

fn leader_candidate_snapshot_to_ffi(
    value: DomainPbftLeaderCandidateSnapshot,
) -> PbftLeaderCandidateSnapshot {
    PbftLeaderCandidateSnapshot {
        vote_hash: value.vote_hash.0,
        block_hash: value.block_hash.0,
        vote_record: PbftVoteStorageRecord {
            hash: value.vote_record.hash.0,
            vote_rlp: value.vote_record.vote_rlp,
        },
        proposed_block_found: value.proposed_block_found,
        proposed_block_is_valid: value.proposed_block_is_valid,
        proposed_block_rlp: value.proposed_block_rlp,
        pivot_hash: value.pivot_hash.0,
        block_in_chain: value.block_in_chain,
        needs_external_validation: value.needs_external_validation,
    }
}

fn leader_selection_snapshot_to_ffi(
    value: DomainPbftLeaderSelectionSnapshot,
) -> PbftLeaderSelectionSnapshot {
    PbftLeaderSelectionSnapshot {
        status: value.status.as_u8(),
        error_code: value.error_code,
        period: value.period,
        round: value.round,
        snapshot_fingerprint: value.snapshot_fingerprint,
        candidates: value
            .candidates
            .into_iter()
            .map(leader_candidate_snapshot_to_ffi)
            .collect(),
    }
}

fn leader_selection_finish_request_to_domain(
    value: PbftLeaderSelectionFinishRequest,
) -> DomainPbftLeaderSelectionFinishRequest {
    DomainPbftLeaderSelectionFinishRequest {
        period: value.period,
        round: value.round,
        snapshot_fingerprint: value.snapshot_fingerprint,
        validations: value
            .validations
            .into_iter()
            .map(|report| DomainPbftLeaderCandidateValidation {
                vote_hash: H256::from(report.vote_hash),
                block_hash: H256::from(report.block_hash),
                status: match report.status {
                    1 => DomainPbftLeaderCandidateValidationStatus::Validated,
                    2 => DomainPbftLeaderCandidateValidationStatus::Rejected,
                    _ => DomainPbftLeaderCandidateValidationStatus::Invalid,
                },
            })
            .collect(),
    }
}

fn leader_selection_result_to_ffi(
    value: DomainPbftLeaderSelectionResult,
) -> PbftLeaderSelectionResult {
    PbftLeaderSelectionResult {
        status: value.status.as_u8(),
        error_code: value.error_code,
        selected: value.selected,
        selected_vote: PbftVoteStorageRecord {
            hash: value.selected_vote.hash.0,
            vote_rlp: value.selected_vote.vote_rlp,
        },
        selected_block_rlp: value.selected_block_rlp,
    }
}

impl BridgePbftService {
    /// Converts the native reward cursor into the stable CXX snapshot carrier.
    pub fn pbft_service_verified_votes_reward_vote_cursor(
        &self,
    ) -> Result<FfiRewardVoteCursorSnapshot, anyhow::Error> {
        self.0
            .reward_vote_cursor_snapshot()
            .map(reward_vote_cursor_snapshot_to_ffi)
    }

    /// Returns the native finalized reward period; native lock errors propagate.
    pub fn pbft_service_verified_votes_reward_vote_period(&self) -> Result<u64, anyhow::Error> {
        self.0.reward_vote_period()
    }

    /// Converts requested CXX hashes, delegates ordered selection, and maps its typed result.
    pub fn pbft_service_verified_votes_select_reward_vote_payloads(
        &self,
        block_period: u64,
        requested_vote_hashes: Vec<PbftFinalizationHash>,
    ) -> Result<FfiPbftRewardVotePayloadSelection, anyhow::Error> {
        let requested_vote_hashes = requested_vote_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect();
        self.0
            .select_reward_vote_payloads(block_period, requested_vote_hashes)
            .map(reward_vote_payload_selection_to_ffi)
    }

    /// Converts and delegates the complete native reset/storage/live-publication task.
    pub fn pbft_service_verified_votes_apply_reward_votes_reset(
        &self,
        request: FfiPbftRewardVotesResetRequest,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult, anyhow::Error> {
        self.0
            .apply_reward_votes_reset(reward_votes_reset_apply_request_to_domain(request))
            .map(pbft_finalization_apply_result_to_ffi)
    }

    /// Maps one coherent native cursor-and-payload snapshot into stable CXX carriers.
    pub fn pbft_service_verified_votes_current_reward_snapshot(
        &self,
    ) -> Result<FfiRewardVotePayloadSnapshot, anyhow::Error> {
        self.0
            .current_reward_vote_snapshot()
            .map(reward_vote_payload_snapshot_to_ffi)
    }
}

fn copy_vote_payload(value: &VerifiedVotePayload) -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: value.vote_hash,
        block_hash: value.block_hash,
        voter: value.voter,
        period: value.period,
        round: value.round,
        step: value.step,
        vote_type: value.vote_type,
        weight: value.weight,
    }
}

fn flags_to_domain(value: PbftVoteEventFactFlags) -> DomainPbftVoteEventFactFlags {
    DomainPbftVoteEventFactFlags {
        vote_already_known: value.vote_already_known,
        carries_proposed_block: value.carries_proposed_block,
        valid_stale_reward_vote: false,
    }
}

fn admission_validation_request_to_domain(
    value: PbftVoteAdmissionValidationRequest,
) -> DomainPbftVoteAdmissionValidationRequest {
    DomainPbftVoteAdmissionValidationRequest {
        strict_vrf: value.strict_vrf,
        committee_size: value.committee_size,
        number_of_proposers: value.number_of_proposers,
        has_preverified_weight: value.has_preverified_weight,
        preverified_weight: value.preverified_weight,
    }
}

fn reward_vote_cursor_snapshot_to_ffi(
    value: DomainRewardVoteCursorSnapshot,
) -> FfiRewardVoteCursorSnapshot {
    FfiRewardVoteCursorSnapshot {
        found: value.found,
        period: value.period,
        round: value.round,
        step: value.step,
        block_hash: value.block_hash.0,
    }
}

fn reward_vote_payload_snapshot_to_ffi(
    value: DomainRewardVotePayloadSnapshot,
) -> FfiRewardVotePayloadSnapshot {
    FfiRewardVotePayloadSnapshot {
        cursor: reward_vote_cursor_snapshot_to_ffi(value.cursor),
        records: value
            .records
            .into_iter()
            .map(|record| PbftVoteStorageRecord {
                hash: record.hash.0,
                vote_rlp: record.vote_rlp,
            })
            .collect(),
    }
}

fn reward_vote_payload_selection_to_ffi(
    value: PbftRewardVotePayloadSelection,
) -> FfiPbftRewardVotePayloadSelection {
    FfiPbftRewardVotePayloadSelection {
        accepted: value.accepted,
        status: value.status.as_u8(),
        error_code: value.status.legacy_error_code().to_owned(),
        selected_period: value.selected_period,
        selected_round: value.selected_round,
        selected_block_hash: value.selected_block_hash.0,
        selected_vote_hashes: value
            .selected_vote_hashes
            .into_iter()
            .map(|hash| PbftFinalizationHash { hash: hash.0 })
            .collect(),
        selected_records: value
            .selected_records
            .into_iter()
            .map(|record| PbftVoteStorageRecord {
                hash: record.hash.0,
                vote_rlp: record.vote_rlp,
            })
            .collect(),
        missing_vote_hash: value.missing_vote_hash.unwrap_or_default().0,
    }
}

fn reward_votes_reset_apply_request_to_domain(
    request: FfiPbftRewardVotesResetRequest,
) -> RewardVoteResetApplyRequest {
    RewardVoteResetApplyRequest {
        period: request.period,
        round: request.round,
        step: request.step,
        block_hash: H256::from(request.block_hash),
        sync: request.sync,
    }
}

fn runtime_outcome_to_ffi(
    validation: PbftCanonicalVoteValidation,
    transaction: PbftVoteAdmissionTransactionResult,
    weighted_vote_rlp: Vec<u8>,
) -> PbftVoteAdmissionRuntimeResult {
    let transition_published = transaction.transition_published;
    let persistence_required = transaction.persistence_required;
    let persistence_status = transaction.persistence_status.as_u8();
    let persistence_applied_writes = transaction.persistence_applied_writes;
    let persistence_error_code = transaction.persistence_error_code;
    let outcome = transaction.outcome;
    let progress_fact = outcome.precheck.progress_fact;
    let empty_vote = empty_vote_payload();
    let vote = progress_fact
        .map(progress_fact_to_vote)
        .unwrap_or(empty_vote);
    let progress = outcome
        .execution
        .as_ref()
        .map(|execution| execution_plan_to_ffi(execution.pipeline_step.progress_plan.clone()));
    let status = progress
        .as_ref()
        .map(|progress| progress.status)
        .unwrap_or(outcome.precheck.event_status.as_u8());
    let mut error_code = progress
        .as_ref()
        .map(|progress| progress.error_code.clone())
        .unwrap_or_else(|| outcome.precheck.error_code.to_owned());
    if !transition_published && !persistence_error_code.is_empty() {
        error_code = persistence_error_code;
    }
    let add_outcome = outcome.add_outcome.map(|add| {
        outcome_to_ffi_add_vote_outcome(
            copy_vote_payload(&vote),
            add,
            false,
            empty_storage_record(),
            false,
            empty_step_vote_payload_entry(),
        )
    });
    let (slashing_incoming_vote, slashing_conflicting_vote) = if transition_published {
        if let Some(payloads) = outcome.slashing_payloads {
            (
                vote_payload_record_to_ffi(payloads.incoming),
                vote_payload_record_to_ffi(payloads.conflicting),
            )
        } else {
            (empty_storage_record(), empty_storage_record())
        }
    } else {
        (empty_storage_record(), empty_storage_record())
    };

    PbftVoteAdmissionRuntimeResult {
        status,
        error_code,
        accepted: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.accepted)
                .unwrap_or(false),
        rejected: !validation.accepted || !transition_published,
        has_validation: true,
        replay_should_mark: outcome.replay.should_mark,
        replay_inserted: transition_published && outcome.replay.inserted,
        replay_already_present: outcome.replay.already_present,
        validation: validation.into(),
        has_vote: progress_fact.is_some(),
        vote,
        has_verified_vote_add: transition_published && add_outcome.is_some(),
        verified_vote_add: add_outcome.unwrap_or_else(|| {
            outcome_to_ffi_add_vote_outcome(
                empty_vote_payload(),
                empty_add_outcome(),
                false,
                empty_storage_record(),
                false,
                empty_step_vote_payload_entry(),
            )
        }),
        persistence_required,
        persistence_status,
        persistence_applied_writes,
        transition_published,
        mark_vote_known: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.mark_vote_known)
                .unwrap_or(false),
        mark_vote_known_hash: progress
            .as_ref()
            .map(|progress| progress.mark_vote_known_hash)
            .unwrap_or_default(),
        request_proposed_block_sidecar: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.request_proposed_block_sidecar)
                .unwrap_or(false),
        proposed_block_sidecar_hash: progress
            .as_ref()
            .map(|progress| progress.proposed_block_sidecar_hash)
            .unwrap_or_default(),
        proposed_block_sidecar_period: progress
            .as_ref()
            .map(|progress| progress.proposed_block_sidecar_period)
            .unwrap_or_default(),
        gossip_vote: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.gossip_vote)
                .unwrap_or(false),
        gossip_vote_hash: progress
            .as_ref()
            .map(|progress| progress.gossip_vote_hash)
            .unwrap_or_default(),
        report_slashing: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.report_slashing)
                .unwrap_or(false),
        slashing_incoming_vote,
        slashing_conflicting_vote,
        network_t_plus_one_step_updated: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.network_t_plus_one_step_updated)
                .unwrap_or(false),
        drive_pbft_progress: transition_published
            && progress
                .as_ref()
                .map(|progress| progress.drive_pbft_progress)
                .unwrap_or(false),
        progress_period: progress
            .as_ref()
            .map(|progress| progress.progress_period)
            .unwrap_or_default(),
        progress_round: progress
            .as_ref()
            .map(|progress| progress.progress_round)
            .unwrap_or_default(),
        weighted_vote_rlp,
    }
}

fn progress_fact_to_vote(value: rustaxa_consensus::PbftVoteProgressFact) -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: value.identity.vote_hash.0,
        block_hash: value.identity.block_hash.0,
        voter: value.identity.voter.0,
        period: value.identity.period,
        round: value.identity.round,
        step: value.identity.step,
        vote_type: value.vote_type.into(),
        weight: value.weight,
    }
}

fn empty_vote_payload() -> VerifiedVotePayload {
    VerifiedVotePayload {
        vote_hash: [0; 32],
        block_hash: [0; 32],
        voter: [0; 20],
        period: 0,
        round: 0,
        step: 0,
        vote_type: 0,
        weight: 0,
    }
}

fn empty_add_outcome() -> ConsensusAddVerifiedVoteOutcome {
    ConsensusAddVerifiedVoteOutcome {
        inserted: false,
        total_weight: 0,
        votes_count: 0,
        conflicting_vote_hash: None,
        used_secondary_slot: false,
        duplicate_vote_hash: false,
        threshold_decision: None,
    }
}

fn empty_storage_record() -> PbftVoteStorageRecord {
    PbftVoteStorageRecord {
        hash: [0; 32],
        vote_rlp: Vec::new(),
    }
}

fn empty_step_vote_payload_entry() -> VerifiedStepVotePayloadEntry {
    VerifiedStepVotePayloadEntry {
        block_hash: [0; 32],
        total_weight: 0,
        votes: Vec::new(),
    }
}

fn outcome_to_ffi_add_vote_outcome(
    vote: VerifiedVotePayload,
    value: ConsensusAddVerifiedVoteOutcome,
    conflicting_vote_found: bool,
    conflicting_vote: PbftVoteStorageRecord,
    bucket_found: bool,
    bucket: VerifiedStepVotePayloadEntry,
) -> FfiVerifiedVoteAddOutcome {
    let (
        threshold_applied,
        t_plus_one_reached,
        network_t_plus_one_step_updated,
        two_t_plus_one_reached,
        two_t_plus_one_kind_found,
        two_t_plus_one_kind,
        two_t_plus_one_round_found,
        two_t_plus_one_inserted,
    ) = value
        .threshold_decision
        .map(|threshold| {
            (
                true,
                threshold.t_plus_one_reached,
                threshold.network_t_plus_one_step_updated,
                threshold.two_t_plus_one_reached,
                threshold.two_t_plus_one_kind.is_some(),
                threshold.two_t_plus_one_kind.map(Into::into).unwrap_or(0),
                threshold
                    .two_t_plus_one_insert_outcome
                    .map(|outcome| outcome.round_found)
                    .unwrap_or(false),
                threshold
                    .two_t_plus_one_insert_outcome
                    .map(|outcome| outcome.inserted)
                    .unwrap_or(false),
            )
        })
        .unwrap_or((false, false, false, false, false, 0u8, false, false));

    FfiVerifiedVoteAddOutcome {
        vote,
        inserted: value.inserted,
        total_weight: value.total_weight,
        votes_count: value.votes_count,
        conflict_found: value.conflicting_vote_hash.is_some(),
        conflicting_vote_hash: value.conflicting_vote_hash.unwrap_or_default().into(),
        conflicting_vote_found,
        conflicting_vote,
        bucket_found,
        bucket,
        used_secondary_slot: value.used_secondary_slot,
        duplicate_vote_hash: value.duplicate_vote_hash,
        threshold_applied,
        t_plus_one_reached,
        network_t_plus_one_step_updated,
        two_t_plus_one_reached,
        two_t_plus_one_kind_found,
        two_t_plus_one_kind,
        two_t_plus_one_round_found,
        two_t_plus_one_inserted,
    }
}

impl From<ConsensusDetermineNewRoundOutcome> for DetermineNewRoundOutcome {
    fn from(value: ConsensusDetermineNewRoundOutcome) -> Self {
        Self {
            found: true,
            new_round: value.new_round,
            source_round: value.source_round,
            source_kind: value.source_kind.into(),
            block_hash: value.block_hash.into(),
            step: value.step,
        }
    }
}

impl From<VerifiedVote> for VerifiedVotePayload {
    fn from(value: VerifiedVote) -> Self {
        Self {
            vote_hash: value.vote_hash.into(),
            block_hash: value.block_hash.into(),
            voter: value.voter.into(),
            period: value.period,
            round: value.round,
            step: value.step,
            vote_type: value.vote_type.into(),
            weight: value.weight,
        }
    }
}

fn optimize_bundle_plan_to_ffi(
    value: ConsensusPbftOptimizedVoteBundlePlan,
) -> PbftOptimizedVoteBundlePlan {
    let found = value.plan.is_some();
    let (block_hash, period, round, step, vote_hashes) = match value.plan {
        Some(plan) => (
            plan.block_hash.0,
            plan.period,
            plan.round,
            plan.step,
            plan.vote_hashes
                .into_iter()
                .map(|hash| PbftFinalizationHash { hash: hash.0 })
                .collect(),
        ),
        None => ([0u8; 32], value.period, value.round, 0, Vec::new()),
    };

    PbftOptimizedVoteBundlePlan {
        found,
        status: value.status.as_u8(),
        error_code: value.status.legacy_error_code().to_owned(),
        kind: value.kind.into(),
        block_hash,
        period,
        round,
        step,
        vote_hashes,
    }
}

fn optimize_bundle_build_result_to_ffi(
    value: ConsensusPbftOptimizedVoteBundleBuildResult,
) -> PbftOptimizedVoteBundleBuildResult {
    PbftOptimizedVoteBundleBuildResult {
        status: value.status.as_u8(),
        error_code: value.status.legacy_error_code().to_owned(),
        vote_hashes: value
            .vote_hashes
            .into_iter()
            .map(|hash| PbftFinalizationHash { hash: hash.0 })
            .collect(),
        votes_bundle_rlp: value.votes_bundle_rlp,
    }
}

fn optimize_bundle_request_to_domain(
    value: PbftOptimizedVoteBundleBuildRequest,
) -> Option<ConsensusPbftOptimizedVoteBundleBuildRequest> {
    let kind = TwoTPlusOneVotedBlockType::try_from(value.kind).ok()?;
    Some(ConsensusPbftOptimizedVoteBundleBuildRequest {
        kind,
        block_hash: H256::from(value.block_hash),
        period: value.period,
        round: value.round,
        step: value.step,
        vote_hashes: value
            .vote_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect(),
    })
}

impl BridgePbftService {
    /// Reads one canonical verified-vote count under the service lock boundary.
    pub fn pbft_service_verified_votes_size(&self) -> Result<u64, anyhow::Error> {
        self.0.verified_votes_size()
    }

    /// Checks one replay-protection membership bit.
    pub fn pbft_service_verified_votes_replay_contains(
        &self,
        vote_hash: &[u8; 32],
    ) -> Result<bool, anyhow::Error> {
        self.0
            .verified_votes_replay_contains(H256::from(*vote_hash))
    }

    /// Inserts one replay-protection membership bit.
    pub fn pbft_service_verified_votes_replay_insert(
        &self,
        vote_hash: &[u8; 32],
    ) -> Result<bool, anyhow::Error> {
        self.0.verified_votes_replay_insert(H256::from(*vote_hash))
    }

    /// Determines the next round from next-vote 2t+1 state.
    pub fn pbft_service_verified_votes_determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> Result<DetermineNewRoundOutcome, anyhow::Error> {
        Ok(self
            .0
            .verified_votes_determine_new_round(period, current_round)?
            .map(Into::into)
            .unwrap_or(DetermineNewRoundOutcome {
                found: false,
                new_round: 0,
                source_round: 0,
                source_kind: 0,
                block_hash: [0u8; 32],
                step: 0,
            }))
    }

    /// Returns one next-step 2t+1 voted-block mapping.
    pub fn pbft_service_verified_votes_get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotedBlockLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(self
            .0
            .verified_votes_get_two_t_plus_one_voted_block(period, round, kind)?
            .map(|value| TwoTPlusOneVotedBlockLookup {
                found: true,
                block_hash: value.block_hash.0,
                step: value.step,
            })
            .unwrap_or(TwoTPlusOneVotedBlockLookup {
                found: false,
                block_hash: [0u8; 32],
                step: 0,
            }))
    }

    /// Returns retained weighted payloads for one mapped 2t+1 voted block.
    pub fn pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotePayloadsLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(
            match self
                .0
                .verified_votes_get_two_t_plus_one_voted_block_payloads(period, round, kind)?
            {
                Some(value) => {
                    let votes = value
                        .votes
                        .into_iter()
                        .map(|vote| PbftVoteStorageRecord {
                            hash: vote.hash.0,
                            vote_rlp: vote.vote_rlp,
                        })
                        .collect();
                    TwoTPlusOneVotePayloadsLookup {
                        found: true,
                        block_hash: value.block_hash.0,
                        step: value.step,
                        votes,
                    }
                }
                None => TwoTPlusOneVotePayloadsLookup {
                    found: false,
                    block_hash: [0u8; 32],
                    step: 0,
                    votes: Vec::new(),
                },
            },
        )
    }

    /// Returns one next-vote optimized bundle egress plan.
    pub fn pbft_service_verified_votes_plan_next_votes_bundle_egress(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftNextVotesBundleEgressPlan, anyhow::Error> {
        let result: ConsensusPbftNextVotesBundleEgressPlan = self
            .0
            .verified_votes_plan_next_votes_bundle_egress(period, round)?;
        Ok(PbftNextVotesBundleEgressPlan {
            status: result.status.as_u8(),
            error_code: result.status.legacy_error_code().to_owned(),
            period: result.period,
            round: result.round,
            next_votes: optimize_bundle_plan_to_ffi(result.next_votes),
            next_null_votes: optimize_bundle_plan_to_ffi(result.next_null_votes),
        })
    }

    /// Builds one optimized verified-vote bundle by retained payload hash list.
    pub fn pbft_service_verified_votes_build_optimized_votes_bundle_egress(
        &self,
        request: PbftOptimizedVoteBundleBuildRequest,
    ) -> Result<PbftOptimizedVoteBundleBuildResult, anyhow::Error> {
        let Some(request) = optimize_bundle_request_to_domain(request) else {
            let status = rustaxa_consensus::VerifiedVoteOptimizedBundleStatus::UnsupportedKind;
            return Ok(PbftOptimizedVoteBundleBuildResult {
                status: status.as_u8(),
                error_code: status.legacy_error_code().to_owned(),
                vote_hashes: Vec::new(),
                votes_bundle_rlp: Vec::new(),
            });
        };
        self.0
            .verified_votes_build_optimized_votes_bundle_egress(request)
            .map(optimize_bundle_build_result_to_ffi)
    }

    /// Applies one bounded verified-vote cleanup pass.
    pub fn pbft_service_verified_votes_cleanup_votes_by_period(
        &self,
        pbft_period: u64,
    ) -> Result<(), anyhow::Error> {
        self.0.verified_votes_cleanup_votes_by_period(pbft_period)
    }

    /// Loads retained own verified-vote records from storage.
    pub fn pbft_service_verified_votes_own_vote_records(
        &self,
    ) -> Result<Vec<PbftVoteStorageRecord>, anyhow::Error> {
        Ok(self
            .0
            .verified_votes_own_vote_records()?
            .into_iter()
            .map(vote_storage_record_to_ffi)
            .collect())
    }

    /// Persists one latest-round own verified vote from canonical signed bytes
    /// and an authoritative nonzero weight.
    ///
    /// Rust builds and validates the weighted storage record. Codec and storage
    /// failures cross CXX as bridge errors; durable write rejection remains a
    /// typed persistence result.
    pub fn pbft_service_verified_votes_save_own_verified_vote(
        &self,
        canonical_vote_rlp: &[u8],
        weight: u64,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        self.0
            .verified_votes_save_own_verified_vote(canonical_vote_rlp, weight)
            .map(pbft_vote_persistence_to_ffi)
    }

    /// Clears all latest-round own verified votes.
    pub fn pbft_service_verified_votes_clear_own_verified_votes(
        &self,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        self.0
            .verified_votes_clear_own_verified_votes()
            .map(pbft_vote_persistence_to_ffi)
    }

    /// Persists generated vote-progress storage effects from CXX identities.
    ///
    /// The request contains no weighted vote or bundle bytes. Rust resolves the
    /// retained extra-reward payload and exact native 2t+1 mapping, constructs
    /// the canonical bundle, and returns a typed atomic-write result. Invalid
    /// raw kinds return a typed rejection; missing native state is a bridge
    /// error and no write is applied.
    pub fn pbft_service_verified_votes_persist_pbft_vote_progress(
        &self,
        write: crate::ffi::rustaxa_ffi::PbftVoteProgressPersistenceWrite,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        let write = match vote_progress_write_to_domain(write) {
            Ok(write) => write,
            Err(result) => return Ok(pbft_vote_persistence_to_ffi(result)),
        };
        self.0
            .verified_votes_persist_pbft_vote_progress(write)
            .map(pbft_vote_persistence_to_ffi)
    }

    /// Returns one coherent verified-vote snapshot.
    pub fn pbft_service_verified_votes_state_snapshot(
        &self,
    ) -> Result<VerifiedVotesStateSnapshot, anyhow::Error> {
        let snapshot: ConsensusVerifiedVotesStateSnapshot =
            self.0.verified_votes_state_snapshot()?;
        Ok(VerifiedVotesStateSnapshot {
            votes: snapshot
                .votes
                .into_iter()
                .map(|entry| VerifiedVoteStateSnapshotEntry {
                    vote: entry.vote.into(),
                    weighted_vote: PbftVoteStorageRecord {
                        hash: entry.weighted_vote.hash.0,
                        vote_rlp: entry.weighted_vote.vote_rlp,
                    },
                })
                .collect(),
            round_markers: snapshot
                .round_markers
                .into_iter()
                .map(|entry| RoundMarkerSnapshot {
                    period: entry.period,
                    round: entry.round,
                    network_t_plus_one_step: entry.network_t_plus_one_step,
                })
                .collect(),
            two_t_plus_one: snapshot
                .two_t_plus_one
                .into_iter()
                .map(|entry| TwoTPlusOneSnapshotEntry {
                    period: entry.period,
                    round: entry.round,
                    kind: entry.kind.into(),
                    block_hash: entry.block_hash.0,
                    step: entry.step,
                })
                .collect(),
        })
    }

    /// Returns one step's retained payload buckets.
    pub fn pbft_service_verified_votes_step_payloads(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> Result<VerifiedStepVotePayloadsLookup, anyhow::Error> {
        let value = self.0.verified_votes_step_payloads(period, round, step)?;
        let found = value.is_some();
        let entries = value
            .unwrap_or_default()
            .into_iter()
            .map(|entry| VerifiedStepVotePayloadEntry {
                block_hash: entry.block_hash.0,
                total_weight: entry.total_weight,
                votes: entry
                    .votes
                    .into_iter()
                    .map(|vote| PbftVoteStorageRecord {
                        hash: vote.hash.0,
                        vote_rlp: vote.vote_rlp,
                    })
                    .collect(),
            })
            .collect();

        Ok(VerifiedStepVotePayloadsLookup { found, entries })
    }
}
