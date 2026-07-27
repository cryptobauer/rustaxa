use crate::ffi::rustaxa_ffi::{
    DetermineNewRoundOutcome, PbftFinalizationHash, PbftLeaderCandidateSnapshot,
    PbftLeaderCandidateValidation, PbftLeaderSelectionFinishRequest, PbftLeaderSelectionResult,
    PbftLeaderSelectionSnapshot, PbftNextVotesBundleEgressPlan,
    PbftOptimizedVoteBundleBuildRequest, PbftOptimizedVoteBundleBuildResult,
    PbftOptimizedVoteBundlePlan,
    PbftRewardVotePayloadSelection as FfiPbftRewardVotePayloadSelection,
    PbftRewardVotesResetRequest as FfiPbftRewardVotesResetRequest,
    PbftTwoTPlusOneThresholdFact as FfiPbftTwoTPlusOneThresholdFact,
    PbftTwoTPlusOneThresholdPlan as FfiPbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneVoteBundle,
    PbftVoteAdmissionRuntimeResult, PbftVoteAdmissionValidationRequest, PbftVoteEventFactFlags,
    PbftVoteProgressContext as FfiPbftVoteProgressContext, PbftVoteRuntimeValidationResult,
    PbftVoteStorageRecord, RewardVoteCursorCommitResult as FfiRewardVoteCursorCommitResult,
    RewardVoteCursorSnapshot as FfiRewardVoteCursorSnapshot, RewardVotePayloadSnapshot,
    RoundMarkerSnapshot, TwoTPlusOneSnapshotEntry, TwoTPlusOneVotePayloadsLookup,
    TwoTPlusOneVotedBlockLookup, VerifiedStepVotePayloadEntry, VerifiedStepVotePayloadsLookup,
    VerifiedVoteAddOutcome as FfiVerifiedVoteAddOutcome, VerifiedVotePayload,
    VerifiedVoteStateSnapshotEntry, VerifiedVotesStateSnapshot,
};
#[cfg(test)]
use crate::ffi::BridgeStorage;
use crate::ffi::{BridgeFinalChain, BridgePbftService};
use crate::pbft_vote_progress::{context_to_domain, execution_plan_to_ffi};

/// Rust-private compatibility facts retained for focused runtime unit tests.
///
/// Production C++ no longer materializes this aggregate; the composed
/// FinalChain admission route accepts only policy inputs and resolves external
/// state inside Rust.
#[derive(Clone, Copy)]
#[cfg(test)]
struct PbftVoteValidationExternalFacts {
    voter_dpos_ready: bool,
    voter_dpos_vote_count: u64,
    total_dpos_ready: bool,
    total_dpos_vote_count: u64,
    future_dpos_state: bool,
    unknown_error: bool,
    vrf_key_ready: bool,
    has_vrf_key: bool,
    vrf_public_key: [u8; 32],
    strict_vrf: bool,
    committee_size: u64,
    number_of_proposers: u64,
    has_preverified_weight: bool,
    preverified_weight: u64,
}
use crate::pbft_vote_validation::threshold_plan_to_ffi;
#[cfg(test)]
use ethereum_types::H160;
use ethereum_types::H256;
use rustaxa_consensus::pbft_chain::pbft_block_exists_in_storage;
#[cfg(test)]
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_finalization_storage_writes, PbftFinalizationStorageWriteIntent,
    PbftFinalizationStorageWriteStage as DomainPbftFinalizationStorageWriteStage,
};
use rustaxa_consensus::pbft_finalize::{
    apply_pbft_reward_votes_reset_storage, PbftFinalizedPeriodApplyResult,
    PbftRewardVotesResetStorageRequest,
};
use rustaxa_consensus::pbft_manager::{
    plan_pbft_manager_leader_candidates, PbftManagerLeaderBlockValidationStatus,
    PbftManagerLeaderCandidateInputFact, PbftManagerLeaderSelectionStatus,
};
use rustaxa_consensus::pbft_reward_votes::PbftRewardVotesStatus;
use rustaxa_consensus::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdStatus,
};
use rustaxa_consensus::pbft_vote_event::PbftVoteEventFactFlags as DomainPbftVoteEventFactFlags;
#[cfg(test)]
use rustaxa_consensus::pbft_vote_payload::PbftVotePayloadRecord as DomainPbftVotePayloadRecord;
use rustaxa_consensus::pbft_vote_payload::{
    build_optimized_pbft_vote_bundle, build_weighted_pbft_vote_payload,
};
use rustaxa_consensus::pbft_vote_storage::{
    clear_own_verified_votes, persist_pbft_vote_progress, save_own_verified_vote,
};
use rustaxa_consensus::pbft_vote_validation::{
    inspect_canonical_pbft_vote, validate_canonical_pbft_vote, PbftCanonicalVoteInspectionStatus,
    PbftCanonicalVoteValidation,
    PbftVoteAdmissionValidationRequest as DomainPbftVoteAdmissionValidationRequest,
    PbftVoteValidationExternalFacts as DomainPbftVoteValidationExternalFacts,
};
use rustaxa_consensus::verified_votes::{
    AddVerifiedVoteOutcome as ConsensusAddVerifiedVoteOutcome,
    DetermineNewRoundOutcome as ConsensusDetermineNewRoundOutcome, PbftVoteType,
    TwoTPlusOneVotedBlockType, VerifiedVote,
};
use rustaxa_consensus::{
    PbftTwoTPlusOneVoteBundle as DomainPbftTwoTPlusOneVoteBundle, PbftVoteAdmissionRuntime,
    PbftVoteAdmissionTransactionResult,
    PbftVotePersistenceResult as DomainPbftVotePersistenceResult,
    PbftVoteProgressPersistenceWrite as DomainPbftVoteProgressPersistenceWrite,
    PbftVoteStorageRecord as DomainPbftVoteStorageRecord, RewardVoteCursor,
    RewardVoteCursorCommitStatus,
};
use rustaxa_storage::Storage;
use rustaxa_vdf::vrf;
use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};

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
const PBFT_LEADER_SELECTED: u8 = 0;
const PBFT_LEADER_NO_CANDIDATES: u8 = 1;
const PBFT_LEADER_NO_ELIGIBLE: u8 = 2;
const PBFT_LEADER_STALE_SNAPSHOT: u8 = 3;
const PBFT_LEADER_INVALID_VALIDATION_REPORT: u8 = 4;
const PBFT_LEADER_VALIDATED: u8 = 1;
const PBFT_LEADER_REJECTED: u8 = 2;

#[cfg(test)]
struct UniqueVoterInsertOutcome {
    accepted: bool,
    conflict_found: bool,
    conflicting_vote_hash: [u8; 32],
    conflicting_vote_found: bool,
    conflicting_vote: PbftVoteStorageRecord,
    bucket_found: bool,
    bucket: VerifiedStepVotePayloadEntry,
    used_secondary_slot: bool,
    duplicate_vote_hash: bool,
}

#[cfg(test)]
struct VotedValueInsertOutcome {
    inserted: bool,
    total_weight: u64,
    votes_count: usize,
    conflicting_vote_found: bool,
    conflicting_vote: PbftVoteStorageRecord,
    bucket_found: bool,
    bucket: VerifiedStepVotePayloadEntry,
}

#[cfg(test)]
struct AtomicVoteInsertOutcome {
    inserted: bool,
    total_weight: u64,
    votes_count: usize,
    conflict_found: bool,
    conflicting_vote_hash: [u8; 32],
    conflicting_vote_found: bool,
    conflicting_vote: PbftVoteStorageRecord,
    bucket_found: bool,
    bucket: VerifiedStepVotePayloadEntry,
    used_secondary_slot: bool,
    duplicate_vote_hash: bool,
}

fn threshold_fact_from_request(
    fact: &FfiPbftTwoTPlusOneThresholdFact,
    current_pbft_chain_size: u64,
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
        current_pbft_chain_size,
        committee_size: fact.committee_size,
        number_of_proposers: fact.number_of_proposers,
        has_total_dpos_votes_count: false,
        total_dpos_votes_count: 0,
        future_dpos_state: false,
        unknown_error: false,
    })
}

struct VerifiedVotesAccess<'a> {
    runtime: &'a mut PbftVoteAdmissionRuntime,
    storage: &'a Storage,
}

fn verified_votes_storage<'a>(
    runtime: &'a VerifiedVotesAccess<'_>,
) -> Result<&'a Storage, anyhow::Error> {
    Ok(runtime.storage)
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

impl VerifiedVotesAccess<'_> {
    #[cfg(test)]
    fn retained_vote_payload(&self, vote_hash: Option<H256>) -> (bool, PbftVoteStorageRecord) {
        vote_hash
            .and_then(|hash| self.runtime.weighted_payload(hash).cloned())
            .map(|record| (true, record.into()))
            .unwrap_or_else(|| (false, empty_storage_record()))
    }

    #[cfg(test)]
    fn retained_vote_bucket(&self, vote: &VerifiedVote) -> (bool, VerifiedStepVotePayloadEntry) {
        let Some(bucket) = self
            .runtime
            .verified_votes()
            .get_step_votes(vote.period, vote.round, vote.step)
            .and_then(|entries| {
                entries
                    .into_iter()
                    .find(|entry| entry.block_hash == vote.block_hash)
            })
        else {
            return (false, empty_step_vote_payload_entry());
        };
        let records = bucket
            .vote_hashes
            .into_iter()
            .map(|hash| self.runtime.weighted_payload(hash).cloned())
            .collect::<Option<Vec<_>>>();
        let Some(records) = records else {
            return (false, empty_step_vote_payload_entry());
        };
        (
            true,
            VerifiedStepVotePayloadEntry {
                block_hash: bucket.block_hash.into(),
                total_weight: bucket.total_weight,
                votes: records.into_iter().map(Into::into).collect(),
            },
        )
    }

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
        fact: PbftTwoTPlusOneThresholdFact,
    ) -> PbftTwoTPlusOneThresholdPlan {
        self.runtime.plan_two_t_plus_one_threshold(fact)
    }

    /// Inserts `vote` into unique-voter tracking.
    #[cfg(test)]
    fn verified_votes_insert_unique_voter(
        &mut self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<UniqueVoterInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let weighted_vote = validate_mutation_weighted_vote(&vote, weighted_vote)?;
        let outcome = self.runtime.verified_votes_mut().insert_unique_voter(&vote);
        if outcome.accepted {
            self.runtime.retain_weighted_payload(&vote, weighted_vote)?;
        }
        let (conflicting_vote_found, conflicting_vote) =
            self.retained_vote_payload(outcome.conflicting_vote_hash);
        Ok(UniqueVoterInsertOutcome {
            accepted: outcome.accepted,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            conflicting_vote_found,
            conflicting_vote,
            bucket_found: false,
            bucket: empty_step_vote_payload_entry(),
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
    }

    /// Inserts `vote` into voted-value aggregation.
    ///
    /// Compatibility/test helper only; production admission must retain the
    /// canonical vote payload sidecars through `verified_votes_admit_and_persist`.
    #[cfg(test)]
    fn verified_votes_insert_voted_value(
        &mut self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<VotedValueInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let weighted_vote = validate_mutation_weighted_vote(&vote, weighted_vote)?;
        let outcome = self
            .runtime
            .verified_votes_mut()
            .insert_voted_value(vote.clone())?;
        if outcome.inserted {
            self.runtime.retain_weighted_payload(&vote, weighted_vote)?;
        }
        let (bucket_found, bucket) = self.retained_vote_bucket(&vote);
        Ok(VotedValueInsertOutcome {
            inserted: outcome.inserted,
            total_weight: outcome.total_weight,
            votes_count: outcome.votes_count,
            conflicting_vote_found: false,
            conflicting_vote: empty_storage_record(),
            bucket_found,
            bucket,
        })
    }

    /// Atomically inserts `vote` into unique-voter and voted-value state.
    ///
    /// This returns conflict details for slashing decisions when uniqueness
    /// fails and voted-value aggregation counters when insertion succeeds.
    /// Compatibility/test helper only; production routing should not bypass the
    /// canonical admission runtime because threshold bundles and slashing
    /// evidence require retained payload records.
    #[cfg(test)]
    fn verified_votes_insert_vote_atomic(
        &mut self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<AtomicVoteInsertOutcome, anyhow::Error> {
        let vote = payload_to_vote(vote)?;
        let weighted_vote = validate_mutation_weighted_vote(&vote, weighted_vote)?;
        let outcome = self
            .runtime
            .verified_votes_mut()
            .insert_vote_atomic(vote.clone())?;
        if outcome.inserted {
            self.runtime.retain_weighted_payload(&vote, weighted_vote)?;
        }
        let (conflicting_vote_found, conflicting_vote) =
            self.retained_vote_payload(outcome.conflicting_vote_hash);
        let (bucket_found, bucket) = self.retained_vote_bucket(&vote);
        Ok(AtomicVoteInsertOutcome {
            inserted: outcome.inserted,
            total_weight: outcome.total_weight,
            votes_count: outcome.votes_count,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            conflicting_vote_found,
            conflicting_vote,
            bucket_found,
            bucket,
            used_secondary_slot: outcome.used_secondary_slot,
            duplicate_vote_hash: outcome.duplicate_vote_hash,
        })
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

    /// Adds one vote fact with optional threshold side effects.
    #[cfg(test)]
    fn verified_votes_add_verified_vote(
        &mut self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
        two_t_plus_one_threshold: u64,
        apply_threshold_decision: bool,
    ) -> Result<FfiVerifiedVoteAddOutcome, anyhow::Error> {
        let report_vote = copy_vote_payload(&vote);
        let vote = payload_to_vote(vote)?;
        let weighted_vote = validate_mutation_weighted_vote(&vote, weighted_vote)?;
        let threshold = if apply_threshold_decision {
            Some(two_t_plus_one_threshold)
        } else {
            None
        };

        let outcome = self
            .runtime
            .verified_votes_mut()
            .add_verified_vote(vote.clone(), threshold)?;
        if outcome.inserted {
            self.runtime.retain_weighted_payload(&vote, weighted_vote)?;
        }
        let (conflicting_vote_found, conflicting_vote) =
            self.retained_vote_payload(outcome.conflicting_vote_hash);
        let (bucket_found, bucket) = self.retained_vote_bucket(&vote);
        Ok(outcome_to_ffi_add_vote_outcome(
            report_vote,
            outcome,
            conflicting_vote_found,
            conflicting_vote,
            bucket_found,
            bucket,
        ))
    }

    /// Persists and publishes one already validated PBFT vote transition.
    ///
    /// The caller must supply validation produced for the exact canonical vote bytes. This helper owns the bounded
    /// replay/round/payload checkpoint and commits required progress writes before publishing any mutation or effect.
    fn verified_votes_admit_prevalidated(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation: PbftCanonicalVoteValidation,
        flags: PbftVoteEventFactFlags,
        context: FfiPbftVoteProgressContext,
    ) -> Result<PbftVoteAdmissionRuntimeResult, anyhow::Error> {
        let storage = self.storage;
        let result = self.runtime.admit_validated_vote_transactional(
            canonical_vote_rlp,
            &validation,
            flags_to_domain(flags),
            context_to_domain(&context),
            |write| persist_pbft_vote_progress(storage, write),
        )?;
        Ok(runtime_outcome_to_ffi(validation, result, context))
    }

    #[cfg(test)]
    fn verified_votes_admit_and_persist(
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
        self.verified_votes_admit_prevalidated(canonical_vote_rlp, validation, flags, context)
    }

    /// Removes periods lower than `pbft_period`.
    pub fn verified_votes_cleanup_votes_by_period(&mut self, pbft_period: u64) {
        self.runtime.cleanup_votes_by_period(pbft_period);
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
        let storage = self.storage;
        let result = self
            .runtime
            .commit_reward_vote_cursor(storage, cursor, reset_generation)?;
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
    /// clears and PBFT-service production queries sharing the storage.
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
    /// and PBFT-service production queries sharing the storage.
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
    #[cfg(test)]
    fn verified_votes_apply_pbft_finalization_storage_writes(
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

    /// Returns all C++ materialization state from one coherent vote-runtime lock epoch.
    fn verified_votes_state_snapshot(&self) -> Result<VerifiedVotesStateSnapshot, anyhow::Error> {
        let votes = self
            .runtime
            .verified_votes()
            .snapshot_votes()
            .into_iter()
            .map(|vote| {
                let weighted_vote = self
                    .runtime
                    .weighted_payload(vote.vote_hash)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!("PBFT_VERIFIED_VOTES_MISSING_RETAINED_PAYLOAD")
                    })?;
                Ok(VerifiedVoteStateSnapshotEntry {
                    vote: vote.into(),
                    weighted_vote: weighted_vote.into(),
                })
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;
        Ok(VerifiedVotesStateSnapshot {
            votes,
            round_markers: self.verified_votes_snapshot_round_markers(),
            two_t_plus_one: self.verified_votes_snapshot_two_t_plus_one(),
        })
    }

    /// Returns one step's buckets with retained records aligned to canonical hash order.
    fn verified_votes_step_payloads(
        &self,
        period: u64,
        round: u64,
        step: u64,
    ) -> Result<VerifiedStepVotePayloadsLookup, anyhow::Error> {
        let Some(entries) = self
            .runtime
            .verified_votes()
            .get_step_votes(period, round, step)
        else {
            return Ok(VerifiedStepVotePayloadsLookup {
                found: false,
                entries: Vec::new(),
            });
        };
        let entries = entries
            .into_iter()
            .map(|entry| {
                let votes = entry
                    .vote_hashes
                    .into_iter()
                    .map(|vote_hash| {
                        self.runtime
                            .weighted_payload(vote_hash)
                            .cloned()
                            .map(Into::into)
                            .ok_or_else(|| {
                                anyhow::anyhow!("PBFT_VERIFIED_VOTES_STEP_MISSING_RETAINED_PAYLOAD")
                            })
                    })
                    .collect::<Result<Vec<_>, anyhow::Error>>()?;
                Ok(VerifiedStepVotePayloadEntry {
                    block_hash: entry.block_hash.into(),
                    total_weight: entry.total_weight,
                    votes,
                })
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;
        Ok(VerifiedStepVotePayloadsLookup {
            found: true,
            entries,
        })
    }

    /// Returns the reward cursor and its retained payloads from one runtime lock epoch.
    fn verified_votes_current_reward_snapshot(
        &self,
    ) -> Result<RewardVotePayloadSnapshot, anyhow::Error> {
        Ok(RewardVotePayloadSnapshot {
            cursor: self.verified_votes_reward_vote_cursor(),
            records: self.verified_votes_current_reward_vote_payloads()?,
        })
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
impl BridgePbftService {
    fn with_verified_votes<T>(
        &self,
        operation: impl FnOnce(&mut VerifiedVotesAccess<'_>) -> Result<T, anyhow::Error>,
    ) -> Result<T, anyhow::Error> {
        let service = self.verified_votes();
        let mut runtime = service.lock()?;
        operation(&mut VerifiedVotesAccess {
            runtime: &mut runtime,
            storage: service.storage(),
        })
    }

    fn publish_vote_validation(
        &self,
        validation: PbftCanonicalVoteValidation,
        weighted_vote_rlp: Vec<u8>,
    ) -> Result<PbftVoteRuntimeValidationResult, anyhow::Error> {
        let replay = self
            .with_verified_votes(|votes| Ok(votes.runtime.record_validation_replay(&validation)))?;
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
        let validation = self.pbft_service_verified_votes_validate_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            strict_vrf,
            committee_size,
            number_of_proposers,
            false,
            0,
        )?;
        let weighted_vote_rlp = if validation.accepted && validation.weight_calculated {
            build_weighted_pbft_vote_payload(canonical_vote_rlp, validation.calculated_weight)?
                .vote_rlp
        } else {
            Vec::new()
        };
        self.publish_vote_validation(validation, weighted_vote_rlp)
    }

    fn pbft_service_verified_votes_validate_with_final_chain_internal(
        &self,
        final_chain: &BridgeFinalChain,
        canonical_vote_rlp: &[u8],
        strict_vrf: bool,
        committee_size: u64,
        number_of_proposers: u64,
        has_preverified_weight: bool,
        preverified_weight: u64,
    ) -> Result<PbftCanonicalVoteValidation, anyhow::Error> {
        let inspection = inspect_canonical_pbft_vote(canonical_vote_rlp)?;
        let request = admission_validation_request_to_domain(PbftVoteAdmissionValidationRequest {
            strict_vrf,
            committee_size,
            number_of_proposers,
            has_preverified_weight,
            preverified_weight,
        });
        let mut facts = admission_validation_request_to_facts(request);

        if inspection.status != PbftCanonicalVoteInspectionStatus::Valid {
            return validate_canonical_pbft_vote(canonical_vote_rlp, facts);
        }

        if facts.has_preverified_weight {
            return validate_canonical_pbft_vote(canonical_vote_rlp, facts);
        }

        let Some(dpos_period) = inspection.period.checked_sub(1) else {
            facts.unknown_error = true;
            return validate_canonical_pbft_vote(canonical_vote_rlp, facts);
        };
        let voter = inspection.recovered_voter.0;
        match final_chain
            .0
            .pbft_dpos_eligible_vote_count(dpos_period, voter)
        {
            Ok(Some(votes)) => {
                facts.voter_dpos_ready = true;
                facts.voter_dpos_vote_count = votes;
            }
            Ok(None) => facts.future_dpos_state = true,
            Err(_) => facts.unknown_error = true,
        }
        let validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        if validation.rejected || facts.future_dpos_state || facts.unknown_error {
            return Ok(validation);
        }

        let cached_key =
            self.with_verified_votes(|votes| Ok(votes.runtime.validation_vrf_key(voter)))?;
        let mut key_lookup_error = false;
        if let Some(key) = cached_key {
            facts.has_vrf_key = true;
            facts.vrf_public_key = key;
        } else {
            match final_chain.0.pbft_vrf_key_with_fallback(dpos_period, voter) {
                Ok(Some(key)) => {
                    facts.has_vrf_key = true;
                    facts.vrf_public_key = key;
                    self.with_verified_votes(|votes| {
                        votes.runtime.cache_validation_vrf_key(voter, key);
                        Ok(())
                    })?;
                }
                Ok(None) => {}
                Err(_) => key_lookup_error = true,
            }
        }
        facts.vrf_key_ready = !key_lookup_error;
        facts.unknown_error = key_lookup_error;
        let validation = validate_canonical_pbft_vote(canonical_vote_rlp, facts)?;
        if validation.rejected || facts.unknown_error {
            return Ok(validation);
        }

        match final_chain
            .0
            .pbft_dpos_eligible_total_vote_count(dpos_period)
        {
            Ok(Some(total)) => {
                facts.total_dpos_ready = true;
                facts.total_dpos_vote_count = total;
            }
            Ok(None) => facts.future_dpos_state = true,
            Err(_) => facts.unknown_error = true,
        }
        validate_canonical_pbft_vote(canonical_vote_rlp, facts)
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
        let validation = self.pbft_service_verified_votes_validate_with_final_chain_internal(
            final_chain,
            canonical_vote_rlp,
            validation_request.strict_vrf,
            validation_request.committee_size,
            validation_request.number_of_proposers,
            validation_request.has_preverified_weight,
            validation_request.preverified_weight,
        )?;
        self.with_verified_votes(|votes| {
            votes.verified_votes_admit_prevalidated(canonical_vote_rlp, validation, flags, context)
        })
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
        let current_pbft_chain_size = self
            .chain()
            .read()
            .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?
            .state
            .head()
            .size;
        let initial_fact = match threshold_fact_from_request(&fact, current_pbft_chain_size) {
            Ok(fact) => fact,
            Err(plan) => return Ok(threshold_plan_to_ffi(plan)),
        };
        let initial = self.with_verified_votes(|votes| {
            Ok(votes.verified_votes_two_t_plus_one_threshold(initial_fact))
        })?;
        if !initial.needs_total_dpos_votes {
            return Ok(threshold_plan_to_ffi(initial));
        }

        let mut enriched = initial_fact;
        match final_chain
            .0
            .pbft_dpos_eligible_total_vote_count(enriched.pbft_period)
        {
            Ok(Some(total)) => {
                enriched.has_total_dpos_votes_count = true;
                enriched.total_dpos_votes_count = total;
            }
            Ok(None) => enriched.future_dpos_state = true,
            Err(_) => enriched.unknown_error = true,
        }

        enriched.current_pbft_chain_size = self
            .chain()
            .read()
            .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?
            .state
            .head()
            .size;

        self.with_verified_votes(
            |votes| Ok(votes.verified_votes_two_t_plus_one_threshold(enriched)),
        )
        .map(threshold_plan_to_ffi)
    }

    /// Captures the complete owned proposal-vote candidate set for one period and round.
    ///
    /// The method acquires `verified_votes`, `proposed_blocks` (read), then
    /// `chain` (read), returns candidates sorted by vote hash, and fingerprints
    /// every vote/proposal/chain fact that finish must revalidate. Missing,
    /// null, already-finalized, and already-valid candidates remain explicit
    /// snapshot states; only an unavailable vote runtime is a bridge error.
    pub fn pbft_service_prepare_leader_selection(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftLeaderSelectionSnapshot, anyhow::Error> {
        self.with_verified_votes(|votes| {
            let proposed = self
                .proposed_blocks()
                .read()
                .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_PROPOSED_BLOCKS_LOCK_POISONED"))?;
            let _chain = self
                .chain()
                .read()
                .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?;
            build_leader_selection_snapshot(votes, &proposed, votes.storage, period, round)
        })
    }

    /// Revalidates and finishes one prepared PBFT leader selection.
    ///
    /// The method acquires `verified_votes`, `proposed_blocks` (write), then
    /// `chain` (read), rebuilds the content fingerprint before any mutation,
    /// validates the external report set exactly, invokes the existing Rust
    /// leader planner, and marks only planner-emitted `valid_blocks`. Stale or
    /// malformed reports return an owned non-selected result without changing
    /// proposed-block validity. The selected vote and block bytes are copied
    /// before all guards are released.
    pub fn pbft_service_finish_leader_selection(
        &self,
        request: PbftLeaderSelectionFinishRequest,
    ) -> Result<PbftLeaderSelectionResult, anyhow::Error> {
        self.with_verified_votes(|votes| {
            let mut proposed = self
                .proposed_blocks()
                .write()
                .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_PROPOSED_BLOCKS_LOCK_POISONED"))?;
            let _chain = self
                .chain()
                .read()
                .map_err(|_| anyhow::anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?;
            let snapshot = build_leader_selection_snapshot(
                votes,
                &proposed,
                votes.storage,
                request.period,
                request.round,
            )?;
            if snapshot.snapshot_fingerprint != request.snapshot_fingerprint {
                return Ok(empty_leader_selection_result(
                    PBFT_LEADER_STALE_SNAPSHOT,
                    "PBFT_LEADER_SELECTION_STALE_SNAPSHOT",
                ));
            }

            let validations = match validate_leader_selection_reports(
                &snapshot.candidates,
                request.validations,
            ) {
                Ok(validations) => validations,
                Err(()) => {
                    return Ok(empty_leader_selection_result(
                        PBFT_LEADER_INVALID_VALIDATION_REPORT,
                        "PBFT_LEADER_SELECTION_INVALID_VALIDATION_REPORT",
                    ));
                }
            };

            let mut facts = Vec::with_capacity(snapshot.candidates.len());
            for candidate in &snapshot.candidates {
                let inspection = inspect_canonical_pbft_vote(&candidate.vote_record.vote_rlp)?;
                let validation_status = if candidate.proposed_block_is_valid {
                    PbftManagerLeaderBlockValidationStatus::AlreadyValid
                } else {
                    match validations.get(&H256::from(candidate.vote_hash)).copied() {
                        Some(PBFT_LEADER_VALIDATED) => {
                            PbftManagerLeaderBlockValidationStatus::Validated
                        }
                        Some(PBFT_LEADER_REJECTED) => {
                            PbftManagerLeaderBlockValidationStatus::Rejected
                        }
                        None => PbftManagerLeaderBlockValidationStatus::Rejected,
                        Some(_) => unreachable!("validation statuses were checked"),
                    }
                };
                facts.push(PbftManagerLeaderCandidateInputFact {
                    vote_hash: H256::from(candidate.vote_hash),
                    block_hash: H256::from(candidate.block_hash),
                    period: request.period,
                    credential: vrf::proof_to_hash(&inspection.vrf_proof)?,
                    voter_public_key: inspection.recovered_public_key,
                    weight_found: inspection.has_embedded_weight,
                    weight: inspection.embedded_weight,
                    block_in_chain: candidate.block_in_chain,
                    proposed_block_found: candidate.proposed_block_found,
                    block_validation_status: validation_status,
                    pivot_hash: H256::from(candidate.pivot_hash),
                });
            }

            let plan = plan_pbft_manager_leader_candidates(facts);
            if plan.status == PbftManagerLeaderSelectionStatus::InvalidFact {
                return Ok(empty_leader_selection_result(
                    PBFT_LEADER_INVALID_VALIDATION_REPORT,
                    plan.error_code,
                ));
            }
            let selected_payload = if plan.selected {
                snapshot.candidates.iter().find(|candidate| {
                    candidate.vote_hash == plan.selected_vote_hash.0
                        && candidate.block_hash == plan.selected_block_hash.0
                })
            } else {
                None
            };
            if plan.selected && selected_payload.is_none() {
                return Err(anyhow::anyhow!(
                    "PBFT_LEADER_SELECTION_PLANNER_SELECTED_UNKNOWN_CANDIDATE"
                ));
            }

            for command in &plan.valid_blocks {
                proposed.mark_valid(command.period, command.block_hash)?;
            }

            let Some(selected) = selected_payload else {
                return Ok(empty_leader_selection_result(
                    if snapshot.candidates.is_empty() {
                        PBFT_LEADER_NO_CANDIDATES
                    } else {
                        PBFT_LEADER_NO_ELIGIBLE
                    },
                    plan.error_code,
                ));
            };
            Ok(PbftLeaderSelectionResult {
                status: PBFT_LEADER_SELECTED,
                error_code: plan.error_code.to_owned(),
                selected: true,
                selected_vote: PbftVoteStorageRecord {
                    hash: selected.vote_record.hash,
                    vote_rlp: selected.vote_record.vote_rlp.clone(),
                },
                selected_block_rlp: selected.proposed_block_rlp.clone(),
            })
        })
    }
}

fn build_leader_selection_snapshot(
    votes: &VerifiedVotesAccess<'_>,
    proposed: &rustaxa_consensus::proposed_blocks::ProposedBlocks,
    storage: &Storage,
    period: u64,
    round: u64,
) -> Result<PbftLeaderSelectionSnapshot, anyhow::Error> {
    let mut candidates = Vec::new();
    for vote in votes
        .runtime
        .verified_votes()
        .snapshot_votes()
        .into_iter()
        .filter(|vote| {
            vote.period == period
                && vote.round == round
                && vote.step == 1
                && vote.vote_type == PbftVoteType::Propose
        })
    {
        let vote_record = votes
            .runtime
            .weighted_payload(vote.vote_hash)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("PBFT_LEADER_SELECTION_MISSING_VOTE_PAYLOAD"))?;
        let proposed_block = proposed.get(period, vote.block_hash);
        let block_in_chain = if vote.block_hash == H256::zero() {
            false
        } else {
            pbft_block_exists_in_storage(storage, vote.block_hash)?
        };
        let proposed_block_found = proposed_block.is_some();
        let proposed_block_is_valid = proposed_block
            .as_ref()
            .map(|block| block.is_valid)
            .unwrap_or(false);
        candidates.push(PbftLeaderCandidateSnapshot {
            vote_hash: vote.vote_hash.into(),
            block_hash: vote.block_hash.into(),
            vote_record: vote_record.into(),
            proposed_block_found,
            proposed_block_is_valid,
            proposed_block_rlp: proposed_block
                .as_ref()
                .map(|block| block.block_rlp.clone())
                .unwrap_or_default(),
            pivot_hash: proposed_block
                .as_ref()
                .map(|block| block.pivot_hash.into())
                .unwrap_or([0; 32]),
            block_in_chain,
            needs_external_validation: vote.block_hash != H256::zero()
                && !block_in_chain
                && proposed_block_found
                && !proposed_block_is_valid,
        });
    }
    candidates.sort_by_key(|candidate| candidate.vote_hash);
    let snapshot_fingerprint = leader_selection_fingerprint(period, round, &candidates);
    Ok(PbftLeaderSelectionSnapshot {
        status: if candidates.is_empty() {
            PBFT_LEADER_NO_CANDIDATES
        } else {
            PBFT_LEADER_SELECTED
        },
        error_code: if candidates.is_empty() {
            "PBFT_LEADER_SELECTION_NO_CANDIDATES"
        } else {
            "PBFT_LEADER_SELECTION_READY"
        }
        .to_owned(),
        period,
        round,
        snapshot_fingerprint,
        candidates,
    })
}

fn validate_leader_selection_reports(
    candidates: &[PbftLeaderCandidateSnapshot],
    reports: Vec<PbftLeaderCandidateValidation>,
) -> Result<BTreeMap<H256, u8>, ()> {
    let expected = candidates
        .iter()
        .filter(|candidate| candidate.needs_external_validation)
        .map(|candidate| (H256::from(candidate.vote_hash), candidate.block_hash))
        .collect::<BTreeMap<_, _>>();
    if reports.len() != expected.len() {
        return Err(());
    }
    let mut seen = BTreeSet::new();
    let mut validated = BTreeMap::new();
    for report in reports {
        let vote_hash = H256::from(report.vote_hash);
        if !seen.insert(vote_hash)
            || expected.get(&vote_hash).copied() != Some(report.block_hash)
            || !matches!(report.status, PBFT_LEADER_VALIDATED | PBFT_LEADER_REJECTED)
        {
            return Err(());
        }
        validated.insert(vote_hash, report.status);
    }
    Ok(validated)
}

fn leader_selection_fingerprint(
    period: u64,
    round: u64,
    candidates: &[PbftLeaderCandidateSnapshot],
) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(b"RUSTAXA_PBFT_LEADER_SELECTION_V1");
    hasher.update(&period.to_be_bytes());
    hasher.update(&round.to_be_bytes());
    hasher.update(&(candidates.len() as u64).to_be_bytes());
    for candidate in candidates {
        hasher.update(&candidate.vote_hash);
        hasher.update(&candidate.block_hash);
        hasher.update(&candidate.vote_record.hash);
        hasher.update(&keccak256(&candidate.vote_record.vote_rlp));
        hasher.update(&[u8::from(candidate.proposed_block_found)]);
        hasher.update(&[u8::from(candidate.proposed_block_is_valid)]);
        hasher.update(&candidate.pivot_hash);
        hasher.update(&keccak256(&candidate.proposed_block_rlp));
        hasher.update(&[u8::from(candidate.block_in_chain)]);
    }
    let mut fingerprint = [0; 32];
    hasher.finalize(&mut fingerprint);
    fingerprint
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut output = [0; 32];
    hasher.finalize(&mut output);
    output
}

fn empty_leader_selection_result(status: u8, error_code: &str) -> PbftLeaderSelectionResult {
    PbftLeaderSelectionResult {
        status,
        error_code: error_code.to_owned(),
        selected: false,
        selected_vote: empty_storage_record(),
        selected_block_rlp: Vec::new(),
    }
}

macro_rules! service_verified_votes_plain {
    ($(fn $name:ident($($arg:ident: $arg_ty:ty),*) -> $result:ty => $inner:ident;)*) => {
        impl BridgePbftService {
            $(
                #[doc = "Runs the named verified-votes operation under the service-owned vote mutex."]
                pub fn $name(&self, $($arg: $arg_ty),*) -> Result<$result, anyhow::Error> {
                    self.with_verified_votes(|votes| Ok(votes.$inner($($arg),*)))
                }
            )*
        }
    };
}

macro_rules! service_verified_votes_fallible {
    ($(fn $name:ident($($arg:ident: $arg_ty:ty),*) -> $result:ty => $inner:ident;)*) => {
        impl BridgePbftService {
            $(
                #[doc = "Runs the named fallible verified-votes operation under the service-owned vote mutex."]
                pub fn $name(&self, $($arg: $arg_ty),*) -> Result<$result, anyhow::Error> {
                    self.with_verified_votes(|votes| votes.$inner($($arg),*))
                }
            )*
        }
    };
}

service_verified_votes_plain! {
    fn pbft_service_verified_votes_size() -> u64 => verified_votes_size;
    fn pbft_service_verified_votes_replay_contains(vote_hash: &[u8; 32]) -> bool => verified_votes_replay_contains;
    fn pbft_service_verified_votes_replay_insert(vote_hash: &[u8; 32]) -> bool => verified_votes_replay_insert;
    fn pbft_service_verified_votes_determine_new_round(period: u64, current_round: u64) -> DetermineNewRoundOutcome => verified_votes_determine_new_round;
    fn pbft_service_verified_votes_plan_next_votes_bundle_egress(period: u64, round: u64) -> PbftNextVotesBundleEgressPlan => verified_votes_plan_next_votes_bundle_egress;
    fn pbft_service_verified_votes_build_optimized_votes_bundle_egress(request: PbftOptimizedVoteBundleBuildRequest) -> PbftOptimizedVoteBundleBuildResult => verified_votes_build_optimized_votes_bundle_egress;
    fn pbft_service_verified_votes_cleanup_votes_by_period(pbft_period: u64) -> () => verified_votes_cleanup_votes_by_period;
    fn pbft_service_verified_votes_reward_vote_cursor() -> FfiRewardVoteCursorSnapshot => verified_votes_reward_vote_cursor;
    fn pbft_service_verified_votes_reward_vote_period() -> u64 => verified_votes_reward_vote_period;
}

service_verified_votes_fallible! {
    fn pbft_service_verified_votes_own_vote_records() -> Vec<PbftVoteStorageRecord> => verified_votes_own_vote_records;
    fn pbft_service_verified_votes_get_two_t_plus_one_voted_block(period: u64, round: u64, kind: u8) -> TwoTPlusOneVotedBlockLookup => verified_votes_get_two_t_plus_one_voted_block;
    fn pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(period: u64, round: u64, kind: u8) -> TwoTPlusOneVotePayloadsLookup => verified_votes_get_two_t_plus_one_voted_block_payloads;
    fn pbft_service_verified_votes_select_reward_vote_payloads(block_period: u64, requested_vote_hashes: Vec<PbftFinalizationHash>) -> FfiPbftRewardVotePayloadSelection => verified_votes_select_reward_vote_payloads;
    fn pbft_service_verified_votes_commit_reward_vote_cursor(write_intent: &crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan, reset_generation: u64) -> FfiRewardVoteCursorCommitResult => verified_votes_commit_reward_vote_cursor;
    fn pbft_service_verified_votes_save_own_verified_vote(record: PbftVoteStorageRecord) -> crate::ffi::rustaxa_ffi::PbftVotePersistenceResult => verified_votes_save_own_verified_vote;
    fn pbft_service_verified_votes_clear_own_verified_votes() -> crate::ffi::rustaxa_ffi::PbftVotePersistenceResult => verified_votes_clear_own_verified_votes;
    fn pbft_service_verified_votes_persist_pbft_vote_progress(write: crate::ffi::rustaxa_ffi::PbftVoteProgressPersistenceWrite) -> crate::ffi::rustaxa_ffi::PbftVotePersistenceResult => verified_votes_persist_pbft_vote_progress;
    fn pbft_service_verified_votes_prepare_reward_votes_reset_stage(write_intent: &crate::ffi::rustaxa_ffi::PbftFinalizationStorageWritePlan) -> crate::ffi::rustaxa_ffi::PbftFinalizationStorageWriteStage => verified_votes_prepare_reward_votes_reset_stage;
    fn pbft_service_verified_votes_apply_reward_votes_reset(request: FfiPbftRewardVotesResetRequest) -> crate::ffi::rustaxa_ffi::PbftFinalizedPeriodApplyResult => verified_votes_apply_reward_votes_reset;
    fn pbft_service_verified_votes_state_snapshot() -> VerifiedVotesStateSnapshot => verified_votes_state_snapshot;
    fn pbft_service_verified_votes_step_payloads(period: u64, round: u64, step: u64) -> VerifiedStepVotePayloadsLookup => verified_votes_step_payloads;
    fn pbft_service_verified_votes_current_reward_snapshot() -> RewardVotePayloadSnapshot => verified_votes_current_reward_snapshot;
}

#[cfg(test)]
impl BridgePbftService {
    fn pbft_service_verified_votes_insert_unique_voter(
        &self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<UniqueVoterInsertOutcome, anyhow::Error> {
        self.with_verified_votes(|votes| {
            votes.verified_votes_insert_unique_voter(vote, weighted_vote)
        })
    }

    fn pbft_service_verified_votes_insert_voted_value(
        &self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<VotedValueInsertOutcome, anyhow::Error> {
        self.with_verified_votes(|votes| {
            votes.verified_votes_insert_voted_value(vote, weighted_vote)
        })
    }

    fn pbft_service_verified_votes_insert_vote_atomic(
        &self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
    ) -> Result<AtomicVoteInsertOutcome, anyhow::Error> {
        self.with_verified_votes(|votes| {
            votes.verified_votes_insert_vote_atomic(vote, weighted_vote)
        })
    }

    fn pbft_service_verified_votes_add_verified_vote(
        &self,
        vote: VerifiedVotePayload,
        weighted_vote: PbftVoteStorageRecord,
        two_t_plus_one_threshold: u64,
        apply_threshold_decision: bool,
    ) -> Result<FfiVerifiedVoteAddOutcome, anyhow::Error> {
        self.with_verified_votes(|votes| {
            votes.verified_votes_add_verified_vote(
                vote,
                weighted_vote,
                two_t_plus_one_threshold,
                apply_threshold_decision,
            )
        })
    }

    /// Exercises the retired externally supplied-facts admission path in native
    /// Rust tests without restoring it to the CXX surface.
    fn pbft_service_verified_votes_admit_and_persist(
        &self,
        canonical_vote_rlp: &[u8],
        validation_facts: PbftVoteValidationExternalFacts,
        flags: PbftVoteEventFactFlags,
        context: FfiPbftVoteProgressContext,
    ) -> Result<PbftVoteAdmissionRuntimeResult, anyhow::Error> {
        self.with_verified_votes(|votes| {
            votes.verified_votes_admit_and_persist(
                canonical_vote_rlp,
                validation_facts,
                flags,
                context,
            )
        })
    }
}

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

#[cfg(test)]
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

fn admission_validation_request_to_facts(
    value: DomainPbftVoteAdmissionValidationRequest,
) -> DomainPbftVoteValidationExternalFacts {
    DomainPbftVoteValidationExternalFacts {
        voter_dpos_ready: false,
        voter_dpos_vote_count: 0,
        total_dpos_ready: false,
        total_dpos_vote_count: 0,
        future_dpos_state: false,
        unknown_error: false,
        vrf_key_ready: false,
        has_vrf_key: false,
        vrf_public_key: [0; 32],
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
    transaction: PbftVoteAdmissionTransactionResult,
    context: FfiPbftVoteProgressContext,
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
            (payloads.incoming.into(), payloads.conflicting.into())
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

fn empty_step_vote_payload_entry() -> VerifiedStepVotePayloadEntry {
    VerifiedStepVotePayloadEntry {
        block_hash: [0; 32],
        total_weight: 0,
        votes: Vec::new(),
    }
}

#[cfg(test)]
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

#[cfg(test)]
fn validate_mutation_weighted_vote(
    vote: &VerifiedVote,
    record: PbftVoteStorageRecord,
) -> Result<DomainPbftVotePayloadRecord, anyhow::Error> {
    let record = vote_storage_record_to_domain(record);
    let inspection = inspect_canonical_pbft_vote(&record.vote_rlp)?;
    anyhow::ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid
            && inspection.signature_valid
            && inspection.has_embedded_weight,
        "PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_INVALID"
    );
    anyhow::ensure!(
        record.hash == vote.vote_hash
            && inspection.vote_hash == vote.vote_hash
            && inspection.block_hash == vote.block_hash
            && inspection.recovered_voter == vote.voter
            && inspection.period == vote.period
            && inspection.round == vote.round
            && inspection.step == vote.step
            && inspection.vote_type == vote.vote_type
            && inspection.embedded_weight == vote.weight,
        "PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_METADATA_MISMATCH"
    );
    Ok(DomainPbftVotePayloadRecord {
        hash: record.hash,
        vote_rlp: record.vote_rlp,
    })
}

#[cfg(test)]
impl From<rustaxa_consensus::verified_votes::VotedValueInsertOutcome> for VotedValueInsertOutcome {
    fn from(value: rustaxa_consensus::verified_votes::VotedValueInsertOutcome) -> Self {
        Self {
            inserted: value.inserted,
            total_weight: value.total_weight,
            votes_count: value.votes_count,
            conflicting_vote_found: false,
            conflicting_vote: empty_storage_record(),
            bucket_found: false,
            bucket: empty_step_vote_payload_entry(),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi;
    use crate::ffi::rustaxa_ffi::PbftServiceConfig;
    use crate::final_chain::create_final_chain;
    use crate::pillar_chain::create_pillar_test_service_from_storage as full_service;
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
    use rustaxa_consensus::pbft_vote_runtime::{
        PbftVoteAdmissionPersistenceStatus, PbftVoteRuntimeAdmissionOutcome,
        PbftVoteRuntimeReplayOutcome,
    };
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

    fn verified_votes_service_for_test(
        storage: Option<&BridgeStorage>,
    ) -> Result<Box<BridgePbftService>, anyhow::Error> {
        let storage = storage
            .map(|storage| BridgeStorage(storage.0.clone()))
            .unwrap_or_else(|| temp_bridge_storage("native_owner_fixture"));
        crate::pbft_manager::create_pbft_service_from_storage(
            &storage,
            PbftServiceConfig {
                genesis_lambda_ms: 100,
                cacti_lambda_max_ms: 100,
                cacti_lambda_default_ms: 100,
                cacti_block: u64::MAX,
                max_exponential_lambda_ms: 60_000,
                max_steps: 13,
                deadline_ms: 400,
                polling_interval_ms: 100,
                report_malicious_behaviour: true,
                magnolia_activation_period: 0,
            },
        )
    }

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
    fn full_service_supports_verified_votes_runtime_by_default() {
        let service = full_service(&temp_bridge_storage("full")).unwrap();
        assert_eq!(service.pbft_service_verified_votes_size().unwrap(), 0);
    }

    #[test]
    fn leader_prepare_empty_and_deterministically_fingerprints_sorted_candidates() {
        let storage = temp_bridge_storage("leader_prepare_order");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let empty = service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        assert_eq!(empty.status, PBFT_LEADER_NO_CANDIDATES);
        assert!(empty.candidates.is_empty());
        let empty_result = service
            .pbft_service_finish_leader_selection(finish_request(&empty, Vec::new()))
            .unwrap();
        assert_eq!(empty_result.status, PBFT_LEADER_NO_CANDIDATES);

        insert_proposal_vote(&service, [0x41; 32], NODE_SECRET_TWO, 2);
        insert_proposal_vote(&service, [0x42; 32], NODE_SECRET, 3);
        insert_proposed_block(&service, [0x41; 32], [0x51; 32], vec![0x41, 0x01]);
        insert_proposed_block(&service, [0x42; 32], [0x52; 32], vec![0x42, 0x02]);
        let first = service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        let second = service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        assert_eq!(first.snapshot_fingerprint, second.snapshot_fingerprint);
        assert_eq!(first.candidates.len(), 2);
        assert!(first.candidates[0].vote_hash < first.candidates[1].vote_hash);
        assert!(first
            .candidates
            .iter()
            .all(|candidate| candidate.needs_external_validation));
    }

    #[test]
    fn leader_prepare_preserves_missing_already_valid_and_in_chain_states() {
        let storage = temp_bridge_storage("leader_prepare_states");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let missing_vote = insert_proposal_vote(&service, [0x43; 32], NODE_SECRET, 2);
        let valid_vote = insert_proposal_vote(&service, [0x44; 32], NODE_SECRET_TWO, 2);
        insert_proposed_block(&service, [0x44; 32], [0x54; 32], vec![0x44]);
        service
            .proposed_blocks()
            .write()
            .unwrap()
            .mark_valid(12, H256::from([0x44; 32]))
            .unwrap();
        let in_chain_vote = insert_proposal_vote(&service, [0x45; 32], [0x52; 32], 2);
        insert_proposed_block(&service, [0x45; 32], [0x55; 32], vec![0x45]);
        storage
            .0
            .period()
            .write_pbft_period(H256::from([0x45; 32]), 12)
            .unwrap();
        let snapshot = service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        let missing = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == missing_vote)
            .unwrap();
        assert!(!missing.proposed_block_found);
        assert!(!missing.needs_external_validation);
        let valid = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == valid_vote)
            .unwrap();
        assert!(valid.proposed_block_is_valid);
        assert!(!valid.needs_external_validation);
        let in_chain = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == in_chain_vote)
            .unwrap();
        assert!(in_chain.block_in_chain);
        assert!(!in_chain.needs_external_validation);
        let result = service
            .pbft_service_finish_leader_selection(finish_request(&snapshot, Vec::new()))
            .unwrap();
        assert_eq!(result.status, PBFT_LEADER_SELECTED);
        assert_eq!(result.selected_vote.hash, valid_vote);
    }

    #[test]
    fn leader_finish_accepts_or_rejects_without_extra_materialization_reads() {
        let storage = temp_bridge_storage("leader_finish_accept_reject");
        let accepted_service = verified_votes_service_for_test(Some(&storage)).unwrap();
        insert_proposal_vote(&accepted_service, [0x46; 32], NODE_SECRET, 4);
        insert_proposed_block(&accepted_service, [0x46; 32], [0x56; 32], vec![0x46, 0x99]);
        let snapshot = accepted_service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        let result = accepted_service
            .pbft_service_finish_leader_selection(finish_request(
                &snapshot,
                vec![leader_validation(
                    &snapshot.candidates[0],
                    PBFT_LEADER_VALIDATED,
                )],
            ))
            .unwrap();
        assert_eq!(result.status, PBFT_LEADER_SELECTED);
        assert!(result.selected);
        assert_eq!(result.selected_vote.hash, snapshot.candidates[0].vote_hash);
        assert_eq!(result.selected_block_rlp, vec![0x46, 0x99]);
        assert!(
            accepted_service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(12, H256::from([0x46; 32]))
                .unwrap()
                .is_valid
        );

        let rejected_storage = temp_bridge_storage("leader_finish_rejected");
        let rejected_service = verified_votes_service_for_test(Some(&rejected_storage)).unwrap();
        insert_proposal_vote(&rejected_service, [0x47; 32], NODE_SECRET, 4);
        insert_proposed_block(&rejected_service, [0x47; 32], [0x57; 32], vec![0x47]);
        let snapshot = rejected_service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        let result = rejected_service
            .pbft_service_finish_leader_selection(finish_request(
                &snapshot,
                vec![leader_validation(
                    &snapshot.candidates[0],
                    PBFT_LEADER_REJECTED,
                )],
            ))
            .unwrap();
        assert_eq!(result.status, PBFT_LEADER_NO_ELIGIBLE);
        assert!(!result.selected);
        assert!(
            !rejected_service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(12, H256::from([0x47; 32]))
                .unwrap()
                .is_valid
        );
    }

    #[test]
    fn leader_finish_rejects_invalid_reports_without_marking_valid() {
        let storage = temp_bridge_storage("leader_invalid_reports");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        insert_proposal_vote(&service, [0x48; 32], NODE_SECRET, 3);
        insert_proposed_block(&service, [0x48; 32], [0x58; 32], vec![0x48]);
        let snapshot = service
            .pbft_service_prepare_leader_selection(12, 2)
            .unwrap();
        let invalid_cases = vec![
            Vec::new(),
            vec![leader_validation(&snapshot.candidates[0], 99)],
            vec![
                leader_validation(&snapshot.candidates[0], PBFT_LEADER_VALIDATED),
                leader_validation(&snapshot.candidates[0], PBFT_LEADER_VALIDATED),
            ],
            vec![PbftLeaderCandidateValidation {
                vote_hash: snapshot.candidates[0].vote_hash,
                block_hash: [0x99; 32],
                status: PBFT_LEADER_VALIDATED,
            }],
            vec![PbftLeaderCandidateValidation {
                vote_hash: [0x98; 32],
                block_hash: snapshot.candidates[0].block_hash,
                status: PBFT_LEADER_VALIDATED,
            }],
        ];
        for validations in invalid_cases {
            let result = service
                .pbft_service_finish_leader_selection(finish_request(&snapshot, validations))
                .unwrap();
            assert_eq!(result.status, PBFT_LEADER_INVALID_VALIDATION_REPORT);
            assert!(
                !service
                    .proposed_blocks()
                    .read()
                    .unwrap()
                    .get(12, H256::from([0x48; 32]))
                    .unwrap()
                    .is_valid
            );
        }
    }

    #[test]
    fn leader_finish_detects_vote_proposed_and_chain_staleness_without_mutation() {
        let scenarios = ["vote", "proposed", "chain"];
        for scenario in scenarios {
            let storage = temp_bridge_storage(&format!("leader_stale_{scenario}"));
            let service = verified_votes_service_for_test(Some(&storage)).unwrap();
            insert_proposal_vote(&service, [0x49; 32], NODE_SECRET, 3);
            insert_proposed_block(&service, [0x49; 32], [0x59; 32], vec![0x49]);
            let snapshot = service
                .pbft_service_prepare_leader_selection(12, 2)
                .unwrap();
            match scenario {
                "vote" => {
                    insert_proposal_vote(&service, [0x4A; 32], NODE_SECRET_TWO, 2);
                }
                "proposed" => {
                    let mut proposed = service.proposed_blocks().write().unwrap();
                    proposed.cleanup_before(13);
                    assert!(proposed.push(
                        12,
                        H256::from([0x49; 32]),
                        H256::from([0x5A; 32]),
                        vec![0x49, 0x01],
                    ));
                }
                "chain" => storage
                    .0
                    .period()
                    .write_pbft_period(H256::from([0x49; 32]), 12)
                    .unwrap(),
                _ => unreachable!(),
            }
            let result = service
                .pbft_service_finish_leader_selection(finish_request(
                    &snapshot,
                    vec![leader_validation(
                        &snapshot.candidates[0],
                        PBFT_LEADER_VALIDATED,
                    )],
                ))
                .unwrap();
            assert_eq!(result.status, PBFT_LEADER_STALE_SNAPSHOT);
            assert!(
                !service
                    .proposed_blocks()
                    .read()
                    .unwrap()
                    .get(12, H256::from([0x49; 32]))
                    .unwrap()
                    .is_valid
            );
        }
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
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
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
                .pbft_service_verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
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
        let records = votes
            .pbft_service_verified_votes_own_vote_records()
            .unwrap();
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
        let votes = verified_votes_service_for_test(Some(&reopened)).unwrap();
        assert_eq!(
            votes
                .pbft_service_verified_votes_own_vote_records()
                .unwrap()
                .len(),
            2
        );

        let result = votes
            .pbft_service_verified_votes_clear_own_verified_votes()
            .unwrap();
        assert_eq!(result.status, 0);
        assert_eq!(result.applied_writes, 2);
        assert!(votes
            .pbft_service_verified_votes_own_vote_records()
            .unwrap()
            .is_empty());

        drop(votes);
        drop(reopened);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn own_vote_records_reject_malformed_payload_and_hash_mismatch() {
        let storage = temp_bridge_storage("own_vote_validation");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();

        assert!(votes
            .pbft_service_verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
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
            .pbft_service_verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
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
        assert!(votes
            .pbft_service_verified_votes_own_vote_records()
            .is_err());

        votes
            .pbft_service_verified_votes_clear_own_verified_votes()
            .unwrap();
        storage
            .0
            .pbft()
            .write_own_verified_vote(wrong_hash, &weighted.vote_rlp)
            .unwrap();
        let error = match votes.pbft_service_verified_votes_own_vote_records() {
            Ok(_) => panic!("mismatched own-vote key must be rejected"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("VERIFIED_VOTES_OWN_VOTE_HASH_MISMATCH"));
    }

    #[test]
    fn lifecycle_reset_clears_votes_saved_and_queried_through_another_handle() {
        let storage = temp_bridge_storage("own_vote_cross_handle_lifecycle");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let weighted =
            build_weighted_pbft_vote_payload(&generated_vote([0x34; 32], NODE_SECRET).vote_rlp, 5)
                .unwrap();
        votes
            .pbft_service_verified_votes_save_own_verified_vote(PbftVoteStorageRecord {
                hash: weighted.hash.0,
                vote_rlp: weighted.vote_rlp,
            })
            .unwrap();
        assert_eq!(
            votes
                .pbft_service_verified_votes_own_vote_records()
                .unwrap()
                .len(),
            1
        );

        let runtime = crate::pbft_manager::create_pbft_manager_runtime_from_storage(
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
            &runtime,
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
        assert_eq!(
            votes
                .pbft_service_verified_votes_own_vote_records()
                .unwrap()
                .len(),
            1
        );

        let applied = crate::pbft_manager::pbft_manager_runtime_execute_lifecycle_transition(
            &runtime,
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
        assert!(votes
            .pbft_service_verified_votes_own_vote_records()
            .unwrap()
            .is_empty());
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

    fn u256_be(value: u64) -> Vec<u8> {
        ethereum_types::U256::from(value).to_big_endian().to_vec()
    }

    fn final_chain_with_one_validator(storage: &BridgeStorage) -> Box<BridgeFinalChain> {
        final_chain_with_validator(storage, [0x51; 20], [0x52; 32], 5_000)
    }

    fn final_chain_with_validator(
        storage: &BridgeStorage,
        address: [u8; 20],
        vrf_key: [u8; 32],
        stake: u64,
    ) -> Box<BridgeFinalChain> {
        create_final_chain(
            storage,
            0,
            0,
            Vec::new(),
            vec![rustaxa_ffi::GenesisValidator {
                address,
                owner: address,
                vrf_key,
                commission: 0,
                description: String::new(),
                endpoint: String::new(),
                total_stake: u256_be(stake),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: address,
                    stake: u256_be(stake),
                }],
            }],
            rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: u256_be(1_000),
                vote_eligibility_balance_step: u256_be(1_000),
                validator_maximum_stake: u256_be(30_000),
                minimum_deposit: Vec::new(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .unwrap()
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
        generated_vote_at_period(block_hash, node_secret, vote_type, 12, step)
    }

    fn generated_vote_at_period(
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        vote_type: PbftVoteType,
        period: u64,
        step: u64,
    ) -> rustaxa_consensus::PbftGeneratedVote {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type,
            period,
            round: 2,
            step,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
    }

    fn mutation_vote(
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        vote_type: PbftVoteType,
        step: u64,
        weight: u64,
    ) -> (VerifiedVotePayload, PbftVoteStorageRecord) {
        let generated = generated_vote_for_type(block_hash, node_secret, vote_type, step);
        let weighted = build_weighted_pbft_vote_payload(&generated.vote_rlp, weight).unwrap();
        (
            VerifiedVotePayload {
                vote_hash: generated.vote_hash.into(),
                block_hash: generated.block_hash.into(),
                voter: generated.voter.into(),
                period: generated.period,
                round: generated.round,
                step: generated.step,
                vote_type: generated.vote_type.into(),
                weight,
            },
            weighted.into(),
        )
    }

    fn copy_storage_record(record: &PbftVoteStorageRecord) -> PbftVoteStorageRecord {
        PbftVoteStorageRecord {
            hash: record.hash,
            vote_rlp: record.vote_rlp.clone(),
        }
    }

    fn insert_proposal_vote(
        service: &BridgePbftService,
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        weight: u64,
    ) -> [u8; 32] {
        let (vote, weighted) =
            mutation_vote(block_hash, node_secret, PbftVoteType::Propose, 1, weight);
        let vote_hash = vote.vote_hash;
        service
            .pbft_service_verified_votes_add_verified_vote(vote, weighted, u64::MAX, false)
            .unwrap();
        vote_hash
    }

    fn insert_proposed_block(
        service: &BridgePbftService,
        block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        block_rlp: Vec<u8>,
    ) {
        assert!(service.proposed_blocks().write().unwrap().push(
            12,
            H256::from(block_hash),
            H256::from(pivot_hash),
            block_rlp,
        ));
    }

    fn leader_validation(
        candidate: &PbftLeaderCandidateSnapshot,
        status: u8,
    ) -> PbftLeaderCandidateValidation {
        PbftLeaderCandidateValidation {
            vote_hash: candidate.vote_hash,
            block_hash: candidate.block_hash,
            status,
        }
    }

    fn finish_request(
        snapshot: &PbftLeaderSelectionSnapshot,
        validations: Vec<PbftLeaderCandidateValidation>,
    ) -> PbftLeaderSelectionFinishRequest {
        PbftLeaderSelectionFinishRequest {
            period: snapshot.period,
            round: snapshot.round,
            snapshot_fingerprint: snapshot.snapshot_fingerprint,
            validations,
        }
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

    fn admission_validation_request(
        has_preverified_weight: bool,
        preverified_weight: u64,
    ) -> PbftVoteAdmissionValidationRequest {
        PbftVoteAdmissionValidationRequest {
            strict_vrf: true,
            committee_size: 100,
            number_of_proposers: 20,
            has_preverified_weight,
            preverified_weight,
        }
    }

    #[test]
    fn one_shot_vote_admission_with_final_chain_accepts_preverified_weight() {
        let storage = temp_bridge_storage("one_shot_vote_admission_preverified_weight");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let vrf_key = vrf::public_key_from_secret(&VRF_SECRET).unwrap();
        let final_chain = final_chain_with_validator(&storage, voter, vrf_key, 5_000);
        let vote = generated_vote_at_period([0x73; 32], NODE_SECRET, PbftVoteType::Cert, 12, 3);

        let result = service
            .pbft_service_verified_votes_admit_and_persist_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                admission_validation_request(true, 40),
                runtime_flags(),
                runtime_context(80),
            )
            .unwrap();

        assert!(result.accepted);
        assert!(result.transition_published);
        assert!(!result.persistence_required);
        assert!(result.has_vote);
        assert_eq!(result.validation.calculated_weight, 40);
        assert_eq!(result.validation.calculated_weight, result.vote.weight);
    }

    #[test]
    fn one_shot_vote_admission_with_final_chain_rejects_zero_stake() {
        let storage = temp_bridge_storage("one_shot_vote_admission_zero_stake");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_validator(
            &storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            500,
        );
        let vote = generated_vote_at_period([0x74; 32], NODE_SECRET, PbftVoteType::Cert, 1, 3);

        let result = service
            .pbft_service_verified_votes_admit_and_persist_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                admission_validation_request(false, 0),
                runtime_flags(),
                runtime_context(80),
            )
            .unwrap();

        assert_eq!(
            result.validation.status,
            PbftVoteValidationStatus::ZeroStake.as_u8()
        );
        assert!(!result.accepted);
        assert!(result.transition_published);
        assert!(result.replay_inserted);
        assert!(!result.persistence_required);
        assert!(!result.has_verified_vote_add);
        assert_eq!(result.validation.calculated_weight, 0);
    }

    #[test]
    fn one_shot_vote_validation_uses_final_chain_and_marks_replay_once() {
        let storage = temp_bridge_storage("one_shot_vote_validation");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let vrf_key = vrf::public_key_from_secret(&VRF_SECRET).unwrap();
        let final_chain = final_chain_with_validator(&storage, voter, vrf_key, 5_000);
        let vote = generated_vote_at_period([0x71; 32], NODE_SECRET, PbftVoteType::Cert, 1, 3);

        let first = service
            .pbft_service_verified_votes_validate_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                true,
                100,
                20,
            )
            .unwrap();
        assert_eq!(first.status, PbftVoteValidationStatus::Valid.as_u8());
        assert!(first.accepted);
        assert!(first.validation.weight_calculated);
        assert!(first.validation.calculated_weight > 0);
        assert!(first.replay_should_mark);
        assert!(first.replay_inserted);
        assert!(!first.replay_already_present);

        let repeated = service
            .pbft_service_verified_votes_validate_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                true,
                100,
                20,
            )
            .unwrap();
        assert!(!repeated.replay_inserted);
        assert!(repeated.replay_already_present);
    }

    #[test]
    fn one_shot_vote_validation_stops_after_zero_voter_stake() {
        let storage = temp_bridge_storage("one_shot_zero_stake");
        let service = verified_votes_service_for_test(Some(&storage)).unwrap();
        let voter = voter_from_secret(&NODE_SECRET);
        let final_chain = final_chain_with_validator(
            &storage,
            voter,
            vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
            500,
        );
        let vote = generated_vote_at_period([0x72; 32], NODE_SECRET, PbftVoteType::Cert, 1, 3);

        let result = service
            .pbft_service_verified_votes_validate_with_final_chain(
                &final_chain,
                &vote.vote_rlp,
                true,
                100,
                20,
            )
            .unwrap();
        assert_eq!(result.status, PbftVoteValidationStatus::ZeroStake.as_u8());
        assert!(result.rejected);
        assert!(!result.validation.vrf_valid);
        assert!(!result.validation.weight_calculated);
        assert!(result.replay_inserted);
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

    fn threshold_fact(has_total_dpos_votes_count: bool) -> PbftTwoTPlusOneThresholdFact {
        PbftTwoTPlusOneThresholdFact {
            pbft_period: 3,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: 3,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count,
            total_dpos_votes_count: if has_total_dpos_votes_count { 100 } else { 0 },
            future_dpos_state: false,
            unknown_error: false,
        }
    }

    fn threshold_request(pbft_period: u64) -> FfiPbftTwoTPlusOneThresholdFact {
        FfiPbftTwoTPlusOneThresholdFact {
            pbft_period,
            vote_type: PbftVoteType::Cert.into(),
            committee_size: 100,
            number_of_proposers: 20,
        }
    }

    fn plan_threshold_for_test(
        service: &BridgePbftService,
        fact: PbftTwoTPlusOneThresholdFact,
    ) -> PbftTwoTPlusOneThresholdPlan {
        service
            .with_verified_votes(|votes| Ok(votes.verified_votes_two_t_plus_one_threshold(fact)))
            .unwrap()
    }

    #[test]
    fn composed_threshold_uses_rust_chain_and_final_chain_state() {
        let storage = temp_bridge_storage("composed_threshold_ready");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let final_chain = final_chain_with_one_validator(&storage);
        let fact = threshold_request(0);

        let plan = votes
            .pbft_service_verified_votes_two_t_plus_one_threshold_with_final_chain(
                &final_chain,
                fact,
            )
            .unwrap();

        assert_eq!(
            plan.status,
            PbftTwoTPlusOneThresholdStatus::Available.as_u8()
        );
        assert!(plan.has_threshold);
        assert_eq!(plan.threshold, 4);
    }

    #[test]
    fn composed_threshold_reports_future_final_chain_state() {
        let storage = temp_bridge_storage("composed_threshold_future");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let final_chain = final_chain_with_one_validator(&storage);
        let fact = threshold_request(1);

        let plan = votes
            .pbft_service_verified_votes_two_t_plus_one_threshold_with_final_chain(
                &final_chain,
                fact,
            )
            .unwrap();

        assert_eq!(
            plan.status,
            PbftTwoTPlusOneThresholdStatus::FutureDposState.as_u8()
        );
        assert!(!plan.has_threshold);
        assert_eq!(plan.error_code, "PBFT_TWO_T_PLUS_ONE_FUTURE_DPOS_STATE");
    }

    #[test]
    fn composed_threshold_cache_hit_does_not_require_final_chain_state() {
        let storage = temp_bridge_storage("composed_threshold_cache");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let final_chain = final_chain_with_one_validator(&storage);
        let mut seeded_fact = threshold_fact(true);
        seeded_fact.pbft_period = 1;
        seeded_fact.current_pbft_chain_size = 1;
        let seeded = plan_threshold_for_test(&votes, seeded_fact);
        assert!(seeded.cached);

        let composed_fact = threshold_request(1);
        let cached = votes
            .pbft_service_verified_votes_two_t_plus_one_threshold_with_final_chain(
                &final_chain,
                composed_fact,
            )
            .unwrap();

        assert_eq!(
            cached.status,
            PbftTwoTPlusOneThresholdStatus::Available.as_u8()
        );
        assert_eq!(cached.threshold, seeded.threshold);
    }

    #[test]
    fn bridge_add_verified_vote_reports_threshold_and_step_snapshot() {
        let votes = verified_votes_service_for_test(None).unwrap();
        let (first_vote, first_weighted) =
            mutation_vote([0x44; 32], NODE_SECRET, PbftVoteType::Next, 5, 3);
        let (second_vote, second_weighted) =
            mutation_vote([0x44; 32], NODE_SECRET_TWO, PbftVoteType::Next, 5, 2);

        let first = votes
            .pbft_service_verified_votes_add_verified_vote(first_vote, first_weighted, 5, true)
            .expect("first vote is accepted");
        assert!(first.inserted);
        assert!(first.threshold_applied);
        assert!(first.t_plus_one_reached);
        assert!(!first.two_t_plus_one_reached);

        let second = votes
            .pbft_service_verified_votes_add_verified_vote(second_vote, second_weighted, 5, true)
            .expect("second vote is accepted");
        assert!(second.inserted);
        assert!(second.two_t_plus_one_reached);
        assert!(second.two_t_plus_one_kind_found);
        assert!(second.two_t_plus_one_inserted);

        assert!(first.bucket_found);
        assert!(second.bucket_found);
        assert_eq!(second.bucket.votes.len(), 2);
    }

    #[test]
    fn bridge_admission_accepts_consecutive_generated_votes() {
        let storage = temp_bridge_storage("admission_retained_payloads");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let first = generated_vote([0x22; 32], NODE_SECRET);
        let second = generated_vote([0x22; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        let first_result = votes
            .pbft_service_verified_votes_admit_and_persist(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        assert!(first_result.accepted);
        assert!(first_result.transition_published);
        assert!(!first_result.persistence_required);

        let second_result = votes
            .pbft_service_verified_votes_admit_and_persist(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");
        assert!(second_result.accepted);
        assert!(second_result.transition_published);
        assert!(!second_result.persistence_required);

        let payloads = votes
            .pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
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

        let state = votes
            .pbft_service_verified_votes_state_snapshot()
            .expect("combined state snapshot resolves every retained payload");
        assert_eq!(state.votes.len(), 2);
        assert!(state
            .votes
            .iter()
            .all(|entry| entry.vote.vote_hash == entry.weighted_vote.hash));
        let step = votes
            .pbft_service_verified_votes_step_payloads(12, 2, 3)
            .expect("combined step snapshot resolves every retained payload");
        assert!(step.found);
        assert_eq!(step.entries.len(), 1);
        assert_eq!(step.entries[0].votes.len(), 2);
    }

    #[test]
    fn mutation_outcomes_own_conflict_and_bucket_payloads_across_cleanup() {
        let votes = verified_votes_service_for_test(None).unwrap();
        let generated = generated_vote([0x23; 32], NODE_SECRET);
        let admitted = votes
            .pbft_service_verified_votes_admit_and_persist(
                &generated.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("seed vote is admitted with a retained payload");
        let admitted_vote = copy_vote_payload(&admitted.vote);
        let admitted_weighted =
            build_weighted_pbft_vote_payload(&generated.vote_rlp, admitted_vote.weight)
                .expect("admitted vote has a canonical weighted payload");
        let admitted_storage: PbftVoteStorageRecord = admitted_weighted.into();

        let voted = votes
            .pbft_service_verified_votes_insert_voted_value(
                copy_vote_payload(&admitted_vote),
                copy_storage_record(&admitted_storage),
            )
            .unwrap();
        assert!(voted.bucket_found);
        assert_eq!(voted.bucket.votes.len(), 1);
        assert_eq!(voted.bucket.votes[0].hash, admitted_storage.hash);

        let atomic = votes
            .pbft_service_verified_votes_insert_vote_atomic(
                copy_vote_payload(&admitted_vote),
                copy_storage_record(&admitted_storage),
            )
            .unwrap();
        assert!(atomic.bucket_found);
        assert_eq!(atomic.bucket.votes[0].hash, admitted_storage.hash);

        let added = votes
            .pbft_service_verified_votes_add_verified_vote(
                copy_vote_payload(&admitted_vote),
                copy_storage_record(&admitted_storage),
                u64::MAX,
                false,
            )
            .unwrap();
        assert!(added.bucket_found);
        assert_eq!(added.bucket.votes[0].hash, admitted_storage.hash);

        let conflicting = generated_vote([0x24; 32], NODE_SECRET);
        let conflicting_weighted =
            build_weighted_pbft_vote_payload(&conflicting.vote_rlp, admitted_vote.weight).unwrap();
        let unique = votes
            .pbft_service_verified_votes_insert_unique_voter(
                VerifiedVotePayload {
                    vote_hash: conflicting.vote_hash.into(),
                    block_hash: conflicting.block_hash.into(),
                    voter: conflicting.voter.into(),
                    period: conflicting.period,
                    round: conflicting.round,
                    step: conflicting.step,
                    vote_type: conflicting.vote_type.into(),
                    weight: admitted_vote.weight,
                },
                conflicting_weighted.into(),
            )
            .unwrap();
        assert!(unique.conflict_found);
        assert!(unique.conflicting_vote_found);
        assert_eq!(unique.conflicting_vote.hash, admitted_storage.hash);

        let owned_conflict = unique.conflicting_vote;
        let owned_bucket = added.bucket;
        votes
            .pbft_service_verified_votes_cleanup_votes_by_period(13)
            .unwrap();
        assert!(votes
            .pbft_service_verified_votes_state_snapshot()
            .unwrap()
            .votes
            .is_empty());
        assert!(!owned_conflict.vote_rlp.is_empty());
        assert_eq!(owned_bucket.votes[0].hash, admitted_storage.hash);
    }

    #[test]
    fn mutation_weighted_payload_mismatch_fails_before_unique_state_changes() {
        let votes = verified_votes_service_for_test(None).unwrap();
        let (vote, weighted) = mutation_vote([0x25; 32], NODE_SECRET, PbftVoteType::Cert, 3, 4);
        let mut mismatched = copy_vote_payload(&vote);
        mismatched.block_hash = [0x26; 32];
        let error = match votes.pbft_service_verified_votes_insert_unique_voter(
            mismatched,
            copy_storage_record(&weighted),
        ) {
            Ok(_) => panic!("weighted payload metadata mismatch must fail before mutation"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("PBFT_VERIFIED_VOTE_WEIGHTED_PAYLOAD_METADATA_MISMATCH"));

        let accepted = votes
            .pbft_service_verified_votes_insert_unique_voter(vote, weighted)
            .unwrap();
        assert!(accepted.accepted);
        assert!(!accepted.duplicate_vote_hash);
    }

    #[test]
    fn bridge_builds_optimized_bundle_from_retained_payloads() {
        let storage = temp_bridge_storage("optimized_bundle_retained_payloads");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let first = generated_vote([0x24; 32], NODE_SECRET);
        let second = generated_vote([0x24; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        votes
            .pbft_service_verified_votes_admit_and_persist(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .pbft_service_verified_votes_admit_and_persist(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let lookup = votes
            .pbft_service_verified_votes_get_two_t_plus_one_voted_block_payloads(
                12,
                2,
                TwoTPlusOneVotedBlockType::CertVotedBlock.into(),
            )
            .expect("2t+1 retained payload lookup succeeds");
        assert!(lookup.found);

        let result = votes
            .pbft_service_verified_votes_build_optimized_votes_bundle_egress(
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
            )
            .unwrap();

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
        let storage = temp_bridge_storage("next_bundle_egress");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let first = generated_vote_for_type([0x25; 32], NODE_SECRET, PbftVoteType::Next, 4);
        let second = generated_vote_for_type([0x25; 32], NODE_SECRET_TWO, PbftVoteType::Next, 4);

        votes
            .pbft_service_verified_votes_admit_and_persist(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .pbft_service_verified_votes_admit_and_persist(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let plan = votes
            .pbft_service_verified_votes_plan_next_votes_bundle_egress(12, 2)
            .unwrap();
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
        let rejected = votes
            .pbft_service_verified_votes_build_optimized_votes_bundle_egress(
                PbftOptimizedVoteBundleBuildRequest {
                    kind,
                    block_hash,
                    period,
                    round,
                    step,
                    vote_hashes: reversed,
                },
            )
            .unwrap();
        assert_eq!(rejected.status, PBFT_OPTIMIZED_BUNDLE_ORDER_MISMATCH);
    }

    #[test]
    fn bridge_selects_reward_vote_payloads_in_requested_order() {
        let storage = temp_bridge_storage("reward_selection_cursor");
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        let first = generated_vote([0x33; 32], NODE_SECRET);
        let second = generated_vote([0x33; 32], NODE_SECRET_TWO);
        let first_hash: [u8; 32] = first.vote_hash.into();
        let second_hash: [u8; 32] = second.vote_hash.into();

        votes
            .pbft_service_verified_votes_admit_and_persist(
                &first.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("first generated vote is admitted");
        votes
            .pbft_service_verified_votes_admit_and_persist(
                &second.vote_rlp,
                validation_facts(),
                runtime_flags(),
                runtime_context(80),
            )
            .expect("second generated vote is admitted");

        let applied = votes
            .pbft_service_verified_votes_apply_reward_votes_reset(FfiPbftRewardVotesResetRequest {
                period: 12,
                round: 2,
                step: 3,
                block_hash: [0x33; 32],
                sync: false,
            })
            .unwrap();
        let committed = votes
            .pbft_service_verified_votes_commit_reward_vote_cursor(
                &reward_reset_intent([0x33; 32]),
                applied.reward_votes_reset_generation,
            )
            .unwrap();
        assert_eq!(committed.status, 0);
        assert_eq!(
            votes
                .pbft_service_verified_votes_reward_vote_period()
                .unwrap(),
            12
        );
        let cursor = votes
            .pbft_service_verified_votes_reward_vote_cursor()
            .unwrap();
        assert!(cursor.found);
        assert_eq!(cursor.round, 2);
        assert_eq!(cursor.step, 3);
        let reward_snapshot = votes
            .pbft_service_verified_votes_current_reward_snapshot()
            .unwrap();
        assert_eq!(reward_snapshot.cursor.period, cursor.period);
        assert_eq!(reward_snapshot.cursor.round, cursor.round);
        assert_eq!(reward_snapshot.cursor.step, cursor.step);
        assert_eq!(reward_snapshot.cursor.block_hash, cursor.block_hash);
        assert_eq!(reward_snapshot.records.len(), 2);
        let repeated = votes
            .pbft_service_verified_votes_commit_reward_vote_cursor(
                &reward_reset_intent([0x33; 32]),
                applied.reward_votes_reset_generation,
            )
            .unwrap();
        assert_eq!(repeated.status, 1);

        let selection = votes
            .pbft_service_verified_votes_select_reward_vote_payloads(
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

        let rejected = runtime_outcome_to_ffi(
            validation.clone(),
            PbftVoteAdmissionTransactionResult {
                outcome: outcome.clone(),
                persistence_required: true,
                persistence_status: PbftVoteAdmissionPersistenceStatus::Rejected,
                persistence_applied_writes: 0,
                transition_published: false,
                persistence_error_code: "PBFT_VOTE_PERSIST_STORAGE_OR_LOCK_FAILURE".to_owned(),
            },
            FfiPbftVoteProgressContext {
                current_period: 3,
                current_round: 2,
                max_future_period_delta: 1,
                has_two_t_plus_one_threshold: true,
                two_t_plus_one_threshold: 5,
                require_proposed_block_sidecar: false,
                slashing_enabled: true,
            },
        );
        assert!(!rejected.accepted);
        assert!(rejected.rejected);
        assert!(!rejected.transition_published);
        assert_eq!(
            rejected.persistence_status,
            PbftVoteAdmissionPersistenceStatus::Rejected.as_u8()
        );
        assert!(!rejected.replay_inserted);
        assert!(!rejected.has_verified_vote_add);
        assert!(!rejected.mark_vote_known);
        assert!(!rejected.request_proposed_block_sidecar);
        assert!(!rejected.gossip_vote);
        assert!(!rejected.report_slashing);
        assert!(!rejected.drive_pbft_progress);
        assert!(rejected.slashing_incoming_vote.vote_rlp.is_empty());
        assert!(rejected.slashing_conflicting_vote.vote_rlp.is_empty());

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
            context,
        );

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
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
        for secret in [NODE_SECRET, NODE_SECRET_TWO] {
            let vote = generated_vote([0x35; 32], secret);
            votes
                .pbft_service_verified_votes_admit_and_persist(
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
            .pbft_service_verified_votes_prepare_reward_votes_reset_stage(&reward_reset_intent(
                [0x35; 32],
            ))
            .unwrap();
        assert!(stage.has_reward_votes_reset);
        assert!(!stage.reward_votes_bundle_rlp.is_empty());
        assert!(votes
            .pbft_service_verified_votes_prepare_reward_votes_reset_stage(&reward_reset_intent(
                [0x36; 32]
            ))
            .is_err());

        let result = votes
            .pbft_service_verified_votes_apply_reward_votes_reset(FfiPbftRewardVotesResetRequest {
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
        let votes = verified_votes_service_for_test(Some(&storage)).unwrap();
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
            .with_verified_votes(|runtime| {
                runtime.verified_votes_apply_pbft_finalization_storage_writes(
                    &write_intent,
                    vec![primary_stage],
                    false,
                )
            })
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
