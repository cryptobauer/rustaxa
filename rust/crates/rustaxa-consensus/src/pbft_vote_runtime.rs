//! PBFT vote admission runtime for Rust-owned vote state and payloads.
//!
//! This module is the stateful companion to the side-effect-free PBFT vote
//! admission and progress planners. It owns the verified-vote index plus the
//! canonical/weighted payload sidecar needed by storage and slashing effects.
//! Callers still supply FinalChain/key validation facts and execute returned
//! side effects at the boundary; the runtime owns the deterministic mutation
//! ordering and the payload bytes derived from the admitted canonical vote.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow};
use ethereum_types::H256;

use crate::pbft_vote_admission::{
    PbftVoteAdmissionExecution, PbftVoteAdmissionPrecheck, PbftVoteAdmissionSession,
};
use crate::pbft_vote_event::PbftVoteEventFactFlags;
use crate::pbft_vote_payload::{
    PbftVotePayloadRecord, build_slashing_pbft_vote_payload, build_weighted_pbft_vote_bundle,
    build_weighted_pbft_vote_payload,
};
use crate::pbft_vote_progress::PbftVoteProgressContext;
use crate::pbft_vote_validation::PbftCanonicalVoteValidation;
use crate::verified_votes::{AddVerifiedVoteOutcome, VerifiedVote, VerifiedVotes, VotesWithWeight};

/// Canonical and weighted PBFT vote payloads retained for one admitted vote.
///
/// `slashing` is the unweighted signed vote payload used for double-vote proof
/// calldata. `weighted` is the storage payload used for own votes, extra reward
/// votes, and 2t+1 vote bundles. Both records use the same canonical vote hash.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimePayload {
    /// Unweighted signed PBFT vote payload for slashing evidence.
    pub slashing: PbftVotePayloadRecord,
    /// Weighted PBFT vote payload for storage.
    pub weighted: PbftVotePayloadRecord,
}

/// Runtime-built 2t+1 vote bundle ready for storage persistence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeBundle {
    /// Number of weighted vote records included in the bundle.
    pub votes_count: usize,
    /// Raw legacy RLP list of weighted vote records.
    pub votes_bundle_rlp: Vec<u8>,
}

/// Runtime-built slashing payload pair for a duplicate-voter conflict.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeSlashingPayloads {
    /// Incoming conflicting vote payload.
    pub incoming: PbftVotePayloadRecord,
    /// Existing conflicting vote payload from the runtime sidecar.
    pub conflicting: PbftVotePayloadRecord,
}

/// Complete result of one validation-backed vote admission transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeAdmissionOutcome {
    /// Pre-mutation planner output, including validation and progress facts.
    pub precheck: PbftVoteAdmissionPrecheck,
    /// Terminal execution plan after the runtime applied the verified-vote
    /// mutation, when a mutation was requested.
    pub execution: Option<PbftVoteAdmissionExecution>,
    /// Verified-vote insertion report, when one was applied.
    pub add_outcome: Option<AddVerifiedVoteOutcome>,
    /// Weighted storage record for this vote, when the vote was accepted and
    /// payload construction succeeded.
    pub storage_vote: Option<PbftVotePayloadRecord>,
    /// Weighted 2t+1 vote bundle, when threshold progress requested bundle
    /// persistence and all selected payload sidecars were present.
    pub two_t_plus_one_bundle: Option<PbftVoteRuntimeBundle>,
    /// Unweighted slashing evidence payloads, when a duplicate-voter conflict
    /// was reported and both payloads were available.
    pub slashing_payloads: Option<PbftVoteRuntimeSlashingPayloads>,
}

/// Rust-owned PBFT vote admission state.
///
/// The runtime owns deterministic verified-vote metadata and the byte payloads
/// needed after admission. It does not own live C++ `PbftVote` objects, storage
/// handles, network peers, or transaction submission.
#[derive(Debug, Clone, Default)]
pub struct PbftVoteAdmissionRuntime {
    verified_votes: VerifiedVotes,
    payloads: BTreeMap<H256, PbftVoteRuntimePayload>,
}

impl PbftVoteAdmissionRuntime {
    /// Creates an empty PBFT vote admission runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns immutable access to the Rust verified-vote index.
    #[must_use]
    pub const fn verified_votes(&self) -> &VerifiedVotes {
        &self.verified_votes
    }

    /// Returns mutable access to the Rust verified-vote index.
    pub const fn verified_votes_mut(&mut self) -> &mut VerifiedVotes {
        &mut self.verified_votes
    }

    /// Returns the weighted storage payload for `vote_hash`, when retained.
    #[must_use]
    pub fn weighted_payload(&self, vote_hash: H256) -> Option<&PbftVotePayloadRecord> {
        self.payloads
            .get(&vote_hash)
            .map(|payload| &payload.weighted)
    }

    /// Returns the slashing payload for `vote_hash`, when retained.
    #[must_use]
    pub fn slashing_payload(&self, vote_hash: H256) -> Option<&PbftVotePayloadRecord> {
        self.payloads
            .get(&vote_hash)
            .map(|payload| &payload.slashing)
    }

    /// Removes votes and payloads for periods older than `pbft_period`.
    ///
    /// This mirrors the existing verified-vote cleanup semantics and prunes the
    /// payload sidecar from the post-cleanup vote snapshot so payloads cannot
    /// outlive their consensus metadata.
    pub fn cleanup_votes_by_period(&mut self, pbft_period: u64) {
        self.verified_votes.cleanup_votes_by_period(pbft_period);
        let keep: std::collections::BTreeSet<_> = self
            .verified_votes
            .snapshot_votes()
            .into_iter()
            .map(|vote| vote.vote_hash)
            .collect();
        self.payloads
            .retain(|vote_hash, _| keep.contains(vote_hash));
    }

    /// Admits one validation-backed canonical PBFT vote into Rust-owned state.
    ///
    /// Inputs:
    /// - `canonical_vote_rlp`: legacy signed vote bytes supplied by the
    ///   boundary.
    /// - `validation`: accepted or rejected validation output for the same
    ///   canonical vote bytes.
    /// - `flags`: ingress/reward flags used by the vote-progress planner.
    /// - `context`: scalar PBFT vote-progress context, including optional
    ///   threshold and slashing enablement.
    ///
    /// Outputs:
    /// - Precheck/execution plans matching the existing session contract.
    /// - Rust-owned storage and slashing payloads for any requested side
    ///   effects.
    ///
    /// Edge behavior:
    /// - Rejected validation returns the terminal precheck without mutating
    ///   verified-vote state or payload sidecars.
    /// - Duplicate-voter conflicts do not insert the incoming vote, but still
    ///   return incoming and existing slashing payloads when available.
    /// - Missing retained payloads for a Rust-selected 2t+1 bundle or conflict
    ///   are hard errors because that indicates runtime state corruption.
    pub fn admit_validated_vote(
        &mut self,
        canonical_vote_rlp: &[u8],
        validation: &PbftCanonicalVoteValidation,
        flags: PbftVoteEventFactFlags,
        context: PbftVoteProgressContext,
    ) -> Result<PbftVoteRuntimeAdmissionOutcome> {
        let mut session = PbftVoteAdmissionSession::from_validation(validation, flags, context);
        let precheck = session.precheck();
        if !precheck.should_insert() {
            return Ok(PbftVoteRuntimeAdmissionOutcome {
                precheck,
                execution: None,
                add_outcome: None,
                storage_vote: None,
                two_t_plus_one_bundle: None,
                slashing_payloads: None,
            });
        }

        let fact = precheck.progress_fact.ok_or_else(|| {
            anyhow!("PBFT vote runtime precheck requested insert without progress fact")
        })?;
        let storage_vote = build_weighted_pbft_vote_payload(canonical_vote_rlp, fact.weight)?;
        let slashing_vote = build_slashing_pbft_vote_payload(canonical_vote_rlp)?;
        if storage_vote.hash != fact.identity.vote_hash
            || slashing_vote.hash != fact.identity.vote_hash
        {
            return Err(anyhow!(
                "PBFT vote runtime payload hash mismatches progress fact"
            ));
        }

        let vote = VerifiedVote::new(
            fact.identity.vote_hash,
            fact.identity.block_hash,
            fact.identity.voter,
            fact.identity.period,
            fact.identity.round,
            fact.identity.step,
            fact.vote_type,
            fact.weight,
        )?;
        let add_outcome = self
            .verified_votes
            .add_verified_vote(vote, context.two_t_plus_one_threshold)?;

        let execution = session.report_verified_vote_add(add_outcome);
        let mut retained_storage_vote = None;
        if add_outcome.inserted {
            self.payloads.insert(
                fact.identity.vote_hash,
                PbftVoteRuntimePayload {
                    slashing: slashing_vote.clone(),
                    weighted: storage_vote.clone(),
                },
            );
            retained_storage_vote = Some(storage_vote.clone());
        }

        let slashing_payloads = if let Some(conflicting_vote_hash) =
            add_outcome.conflicting_vote_hash
        {
            let conflicting = self
                    .slashing_payload(conflicting_vote_hash)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "PBFT vote runtime missing slashing payload for conflicting vote {conflicting_vote_hash:#x}"
                        )
                    })?;
            Some(PbftVoteRuntimeSlashingPayloads {
                incoming: slashing_vote,
                conflicting,
            })
        } else {
            None
        };

        let two_t_plus_one_bundle = if execution
            .pipeline_step
            .progress_plan
            .threshold_decision
            .is_some_and(|decision| {
                decision
                    .two_t_plus_one_insert_outcome
                    .is_some_and(|outcome| outcome.inserted)
            }) {
            let decision = execution
                .pipeline_step
                .progress_plan
                .threshold_decision
                .expect("checked above");
            let kind = decision
                .two_t_plus_one_kind
                .ok_or_else(|| anyhow!("PBFT vote runtime threshold inserted without kind"))?;
            let vote_hashes = self
                .verified_votes
                .get_two_t_plus_one_voted_block_vote_hashes(
                    fact.identity.period,
                    fact.identity.round,
                    kind,
                );
            let mut records = Vec::with_capacity(vote_hashes.len());
            for vote_hash in vote_hashes {
                records.push(self.weighted_payload(vote_hash).cloned().ok_or_else(|| {
                    anyhow!(
                        "PBFT vote runtime missing weighted payload for 2t+1 vote {vote_hash:#x}"
                    )
                })?);
            }
            Some(PbftVoteRuntimeBundle {
                votes_count: records.len(),
                votes_bundle_rlp: build_weighted_pbft_vote_bundle(&records)?,
            })
        } else {
            None
        };

        Ok(PbftVoteRuntimeAdmissionOutcome {
            precheck,
            execution: Some(execution),
            add_outcome: Some(add_outcome),
            storage_vote: retained_storage_vote,
            two_t_plus_one_bundle,
            slashing_payloads,
        })
    }

    /// Returns the vote bucket for a successfully inserted vote.
    ///
    /// This helper is used by the temporary C++ sidecar facade to reconstruct
    /// legacy `VotesWithWeight` objects from Rust metadata while the public C++
    /// API still returns live `PbftVote` pointers.
    #[must_use]
    pub fn inserted_votes_with_weight(&self, vote: &VerifiedVote) -> Option<VotesWithWeight> {
        self.verified_votes
            .get_step_votes(vote.period, vote.round, vote.step)?;
        self.verified_votes
            .votes_with_weight(vote.period, vote.round, vote.step, vote.block_hash)
    }
}

trait RuntimePrecheckExt {
    fn should_insert(&self) -> bool;
}

impl RuntimePrecheckExt for PbftVoteAdmissionPrecheck {
    fn should_insert(&self) -> bool {
        self.pipeline_step.as_ref().is_some_and(|step| {
            step.progress_plan.status
                == crate::pbft_vote_progress::PbftVoteProgressStatus::PendingVerifiedVoteInsert
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_event::PbftVoteEventFactFlags;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_validation::{
        PbftVoteValidationExternalFacts, validate_canonical_pbft_vote,
    };
    use crate::verified_votes::PbftVoteType;
    use k256::ecdsa::SigningKey;
    use rustaxa_vdf::vrf;
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

    fn voter_from_secret(secret: &[u8; 32]) -> [u8; 20] {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        output[12..].try_into().unwrap()
    }

    fn vote_rlp_from_secret(block_hash: [u8; 32], step: u64, node_secret: [u8; 32]) -> Vec<u8> {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Cert,
            period: 12,
            round: 2,
            step,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret).into(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap()
        .vote_rlp
    }

    fn vote_rlp(block_hash: [u8; 32], step: u64) -> Vec<u8> {
        vote_rlp_from_secret(block_hash, step, NODE_SECRET)
    }

    fn validation(vote_rlp: &[u8]) -> PbftCanonicalVoteValidation {
        validate_canonical_pbft_vote(
            vote_rlp,
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
            },
        )
        .unwrap()
    }

    fn flags() -> PbftVoteEventFactFlags {
        PbftVoteEventFactFlags {
            vote_already_known: false,
            carries_proposed_block: true,
            valid_stale_reward_vote: false,
        }
    }

    fn context(threshold: Option<u64>) -> PbftVoteProgressContext {
        PbftVoteProgressContext {
            current_period: 12,
            max_future_period_delta: 0,
            two_t_plus_one_threshold: threshold,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    #[test]
    fn runtime_admits_vote_and_retains_payloads() {
        let rlp = vote_rlp([1; 32], 3);
        let validation = validation(&rlp);
        let mut runtime = PbftVoteAdmissionRuntime::new();

        let outcome = runtime
            .admit_validated_vote(&rlp, &validation, flags(), context(Some(50)))
            .unwrap();

        assert!(
            outcome
                .execution
                .unwrap()
                .pipeline_step
                .progress_plan
                .status
                == crate::pbft_vote_progress::PbftVoteProgressStatus::Accepted
        );
        assert!(outcome.storage_vote.is_some());
        assert!(runtime.weighted_payload(validation.vote_hash).is_some());
        assert!(runtime.slashing_payload(validation.vote_hash).is_some());
    }

    #[test]
    fn runtime_builds_two_t_plus_one_bundle_from_retained_payloads() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([2; 32], 3);
        let second = vote_rlp_from_secret([2; 32], 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);

        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        let outcome = runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();

        let bundle = outcome.two_t_plus_one_bundle.unwrap();
        assert_eq!(bundle.votes_count, 2);
        assert!(!bundle.votes_bundle_rlp.is_empty());
    }

    #[test]
    fn runtime_returns_slashing_payloads_for_conflict() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let first = vote_rlp([4; 32], 3);
        let second = vote_rlp([5; 32], 3);
        let first_validation = validation(&first);
        let second_validation = validation(&second);

        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(100)))
            .unwrap();
        let outcome = runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(100)))
            .unwrap();

        let slashing = outcome.slashing_payloads.unwrap();
        assert_eq!(slashing.conflicting.hash, first_validation.vote_hash);
        assert_eq!(slashing.incoming.hash, second_validation.vote_hash);
    }
}
