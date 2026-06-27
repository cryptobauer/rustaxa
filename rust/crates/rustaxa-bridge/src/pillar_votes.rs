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
    PillarVoteBundleApplyPlan, PillarVoteBundleInspectionPlan, PillarVoteIdentityPayload,
    PillarVoteInsertOutcome, PillarVoteInspection, PillarVotePayload, PillarVoteRecord,
    PillarVoteRelevanceFact as FfiPillarVoteRelevanceFact,
    PillarVoteRelevancePlan as FfiPillarVoteRelevancePlan, PillarVoteRlpPayload,
    PillarVoteUniqueOutcome, PillarVoteWeightedRlpPayload, PillarVotesPayloadLookup,
};
use crate::ffi::BridgePillarVotes;
use anyhow::{ensure, Result};
use ethereum_types::{H160, H256};
use rustaxa_consensus::{
    inspect_pillar_vote_from_rlp, PillarVoteBundlePlanner,
    PillarVoteFact as ConsensusPillarVoteFact, PillarVoteIdentity as ConsensusPillarVoteIdentity,
    PillarVoteInsertOutcome as ConsensusPillarVoteInsertOutcome,
    PillarVoteInspection as ConsensusPillarVoteInspection,
    PillarVoteRelevanceFact as ConsensusPillarVoteRelevanceFact,
    PillarVoteRelevancePlan as ConsensusPillarVoteRelevancePlan, PillarVotes, VerifiedPillarVote,
};
use rustaxa_types::PillarVote;
use std::collections::HashMap;

const PILLAR_VOTE_BUNDLE_STATUS_VALID: u8 = 0;
const PILLAR_VOTE_BUNDLE_STATUS_EMPTY: u8 = 1;
const PILLAR_VOTE_BUNDLE_STATUS_PREVALIDATION_FAILED: u8 = 4;
const PILLAR_VOTE_BUNDLE_STATUS_ZERO_WEIGHT: u8 = 5;

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
pub fn create_pillar_votes_index() -> Box<BridgePillarVotes> {
    Box::new(BridgePillarVotes(PillarVotes::new()))
}

impl BridgePillarVotes {
    /// Returns whether threshold/vote state exists for `period`.
    pub fn pillar_votes_period_data_initialized(&self, period: u64) -> bool {
        self.0.period_data_initialized(period)
    }

    /// Initializes period-wide threshold data.
    ///
    /// The first initialization for `period` wins; existing state is unchanged
    /// for repeated calls with the same period.
    pub fn pillar_votes_init_period_data(&mut self, period: u64, threshold: u64) -> bool {
        self.0.initialize_period_data(period, threshold)
    }

    /// Checks exact `(period, block_hash, vote_hash)` membership.
    pub fn pillar_votes_vote_exists(&self, vote: PillarVotePayload) -> Result<bool> {
        let vote = payload_to_vote(vote)?;
        Ok(self.0.vote_exists(&vote))
    }

    /// Checks whether a vote is unique for `(period, voter)` without mutating state.
    pub fn pillar_votes_is_unique_vote(
        &self,
        vote: PillarVotePayload,
    ) -> Result<PillarVoteUniqueOutcome> {
        let vote = payload_to_vote(vote)?;
        Ok(PillarVoteUniqueOutcome {
            is_unique: self.0.is_unique_vote(&vote),
        })
    }

    /// Inserts one verified pillar vote and returns deterministic aggregate state.
    pub fn pillar_votes_insert_vote(
        &mut self,
        vote: PillarVotePayload,
    ) -> Result<PillarVoteInsertOutcome> {
        let vote = payload_to_vote(vote)?;
        Ok(self.0.add_verified_vote(vote)?.into())
    }

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

    /// Checks whether a recovered vote identity is unique before weight lookup.
    pub fn pillar_votes_is_unique_identity(
        &self,
        vote: PillarVoteIdentityPayload,
    ) -> Result<PillarVoteUniqueOutcome> {
        Ok(PillarVoteUniqueOutcome {
            is_unique: self
                .0
                .is_unique_vote_identity(identity_payload_to_consensus(vote)),
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

fn identity_payload_to_consensus(value: PillarVoteIdentityPayload) -> ConsensusPillarVoteIdentity {
    ConsensusPillarVoteIdentity {
        period: value.period,
        vote_hash: H256::from(value.vote_hash),
        voter: H160::from(value.voter),
    }
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
        H160::from(value.voter),
        value.weight,
    )
}

impl From<ConsensusPillarVoteInsertOutcome> for PillarVoteInsertOutcome {
    fn from(value: ConsensusPillarVoteInsertOutcome) -> Self {
        Self {
            accepted: value.accepted,
            duplicate: value.duplicate,
            conflicting_vote_hash: value.conflicting_vote_hash.unwrap_or_default().into(),
            block_weight: value.block_weight,
            conflict_found: value.conflicting_vote_hash.is_some(),
        }
    }
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
    use k256::ecdsa::SigningKey;

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

    #[test]
    fn insert_vote_accepts_votes_and_tracks_weight() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(10, 10));

        let first = vote(10, 11, 1, 0xAA, 4);
        let second = vote(10, 11, 2, 0xAB, 6);

        let first_outcome = votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        let second_outcome = votes.pillar_votes_insert_vote(second).unwrap();

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
        assert!(votes.pillar_votes_init_period_data(11, 1));

        let first = vote(11, 12, 1, 0xAC, 6);
        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        let duplicate = votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();

        assert!(duplicate.accepted);
        assert!(duplicate.duplicate);
        assert!(!duplicate.conflict_found);
        assert_eq!(duplicate.block_weight, 6);

        let lookup = votes.pillar_votes_get_verified_vote_payloads(11, &first.block_hash, false);
        assert_eq!(lookup.votes.len(), 1);
        assert_eq!(lookup.votes[0].vote_hash, first.vote_hash);
    }

    #[test]
    fn unique_vote_rejects_conflicting_voter() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(12, 1));

        let first = vote(12, 13, 1, 0xB0, 5);
        let conflict = vote(12, 14, 1, 0xB1, 5);

        let inserted = votes.pillar_votes_insert_vote(first).unwrap();
        assert!(inserted.accepted);

        let unique = votes.pillar_votes_is_unique_vote(conflict).unwrap();
        assert!(!unique.is_unique);
    }

    #[test]
    fn vote_exists_and_period_initialized_mirror_registry_state() {
        let mut votes = create_pillar_votes_index();
        let first = vote(12, 13, 1, 0xAF, 5);

        assert!(!votes.pillar_votes_period_data_initialized(12));
        votes.pillar_votes_init_period_data(12, 1);
        assert!(votes.pillar_votes_period_data_initialized(12));
        assert!(!votes
            .pillar_votes_vote_exists(clone_payload(&first))
            .unwrap());

        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        assert!(votes.pillar_votes_vote_exists(first).unwrap());
    }

    #[test]
    fn above_threshold_lookup_selects_minimum_prefix_when_met() {
        let mut votes = create_pillar_votes_index();
        assert!(votes.pillar_votes_init_period_data(13, 7));

        let low = vote(13, 15, 1, 0xC0, 1);
        let mid = vote(13, 15, 2, 0xC1, 3);
        let high = vote(13, 15, 3, 0xC2, 4);
        votes.pillar_votes_insert_vote(low).unwrap();
        votes.pillar_votes_insert_vote(clone_payload(&mid)).unwrap();
        votes
            .pillar_votes_insert_vote(clone_payload(&high))
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
        assert!(votes.pillar_votes_init_period_data(14, 10));

        let first = vote(14, 16, 1, 0xD0, 4);
        let second = vote(14, 16, 2, 0xD1, 5);
        votes
            .pillar_votes_insert_vote(clone_payload(&first))
            .unwrap();
        votes
            .pillar_votes_insert_vote(clone_payload(&second))
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
            assert!(votes.pillar_votes_init_period_data(period, 1));
            votes
                .pillar_votes_insert_vote(vote(
                    period,
                    20,
                    period,
                    (period as u8).wrapping_add(0x10),
                    1,
                ))
                .unwrap();
        }

        votes.pillar_votes_cleanup_votes_by_period(22);

        assert!(votes
            .pillar_votes_insert_vote(vote(20, 20, 30, 0xE0, 1))
            .is_err());
        assert!(votes
            .pillar_votes_is_unique_vote(vote(22, 20, 22, 0xE2, 1))
            .is_ok());
        assert!(votes
            .pillar_votes_is_unique_vote(vote(22, 20, 23, 0xE3, 1))
            .is_ok());
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
