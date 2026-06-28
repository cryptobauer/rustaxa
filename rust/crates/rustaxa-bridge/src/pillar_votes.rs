//! CXX bridge wrappers for deterministic pillar-vote aggregation.
//!
//! The bridge accepts plain C++-style vote payloads and converts them into
//! `rustaxa_consensus::VerifiedPillarVote` domain values for stateful
//! aggregation. It also exposes canonical RLP inspection and weighted-RLP
//! bundle planning for C++ sync paths that keep FinalChain DPoS reads external.
//!
//! Pillar-vote inspection delegates byte-level signature recovery to
//! `rustaxa-types`; this layer exposes the CXX-compatible boundary and
//! enforces local bridge-domain invariants before delegating aggregation rules
//! to [`PillarVotes`].

use crate::ffi::rustaxa_ffi::{
    PillarBlockFinalizationRequest, PillarBlockFinalizationResult, PillarVoteBundleApplyPlan,
    PillarVoteBundleInspectionPlan, PillarVoteInspection, PillarVoteRecord,
    PillarVoteRelevanceFact as FfiPillarVoteRelevanceFact,
    PillarVoteRelevancePlan as FfiPillarVoteRelevancePlan, PillarVoteRlpPayload,
    PillarVoteSingleAdmissionApplyInput, PillarVoteSingleAdmissionApplyPlan,
    PillarVoteSingleAdmissionContext, PillarVoteSingleAdmissionPreparePlan,
    PillarVoteWeightedRlpPayload, PillarVotesPayloadLookup,
};
use crate::ffi::BridgePillarChainRuntime;
#[cfg(test)]
use crate::ffi::BridgePillarVotes;
use anyhow::{ensure, Result};
use ethereum_types::H256;
use rustaxa_consensus::{
    inspect_pillar_vote_from_rlp, PillarVoteBundlePlanner,
    PillarVoteFact as ConsensusPillarVoteFact, PillarVoteIdentity as ConsensusPillarVoteIdentity,
    PillarVoteInspection as ConsensusPillarVoteInspection,
    PillarVoteRelevanceFact as ConsensusPillarVoteRelevanceFact,
    PillarVoteRelevancePlan as ConsensusPillarVoteRelevancePlan, PillarVotes, VerifiedPillarVote,
};
use rustaxa_consensus::{
    plan_pillar_block_finalization, save_finalized_pillar_block_storage,
    PillarBlockFinalizationFact, PillarBlockFinalizationStatus,
};
use rustaxa_types::PillarVote;
use std::collections::HashMap;

const PILLAR_VOTE_BUNDLE_STATUS_VALID: u8 = 0;
const PILLAR_VOTE_BUNDLE_STATUS_EMPTY: u8 = 1;
const PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED: u8 = 4;
const PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT: u8 = 5;
const PILLAR_VOTE_STATUS_VALID: u8 = 0;
const PILLAR_VOTE_STATUS_NOT_UNIQUE: u8 = 5;
const PILLAR_VOTE_STATUS_SIGNATURE_INVALID: u8 = 6;
const PILLAR_VOTE_STATUS_NOT_ELIGIBLE: u8 = 7;
const PILLAR_VOTE_STATUS_INSPECTION_FAILURE: u8 = 9;
const PILLAR_VOTE_STATUS_UNKNOWN: u8 = 255;

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

/// Creates an empty Rust pillar-vote registry for the C++ pillar-vote shim.
#[cfg(test)]
pub fn create_pillar_votes_index() -> Box<BridgePillarVotes> {
    Box::new(BridgePillarVotes(PillarVotes::new()))
}

#[cfg(test)]
impl BridgePillarVotes {
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

impl BridgePillarChainRuntime {
    /// Prepares one pillar vote for admission through the runtime-owned
    /// pillar-vote index.
    pub fn pillar_chain_runtime_prepare_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
    ) -> Result<PillarVoteSingleAdmissionPreparePlan> {
        prepare_single_vote_admission(&self.votes, vote_rlp, context)
    }

    /// Applies one prepared pillar vote to the runtime-owned pillar-vote index.
    pub fn pillar_chain_runtime_apply_prepared_single_vote_admission(
        &mut self,
        input: PillarVoteSingleAdmissionApplyInput,
    ) -> Result<PillarVoteSingleAdmissionApplyPlan> {
        apply_prepared_single_vote_admission(&mut self.votes, input)
    }

    /// Applies one weighted RLP bundle to the runtime-owned pillar-vote index.
    pub fn pillar_chain_runtime_apply_weighted_rlp_bundle(
        &mut self,
        votes: Vec<PillarVoteWeightedRlpPayload>,
        expected_period: u64,
        expected_block_hash: &[u8; 32],
        threshold: u64,
    ) -> Result<PillarVoteBundleApplyPlan> {
        apply_weighted_rlp_bundle(
            &mut self.votes,
            votes,
            expected_period,
            expected_block_hash,
            threshold,
        )
    }

    /// Looks up Rust-retained vote payloads from the runtime-owned index.
    pub fn pillar_chain_runtime_get_verified_vote_payloads(
        &self,
        period: u64,
        block_hash: &[u8; 32],
        above_threshold: bool,
    ) -> PillarVotesPayloadLookup {
        self.votes
            .get_verified_votes(period, H256::from(*block_hash), above_threshold)
            .into()
    }

    /// Finalizes one pillar block for the PBFT finalization boundary.
    ///
    /// Inputs:
    /// - `storage` is the typed pillar-chain storage handle that owns durable
    ///   finalized pillar-block persistence.
    /// - `request` carries only current/last pillar sidecar facts and the
    ///   current block's canonical RLP from the temporary C++ materializer.
    ///
    /// Outputs:
    /// - Returns the deterministic pillar-finalization status plus selected
    ///   vote payloads when finalization or already-finalized replay succeeds.
    /// - Requests a network vote bundle through `should_request_votes` when
    ///   Rust detects missing threshold votes.
    ///
    /// Invariants and edge behavior:
    /// - Rust owns verified-vote lookup, finalization planning, storage
    ///   persistence, and vote cleanup ordering.
    /// - Storage is committed before in-memory vote cleanup. Cleanup is skipped
    ///   if persistence fails.
    /// - C++ still owns network requests, legacy `PillarVote` materialization,
    ///   event emission, and PBFT `PeriodData` payload assembly.
    pub fn pillar_chain_runtime_finalize_block_for_pbft(
        &mut self,
        request: PillarBlockFinalizationRequest,
    ) -> Result<PillarBlockFinalizationResult> {
        let requested_hash = H256::from(request.requested_pillar_block_hash);
        let current_hash = H256::from(request.current_hash);
        let lookup = if request.has_current_pillar_block && current_hash == requested_hash {
            self.votes.get_verified_votes(
                request.current_period.saturating_add(1),
                requested_hash,
                true,
            )
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
            has_current_pillar_block: request.has_current_pillar_block,
            current_period: request.current_period,
            current_hash,
            threshold_met: lookup.threshold_met,
            block_weight: lookup.block_weight,
            selected_weight: lookup.selected_weight,
            selected_vote_count,
            has_last_finalized_pillar_block: request.has_last_finalized_pillar_block,
            last_finalized_hash: H256::from(request.last_finalized_hash),
        });

        let mut persisted = false;
        let mut cleaned_votes = false;
        if plan.status == PillarBlockFinalizationStatus::Ready && plan.should_persist {
            save_finalized_pillar_block_storage(
                self.storage.as_ref(),
                plan.current_period,
                &request.current_block_rlp,
            )?;
            persisted = true;
            self.votes
                .erase_votes(plan.current_period.saturating_add(1));
            cleaned_votes = true;
        }

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

        Ok(PillarBlockFinalizationResult {
            status: plan.status.as_u8(),
            success,
            should_request_votes: plan.should_request_votes,
            persisted,
            cleaned_votes,
            should_emit: plan.should_emit,
            current_period: plan.current_period,
            block_weight: plan.block_weight,
            selected_weight: plan.selected_weight,
            selected_vote_count: plan.selected_vote_count,
            votes,
        })
    }
}

#[cfg(test)]
impl BridgePillarVotes {
    /// Prepares one pillar vote for validation or insertion without reading
    /// FinalChain or mutating aggregation state.
    ///
    /// Inputs:
    /// - `vote_rlp` is the canonical legacy PillarVote bytes.
    /// - `context` supplies local current-pillar facts and toggles for callers
    ///   that need relevance and identity uniqueness before the external DPoS
    ///   lookup. Local generated/reloaded votes can disable those checks and
    ///   still reuse Rust inspection.
    ///
    /// Outputs:
    /// - On status `0`, the recovered `(period, block_hash, vote_hash, voter)`
    ///   identity is ready for C++ to query FinalChain DPoS facts.
    /// - Non-zero statuses match the C++ validation enum and require no
    ///   further external lookup.
    ///
    /// Edge behavior:
    /// - Malformed RLP and bridge-domain invariant failures return inspection
    ///   failure instead of panicking.
    /// - Exact duplicates are detected before relevance checks when relevance
    ///   is requested, preserving the legacy logging order.
    pub fn pillar_votes_prepare_single_vote_admission(
        &self,
        vote_rlp: Vec<u8>,
        context: PillarVoteSingleAdmissionContext,
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

        if context.check_relevance {
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
                    current_pillar_block_period: context
                        .has_current_pillar_block
                        .then_some(context.current_pillar_block_period),
                    current_pillar_block_hash: context
                        .has_current_pillar_block
                        .then_some(H256::from(context.current_pillar_block_hash)),
                    first_pillar_block_period: context.first_pillar_block_period,
                    pillar_blocks_interval: context.pillar_blocks_interval,
                    vote_already_known: self.0.vote_exists(&duplicate_probe),
                })?;
            if !relevance.is_relevant {
                plan.status = relevance.status_code();
                return Ok(plan);
            }
        }

        if context.check_identity_uniqueness
            && !self.0.is_unique_vote_identity(ConsensusPillarVoteIdentity {
                period: inspection.period,
                vote_hash: inspection.vote_hash,
                voter: inspection.voter,
            })
        {
            plan.status = PILLAR_VOTE_STATUS_NOT_UNIQUE;
            return Ok(plan);
        }

        plan.needs_threshold = !self.0.period_data_initialized(inspection.period);
        plan.can_query_dpos = true;
        Ok(plan)
    }

    /// Applies one pillar vote after Rust preparation and the external
    /// FinalChain DPoS lookup have supplied validator weight and, when needed,
    /// period threshold.
    ///
    /// Inputs:
    /// - `input` carries canonical RLP, DPoS weight, and optional threshold.
    ///   C++ remains responsible for obtaining those external facts.
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
        if input.validator_vote_count == 0 {
            return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_NOT_ELIGIBLE));
        }

        let (vote, period) =
            match signed_rlp_to_verified_vote(input.vote_rlp, input.validator_vote_count) {
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

        if !self.0.period_data_initialized(period) {
            if !input.has_threshold {
                return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_UNKNOWN));
            }
            self.0.initialize_period_data(period, input.threshold);
        }

        let outcome = self.0.add_verified_vote(vote)?;
        Ok(PillarVoteSingleAdmissionApplyPlan {
            status: PILLAR_VOTE_STATUS_VALID,
            accepted: outcome.accepted,
            duplicate: outcome.duplicate,
            conflict_found: outcome.conflicting_vote_hash.is_some(),
            conflicting_vote_hash: outcome.conflicting_vote_hash.unwrap_or_default().into(),
            block_weight: outcome.block_weight,
        })
    }
}

/// Inspects a legacy-encoded PillarVote payload without mutating state.
///
/// Use this before inserting a vote to recover voter/address and check
/// signature validity from vote RLP alone.
pub fn pillar_vote_inspect(vote_rlp: &[u8]) -> Result<PillarVoteInspection> {
    Ok(inspect_pillar_vote_from_rlp(vote_rlp)?.into())
}

/// Inspects one batch of canonical pillar-vote bytes before C++ performs the
/// external FinalChain DPoS weight lookup.
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
pub fn inspect_pillar_vote_bundle_rlps(
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
pub fn plan_pillar_vote_relevance(
    fact: FfiPillarVoteRelevanceFact,
) -> Result<FfiPillarVoteRelevancePlan> {
    let fact = relevance_fact_to_consensus_fact(fact)?;
    Ok(FfiPillarVoteRelevancePlan::from(
        rustaxa_consensus::plan_pillar_vote_relevance(fact)?,
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
    }
}

fn prepare_single_vote_admission(
    pillar_votes: &PillarVotes,
    vote_rlp: Vec<u8>,
    context: PillarVoteSingleAdmissionContext,
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

    if context.check_relevance {
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
                current_pillar_block_period: context
                    .has_current_pillar_block
                    .then_some(context.current_pillar_block_period),
                current_pillar_block_hash: context
                    .has_current_pillar_block
                    .then_some(H256::from(context.current_pillar_block_hash)),
                first_pillar_block_period: context.first_pillar_block_period,
                pillar_blocks_interval: context.pillar_blocks_interval,
                vote_already_known: pillar_votes.vote_exists(&duplicate_probe),
            })?;
        if !relevance.is_relevant {
            plan.status = relevance.status_code();
            return Ok(plan);
        }
    }

    if context.check_identity_uniqueness
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
    input: PillarVoteSingleAdmissionApplyInput,
) -> Result<PillarVoteSingleAdmissionApplyPlan> {
    if input.validator_vote_count == 0 {
        return Ok(single_admission_apply_plan(PILLAR_VOTE_STATUS_NOT_ELIGIBLE));
    }

    let (vote, period) =
        match signed_rlp_to_verified_vote(input.vote_rlp, input.validator_vote_count) {
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
    use crate::pillar_chain::{create_pillar_chain_runtime, create_pillar_chain_storage};
    use crate::storage::create_storage;
    use ethereum_types::H160;
    use k256::ecdsa::SigningKey;
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
        let mut vote = PillarVote {
            period,
            block_hash: H256::from_low_u64_be(block),
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

    fn single_admission_context(block_hash: H256) -> PillarVoteSingleAdmissionContext {
        PillarVoteSingleAdmissionContext {
            has_current_pillar_block: true,
            current_pillar_block_period: 41,
            current_pillar_block_hash: block_hash.into(),
            first_pillar_block_period: 40,
            pillar_blocks_interval: 10,
            check_relevance: true,
            check_identity_uniqueness: true,
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
                    vote_rlp: vote.encode_rlp(),
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
    fn pillar_chain_runtime_finalizes_block_for_pbft_with_owned_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_finalization");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let mut runtime = create_pillar_chain_runtime(&storage);
            let pillar_storage = create_pillar_chain_storage(&storage);
            let (vote, _) = signed_vote(0x24, 42, 77);
            let block_rlp = vec![0xC1, 0x03];

            let applied = runtime
                .pillar_chain_runtime_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_rlp: vote.encode_rlp(),
                        validator_vote_count: 6,
                        has_threshold: true,
                        threshold: 5,
                    },
                )
                .expect("vote should apply");
            assert_eq!(applied.status, PILLAR_VOTE_STATUS_VALID);
            assert!(applied.accepted);

            let finalized = runtime
                .pillar_chain_runtime_finalize_block_for_pbft(PillarBlockFinalizationRequest {
                    requested_pillar_block_hash: vote.block_hash.into(),
                    has_current_pillar_block: true,
                    current_period: 41,
                    current_hash: vote.block_hash.into(),
                    current_block_rlp: block_rlp.clone(),
                    has_last_finalized_pillar_block: false,
                    last_finalized_hash: [0; 32],
                })
                .expect("pillar finalization should run");

            assert_eq!(finalized.status, 0);
            assert!(finalized.success);
            assert!(finalized.persisted);
            assert!(finalized.cleaned_votes);
            assert!(finalized.should_emit);
            assert_eq!(finalized.selected_vote_count, 1);
            assert_eq!(finalized.votes.len(), 1);
            assert_eq!(
                finalized.votes[0].vote_hash,
                Into::<[u8; 32]>::into(vote.hash(true))
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_block(41)
                    .expect("finalized block should load"),
                block_rlp,
            );
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
                .pillar_votes_prepare_single_vote_admission(
                    vote.encode_rlp(),
                    PillarVoteSingleAdmissionContext {
                        has_current_pillar_block: false,
                        current_pillar_block_period: 0,
                        current_pillar_block_hash: [0; 32],
                        first_pillar_block_period: 50,
                        pillar_blocks_interval: 10,
                        check_relevance: false,
                        check_identity_uniqueness: false,
                    },
                )
                .unwrap();
            assert!(prepared.can_query_dpos);
            assert_eq!(prepared.voter, voter);
            let applied = votes
                .pillar_votes_apply_prepared_single_vote_admission(
                    PillarVoteSingleAdmissionApplyInput {
                        vote_rlp: vote.encode_rlp(),
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

        let applied = votes
            .pillar_votes_apply_prepared_single_vote_admission(
                PillarVoteSingleAdmissionApplyInput {
                    vote_rlp: vote.encode_rlp(),
                    validator_vote_count: 4,
                    has_threshold: true,
                    threshold: 5,
                },
            )
            .unwrap();

        assert_eq!(applied.status, PILLAR_VOTE_STATUS_SIGNATURE_INVALID);
        assert!(!applied.accepted);
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
}
