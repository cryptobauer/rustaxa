//! CXX bridge wrappers for deterministic pillar-vote aggregation.
//!
//! The bridge accepts plain C++-style vote payloads and converts them into
//! `rustaxa_consensus::VerifiedPillarVote` domain values for stateful
//! aggregation. Production admission and weighted-bundle calls compose
//! canonical inspection, pillar state, and borrowed Rust FinalChain DPoS reads
//! inside the service; the lower-level fact-supplied planners are Rust-only
//! test seams rather than CXX contracts.
//!
//! Pillar-vote inspection delegates byte-level signature recovery to
//! `rustaxa-types`; this layer exposes the CXX-compatible boundary and
//! enforces local bridge-domain invariants before delegating aggregation rules
//! to [`PillarVotes`].

#[cfg(test)]
use crate::ffi::rustaxa_ffi::PillarVoteRelevanceFact as FfiPillarVoteRelevanceFact;
use crate::ffi::rustaxa_ffi::{
    PillarBlockFinalizationAcknowledgeRequest, PillarBlockFinalizationAcknowledgeResult,
    PillarBlockFinalizationPrepareResult, PillarBlockFinalizationRequest,
    PillarConsensusThresholdLookup, PillarVoteBundleHash, PillarVoteBundleWithFinalChainPlan,
    PillarVoteInspection, PillarVoteNetworkBundleChunk, PillarVoteNetworkBundleLookup,
    PillarVoteRecord, PillarVoteRelevancePlan as FfiPillarVoteRelevancePlan, PillarVoteRlpPayload,
    PillarVoteRuntimeRelevanceContext, PillarVoteSingleAdmissionContext,
    PillarVoteSingleAdmissionPreparePlan as FfiPillarVoteSingleAdmissionPreparePlan,
    PillarVoteSingleAdmissionWithFinalChainPlan, PillarVotesPayloadLookup,
};
use crate::ffi::{BridgeFinalChain, BridgePbftService, PillarChainState, SingleVotePreparation};
use anyhow::{anyhow, ensure, Result};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_consensus::{
    inspect_pillar_vote_from_rlp, PillarVoteBundlePlanner,
    PillarVoteFact as ConsensusPillarVoteFact, PillarVoteIdentity as ConsensusPillarVoteIdentity,
    PillarVoteInspection as ConsensusPillarVoteInspection,
    PillarVoteRelevanceFact as ConsensusPillarVoteRelevanceFact,
    PillarVoteRelevancePlan as ConsensusPillarVoteRelevancePlan, PillarVotes, VerifiedPillarVote,
};
use rustaxa_consensus::{
    plan_pillar_block_finalization, PillarBlockFinalizationFact, PillarBlockFinalizationStatus,
};
use rustaxa_storage::Storage;
#[cfg(test)]
use rustaxa_types::CurrentPillarBlockDataDb;
use rustaxa_types::{
    decode_optimized_pillar_votes_bundle_rlp, encode_optimized_pillar_votes_bundle_rlp,
    PillarBlock, PillarVote,
};
use std::collections::HashMap;

const PILLAR_VOTE_BUNDLE_STATUS_VALID: u8 = 0;
const PILLAR_VOTE_BUNDLE_STATUS_EMPTY: u8 = 1;
const PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED: u8 = 4;
const PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT: u8 = 5;
const PILLAR_VOTE_BUNDLE_STATUS_STALE_ANCHOR: u8 = 9;
const PILLAR_VOTE_STATUS_VALID: u8 = 0;
const PILLAR_VOTE_STATUS_NOT_UNIQUE: u8 = 5;
const PILLAR_VOTE_STATUS_SIGNATURE_INVALID: u8 = 6;
const PILLAR_VOTE_STATUS_NOT_ELIGIBLE: u8 = 7;
const PILLAR_VOTE_STATUS_FUTURE_PERIOD: u8 = 8;
const PILLAR_VOTE_STATUS_INSPECTION_FAILURE: u8 = 9;
const PILLAR_VOTE_STATUS_STALE_ANCHOR: u8 = 10;
const PILLAR_VOTE_STATUS_MISSING_PREPARATION: u8 = 11;
/// Maximum unresolved single-vote preparations retained for one anchor generation.
///
/// Once full, insertion deterministically evicts the lowest canonical vote
/// hash. Core apply of an evicted token returns missing-preparation status; the
/// caller boundary must preserve external-vs-trusted provenance when deciding
/// whether a new preparation is permitted.
const MAX_SINGLE_VOTE_PREPARATIONS: usize = 4_096;
const MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS: usize = 16;
const PILLAR_VOTE_STATUS_UNKNOWN: u8 = 255;

/// Rust-private weighted bytes used between FinalChain composition and bundle apply.
pub(crate) struct PillarVoteWeightedRlpPayload {
    vote_rlp: Vec<u8>,
    weight: u64,
}

/// Rust-private facts used to consume one retained single-vote preparation.
pub(crate) struct PillarVoteSingleAdmissionApplyInput {
    vote_hash: [u8; 32],
    validator_vote_count: u64,
    has_threshold: bool,
    threshold: u64,
}

/// Rust-private result of applying one retained single-vote preparation.
pub(crate) struct PillarVoteSingleAdmissionApplyPlan {
    status: u8,
    accepted: bool,
    duplicate: bool,
    conflict_found: bool,
    conflicting_vote_hash: [u8; 32],
    block_weight: u64,
}

/// Rust-private generation-bound preparation retained across unlocked FinalChain reads.
pub(crate) struct PillarVoteSingleAdmissionPreparePlan {
    status: u8,
    can_query_dpos: bool,
    needs_threshold: bool,
    period: u64,
    block_hash: [u8; 32],
    vote_hash: [u8; 32],
    voter: [u8; 20],
    anchor_generation: u64,
    has_current_anchor: bool,
    current_period: u64,
    current_hash: [u8; 32],
}

/// Rust-private generation-bound result of inspecting a synced vote bundle.
pub(crate) struct PillarVoteWeightedBundlePreparePlan {
    status: u8,
    can_query_dpos: bool,
    inspections: Vec<PillarVoteInspection>,
    first_bad_vote_hash: [u8; 32],
    expected_block_hash: [u8; 32],
    anchor_generation: u64,
    has_current_anchor: bool,
    current_period: u64,
    current_hash: [u8; 32],
}

/// Rust-private input for generation-bound synced-bundle mutation.
pub(crate) struct PillarVoteWeightedBundleApplyInput {
    votes: Vec<PillarVoteWeightedRlpPayload>,
    required_votes_period: u64,
    threshold: u64,
    anchor_generation: u64,
}

/// Rust-private result of applying a weighted synced bundle.
pub(crate) struct PillarVoteBundleApplyPlan {
    status: u8,
    block_weight: u64,
    selected_weight: u64,
    first_bad_vote_hash: [u8; 32],
    insert_failed: bool,
    insert_failed_vote_hash: [u8; 32],
    applied_votes: u64,
}

#[cfg(test)]
struct PillarVotesTestFixture(PillarVotes, HashMap<H256, SingleVotePreparation>);

struct WeightedRlpBundlePlanWork {
    plan: WeightedRlpBundlePlan,
    votes_by_hash: HashMap<H256, VerifiedPillarVote>,
}

struct WeightedRlpBundlePlan {
    status: u8,
    accepted_vote_hashes: Vec<H256>,
    block_weight: u64,
    selected_weight: u64,
    first_bad_vote_hash: [u8; 32],
}

/// Internal batch-inspection result used by generation-bound bundle prepare.
///
/// This is intentionally not a standalone CXX API: callers must prepare
/// through the PBFT-service-owned pillar state so inspection is bound to an anchor
/// generation.
struct PillarVoteBundleInspectionPlan {
    status: u8,
    inspections: Vec<PillarVoteInspection>,
    first_bad_vote_hash: [u8; 32],
}

/// Creates an empty Rust pillar-vote registry for bridge-module tests.
#[cfg(test)]
fn create_pillar_votes_index() -> Box<PillarVotesTestFixture> {
    Box::new(PillarVotesTestFixture(PillarVotes::new(), HashMap::new()))
}

#[cfg(test)]
impl PillarVotesTestFixture {
    /// Validates and applies one weighted synced pillar-vote bundle.
    ///
    /// Inputs:
    /// - `votes` are canonical vote RLP payloads paired with externally
    ///   supplied FinalChain DPoS weights.
    /// - `expected_period`, `expected_block_hash`, and `threshold` are the
    ///   deterministic bundle constraints.
    ///
    /// Outputs:
    /// - Returns the weighted bundle status and aggregate weights.
    /// - When status is valid, selected votes are inserted into Rust-owned
    ///   aggregation state and `applied_votes` reports how many selected
    ///   records were accepted or already present.
    ///
    /// Invariants and edge behavior:
    /// - This method never reads FinalChain and never materializes C++ votes.
    /// - Period threshold state is initialized before insertion; existing
    ///   period state is left unchanged.
    /// - Exact duplicate selected votes are treated as successful idempotent
    ///   applies. Same-voter conflicts or insertion errors return
    ///   `insert_failed` with the offending vote hash instead of mutating
    ///   C++-owned state.
    pub fn pillar_votes_apply_weighted_rlp_bundle(
        &mut self,
        votes: Vec<PillarVoteWeightedRlpPayload>,
        expected_period: u64,
        expected_block_hash: &[u8; 32],
        threshold: u64,
    ) -> Result<PillarVoteBundleApplyPlan> {
        let work =
            plan_weighted_rlp_bundle(votes, expected_period, expected_block_hash, threshold)?;
        if work.plan.status != PILLAR_VOTE_BUNDLE_STATUS_VALID {
            return Ok(PillarVoteBundleApplyPlan {
                status: work.plan.status,
                block_weight: work.plan.block_weight,
                selected_weight: work.plan.selected_weight,
                first_bad_vote_hash: work.plan.first_bad_vote_hash,
                insert_failed: false,
                insert_failed_vote_hash: [0; 32],
                applied_votes: 0,
            });
        }

        self.0.initialize_period_data(expected_period, threshold);
        let mut applied_votes = 0u64;
        for accepted_vote_hash in &work.plan.accepted_vote_hashes {
            let vote_hash = *accepted_vote_hash;
            let Some(vote) = work.votes_by_hash.get(&vote_hash).cloned() else {
                return Ok(PillarVoteBundleApplyPlan {
                    status: work.plan.status,
                    block_weight: work.plan.block_weight,
                    selected_weight: work.plan.selected_weight,
                    first_bad_vote_hash: work.plan.first_bad_vote_hash,
                    insert_failed: true,
                    insert_failed_vote_hash: accepted_vote_hash.0,
                    applied_votes,
                });
            };

            match self.0.add_verified_vote(vote) {
                Ok(outcome) if outcome.accepted || outcome.duplicate => {
                    applied_votes = applied_votes.saturating_add(1);
                }
                Ok(_) | Err(_) => {
                    return Ok(PillarVoteBundleApplyPlan {
                        status: work.plan.status,
                        block_weight: work.plan.block_weight,
                        selected_weight: work.plan.selected_weight,
                        first_bad_vote_hash: work.plan.first_bad_vote_hash,
                        insert_failed: true,
                        insert_failed_vote_hash: accepted_vote_hash.0,
                        applied_votes,
                    });
                }
            }
        }

        Ok(PillarVoteBundleApplyPlan {
            status: work.plan.status,
            block_weight: work.plan.block_weight,
            selected_weight: work.plan.selected_weight,
            first_bad_vote_hash: work.plan.first_bad_vote_hash,
            insert_failed: false,
            insert_failed_vote_hash: [0; 32],
            applied_votes,
        })
    }

    /// Looks up Rust-retained pillar vote payloads for C++ edge materialization.
    ///
    /// This keeps deterministic selection in Rust while avoiding dependency on
    /// live C++ `PillarVote` sidecars for returned vote objects.
    pub fn pillar_votes_get_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> PillarVotesPayloadLookup {
        self.0
            .get_verified_votes(period, H256::from(*block_hash), above_threshold)
            .into()
    }

    /// Removes all pillar-vote state for periods lower than `min_period`.
    pub fn pillar_votes_cleanup_votes_by_period(&mut self, min_period: u64) {
        self.0.erase_votes(min_period);
    }
}

impl PillarChainState {
    /// Prepares one pillar vote for admission through the runtime-owned
    /// pillar-vote index.
    pub fn pbft_service_pillar_prepare_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        let retained_vote_rlp = vote_rlp.clone();
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let plan = prepare_single_vote_admission(
            &self.votes,
            snapshot.anchor,
            snapshot.generation,
            vote_rlp,
            Some(&context),
        )?;
        if plan.can_query_dpos {
            self.retain_single_vote_preparation(
                H256::from(plan.vote_hash),
                SingleVotePreparation {
                    vote_rlp: retained_vote_rlp,
                    anchor_generation: snapshot.generation,
                    period: plan.period,
                    block_hash: H256::from(plan.block_hash),
                    voter: ethereum_types::H160::from(plan.voter),
                    needs_threshold: plan.needs_threshold,
                    current_anchor: snapshot.anchor,
                    first_pillar_block_period: context.first_pillar_block_period,
                    pillar_blocks_interval: context.pillar_blocks_interval,
                    trusted_local_or_restore: false,
                },
            )?;
        }
        Ok(plan)
    }

    /// Prepares a locally generated or restart-restored vote for trusted apply.
    ///
    /// This route still validates canonical bytes and signature, and binds the
    /// one-time preparation to the current anchor generation. It intentionally
    /// skips network relevance and identity uniqueness because those votes were
    /// created locally or already accepted before persistence. External network
    /// admission must use `pbft_service_pillar_prepare_single_vote_admission`.
    pub fn pbft_service_pillar_prepare_trusted_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        let retained_vote_rlp = vote_rlp.clone();
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        if let Ok(inspection) = inspect_pillar_vote_from_rlp(&vote_rlp) {
            let registry = self
                .single_vote_preparations
                .lock()
                .map_err(|_| anyhow!("single pillar vote preparation lock poisoned"))?;
            if let Some(existing) = registry.entries.get(&inspection.vote_hash) {
                return Ok(preparation_plan(existing));
            }
        }
        let plan = prepare_single_vote_admission(
            &self.votes,
            snapshot.anchor,
            snapshot.generation,
            vote_rlp,
            None,
        )?;
        if plan.can_query_dpos {
            self.retain_single_vote_preparation(
                H256::from(plan.vote_hash),
                SingleVotePreparation {
                    vote_rlp: retained_vote_rlp,
                    anchor_generation: snapshot.generation,
                    period: plan.period,
                    block_hash: H256::from(plan.block_hash),
                    voter: ethereum_types::H160::from(plan.voter),
                    needs_threshold: plan.needs_threshold,
                    current_anchor: snapshot.anchor,
                    first_pillar_block_period: 0,
                    pillar_blocks_interval: 0,
                    trusted_local_or_restore: true,
                },
            )?;
        }
        Ok(plan)
    }

    /// Evaluates one canonical pillar vote against runtime-owned vote state.
    ///
    /// Inputs:
    /// - `vote_rlp` is the canonical C++ vote payload.
    /// - `context` carries immutable pillar scheduling configuration; current
    ///   anchor facts come from the runtime snapshot.
    ///
    /// Outputs:
    /// - Returns the same stable relevance status DTO used by the network facade.
    /// - Duplicate detection is derived from the runtime-owned Rust vote index.
    ///
    /// Invariants and edge behavior:
    /// - This method does not call FinalChain, request network data, emit events,
    ///   or materialize C++ `PillarVote` objects.
    /// - Malformed vote bytes map to the existing unknown relevance status.
    pub fn pbft_service_pillar_plan_vote_relevance(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteRuntimeRelevanceContext,
    ) -> Result<FfiPillarVoteRelevancePlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        runtime_plan_vote_relevance(&self.votes, snapshot.anchor, vote_rlp, context)
    }

    /// Applies one prepared pillar vote to the runtime-owned pillar-vote index.
    ///
    /// The canonical vote hash consumes exactly one retained preparation. A
    /// checked external preparation reruns relevance and identity validation
    /// under the same anchor read lock used for mutation. Trusted local/restart
    /// preparations skip only those two already-established checks.
    pub fn pbft_service_pillar_apply_prepared_single_vote_admission(
        &mut self,
        input: PillarVoteSingleAdmissionApplyInput,
    ) -> Result<PillarVoteSingleAdmissionApplyPlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let vote_hash = H256::from(input.vote_hash);
        let preparation = self
            .single_vote_preparations
            .lock()
            .map_err(|_| anyhow!("single pillar vote preparation lock poisoned"))?
            .entries
            .remove(&vote_hash);
        let Some(preparation) = preparation else {
            return Ok(single_admission_apply_plan(
                PILLAR_VOTE_STATUS_MISSING_PREPARATION,
            ));
        };
        if snapshot.generation != preparation.anchor_generation {
            return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_STALE_ANCHOR));
        }
        if !preparation.trusted_local_or_restore {
            let context = PillarVoteSingleAdmissionContext {
                first_pillar_block_period: preparation.first_pillar_block_period,
                pillar_blocks_interval: preparation.pillar_blocks_interval,
            };
            let revalidated = prepare_single_vote_admission(
                &self.votes,
                snapshot.anchor,
                snapshot.generation,
                preparation.vote_rlp.clone(),
                Some(&context),
            )?;
            if !revalidated.can_query_dpos || H256::from(revalidated.vote_hash) != vote_hash {
                return Ok(single_admission_apply_plan(revalidated.status));
            }
        }
        // Keep the read guard through mutation so a current-block writer cannot
        // publish a new generation between validation and insertion.
        apply_prepared_single_vote_admission(&mut self.votes, preparation.vote_rlp, input)
    }

    fn retain_single_vote_preparation(
        &self,
        vote_hash: H256,
        preparation: SingleVotePreparation,
    ) -> Result<()> {
        let mut registry = self
            .single_vote_preparations
            .lock()
            .map_err(|_| anyhow!("single pillar vote preparation lock poisoned"))?;
        registry
            .entries
            .retain(|_, retained| retained.anchor_generation == preparation.anchor_generation);
        if !registry.entries.contains_key(&vote_hash)
            && registry.entries.len() >= MAX_SINGLE_VOTE_PREPARATIONS
        {
            registry.entries.pop_first();
        }
        registry.entries.insert(vote_hash, preparation);
        Ok(())
    }

    /// Removes only the preparation identified by both vote hash and anchor generation.
    fn discard_single_vote_preparation(
        &self,
        vote_hash: H256,
        anchor_generation: u64,
    ) -> Result<()> {
        let mut registry = self
            .single_vote_preparations
            .lock()
            .map_err(|_| anyhow!("single pillar vote preparation lock poisoned"))?;
        if registry
            .entries
            .get(&vote_hash)
            .is_some_and(|entry| entry.anchor_generation == anchor_generation)
        {
            registry.entries.remove(&vote_hash);
        }
        Ok(())
    }

    /// Prepares one synced vote bundle before the service borrows FinalChain.
    ///
    /// Rust inspects all canonical vote bytes and binds the returned expected
    /// hash to the current anchor generation. The composed service supplies
    /// DPoS weights after this method reports `can_query_dpos`; this low-level
    /// planner is not a CXX contract.
    pub fn pbft_service_pillar_prepare_weighted_rlp_bundle(
        &self,
        vote_rlps: Vec<PillarVoteRlpPayload>,
        required_votes_period: u64,
    ) -> Result<PillarVoteWeightedBundlePreparePlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        prepare_weighted_rlp_bundle(
            snapshot.anchor,
            snapshot.generation,
            vote_rlps,
            required_votes_period,
        )
    }

    /// Applies one generation-bound weighted RLP bundle to the vote index.
    ///
    /// The current anchor generation, required period, and expected hash are
    /// revalidated before any period initialization or vote insertion.
    pub fn pbft_service_pillar_apply_weighted_rlp_bundle(
        &mut self,
        input: PillarVoteWeightedBundleApplyInput,
    ) -> Result<PillarVoteBundleApplyPlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let Some(anchor) = snapshot.anchor else {
            return Ok(bundle_apply_rejection(
                PILLAR_VOTE_BUNDLE_STATUS_STALE_ANCHOR,
            ));
        };
        if snapshot.generation != input.anchor_generation
            || anchor.period.checked_add(1) != Some(input.required_votes_period)
        {
            return Ok(bundle_apply_rejection(
                PILLAR_VOTE_BUNDLE_STATUS_STALE_ANCHOR,
            ));
        }
        let expected_block_hash: [u8; 32] = anchor.hash.into();
        // Keep the read guard through every mutation for the same generation
        // binding guaranteed by single-vote admission.
        apply_weighted_rlp_bundle(
            &mut self.votes,
            input.votes,
            input.required_votes_period,
            &expected_block_hash,
            input.threshold,
        )
    }

    /// Looks up pillar-vote payloads through the runtime-owned index.
    ///
    /// Inputs:
    /// - `period` and `block_hash` identify the requested pillar-vote set.
    /// - `above_threshold` preserves the live runtime selection contract for
    ///   callers that only want already-thresholded votes.
    ///
    /// Outputs:
    /// - Returns live runtime vote payloads when retained.
    /// - Falls back to the stored `PeriodData` pillar-vote bundle after restart
    ///   when the live runtime has no retained votes for the request.
    ///
    /// Invariants and edge behavior:
    /// - Storage fallback verifies the embedded period and block hash before
    ///   returning payloads so C++ does not decode unrelated period sidecars.
    /// - Stored period data does not preserve the original vote weights at this
    ///   boundary; fallback records therefore carry zero weight while preserving
    ///   canonical vote bytes and hashes for temporary C++ materialization.
    pub fn pbft_service_pillar_get_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> Result<PillarVotesPayloadLookup> {
        let requested_hash = H256::from(*block_hash);
        let runtime_lookup = self
            .votes
            .get_verified_votes(period, requested_hash, above_threshold);
        if !runtime_lookup.votes.is_empty()
            || runtime_lookup.threshold_met
            || runtime_lookup.block_weight > 0
            || runtime_lookup.selected_weight > 0
        {
            return Ok(runtime_lookup.into());
        }

        let stored_votes =
            load_stored_period_pillar_votes(self.storage.as_ref(), period, requested_hash)?;
        Ok(stored_votes_to_payload_lookup(stored_votes))
    }

    /// Builds packet-ready pillar-vote bundle chunks for network serving.
    ///
    /// Inputs:
    /// - `period` and `block_hash` identify the requested pillar-vote set.
    /// - `max_votes_per_bundle` is the tarcap packet limit supplied by C++.
    ///
    /// Outputs:
    /// - Returns optimized pillar-vote bundle RLP chunks plus the vote hashes
    ///   included in each chunk, in the same order as the signatures.
    /// - Uses live runtime-owned votes first. If none are retained after
    ///   restart, falls back to the stored `PeriodData` pillar-vote bundle.
    ///
    /// Invariants and edge behavior:
    /// - The returned `votes_bundle_rlp` is the inner
    ///   `OptimizedPillarVotesBundle` payload, not the tarcap packet wrapper.
    /// - Empty lookups return an empty chunk list.
    /// - Storage fallback verifies the embedded period and block hash before
    ///   returning chunks, so network serving cannot answer a request with a
    ///   different finalized pillar-vote bundle.
    pub fn pbft_service_pillar_build_verified_vote_network_bundles(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        max_votes_per_bundle: usize,
    ) -> Result<PillarVoteNetworkBundleLookup> {
        ensure!(
            max_votes_per_bundle > 0,
            "pillar vote network bundle chunk size must be non-zero"
        );

        let requested_hash = H256::from(*block_hash);
        let runtime_lookup = self.votes.get_verified_votes(period, requested_hash, false);
        if !runtime_lookup.votes.is_empty() {
            let votes = runtime_lookup
                .votes
                .into_iter()
                .map(|vote| (vote.vote, vote.vote_hash))
                .collect::<Vec<_>>();
            return Ok(PillarVoteNetworkBundleLookup {
                from_storage: false,
                chunks: build_network_bundle_chunks(votes, max_votes_per_bundle)?,
            });
        }

        let stored_votes =
            load_stored_period_pillar_votes(self.storage.as_ref(), period, requested_hash)?;
        Ok(PillarVoteNetworkBundleLookup {
            from_storage: true,
            chunks: build_network_bundle_chunks(stored_votes, max_votes_per_bundle)?,
        })
    }

    /// Prepares one pillar block for PBFT finalization.
    ///
    /// Inputs:
    /// - `request` carries only the requested hash. Current identity, canonical
    ///   block RLP, and latest-finalized identity come from the runtime
    ///   snapshot.
    ///
    /// Outputs:
    /// - Returns the deterministic pillar-finalization status plus selected
    ///   vote payloads when the ready path succeeds.
    /// - Returns a generation-bound one-time preparation token and the canonical
    ///   pillar block payload for the PBFT primary storage stage.
    ///
    /// Edge behavior:
    /// - `AlreadyFinalized` is checked before any live-vote lookup, so a
    ///   cleaned vote state cannot force duplicate finalization into
    ///   `MissingVotes`.
    /// - No storage is mutated in this call.
    /// - C++ still owns network requests, legacy `PillarVote` materialization,
    ///   event emission, and PBFT `PeriodData` payload assembly.
    pub fn pbft_service_pillar_prepare_finalized_block_for_pbft(
        &mut self,
        request: PillarBlockFinalizationRequest,
    ) -> Result<PillarBlockFinalizationPrepareResult> {
        let requested_hash = H256::from(request.requested_pillar_block_hash);
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let current_anchor = snapshot.anchor;
        let current_block_rlp = snapshot.current_block_rlp.clone();
        let current_period = current_anchor
            .map(|anchor| anchor.period)
            .unwrap_or_default();
        let current_hash = current_anchor.map(|anchor| anchor.hash).unwrap_or_default();
        let lookup = if snapshot
            .latest_finalized_block
            .as_ref()
            .is_some_and(|block| block.hash() == requested_hash)
        {
            rustaxa_consensus::PillarVotesLookup {
                threshold_met: false,
                block_weight: 0,
                selected_weight: 0,
                votes: Vec::new(),
            }
        } else if current_anchor.is_some() && current_hash == requested_hash {
            if let Some(vote_period) = current_period.checked_add(1) {
                self.votes
                    .get_verified_votes(vote_period, requested_hash, true)
            } else {
                rustaxa_consensus::PillarVotesLookup {
                    threshold_met: false,
                    block_weight: 0,
                    selected_weight: 0,
                    votes: Vec::new(),
                }
            }
        } else {
            rustaxa_consensus::PillarVotesLookup {
                threshold_met: false,
                block_weight: 0,
                selected_weight: 0,
                votes: Vec::new(),
            }
        };

        let selected_vote_count = lookup.votes.len() as u64;
        let plan = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested_hash,
            has_current_pillar_block: current_anchor.is_some(),
            current_period,
            current_hash,
            threshold_met: lookup.threshold_met,
            block_weight: lookup.block_weight,
            selected_weight: lookup.selected_weight,
            selected_vote_count,
            has_last_finalized_pillar_block: snapshot.latest_finalized_block.is_some(),
            last_finalized_hash: snapshot
                .latest_finalized_block
                .as_ref()
                .map(PillarBlock::hash)
                .unwrap_or_default(),
        });

        let votes = if plan.return_votes {
            lookup
                .votes
                .into_iter()
                .map(PillarVoteRecord::from)
                .collect()
        } else {
            Vec::new()
        };
        let success = plan.return_votes && !votes.is_empty();

        if plan.status == PillarBlockFinalizationStatus::Ready && plan.should_persist {
            let preparation_anchor_generation = snapshot.generation;
            let preparation_token = {
                let mut preparations = self
                    .pillar_block_finalization_preparations
                    .lock()
                    .map_err(|_| anyhow!("pillar block finalization preparation lock poisoned"))?;
                preparations
                    .retain(|_, prepared| prepared.anchor_generation == snapshot.generation);
                if let Some((token, _)) = preparations.iter().find(|(_, prepared)| {
                    prepared.anchor_generation == preparation_anchor_generation
                        && prepared.prepared_pillar_block_period == plan.current_period
                        && prepared.prepared_pillar_block_rlp == current_block_rlp
                }) {
                    *token
                } else {
                    if preparations.len() >= MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS {
                        let oldest_token = preparations.keys().min().copied().ok_or_else(|| {
                            anyhow!("PILLAR_BLOCK_FINALIZATION_PREPARATION_CAP_EMPTY")
                        })?;
                        preparations.remove(&oldest_token);
                    }
                    let preparation_token = self
                        .next_pillar_block_finalization_preparation_token
                        .checked_add(1)
                        .ok_or_else(|| {
                            anyhow!("PILLAR_BLOCK_FINALIZATION_TOKEN_SEQUENCE_OVERFLOW")
                        })?;
                    self.next_pillar_block_finalization_preparation_token = preparation_token;
                    preparations.insert(
                        preparation_token,
                        crate::ffi::PillarBlockFinalizationPreparation {
                            anchor_generation: preparation_anchor_generation,
                            prepared_pillar_block_period: plan.current_period,
                            prepared_pillar_block_rlp: current_block_rlp.clone(),
                            matching_vote_cleanup_min_period: plan
                                .current_period
                                .checked_add(1)
                                .unwrap_or(0),
                            should_emit: plan.should_emit,
                        },
                    );
                    preparation_token
                }
            };

            return Ok(PillarBlockFinalizationPrepareResult {
                status: plan.status.as_u8(),
                success,
                should_request_votes: plan.should_request_votes,
                has_request_votes_period: plan.request_votes_period.is_some(),
                request_votes_period: plan.request_votes_period.unwrap_or_default(),
                should_emit: plan.should_emit,
                current_period: plan.current_period,
                current_hash: current_hash.0,
                block_weight: plan.block_weight,
                selected_weight: plan.selected_weight,
                selected_vote_count: plan.selected_vote_count,
                prepared_pillar_block_period: plan.current_period,
                prepared_pillar_block_rlp: current_block_rlp,
                has_prepared_pillar_block: true,
                preparation_anchor_generation,
                preparation_token,
                votes,
            });
        }

        Ok(PillarBlockFinalizationPrepareResult {
            status: plan.status.as_u8(),
            success,
            should_request_votes: plan.should_request_votes,
            has_request_votes_period: plan.request_votes_period.is_some(),
            request_votes_period: plan.request_votes_period.unwrap_or_default(),
            should_emit: plan.should_emit,
            current_period: plan.current_period,
            current_hash: current_hash.into(),
            block_weight: plan.block_weight,
            selected_weight: plan.selected_weight,
            selected_vote_count: plan.selected_vote_count,
            prepared_pillar_block_period: 0,
            prepared_pillar_block_rlp: Vec::new(),
            has_prepared_pillar_block: false,
            preparation_anchor_generation: snapshot.generation,
            preparation_token: 0,
            votes,
        })
    }

    /// Acknowledges one prepared pillar-block finalization.
    ///
    /// Inputs:
    /// - `request` binds a one-time preparation token to the generation observed
    ///   during prepare.
    /// - Snapshot generation and token must match to be eligible for acknowledgement.
    ///
    /// Outputs:
    /// - Mirrors the latest finalized pillar identity into the runtime snapshot.
    /// - Cleans matching in-memory vote state.
    /// - Returns whether compatibility event emission should run.
    pub fn pbft_service_pillar_ack_finalize_block_for_pbft(
        &mut self,
        request: PillarBlockFinalizationAcknowledgeRequest,
    ) -> Result<PillarBlockFinalizationAcknowledgeResult> {
        let (
            preparation_anchor_generation,
            prepared_pillar_block_period,
            prepared_pillar_block_rlp,
            matching_vote_cleanup_min_period,
            should_emit,
        ) = {
            let snapshot = self
                .current_anchor
                .read()
                .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
            ensure!(
                snapshot.generation == request.anchor_generation,
                "PILLAR_BLOCK_FINALIZATION_ACK_STALE_GENERATION"
            );

            let preparations = self
                .pillar_block_finalization_preparations
                .lock()
                .map_err(|_| anyhow!("pillar block finalization preparation lock poisoned"))?;
            let prepared = preparations
                .get(&request.preparation_token)
                .ok_or_else(|| anyhow!("PILLAR_BLOCK_FINALIZATION_ACK_TOKEN_REUSED"))?;
            ensure!(
                prepared.anchor_generation == request.anchor_generation,
                "PILLAR_BLOCK_FINALIZATION_ACK_MISMATCHED_GENERATION"
            );
            (
                prepared.anchor_generation,
                prepared.prepared_pillar_block_period,
                prepared.prepared_pillar_block_rlp.clone(),
                prepared.matching_vote_cleanup_min_period,
                prepared.should_emit,
            )
        };

        let persisted_block = self
            .storage
            .pillar()
            .rlp(prepared_pillar_block_period)?
            .ok_or_else(|| anyhow!("PILLAR_BLOCK_FINALIZATION_PREPARED_BLOCK_NOT_PERSISTENT"))?;
        ensure!(
            persisted_block == prepared_pillar_block_rlp,
            "PILLAR_BLOCK_FINALIZATION_PREPARED_BLOCK_MISMATCH"
        );

        let finalized_block = PillarBlock::decode_rlp(&prepared_pillar_block_rlp)?;
        let finalized_hash = finalized_block.hash();
        ensure!(
            finalized_block.encode_rlp() == prepared_pillar_block_rlp,
            "PILLAR_BLOCK_FINALIZATION_PREPARED_BLOCK_NON_CANONICAL"
        );

        {
            let mut snapshot = self
                .current_anchor
                .write()
                .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
            ensure!(
                snapshot.generation == request.anchor_generation,
                "PILLAR_BLOCK_FINALIZATION_ACK_STALE_GENERATION"
            );
            snapshot.latest_finalized_block = Some(finalized_block);
            snapshot.latest_finalized_block_rlp = prepared_pillar_block_rlp.clone();
        }

        {
            let mut preparations = self
                .pillar_block_finalization_preparations
                .lock()
                .map_err(|_| anyhow!("pillar block finalization preparation lock poisoned"))?;
            let prepared = preparations
                .remove(&request.preparation_token)
                .ok_or_else(|| anyhow!("PILLAR_BLOCK_FINALIZATION_ACK_TOKEN_REUSED"))?;
            ensure!(
                prepared.anchor_generation == preparation_anchor_generation,
                "PILLAR_BLOCK_FINALIZATION_ACK_MISMATCHED_GENERATION"
            );
        }

        self.votes.erase_votes(matching_vote_cleanup_min_period);

        Ok(PillarBlockFinalizationAcknowledgeResult {
            should_emit,
            latest_finalized_period: prepared_pillar_block_period,
            latest_finalized_hash: finalized_hash.0,
        })
    }
}

#[allow(dead_code)]
impl BridgePbftService {
    /// Validates one external pillar vote with Rust-owned FinalChain composition.
    ///
    /// `vote_rlp` and `context` enter the checked network-admission path. The
    /// returned preparation plan preserves deterministic rejection status and
    /// recovered identity for the C++ facade. A ready preparation queries DPoS
    /// at `period - 1` without the pillar mutex; future and zero-weight results
    /// discard only that exact generation-bound preparation. Infrastructure
    /// errors also clean the exact preparation and propagate to the caller.
    pub fn pbft_service_pillar_validate_single_vote_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<FfiPillarVoteSingleAdmissionPreparePlan> {
        let mut prepared = self
            .pillar_state(true)?
            .pbft_service_pillar_prepare_single_vote_admission(vote_rlp, context)?;
        if !prepared.can_query_dpos || prepared.period == 0 {
            return Ok(single_validation_result(&prepared));
        }

        let vote_hash = H256::from(prepared.vote_hash);
        let vote_count = final_chain
            .0
            .pbft_dpos_eligible_vote_count(prepared.period - 1, prepared.voter);
        match vote_count {
            Ok(Some(weight)) if weight > 0 => Ok(single_validation_result(&prepared)),
            Ok(Some(_)) => {
                self.pillar_state(true)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                prepared.status = PILLAR_VOTE_STATUS_NOT_ELIGIBLE;
                prepared.can_query_dpos = false;
                Ok(single_validation_result(&prepared))
            }
            Ok(None) => {
                self.pillar_state(true)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                prepared.status = PILLAR_VOTE_STATUS_FUTURE_PERIOD;
                prepared.can_query_dpos = false;
                Ok(single_validation_result(&prepared))
            }
            Err(error) => {
                self.pillar_state(true)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                Err(error)
            }
        }
    }

    /// Prepares, weights, and applies one pillar vote without exposing DPoS facts to C++.
    ///
    /// Checked external votes use `context`; trusted local/restart votes use the
    /// explicitly selected preparation route. Rust queries validator weight and,
    /// when required, total weight at `period - 1` after releasing the pillar
    /// mutex. Apply then reacquires the mutex and consumes the exact retained
    /// preparation, so anchor replacement returns the established stale status.
    /// The result includes compatibility identity and insertion telemetry; query
    /// failures never mutate aggregation state and clean only the matching token.
    pub fn pbft_service_pillar_apply_single_vote_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
        trusted_local_or_restore: bool,
    ) -> Result<PillarVoteSingleAdmissionWithFinalChainPlan> {
        let prepared = if trusted_local_or_restore {
            self.pillar_state(false)?
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote_rlp)?
        } else {
            self.pillar_state(true)?
                .pbft_service_pillar_prepare_single_vote_admission(vote_rlp, context)?
        };
        if !prepared.can_query_dpos || prepared.period == 0 {
            return Ok(single_with_final_chain_rejection(
                &prepared,
                prepared.status,
            ));
        }

        let vote_hash = H256::from(prepared.vote_hash);
        let dpos_period = prepared.period - 1;
        let validator_vote_count = match final_chain
            .0
            .pbft_dpos_eligible_vote_count(dpos_period, prepared.voter)
        {
            Ok(Some(weight)) if weight > 0 => weight,
            Ok(Some(_)) => {
                self.pillar_state(false)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                return Ok(single_with_final_chain_rejection(
                    &prepared,
                    PILLAR_VOTE_STATUS_NOT_ELIGIBLE,
                ));
            }
            Ok(None) => {
                self.pillar_state(false)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                return Ok(single_with_final_chain_rejection(
                    &prepared,
                    PILLAR_VOTE_STATUS_FUTURE_PERIOD,
                ));
            }
            Err(error) => {
                self.pillar_state(false)?
                    .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                return Err(error);
            }
        };
        let threshold = if prepared.needs_threshold {
            match final_chain
                .0
                .pbft_dpos_eligible_total_vote_count(dpos_period)
            {
                Ok(Some(total)) => Some(rustaxa_consensus::plan_pillar_consensus_threshold(total)),
                Ok(None) => {
                    self.pillar_state(false)?
                        .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                    return Ok(single_with_final_chain_rejection(
                        &prepared,
                        PILLAR_VOTE_STATUS_FUTURE_PERIOD,
                    ));
                }
                Err(error) => {
                    self.pillar_state(false)?
                        .discard_single_vote_preparation(vote_hash, prepared.anchor_generation)?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let applied = self
            .pillar_state(false)?
            .pbft_service_pillar_apply_prepared_single_vote_admission(
                PillarVoteSingleAdmissionApplyInput {
                    vote_hash: prepared.vote_hash,
                    validator_vote_count,
                    has_threshold: threshold.is_some(),
                    threshold: threshold.unwrap_or_default(),
                },
            )?;
        Ok(PillarVoteSingleAdmissionWithFinalChainPlan {
            status: applied.status,
            accepted: applied.accepted,
            duplicate: applied.duplicate,
            conflict_found: applied.conflict_found,
            conflicting_vote_hash: applied.conflicting_vote_hash,
            block_weight: applied.block_weight,
            validator_vote_count,
            period: prepared.period,
            vote_hash: prepared.vote_hash,
            voter: prepared.voter,
        })
    }

    /// Resolves the compatibility threshold directly from Rust FinalChain state.
    ///
    /// The caller supplies the DPoS period. Rust first verifies pillar readiness,
    /// releases its mutex, and queries the borrowed FinalChain. Ready totals are
    /// converted with the canonical strict-majority formula; future or failed
    /// snapshots return `available == false` plus the legacy-compatible error
    /// text rather than throwing. No pillar state is mutated.
    pub fn pbft_service_pillar_consensus_threshold_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        period: u64,
    ) -> Result<PillarConsensusThresholdLookup> {
        drop(self.pillar_state(true)?);
        Ok(
            match final_chain.0.pbft_dpos_eligible_total_vote_count(period) {
                Ok(Some(total)) => PillarConsensusThresholdLookup {
                    available: true,
                    threshold: rustaxa_consensus::plan_pillar_consensus_threshold(total),
                    error_code: String::new(),
                },
                Ok(None) => PillarConsensusThresholdLookup {
                    available: false,
                    threshold: 0,
                    error_code: "PBFT_FINAL_CHAIN_TOTAL_VOTES_FUTURE_PERIOD".into(),
                },
                Err(error) => PillarConsensusThresholdLookup {
                    available: false,
                    threshold: 0,
                    error_code: format!("PBFT_FINAL_CHAIN_TOTAL_VOTES_UNAVAILABLE: {error}"),
                },
            },
        )
    }

    pub fn pbft_service_pillar_prepare_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        self.pillar_state(true)?
            .pbft_service_pillar_prepare_single_vote_admission(vote_rlp, context)
    }

    pub fn pbft_service_pillar_prepare_trusted_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        self.pillar_state(false)?
            .pbft_service_pillar_prepare_trusted_single_vote_admission(vote_rlp)
    }

    pub fn pbft_service_pillar_plan_vote_relevance(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteRuntimeRelevanceContext,
    ) -> Result<FfiPillarVoteRelevancePlan> {
        self.pillar_state(true)?
            .pbft_service_pillar_plan_vote_relevance(vote_rlp, context)
    }

    pub fn pbft_service_pillar_apply_prepared_single_vote_admission(
        &self,
        input: PillarVoteSingleAdmissionApplyInput,
    ) -> Result<PillarVoteSingleAdmissionApplyPlan> {
        self.pillar_state(false)?
            .pbft_service_pillar_apply_prepared_single_vote_admission(input)
    }

    pub fn pbft_service_pillar_prepare_weighted_rlp_bundle(
        &self,
        vote_rlps: Vec<PillarVoteRlpPayload>,
        required_votes_period: u64,
    ) -> Result<PillarVoteWeightedBundlePreparePlan> {
        self.pillar_state(true)?
            .pbft_service_pillar_prepare_weighted_rlp_bundle(vote_rlps, required_votes_period)
    }

    pub fn pbft_service_pillar_apply_weighted_rlp_bundle(
        &self,
        input: PillarVoteWeightedBundleApplyInput,
    ) -> Result<PillarVoteBundleApplyPlan> {
        self.pillar_state(true)?
            .pbft_service_pillar_apply_weighted_rlp_bundle(input)
    }

    /// Inspects, weights, and applies one synced pillar-vote bundle in Rust.
    ///
    /// Canonical vote bytes are inspected and bound to the current anchor before
    /// the pillar mutex is released. Total and ordered validator weights are then
    /// queried at `required_votes_period - 1`; zero/unavailable validator weight
    /// preserves the first-bad-vote rejection, while an unavailable total marks
    /// `missing_threshold`. Apply reacquires the mutex and verifies the prepared
    /// anchor generation before initializing or mutating vote aggregation.
    pub fn pbft_service_pillar_apply_rlp_bundle_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        vote_rlps: Vec<PillarVoteRlpPayload>,
        required_votes_period: u64,
    ) -> Result<PillarVoteBundleWithFinalChainPlan> {
        let canonical_rlps = vote_rlps
            .iter()
            .map(|payload| payload.vote_rlp.clone())
            .collect::<Vec<_>>();
        let prepared = self
            .pillar_state(true)?
            .pbft_service_pillar_prepare_weighted_rlp_bundle(vote_rlps, required_votes_period)?;
        if !prepared.can_query_dpos || required_votes_period == 0 {
            let status = match prepared.status {
                PILLAR_VOTE_BUNDLE_STATUS_EMPTY => PILLAR_VOTE_BUNDLE_STATUS_EMPTY,
                PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED => {
                    PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED
                }
                _ => PILLAR_VOTE_STATUS_UNKNOWN,
            };
            return Ok(bundle_with_final_chain_rejection(
                prepared.status,
                false,
                status,
                prepared.first_bad_vote_hash,
            ));
        }

        let dpos_period = required_votes_period - 1;
        let total_vote_count = match final_chain
            .0
            .pbft_dpos_eligible_total_vote_count(dpos_period)
        {
            Ok(Some(total)) => total,
            Ok(None) | Err(_) => {
                return Ok(bundle_with_final_chain_rejection(
                    prepared.status,
                    true,
                    PILLAR_VOTE_STATUS_UNKNOWN,
                    prepared.first_bad_vote_hash,
                ));
            }
        };
        ensure!(
            prepared.inspections.len() == canonical_rlps.len(),
            "pillar bundle inspection count mismatch"
        );
        let mut weighted = Vec::with_capacity(canonical_rlps.len());
        for (inspection, vote_rlp) in prepared.inspections.iter().zip(canonical_rlps) {
            let weight = final_chain
                .0
                .pbft_dpos_eligible_vote_count(dpos_period, inspection.voter)
                .ok()
                .flatten()
                .unwrap_or_default();
            if weight == 0 {
                return Ok(bundle_with_final_chain_rejection(
                    prepared.status,
                    false,
                    PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT,
                    inspection.vote_hash,
                ));
            }
            weighted.push(PillarVoteWeightedRlpPayload { vote_rlp, weight });
        }
        let applied = self
            .pillar_state(true)?
            .pbft_service_pillar_apply_weighted_rlp_bundle(PillarVoteWeightedBundleApplyInput {
                votes: weighted,
                required_votes_period,
                threshold: rustaxa_consensus::plan_pillar_consensus_threshold(total_vote_count),
                anchor_generation: prepared.anchor_generation,
            })?;
        Ok(PillarVoteBundleWithFinalChainPlan {
            prepare_status: prepared.status,
            missing_threshold: false,
            status: applied.status,
            block_weight: applied.block_weight,
            selected_weight: applied.selected_weight,
            first_bad_vote_hash: applied.first_bad_vote_hash,
            insert_failed: applied.insert_failed,
            insert_failed_vote_hash: applied.insert_failed_vote_hash,
            applied_votes: applied.applied_votes,
        })
    }

    pub fn pbft_service_pillar_get_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> Result<PillarVotesPayloadLookup> {
        self.pillar_state(true)?
            .pbft_service_pillar_get_verified_vote_payloads(period, block_hash, above_threshold)
    }

    pub fn pbft_service_pillar_build_verified_vote_network_bundles(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        max_votes_per_bundle: usize,
    ) -> Result<PillarVoteNetworkBundleLookup> {
        self.pillar_state(true)?
            .pbft_service_pillar_build_verified_vote_network_bundles(
                period,
                block_hash,
                max_votes_per_bundle,
            )
    }

    pub fn pbft_service_pillar_prepare_finalized_block_for_pbft(
        &self,
        request: PillarBlockFinalizationRequest,
    ) -> Result<PillarBlockFinalizationPrepareResult> {
        self.pillar_state(true)?
            .pbft_service_pillar_prepare_finalized_block_for_pbft(request)
    }

    pub fn pbft_service_pillar_ack_finalize_block_for_pbft(
        &self,
        request: PillarBlockFinalizationAcknowledgeRequest,
    ) -> Result<PillarBlockFinalizationAcknowledgeResult> {
        self.pillar_state(true)?
            .pbft_service_pillar_ack_finalize_block_for_pbft(request)
    }
}

fn single_with_final_chain_rejection(
    prepared: &PillarVoteSingleAdmissionPreparePlan,
    status: u8,
) -> PillarVoteSingleAdmissionWithFinalChainPlan {
    PillarVoteSingleAdmissionWithFinalChainPlan {
        status,
        accepted: false,
        duplicate: false,
        conflict_found: false,
        conflicting_vote_hash: [0; 32],
        block_weight: 0,
        validator_vote_count: 0,
        period: prepared.period,
        vote_hash: prepared.vote_hash,
        voter: prepared.voter,
    }
}

fn single_validation_result(
    prepared: &PillarVoteSingleAdmissionPreparePlan,
) -> FfiPillarVoteSingleAdmissionPreparePlan {
    FfiPillarVoteSingleAdmissionPreparePlan {
        status: prepared.status,
        period: prepared.period,
        vote_hash: prepared.vote_hash,
        voter: prepared.voter,
    }
}

fn bundle_with_final_chain_rejection(
    prepare_status: u8,
    missing_threshold: bool,
    status: u8,
    first_bad_vote_hash: [u8; 32],
) -> PillarVoteBundleWithFinalChainPlan {
    PillarVoteBundleWithFinalChainPlan {
        prepare_status,
        missing_threshold,
        status,
        block_weight: 0,
        selected_weight: 0,
        first_bad_vote_hash,
        insert_failed: false,
        insert_failed_vote_hash: [0; 32],
        applied_votes: 0,
    }
}

fn build_network_bundle_chunks(
    votes: Vec<(PillarVote, H256)>,
    max_votes_per_bundle: usize,
) -> Result<Vec<PillarVoteNetworkBundleChunk>> {
    if votes.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::with_capacity(votes.len().div_ceil(max_votes_per_bundle));
    for vote_chunk in votes.chunks(max_votes_per_bundle) {
        let vote_hashes = vote_chunk
            .iter()
            .map(|(_, vote_hash)| PillarVoteBundleHash {
                hash: (*vote_hash).into(),
            })
            .collect();
        let chunk_votes = vote_chunk
            .iter()
            .map(|(vote, _)| vote.clone())
            .collect::<Vec<_>>();
        chunks.push(PillarVoteNetworkBundleChunk {
            vote_hashes,
            votes_bundle_rlp: encode_optimized_pillar_votes_bundle_rlp(&chunk_votes)?,
        });
    }
    Ok(chunks)
}

fn load_stored_period_pillar_votes(
    storage: &Storage,
    period: u64,
    requested_hash: H256,
) -> Result<Vec<(PillarVote, H256)>> {
    let period_data = storage.period().data_raw(period)?;
    if period_data.is_empty() {
        return Ok(Vec::new());
    }

    let rlp = Rlp::new(&period_data);
    if rlp.item_count()? <= 4 {
        return Ok(Vec::new());
    }

    let votes_bundle_rlp = rlp.at(4)?.as_raw().to_vec();
    let votes = decode_optimized_pillar_votes_bundle_rlp(&votes_bundle_rlp)?;
    if votes.is_empty() {
        return Ok(Vec::new());
    }

    ensure!(
        votes
            .iter()
            .all(|vote| vote.period == period && vote.block_hash == requested_hash),
        "stored pillar vote bundle does not match requested period/hash"
    );

    Ok(votes
        .into_iter()
        .map(|vote| {
            let vote_hash = vote.hash(true);
            (vote, vote_hash)
        })
        .collect())
}

fn stored_votes_to_payload_lookup(
    stored_votes: Vec<(PillarVote, H256)>,
) -> PillarVotesPayloadLookup {
    PillarVotesPayloadLookup {
        threshold_met: !stored_votes.is_empty(),
        block_weight: 0,
        selected_weight: 0,
        votes: stored_votes
            .into_iter()
            .map(|(vote, vote_hash)| PillarVoteRecord {
                vote_hash: vote_hash.into(),
                weight: 0,
                vote_rlp: vote.encode_rlp(),
            })
            .collect(),
    }
}

#[cfg(test)]
impl PillarVotesTestFixture {
    /// Prepares one pillar vote for validation or insertion without reading
    /// FinalChain or mutating aggregation state.
    ///
    /// Inputs:
    /// - `vote_rlp` is the canonical legacy PillarVote bytes.
    /// - `context` supplies immutable scheduling configuration. Relevance and
    ///   identity uniqueness are always checked before supplying DPoS facts.
    ///
    /// Outputs:
    /// - On status `0`, the recovered `(period, block_hash, vote_hash, voter)`
    ///   identity is ready for a Rust test to supply FinalChain DPoS facts.
    /// - Non-zero statuses match the compatibility validation enum and require
    ///   no further lookup.
    ///
    /// Edge behavior:
    /// - Malformed RLP and bridge-domain invariant failures return inspection
    ///   failure instead of panicking.
    /// - Exact duplicates are detected before relevance checks when relevance
    ///   is requested, preserving the legacy logging order.
    pub fn pillar_votes_prepare_single_vote_admission(
        &mut self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        let retained = vote_rlp.clone();
        let plan = prepare_single_vote_admission(&self.0, None, 0, vote_rlp, Some(&context))?;
        if plan.can_query_dpos {
            self.1.insert(
                H256::from(plan.vote_hash),
                SingleVotePreparation {
                    vote_rlp: retained,
                    anchor_generation: 0,
                    period: plan.period,
                    block_hash: H256::from(plan.block_hash),
                    voter: ethereum_types::H160::from(plan.voter),
                    needs_threshold: plan.needs_threshold,
                    current_anchor: None,
                    first_pillar_block_period: context.first_pillar_block_period,
                    pillar_blocks_interval: context.pillar_blocks_interval,
                    trusted_local_or_restore: false,
                },
            );
        }
        Ok(plan)
    }

    fn pillar_votes_prepare_trusted_single_vote_admission(
        &mut self,
        vote_rlp: Vec<u8>,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        let retained = vote_rlp.clone();
        let plan = prepare_single_vote_admission(&self.0, None, 0, vote_rlp, None)?;
        if plan.can_query_dpos {
            self.1.insert(
                H256::from(plan.vote_hash),
                SingleVotePreparation {
                    vote_rlp: retained,
                    anchor_generation: 0,
                    period: plan.period,
                    block_hash: H256::from(plan.block_hash),
                    voter: ethereum_types::H160::from(plan.voter),
                    needs_threshold: plan.needs_threshold,
                    current_anchor: None,
                    first_pillar_block_period: 0,
                    pillar_blocks_interval: 0,
                    trusted_local_or_restore: true,
                },
            );
        }
        Ok(plan)
    }

    /// Applies one pillar vote after a Rust test supplies validator weight and,
    /// when needed, the period threshold.
    ///
    /// Inputs:
    /// - `input` carries canonical RLP, DPoS weight, and optional threshold.
    ///   Production obtains these facts through the composed service instead.
    ///
    /// Outputs:
    /// - Returns insertion status plus duplicate/conflict metadata from the
    ///   Rust-owned `PillarVotes` registry.
    ///
    /// Invariants and edge behavior:
    /// - RLP identity and recovered voter are derived in Rust before mutation;
    ///   C++ cannot supply or override vote identity.
    /// - A zero DPoS weight is rejected as not eligible.
    /// - If period state is absent and no threshold is supplied, the method
    ///   returns unknown and does not mutate state.
    pub fn pillar_votes_apply_prepared_single_vote_admission(
        &mut self,
        input: PillarVoteSingleAdmissionApplyInput,
    ) -> Result<PillarVoteSingleAdmissionApplyPlan> {
        let Some(preparation) = self.1.remove(&H256::from(input.vote_hash)) else {
            return Ok(single_admission_apply_plan(
                PILLAR_VOTE_STATUS_MISSING_PREPARATION,
            ));
        };
        apply_prepared_single_vote_admission(&mut self.0, preparation.vote_rlp, input)
    }
}

/// Inspects a legacy-encoded PillarVote payload without mutating state.
///
/// Use this before inserting a vote to recover voter/address and check
/// signature validity from vote RLP alone.
pub fn pillar_vote_inspect(vote_rlp: &[u8]) -> Result<PillarVoteInspection> {
    Ok(inspect_pillar_vote_from_rlp(vote_rlp)?.into())
}

/// Inspects one batch of canonical pillar-vote bytes for Rust-only planner tests.
///
/// Inputs:
/// - `votes`: ordered canonical vote RLP payloads from the sync bundle.
///
/// Outputs:
/// - Returns the recovered vote identity for every valid payload.
/// - Returns a stable bundle status and first bad vote hash for empty,
///   malformed, or invalid-signature input.
///
/// Invariants and edge behavior:
/// - The returned `inspections` order matches the input order.
/// - This function does not read FinalChain state and does not mutate the
///   pillar-vote index.
fn inspect_pillar_vote_bundle_rlps(
    votes: Vec<PillarVoteRlpPayload>,
) -> Result<PillarVoteBundleInspectionPlan> {
    if votes.is_empty() {
        return Ok(PillarVoteBundleInspectionPlan {
            status: PILLAR_VOTE_BUNDLE_STATUS_EMPTY,
            inspections: Vec::new(),
            first_bad_vote_hash: [0; 32],
        });
    }

    let mut inspections = Vec::with_capacity(votes.len());
    for vote in votes {
        let inspection = match inspect_pillar_vote_from_rlp(&vote.vote_rlp) {
            Ok(inspection) => inspection,
            Err(_) => {
                return Ok(PillarVoteBundleInspectionPlan {
                    status: PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED,
                    inspections: Vec::new(),
                    first_bad_vote_hash: [0; 32],
                })
            }
        };
        let output = PillarVoteInspection::from(inspection);
        if !output.signature_valid {
            return Ok(PillarVoteBundleInspectionPlan {
                status: PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED,
                inspections: Vec::new(),
                first_bad_vote_hash: output.vote_hash,
            });
        }
        inspections.push(output);
    }

    Ok(PillarVoteBundleInspectionPlan {
        status: PILLAR_VOTE_BUNDLE_STATUS_VALID,
        inspections,
        first_bad_vote_hash: [0; 32],
    })
}

fn plan_weighted_rlp_bundle(
    votes: Vec<PillarVoteWeightedRlpPayload>,
    expected_period: u64,
    expected_block_hash: &[u8; 32],
    threshold: u64,
) -> Result<WeightedRlpBundlePlanWork> {
    if votes.is_empty() {
        return Ok(WeightedRlpBundlePlanWork {
            plan: WeightedRlpBundlePlan {
                status: PILLAR_VOTE_BUNDLE_STATUS_EMPTY,
                accepted_vote_hashes: Vec::new(),
                block_weight: 0,
                selected_weight: 0,
                first_bad_vote_hash: [0; 32],
            },
            votes_by_hash: HashMap::new(),
        });
    }

    let mut facts = Vec::with_capacity(votes.len());
    let mut votes_by_hash = HashMap::with_capacity(votes.len());

    for vote in votes {
        let decoded_vote = match PillarVote::decode_rlp(&vote.vote_rlp) {
            Ok(vote) => vote,
            Err(_) => {
                return Ok(WeightedRlpBundlePlanWork {
                    plan: WeightedRlpBundlePlan {
                        status: PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED,
                        accepted_vote_hashes: Vec::new(),
                        block_weight: 0,
                        selected_weight: 0,
                        first_bad_vote_hash: [0; 32],
                    },
                    votes_by_hash: HashMap::new(),
                })
            }
        };
        let inspection = match inspect_pillar_vote_from_rlp(&vote.vote_rlp) {
            Ok(inspection) => inspection,
            Err(_) => {
                return Ok(WeightedRlpBundlePlanWork {
                    plan: WeightedRlpBundlePlan {
                        status: PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED,
                        accepted_vote_hashes: Vec::new(),
                        block_weight: 0,
                        selected_weight: 0,
                        first_bad_vote_hash: [0; 32],
                    },
                    votes_by_hash: HashMap::new(),
                })
            }
        };
        let vote_hash = inspection.vote_hash;
        if !inspection.signature_valid {
            return Ok(WeightedRlpBundlePlanWork {
                plan: WeightedRlpBundlePlan {
                    status: PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED,
                    accepted_vote_hashes: Vec::new(),
                    block_weight: 0,
                    selected_weight: 0,
                    first_bad_vote_hash: vote_hash.into(),
                },
                votes_by_hash: HashMap::new(),
            });
        }
        if vote.weight == 0 {
            return Ok(WeightedRlpBundlePlanWork {
                plan: WeightedRlpBundlePlan {
                    status: PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT,
                    accepted_vote_hashes: Vec::new(),
                    block_weight: 0,
                    selected_weight: 0,
                    first_bad_vote_hash: vote_hash.into(),
                },
                votes_by_hash: HashMap::new(),
            });
        }

        votes_by_hash.insert(
            vote_hash,
            VerifiedPillarVote::from_parts(decoded_vote, vote_hash, inspection.voter, vote.weight)?,
        );
        facts.push(ConsensusPillarVoteFact {
            vote_hash,
            period: inspection.period,
            block_hash: inspection.block_hash,
            voter: inspection.voter,
            weight: vote.weight,
            prevalidated: true,
        });
    }

    let planner =
        PillarVoteBundlePlanner::new(expected_period, H256::from(*expected_block_hash), threshold);
    let plan = planner.plan(&facts);
    let accepted_vote_hashes = plan
        .accepted_votes
        .iter()
        .map(|vote| vote.vote_hash)
        .collect();

    Ok(WeightedRlpBundlePlanWork {
        plan: WeightedRlpBundlePlan {
            status: plan.status.as_u8(),
            accepted_vote_hashes,
            block_weight: plan.block_weight,
            selected_weight: plan.selected_weight,
            first_bad_vote_hash: plan.first_bad_vote_hash.into(),
        },
        votes_by_hash,
    })
}

/// Evaluates one pillar-vote relevance query and returns a deterministic reason.
#[cfg(test)]
pub fn plan_pillar_vote_relevance(
    fact: FfiPillarVoteRelevanceFact,
) -> Result<FfiPillarVoteRelevancePlan> {
    let fact = relevance_fact_to_consensus_fact(fact)?;
    Ok(FfiPillarVoteRelevancePlan::from(
        rustaxa_consensus::plan_pillar_vote_relevance(fact)?,
    ))
}

fn unknown_relevance_plan() -> FfiPillarVoteRelevancePlan {
    FfiPillarVoteRelevancePlan {
        status: 255,
        is_relevant: false,
    }
}

fn runtime_plan_vote_relevance(
    pillar_votes: &PillarVotes,
    current_anchor: Option<rustaxa_consensus::PillarCurrentAnchor>,
    vote_rlp: Vec<u8>,
    context: PillarVoteRuntimeRelevanceContext,
) -> Result<FfiPillarVoteRelevancePlan> {
    let Ok(vote) = PillarVote::decode_rlp(&vote_rlp) else {
        return Ok(unknown_relevance_plan());
    };
    let vote_hash = vote.hash(true);
    let lookup = pillar_votes.get_verified_votes(vote.period, vote.block_hash, false);
    let vote_already_known = lookup
        .votes
        .iter()
        .any(|known| known.vote_hash == vote_hash);

    Ok(FfiPillarVoteRelevancePlan::from(
        rustaxa_consensus::plan_pillar_vote_relevance(ConsensusPillarVoteRelevanceFact {
            vote_period: vote.period,
            vote_block_hash: vote.block_hash,
            current_pillar_block_period: current_anchor.map(|anchor| anchor.period),
            current_pillar_block_hash: current_anchor.map(|anchor| anchor.hash),
            first_pillar_block_period: context.first_pillar_block_period,
            pillar_blocks_interval: context.pillar_blocks_interval,
            vote_already_known,
        })?,
    ))
}

fn single_admission_plan(status: u8) -> PillarVoteSingleAdmissionPreparePlan {
    PillarVoteSingleAdmissionPreparePlan {
        status,
        can_query_dpos: false,
        needs_threshold: false,
        period: 0,
        block_hash: [0; 32],
        vote_hash: [0; 32],
        voter: [0; 20],
        anchor_generation: 0,
        has_current_anchor: false,
        current_period: 0,
        current_hash: [0; 32],
    }
}

fn preparation_plan(preparation: &SingleVotePreparation) -> PillarVoteSingleAdmissionPreparePlan {
    let inspection = inspect_pillar_vote_from_rlp(&preparation.vote_rlp)
        .expect("retained single-vote preparation was inspected before insertion");
    let (has_current_anchor, current_period, current_hash) = preparation
        .current_anchor
        .map(|anchor| (true, anchor.period, anchor.hash.into()))
        .unwrap_or((false, 0, [0; 32]));
    PillarVoteSingleAdmissionPreparePlan {
        status: PILLAR_VOTE_STATUS_VALID,
        can_query_dpos: true,
        needs_threshold: preparation.needs_threshold,
        period: preparation.period,
        block_hash: preparation.block_hash.into(),
        vote_hash: inspection.vote_hash.into(),
        voter: preparation.voter.into(),
        anchor_generation: preparation.anchor_generation,
        has_current_anchor,
        current_period,
        current_hash,
    }
}

fn prepare_single_vote_admission(
    pillar_votes: &PillarVotes,
    current_anchor: Option<rustaxa_consensus::PillarCurrentAnchor>,
    anchor_generation: u64,
    vote_rlp: Vec<u8>,
    checked_context: Option<&PillarVoteSingleAdmissionContext>,
) -> Result<PillarVoteSingleAdmissionPreparePlan> {
    let decoded_vote = match PillarVote::decode_rlp(&vote_rlp) {
        Ok(vote) => vote,
        Err(_) => return Ok(single_admission_plan(PILLAR_VOTE_STATUS_INSPECTION_FAILURE)),
    };
    let inspection = match inspect_pillar_vote_from_rlp(&vote_rlp) {
        Ok(inspection) => inspection,
        Err(_) => return Ok(single_admission_plan(PILLAR_VOTE_STATUS_INSPECTION_FAILURE)),
    };
    let mut plan = single_admission_plan(PILLAR_VOTE_STATUS_VALID);
    plan.period = inspection.period;
    plan.block_hash = inspection.block_hash.into();
    plan.vote_hash = inspection.vote_hash.into();
    plan.voter = inspection.voter.into();
    plan.anchor_generation = anchor_generation;
    if let Some(anchor) = current_anchor {
        plan.has_current_anchor = true;
        plan.current_period = anchor.period;
        plan.current_hash = anchor.hash.into();
    }

    ensure!(
        decoded_vote.period == inspection.period,
        "pillar vote decoded period does not match recovered inspection"
    );
    ensure!(
        decoded_vote.block_hash == inspection.block_hash,
        "pillar vote decoded block hash does not match recovered inspection"
    );
    ensure!(
        decoded_vote.hash(true) == inspection.vote_hash,
        "pillar vote decoded hash does not match recovered inspection"
    );

    if !inspection.signature_valid {
        plan.status = PILLAR_VOTE_STATUS_SIGNATURE_INVALID;
        return Ok(plan);
    }

    if let Some(context) = checked_context {
        let duplicate_probe = VerifiedPillarVote::from_parts(
            decoded_vote.clone(),
            inspection.vote_hash,
            inspection.voter,
            1,
        )?;
        let relevance =
            rustaxa_consensus::plan_pillar_vote_relevance(ConsensusPillarVoteRelevanceFact {
                vote_period: inspection.period,
                vote_block_hash: inspection.block_hash,
                current_pillar_block_period: current_anchor.map(|anchor| anchor.period),
                current_pillar_block_hash: current_anchor.map(|anchor| anchor.hash),
                first_pillar_block_period: context.first_pillar_block_period,
                pillar_blocks_interval: context.pillar_blocks_interval,
                vote_already_known: pillar_votes.vote_exists(&duplicate_probe),
            })?;
        if !relevance.is_relevant {
            plan.status = relevance.status_code();
            return Ok(plan);
        }
    }

    if checked_context.is_some()
        && !pillar_votes.is_unique_vote_identity(ConsensusPillarVoteIdentity {
            period: inspection.period,
            vote_hash: inspection.vote_hash,
            voter: inspection.voter,
        })
    {
        plan.status = PILLAR_VOTE_STATUS_NOT_UNIQUE;
        return Ok(plan);
    }

    plan.needs_threshold = !pillar_votes.period_data_initialized(inspection.period);
    plan.can_query_dpos = true;
    Ok(plan)
}

fn prepare_weighted_rlp_bundle(
    current_anchor: Option<rustaxa_consensus::PillarCurrentAnchor>,
    anchor_generation: u64,
    vote_rlps: Vec<PillarVoteRlpPayload>,
    required_votes_period: u64,
) -> Result<PillarVoteWeightedBundlePreparePlan> {
    let mut plan = PillarVoteWeightedBundlePreparePlan {
        status: PILLAR_VOTE_BUNDLE_STATUS_VALID,
        can_query_dpos: false,
        inspections: Vec::new(),
        first_bad_vote_hash: [0; 32],
        expected_block_hash: [0; 32],
        anchor_generation,
        has_current_anchor: false,
        current_period: 0,
        current_hash: [0; 32],
    };
    if vote_rlps.is_empty() {
        plan.status = PILLAR_VOTE_BUNDLE_STATUS_EMPTY;
        return Ok(plan);
    }
    let Some(anchor) = current_anchor else {
        plan.status = 2;
        return Ok(plan);
    };
    plan.has_current_anchor = true;
    plan.current_period = anchor.period;
    plan.current_hash = anchor.hash.into();
    plan.expected_block_hash = anchor.hash.into();
    if anchor.period.checked_add(1) != Some(required_votes_period) {
        plan.status = 3;
        return Ok(plan);
    }
    let inspection = inspect_pillar_vote_bundle_rlps(vote_rlps)?;
    plan.status = inspection.status;
    plan.first_bad_vote_hash = inspection.first_bad_vote_hash;
    plan.inspections = inspection.inspections;
    plan.can_query_dpos = plan.status == PILLAR_VOTE_BUNDLE_STATUS_VALID;
    Ok(plan)
}

fn single_admission_apply_plan(status: u8) -> PillarVoteSingleAdmissionApplyPlan {
    PillarVoteSingleAdmissionApplyPlan {
        status,
        accepted: false,
        duplicate: false,
        conflict_found: false,
        conflicting_vote_hash: [0; 32],
        block_weight: 0,
    }
}

fn apply_prepared_single_vote_admission(
    pillar_votes: &mut PillarVotes,
    vote_rlp: Vec<u8>,
    input: PillarVoteSingleAdmissionApplyInput,
) -> Result<PillarVoteSingleAdmissionApplyPlan> {
    if input.validator_vote_count == 0 {
        return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_NOT_ELIGIBLE));
    }

    let (vote, period) = match signed_rlp_to_verified_vote(vote_rlp, input.validator_vote_count) {
        Ok(Some(vote)) => vote,
        Ok(None) => {
            return Ok(single_admission_apply_plan(
                PILLAR_VOTE_STATUS_SIGNATURE_INVALID,
            ));
        }
        Err(_) => {
            return Ok(single_admission_apply_plan(
                PILLAR_VOTE_STATUS_INSPECTION_FAILURE,
            ));
        }
    };

    if !pillar_votes.period_data_initialized(period) {
        if !input.has_threshold {
            return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_UNKNOWN));
        }
        pillar_votes.initialize_period_data(period, input.threshold);
    }

    let outcome = pillar_votes.add_verified_vote(vote)?;
    Ok(PillarVoteSingleAdmissionApplyPlan {
        status: PILLAR_VOTE_STATUS_VALID,
        accepted: outcome.accepted,
        duplicate: outcome.duplicate,
        conflict_found: outcome.conflicting_vote_hash.is_some(),
        conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
        block_weight: outcome.block_weight,
    })
}

fn bundle_apply_rejection(status: u8) -> PillarVoteBundleApplyPlan {
    PillarVoteBundleApplyPlan {
        status,
        block_weight: 0,
        selected_weight: 0,
        first_bad_vote_hash: [0; 32],
        insert_failed: false,
        insert_failed_vote_hash: [0; 32],
        applied_votes: 0,
    }
}

fn apply_weighted_rlp_bundle(
    pillar_votes: &mut PillarVotes,
    votes: Vec<PillarVoteWeightedRlpPayload>,
    expected_period: u64,
    expected_block_hash: &[u8; 32],
    threshold: u64,
) -> Result<PillarVoteBundleApplyPlan> {
    let work = plan_weighted_rlp_bundle(votes, expected_period, expected_block_hash, threshold)?;
    if work.plan.status != PILLAR_VOTE_BUNDLE_STATUS_VALID {
        return Ok(PillarVoteBundleApplyPlan {
            status: work.plan.status,
            block_weight: work.plan.block_weight,
            selected_weight: work.plan.selected_weight,
            first_bad_vote_hash: work.plan.first_bad_vote_hash,
            insert_failed: false,
            insert_failed_vote_hash: [0; 32],
            applied_votes: 0,
        });
    }

    pillar_votes.initialize_period_data(expected_period, threshold);
    let mut applied_votes = 0u64;
    for accepted_vote_hash in &work.plan.accepted_vote_hashes {
        let vote_hash = *accepted_vote_hash;
        let Some(vote) = work.votes_by_hash.get(&vote_hash).cloned() else {
            return Ok(PillarVoteBundleApplyPlan {
                status: work.plan.status,
                block_weight: work.plan.block_weight,
                selected_weight: work.plan.selected_weight,
                first_bad_vote_hash: work.plan.first_bad_vote_hash,
                insert_failed: true,
                insert_failed_vote_hash: accepted_vote_hash.0,
                applied_votes,
            });
        };

        match pillar_votes.add_verified_vote(vote) {
            Ok(outcome) if outcome.accepted || outcome.duplicate => {
                applied_votes = applied_votes.saturating_add(1);
            }
            Ok(_) | Err(_) => {
                return Ok(PillarVoteBundleApplyPlan {
                    status: work.plan.status,
                    block_weight: work.plan.block_weight,
                    selected_weight: work.plan.selected_weight,
                    first_bad_vote_hash: work.plan.first_bad_vote_hash,
                    insert_failed: true,
                    insert_failed_vote_hash: accepted_vote_hash.0,
                    applied_votes,
                });
            }
        }
    }

    Ok(PillarVoteBundleApplyPlan {
        status: work.plan.status,
        block_weight: work.plan.block_weight,
        selected_weight: work.plan.selected_weight,
        first_bad_vote_hash: work.plan.first_bad_vote_hash,
        insert_failed: false,
        insert_failed_vote_hash: [0; 32],
        applied_votes,
    })
}

#[cfg(test)]
fn relevance_fact_to_consensus_fact(
    value: FfiPillarVoteRelevanceFact,
) -> Result<ConsensusPillarVoteRelevanceFact> {
    let current_pillar_block_period = if value.has_current_pillar_block {
        Some(value.current_pillar_block_period)
    } else {
        None
    };
    let current_pillar_block_hash = if value.has_current_pillar_block {
        Some(H256::from(value.current_pillar_block_hash))
    } else {
        None
    };

    Ok(ConsensusPillarVoteRelevanceFact {
        vote_period: value.vote_period,
        vote_block_hash: H256::from(value.vote_block_hash),
        current_pillar_block_period,
        current_pillar_block_hash,
        first_pillar_block_period: value.first_pillar_block_period,
        pillar_blocks_interval: value.pillar_blocks_interval,
        vote_already_known: value.vote_already_known,
    })
}

#[cfg(test)]
struct PillarVotePayload {
    vote_hash: [u8; 32],
    block_hash: [u8; 32],
    voter: [u8; 20],
    period: u64,
    weight: u64,
    vote_rlp: Vec<u8>,
}

fn signed_rlp_to_verified_vote(
    vote_rlp: Vec<u8>,
    weight: u64,
) -> Result<Option<(VerifiedPillarVote, u64)>> {
    let vote = PillarVote::decode_rlp(&vote_rlp)?;
    let inspection = inspect_pillar_vote_from_rlp(&vote_rlp)?;
    if !inspection.signature_valid {
        return Ok(None);
    }
    ensure!(
        vote.period == inspection.period,
        "pillar vote decoded period does not match recovered inspection"
    );
    ensure!(
        vote.block_hash == inspection.block_hash,
        "pillar vote decoded block hash does not match recovered inspection"
    );
    ensure!(
        vote.hash(true) == inspection.vote_hash,
        "pillar vote decoded hash does not match recovered inspection"
    );

    let period = inspection.period;
    Ok(Some((
        VerifiedPillarVote::from_parts(vote, inspection.vote_hash, inspection.voter, weight)?,
        period,
    )))
}

#[cfg(test)]
fn payload_to_vote(value: PillarVotePayload) -> Result<VerifiedPillarVote> {
    let vote = PillarVote::decode_rlp(&value.vote_rlp)?;
    ensure!(
        value.period == vote.period,
        "pillar vote payload period mismatch: {payload_period} != {vote_period}",
        payload_period = value.period,
        vote_period = vote.period
    );
    ensure!(
        H256::from(value.block_hash) == vote.block_hash,
        "pillar vote payload block hash mismatch for period {}",
        value.period
    );
    ensure!(
        H256::from(value.vote_hash) == vote.hash(true),
        "pillar vote payload hash mismatch for period {}",
        value.period
    );

    VerifiedPillarVote::from_parts(
        vote,
        H256::from(value.vote_hash),
        ethereum_types::H160::from(value.voter),
        value.weight,
    )
}

impl From<rustaxa_consensus::VerifiedPillarVote> for PillarVoteRecord {
    fn from(value: rustaxa_consensus::VerifiedPillarVote) -> Self {
        Self {
            vote_hash: value.vote_hash.into(),
            weight: value.weight,
            vote_rlp: value.vote.encode_rlp(),
        }
    }
}

impl From<rustaxa_consensus::PillarVotesLookup> for PillarVotesPayloadLookup {
    fn from(value: rustaxa_consensus::PillarVotesLookup) -> Self {
        Self {
            threshold_met: value.threshold_met,
            block_weight: value.block_weight,
            selected_weight: value.selected_weight,
            votes: value
                .votes
                .into_iter()
                .map(PillarVoteRecord::from)
                .collect(),
        }
    }
}

impl From<ConsensusPillarVoteRelevancePlan> for FfiPillarVoteRelevancePlan {
    fn from(value: ConsensusPillarVoteRelevancePlan) -> Self {
        Self {
            status: value.status_code(),
            is_relevant: value.is_relevant,
        }
    }
}

impl From<ConsensusPillarVoteInspection> for PillarVoteInspection {
    fn from(value: ConsensusPillarVoteInspection) -> Self {
        Self {
            status: u8::from(!value.signature_valid),
            period: value.period,
            block_hash: value.block_hash.into(),
            vote_hash: value.vote_hash.into(),
            voter: value.voter.into(),
            signature_valid: value.signature_valid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi;
    use crate::final_chain::create_final_chain;
    use crate::pillar_chain::{
        create_pillar_chain_storage, create_pillar_test_service_from_storage,
    };
    use crate::storage::create_storage;
    use ethereum_types::H160;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn u256_be(value: u64) -> Vec<u8> {
        ethereum_types::U256::from(value).to_big_endian().to_vec()
    }

    fn final_chain_for_voters(
        storage: &crate::ffi::BridgeStorage,
        voters: &[[u8; 20]],
    ) -> Box<BridgeFinalChain> {
        let validators = voters
            .iter()
            .map(|address| rustaxa_ffi::GenesisValidator {
                address: *address,
                owner: *address,
                vrf_key: [address[0]; 32],
                commission: 0,
                description: String::new(),
                endpoint: String::new(),
                total_stake: u256_be(5_000),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: *address,
                    stake: u256_be(5_000),
                }],
            })
            .collect();
        create_final_chain(
            storage,
            0,
            0,
            Vec::new(),
            validators,
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

    fn signature(seed: u8) -> [u8; 65] {
        let mut signature = [seed; 65];
        signature[64] = seed & 1;
        signature
    }

    fn vote(period: u64, block: u64, voter: u64, seed: u8, weight: u64) -> PillarVotePayload {
        let vote = PillarVote {
            period,
            block_hash: H256::from_low_u64_be(block),
            signature: signature(seed),
        };
        PillarVotePayload {
            vote_hash: vote.hash(true).into(),
            block_hash: vote.block_hash.into(),
            voter: [voter as u8; 20],
            period,
            weight,
            vote_rlp: vote.encode_rlp(),
        }
    }

    fn clone_payload(value: &PillarVotePayload) -> PillarVotePayload {
        PillarVotePayload {
            vote_hash: value.vote_hash,
            block_hash: value.block_hash,
            voter: value.voter,
            period: value.period,
            weight: value.weight,
            vote_rlp: value.vote_rlp.clone(),
        }
    }

    fn keccak256(data: &[u8]) -> H256 {
        use tiny_keccak::{Hasher, Keccak};

        let mut output = [0u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(data);
        hasher.finalize(&mut output);
        H256::from(output)
    }

    fn signed_vote(seed: u8, period: u64, block: u64) -> (PillarVote, [u8; 20]) {
        let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
        signed_vote_with_key(&signing_key, period, block)
    }

    fn signed_vote_with_key(
        signing_key: &SigningKey,
        period: u64,
        block: u64,
    ) -> (PillarVote, [u8; 20]) {
        signed_vote_with_key_and_hash(signing_key, period, H256::from_low_u64_be(block))
    }

    fn signed_vote_with_key_and_hash(
        signing_key: &SigningKey,
        period: u64,
        block_hash: H256,
    ) -> (PillarVote, [u8; 20]) {
        let mut vote = PillarVote {
            period,
            block_hash,
            signature: [0u8; 65],
        };
        let unsigned_hash = vote.hash(false);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(unsigned_hash.as_bytes())
            .unwrap();
        let signature_bytes_fixed = signature.to_bytes();
        let mut signature_bytes = [0u8; 65];
        signature_bytes[..64].copy_from_slice(&signature_bytes_fixed);
        signature_bytes[64] = recovery_id.to_byte();
        vote.signature = signature_bytes;

        let voter = {
            let verifying_key = signing_key.verifying_key();
            let public_key = verifying_key.to_encoded_point(false);
            let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
            public_key_hash.as_bytes()[12..].try_into().unwrap()
        };

        (vote, voter)
    }

    fn current_data(period: u64) -> (PillarBlock, Vec<u8>) {
        let block = PillarBlock {
            period,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash: H256::from_low_u64_be(2),
            bridge_root: H256::from_low_u64_be(3),
            epoch: 4,
            validator_vote_count_changes: Vec::new(),
        };
        let bytes = CurrentPillarBlockDataDb {
            pillar_block: block.clone(),
            vote_counts: Vec::new(),
        }
        .encode_rlp();
        (block, bytes)
    }

    fn period_data_with_pillar_votes(votes: &[PillarVote]) -> Vec<u8> {
        let votes_bundle = encode_optimized_pillar_votes_bundle_rlp(votes).unwrap();
        let mut period_data = RlpStream::new_list(5);
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.append_empty_data();
        period_data.begin_list(0);
        period_data.append_raw(&votes_bundle, 1);
        period_data.out().to_vec()
    }

    #[test]
    fn inspect_pillar_vote_recovers_voter_and_signature_status() {
        let (vote, voter) = signed_vote(0x11, 9_999, 77);
        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(inspected.signature_valid);
        assert_eq!(inspected.status, 0);
        assert_eq!(inspected.period, 9_999);
        assert_eq!(H256::from(inspected.block_hash), H256::from_low_u64_be(77));
        assert_eq!(H256::from(inspected.vote_hash), vote.hash(true));
        assert_eq!(inspected.voter, voter);
    }

    #[test]
    fn inspect_pillar_vote_reports_invalid_signature_without_error() {
        let (mut vote, _) = signed_vote(0x12, 100, 78);
        vote.signature = [0u8; 65];

        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(!inspected.signature_valid);
        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.voter, [0u8; 20]);
    }

    #[test]
    fn inspect_pillar_vote_rejects_out_of_range_recovery_id() {
        let (mut vote, _) = signed_vote(0x13, 101, 79);
        vote.signature[64] = 4;

        let inspected = pillar_vote_inspect(&vote.encode_rlp()).unwrap();

        assert!(!inspected.signature_valid);
        assert_eq!(inspected.status, 1);
        assert_eq!(inspected.voter, [0u8; 20]);
    }

    #[test]
    fn inspect_pillar_vote_rejects_malformed_rlp() {
        assert!(pillar_vote_inspect(&[1, 2, 3]).is_err());
    }

    fn single_admission_context(_block_hash: H256) -> PillarVoteSingleAdmissionContext {
        PillarVoteSingleAdmissionContext {
            first_pillar_block_period: 41,
            pillar_blocks_interval: 10,
        }
    }

    #[test]
    fn single_vote_admission_prepare_and_apply_insert_vote() {
        let mut votes = create_pillar_votes_index();
        let (vote, voter) = signed_vote(0x21, 42, 77);

        let prepared = votes
            .pillar_votes_prepare_single_vote_admission(
                vote.encode_rlp(),
                single_admission_context(vote.block_hash),
            )
            .unwrap();

        assert!(prepared.can_query_dpos);
        assert!(prepared.needs_threshold);
        assert_eq!(prepared.period, 42);
        assert_eq!(H256::from(prepared.block_hash), vote.block_hash);
        assert_eq!(H256::from(prepared.vote_hash), vote.hash(true));
        assert_eq!(prepared.voter, voter);

        let applied = votes
            .pillar_votes_apply_prepared_single_vote_admission(
                PillarVoteSingleAdmissionApplyInput {
                    vote_hash: vote.hash(true).into(),
                    validator_vote_count: 6,
                    has_threshold: true,
                    threshold: 5,
                },
            )
            .unwrap();

        assert_eq!(applied.status, PILLAR_VOTE_STATUS_VALID);
        assert!(applied.accepted);
        assert!(!applied.duplicate);
        assert_eq!(applied.block_weight, 6);

        let duplicate_prepare = votes
            .pillar_votes_prepare_single_vote_admission(
                vote.encode_rlp(),
                single_admission_context(vote.block_hash),
            )
            .unwrap();
        assert_eq!(duplicate_prepare.status, 1);
        assert!(!duplicate_prepare.can_query_dpos);
    }

    #[test]
    fn pbft_service_pillar_prepares_and_acknowledges_block_for_pbft_with_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_finalization");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);
            let (block, current_data_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_data_rlp)
                .expect("current block should apply");
            let signing_key = SigningKey::from_slice(&[0x24; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            let block_rlp = block.encode_rlp();
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .expect("trusted vote should prepare");

            let applied = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .expect("vote should apply");
            assert_eq!(applied.status, PILLAR_VOTE_STATUS_VALID);
            assert!(applied.accepted);

            let prepared = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .expect("pillar finalization should prepare");

            assert_eq!(prepared.status, 0);
            assert!(prepared.success);
            assert!(prepared.should_emit);
            assert!(prepared.has_prepared_pillar_block);
            assert_eq!(prepared.prepared_pillar_block_period, 41);
            assert_eq!(prepared.prepared_pillar_block_rlp, block_rlp);
            assert_eq!(prepared.selected_vote_count, 1);
            assert_eq!(prepared.votes.len(), 1);
            assert_eq!(
                prepared.votes[0].vote_hash,
                Into::<[u8; 32]>::into(vote.hash(true))
            );

            pillar_storage
                .pillar_chain_storage_apply_finalized_block(41, block_rlp.clone())
                .expect("PBFT primary batch fixture should persist the prepared row");
            let acknowledged = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(
                    PillarBlockFinalizationAcknowledgeRequest {
                        anchor_generation: prepared.preparation_anchor_generation,
                        preparation_token: prepared.preparation_token,
                    },
                )
                .expect("pillar finalization should acknowledge");
            assert!(acknowledged.should_emit);

            assert_eq!(acknowledged.latest_finalized_period, 41);
            assert_eq!(
                acknowledged.latest_finalized_hash,
                Into::<[u8; 32]>::into(block.hash())
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_block(41)
                    .expect("persisted prepared row should remain durable"),
                block_rlp
            );
            assert_eq!(
                runtime
                    .pbft_service_pillar_latest_finalized_block_rlp()
                    .expect("runtime latest-finalized snapshot should update"),
                block_rlp,
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_single_admission_rejects_stale_anchor_generation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_stale_single");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (vote, _) = signed_vote(0x25, 42, 77);
            let prepared = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert_eq!(prepared.anchor_generation, 0);

            runtime
                .pbft_service_pillar_apply_current_block_data(current_data(41).1)
                .unwrap();
            let rejected = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();
            assert_eq!(rejected.status, PILLAR_VOTE_STATUS_STALE_ANCHOR);
            assert!(!rejected.accepted);
            assert!(!runtime
                .pillar_state(false)
                .unwrap()
                .votes
                .period_data_initialized(vote.period));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn trusted_prepare_reuses_checked_token_and_cannot_refresh_stale_anchor() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_checked_token_reuse");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let key = SigningKey::from_slice(&[0x27; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, block.hash());
            let checked = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert_eq!(checked.anchor_generation, 1);

            runtime
                .pbft_service_pillar_apply_current_block_data(current_data(51).1)
                .unwrap();
            let reused = runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            assert_eq!(reused.anchor_generation, checked.anchor_generation);
            assert_eq!(reused.current_period, checked.current_period);
            assert_eq!(reused.current_hash, checked.current_hash);

            let rejected = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();
            assert_eq!(rejected.status, PILLAR_VOTE_STATUS_STALE_ANCHOR);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn checked_reprepare_after_anchor_change_revalidates_and_stays_untrusted() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_checked_reprepare");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (old_block, old_current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(old_current_rlp)
                .unwrap();
            let key = SigningKey::from_slice(&[0x2A; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, old_block.hash());
            let checked = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert!(checked.can_query_dpos);
            assert_eq!(checked.anchor_generation, 1);

            let replacement_block = PillarBlock {
                period: 41,
                state_root: H256::from_low_u64_be(999),
                previous_pillar_block_hash: H256::from_low_u64_be(2),
                bridge_root: H256::from_low_u64_be(3),
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            };
            runtime
                .pbft_service_pillar_apply_current_block_data(
                    CurrentPillarBlockDataDb {
                        pillar_block: replacement_block,
                        vote_counts: Vec::new(),
                    }
                    .encode_rlp(),
                )
                .unwrap();

            let rechecked = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert_eq!(rechecked.status, 4);
            assert!(!rechecked.can_query_dpos);
            let state = runtime.pillar_state(false).unwrap();
            let registry = state.single_vote_preparations.lock().unwrap();
            let retained = registry.entries.get(&vote.hash(true)).unwrap();
            assert_eq!(retained.anchor_generation, checked.anchor_generation);
            assert!(!retained.trusted_local_or_restore);
            drop(registry);
            drop(state);

            let rejected = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();
            assert_eq!(rejected.status, PILLAR_VOTE_STATUS_STALE_ANCHOR);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn checked_apply_revalidates_identity_after_prepare_race() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_identity_race");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let key = SigningKey::from_slice(&[0x28; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, block.hash());
            let prepared = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert!(prepared.can_query_dpos);

            let (conflict, _) = signed_vote_with_key(&key, 42, 999);
            let (conflict, _) = signed_rlp_to_verified_vote(conflict.encode_rlp(), 4)
                .unwrap()
                .unwrap();
            runtime
                .pillar_state(false)
                .unwrap()
                .votes
                .initialize_period_data(42, 5);
            assert!(
                runtime
                    .pillar_state(false)
                    .unwrap()
                    .votes
                    .add_verified_vote(conflict)
                    .unwrap()
                    .accepted
            );

            let rejected = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: false,
                        threshold: 0,
                    },
                )
                .unwrap();
            assert_eq!(rejected.status, PILLAR_VOTE_STATUS_NOT_UNIQUE);
            assert!(!rejected.accepted);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn checked_apply_revalidates_relevance_after_duplicate_race() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_relevance_race");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let key = SigningKey::from_slice(&[0x29; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, block.hash());
            let prepared = runtime
                .pbft_service_pillar_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 41,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert!(prepared.can_query_dpos);

            let (inserted, _) = signed_rlp_to_verified_vote(vote.encode_rlp(), 4)
                .unwrap()
                .unwrap();
            runtime
                .pillar_state(false)
                .unwrap()
                .votes
                .initialize_period_data(42, 5);
            assert!(
                runtime
                    .pillar_state(false)
                    .unwrap()
                    .votes
                    .add_verified_vote(inserted)
                    .unwrap()
                    .accepted
            );

            let rejected = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: false,
                        threshold: 0,
                    },
                )
                .unwrap();
            assert_eq!(rejected.status, 1);
            assert!(!rejected.accepted);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn single_vote_preparation_registry_is_bounded_and_apply_fails_closed() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_preparation_bound");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            {
                let state = runtime.pillar_state(false).unwrap();
                for value in 0..=MAX_SINGLE_VOTE_PREPARATIONS {
                    let vote_hash = H256::from_low_u64_be(value as u64);
                    state
                        .retain_single_vote_preparation(
                            vote_hash,
                            SingleVotePreparation {
                                vote_rlp: vec![0x80],
                                anchor_generation: 0,
                                period: 1,
                                block_hash: H256::zero(),
                                voter: ethereum_types::H160::zero(),
                                needs_threshold: false,
                                current_anchor: None,
                                first_pillar_block_period: 0,
                                pillar_blocks_interval: 1,
                                trusted_local_or_restore: true,
                            },
                        )
                        .unwrap();
                }
                assert_eq!(
                    state.single_vote_preparations.lock().unwrap().entries.len(),
                    MAX_SINGLE_VOTE_PREPARATIONS
                );
            }
            let missing = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: H256::zero().into(),
                        validator_vote_count: 1,
                        has_threshold: true,
                        threshold: 1,
                    },
                )
                .unwrap();
            assert_eq!(missing.status, PILLAR_VOTE_STATUS_MISSING_PREPARATION);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_weighted_bundle_prepare_and_apply_are_generation_bound() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_stale_bundle");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let key = SigningKey::from_slice(&[0x26; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, block.hash());
            let prepared = runtime
                .pbft_service_pillar_prepare_weighted_rlp_bundle(
                    vec![PillarVoteRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                    }],
                    42,
                )
                .unwrap();
            assert_eq!(prepared.status, 0);
            assert!(prepared.can_query_dpos);
            assert_eq!(prepared.inspections.len(), 1);
            assert_eq!(
                prepared.expected_block_hash,
                Into::<[u8; 32]>::into(block.hash())
            );
            assert_eq!(prepared.anchor_generation, 1);

            runtime
                .pbft_service_pillar_apply_current_block_data(current_data(51).1)
                .unwrap();
            let rejected = runtime
                .pbft_service_pillar_apply_weighted_rlp_bundle(PillarVoteWeightedBundleApplyInput {
                    votes: vec![PillarVoteWeightedRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                        weight: 6,
                    }],
                    required_votes_period: 42,
                    threshold: 5,
                    anchor_generation: prepared.anchor_generation,
                })
                .unwrap();
            assert_eq!(rejected.status, PILLAR_VOTE_BUNDLE_STATUS_STALE_ANCHOR);
            assert_eq!(rejected.applied_votes, 0);
            assert!(!runtime
                .pillar_state(false)
                .unwrap()
                .votes
                .period_data_initialized(42));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_finalization_derives_anchor_and_suppresses_period_overflow() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_finalize_overflow");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(u64::MAX);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let result = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: block.hash().into(),
                    },
                )
                .unwrap();
            assert_eq!(result.status, 3);
            assert_eq!(result.current_period, u64::MAX);
            assert_eq!(result.current_hash, Into::<[u8; 32]>::into(block.hash()));
            assert!(!result.should_request_votes);
            assert!(!result.has_request_votes_period);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_finalization_prepares_already_finalized_without_votes_after_ack_cleanup() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_already_finalized_replay");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            let pillar_storage = create_pillar_chain_storage(&storage);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp.clone())
                .unwrap();
            let signing_key = SigningKey::from_slice(&[0x30; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();
            let prepared = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .unwrap();
            pillar_storage
                .pillar_chain_storage_apply_finalized_block(41, block.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(
                    PillarBlockFinalizationAcknowledgeRequest {
                        anchor_generation: prepared.preparation_anchor_generation,
                        preparation_token: prepared.preparation_token,
                    },
                )
                .unwrap();

            let replayed = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .unwrap();
            assert_eq!(replayed.status, 4);
            assert!(!replayed.has_prepared_pillar_block);
            assert!(!replayed.should_emit);
            assert!(replayed.votes.is_empty());
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_finalization_prepare_is_reused_within_generation_for_same_block() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_finalization_reuse");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp.clone())
                .unwrap();
            let signing_key = SigningKey::from_slice(&[0x20; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();

            let first = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .expect("first finalization prepare should succeed");
            let second = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .expect("second finalization prepare should reuse");

            assert!(first.has_prepared_pillar_block);
            assert!(second.has_prepared_pillar_block);
            assert_eq!(first.preparation_token, second.preparation_token);
            assert_eq!(
                runtime
                    .pillar_state(false)
                    .unwrap()
                    .pillar_block_finalization_preparations
                    .lock()
                    .unwrap()
                    .len(),
                1
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_finalization_ack_preserves_token_until_prepared_row_is_persistent_and_matching() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_finalization_ack_retry");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).expect("storage should open");
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let pillar_storage = create_pillar_chain_storage(&storage);
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp)
                .unwrap();
            let signing_key = SigningKey::from_slice(&[0x50; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();

            let prepared = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .unwrap();
            let ack_request =
                |generation: u64, token: u64| PillarBlockFinalizationAcknowledgeRequest {
                    anchor_generation: generation,
                    preparation_token: token,
                };

            let missing = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(ack_request(
                    prepared.preparation_anchor_generation,
                    prepared.preparation_token,
                ))
                .expect_err("prepared pillar row should be required");
            assert!(missing
                .to_string()
                .contains("PILLAR_BLOCK_FINALIZATION_PREPARED_BLOCK_NOT_PERSISTENT"));

            let wrong_block = PillarBlock {
                period: 41,
                state_root: H256::from_low_u64_be(99),
                previous_pillar_block_hash: block.previous_pillar_block_hash,
                bridge_root: block.bridge_root,
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            }
            .encode_rlp();
            pillar_storage
                .pillar_chain_storage_apply_finalized_block(41, wrong_block)
                .unwrap();

            let stale = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(ack_request(
                    prepared.preparation_anchor_generation,
                    prepared.preparation_token,
                ))
                .expect_err("prepared block hash mismatch should preserve token");
            assert!(stale
                .to_string()
                .contains("PILLAR_BLOCK_FINALIZATION_PREPARED_BLOCK_MISMATCH"));

            pillar_storage
                .pillar_chain_storage_apply_finalized_block(41, block.encode_rlp())
                .unwrap();
            let acknowledged = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(ack_request(
                    prepared.preparation_anchor_generation,
                    prepared.preparation_token,
                ))
                .expect("matching persisted prepared pillar row should allow ack");
            assert!(acknowledged.should_emit);
            assert_eq!(acknowledged.latest_finalized_period, 41);
            assert_eq!(
                acknowledged.latest_finalized_hash,
                Into::<[u8; 32]>::into(block.hash())
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn pbft_service_pillar_finalization_ack_rejects_reused_token() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_prep_token_reuse");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let pillar_storage = create_pillar_chain_storage(&storage);
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp.clone())
                .unwrap();
            let signing_key = SigningKey::from_slice(&[0x31; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .unwrap();
            let prepared = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .unwrap();
            pillar_storage
                .pillar_chain_storage_apply_finalized_block(41, block.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(
                    PillarBlockFinalizationAcknowledgeRequest {
                        anchor_generation: prepared.preparation_anchor_generation,
                        preparation_token: prepared.preparation_token,
                    },
                )
                .unwrap();

            let repeated = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(
                    PillarBlockFinalizationAcknowledgeRequest {
                        anchor_generation: prepared.preparation_anchor_generation,
                        preparation_token: prepared.preparation_token,
                    },
                )
                .expect_err("reused token should reject");
            assert!(repeated
                .to_string()
                .contains("PILLAR_BLOCK_FINALIZATION_ACK_TOKEN_REUSED"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn pbft_service_pillar_finalization_ack_rejects_stale_generation() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_prep_token_stale_generation");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (block, current_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_rlp.clone())
                .unwrap();
            let signing_key = SigningKey::from_slice(&[0x32; 32]).unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&signing_key, 42, block.hash());
            runtime
                .pbft_service_pillar_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        has_threshold: true,
                        threshold: 5,
                        validator_vote_count: 6,
                    },
                )
                .unwrap();
            let prepared = runtime
                .pbft_service_pillar_prepare_finalized_block_for_pbft(
                    PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: vote.block_hash.into(),
                    },
                )
                .unwrap();

            runtime
                .pbft_service_pillar_apply_current_block_data(current_data(42).1)
                .unwrap();
            let stale = runtime
                .pbft_service_pillar_ack_finalize_block_for_pbft(
                    PillarBlockFinalizationAcknowledgeRequest {
                        anchor_generation: prepared.preparation_anchor_generation,
                        preparation_token: prepared.preparation_token,
                    },
                )
                .expect_err("stale generation should reject");
            assert!(stale
                .to_string()
                .contains("PILLAR_BLOCK_FINALIZATION_ACK_STALE_GENERATION"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn single_vote_admission_apply_reports_same_voter_conflict() {
        let mut votes = create_pillar_votes_index();
        let signing_key = SigningKey::from_slice(&[0x31; 32]).unwrap();
        let (first, voter) = signed_vote_with_key(&signing_key, 51, 88);
        let (conflict, _) = signed_vote_with_key(&signing_key, 51, 89);

        for vote in [&first, &conflict] {
            let prepared = votes
                .pillar_votes_prepare_trusted_single_vote_admission(vote.encode_rlp())
                .unwrap();
            assert!(prepared.can_query_dpos);
            assert_eq!(prepared.voter, voter);
            let applied = votes
                .pillar_votes_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 4,
                        has_threshold: prepared.needs_threshold,
                        threshold: 5,
                    },
                )
                .unwrap();
            if vote.block_hash == first.block_hash {
                assert!(applied.accepted);
            } else {
                assert!(!applied.accepted);
                assert!(applied.conflict_found);
                assert_eq!(H256::from(applied.conflicting_vote_hash), first.hash(true));
            }
        }
    }

    #[test]
    fn single_vote_admission_apply_rejects_invalid_signature() {
        let mut votes = create_pillar_votes_index();
        let (mut vote, _) = signed_vote(0x41, 61, 98);
        vote.signature = [0u8; 65];

        let prepared = votes
            .pillar_votes_prepare_trusted_single_vote_admission(vote.encode_rlp())
            .unwrap();
        assert_eq!(prepared.status, PILLAR_VOTE_STATUS_SIGNATURE_INVALID);
        assert!(!prepared.can_query_dpos);
    }

    #[test]
    fn insert_vote_accepts_votes_and_tracks_weight() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.0.initialize_period_data(10, 10));

        let first = vote(10, 11, 1, 0xAA, 4);
        let second = vote(10, 11, 2, 0xAB, 6);

        let first_outcome = votes
            .0
            .add_verified_vote(payload_to_vote(first).unwrap())
            .unwrap();
        let second_outcome = votes
            .0
            .add_verified_vote(payload_to_vote(second).unwrap())
            .unwrap();

        assert!(first_outcome.accepted);
        assert!(!first_outcome.duplicate);
        assert_eq!(first_outcome.block_weight, 4);
        assert!(second_outcome.accepted);
        assert!(!second_outcome.duplicate);
        assert_eq!(second_outcome.block_weight, 10);
    }

    #[test]
    fn duplicate_vote_hash_is_rejected_for_weight_recount_and_kept_unique() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.0.initialize_period_data(11, 1));

        let first = vote(11, 12, 1, 0xAC, 6);
        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&first)).unwrap())
            .unwrap();
        let duplicate = votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&first)).unwrap())
            .unwrap();

        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert!(duplicate.conflicting_vote_hash.is_none());
        assert_eq!(duplicate.block_weight, 6);

        let lookup = votes.pillar_votes_get_verified_vote_payloads(11, &first.block_hash, false);
        assert_eq!(lookup.votes.len(), 1);
        assert_eq!(lookup.votes[0].vote_hash, first.vote_hash);
    }

    #[test]
    fn unique_vote_rejects_conflicting_voter() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.0.initialize_period_data(12, 1));

        let first = vote(12, 13, 1, 0xB0, 5);
        let conflict = vote(12, 14, 1, 0xB1, 5);

        let inserted = votes
            .0
            .add_verified_vote(payload_to_vote(first).unwrap())
            .unwrap();
        assert!(inserted.accepted);

        assert!(!votes.0.is_unique_vote(&payload_to_vote(conflict).unwrap()));
    }

    #[test]
    fn vote_exists_and_period_initialized_mirror_registry_state() {
        let mut votes = create_pillar_votes_index();
        let first = vote(12, 13, 1, 0xAF, 5);

        assert!(!votes.0.period_data_initialized(12));
        votes.0.initialize_period_data(12, 1);
        assert!(votes.0.period_data_initialized(12));
        assert!(!votes
            .0
            .vote_exists(&payload_to_vote(clone_payload(&first)).unwrap()));

        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&first)).unwrap())
            .unwrap();
        assert!(votes.0.vote_exists(&payload_to_vote(first).unwrap()));
    }

    #[test]
    fn above_threshold_lookup_selects_minimum_prefix_when_met() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.0.initialize_period_data(13, 7));

        let low = vote(13, 15, 1, 0xC0, 1);
        let mid = vote(13, 15, 2, 0xC1, 3);
        let high = vote(13, 15, 3, 0xC2, 4);
        votes
            .0
            .add_verified_vote(payload_to_vote(low).unwrap())
            .unwrap();
        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&mid)).unwrap())
            .unwrap();
        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&high)).unwrap())
            .unwrap();

        let payload_lookup =
            votes.pillar_votes_get_verified_vote_payloads(13, &high.block_hash, true);
        assert!(payload_lookup.threshold_met);
        assert_eq!(payload_lookup.block_weight, 8);
        assert_eq!(payload_lookup.selected_weight, 7);
        assert_eq!(payload_lookup.votes.len(), 2);
        assert_eq!(payload_lookup.votes[0].vote_hash, high.vote_hash);
        assert_eq!(payload_lookup.votes[0].weight, 4);
        assert_eq!(payload_lookup.votes[0].vote_rlp, high.vote_rlp);
        assert_eq!(payload_lookup.votes[1].vote_hash, mid.vote_hash);
        assert_eq!(payload_lookup.votes[1].weight, 3);
        assert_eq!(payload_lookup.votes[1].vote_rlp, mid.vote_rlp);
    }

    #[test]
    fn above_threshold_lookup_returns_empty_until_threshold() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.0.initialize_period_data(14, 10));

        let first = vote(14, 16, 1, 0xD0, 4);
        let second = vote(14, 16, 2, 0xD1, 5);
        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&first)).unwrap())
            .unwrap();
        votes
            .0
            .add_verified_vote(payload_to_vote(clone_payload(&second)).unwrap())
            .unwrap();

        let lookup = votes.pillar_votes_get_verified_vote_payloads(14, &first.block_hash, true);
        assert!(!lookup.threshold_met);
        assert_eq!(lookup.block_weight, 9);
        assert_eq!(lookup.selected_weight, 0);
        assert!(lookup.votes.is_empty());
    }

    #[test]
    fn cleanup_votes_removes_only_stale_periods() {
        let mut votes = create_pillar_votes_index();
        for period in 20..23 {
            assert!(votes.0.initialize_period_data(period, 1));
            votes
                .0
                .add_verified_vote(
                    payload_to_vote(vote(
                        period,
                        20,
                        period,
                        (period as u8).wrapping_add(0x10),
                        1,
                    ))
                    .unwrap(),
                )
                .unwrap();
        }

        votes.pillar_votes_cleanup_votes_by_period(22);

        assert!(votes
            .0
            .add_verified_vote(payload_to_vote(vote(20, 20, 30, 0xE0, 1)).unwrap())
            .is_err());
        assert!(!votes
            .0
            .is_unique_vote(&payload_to_vote(vote(22, 20, 22, 0xE2, 1)).unwrap()));
        assert!(votes
            .0
            .is_unique_vote(&payload_to_vote(vote(22, 20, 23, 0xE3, 1)).unwrap()));
        let retained = votes.pillar_votes_get_verified_vote_payloads(
            22,
            &vote(22, 20, 22, 0xE2, 1).block_hash,
            false,
        );
        assert_eq!(retained.votes.len(), 1);
        assert_eq!(retained.votes[0].weight, 1);
    }

    #[test]
    fn plain_payload_round_trips_vote_rlp_and_hash_fields() {
        let vote = PillarVote {
            period: 50,
            block_hash: H256::from_low_u64_be(99),
            signature: [0x11u8; 65],
        };
        let payload = PillarVotePayload {
            vote_hash: vote.hash(true).into(),
            block_hash: vote.block_hash.into(),
            voter: [5u8; 20],
            period: vote.period,
            weight: 7,
            vote_rlp: vote.encode_rlp(),
        };

        let decoded = payload_to_vote(clone_payload(&payload)).unwrap();
        assert_eq!(decoded.vote.period, payload.period);
        assert_eq!(decoded.voter, H160::from(payload.voter));
        assert_eq!(decoded.weight, payload.weight);
        assert_eq!(decoded.vote.encode_rlp(), payload.vote_rlp);
    }

    #[test]
    fn inspect_bundle_rlps_returns_ordered_vote_identities() {
        let (first, first_voter) = signed_vote(0x21, 40, 1234);
        let (second, second_voter) = signed_vote(0x22, 40, 1234);
        let votes = vec![
            PillarVoteRlpPayload {
                vote_rlp: first.encode_rlp(),
            },
            PillarVoteRlpPayload {
                vote_rlp: second.encode_rlp(),
            },
        ];

        let plan = inspect_pillar_vote_bundle_rlps(votes).unwrap();

        assert_eq!(plan.status, PILLAR_VOTE_BUNDLE_STATUS_VALID);
        assert_eq!(plan.inspections.len(), 2);
        assert_eq!(plan.inspections[0].vote_hash, first.hash(true).0);
        assert_eq!(plan.inspections[0].voter, first_voter);
        assert_eq!(plan.inspections[1].vote_hash, second.hash(true).0);
        assert_eq!(plan.inspections[1].voter, second_voter);
    }

    #[test]
    fn apply_weighted_rlp_bundle_inserts_selected_votes() {
        let (first, _) = signed_vote(0x31, 40, 1234);
        let (second, _) = signed_vote(0x32, 40, 1234);
        let expected_block_hash = H256::from_low_u64_be(1234).into();
        let votes = vec![
            PillarVoteWeightedRlpPayload {
                vote_rlp: first.encode_rlp(),
                weight: 4,
            },
            PillarVoteWeightedRlpPayload {
                vote_rlp: second.encode_rlp(),
                weight: 3,
            },
        ];
        let mut registry = create_pillar_votes_index();

        let plan = registry
            .pillar_votes_apply_weighted_rlp_bundle(votes, 40, &expected_block_hash, 7)
            .unwrap();

        assert_eq!(plan.status, PILLAR_VOTE_BUNDLE_STATUS_VALID);
        assert!(!plan.insert_failed);
        assert_eq!(plan.applied_votes, 2);
        let lookup =
            registry.pillar_votes_get_verified_vote_payloads(40, &expected_block_hash, true);
        assert!(lookup.threshold_met);
        assert_eq!(lookup.selected_weight, 7);
        assert_eq!(lookup.votes.len(), 2);
    }

    #[test]
    fn network_bundle_chunks_use_runtime_votes_without_materializing_cpp_votes() {
        let dir = unique_temp_dir("pillar_network_runtime_bundle");
        let storage = create_storage(dir.to_string_lossy().as_ref()).expect("storage should open");
        let runtime = create_pillar_test_service_from_storage(&storage)
            .expect("pillar runtime should initialize");
        let period = 92;
        let (current_block, current_data_rlp) = current_data(period - 1);
        runtime
            .pbft_service_pillar_apply_current_block_data(current_data_rlp)
            .expect("current block should apply");
        let block = current_block.hash();
        let first_key = SigningKey::from_slice(&[0x61; 32]).unwrap();
        let second_key = SigningKey::from_slice(&[0x62; 32]).unwrap();
        let (first, _) = signed_vote_with_key_and_hash(&first_key, period, block);
        let (second, _) = signed_vote_with_key_and_hash(&second_key, period, block);
        let votes = vec![
            PillarVoteWeightedRlpPayload {
                vote_rlp: first.encode_rlp(),
                weight: 5,
            },
            PillarVoteWeightedRlpPayload {
                vote_rlp: second.encode_rlp(),
                weight: 4,
            },
        ];

        runtime
            .pbft_service_pillar_apply_weighted_rlp_bundle(PillarVoteWeightedBundleApplyInput {
                votes,
                required_votes_period: period,
                threshold: 1,
                anchor_generation: 1,
            })
            .unwrap();
        let lookup = runtime
            .pbft_service_pillar_build_verified_vote_network_bundles(
                period,
                block.as_fixed_bytes(),
                1,
            )
            .unwrap();

        assert!(!lookup.from_storage);
        assert_eq!(lookup.chunks.len(), 2);
        let first_decoded =
            decode_optimized_pillar_votes_bundle_rlp(&lookup.chunks[0].votes_bundle_rlp).unwrap();
        let second_decoded =
            decode_optimized_pillar_votes_bundle_rlp(&lookup.chunks[1].votes_bundle_rlp).unwrap();
        assert_eq!(first_decoded.len(), 1);
        assert_eq!(second_decoded.len(), 1);
        assert_eq!(first_decoded[0].period, period);
        assert_eq!(first_decoded[0].block_hash, block);
        assert_eq!(second_decoded[0].period, period);
        assert_eq!(second_decoded[0].block_hash, block);
        assert_eq!(
            lookup.chunks[0].vote_hashes[0].hash,
            <[u8; 32]>::from(first_decoded[0].hash(true))
        );
        assert_eq!(
            lookup.chunks[1].vote_hashes[0].hash,
            <[u8; 32]>::from(second_decoded[0].hash(true))
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn network_bundle_chunks_fall_back_to_matching_stored_period_data() {
        let dir = unique_temp_dir("pillar_network_storage_bundle");
        let storage = create_storage(dir.to_string_lossy().as_ref()).expect("storage should open");
        let runtime = create_pillar_test_service_from_storage(&storage)
            .expect("pillar runtime should initialize");
        let period = 93;
        let block = H256::from_low_u64_be(9300);
        let (first, _) = signed_vote(0x63, period, 9300);
        let (second, _) = signed_vote(0x64, period, 9300);
        storage
            .0
            .period()
            .write(
                period,
                &period_data_with_pillar_votes(&[first.clone(), second.clone()]),
            )
            .unwrap();

        let lookup = runtime
            .pbft_service_pillar_build_verified_vote_network_bundles(
                period,
                block.as_fixed_bytes(),
                250,
            )
            .unwrap();

        assert!(lookup.from_storage);
        assert_eq!(lookup.chunks.len(), 1);
        assert_eq!(lookup.chunks[0].vote_hashes.len(), 2);
        let decoded =
            decode_optimized_pillar_votes_bundle_rlp(&lookup.chunks[0].votes_bundle_rlp).unwrap();
        assert_eq!(decoded, vec![first, second]);

        let mismatched = match runtime.pbft_service_pillar_build_verified_vote_network_bundles(
            period,
            H256::from_low_u64_be(9301).as_fixed_bytes(),
            250,
        ) {
            Ok(_) => panic!("mismatched storage bundle should be rejected"),
            Err(err) => err,
        };
        assert!(mismatched
            .to_string()
            .contains("stored pillar vote bundle does not match"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn verified_vote_payload_lookup_falls_back_to_matching_stored_period_data() {
        let dir = unique_temp_dir("pillar_payload_lookup_storage_bundle");
        let storage = create_storage(dir.to_string_lossy().as_ref()).expect("storage should open");
        let runtime = create_pillar_test_service_from_storage(&storage)
            .expect("pillar runtime should initialize");
        let period = 94;
        let block = H256::from_low_u64_be(9400);
        let (vote, _) = signed_vote(0x65, period, 9400);
        storage
            .0
            .period()
            .write(
                period,
                &period_data_with_pillar_votes(std::slice::from_ref(&vote)),
            )
            .unwrap();

        let lookup = runtime
            .pbft_service_pillar_get_verified_vote_payloads(period, block.as_fixed_bytes(), true)
            .unwrap();

        assert!(lookup.threshold_met);
        assert_eq!(lookup.block_weight, 0);
        assert_eq!(lookup.selected_weight, 0);
        assert_eq!(lookup.votes.len(), 1);
        assert_eq!(lookup.votes[0].vote_hash, <[u8; 32]>::from(vote.hash(true)));
        assert_eq!(lookup.votes[0].weight, 0);
        assert_eq!(lookup.votes[0].vote_rlp, vote.encode_rlp());

        let mismatched = match runtime.pbft_service_pillar_get_verified_vote_payloads(
            period,
            H256::from_low_u64_be(9401).as_fixed_bytes(),
            true,
        ) {
            Ok(_) => panic!("mismatched storage bundle should be rejected"),
            Err(err) => err,
        };
        assert!(mismatched
            .to_string()
            .contains("stored pillar vote bundle does not match"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn verified_vote_payload_lookup_keeps_live_below_threshold_state() {
        let dir = unique_temp_dir("pillar_payload_lookup_live_below_threshold");
        let storage = create_storage(dir.to_string_lossy().as_ref()).expect("storage should open");
        let runtime = create_pillar_test_service_from_storage(&storage)
            .expect("pillar runtime should initialize");
        let period = 95;
        let block = H256::from_low_u64_be(9500);
        let (live_vote, _) = signed_vote(0x66, period, 9500);
        let (stored_vote, _) = signed_vote(0x67, period, 9500);
        storage
            .0
            .period()
            .write(
                period,
                &period_data_with_pillar_votes(std::slice::from_ref(&stored_vote)),
            )
            .unwrap();
        let (verified_live_vote, _) = signed_rlp_to_verified_vote(live_vote.encode_rlp(), 2)
            .unwrap()
            .expect("signed live vote should verify");
        {
            let mut state = runtime.pillar_state(false).unwrap();
            assert!(state.votes.initialize_period_data(period, 10));
            state
                .votes
                .add_verified_vote(verified_live_vote)
                .expect("live below-threshold vote should be retained");
        }

        let lookup = runtime
            .pbft_service_pillar_get_verified_vote_payloads(period, block.as_fixed_bytes(), true)
            .unwrap();

        assert!(!lookup.threshold_met);
        assert_eq!(lookup.block_weight, 2);
        assert_eq!(lookup.selected_weight, 0);
        assert!(lookup.votes.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn weighted_rlp_bundle_rejects_zero_external_weight() {
        let (first, _) = signed_vote(0x26, 40, 1234);
        let expected_block_hash = H256::from_low_u64_be(1234).into();
        let votes = vec![PillarVoteWeightedRlpPayload {
            vote_rlp: first.encode_rlp(),
            weight: 0,
        }];

        let mut registry = create_pillar_votes_index();
        let plan = registry
            .pillar_votes_apply_weighted_rlp_bundle(votes, 40, &expected_block_hash, 7)
            .unwrap();

        assert_eq!(plan.status, PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT);
        assert_eq!(plan.first_bad_vote_hash, first.hash(true).0);
        assert_eq!(plan.applied_votes, 0);
    }

    #[test]
    fn weighted_rlp_bundle_rejects_invalid_signature() {
        let (mut first, _) = signed_vote(0x27, 40, 1234);
        first.signature = [0; 65];
        let expected_block_hash = H256::from_low_u64_be(1234).into();
        let votes = vec![PillarVoteWeightedRlpPayload {
            vote_rlp: first.encode_rlp(),
            weight: 5,
        }];

        let mut registry = create_pillar_votes_index();
        let plan = registry
            .pillar_votes_apply_weighted_rlp_bundle(votes, 40, &expected_block_hash, 7)
            .unwrap();

        assert_eq!(plan.status, PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED);
        assert_eq!(plan.first_bad_vote_hash, first.hash(true).0);
        assert_eq!(plan.applied_votes, 0);
    }

    fn relevance_fact(
        vote_period: u64,
        vote_block_hash: u64,
        current_pillar_block_period: Option<u64>,
        current_pillar_block_hash: u64,
        vote_already_known: bool,
    ) -> FfiPillarVoteRelevanceFact {
        FfiPillarVoteRelevanceFact {
            vote_period,
            vote_block_hash: H256::from_low_u64_be(vote_block_hash).into(),
            current_pillar_block_period: current_pillar_block_period.unwrap_or_default(),
            current_pillar_block_hash: H256::from_low_u64_be(current_pillar_block_hash).into(),
            has_current_pillar_block: current_pillar_block_period.is_some(),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 10,
            vote_already_known,
        }
    }

    #[test]
    fn plan_relevance_with_no_current_block_matches_first_period_plus_one() {
        let fact = relevance_fact(11, 10_001, None, 0, false);

        let plan = plan_pillar_vote_relevance(fact).unwrap();

        assert!(plan.is_relevant);
        assert_eq!(plan.status, 0);
    }

    #[test]
    fn plan_relevance_rejects_hash_mismatch_for_next_period() {
        let fact = relevance_fact(21, 777, Some(20), 333, false);

        let plan = plan_pillar_vote_relevance(fact).unwrap();

        assert!(!plan.is_relevant);
        assert_eq!(plan.status, 4);
    }

    #[test]
    fn plan_relevance_accepts_future_period_relative_to_current() {
        let fact = relevance_fact(31, 888, Some(20), 333, false);

        let plan = plan_pillar_vote_relevance(fact).unwrap();

        assert!(plan.is_relevant);
        assert_eq!(plan.status, 0);
    }

    #[test]
    fn plan_relevance_reports_known_vote_as_irrelevant() {
        let fact = relevance_fact(31, 888, Some(20), 333, true);

        let plan = plan_pillar_vote_relevance(fact).unwrap();

        assert!(!plan.is_relevant);
        assert_eq!(plan.status, 1);
    }

    #[test]
    fn runtime_relevance_derives_known_vote_from_owned_index() {
        let storage_dir = unique_temp_dir("pillar_runtime_relevance");
        let storage = create_storage(storage_dir.to_str().unwrap()).expect("storage should open");
        let runtime = create_pillar_test_service_from_storage(&storage)
            .expect("pillar runtime should initialize");
        let vote = vote(31, 888, 1, 0xD4, 6);
        {
            let mut state = runtime.pillar_state(false).unwrap();
            state.votes.initialize_period_data(31, 1);
            state
                .votes
                .add_verified_vote(payload_to_vote(clone_payload(&vote)).unwrap())
                .unwrap();
        }

        let plan = runtime
            .pbft_service_pillar_plan_vote_relevance(
                vote.vote_rlp,
                PillarVoteRuntimeRelevanceContext {
                    first_pillar_block_period: 10,
                    pillar_blocks_interval: 10,
                },
            )
            .unwrap();

        assert!(!plan.is_relevant);
        assert_eq!(plan.status, 1);
        let _ = fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn composed_single_vote_maps_zero_weight_and_cleans_exact_preparation() {
        let storage_dir = unique_temp_dir("pillar_composed_single_zero");
        {
            let storage = create_storage(storage_dir.to_str().unwrap()).unwrap();
            let final_chain = final_chain_for_voters(&storage, &[]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(
                &SigningKey::from_slice(&[0x71; 32]).unwrap(),
                1,
                anchor.hash(),
            );

            let plan = runtime
                .pbft_service_pillar_validate_single_vote_with_final_chain(
                    &final_chain,
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert_eq!(plan.status, PILLAR_VOTE_STATUS_NOT_ELIGIBLE);
            assert_eq!(plan.period, 1);
            assert_eq!(plan.vote_hash, vote.hash(true).0);

            let missing = runtime
                .pbft_service_pillar_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_hash: vote.hash(true).into(),
                        validator_vote_count: 5,
                        has_threshold: false,
                        threshold: 0,
                    },
                )
                .unwrap();
            assert_eq!(missing.status, PILLAR_VOTE_STATUS_MISSING_PREPARATION);
        }
        let _ = fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn composed_single_vote_checked_apply_queries_weight_and_threshold() {
        let storage_dir = unique_temp_dir("pillar_composed_single_apply");
        {
            let storage = create_storage(storage_dir.to_str().unwrap()).unwrap();
            let key = SigningKey::from_slice(&[0x72; 32]).unwrap();
            let (_, voter) = signed_vote_with_key(&key, 1, 1);
            let final_chain = final_chain_for_voters(&storage, &[voter]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 1, anchor.hash());

            let applied = runtime
                .pbft_service_pillar_apply_single_vote_with_final_chain(
                    &final_chain,
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                    false,
                )
                .unwrap();
            assert_eq!(applied.status, PILLAR_VOTE_STATUS_VALID);
            assert!(applied.accepted);
            assert!(applied.validator_vote_count > 0);
            assert_eq!(applied.voter, voter);
        }
        let _ = fs::remove_dir_all(storage_dir);
    }

    #[test]
    fn composed_bundle_distinguishes_missing_total_from_first_zero_weight() {
        let future_dir = unique_temp_dir("pillar_composed_bundle_future");
        {
            let storage = create_storage(future_dir.to_str().unwrap()).unwrap();
            let key = SigningKey::from_slice(&[0x73; 32]).unwrap();
            let (_, voter) = signed_vote_with_key(&key, 42, 1);
            let final_chain = final_chain_for_voters(&storage, &[voter]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(&key, 42, anchor.hash());
            let plan = runtime
                .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
                    &final_chain,
                    vec![PillarVoteRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                    }],
                    42,
                )
                .unwrap();
            assert!(plan.missing_threshold);
        }
        let _ = fs::remove_dir_all(future_dir);

        let zero_dir = unique_temp_dir("pillar_composed_bundle_zero");
        {
            let storage = create_storage(zero_dir.to_str().unwrap()).unwrap();
            let final_chain = final_chain_for_voters(&storage, &[]);
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let (anchor, anchor_rlp) = current_data(0);
            runtime
                .pbft_service_pillar_apply_current_block_data(anchor_rlp)
                .unwrap();
            let (vote, _) = signed_vote_with_key_and_hash(
                &SigningKey::from_slice(&[0x74; 32]).unwrap(),
                1,
                anchor.hash(),
            );
            let vote_hash: [u8; 32] = vote.hash(true).into();
            let plan = runtime
                .pbft_service_pillar_apply_rlp_bundle_with_final_chain(
                    &final_chain,
                    vec![PillarVoteRlpPayload {
                        vote_rlp: vote.encode_rlp(),
                    }],
                    1,
                )
                .unwrap();
            assert_eq!(plan.status, PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT);
            assert_eq!(plan.first_bad_vote_hash, vote_hash);
        }
        let _ = fs::remove_dir_all(zero_dir);
    }
}
