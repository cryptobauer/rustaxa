//! Native selection and acknowledgement for periodic consensus vote gossip.
//!
//! This module owns the `MaybeBroadcastVotes` daemon action end to end: cadence
//! selection, canonical payload lookup/encoding, ordered transport requests,
//! and exact acknowledgement validation. It exposes no manager action, bridge
//! carrier, C++ object, or signing secret. Physical tarcap gossip remains a
//! named host leaf and counters are committed only after every selected request
//! is acknowledged successfully.

use anyhow::{Result, anyhow, ensure};
use ethereum_types::H256;

use crate::pbft_service::PbftService;
use crate::pbft_vote_payload::{PbftVotePayloadRecord, build_optimized_pbft_vote_bundle};
use crate::pbft_vote_runtime::{RewardVotePayloadSnapshot, VerifiedVotesTwoTPlusOneVotePayloads};
use crate::pbft_vote_storage::PbftVoteStorageRecord;
use crate::pbft_vote_validation::{PbftCanonicalVoteInspectionStatus, inspect_canonical_pbft_vote};
use crate::proposed_blocks::ProposedBlockEntry;
use crate::verified_votes::TwoTPlusOneVotedBlockType;

/// Monotonic daemon-action identity supplied by the enclosing application run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaybeBroadcastVotesActionId(pub u64);

/// Native broadcast cadence counters committed after a complete transport batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VoteBroadcastCounters {
    pub broadcast_votes: u32,
    pub rebroadcast_votes: u32,
    pub broadcast_reward_votes: u32,
    pub rebroadcast_reward_votes: u32,
}

/// Complete scalar input for one `MaybeBroadcastVotes` daemon action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaybeBroadcastVotesInput {
    pub action_id: MaybeBroadcastVotesActionId,
    pub period: u64,
    pub round: u64,
    pub round_elapsed_ms: u64,
    pub period_elapsed_ms: u64,
    pub current_round_lambda_ms: u64,
    pub broadcast_lambda_threshold: u32,
    pub rebroadcast_lambda_threshold: u32,
    pub counters: VoteBroadcastCounters,
}

/// Semantic family of one selected transport request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VoteBroadcastFamily {
    RewardVotes,
    OwnVote,
    OwnPillarVote,
    SoftVotes,
    PreviousRoundNextVotes,
    PreviousRoundNextNullVotes,
}

/// Stable identity of one request inside a daemon action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VoteBroadcastRequestId {
    pub action_id: MaybeBroadcastVotesActionId,
    pub ordinal: u32,
}

/// Canonical transport request selected by native consensus.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsensusVoteTransportRequest {
    Vote {
        request_id: VoteBroadcastRequestId,
        family: VoteBroadcastFamily,
        canonical_vote_rlp: Vec<u8>,
        proposed_block_rlp: Option<Vec<u8>>,
        rebroadcast: bool,
    },
    VoteBundle {
        request_id: VoteBroadcastRequestId,
        family: VoteBroadcastFamily,
        canonical_votes_bundle_rlp: Vec<u8>,
        rebroadcast: bool,
    },
    PillarVote {
        request_id: VoteBroadcastRequestId,
        family: VoteBroadcastFamily,
        canonical_pillar_vote_rlp: Vec<u8>,
        rebroadcast: bool,
    },
}

impl ConsensusVoteTransportRequest {
    pub const fn request_id(&self) -> VoteBroadcastRequestId {
        match self {
            Self::Vote { request_id, .. }
            | Self::VoteBundle { request_id, .. }
            | Self::PillarVote { request_id, .. } => *request_id,
        }
    }

    pub const fn family(&self) -> VoteBroadcastFamily {
        match self {
            Self::Vote { family, .. }
            | Self::VoteBundle { family, .. }
            | Self::PillarVote { family, .. } => *family,
        }
    }
}

/// Ordered native transport batch and its not-yet-committed counter update.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaybeBroadcastVotesBatch {
    pub requests: Vec<ConsensusVoteTransportRequest>,
    pub(crate) next_counters: VoteBroadcastCounters,
}

/// Typed physical transport acknowledgement for one selected request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteBroadcastAcknowledgement {
    pub request_id: VoteBroadcastRequestId,
    pub family: VoteBroadcastFamily,
    pub succeeded: bool,
    pub error_code: String,
}

/// Counter update authorized by a complete, exact, successful acknowledgement set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaybeBroadcastVotesCommit {
    pub counters: VoteBroadcastCounters,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BroadcastScope {
    Period,
    Round,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CadenceDecision {
    scope: BroadcastScope,
    rebroadcast: bool,
    next_counters: VoteBroadcastCounters,
}

trait BroadcastVoteState {
    fn reward_votes(&self) -> Result<RewardVotePayloadSnapshot>;
    fn own_votes(&self) -> Result<Vec<PbftVoteStorageRecord>>;
    fn two_t_plus_one(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotePayloads>>;
    fn proposed_block(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry>;
    fn own_pillar_vote(&self) -> Result<Vec<u8>>;
}

impl BroadcastVoteState for PbftService {
    fn reward_votes(&self) -> Result<RewardVotePayloadSnapshot> {
        self.current_reward_vote_snapshot()
    }

    fn own_votes(&self) -> Result<Vec<PbftVoteStorageRecord>> {
        self.verified_votes_own_vote_records()
    }

    fn two_t_plus_one(
        &self,
        period: u64,
        round: u64,
        kind: TwoTPlusOneVotedBlockType,
    ) -> Result<Option<VerifiedVotesTwoTPlusOneVotePayloads>> {
        self.verified_votes_get_two_t_plus_one_voted_block_payloads(period, round, kind)
    }

    fn proposed_block(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry> {
        self.proposed_block(period, block_hash)
    }

    fn own_pillar_vote(&self) -> Result<Vec<u8>> {
        self.own_pillar_block_vote()
    }
}

fn threshold_exceeded(elapsed_ms: u64, lambda_ms: u64, threshold: u32, counter: u32) -> bool {
    elapsed_ms / lambda_ms > u64::from(threshold).saturating_mul(u64::from(counter))
}

fn increment(counter: u32) -> Result<u32> {
    counter
        .checked_add(1)
        .ok_or_else(|| anyhow!("MAYBE_BROADCAST_VOTES_COUNTER_EXHAUSTED"))
}

fn select_cadence(input: MaybeBroadcastVotesInput) -> Result<Option<CadenceDecision>> {
    ensure!(
        input.action_id.0 != 0,
        "MAYBE_BROADCAST_VOTES_ZERO_ACTION_ID"
    );
    ensure!(input.round > 0, "MAYBE_BROADCAST_VOTES_ZERO_ROUND");
    ensure!(
        input.current_round_lambda_ms > 0,
        "MAYBE_BROADCAST_VOTES_ZERO_LAMBDA"
    );
    ensure!(
        input.broadcast_lambda_threshold > 0 && input.rebroadcast_lambda_threshold > 0,
        "MAYBE_BROADCAST_VOTES_ZERO_THRESHOLD"
    );
    ensure!(
        input.counters.broadcast_votes > 0
            && input.counters.rebroadcast_votes > 0
            && input.counters.broadcast_reward_votes > 0
            && input.counters.rebroadcast_reward_votes > 0,
        "MAYBE_BROADCAST_VOTES_ZERO_COUNTER"
    );

    let mut next = input.counters;
    if threshold_exceeded(
        input.round_elapsed_ms,
        input.current_round_lambda_ms,
        input.rebroadcast_lambda_threshold,
        input.counters.rebroadcast_votes,
    ) {
        next.broadcast_votes = increment(next.broadcast_votes)?;
        next.rebroadcast_votes = increment(next.rebroadcast_votes)?;
        return Ok(Some(CadenceDecision {
            scope: BroadcastScope::Round,
            rebroadcast: true,
            next_counters: next,
        }));
    }
    if threshold_exceeded(
        input.round_elapsed_ms,
        input.current_round_lambda_ms,
        input.broadcast_lambda_threshold,
        input.counters.broadcast_votes,
    ) {
        next.broadcast_votes = increment(next.broadcast_votes)?;
        return Ok(Some(CadenceDecision {
            scope: BroadcastScope::Round,
            rebroadcast: false,
            next_counters: next,
        }));
    }
    if threshold_exceeded(
        input.period_elapsed_ms,
        input.current_round_lambda_ms,
        input.rebroadcast_lambda_threshold,
        input.counters.rebroadcast_reward_votes,
    ) {
        next.broadcast_reward_votes = increment(next.broadcast_reward_votes)?;
        next.rebroadcast_reward_votes = increment(next.rebroadcast_reward_votes)?;
        return Ok(Some(CadenceDecision {
            scope: BroadcastScope::Period,
            rebroadcast: true,
            next_counters: next,
        }));
    }
    if threshold_exceeded(
        input.period_elapsed_ms,
        input.current_round_lambda_ms,
        input.broadcast_lambda_threshold,
        input.counters.broadcast_reward_votes,
    ) {
        next.broadcast_reward_votes = increment(next.broadcast_reward_votes)?;
        return Ok(Some(CadenceDecision {
            scope: BroadcastScope::Period,
            rebroadcast: false,
            next_counters: next,
        }));
    }
    Ok(None)
}

fn optimized_bundle(records: &[PbftVotePayloadRecord]) -> Result<Vec<u8>> {
    let first = records
        .first()
        .ok_or_else(|| anyhow!("MAYBE_BROADCAST_VOTES_EMPTY_BUNDLE"))?;
    let inspection = inspect_canonical_pbft_vote(&first.vote_rlp)?;
    ensure!(
        inspection.status == PbftCanonicalVoteInspectionStatus::Valid && inspection.signature_valid,
        "MAYBE_BROADCAST_VOTES_INVALID_BUNDLE_VOTE"
    );
    Ok(build_optimized_pbft_vote_bundle(
        records,
        inspection.block_hash,
        inspection.period,
        inspection.round,
        inspection.step,
    )?
    .bundle_rlp)
}

fn push_bundle(
    requests: &mut Vec<ConsensusVoteTransportRequest>,
    action_id: MaybeBroadcastVotesActionId,
    family: VoteBroadcastFamily,
    records: &[PbftVotePayloadRecord],
    rebroadcast: bool,
) -> Result<()> {
    if records.is_empty() {
        return Ok(());
    }
    requests.push(ConsensusVoteTransportRequest::VoteBundle {
        request_id: VoteBroadcastRequestId {
            action_id,
            ordinal: u32::try_from(requests.len() + 1)
                .map_err(|_| anyhow!("MAYBE_BROADCAST_VOTES_TOO_MANY_REQUESTS"))?,
        },
        family,
        canonical_votes_bundle_rlp: optimized_bundle(records)?,
        rebroadcast,
    });
    Ok(())
}

fn select_with_state<S: BroadcastVoteState>(
    state: &S,
    input: MaybeBroadcastVotesInput,
) -> Result<Option<MaybeBroadcastVotesBatch>> {
    let Some(cadence) = select_cadence(input)? else {
        return Ok(None);
    };

    let mut requests = Vec::new();
    let reward_votes = state.reward_votes()?;
    push_bundle(
        &mut requests,
        input.action_id,
        VoteBroadcastFamily::RewardVotes,
        &reward_votes.records,
        cadence.rebroadcast,
    )?;

    for own_vote in state.own_votes()? {
        let inspection = inspect_canonical_pbft_vote(&own_vote.vote_rlp)?;
        ensure!(
            inspection.status == PbftCanonicalVoteInspectionStatus::Valid
                && inspection.signature_valid
                && inspection.vote_hash == own_vote.hash,
            "MAYBE_BROADCAST_VOTES_INVALID_OWN_VOTE"
        );
        let proposed_block_rlp = state
            .proposed_block(inspection.period, inspection.block_hash)
            .map(|entry| entry.block_rlp);
        requests.push(ConsensusVoteTransportRequest::Vote {
            request_id: VoteBroadcastRequestId {
                action_id: input.action_id,
                ordinal: u32::try_from(requests.len() + 1)
                    .map_err(|_| anyhow!("MAYBE_BROADCAST_VOTES_TOO_MANY_REQUESTS"))?,
            },
            family: VoteBroadcastFamily::OwnVote,
            canonical_vote_rlp: own_vote.vote_rlp,
            proposed_block_rlp,
            rebroadcast: cadence.rebroadcast,
        });
    }

    let own_pillar_vote = state.own_pillar_vote()?;
    if !own_pillar_vote.is_empty() {
        requests.push(ConsensusVoteTransportRequest::PillarVote {
            request_id: VoteBroadcastRequestId {
                action_id: input.action_id,
                ordinal: u32::try_from(requests.len() + 1)
                    .map_err(|_| anyhow!("MAYBE_BROADCAST_VOTES_TOO_MANY_REQUESTS"))?,
            },
            family: VoteBroadcastFamily::OwnPillarVote,
            canonical_pillar_vote_rlp: own_pillar_vote,
            rebroadcast: cadence.rebroadcast,
        });
    }

    if cadence.scope == BroadcastScope::Round {
        if let Some(soft) = state.two_t_plus_one(
            input.period,
            input.round,
            TwoTPlusOneVotedBlockType::SoftVotedBlock,
        )? {
            push_bundle(
                &mut requests,
                input.action_id,
                VoteBroadcastFamily::SoftVotes,
                &soft.votes,
                cadence.rebroadcast,
            )?;
        }
        if input.round > 1 {
            if let Some(next) = state.two_t_plus_one(
                input.period,
                input.round - 1,
                TwoTPlusOneVotedBlockType::NextVotedBlock,
            )? {
                push_bundle(
                    &mut requests,
                    input.action_id,
                    VoteBroadcastFamily::PreviousRoundNextVotes,
                    &next.votes,
                    cadence.rebroadcast,
                )?;
            }
            if let Some(next_null) = state.two_t_plus_one(
                input.period,
                input.round - 1,
                TwoTPlusOneVotedBlockType::NextVotedNullBlock,
            )? {
                push_bundle(
                    &mut requests,
                    input.action_id,
                    VoteBroadcastFamily::PreviousRoundNextNullVotes,
                    &next_null.votes,
                    cadence.rebroadcast,
                )?;
            }
        }
    }

    Ok(Some(MaybeBroadcastVotesBatch {
        requests,
        next_counters: cadence.next_counters,
    }))
}

/// Selects one native vote-gossip batch from authoritative service state.
///
/// `None` means no cadence threshold was reached. A selected batch preserves
/// legacy family priority and request order: reward bundle, individual own
/// votes, own pillar vote, then round-only soft/previous-next bundles. Missing
/// families are omitted. Decode, invariant, and storage failures abort without
/// authorizing counter mutation.
pub fn select_maybe_broadcast_votes(
    service: &PbftService,
    input: MaybeBroadcastVotesInput,
) -> Result<Option<MaybeBroadcastVotesBatch>> {
    select_with_state(service, input)
}

/// Validates the complete transport acknowledgement set for one selected batch.
///
/// Acknowledgements must match request count, order, action identity, ordinal,
/// and family exactly. A transport rejection is a valid retryable outcome and
/// returns `None`, authorizing no counter mutation. Stale, missing, duplicate,
/// or extra reports remain terminal contract errors. Empty selected batches
/// commit their cadence counters immediately, matching legacy successful
/// no-payload ticks.
pub fn validate_maybe_broadcast_votes_acknowledgements(
    batch: &MaybeBroadcastVotesBatch,
    acknowledgements: &[VoteBroadcastAcknowledgement],
) -> Result<Option<MaybeBroadcastVotesCommit>> {
    ensure!(
        batch.requests.len() == acknowledgements.len(),
        "MAYBE_BROADCAST_VOTES_ACK_COUNT_MISMATCH"
    );
    for (request, acknowledgement) in batch.requests.iter().zip(acknowledgements) {
        ensure!(
            request.request_id() == acknowledgement.request_id,
            "MAYBE_BROADCAST_VOTES_STALE_ACK"
        );
        ensure!(
            request.family() == acknowledgement.family,
            "MAYBE_BROADCAST_VOTES_ACK_FAMILY_MISMATCH"
        );
    }
    if acknowledgements
        .iter()
        .any(|acknowledgement| !acknowledgement.succeeded)
    {
        return Ok(None);
    }
    Ok(Some(MaybeBroadcastVotesCommit {
        counters: batch.next_counters,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote};
    use crate::pbft_vote_payload::build_weighted_pbft_vote_payload;
    use crate::pbft_vote_runtime::RewardVoteCursorSnapshot;
    use crate::verified_votes::PbftVoteType;
    use ethereum_types::H160;
    use k256::ecdsa::SigningKey;
    use rustaxa_vdf::vrf;
    use tiny_keccak::{Hasher, Keccak};

    const NODE_SECRET: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn voter() -> H160 {
        let key = SigningKey::from_slice(&NODE_SECRET).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        let mut output = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&public_key.as_bytes()[1..]);
        hasher.finalize(&mut output);
        H160::from_slice(&output[12..])
    }

    fn weighted_vote(
        block_hash: H256,
        period: u64,
        round: u64,
        step: u64,
    ) -> PbftVotePayloadRecord {
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash,
            vote_type: match step {
                1 => PbftVoteType::Propose,
                2 => PbftVoteType::Soft,
                3 => PbftVoteType::Cert,
                _ => PbftVoteType::Next,
            },
            period,
            round,
            step,
            node_secret: NODE_SECRET,
            vrf_secret: VRF_SECRET,
            expected_voter: voter(),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        assert!(generated.accepted);
        build_weighted_pbft_vote_payload(&generated.vote_rlp, 1).unwrap()
    }

    struct FakeState {
        reward: Vec<PbftVotePayloadRecord>,
        own: Vec<PbftVoteStorageRecord>,
        soft: Option<VerifiedVotesTwoTPlusOneVotePayloads>,
        proposed: Option<ProposedBlockEntry>,
        pillar: Vec<u8>,
    }

    impl BroadcastVoteState for FakeState {
        fn reward_votes(&self) -> Result<RewardVotePayloadSnapshot> {
            Ok(RewardVotePayloadSnapshot {
                cursor: RewardVoteCursorSnapshot {
                    found: !self.reward.is_empty(),
                    period: 0,
                    round: 0,
                    step: 0,
                    block_hash: H256::zero(),
                },
                records: self.reward.clone(),
            })
        }

        fn own_votes(&self) -> Result<Vec<PbftVoteStorageRecord>> {
            Ok(self.own.clone())
        }

        fn two_t_plus_one(
            &self,
            _period: u64,
            _round: u64,
            kind: TwoTPlusOneVotedBlockType,
        ) -> Result<Option<VerifiedVotesTwoTPlusOneVotePayloads>> {
            Ok((kind == TwoTPlusOneVotedBlockType::SoftVotedBlock)
                .then(|| self.soft.clone())
                .flatten())
        }

        fn proposed_block(&self, period: u64, block_hash: H256) -> Option<ProposedBlockEntry> {
            self.proposed
                .clone()
                .filter(|entry| entry.period == period && entry.block_hash == block_hash)
        }

        fn own_pillar_vote(&self) -> Result<Vec<u8>> {
            Ok(self.pillar.clone())
        }
    }

    fn input() -> MaybeBroadcastVotesInput {
        MaybeBroadcastVotesInput {
            action_id: MaybeBroadcastVotesActionId(7),
            period: 9,
            round: 3,
            round_elapsed_ms: 0,
            period_elapsed_ms: 0,
            current_round_lambda_ms: 100,
            broadcast_lambda_threshold: 2,
            rebroadcast_lambda_threshold: 5,
            counters: VoteBroadcastCounters {
                broadcast_votes: 1,
                rebroadcast_votes: 1,
                broadcast_reward_votes: 1,
                rebroadcast_reward_votes: 1,
            },
        }
    }

    #[test]
    fn cadence_preserves_round_priority_and_strict_thresholds() {
        let mut fact = input();
        fact.round_elapsed_ms = 200;
        assert_eq!(select_cadence(fact).unwrap(), None);

        fact.round_elapsed_ms = 600;
        fact.period_elapsed_ms = 1_000;
        let decision = select_cadence(fact).unwrap().unwrap();
        assert_eq!(decision.scope, BroadcastScope::Round);
        assert!(decision.rebroadcast);
        assert_eq!(decision.next_counters.broadcast_votes, 2);
        assert_eq!(decision.next_counters.rebroadcast_votes, 2);
        assert_eq!(decision.next_counters.broadcast_reward_votes, 1);
    }

    #[test]
    fn cadence_selects_period_broadcast_after_round_paths_miss() {
        let mut fact = input();
        fact.period_elapsed_ms = 300;
        let decision = select_cadence(fact).unwrap().unwrap();
        assert_eq!(decision.scope, BroadcastScope::Period);
        assert!(!decision.rebroadcast);
        assert_eq!(decision.next_counters.broadcast_reward_votes, 2);
    }

    #[test]
    fn cadence_rejects_zero_and_overflow_inputs() {
        let mut fact = input();
        fact.current_round_lambda_ms = 0;
        assert!(select_cadence(fact).is_err());

        fact = input();
        fact.round_elapsed_ms = u64::MAX;
        fact.counters.broadcast_votes = u32::MAX;
        assert!(select_cadence(fact).is_err());
    }

    fn request(ordinal: u32, family: VoteBroadcastFamily) -> ConsensusVoteTransportRequest {
        ConsensusVoteTransportRequest::VoteBundle {
            request_id: VoteBroadcastRequestId {
                action_id: MaybeBroadcastVotesActionId(7),
                ordinal,
            },
            family,
            canonical_votes_bundle_rlp: vec![0xc0],
            rebroadcast: false,
        }
    }

    #[test]
    fn acknowledgements_must_match_exact_order_identity_and_family() {
        let batch = MaybeBroadcastVotesBatch {
            requests: vec![
                request(1, VoteBroadcastFamily::RewardVotes),
                request(2, VoteBroadcastFamily::SoftVotes),
            ],
            next_counters: input().counters,
        };
        let valid = vec![
            VoteBroadcastAcknowledgement {
                request_id: batch.requests[0].request_id(),
                family: VoteBroadcastFamily::RewardVotes,
                succeeded: true,
                error_code: String::new(),
            },
            VoteBroadcastAcknowledgement {
                request_id: batch.requests[1].request_id(),
                family: VoteBroadcastFamily::SoftVotes,
                succeeded: true,
                error_code: String::new(),
            },
        ];
        assert_eq!(
            validate_maybe_broadcast_votes_acknowledgements(&batch, &valid)
                .unwrap()
                .unwrap()
                .counters,
            input().counters
        );

        let mut stale = valid.clone();
        stale[1].request_id.action_id = MaybeBroadcastVotesActionId(8);
        assert!(validate_maybe_broadcast_votes_acknowledgements(&batch, &stale).is_err());

        let mut failed = valid.clone();
        failed[0].succeeded = false;
        failed[0].error_code = "transport down".to_owned();
        assert_eq!(
            validate_maybe_broadcast_votes_acknowledgements(&batch, &failed).unwrap(),
            None
        );

        assert_eq!(
            validate_maybe_broadcast_votes_acknowledgements(&batch, &valid)
                .unwrap()
                .unwrap()
                .counters,
            input().counters
        );
    }

    #[test]
    fn selected_empty_batch_commits_cadence_without_acknowledgements() {
        let expected = input().counters;
        let batch = MaybeBroadcastVotesBatch {
            requests: Vec::new(),
            next_counters: expected,
        };
        assert_eq!(
            validate_maybe_broadcast_votes_acknowledgements(&batch, &[])
                .unwrap()
                .unwrap()
                .counters,
            expected
        );
    }

    #[test]
    fn round_selection_emits_canonical_families_in_transport_order() {
        let block_hash = H256::from_low_u64_be(9);
        let reward = weighted_vote(block_hash, 9, 3, 3);
        let own = weighted_vote(block_hash, 9, 3, 1);
        let soft = weighted_vote(block_hash, 9, 3, 2);
        let state = FakeState {
            reward: vec![reward],
            own: vec![PbftVoteStorageRecord {
                hash: own.hash,
                vote_rlp: own.vote_rlp,
            }],
            soft: Some(VerifiedVotesTwoTPlusOneVotePayloads {
                block_hash,
                step: 2,
                votes: vec![soft],
            }),
            proposed: Some(ProposedBlockEntry {
                period: 9,
                block_hash,
                block_rlp: vec![0xc0],
                pivot_hash: H256::zero(),
                is_valid: false,
            }),
            pillar: vec![0xc1, 0x01],
        };
        let mut fact = input();
        fact.round_elapsed_ms = 300;
        let batch = select_with_state(&state, fact).unwrap().unwrap();
        assert_eq!(batch.requests.len(), 4);
        assert_eq!(batch.requests[0].family(), VoteBroadcastFamily::RewardVotes);
        assert_eq!(batch.requests[1].family(), VoteBroadcastFamily::OwnVote);
        assert_eq!(
            batch.requests[2].family(),
            VoteBroadcastFamily::OwnPillarVote
        );
        assert_eq!(batch.requests[3].family(), VoteBroadcastFamily::SoftVotes);
        assert!(matches!(
            &batch.requests[1],
            ConsensusVoteTransportRequest::Vote {
                proposed_block_rlp: Some(bytes),
                ..
            } if bytes == &vec![0xc0]
        ));
    }
}
