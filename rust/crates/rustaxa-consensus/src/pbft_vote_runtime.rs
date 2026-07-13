//! PBFT vote admission runtime for Rust-owned vote state and payloads.
//!
//! This module is the stateful companion to the side-effect-free PBFT vote
//! admission and progress planners. It owns the verified-vote index plus the
//! canonical/weighted payload sidecar needed by storage and slashing effects.
//! Callers still supply FinalChain/key validation facts and execute returned
//! side effects at the boundary; the runtime owns the deterministic mutation
//! ordering and the payload bytes derived from the admitted canonical vote.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_storage::Storage;

use crate::pbft_reward_votes::{
    PbftRewardVoteRoundCandidate, PbftRewardVoteSelectionFact, PbftRewardVoteSelectionPlan,
    PbftRewardVotesStatus, plan_pbft_reward_votes,
};
use crate::pbft_thresholds::{
    PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdPlan, PbftTwoTPlusOneThresholdRuntime,
};
use crate::pbft_vote_admission::{
    PbftVoteAdmissionExecution, PbftVoteAdmissionPrecheck, PbftVoteAdmissionSession,
};
use crate::pbft_vote_event::PbftVoteEventFactFlags;
use crate::pbft_vote_payload::{
    PbftVotePayloadRecord, build_slashing_pbft_vote_payload, build_weighted_pbft_vote_bundle,
    build_weighted_pbft_vote_payload,
};
use crate::pbft_vote_progress::PbftVoteProgressContext;
use crate::pbft_vote_validation::{
    PbftCanonicalVoteInspection, PbftCanonicalVoteInspectionStatus, PbftCanonicalVoteValidation,
    PbftVoteReplayCache, inspect_canonical_pbft_vote,
};
use crate::verified_votes::{
    AddVerifiedVoteOutcome, TwoTPlusOneVotedBlockType, VerifiedVote, VerifiedVotes, VotesWithWeight,
};

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

/// Compact startup report produced while restoring verified-vote state.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeRestoreSnapshot {
    /// Extra reward-vote hashes in deterministic hash order.
    ///
    /// Payloads and own-vote membership are queried from their authoritative
    /// Rust runtime/storage owners instead of being copied into this report.
    pub extra_reward_vote_hashes: Vec<H256>,
    /// Whether a cert `2t+1` bundle supplied reward-vote coordinates.
    pub has_reward_vote_info: bool,
    /// Reward-vote PBFT period when present.
    pub reward_vote_period: u64,
    /// Reward-vote PBFT round when present.
    pub reward_vote_round: u64,
    /// Reward-voted PBFT block hash when present.
    pub reward_vote_block_hash: H256,
}

#[derive(Debug, Clone)]
struct RestoreVoteSource {
    vote_rlp: Vec<u8>,
    extra_reward_vote: bool,
}

fn inspect_restored_weighted_vote(vote_rlp: &[u8]) -> Result<PbftCanonicalVoteInspection> {
    let inspection = inspect_canonical_pbft_vote(vote_rlp)?;
    ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid && inspection.signature_valid,
        "stored verified vote is malformed or has an invalid signature"
    );
    ensure!(
        inspection.has_embedded_weight && inspection.embedded_weight > 0,
        "stored verified vote must contain a non-zero embedded weight"
    );
    Ok(inspection)
}

fn validate_restored_bundle_kind(
    kind: TwoTPlusOneVotedBlockType,
    vote: &PbftCanonicalVoteInspection,
) -> Result<()> {
    let valid = match kind {
        TwoTPlusOneVotedBlockType::SoftVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Soft
        }
        TwoTPlusOneVotedBlockType::CertVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Cert
        }
        TwoTPlusOneVotedBlockType::NextVotedBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Next
                && !vote.block_hash.is_zero()
        }
        TwoTPlusOneVotedBlockType::NextVotedNullBlock => {
            vote.vote_type == crate::verified_votes::PbftVoteType::Next && vote.block_hash.is_zero()
        }
    };
    ensure!(
        valid,
        "stored 2t+1 vote does not match bundle category {kind:?}"
    );
    Ok(())
}

fn merge_restore_source(
    sources: &mut BTreeMap<H256, RestoreVoteSource>,
    vote_hash: H256,
    vote_rlp: Vec<u8>,
    extra_reward_vote: bool,
) -> Result<()> {
    if let Some(existing) = sources.get_mut(&vote_hash) {
        ensure!(
            existing.vote_rlp == vote_rlp,
            "stored verified vote hash has inconsistent weighted payloads"
        );
        existing.extra_reward_vote |= extra_reward_vote;
    } else {
        sources.insert(
            vote_hash,
            RestoreVoteSource {
                vote_rlp,
                extra_reward_vote,
            },
        );
    }
    Ok(())
}

/// Runtime-built 2t+1 vote bundle ready for storage persistence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeBundle {
    /// Threshold family whose vote bundle reached `2t+1`.
    pub kind: TwoTPlusOneVotedBlockType,
    /// PBFT period for the persisted bundle.
    pub period: u64,
    /// PBFT round for the persisted bundle.
    pub round: u64,
    /// PBFT step of the vote that triggered persistence.
    pub step: u64,
    /// Block hash selected by the threshold family.
    pub block_hash: H256,
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

/// Rust-owned PBFT reward-vote selection with retained weighted payloads.
///
/// The selection mirrors legacy reward-vote lookup order while keeping the
/// candidate construction and payload resolution under the vote admission
/// runtime that owns verified-vote metadata and retained bytes.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftRewardVotePayloadSelection {
    /// True when the PBFT block reward-vote references are accepted.
    pub accepted: bool,
    /// Stable reward-vote selection status.
    pub status: PbftRewardVotesStatus,
    /// Reward period used for lookup.
    pub selected_period: u64,
    /// Round that satisfied the requested vote hashes.
    pub selected_round: u64,
    /// Reward block hash used for lookup.
    pub selected_block_hash: H256,
    /// Requested vote hashes in PBFT-block order when accepted.
    pub selected_vote_hashes: Vec<H256>,
    /// Retained weighted records in the same order as `selected_vote_hashes`.
    pub selected_records: Vec<PbftVotePayloadRecord>,
    /// First missing vote hash when selection failed or payload retention is incomplete.
    pub missing_vote_hash: Option<H256>,
}

/// Runtime replay-cache result for one validation or admission transition.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftVoteRuntimeReplayOutcome {
    /// Whether validation reached a state that should be replay-protected.
    pub should_mark: bool,
    /// Whether the runtime inserted a new replay-cache entry.
    pub inserted: bool,
    /// Whether the replay entry already existed before this transition.
    pub already_present: bool,
}

/// Complete result of one validation-backed vote admission transition.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteRuntimeAdmissionOutcome {
    /// Pre-mutation planner output, including validation and progress facts.
    pub precheck: PbftVoteAdmissionPrecheck,
    /// Replay-cache mutation result for this transition.
    pub replay: PbftVoteRuntimeReplayOutcome,
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
#[derive(Debug, Clone)]
pub struct PbftVoteAdmissionRuntime {
    verified_votes: VerifiedVotes,
    replay_cache: PbftVoteReplayCache,
    threshold_runtime: PbftTwoTPlusOneThresholdRuntime,
    payloads: BTreeMap<H256, PbftVoteRuntimePayload>,
}

impl Default for PbftVoteAdmissionRuntime {
    fn default() -> Self {
        Self::new_with_replay_cache(1_000_000, 1_000)
    }
}

impl PbftVoteAdmissionRuntime {
    /// Creates an empty PBFT vote admission runtime.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty PBFT vote admission runtime with explicit replay-cache
    /// bounds.
    ///
    /// Inputs:
    /// - `replay_max_size`: maximum retained validated vote hashes.
    /// - `replay_delete_step`: number of oldest hashes removed after capacity
    ///   is crossed.
    ///
    /// Outputs:
    /// - Runtime owning verified-vote state, replay protection, threshold
    ///   cache, and retained vote payload sidecars.
    #[must_use]
    pub fn new_with_replay_cache(replay_max_size: usize, replay_delete_step: usize) -> Self {
        Self {
            verified_votes: VerifiedVotes::new(),
            replay_cache: PbftVoteReplayCache::new(replay_max_size, replay_delete_step),
            threshold_runtime: PbftTwoTPlusOneThresholdRuntime::new(),
            payloads: BTreeMap::new(),
        }
    }

    /// Restores the authoritative vote runtime directly from native Rust storage.
    ///
    /// The restore reads own votes, extra reward votes, and each typed latest-round
    /// `2t+1` slot. Weighted payloads are decoded and signature-checked, overlaps
    /// are deduplicated by canonical vote hash, and contradictory bytes for one
    /// hash are rejected. Typed bundles must contain votes with identical
    /// coordinates and a vote type/block-nullness matching their storage key.
    ///
    /// The returned snapshot contains only extra-reward hashes and cert-bundle
    /// reward coordinates still required by startup compatibility wiring.
    /// Runtime metadata, payload retention, uniqueness indexes, and `2t+1`
    /// mappings are all reconstructed here before the runtime is returned.
    pub fn restore_from_storage(
        storage: &Storage,
    ) -> Result<(Self, PbftVoteRuntimeRestoreSnapshot)> {
        let own_votes = storage
            .pbft()
            .own_verified_vote_records()
            .context("VERIFIED_VOTES_RESTORE_OWN_READ")?;
        let reward_votes = storage
            .pbft()
            .reward_votes_rlp()
            .context("VERIFIED_VOTES_RESTORE_REWARD_READ")?;
        let bundles = storage
            .pbft()
            .two_t_plus_one_votes_bundles()
            .context("VERIFIED_VOTES_RESTORE_TWO_T_PLUS_ONE_READ")?;

        let mut sources = BTreeMap::<H256, RestoreVoteSource>::new();
        let mut bundle_mappings = Vec::new();
        let mut reward_info = None;

        for bundle in bundles {
            let kind = TwoTPlusOneVotedBlockType::try_from(bundle.kind)?;
            let rlp = Rlp::new(&bundle.votes_bundle_rlp);
            ensure!(
                rlp.is_list() && rlp.item_count()? > 0,
                "stored 2t+1 vote bundle {kind:?} must be a non-empty RLP list"
            );
            let mut coordinates = None;
            for vote in rlp.iter() {
                let vote_rlp = vote.as_raw().to_vec();
                let inspection = inspect_restored_weighted_vote(&vote_rlp)?;
                validate_restored_bundle_kind(kind, &inspection)?;
                let current = (
                    inspection.period,
                    inspection.round,
                    inspection.step,
                    inspection.block_hash,
                );
                if let Some(expected) = coordinates {
                    ensure!(
                        expected == current,
                        "stored 2t+1 vote bundle {kind:?} contains inconsistent vote coordinates"
                    );
                } else {
                    coordinates = Some(current);
                }
                merge_restore_source(&mut sources, inspection.vote_hash, vote_rlp, false)?;
            }
            let (period, round, step, block_hash) = coordinates.expect("non-empty bundle checked");
            if kind == TwoTPlusOneVotedBlockType::CertVotedBlock {
                let current = (period, round, block_hash);
                ensure!(
                    reward_info.is_none() || reward_info == Some(current),
                    "stored cert 2t+1 bundles contain inconsistent reward-vote metadata"
                );
                reward_info = Some(current);
            }
            bundle_mappings.push((kind, period, round, step, block_hash));
        }

        for record in own_votes {
            let inspection = inspect_restored_weighted_vote(&record.vote_rlp)?;
            ensure!(
                inspection.vote_hash == record.vote_hash,
                "stored own verified vote hash does not match its storage key"
            );
            merge_restore_source(&mut sources, inspection.vote_hash, record.vote_rlp, false)?;
        }
        for vote_rlp in reward_votes {
            let inspection = inspect_restored_weighted_vote(&vote_rlp)?;
            merge_restore_source(&mut sources, inspection.vote_hash, vote_rlp, true)?;
        }

        let mut runtime = Self::new();
        for (vote_hash, source) in &sources {
            let inspection = inspect_restored_weighted_vote(&source.vote_rlp)?;
            ensure!(
                inspection.vote_hash == *vote_hash,
                "restored vote hash changed during decode"
            );
            let vote = VerifiedVote::new(
                inspection.vote_hash,
                inspection.block_hash,
                inspection.recovered_voter,
                inspection.period,
                inspection.round,
                inspection.step,
                inspection.vote_type,
                inspection.embedded_weight,
            )?;
            let outcome = runtime.verified_votes.add_verified_vote(vote, None)?;
            ensure!(
                outcome.inserted,
                "stored verified vote conflicts with restored uniqueness index"
            );
            let slashing = build_slashing_pbft_vote_payload(&source.vote_rlp)?;
            runtime.payloads.insert(
                *vote_hash,
                PbftVoteRuntimePayload {
                    slashing,
                    weighted: PbftVotePayloadRecord {
                        hash: *vote_hash,
                        vote_rlp: source.vote_rlp.clone(),
                    },
                },
            );
            runtime.replay_insert(*vote_hash);
        }

        for (kind, period, round, step, block_hash) in bundle_mappings {
            let outcome = runtime
                .verified_votes
                .insert_two_t_plus_one_voted_block(period, round, kind, block_hash, step);
            ensure!(
                outcome.round_found && outcome.inserted,
                "stored 2t+1 mapping could not be restored for {kind:?}"
            );
        }

        let extra_reward_vote_hashes = sources
            .iter()
            .filter_map(|(vote_hash, source)| source.extra_reward_vote.then_some(*vote_hash))
            .collect();
        let (has_reward_vote_info, reward_vote_period, reward_vote_round, reward_vote_block_hash) =
            reward_info
                .map(|(period, round, block_hash)| (true, period, round, block_hash))
                .unwrap_or((false, 0, 0, H256::zero()));
        Ok((
            runtime,
            PbftVoteRuntimeRestoreSnapshot {
                extra_reward_vote_hashes,
                has_reward_vote_info,
                reward_vote_period,
                reward_vote_round,
                reward_vote_block_hash,
            },
        ))
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

    /// Returns all retained weighted PBFT vote payloads in deterministic vote-hash order.
    ///
    /// The returned records are suitable for temporary legacy materialization
    /// and network egress. The runtime only exposes payloads whose
    /// verified-vote metadata still exists, because cleanup prunes payloads
    /// against the verified-vote snapshot.
    #[must_use]
    pub fn weighted_payloads(&self) -> Vec<PbftVotePayloadRecord> {
        self.payloads
            .values()
            .map(|payload| payload.weighted.clone())
            .collect()
    }

    /// Returns retained weighted payloads for one Rust-owned 2t+1 mapping.
    ///
    /// Inputs:
    /// - `period`, `round`, and `kind` select the 2t+1 voted-block mapping.
    ///
    /// Outputs:
    /// - `None` when the mapping is absent.
    /// - Ordered weighted records for the mapped voted block when present.
    ///
    /// Error behavior:
    /// - Missing retained payloads for mapped vote hashes are hard errors
    ///   because verified-vote metadata and payload retention must advance
    ///   together.
    pub fn two_t_plus_one_weighted_payloads(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<Vec<PbftVotePayloadRecord>>> {
        if self
            .verified_votes
            .get_two_t_plus_one_voted_block(period, round, kind)
            .is_none()
        {
            return Ok(None);
        }

        let vote_hashes = self
            .verified_votes
            .get_two_t_plus_one_voted_block_vote_hashes(period, round, kind);
        let mut records = Vec::with_capacity(vote_hashes.len());
        for vote_hash in vote_hashes {
            records.push(self.weighted_payload(vote_hash).cloned().ok_or_else(|| {
                anyhow!("PBFT vote runtime missing weighted payload for 2t+1 vote {vote_hash:#x}")
            })?);
        }
        Ok(Some(records))
    }

    /// Selects PBFT reward votes and resolves retained weighted payloads.
    ///
    /// Inputs:
    /// - `block_period`: period of the PBFT block being validated.
    /// - `reward_period`, `preferred_reward_round`, and `reward_block_hash`:
    ///   current reward-vote metadata maintained by the vote manager.
    /// - `requested_vote_hashes`: hashes listed by the PBFT block, whose order
    ///   must be preserved for temporary C++ sidecar materialization.
    ///
    /// Outputs:
    /// - Rejected selection statuses from the side-effect-free reward planner.
    /// - Accepted selection metadata plus retained weighted payload records in
    ///   requested-hash order.
    ///
    /// Error behavior:
    /// - If an accepted selected vote hash has no retained weighted payload,
    ///   this returns a hard error because production reward selection must
    ///   operate on the unified Rust runtime, not metadata-only compatibility
    ///   helper inserts.
    pub fn select_reward_vote_payloads(
        &self,
        block_period: u64,
        reward_period: u64,
        preferred_reward_round: u64,
        reward_block_hash: H256,
        requested_vote_hashes: Vec<H256>,
    ) -> Result<PbftRewardVotePayloadSelection> {
        let preferred_round_lookup = self.verified_votes.reward_vote_round_candidate(
            reward_period,
            preferred_reward_round,
            reward_block_hash,
        );
        let has_preferred_round = preferred_round_lookup.is_some();
        let preferred_round =
            preferred_round_lookup.unwrap_or_else(|| PbftRewardVoteRoundCandidate {
                round: preferred_reward_round,
                has_cert_step: false,
                has_reward_block: false,
                vote_hashes: Vec::new(),
            });
        let period_rounds_lookup = self
            .verified_votes
            .reward_vote_period_candidates_rev(reward_period, reward_block_hash);
        let has_reward_period = period_rounds_lookup.is_some();
        let period_rounds = period_rounds_lookup.unwrap_or_default();
        let fact = PbftRewardVoteSelectionFact {
            block_period,
            reward_period,
            preferred_reward_round,
            reward_block_hash,
            requested_vote_hashes,
            has_preferred_round,
            preferred_round,
            has_reward_period,
            period_rounds,
        };
        let plan = plan_pbft_reward_votes(fact);
        self.resolve_reward_vote_payload_selection(plan)
    }

    fn resolve_reward_vote_payload_selection(
        &self,
        plan: PbftRewardVoteSelectionPlan,
    ) -> Result<PbftRewardVotePayloadSelection> {
        if !plan.accepted {
            return Ok(PbftRewardVotePayloadSelection {
                accepted: false,
                status: plan.status,
                selected_period: plan.selected_period,
                selected_round: plan.selected_round,
                selected_block_hash: plan.selected_block_hash,
                selected_vote_hashes: plan.selected_vote_hashes,
                selected_records: Vec::new(),
                missing_vote_hash: plan.missing_vote_hash,
            });
        }

        let mut records = Vec::with_capacity(plan.selected_vote_hashes.len());
        for vote_hash in &plan.selected_vote_hashes {
            records.push(self.weighted_payload(*vote_hash).cloned().ok_or_else(|| {
                anyhow!(
                    "PBFT reward-vote selection missing retained weighted payload for vote {vote_hash:#x}"
                )
            })?);
        }

        Ok(PbftRewardVotePayloadSelection {
            accepted: true,
            status: plan.status,
            selected_period: plan.selected_period,
            selected_round: plan.selected_round,
            selected_block_hash: plan.selected_block_hash,
            selected_vote_hashes: plan.selected_vote_hashes,
            selected_records: records,
            missing_vote_hash: None,
        })
    }

    /// Returns the slashing payload for `vote_hash`, when retained.
    #[must_use]
    pub fn slashing_payload(&self, vote_hash: H256) -> Option<&PbftVotePayloadRecord> {
        self.payloads
            .get(&vote_hash)
            .map(|payload| &payload.slashing)
    }

    /// Returns whether `vote_hash` is already retained in validation replay
    /// protection.
    #[must_use]
    pub fn replay_contains(&self, vote_hash: H256) -> bool {
        self.replay_cache.contains(vote_hash)
    }

    /// Inserts `vote_hash` into validation replay protection.
    ///
    /// The return value is true only when the hash was newly retained.
    pub fn replay_insert(&mut self, vote_hash: H256) -> bool {
        self.replay_cache.insert(vote_hash)
    }

    /// Applies replay-cache marking for one validation result.
    ///
    /// Inputs:
    /// - `validation`: canonical vote validation output carrying Rust's
    ///   replay-marker intent.
    ///
    /// Outputs:
    /// - Explicit replay decision and mutation facts so bridge callers do not
    ///   confuse "should mark" with "newly inserted."
    pub fn record_validation_replay(
        &mut self,
        validation: &PbftCanonicalVoteValidation,
    ) -> PbftVoteRuntimeReplayOutcome {
        let should_mark = validation.mark_validated_replay;
        if !should_mark {
            return PbftVoteRuntimeReplayOutcome {
                should_mark,
                inserted: false,
                already_present: false,
            };
        }
        let already_present = self.replay_contains(validation.vote_hash);
        let inserted = self.replay_insert(validation.vote_hash);
        PbftVoteRuntimeReplayOutcome {
            should_mark,
            inserted,
            already_present,
        }
    }

    /// Plans or computes the PBFT `2t+1` threshold from Rust-owned cache state.
    ///
    /// Inputs are explicit scalar facts supplied by the C++ boundary after
    /// FinalChain/PBFT-chain reads. The runtime owns cache lookup/update policy
    /// so threshold state is co-located with vote admission state.
    pub fn plan_two_t_plus_one_threshold(
        &mut self,
        fact: PbftTwoTPlusOneThresholdFact,
    ) -> PbftTwoTPlusOneThresholdPlan {
        self.threshold_runtime.plan_threshold(fact)
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
        let replay = self.record_validation_replay(validation);
        if !precheck.should_insert() {
            return Ok(PbftVoteRuntimeAdmissionOutcome {
                precheck,
                replay,
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
                kind,
                period: fact.identity.period,
                round: fact.identity.round,
                step: fact.identity.step,
                block_hash: fact.identity.block_hash,
                votes_count: records.len(),
                votes_bundle_rlp: build_weighted_pbft_vote_bundle(&records)?,
            })
        } else {
            None
        };

        Ok(PbftVoteRuntimeAdmissionOutcome {
            precheck,
            replay,
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
    use crate::verified_votes::{PbftVoteType, VerifiedVote};
    use ethereum_types::H160;
    use k256::ecdsa::SigningKey;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_vdf::vrf;
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
        vote_rlp_for(block_hash, 12, 2, step, node_secret)
    }

    fn vote_rlp_for(
        block_hash: [u8; 32],
        period: u64,
        round: u64,
        step: u64,
        node_secret: [u8; 32],
    ) -> Vec<u8> {
        generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Cert,
            period,
            round,
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
                has_preverified_weight: false,
                preverified_weight: 0,
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
            current_round: 2,
            max_future_period_delta: 0,
            two_t_plus_one_threshold: threshold,
            require_proposed_block_sidecar: false,
            slashing_enabled: true,
        }
    }

    fn threshold_fact(has_total_dpos_votes_count: bool) -> PbftTwoTPlusOneThresholdFact {
        PbftTwoTPlusOneThresholdFact {
            pbft_period: 12,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: 12,
            committee_size: 100,
            number_of_proposers: 20,
            has_total_dpos_votes_count,
            total_dpos_votes_count: if has_total_dpos_votes_count { 100 } else { 0 },
            future_dpos_state: false,
            unknown_error: false,
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
        assert!(runtime.replay_contains(validation.vote_hash));

        let payloads = runtime.weighted_payloads();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].hash, validation.vote_hash);
        assert!(!payloads[0].vote_rlp.is_empty());
    }

    #[test]
    fn runtime_owns_replay_cache_and_thresholds() {
        let mut runtime = PbftVoteAdmissionRuntime::new_with_replay_cache(1, 1);
        let first_hash = H256::from_low_u64_be(1);
        let second_hash = H256::from_low_u64_be(2);

        assert!(runtime.replay_insert(first_hash));
        assert!(!runtime.replay_insert(first_hash));
        assert!(runtime.replay_contains(first_hash));
        assert!(runtime.replay_insert(second_hash));
        assert!(!runtime.replay_contains(first_hash));
        assert!(runtime.replay_contains(second_hash));

        let plan = runtime.plan_two_t_plus_one_threshold(threshold_fact(true));
        assert!(plan.has_threshold);

        let cached = runtime.plan_two_t_plus_one_threshold(threshold_fact(false));
        assert!(cached.cache_hit);
        assert_eq!(cached.threshold, plan.threshold);
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

        let payloads = runtime
            .two_t_plus_one_weighted_payloads(12, 2, TwoTPlusOneVotedBlockType::CertVotedBlock)
            .unwrap()
            .expect("cert mapping is retained");
        assert_eq!(payloads.len(), 2);
        let payload_hashes = payloads
            .into_iter()
            .map(|payload| payload.hash)
            .collect::<Vec<_>>();
        assert!(payload_hashes.contains(&first_validation.vote_hash));
        assert!(payload_hashes.contains(&second_validation.vote_hash));
        assert!(
            runtime
                .two_t_plus_one_weighted_payloads(12, 3, TwoTPlusOneVotedBlockType::CertVotedBlock)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn runtime_selects_reward_payloads_from_reverse_round_in_requested_order() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let reward_block = [6; 32];
        let preferred = VerifiedVote::new(
            H256::from_low_u64_be(1),
            H256::from(reward_block),
            H160::from_low_u64_be(1),
            12,
            1,
            3,
            PbftVoteType::Cert,
            1,
        )
        .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(preferred, None)
            .unwrap();

        let first = vote_rlp_for(reward_block, 12, 2, 3, NODE_SECRET);
        let second = vote_rlp_for(reward_block, 12, 2, 3, NODE_SECRET_TWO);
        let first_validation = validation(&first);
        let second_validation = validation(&second);
        runtime
            .admit_validated_vote(&first, &first_validation, flags(), context(Some(80)))
            .unwrap();
        runtime
            .admit_validated_vote(&second, &second_validation, flags(), context(Some(80)))
            .unwrap();

        let selection = runtime
            .select_reward_vote_payloads(
                13,
                12,
                1,
                H256::from(reward_block),
                vec![second_validation.vote_hash, first_validation.vote_hash],
            )
            .unwrap();

        assert!(selection.accepted);
        assert_eq!(selection.selected_round, 2);
        assert_eq!(
            selection.selected_vote_hashes,
            vec![second_validation.vote_hash, first_validation.vote_hash]
        );
        assert_eq!(selection.selected_records.len(), 2);
        assert_eq!(
            selection.selected_records[0].hash,
            second_validation.vote_hash
        );
        assert_eq!(
            selection.selected_records[1].hash,
            first_validation.vote_hash
        );
    }

    #[test]
    fn runtime_errors_when_selected_reward_payload_is_missing() {
        let mut runtime = PbftVoteAdmissionRuntime::new();
        let vote_hash = H256::from_low_u64_be(77);
        let reward_block = H256::from_low_u64_be(88);
        let metadata_only_vote = VerifiedVote::new(
            vote_hash,
            reward_block,
            H160::from_low_u64_be(9),
            12,
            1,
            3,
            PbftVoteType::Cert,
            1,
        )
        .unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(metadata_only_vote, None)
            .unwrap();

        let err = runtime
            .select_reward_vote_payloads(13, 12, 1, reward_block, vec![vote_hash])
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("missing retained weighted payload")
        );
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

    #[test]
    fn runtime_restores_and_deduplicates_persisted_vote_families() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_votes_restore_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let canonical = vote_rlp([8; 32], 3);
        let weighted = build_weighted_pbft_vote_payload(&canonical, 7).unwrap();
        let bundle = build_weighted_pbft_vote_bundle(std::slice::from_ref(&weighted)).unwrap();
        storage
            .pbft()
            .replace_two_t_plus_one_votes(1, &bundle)
            .unwrap();
        storage
            .pbft()
            .write_own_verified_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();
        storage
            .pbft()
            .write_extra_reward_vote(weighted.hash, &weighted.vote_rlp)
            .unwrap();

        let (runtime, snapshot) = PbftVoteAdmissionRuntime::restore_from_storage(&storage).unwrap();

        assert_eq!(runtime.verified_votes().size(), 1);
        assert_eq!(runtime.weighted_payloads(), vec![weighted.clone()]);
        assert!(runtime.replay_contains(weighted.hash));
        assert_eq!(snapshot.extra_reward_vote_hashes, vec![weighted.hash]);
        assert!(snapshot.has_reward_vote_info);
        assert_eq!(snapshot.reward_vote_period, 12);
        assert_eq!(snapshot.reward_vote_round, 2);
        assert_eq!(snapshot.reward_vote_block_hash, H256::from([8; 32]));

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_rejects_malformed_persisted_vote() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_verified_votes_bad_restore_{nonce}"));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        storage
            .pbft()
            .write_own_verified_vote(H256::from_low_u64_be(1), &[0x01])
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn runtime_rejects_own_vote_payload_whose_hash_does_not_match_key() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_verified_votes_hash_mismatch_restore_{nonce}"
        ));
        let storage = Storage::new(Config::new(path.clone())).unwrap();
        let weighted = build_weighted_pbft_vote_payload(&vote_rlp([9; 32], 3), 7).unwrap();
        let wrong_hash = H256::from_low_u64_be(99);
        assert_ne!(wrong_hash, weighted.hash);
        storage
            .pbft()
            .write_own_verified_vote(wrong_hash, &weighted.vote_rlp)
            .unwrap();

        assert!(PbftVoteAdmissionRuntime::restore_from_storage(&storage).is_err());

        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }
}
