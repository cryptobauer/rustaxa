use crate::ffi::rustaxa_ffi::{
    AtomicVoteInsertOutcome, DagHash, DetermineNewRoundOutcome, PbftFinalizationHash,
    PbftNextVotesBundleEgressPlan, PbftOptimizedVoteBundleBuildRequest,
    PbftOptimizedVoteBundleBuildResult, PbftOptimizedVoteBundlePlan,
    PbftRewardVotePayloadSelection as FfiPbftRewardVotePayloadSelection,
    PbftRewardVotesResetRequest as FfiPbftRewardVotesResetRequest,
    PbftTwoTPlusOneThresholdFact as FfiPbftTwoTPlusOneThresholdFact,
    PbftTwoTPlusOneThresholdPlan as FfiPbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneVoteBundle,
    PbftVoteAdmissionRuntimeResult, PbftVoteEventFactFlags, PbftVotePayloadLookup,
    PbftVoteProgressContext as FfiPbftVoteProgressContext, PbftVoteRuntimeValidationResult,
    PbftVoteStorageRecord, PbftVoteValidationExternalFacts,
    RewardVoteCursorCommitResult as FfiRewardVoteCursorCommitResult,
    RewardVoteCursorSnapshot as FfiRewardVoteCursorSnapshot, RoundMarkerSnapshot,
    ThresholdDecisionOutcome, TwoTPlusOneInsertOutcome, TwoTPlusOneSnapshotEntry,
    TwoTPlusOneVotePayloadsLookup, TwoTPlusOneVotedBlockLookup, UniqueVoterInsertOutcome,
    VerifiedStepVotesEntry, VerifiedStepVotesLookup,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
    VotedValueInsertOutcome,
};
use crate::ffi::{BridgeStorage, BridgeVerifiedVotes};
use crate::pbft_vote_progress::{context_to_domain, execution_plan_to_ffi};
use crate::pbft_vote_validation::threshold_plan_to_ffi;
use ethereum_types::{H160, H256};
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_finalization_storage_writes, apply_pbft_reward_votes_reset_storage,
    PbftFinalizationStorageWriteIntent,
    PbftFinalizationStorageWriteStage as DomainPbftFinalizationStorageWriteStage,
    PbftFinalizedPeriodApplyResult, PbftRewardVotesResetStorageRequest,
};
use rustaxa_consensus::pbft_reward_votes::PbftRewardVotesStatus;
use rustaxa_consensus::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdStatus,
};
use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactFlags as DomainPbftVoteEventFactFlags;
use rustaxa_consensus::pbft_vote_payload::build_optimized_pbft_vote_bundle;
use rustaxa_consensus::pbft_vote_storage::{
    clear_own_verified_votes, persist_pbft_vote_progress, save_own_verified_vote,
};
use rustaxa_consensus::pbft_vote_validation::{
    inspect_canonical_pbft_vote, validate_canonical_pbft_vote, PbftCanonicalVoteInspectionStatus,
    PbftCanonicalVoteValidation,
    PbftVoteValidationExternalFacts as DomainPbftVoteValidationExternalFacts,
};
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome as ConsensusAddVerifiedVoteOutcome,
    DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome, PbftVoteType,
    ThresholdDecisionOutcome as ConsensusThresholdDecisionOutcome,
    TwoTPlusOneInsertOutcome as ConsensusTwoTPlusOneInsertOutcome, TwoTPlusOneVotedBlockType,
    VerifiedVote,
};
use rustaxa_consensus::{
    PbftTwoTPlusOneVoteBundle as DomainPbftTwoTPlusOneVoteBundle, PbftVoteAdmissionRuntime,
    PbftVotePersistenceResult as DomainPbftVotePersistenceResult,
    PbftVoteProgressPersistenceWrite as DomainPbftVoteProgressPersistenceWrite,
    PbftVoteRuntimeAdmissionOutcome, PbftVoteStorageRecord as DomainPbftVoteStorageRecord,
    RewardVoteCursor, RewardVoteCursorCommitStatus,
};
use rustaxa_storage::Storage;

const PBFT_OPTIMIZED_BUNDLE_READY: u8 = 0;
const PBFT_OPTIMIZED_BUNDLE_NOT_FOUND: u8 = 1;
const PBFT_OPTIMIZED_BUNDLE_EMPTY_REQUEST: u8 = 2;
const PBFT_OPTIMIZED_BUNDLE_UNSUPPORTED_KIND: u8 = 3;
const PBFT_OPTIMIZED_BUNDLE_MAPPING_MISMATCH: u8 = 4;
const PBFT_OPTIMIZED_BUNDLE_HASH_NOT_IN_PLAN: u8 = 5;
const PBFT_OPTIMIZED_BUNDLE_ORDER_MISMATCH: u8 = 6;
const PBFT_OPTIMIZED_BUNDLE_MISSING_PAYLOAD: u8 = 7;
const PBFT_OPTIMIZED_BUNDLE_PAYLOAD_DECODE_ERROR: u8 = 8;
const PBFT_OPTIMIZED_BUNDLE_PAYLOAD_METADATA_MISMATCH: u8 = 9;

#[cfg(test)]
fn create_empty_verified_votes_for_test() -> Box<BridgeVerifiedVotes> {
    Box::new(BridgeVerifiedVotes {
        runtime: PbftVoteAdmissionRuntime::new(),
        storage: None,
    })
}

/// Creates a production verified-votes runtime restored from Rust storage.
///
/// Storage is cloned into the returned handle only after all persisted vote
/// families have been decoded and validated. Any malformed vote, inconsistent
/// typed `2t+1` bundle, or uniqueness conflict fails construction without
/// exposing a partially restored runtime.
pub fn create_verified_votes_index_from_storage(
    storage: &BridgeStorage,
) -> Result<Box<BridgeVerifiedVotes>, anyhow::Error> {
    let runtime = PbftVoteAdmissionRuntime::restore_from_storage(storage.0.as_ref())?;
    Ok(Box::new(BridgeVerifiedVotes {
        runtime,
        storage: Some(storage.0.clone()),
    }))
}

fn verified_votes_storage(runtime: &BridgeVerifiedVotes) -> Result<&Storage, anyhow::Error> {
    runtime
        .storage
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("VERIFIED_VOTES_STORAGE_UNAVAILABLE"))
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

fn vote_storage_record_to_domain(value: PbftVoteStorageRecord) -> DomainPbftVoteStorageRecord {
    DomainPbftVoteStorageRecord {
        hash: H256::from(value.hash),
        vote_rlp: value.vote_rlp,
    }
}

fn validate_own_vote_storage_record(
    record: DomainPbftVoteStorageRecord,
) -> Result<DomainPbftVoteStorageRecord, anyhow::Error> {
    let inspection = inspect_canonical_pbft_vote(&record.vote_rlp)?;
    anyhow::ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid
            && inspection.signature_valid
            && inspection.has_embedded_weight
            && inspection.embedded_weight > 0,
        "VERIFIED_VOTES_OWN_VOTE_PAYLOAD_INVALID"
    );
    anyhow::ensure!(
        inspection.vote_hash == record.hash,
        "VERIFIED_VOTES_OWN_VOTE_HASH_MISMATCH"
    );
    Ok(record)
}

fn two_t_plus_one_bundle_to_domain(
    value: PbftTwoTPlusOneVoteBundle,
) -> DomainPbftTwoTPlusOneVoteBundle {
    DomainPbftTwoTPlusOneVoteBundle {
        kind: value.kind,
        period: value.period,
        round: value.round,
        step: value.step,
        block_hash: H256::from(value.block_hash),
        votes_bundle_rlp: value.votes_bundle_rlp,
    }
}

fn vote_progress_write_to_domain(
    value: crate::ffi::rustaxa_ffi::PbftVoteProgressPersistenceWrite,
) -> DomainPbftVoteProgressPersistenceWrite {
    DomainPbftVoteProgressPersistenceWrite {
        extra_reward_vote: value
            .has_extra_reward_vote
            .then(|| vote_storage_record_to_domain(value.extra_reward_vote)),
        two_t_plus_one_bundle: value
            .has_two_t_plus_one_bundle
            .then(|| two_t_plus_one_bundle_to_domain(value.two_t_plus_one_bundle)),
    }
}

impl BridgeVerifiedVotes {
    fn prepare_reward_votes_reset_bundle(
        &self,
        period: u64,
        round: u64,
        step: u64,
        block_hash: H256,
    ) -> Result<Vec<u8>, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::CertVotedBlock;
        let mapping = self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
            .ok_or_else(|| anyhow::anyhow!("PBFT_REWARD_VOTES_RESET_CERT_MAPPING_MISSING"))?;
        anyhow::ensure!(
            mapping.hash == block_hash && mapping.step == step,
            "PBFT_REWARD_VOTES_RESET_CERT_IDENTITY_MISMATCH"
        );
        let records = self
            .runtime
            .two_t_plus_one_weighted_payloads(period, round, kind)?
            .ok_or_else(|| anyhow::anyhow!("PBFT_REWARD_VOTES_RESET_CERT_MAPPING_MISSING"))?;
        rustaxa_consensus::build_weighted_pbft_vote_bundle(&records)
    }

    /// Loads locally generated weighted vote records from native Rust storage.
    ///
    /// Records are returned in canonical hash-key order. Every row is decoded
    /// as a signed weighted PBFT vote and its decoded canonical hash must equal
    /// the RocksDB key; malformed payloads, invalid signatures, zero weights,
    /// non-32-byte keys, and key/payload mismatches fail the entire lookup.
    pub fn verified_votes_own_vote_records(
        &self,
    ) -> Result<Vec<PbftVoteStorageRecord>, anyhow::Error> {
        let storage = verified_votes_storage(self)?;
        let _guard = storage.lock_own_verified_votes()?;
        storage
            .pbft()
            .own_verified_vote_records()?
            .into_iter()
            .map(|record| {
                let record = validate_own_vote_storage_record(DomainPbftVoteStorageRecord {
                    hash: record.vote_hash,
                    vote_rlp: record.vote_rlp,
                })?;
                Ok(PbftVoteStorageRecord {
                    hash: record.hash.0,
                    vote_rlp: record.vote_rlp,
                })
            })
            .collect()
    }

    /// Returns count of stored verified vote hashes.
    pub fn verified_votes_size(&self) -> u64 {
        self.runtime.verified_votes().size()
    }

    /// Returns whether `vote_hash` is in runtime-owned validation replay protection.
    pub fn verified_votes_replay_contains(&self, vote_hash: &[u8; 32]) -> bool {
        self.runtime.replay_contains(H256::from(*vote_hash))
    }

    /// Inserts `vote_hash` into runtime-owned validation replay protection.
    pub fn verified_votes_replay_insert(&mut self, vote_hash: &[u8; 32]) -> bool {
        self.runtime.replay_insert(H256::from(*vote_hash))
    }

    /// Returns a Rust-owned PBFT `2t+1` threshold plan.
    pub fn verified_votes_two_t_plus_one_threshold(
        &mut self,
        fact: FfiPbftTwoTPlusOneThresholdFact,
    ) -> FfiPbftTwoTPlusOneThresholdPlan {
        let vote_type = match PbftVoteType::try_from(fact.vote_type) {
            Ok(vote_type) => vote_type,
            Err(_) => {
                return threshold_plan_to_ffi(PbftTwoTPlusOneThresholdPlan {
                    status: PbftTwoTPlusOneThresholdStatus::InvalidVoteType,
                    error_code: "PBFT_TWO_T_PLUS_ONE_INVALID_VOTE_TYPE",
                    has_threshold: false,
                    threshold: 0,
                    sortition_threshold: 0,
                    needs_total_dpos_votes: false,
                    cache_hit: false,
                    cached: false,
                });
            }
        };

        threshold_plan_to_ffi(self.runtime.plan_two_t_plus_one_threshold(
            PbftTwoTPlusOneThresholdFact {
                pbft_period: fact.pbft_period,
                vote_type,
                current_pbft_chain_size: fact.current_pbft_chain_size,
                committee_size: fact.committee_size,
                number_of_proposers: fact.number_of_proposers,
                has_total_dpos_votes_count: fact.has_total_dpos_votes_count,
                total_dpos_votes_count: fact.total_dpos_votes_count,
                future_dpos_state: fact.future_dpos_state,
                unknown_error: fact.unknown_error,
            },
        ))
    }

    /// Validates canonical PBFT vote bytes and mutates runtime replay state
    /// when Rust validation requests replay protection.
    pub fn verified_votes_validate_canonical_vote(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation_facts: PbftVoteValidationExternalFacts,
    ) -> Result<PbftVoteRuntimeValidationResult, anyhow::Error> {
        let validation = validate_canonical_pbft_vote(
            canonical_vote_rlp,
            validation_facts_to_domain(validation_facts),
        )?;
        let replay = self.runtime.record_validation_replay(&validation);
        Ok(PbftVoteRuntimeValidationResult {
            status: validation.status.as_u8(),
            error_code: validation.error_code.to_owned(),
            accepted: validation.accepted,
            rejected: validation.rejected,
            validation: validation.into(),
            replay_should_mark: replay.should_mark,
            replay_inserted: replay.inserted,
            replay_already_present: replay.already_present,
        })
    }

    /// Inserts `vote` into unique-voter tracking.
    pub fn verified_votes_insert_unique_voter(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<UniqueVoterInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.runtime.verified_votes_mut().insert_unique_voter(&vote);
        Ok(UniqueVoterInsertOutcome {
            accepted: outcome.accepted,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
    }

    /// Inserts `vote` into voted-value aggregation.
    ///
    /// Compatibility/test helper only; production admission must retain the
    /// canonical vote payload sidecars through `verified_votes_admit_validated_vote`.
    pub fn verified_votes_insert_voted_value(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<VotedValueInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        Ok(self
            .runtime
            .verified_votes_mut()
            .insert_voted_value(vote)?
            .into())
    }

    /// Atomically inserts `vote` into unique-voter and voted-value state.
    ///
    /// This returns conflict details for slashing decisions when uniqueness
    /// fails and voted-value aggregation counters when insertion succeeds.
    /// Compatibility/test helper only; production routing should not bypass the
    /// canonical admission runtime because threshold bundles and slashing
    /// evidence require retained payload records.
    pub fn verified_votes_insert_vote_atomic(
        &mut self,
        vote: VerifiedVotePayload,
    ) -> Result<AtomicVoteInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.runtime.verified_votes_mut().insert_vote_atomic(vote)?;
        Ok(AtomicVoteInsertOutcome {
            inserted: outcome.inserted,
            total_weight: outcome.total_weight,
            votes_count: outcome.votes_count,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
    }

    /// Applies deterministic threshold decisions to verified-votes state.
    ///
    /// The caller provides `total_weight` for vote's voted-value bucket and
    /// `two_t_plus_one_threshold` for this vote type/period.
    /// Compatibility/test helper only. Production admission applies threshold
    /// decisions inside `verified_votes_admit_validated_vote` so bundle
    /// persistence can use Rust-retained weighted payloads.
    pub fn verified_votes_apply_threshold_decision(
        &mut self,
        vote: VerifiedVotePayload,
        total_weight: u64,
        two_t_plus_one_threshold: u64,
    ) -> Result<ThresholdDecisionOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let outcome = self.runtime.verified_votes_mut().apply_threshold_decision(
            &vote,
            total_weight,
            two_t_plus_one_threshold,
        )?;
        Ok(outcome.into())
    }

    /// Sets network t+1 step marker for one round.
    pub fn verified_votes_set_network_t_plus_one_step(
        &mut self,
        period: u64,
        round: u64,
        step: u64,
    ) -> bool {
        self.runtime
            .verified_votes_mut()
            .set_network_t_plus_one_step(period, round, step)
    }

    /// Determines next round from Rust-owned next-vote 2t+1 mappings.
    pub fn verified_votes_determine_new_round(
        &self,
        period: u64,
        current_round: u64,
    ) -> DetermineNewRoundOutcome {
        self.runtime
            .verified_votes()
            .determine_new_round(period, current_round)
            .map(Into::into)
            .unwrap_or(DetermineNewRoundOutcome {
                found: false,
                new_round: 0,
                source_round: 0,
                source_kind: 0,
                block_hash: [0u8; 32],
                step: 0,
            })
    }

    /// Inserts one 2t+1 voted-block mapping for existing round.
    pub fn verified_votes_insert_two_t_plus_one_voted_block(
        &mut self,
        period: u64,
        round: u64,
        kind: u8,
        block_hash: &[u8; 32],
        step: u64,
    ) -> Result<TwoTPlusOneInsertOutcome, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(self
            .runtime
            .verified_votes_mut()
            .insert_two_t_plus_one_voted_block(period, round, kind, H256::from(*block_hash), step)
            .into())
    }

    /// Gets one 2t+1 voted-block mapping.
    pub fn verified_votes_get_two_t_plus_one_voted_block(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotedBlockLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        Ok(self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
            .map(|value| TwoTPlusOneVotedBlockLookup {
                found: true,
                block_hash: value.hash.into(),
                step: value.step,
            })
            .unwrap_or(TwoTPlusOneVotedBlockLookup {
                found: false,
                block_hash: [0u8; 32],
                step: 0,
            }))
    }

    /// Gets retained weighted vote payloads for one mapped 2t+1 voted block.
    ///
    /// The payloads are returned in the same deterministic order as the
    /// verified-vote hash lookup. C++ may materialize temporary live sidecars
    /// from the weighted RLP bytes, but missing retained payloads are reported
    /// as hard errors because Rust owns both metadata and payload retention.
    pub fn verified_votes_get_two_t_plus_one_voted_block_payloads(
        &self,
        period: u64,
        round: u64,
        kind: u8,
    ) -> Result<TwoTPlusOneVotePayloadsLookup, anyhow::Error> {
        let kind = TwoTPlusOneVotedBlockType::try_from(kind)?;
        let voted = self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind);
        let Some(voted) = voted else {
            return Ok(TwoTPlusOneVotePayloadsLookup {
                found: false,
                block_hash: [0u8; 32],
                step: 0,
                votes: Vec::new(),
            });
        };

        let votes = self
            .runtime
            .two_t_plus_one_weighted_payloads(period, round, kind)?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "PBFT vote runtime lost 2t+1 mapping while resolving retained payloads"
                )
            })?
            .into_iter()
            .map(Into::into)
            .collect();

        Ok(TwoTPlusOneVotePayloadsLookup {
            found: true,
            block_hash: voted.hash.into(),
            step: voted.step,
            votes,
        })
    }

    /// Plans optimized PBFT next-vote bundle egress for a previous round.
    ///
    /// Rust selects ordered vote hashes for the next and next-null 2t+1
    /// mappings. C++ uses this plan for peer-known filtering and chunking, then
    /// calls `verified_votes_build_optimized_votes_bundle_egress` for each
    /// chunk that should be sent.
    pub fn verified_votes_plan_next_votes_bundle_egress(
        &self,
        period: u64,
        round: u64,
    ) -> PbftNextVotesBundleEgressPlan {
        PbftNextVotesBundleEgressPlan {
            status: PBFT_OPTIMIZED_BUNDLE_READY,
            error_code: "PBFT_OPTIMIZED_VOTE_BUNDLE_READY".to_owned(),
            period,
            round,
            next_votes: self.optimized_vote_bundle_plan(
                period,
                round,
                TwoTPlusOneVotedBlockType::NextVotedBlock,
            ),
            next_null_votes: self.optimized_vote_bundle_plan(
                period,
                round,
                TwoTPlusOneVotedBlockType::NextVotedNullBlock,
            ),
        }
    }

    /// Builds one peer-filtered optimized PBFT votes bundle from retained payloads.
    ///
    /// The request hashes must be a non-empty ordered subsequence of the Rust
    /// 2t+1 plan for the requested kind. The output is the inner optimized
    /// votes-bundle RLP; C++ remains responsible for tarcap packet wrapping,
    /// peer send policy, and marking sent hashes known.
    pub fn verified_votes_build_optimized_votes_bundle_egress(
        &self,
        request: PbftOptimizedVoteBundleBuildRequest,
    ) -> PbftOptimizedVoteBundleBuildResult {
        let kind = match TwoTPlusOneVotedBlockType::try_from(request.kind) {
            Ok(kind) => kind,
            Err(_) => {
                return optimized_bundle_build_result(
                    PBFT_OPTIMIZED_BUNDLE_UNSUPPORTED_KIND,
                    "PBFT_OPTIMIZED_VOTE_BUNDLE_UNSUPPORTED_KIND",
                    Vec::new(),
                    Vec::new(),
                );
            }
        };

        let requested_hashes: Vec<H256> = request
            .vote_hashes
            .iter()
            .map(|hash| H256::from(hash.hash))
            .collect();
        if requested_hashes.is_empty() {
            return optimized_bundle_build_result(
                PBFT_OPTIMIZED_BUNDLE_EMPTY_REQUEST,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_EMPTY_REQUEST",
                Vec::new(),
                Vec::new(),
            );
        }

        let Some(voted) = self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(request.period, request.round, kind)
        else {
            return optimized_bundle_build_result(
                PBFT_OPTIMIZED_BUNDLE_NOT_FOUND,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_NOT_FOUND",
                requested_hashes,
                Vec::new(),
            );
        };

        if voted.hash != H256::from(request.block_hash) || voted.step != request.step {
            return optimized_bundle_build_result(
                PBFT_OPTIMIZED_BUNDLE_MAPPING_MISMATCH,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_MAPPING_MISMATCH",
                requested_hashes,
                Vec::new(),
            );
        }

        let planned_hashes = self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block_vote_hashes(request.period, request.round, kind);
        if let Err(status) = ensure_ordered_subsequence(&planned_hashes, &requested_hashes) {
            return optimized_bundle_build_result(
                status,
                optimized_bundle_status_error_code(status),
                requested_hashes,
                Vec::new(),
            );
        }

        let mut records = Vec::with_capacity(requested_hashes.len());
        for vote_hash in &requested_hashes {
            let Some(record) = self.runtime.weighted_payload(*vote_hash).cloned() else {
                return optimized_bundle_build_result(
                    PBFT_OPTIMIZED_BUNDLE_MISSING_PAYLOAD,
                    "PBFT_OPTIMIZED_VOTE_BUNDLE_MISSING_RETAINED_PAYLOAD",
                    requested_hashes,
                    Vec::new(),
                );
            };
            records.push(record);
        }

        match build_optimized_pbft_vote_bundle(
            &records,
            voted.hash,
            request.period,
            request.round,
            voted.step,
        ) {
            Ok(bundle) => optimized_bundle_build_result(
                PBFT_OPTIMIZED_BUNDLE_READY,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_READY",
                bundle.vote_hashes,
                bundle.bundle_rlp,
            ),
            Err(err) if err.to_string().contains("mismatches requested metadata") => {
                optimized_bundle_build_result(
                    PBFT_OPTIMIZED_BUNDLE_PAYLOAD_METADATA_MISMATCH,
                    "PBFT_OPTIMIZED_VOTE_BUNDLE_PAYLOAD_METADATA_MISMATCH",
                    requested_hashes,
                    Vec::new(),
                )
            }
            Err(_) => optimized_bundle_build_result(
                PBFT_OPTIMIZED_BUNDLE_PAYLOAD_DECODE_ERROR,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_PAYLOAD_DECODE_ERROR",
                requested_hashes,
                Vec::new(),
            ),
        }
    }

    /// Returns all voted values and their vote hashes for one step.
    pub fn verified_votes_get_step_votes(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> VerifiedStepVotesLookup {
        let Some(step_votes) = self
            .runtime
            .verified_votes()
            .get_step_votes(period, round, step)
        else {
            return VerifiedStepVotesLookup {
                found: false,
                entries: Vec::new(),
            };
        };

        VerifiedStepVotesLookup {
            found: true,
            entries: step_votes
                .into_iter()
                .map(|entry| VerifiedStepVotesEntry {
                    block_hash: entry.block_hash.into(),
                    total_weight: entry.total_weight,
                    vote_hashes: entry
                        .vote_hashes
                        .into_iter()
                        .map(|vote_hash| DagHash {
                            hash: vote_hash.into(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// Adds one vote fact with optional threshold side effects.
    pub fn verified_votes_add_verified_vote(
        &mut self,
        vote: VerifiedVotePayload,
        two_t_plus_one_threshold: u64,
        apply_threshold_decision: bool,
    ) -> Result<FfiVerifiedVoteAddOutcome, anyhow::Error> {
        let report_vote = copy_vote_payload(&vote);
        let vote = payload_to_vote(vote)?;
        let threshold = if apply_threshold_decision {
            Some(two_t_plus_one_threshold)
        } else {
            None
        };

        let outcome = self
            .runtime
            .verified_votes_mut()
            .add_verified_vote(vote, threshold)?;
        Ok(outcome_to_ffi_add_vote_outcome(report_vote, outcome))
    }

    /// Runs one validation-backed PBFT vote admission transition.
    ///
    /// Rust validates canonical vote bytes from caller-supplied external facts,
    /// records vote payload sidecars, mutates the single Rust verified-vote
    /// index owned by this bridge handle, and returns explicit executor
    /// effects for the C++ VoteManager shim.
    pub fn verified_votes_admit_validated_vote(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation_facts: PbftVoteValidationExternalFacts,
        flags: PbftVoteEventFactFlags,
        context: FfiPbftVoteProgressContext,
    ) -> Result<PbftVoteAdmissionRuntimeResult, anyhow::Error> {
        let validation = validate_canonical_pbft_vote(
            canonical_vote_rlp,
            validation_facts_to_domain(validation_facts),
        )?;
        let outcome = self.runtime.admit_validated_vote(
            canonical_vote_rlp,
            &validation,
            flags_to_domain(flags),
            context_to_domain(&context),
        )?;
        Ok(runtime_outcome_to_ffi(validation, outcome, context))
    }

    /// Removes periods lower than `pbft_period`.
    pub fn verified_votes_cleanup_votes_by_period(&mut self, pbft_period: u64) {
        self.runtime.cleanup_votes_by_period(pbft_period);
    }

    /// Returns deterministic flat vote snapshot.
    pub fn verified_votes_snapshot_votes(&self) -> Vec<VerifiedVotePayload> {
        self.runtime
            .verified_votes()
            .snapshot_votes()
            .into_iter()
            .map(Into::into)
            .collect()
    }

    /// Returns one retained weighted PBFT vote payload by canonical vote hash.
    pub fn verified_votes_weighted_payload(&self, vote_hash: &[u8; 32]) -> PbftVotePayloadLookup {
        let Some(vote) = self
            .runtime
            .weighted_payload(H256::from(*vote_hash))
            .cloned()
        else {
            return PbftVotePayloadLookup {
                found: false,
                vote: empty_storage_record(),
            };
        };
        PbftVotePayloadLookup {
            found: true,
            vote: vote.into(),
        }
    }

    /// Selects PBFT reward votes from Rust-owned metadata and retained payloads.
    ///
    /// This is the production bridge for `VoteManager::checkRewardVotes`: Rust
    /// builds preferred/reverse-round candidates from the verified-vote runtime,
    /// applies the reward planner, and resolves retained weighted records in
    /// PBFT-block requested-hash order. Metadata-only compatibility helper
    /// inserts that lack retained payloads are rejected as invariant errors.
    pub fn verified_votes_select_reward_vote_payloads(
        &self,
        block_period: u64,
        requested_vote_hashes: Vec<PbftFinalizationHash>,
    ) -> Result<FfiPbftRewardVotePayloadSelection, anyhow::Error> {
        let selection = self.runtime.select_reward_vote_payloads(
            block_period,
            requested_vote_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        )?;
        Ok(FfiPbftRewardVotePayloadSelection {
            accepted: selection.accepted,
            status: selection.status.as_u8(),
            error_code: reward_error_code(selection.status).to_owned(),
            selected_period: selection.selected_period,
            selected_round: selection.selected_round,
            selected_block_hash: selection.selected_block_hash.into(),
            selected_vote_hashes: selection
                .selected_vote_hashes
                .into_iter()
                .map(|hash| PbftFinalizationHash { hash: hash.into() })
                .collect(),
            selected_records: selection
                .selected_records
                .into_iter()
                .map(Into::into)
                .collect(),
            missing_vote_hash: selection.missing_vote_hash.unwrap_or_default().into(),
        })
    }

    /// Returns the Rust-owned finalized reward-vote cursor snapshot.
    ///
    /// The snapshot is absent before the first durable reward reset. Callers
    /// receive only scalar compatibility facts and cannot mutate cursor state.
    pub fn verified_votes_reward_vote_cursor(&self) -> FfiRewardVoteCursorSnapshot {
        self.runtime
            .reward_vote_cursor()
            .map(|cursor| FfiRewardVoteCursorSnapshot {
                found: true,
                period: cursor.period,
                round: cursor.round,
                step: cursor.step,
                block_hash: cursor.block_hash.into(),
            })
            .unwrap_or(FfiRewardVoteCursorSnapshot {
                found: false,
                period: 0,
                round: 0,
                step: 0,
                block_hash: [0; 32],
            })
    }

    /// Returns the finalized reward-vote period, or zero when no cursor exists.
    pub fn verified_votes_reward_vote_period(&self) -> u64 {
        self.runtime.reward_vote_period()
    }

    /// Returns canonical weighted payloads for the exact finalized reward cursor.
    ///
    /// Missing retained payloads for an installed cursor are reported as an
    /// invariant error; an absent cursor returns an empty list.
    pub fn verified_votes_current_reward_vote_payloads(
        &self,
    ) -> Result<Vec<PbftVoteStorageRecord>, anyhow::Error> {
        Ok(self
            .runtime
            .current_reward_vote_payloads()?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    /// Publishes an already-durable reward cursor into the live Rust runtime.
    ///
    /// The reset generation must identify the active storage-committed reset.
    /// Rust revalidates the cursor mapping, retained payloads, and durable
    /// finalized cursor before applying a monotonic or idempotent update.
    pub fn verified_votes_commit_reward_vote_cursor(
        &mut self,
        write_intent: &crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan,
        reset_generation: u64,
    ) -> Result<FfiRewardVoteCursorCommitResult, anyhow::Error> {
        let cursor = RewardVoteCursor {
            period: write_intent.reward_vote_period,
            round: write_intent.reward_vote_round,
            step: write_intent.reward_vote_step,
            block_hash: H256::from(write_intent.reward_vote_block_hash),
        };
        let storage = self
            .storage
            .clone()
            .ok_or_else(|| anyhow::anyhow!("VERIFIED_VOTES_STORAGE_UNAVAILABLE"))?;
        let result = self
            .runtime
            .commit_reward_vote_cursor(&storage, cursor, reset_generation)?;
        Ok(FfiRewardVoteCursorCommitResult {
            status: match result.status {
                RewardVoteCursorCommitStatus::Applied => 0,
                RewardVoteCursorCommitStatus::AlreadyCurrent => 1,
                RewardVoteCursorCommitStatus::Rejected => 2,
            },
            period: result.cursor.period,
            round: result.cursor.round,
            step: result.cursor.step,
            block_hash: result.cursor.block_hash.into(),
            reset_generation: result.reset_generation,
            error_code: result.error_code.to_owned(),
        })
    }

    /// Returns deterministic 2t+1 mapping snapshot.
    pub fn verified_votes_snapshot_two_t_plus_one(&self) -> Vec<TwoTPlusOneSnapshotEntry> {
        self.runtime
            .verified_votes()
            .snapshot_two_t_plus_one()
            .into_iter()
            .map(|entry| TwoTPlusOneSnapshotEntry {
                period: entry.period,
                round: entry.round,
                kind: entry.kind.into(),
                block_hash: entry.block_hash.into(),
                step: entry.step,
            })
            .collect()
    }

    /// Returns deterministic round marker snapshot.
    pub fn verified_votes_snapshot_round_markers(&self) -> Vec<RoundMarkerSnapshot> {
        self.runtime
            .verified_votes()
            .snapshot_round_markers()
            .into_iter()
            .map(|entry| RoundMarkerSnapshot {
                period: entry.period,
                round: entry.round,
                network_t_plus_one_step: entry.network_t_plus_one_step,
            })
            .collect()
    }

    /// Persists one own verified vote through this runtime's attached Rust storage.
    ///
    /// The weighted signed payload must decode with a non-zero embedded weight,
    /// and its canonical vote hash must equal the supplied storage key. Invalid
    /// records fail before a native Rust batch is created or committed.
    /// A storage-owned mutex serializes the complete save with lifecycle/direct
    /// clears and `BridgeVerifiedVotes` production queries sharing the storage.
    pub fn verified_votes_save_own_verified_vote(
        &self,
        record: PbftVoteStorageRecord,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        save_own_verified_vote(
            verified_votes_storage(self)?,
            validate_own_vote_storage_record(vote_storage_record_to_domain(record))?,
        )
        .map(pbft_vote_persistence_to_ffi)
    }

    /// Clears all own verified votes through attached native Rust storage.
    ///
    /// The zero-input operation enumerates authoritative storage keys itself;
    /// no caller-provided sidecar list can leave a row behind.
    /// A storage-owned mutex serializes enumeration and commit with local saves
    /// and `BridgeVerifiedVotes` production queries sharing the storage.
    pub fn verified_votes_clear_own_verified_votes(
        &self,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        clear_own_verified_votes(verified_votes_storage(self)?).map(pbft_vote_persistence_to_ffi)
    }

    /// Persists accepted PBFT vote-progress effects through attached Rust storage.
    pub fn verified_votes_persist_pbft_vote_progress(
        &self,
        write: crate::ffi::rustaxa_ffi::PbftVoteProgressPersistenceWrite,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        persist_pbft_vote_progress(
            verified_votes_storage(self)?,
            vote_progress_write_to_domain(write),
        )
        .map(pbft_vote_persistence_to_ffi)
    }

    /// Applies PBFT finalization storage stages through attached Rust storage.
    ///
    /// Reward-reset stages never trust caller-provided delete keys. The Rust
    /// storage executor locks and enumerates authoritative extra-reward rows
    /// immediately before constructing and committing its batch.
    pub fn verified_votes_apply_pbft_finalization_storage_writes(
        &self,
        write_intent: &crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan,
        stages: Vec<crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage>,
        sync: bool,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult, anyhow::Error> {
        let domain_intent = PbftFinalizationStorageWriteIntent::from(write_intent);
        apply_pbft_finalization_storage_writes(
            verified_votes_storage(self)?,
            &domain_intent,
            stages
                .into_iter()
                .map(DomainPbftFinalizationStorageWriteStage::from)
                .collect(),
            sync,
        )
        .map(pbft_finalization_apply_result_to_ffi)
    }

    /// Builds the canonical cert-vote bundle for a reward-vote reset stage.
    ///
    /// The supplied finalization intent is only an identity assertion. Its
    /// reward period, round, step, and block hash must match the Rust-owned cert
    /// `2t+1` mapping exactly. The returned stage contains canonical retained
    /// weighted payloads and no caller-selected extra-reward delete keys;
    /// authoritative keys are injected under the storage lock at apply time.
    pub fn verified_votes_prepare_reward_votes_reset_stage(
        &self,
        write_intent: &crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage, anyhow::Error> {
        anyhow::ensure!(
            write_intent.reset_reward_votes,
            "PBFT_REWARD_VOTES_RESET_NOT_REQUESTED"
        );
        let reward_votes_bundle_rlp = self.prepare_reward_votes_reset_bundle(
            write_intent.reward_vote_period,
            write_intent.reward_vote_round,
            write_intent.reward_vote_step,
            H256::from(write_intent.reward_vote_block_hash),
        )?;
        Ok(crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage {
            stage: 4,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            has_sortition_params_change: false,
            sortition_params_change_period: 0,
            sortition_params_change_interval_efficiency: 0,
            sortition_params_change_threshold_upper: 0,
            has_reward_votes_reset: true,
            reward_votes_bundle_rlp,
            has_prepared_pillar_block: false,
            prepared_pillar_block_period: 0,
            prepared_pillar_block_rlp: Vec::new(),
        })
    }

    /// Applies reward-vote reset persistence through a task-specific Rust port.
    ///
    /// Rust derives the canonical bundle from its cert mapping and enumerates
    /// authoritative extra-reward keys under the shared storage lock at apply
    /// time; the caller supplies identity and commit durability only.
    pub fn verified_votes_apply_reward_votes_reset(
        &self,
        request: FfiPbftRewardVotesResetRequest,
    ) -> Result<crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult, anyhow::Error> {
        let reward_votes_bundle_rlp = self.prepare_reward_votes_reset_bundle(
            request.period,
            request.round,
            request.step,
            H256::from(request.block_hash),
        )?;
        apply_pbft_reward_votes_reset_storage(
            verified_votes_storage(self)?,
            PbftRewardVotesResetStorageRequest {
                period: request.period,
                round: request.round,
                step: request.step,
                block_hash: H256::from(request.block_hash),
                reward_votes_bundle_rlp,
            },
            request.sync,
        )
        .map(pbft_finalization_apply_result_to_ffi)
    }

    fn optimized_vote_bundle_plan(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> PbftOptimizedVoteBundlePlan {
        let Some(voted) = self
            .runtime
            .verified_votes()
            .get_two_t_plus_one_voted_block(period, round, kind)
        else {
            return optimized_bundle_plan(
                false,
                PBFT_OPTIMIZED_BUNDLE_NOT_FOUND,
                "PBFT_OPTIMIZED_VOTE_BUNDLE_NOT_FOUND",
                kind.into(),
                H256::zero(),
                period,
                round,
                0,
                Vec::new(),
            );
        };

        optimized_bundle_plan(
            true,
            PBFT_OPTIMIZED_BUNDLE_READY,
            "PBFT_OPTIMIZED_VOTE_BUNDLE_READY",
            kind.into(),
            voted.hash,
            period,
            round,
            voted.step,
            self.runtime
                .verified_votes()
                .get_two_t_plus_one_voted_block_vote_hashes(period, round, kind),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn optimized_bundle_plan(
    found: bool,
    status: u8,
    error_code: &str,
    kind: u8,
    block_hash: H256,
    period: u64,
    round: u64,
    step: u64,
    vote_hashes: Vec<H256>,
) -> PbftOptimizedVoteBundlePlan {
    PbftOptimizedVoteBundlePlan {
        found,
        status,
        error_code: error_code.to_owned(),
        kind,
        block_hash: block_hash.into(),
        period,
        round,
        step,
        vote_hashes: vote_hashes
            .into_iter()
            .map(|hash| PbftFinalizationHash { hash: hash.into() })
            .collect(),
    }
}

fn optimized_bundle_build_result(
    status: u8,
    error_code: &str,
    vote_hashes: Vec<H256>,
    votes_bundle_rlp: Vec<u8>,
) -> PbftOptimizedVoteBundleBuildResult {
    PbftOptimizedVoteBundleBuildResult {
        status,
        error_code: error_code.to_owned(),
        vote_hashes: vote_hashes
            .into_iter()
            .map(|hash| PbftFinalizationHash { hash: hash.into() })
            .collect(),
        votes_bundle_rlp,
    }
}

fn optimized_bundle_status_error_code(status: u8) -> &'static str {
    match status {
        PBFT_OPTIMIZED_BUNDLE_HASH_NOT_IN_PLAN => "PBFT_OPTIMIZED_VOTE_BUNDLE_HASH_NOT_IN_PLAN",
        PBFT_OPTIMIZED_BUNDLE_ORDER_MISMATCH => "PBFT_OPTIMIZED_VOTE_BUNDLE_ORDER_MISMATCH",
        _ => "PBFT_OPTIMIZED_VOTE_BUNDLE_ERROR",
    }
}

fn ensure_ordered_subsequence(
    planned_hashes: &[H256],
    requested_hashes: &[H256],
) -> Result<(), u8> {
    let mut cursor = 0;
    for requested_hash in requested_hashes {
        let Some(relative_position) = planned_hashes[cursor..]
            .iter()
            .position(|planned_hash| planned_hash == requested_hash)
        else {
            if planned_hashes[..cursor].contains(requested_hash) {
                return Err(PBFT_OPTIMIZED_BUNDLE_ORDER_MISMATCH);
            }
            return Err(PBFT_OPTIMIZED_BUNDLE_HASH_NOT_IN_PLAN);
        };
        cursor += relative_position + 1;
    }
    Ok(())
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

fn validation_facts_to_domain(
    value: PbftVoteValidationExternalFacts,
) -> DomainPbftVoteValidationExternalFacts {
    DomainPbftVoteValidationExternalFacts {
        voter_dpos_ready: value.voter_dpos_ready,
        voter_dpos_vote_count: value.voter_dpos_vote_count,
        total_dpos_ready: value.total_dpos_ready,
        total_dpos_vote_count: value.total_dpos_vote_count,
        future_dpos_state: value.future_dpos_state,
        unknown_error: value.unknown_error,
        vrf_key_ready: value.vrf_key_ready,
        has_vrf_key: value.has_vrf_key,
        vrf_public_key: value.vrf_public_key,
        strict_vrf: value.strict_vrf,
        committee_size: value.committee_size,
        number_of_proposers: value.number_of_proposers,
        has_preverified_weight: value.has_preverified_weight,
        preverified_weight: value.preverified_weight,
    }
}

const fn reward_error_code(status: PbftRewardVotesStatus) -> &'static str {
    match status {
        PbftRewardVotesStatus::FirstPeriod | PbftRewardVotesStatus::Accepted => "",
        PbftRewardVotesStatus::MissingPreferredRound => "PBFT_REWARD_VOTES_MISSING_PREFERRED_ROUND",
        PbftRewardVotesStatus::MissingRewardPeriod => "PBFT_REWARD_VOTES_MISSING_REWARD_PERIOD",
        PbftRewardVotesStatus::MissingCertStep => "PBFT_REWARD_VOTES_MISSING_CERT_STEP",
        PbftRewardVotesStatus::MissingRewardBlock => "PBFT_REWARD_VOTES_MISSING_REWARD_BLOCK",
        PbftRewardVotesStatus::MissingRewardVote => "PBFT_REWARD_VOTES_MISSING_REWARD_VOTE",
        PbftRewardVotesStatus::MissingRetainedPayload => {
            "PBFT_REWARD_VOTES_MISSING_RETAINED_PAYLOAD"
        }
    }
}

fn runtime_outcome_to_ffi(
    validation: PbftCanonicalVoteValidation,
    outcome: PbftVoteRuntimeAdmissionOutcome,
    context: FfiPbftVoteProgressContext,
) -> PbftVoteAdmissionRuntimeResult {
    let progress_fact = outcome.precheck.progress_fact;
    let empty_vote = empty_vote_payload();
    let vote = progress_fact
        .map(progress_fact_to_vote)
        .unwrap_or(empty_vote);
    let progress = outcome.execution.as_ref().map(|execution| {
        execution_plan_to_ffi(
            execution.pipeline_step.progress_plan.clone(),
            progress_fact.unwrap_or_else(empty_domain_progress_fact),
            context,
        )
    });
    let status = progress
        .as_ref()
        .map(|progress| progress.status)
        .unwrap_or(outcome.precheck.event_status.as_u8());
    let error_code = progress
        .as_ref()
        .map(|progress| progress.error_code.clone())
        .unwrap_or_else(|| outcome.precheck.error_code.to_owned());
    let add_outcome = outcome
        .add_outcome
        .map(|add| outcome_to_ffi_add_vote_outcome(copy_vote_payload(&vote), add));
    let storage_vote = outcome
        .storage_vote
        .map(Into::into)
        .unwrap_or_else(empty_storage_record);
    let extra_reward_vote = if progress
        .as_ref()
        .map(|progress| progress.persist_extra_reward_vote)
        .unwrap_or(false)
    {
        PbftVoteStorageRecord {
            hash: storage_vote.hash,
            vote_rlp: storage_vote.vote_rlp.clone(),
        }
    } else {
        empty_storage_record()
    };
    let two_t_plus_one_bundle = if let Some(bundle) = outcome.two_t_plus_one_bundle {
        PbftTwoTPlusOneVoteBundle {
            kind: bundle.kind.into(),
            period: bundle.period,
            round: bundle.round,
            step: bundle.step,
            block_hash: bundle.block_hash.into(),
            votes_bundle_rlp: bundle.votes_bundle_rlp,
        }
    } else {
        empty_two_t_plus_one_bundle()
    };
    let (slashing_incoming_vote, slashing_conflicting_vote) =
        if let Some(payloads) = outcome.slashing_payloads {
            (payloads.incoming.into(), payloads.conflicting.into())
        } else {
            (empty_storage_record(), empty_storage_record())
        };

    PbftVoteAdmissionRuntimeResult {
        status,
        error_code,
        accepted: progress
            .as_ref()
            .map(|progress| progress.accepted)
            .unwrap_or(false),
        rejected: !validation.accepted,
        has_validation: true,
        replay_should_mark: outcome.replay.should_mark,
        replay_inserted: outcome.replay.inserted,
        replay_already_present: outcome.replay.already_present,
        validation: validation.into(),
        has_vote: progress_fact.is_some(),
        vote,
        has_verified_vote_add: add_outcome.is_some(),
        verified_vote_add: add_outcome.unwrap_or_else(|| {
            outcome_to_ffi_add_vote_outcome(empty_vote_payload(), empty_add_outcome())
        }),
        has_storage_vote: storage_vote.hash != [0; 32],
        storage_vote,
        persist_extra_reward_vote: progress
            .as_ref()
            .map(|progress| progress.persist_extra_reward_vote)
            .unwrap_or(false),
        extra_reward_vote,
        persist_two_t_plus_one_votes: progress
            .as_ref()
            .map(|progress| progress.persist_two_t_plus_one_votes)
            .unwrap_or(false),
        two_t_plus_one_bundle,
        mark_vote_known: progress
            .as_ref()
            .map(|progress| progress.mark_vote_known)
            .unwrap_or(false),
        mark_vote_known_hash: progress
            .as_ref()
            .map(|progress| progress.mark_vote_known_hash)
            .unwrap_or_default(),
        request_proposed_block_sidecar: progress
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
        gossip_vote: progress
            .as_ref()
            .map(|progress| progress.gossip_vote)
            .unwrap_or(false),
        gossip_vote_hash: progress
            .as_ref()
            .map(|progress| progress.gossip_vote_hash)
            .unwrap_or_default(),
        report_slashing: progress
            .as_ref()
            .map(|progress| progress.report_slashing)
            .unwrap_or(false),
        slashing_incoming_vote,
        slashing_conflicting_vote,
        network_t_plus_one_step_updated: progress
            .as_ref()
            .map(|progress| progress.network_t_plus_one_step_updated)
            .unwrap_or(false),
        drive_pbft_progress: progress
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

fn empty_domain_progress_fact() -> rustaxa_consensus::PbftVoteProgressFact {
    rustaxa_consensus::PbftVoteProgressFact {
        identity: rustaxa_consensus::PbftVoteIdentity {
            vote_hash: [0; 32].into(),
            block_hash: [0; 32].into(),
            period: 0,
            round: 0,
            step: 0,
            voter: [0; 20].into(),
        },
        vote_type: PbftVoteType::Soft,
        weight: 0,
        vote_already_known: false,
        carries_proposed_block: false,
        valid_stale_reward_vote: false,
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

fn empty_two_t_plus_one_bundle() -> PbftTwoTPlusOneVoteBundle {
    PbftTwoTPlusOneVoteBundle {
        kind: 0,
        period: 0,
        round: 0,
        step: 0,
        block_hash: [0; 32],
        votes_bundle_rlp: Vec::new(),
    }
}

fn payload_to_vote(value: VerifiedVotePayload) -> Result<VerifiedVote, anyhow::Error> {
    VerifiedVote::new(
        H256::from(value.vote_hash),
        H256::from(value.block_hash),
        H160::from(value.voter),
        value.period,
        value.round,
        value.step,
        PbftVoteType::try_from(value.vote_type)?,
        value.weight,
    )
}

impl From<rustaxa_consensus::verified_votes::VotedValueInsertOutcome> for VotedValueInsertOutcome {
    fn from(value: rustaxa_consensus::verified_votes::VotedValueInsertOutcome) -> Self {
        Self {
            inserted: value.inserted,
            total_weight: value.total_weight,
            votes_count: value.votes_count,
        }
    }
}

fn outcome_to_ffi_add_vote_outcome(
    vote: VerifiedVotePayload,
    value: ConsensusAddVerifiedVoteOutcome,
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

impl From<ConsensusTwoTPlusOneInsertOutcome> for TwoTPlusOneInsertOutcome {
    fn from(value: ConsensusTwoTPlusOneInsertOutcome) -> Self {
        Self {
            round_found: value.round_found,
            inserted: value.inserted,
        }
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

impl From<ConsensusThresholdDecisionOutcome> for ThresholdDecisionOutcome {
    fn from(value: ConsensusThresholdDecisionOutcome) -> Self {
        let (kind_found, kind) = value
            .two_t_plus_one_kind
            .map(|kind| (true, kind.into()))
            .unwrap_or((false, 0));
        let (round_found, inserted) = value
            .two_t_plus_one_insert_outcome
            .map(|outcome| (outcome.round_found, outcome.inserted))
            .unwrap_or((false, false));

        Self {
            t_plus_one_reached: value.t_plus_one_reached,
            network_t_plus_one_step_updated: value.network_t_plus_one_step_updated,
            two_t_plus_one_reached: value.two_t_plus_one_reached,
            two_t_plus_one_kind_found: kind_found,
            two_t_plus_one_kind: kind,
            two_t_plus_one_round_found: round_found,
            two_t_plus_one_inserted: inserted,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::pbft_finalize::PbftFinalizedPeriodApplyStatus;
    use rustaxa_consensus::pbft_vote_admission::{
        PbftVoteAdmissionExecution, PbftVoteAdmissionPrecheck, PbftVoteAdmissionStatus,
    };
    use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactStatus;
    use rustaxa_consensus::pbft_vote_pipeline::{PbftVotePipelineStatus, PbftVotePipelineStep};
    use rustaxa_consensus::pbft_vote_progress::{
        PbftVoteIdentity, PbftVoteProgressFact, PbftVoteProgressIntent, PbftVoteProgressPlan,
        PbftVoteProgressStatus,
    };
    use rustaxa_consensus::pbft_vote_runtime::PbftVoteRuntimeReplayOutcome;
    use rustaxa_consensus::pbft_vote_validation::PbftVoteValidationStatus;
    use rustaxa_consensus::{
        build_weighted_pbft_vote_payload, generate_pbft_vote, PbftVoteGenerationInput,
    };
    use rustaxa_storage::{Config, Storage};
    use rustaxa_vdf::vrf;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x35; 32];
    const NODE_SECRET_TWO: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn hash(id: u64) -> [u8; 32] {
        H256::from_low_u64_be(id).into()
    }

    fn address(id: u64) -> [u8; 20] {
        H160::from_low_u64_be(id).into()
    }

    #[test]
    fn storage_backed_factory_restores_empty_runtime_and_cursor() {
        let storage = temp_bridge_storage("empty_startup");
        let votes = create_verified_votes_index_from_storage(&storage).unwrap();

        assert_eq!(votes.verified_votes_size(), 0);
        assert!(!votes.verified_votes_reward_vote_cursor().found);
        assert!(votes.storage.is_some());
    }

    #[test]
    fn test_helper_has_no_storage_access() {
        let votes = create_empty_verified_votes_for_test();
        assert!(!votes.verified_votes_reward_vote_cursor().found);
        assert!(votes.verified_votes_own_vote_records().is_err());
    }

    #[test]
    fn own_vote_records_save_read_order_restart_and_clear_all() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/rustaxa_bridge_own_vote_restart_{}_{}",
            std::process::id(),
            nonce
        ));
        let storage = BridgeStorage(Arc::new(Storage::new(Config::new(path.clone())).unwrap()));
        let votes = create_verified_votes_index_from_storage(&storage).unwrap();
        let first =
            build_weighted_pbft_vote_payload(&generated_vote([0x31; 32], NODE_SECRET).vote_rlp, 7)
                .unwrap();
        let second = build_weighted_pbft_vote_payload(
            &generated_vote([0x32; 32], NODE_SECRET_TWO).vote_rlp,
            9,
        )
        .unwrap();

        for record in [&second, &first] {
            let result = votes
                .verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
                    hash: record.hash.0,
                    vote_rlp: record.vote_rlp.clone(),
                })
                .unwrap();
            assert_eq!(result.status, 0);
        }

        let expected_hashes = {
            let mut hashes = vec![first.hash, second.hash];
            hashes.sort_unstable();
            hashes
        };
        let records = votes.verified_votes_own_vote_records().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|record| H256::from(record.hash))
                .collect::<Vec<_>>(),
            expected_hashes
        );

        drop(votes);
        drop(storage);
        let reopened = BridgeStorage(Arc::new(Storage::new(Config::new(path.clone())).unwrap()));
        let votes = create_verified_votes_index_from_storage(&reopened).unwrap();
        assert_eq!(votes.verified_votes_own_vote_records().unwrap().len(), 2);

        let result = votes.verified_votes_clear_own_verified_votes().unwrap();
        assert_eq!(result.status, 0);
        assert_eq!(result.applied_writes, 2);
        assert!(votes.verified_votes_own_vote_records().unwrap().is_empty());

        drop(votes);
        drop(reopened);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn own_vote_records_reject_malformed_payload_and_hash_mismatch() {
        let storage = temp_bridge_storage("own_vote_validation");
        let votes = create_verified_votes_index_from_storage(&storage).unwrap();

        assert!(votes
            .verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
                hash: H256::from_low_u64_be(1).0,
                vote_rlp: vec![0x01],
            })
            .is_err());
        assert!(storage
            .0
            .pbft()
            .own_verified_vote_hashes()
            .unwrap()
            .is_empty());

        let weighted =
            build_weighted_pbft_vote_payload(&generated_vote([0x33; 32], NODE_SECRET).vote_rlp, 5)
                .unwrap();
        let wrong_hash = H256::from_low_u64_be(2);
        assert_ne!(wrong_hash, weighted.hash);
        assert!(votes
            .verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
                hash: wrong_hash.0,
                vote_rlp: weighted.vote_rlp.clone(),
            })
            .is_err());
        assert!(storage
            .0
            .pbft()
            .own_verified_vote_hashes()
            .unwrap()
            .is_empty());

        storage
            .0
            .pbft()
            .write_own_verified_vote(H256::from_low_u64_be(1), &[0x01])
            .unwrap();
        assert!(votes.verified_votes_own_vote_records().is_err());

        votes.verified_votes_clear_own_verified_votes().unwrap();
        storage
            .0
            .pbft()
            .write_own_verified_vote(wrong_hash, &weighted.vote_rlp)
            .unwrap();
        let error = match votes.verified_votes_own_vote_records() {
            Ok(_) => panic!("mismatched own-vote key must be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("VERIFIED_VOTES_OWN_VOTE_HASH_MISMATCH"));
    }

    #[test]
    fn lifecycle_reset_clears_votes_saved_and_queried_through_another_handle() {
        let storage = temp_bridge_storage("own_vote_cross_handle_lifecycle");
        let votes = create_verified_votes_index_from_storage(&storage).unwrap();
        let weighted =
            build_weighted_pbft_vote_payload(&generated_vote([0x34; 32], NODE_SECRET).vote_rlp, 5)
                .unwrap();
        votes
            .verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
                hash: weighted.hash.0,
                vote_rlp: weighted.vote_rlp,
            })
            .unwrap();
        assert_eq!(votes.verified_votes_own_vote_records().unwrap().len(), 1);

        let mut runtime = crate::pbft_manager::create_pbft_manager_runtime_from_storage(
            &storage,
            crate::pbft_manager::TestPbftManagerStartupFact {
                current_period: 10,
                cacti_active_at_chain_size: false,
                genesis_lambda_ms: 100,
                cacti_lambda_max_ms: 1_500,
                cacti_lambda_default_ms: 500,
                cacti_block: 1,
                max_exponential_lambda_ms: 60_000,
                max_steps: 13,
                deadline_ms: 1_000,
                polling_interval_ms: 100,
            },
        )
        .unwrap();
        let rejected = crate::pbft_manager::pbft_manager_runtime_execute_lifecycle_transition(
            &mut runtime,
            crate::ffi::rustaxa_ffi::PbftManagerLifecycleTransitionRequest {
                kind: 255,
                target_period: 10,
                target_round: 4,
                has_network_next_voting_step: false,
                network_next_voting_step: 0,
            },
        )
        .unwrap();
        assert_eq!(rejected.status, 1);
        assert_eq!(votes.verified_votes_own_vote_records().unwrap().len(), 1);

        let applied = crate::pbft_manager::pbft_manager_runtime_execute_lifecycle_transition(
            &mut runtime,
            crate::ffi::rustaxa_ffi::PbftManagerLifecycleTransitionRequest {
                kind: 0,
                target_period: 10,
                target_round: 4,
                has_network_next_voting_step: false,
                network_next_voting_step: 0,
            },
        )
        .unwrap();
        assert_eq!(applied.status, 0);
        assert!(votes.verified_votes_own_vote_records().unwrap().is_empty());
    }

    fn temp_bridge_storage(name: &str) -> BridgeStorage {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = PathBuf::from(format!(
            "/tmp/rustaxa_bridge_verified_votes_{name}_{}_{}",
            std::process::id(),
            nonce
        ));
        BridgeStorage(Arc::new(Storage::new(Config::new(path)).unwrap()))
    }

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = k256::ecdsa::SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn generated_vote(
        block_hash: [u8; 32],
        node_secret: [u8; 32],
    ) -> rustaxa_consensus::PbftGeneratedVote {
        generated_vote_for_type(block_hash, node_secret, PbftVoteType::Cert, 3)
    }

    fn generated_vote_for_type(
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        vote_type: PbftVoteType,
        step: u64,
    ) -> rustaxa_consensus::PbftGeneratedVote {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type,
            period: 12,
            round: 2,
            step,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
    }

    fn validation_facts() -> PbftVoteValidationExternalFacts {
        PbftVoteValidationExternalFacts {
            voter_dpos_ready: true,
            voter_dpos_vote_count: 40,
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
            has_preverified_weight: false,
            preverified_weight: 0,
        }
    }

    fn runtime_flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
        }
    }

    fn runtime_context(threshold: u64) -> FfiPbftVoteProgressContext {
        FfiPbftVoteProgressContext {
            current_period: 12,
            current_round: 2,
            max_future_period_delta: 0,
            has_two_t_plus_one_threshold: true,
            two_t_plus_one_threshold: threshold,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn reward_reset_intent(
        block_hash: [u8; 32],
    ) -> crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan {
        crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan {
            persist_pbft_head: false,
            persist_period_data: false,
            reset_reward_votes: true,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            process_pillar_block: false,
            pbft_block_hash: block_hash,
            pbft_head_hash: [0; 32],
            block_period: 12,
            null_anchor: false,
            anchor_hash: [0; 32],
            reward_vote_period: 12,
            reward_vote_round: 2,
            reward_vote_step: 3,
            reward_vote_block_hash: block_hash,
            period_lambda: 0,
            blocks_per_year: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_status: false,
            pbft_head_payload: Vec::new(),
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        }
    }

    fn payload(
        vote_hash: u64,
        block_hash: u64,
        voter: u64,
        step: u64,
        weight: u64,
    ) -> VerifiedVotePayload {
        VerifiedVotePayload {
            vote_hash: hash(vote_hash),
            block_hash: hash(block_hash),
            voter: address(voter),
            period: 3,
            round: 2,
            step,
            vote_type: PbftVoteType::Next.into(),
            weight,
        }
    }

    fn threshold_fact(has_total_dpos_votes_count: bool) -> FfiPbftTwoTPlusOneThresholdFact {
        FfiPbftTwoTPlusOneThresholdFact {
            pbft_period: 3,
            vote_type: PbftVoteType::Cert.into(),
            current_pbft_chain_size: 3,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count,
            total_dpos_votes_count: if has_total_dpos_votes_count { 100 } else { 0 },
            future_dpos_state: false,
            unknown_error: false,
        }
    }

    #[test]
    fn bridge_facade_owns_replay_and_threshold_cache() {
        let mut votes = create_empty_verified_votes_for_test();
        let vote_hash = hash(99);

        assert!(!votes.verified_votes_replay_contains(&vote_hash));
        assert!(votes.verified_votes_replay_insert(&vote_hash));
        assert!(!votes.verified_votes_replay_insert(&vote_hash));
        assert!(votes.verified_votes_replay_contains(&vote_hash));

        let plan = votes.verified_votes_two_t_plus_one_threshold(threshold_fact(true));
        assert!(plan.has_threshold);
        assert!(plan.cached);

        let cached = votes.verified_votes_two_t_plus_one_threshold(threshold_fact(false));
        assert!(cached.cache_hit);
        assert_eq!(cached.threshold, plan.threshold);
    }

    #[test]
    fn bridge_add_verified_vote_reports_threshold_and_step_snapshot() {
        let mut votes = create_empty_verified_votes_for_test();

        let first = votes
            .verified_votes_add_verified_vote(payload(1, 44, 1, 5, 3), 5, true)
            .expect("first vote is accepted");
        assert!(first.inserted);
        assert!(first.threshold_applied);
        assert!(first.t_plus_one_reached);
        assert!(!first.two_t_plus_one_reached);

        let second = votes
            .verified_votes_add_verified_vote(payload(2, 44, 2, 5, 2), 5, true)
            .expect("second vote is accepted");
        assert!(second.inserted);
        assert!(second.two_t_plus_one_reached);
        assert!(second.two_t_plus_one_kind_found);
        assert!(second.two_t_plus_one_inserted);

        let step_votes = votes.verified_votes_get_step_votes(3, 2, 5);
        assert!(step_votes.found);
        assert_eq!(step_votes.entries.len(), 1);
        assert_eq!(step_votes.entries[0].block_hash, hash(44));
        assert_eq!(step_votes.entries[0].total_weight, 5);
        assert_eq!(step_votes.entries[0].vote_hashes.len(), 2);
    }

    #[test]
    fn bridge_admission_exposes_retained_weighted_payloads() {
        let mut votes = create_empty_verified_votes_for_test();
        let first = generated_vote([0x22; 32], NODE_SECRET);
        let second = generated_vote([0x22; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        let first_result = votes
            .verified_votes_admit_validated_vote(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        assert!(first_result.accepted);
        assert!(first_result.has_storage_vote);

        let lookup = votes.verified_votes_weighted_payload(&first_hash);
        assert!(lookup.found);
        assert_eq!(lookup.vote.hash, first_hash);
        assert!(!lookup.vote.vote_rlp.is_empty());

        let second_result = votes
            .verified_votes_admit_validated_vote(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");
        assert!(second_result.accepted);

        let payloads = votes
            .verified_votes_get_two_t_plus_one_voted_block_payloads(
                12,
                2,
                TwoTPlusOneVotedBlockType::CertVotedBlock.into(),
            )
            .expect("2t+1 retained payload lookup succeeds");
        assert!(payloads.found);
        assert_eq!(payloads.block_hash, [0x22; 32]);
        assert_eq!(payloads.step, 3);
        assert_eq!(payloads.votes.len(), 2);
        assert!(payloads.votes.iter().any(|vote| vote.hash == first_hash));
        assert!(payloads.votes.iter().any(|vote| vote.hash == second_hash));
    }

    #[test]
    fn bridge_builds_optimized_bundle_from_retained_payloads() {
        let mut votes = create_empty_verified_votes_for_test();
        let first = generated_vote([0x24; 32], NODE_SECRET);
        let second = generated_vote([0x24; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        votes
            .verified_votes_admit_validated_vote(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .verified_votes_admit_validated_vote(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let lookup = votes
            .verified_votes_get_two_t_plus_one_voted_block_payloads(
                12,
                2,
                TwoTPlusOneVotedBlockType::CertVotedBlock.into(),
            )
            .expect("2t+1 retained payload lookup succeeds");
        assert!(lookup.found);

        let result = votes.verified_votes_build_optimized_votes_bundle_egress(
            PbftOptimizedVoteBundleBuildRequest {
                kind: TwoTPlusOneVotedBlockType::CertVotedBlock.into(),
                block_hash: [0x24; 32],
                period: 12,
                round: 2,
                step: 3,
                vote_hashes: lookup
                    .votes
                    .into_iter()
                    .map(|vote| PbftFinalizationHash { hash: vote.hash })
                    .collect(),
            },
        );

        assert_eq!(result.status, PBFT_OPTIMIZED_BUNDLE_READY);
        assert_eq!(result.vote_hashes.len(), 2);
        assert!(result
            .vote_hashes
            .iter()
            .any(|hash| hash.hash == first_hash));
        assert!(result
            .vote_hashes
            .iter()
            .any(|hash| hash.hash == second_hash));
        let decoded = rlp::Rlp::new(&result.votes_bundle_rlp);
        assert_eq!(decoded.item_count().unwrap(), 5);
        assert_eq!(decoded.val_at::<H256>(0).unwrap(), H256::from([0x24; 32]));
        assert_eq!(decoded.val_at::<u64>(1).unwrap(), 12);
        assert_eq!(decoded.val_at::<u64>(2).unwrap(), 2);
        assert_eq!(decoded.val_at::<u64>(3).unwrap(), 3);
        assert_eq!(decoded.at(4).unwrap().item_count().unwrap(), 2);
    }

    #[test]
    fn bridge_plans_next_vote_bundle_egress_and_rejects_order_drift() {
        let mut votes = create_empty_verified_votes_for_test();
        let first = generated_vote_for_type([0x25; 32], NODE_SECRET, PbftVoteType::Next, 4);
        let second = generated_vote_for_type([0x25; 32], NODE_SECRET_TWO, PbftVoteType::Next, 4);

        votes
            .verified_votes_admit_validated_vote(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .verified_votes_admit_validated_vote(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let plan = votes.verified_votes_plan_next_votes_bundle_egress(12, 2);
        assert_eq!(plan.status, PBFT_OPTIMIZED_BUNDLE_READY);
        assert!(plan.next_votes.found);
        assert_eq!(
            plan.next_votes.kind,
            u8::from(TwoTPlusOneVotedBlockType::NextVotedBlock)
        );
        assert_eq!(plan.next_votes.vote_hashes.len(), 2);
        assert!(!plan.next_null_votes.found);

        let kind = plan.next_votes.kind;
        let block_hash = plan.next_votes.block_hash;
        let period = plan.next_votes.period;
        let round = plan.next_votes.round;
        let step = plan.next_votes.step;
        let mut reversed: Vec<PbftFinalizationHash> = plan
            .next_votes
            .vote_hashes
            .into_iter()
            .map(|hash| PbftFinalizationHash { hash: hash.hash })
            .collect();
        reversed.reverse();
        let rejected = votes.verified_votes_build_optimized_votes_bundle_egress(
            PbftOptimizedVoteBundleBuildRequest {
                kind,
                block_hash,
                period,
                round,
                step,
                vote_hashes: reversed,
            },
        );
        assert_eq!(rejected.status, PBFT_OPTIMIZED_BUNDLE_ORDER_MISMATCH);
    }

    #[test]
    fn bridge_selects_reward_vote_payloads_in_requested_order() {
        let storage = temp_bridge_storage("reward_selection_cursor");
        let mut votes = create_verified_votes_index_from_storage(&storage).unwrap();
        let first = generated_vote([0x33; 32], NODE_SECRET);
        let second = generated_vote([0x33; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        votes
            .verified_votes_admit_validated_vote(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .verified_votes_admit_validated_vote(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let applied = votes
            .verified_votes_apply_reward_votes_reset(FfiPbftRewardVotesResetRequest {
                period: 12,
                round: 2,
                step: 3,
                block_hash: [0x33; 32],
                sync: false,
            })
            .unwrap();
        let committed = votes
            .verified_votes_commit_reward_vote_cursor(
                &reward_reset_intent([0x33; 32]),
                applied.reward_votes_reset_generation,
            )
            .unwrap();
        assert_eq!(committed.status, 0);
        assert_eq!(votes.verified_votes_reward_vote_period(), 12);
        let cursor = votes.verified_votes_reward_vote_cursor();
        assert!(cursor.found);
        assert_eq!(cursor.round, 2);
        assert_eq!(cursor.step, 3);
        assert_eq!(
            votes
                .verified_votes_current_reward_vote_payloads()
                .unwrap()
                .len(),
            2
        );
        let repeated = votes
            .verified_votes_commit_reward_vote_cursor(
                &reward_reset_intent([0x33; 32]),
                applied.reward_votes_reset_generation,
            )
            .unwrap();
        assert_eq!(repeated.status, 1);

        let selection = votes
            .verified_votes_select_reward_vote_payloads(
                13,
                vec![
                    PbftFinalizationHash { hash: second_hash },
                    PbftFinalizationHash { hash: first_hash },
                ],
            )
            .expect("reward payload selection succeeds");

        assert!(selection.accepted);
        assert_eq!(selection.selected_round, 2);
        assert_eq!(
            selection
                .selected_vote_hashes
                .iter()
                .map(|hash| hash.hash)
                .collect::<Vec<_>>(),
            vec![second_hash, first_hash]
        );
        assert_eq!(selection.selected_records.len(), 2);
        assert_eq!(selection.selected_records[0].hash, second_hash);
        assert_eq!(selection.selected_records[1].hash, first_hash);
        assert!(selection.error_code.is_empty());
    }

    #[test]
    fn runtime_result_flattens_vote_executor_intents() {
        let vote_hash = H256::from(hash(77));
        let block_hash = H256::from(hash(88));
        let voter = H160::from(address(99));
        let validation = PbftCanonicalVoteValidation {
            status: PbftVoteValidationStatus::Valid,
            error_code: "",
            accepted: true,
            rejected: false,
            mark_validated_replay: true,
            vote_hash,
            signing_hash: H256::from(hash(78)),
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
        let context = FfiPbftVoteProgressContext {
            current_period: 3,
            current_round: 2,
            max_future_period_delta: 1,
            has_two_t_plus_one_threshold: true,
            two_t_plus_one_threshold: 5,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        };
        let add_outcome = ConsensusAddVerifiedVoteOutcome {
            inserted: true,
            total_weight: 5,
            votes_count: 1,
            conflicting_vote_hash: None,
            used_secondary_slot: false,
            duplicate_vote_hash: false,
            threshold_decision: None,
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
            add_vote_outcome: Some(add_outcome),
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
            replay: PbftVoteRuntimeReplayOutcome {
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
            add_outcome: Some(add_outcome),
            storage_vote: None,
            two_t_plus_one_bundle: None,
            slashing_payloads: None,
        };

        let result = runtime_outcome_to_ffi(validation, outcome, context);

        assert!(result.accepted);
        assert!(result.mark_vote_known);
        assert_eq!(result.mark_vote_known_hash, hash(77));
        assert!(result.gossip_vote);
        assert_eq!(result.gossip_vote_hash, hash(77));
        assert!(!result.request_proposed_block_sidecar);
        assert!(result.drive_pbft_progress);
        assert_eq!(result.progress_period, 3);
        assert_eq!(result.progress_round, 2);
    }

    #[test]
    fn bridge_applies_reward_votes_reset_storage_request() {
        let storage = temp_bridge_storage("reward_reset");
        let mut votes = create_verified_votes_index_from_storage(&storage).unwrap();
        for secret in [NODE_SECRET, NODE_SECRET_TWO] {
            let vote = generated_vote([0x35; 32], secret);
            votes
                .verified_votes_admit_validated_vote(
                    &vote.vote_rlp,
                    validation_facts(),
                    runtime_flags(),
                    runtime_context(80),
                )
                .unwrap();
        }
        storage
            .0
            .pbft()
            .write_extra_reward_vote(H256::from_low_u64_be(701), &[0x01])
            .unwrap();

        let stage = votes
            .verified_votes_prepare_reward_votes_reset_stage(&reward_reset_intent([0x35; 32]))
            .unwrap();
        assert!(stage.has_reward_votes_reset);
        assert!(!stage.reward_votes_bundle_rlp.is_empty());
        assert!(votes
            .verified_votes_prepare_reward_votes_reset_stage(&reward_reset_intent([0x36; 32]))
            .is_err());

        let result = votes
            .verified_votes_apply_reward_votes_reset(FfiPbftRewardVotesResetRequest {
                period: 12,
                round: 2,
                step: 3,
                block_hash: [0x35; 32],
                sync: true,
            })
            .expect("reward-vote reset storage request applies");

        assert_eq!(
            result.status,
            PbftFinalizedPeriodApplyStatus::Applied.as_u8()
        );
        assert_eq!(result.block_period, 12);
        assert_eq!(result.pbft_block_hash, [0x35; 32]);
        assert_ne!(result.reward_votes_reset_generation, 0);
        assert!(storage
            .0
            .pbft()
            .extra_reward_vote_hashes()
            .unwrap()
            .is_empty());
        assert!(!result.wrote_pbft_head);
        assert!(!result.wrote_period_data);
        assert_eq!(result.dag_index_writes, 0);
        assert_eq!(result.transaction_location_writes, 0);
        assert!(result.error_code.is_empty());
    }

    #[test]
    fn bridge_maps_finalization_storage_apply_rejection_status() {
        let storage = temp_bridge_storage("rejected_storage_write");
        let votes = create_verified_votes_index_from_storage(&storage).unwrap();
        let write_intent = crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan {
            persist_pbft_head: false,
            persist_period_data: false,
            reset_reward_votes: false,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            process_pillar_block: false,
            pbft_block_hash: hash(800),
            pbft_head_hash: hash(801),
            block_period: 11,
            null_anchor: true,
            anchor_hash: [0; 32],
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: [0; 32],
            period_lambda: 0,
            blocks_per_year: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            executed_pbft_status: false,
            pbft_head_payload: Vec::new(),
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        };
        let primary_stage = crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage {
            stage: 0,
            rounds_count_dynamic_lambda: 0,
            dynamic_lambda: 0,
            has_sortition_params_change: false,
            sortition_params_change_period: 0,
            sortition_params_change_interval_efficiency: 0,
            sortition_params_change_threshold_upper: 0,
            has_reward_votes_reset: false,
            reward_votes_bundle_rlp: Vec::new(),
            has_prepared_pillar_block: false,
            prepared_pillar_block_period: 0,
            prepared_pillar_block_rlp: Vec::new(),
        };

        let result = votes
            .verified_votes_apply_pbft_finalization_storage_writes(
                &write_intent,
                vec![primary_stage],
                false,
            )
            .expect("rejected write-set reports through bridge");

        assert_eq!(
            result.status,
            PbftFinalizedPeriodApplyStatus::RejectedWriteSet.as_u8()
        );
        assert_eq!(result.block_period, 11);
        assert_eq!(result.pbft_block_hash, hash(800));
        assert!(!result.wrote_pbft_head);
        assert!(!result.wrote_period_data);
        assert_eq!(result.error_code, "PBFT_FINALIZE_REJECTED_WRITE_SET");
    }
}
