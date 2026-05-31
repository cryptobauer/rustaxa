//! Consensus event pipeline scaffolding.
//!
//! This module defines only consensus-layer vocabulary for the future
//! arena-backed ingress design. Network events, prefilter decisions,
//! dispatcher classification, and ring-buffer allocation belong in the network
//! crate or a dedicated pipeline crate. Consensus events may still reference
//! canonical ingress bytes through an opaque [`IngressPayloadRef`], but the
//! unit passed through consensus is an event.
//!
//! This is intentionally lightweight scaffolding, not a stable public API.
//! Type names, variants, and payload fields are expected to change as the first
//! real network-to-consensus pipeline integration proves the shape.
//!
//! Consensus business logic should be expressed as deterministic protocol
//! planners over explicit state views. A planner receives a consensus event or
//! command plus borrowed facts, then returns a [`ConsensusPlan`]. The plan
//! describes the protocol transition to execute: ordered effects, future write
//! intents, follow-up events, and validation outcome data. Planners should not
//! own long-lived data, perform I/O, spawn async work, or directly mutate
//! pipeline, network, storage, or peer state.

/// Identifies the logical consensus data pipeline that owns an event.
///
/// A consensus event should be handled by one pipeline at a time. Any
/// interaction with another pipeline should be represented by an explicit
/// [`ConsensusEffect`] rather than hidden mutation across pipeline state.
///
/// These pipeline categories are provisional design vocabulary. Keep external
/// use narrow until production routing validates the final boundaries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineKind {
    /// Peer status, readiness, and sync-control signals that affect consensus.
    PeerStatusSyncControl,
    /// Transaction validation planning and pool admission.
    TransactionAdmission,
    /// Live DAG block admission.
    DagBlockAdmission,
    /// DAG synchronization response processing.
    DagSync,
    /// PBFT vote, vote bundle, and round-progress processing.
    PbftVoteProgress,
    /// PBFT chain sync and finalized-period intake.
    PbftSyncIntake,
    /// Pillar vote and pillar-vote bundle processing.
    PillarVoteHandling,
}

/// Opaque handle for canonical ingress payload bytes stored outside consensus.
///
/// This is a data-plane reference, not a consensus identity. Consensus logic
/// must continue to identify domain objects by hashes, periods, rounds, steps,
/// voters, and levels. The reference allows a consensus event to decode late or
/// attach enrichment without copying or materializing C++ objects eagerly.
///
/// The concrete handle shape is open to change once the arena owner and routing
/// crate are chosen.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IngressPayloadRef(pub u64);

/// Origin of a consensus event.
///
/// Events can originate from ingress payloads or from internal consensus
/// scheduling. Internal events deliberately have no ingress payload reference.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum EventOrigin {
    /// Event backed by canonical ingress payload bytes.
    IngressPayload(IngressPayloadRef),
    /// Event produced inside consensus without a network-ingress payload.
    Internal,
}

/// Compact 32-byte hash fact used by typed consensus events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Hash32(pub [u8; 32]);

/// Compact 20-byte address fact used by typed consensus events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Address20(pub [u8; 20]);

/// Consensus-facing peer status or sync-control event.
///
/// This event type is intentionally empty beyond origin for now. A later slice
/// should add consensus-owned status facts without depending on network handler
/// objects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerStatusEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Consensus-facing transaction admission event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Consensus-facing DAG block admission event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DagBlockEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Consensus-facing DAG sync event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DagSyncEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Optional compact PBFT vote facts carried through the PBFT vote pipeline.
///
/// These facts can be attached by an ingress/pipeline stage that has already
/// inspected canonical bytes. They do not require materializing a C++ `PbftVote`
/// or `PbftBlock`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PbftVoteFacts {
    /// Vote hash if already extracted.
    pub vote_hash: Option<Hash32>,
    /// Voted PBFT block hash if already extracted.
    pub block_hash: Option<Hash32>,
    /// Vote period if already extracted.
    pub period: Option<u64>,
    /// Vote round if already extracted.
    pub round: Option<u64>,
    /// Vote step if already extracted.
    pub step: Option<u64>,
    /// Voter address if already extracted.
    pub voter: Option<Address20>,
    /// Whether the original ingress payload carries proposed-block sidecar bytes.
    pub carries_proposed_block: bool,
}

/// Typed event passed through the PBFT vote-progress pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PbftVoteEvent {
    /// Event origin.
    pub origin: EventOrigin,
    /// Optional compact facts discovered before or during consensus processing.
    pub facts: PbftVoteFacts,
}

impl PbftVoteEvent {
    /// Creates a PBFT vote event backed by an ingress payload reference with no
    /// extracted facts.
    #[must_use]
    pub const fn from_ingress(payload_ref: IngressPayloadRef) -> Self {
        Self {
            origin: EventOrigin::IngressPayload(payload_ref),
            facts: PbftVoteFacts {
                vote_hash: None,
                block_hash: None,
                period: None,
                round: None,
                step: None,
                voter: None,
                carries_proposed_block: false,
            },
        }
    }
}

/// Consensus-facing PBFT sync/finalized-period intake event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PbftSyncEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Consensus-facing pillar vote event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PillarVoteEvent {
    /// Event origin.
    pub origin: EventOrigin,
}

/// Event passed through one of the logical consensus pipelines.
///
/// This enum is a scaffold for discussing and testing pipeline ownership. Its
/// variants should not be treated as stable bridge or storage contracts yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusEvent {
    /// Peer status and sync-control pipeline event.
    PeerStatusSyncControl(PeerStatusEvent),
    /// Transaction admission pipeline event.
    TransactionAdmission(TransactionEvent),
    /// DAG block admission pipeline event.
    DagBlockAdmission(DagBlockEvent),
    /// DAG sync pipeline event.
    DagSync(DagSyncEvent),
    /// PBFT vote-progress pipeline event.
    PbftVoteProgress(PbftVoteEvent),
    /// PBFT sync and finalized-period intake pipeline event.
    PbftSyncIntake(PbftSyncEvent),
    /// Pillar vote handling pipeline event.
    PillarVoteHandling(PillarVoteEvent),
}

impl ConsensusEvent {
    /// Returns the logical pipeline that owns this event.
    #[must_use]
    pub const fn pipeline(&self) -> PipelineKind {
        match self {
            Self::PeerStatusSyncControl(_) => PipelineKind::PeerStatusSyncControl,
            Self::TransactionAdmission(_) => PipelineKind::TransactionAdmission,
            Self::DagBlockAdmission(_) => PipelineKind::DagBlockAdmission,
            Self::DagSync(_) => PipelineKind::DagSync,
            Self::PbftVoteProgress(_) => PipelineKind::PbftVoteProgress,
            Self::PbftSyncIntake(_) => PipelineKind::PbftSyncIntake,
            Self::PillarVoteHandling(_) => PipelineKind::PillarVoteHandling,
        }
    }

    /// Returns the event origin.
    #[must_use]
    pub const fn origin(&self) -> EventOrigin {
        match self {
            Self::PeerStatusSyncControl(event) => event.origin,
            Self::TransactionAdmission(event) => event.origin,
            Self::DagBlockAdmission(event) => event.origin,
            Self::DagSync(event) => event.origin,
            Self::PbftVoteProgress(event) => event.origin,
            Self::PbftSyncIntake(event) => event.origin,
            Self::PillarVoteHandling(event) => event.origin,
        }
    }
}

/// Cross-pipeline or egress-visible effect produced by a consensus planner.
///
/// The scaffold keeps these effects broad. Later slices should add
/// pipeline-specific payloads around these variants instead of hiding
/// cross-pipeline behavior behind direct state mutation.
///
/// The effect set is intentionally open-ended while the rewrite identifies the
/// exact executor boundaries between network, consensus, storage, and peer
/// state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusEffect {
    /// Mark a message-derived hash or identity as known for the source peer.
    MarkKnown,
    /// Admit the message-derived fact into the owning consensus state.
    Admit,
    /// Drop the event without peer punishment.
    Drop,
    /// Gossip or rebroadcast a message-derived fact to peers.
    Gossip,
    /// Request PBFT, DAG, vote, or pillar synchronization.
    RequestSync,
    /// Report malicious or protocol-invalid peer behavior.
    ReportMalicious,
    /// Preserve peer-order blocking between ingress message kinds.
    BlockPeerOrder,
    /// Enqueue finalized-period data for PBFT manager processing.
    EnqueuePeriodData,
    /// Notify PBFT runtime that vote or block state may drive progress.
    DrivePbftProgress,
}

/// Minimal protocol plan returned by a deterministic consensus planner.
///
/// This envelope is side-effect-free: it describes the planned protocol state
/// transition but does not execute it. Callers are responsible for applying
/// effects through the appropriate network, peer-state, storage, or consensus
/// executor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusPlan {
    /// Pipeline that produced the plan.
    pub pipeline: PipelineKind,
    /// Origin the plan applies to.
    pub origin: EventOrigin,
    /// Ordered effects to execute outside the planner.
    pub effects: Vec<ConsensusEffect>,
}

impl ConsensusPlan {
    /// Creates a protocol plan and preserves the caller-provided effect order.
    /// Empty effect lists are valid and represent a no-op plan.
    #[must_use]
    pub fn new(pipeline: PipelineKind, origin: EventOrigin, effects: Vec<ConsensusEffect>) -> Self {
        Self {
            pipeline,
            origin,
            effects,
        }
    }

    /// Returns whether the decision includes the requested effect.
    #[must_use]
    pub fn contains(&self, effect: ConsensusEffect) -> bool {
        self.effects.contains(&effect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consensus_event_variants_map_to_logical_pipelines() {
        let origin = EventOrigin::IngressPayload(IngressPayloadRef(1));
        let cases = [
            (
                ConsensusEvent::PeerStatusSyncControl(PeerStatusEvent { origin }),
                PipelineKind::PeerStatusSyncControl,
            ),
            (
                ConsensusEvent::TransactionAdmission(TransactionEvent { origin }),
                PipelineKind::TransactionAdmission,
            ),
            (
                ConsensusEvent::DagBlockAdmission(DagBlockEvent { origin }),
                PipelineKind::DagBlockAdmission,
            ),
            (
                ConsensusEvent::DagSync(DagSyncEvent { origin }),
                PipelineKind::DagSync,
            ),
            (
                ConsensusEvent::PbftVoteProgress(PbftVoteEvent::from_ingress(IngressPayloadRef(1))),
                PipelineKind::PbftVoteProgress,
            ),
            (
                ConsensusEvent::PbftSyncIntake(PbftSyncEvent { origin }),
                PipelineKind::PbftSyncIntake,
            ),
            (
                ConsensusEvent::PillarVoteHandling(PillarVoteEvent { origin }),
                PipelineKind::PillarVoteHandling,
            ),
        ];

        for (event, expected_pipeline) in cases {
            assert_eq!(event.pipeline(), expected_pipeline);
            assert_eq!(event.origin(), origin);
        }
    }

    #[test]
    fn pbft_vote_event_can_reference_ingress_payload_without_network_event() {
        let event = PbftVoteEvent::from_ingress(IngressPayloadRef(11));

        assert_eq!(
            event.origin,
            EventOrigin::IngressPayload(IngressPayloadRef(11))
        );
        assert_eq!(event.facts, PbftVoteFacts::default());
    }

    #[test]
    fn pbft_vote_event_can_carry_compact_facts_without_materialized_objects() {
        let vote_event = PbftVoteEvent {
            origin: EventOrigin::IngressPayload(IngressPayloadRef(12)),
            facts: PbftVoteFacts {
                vote_hash: Some(Hash32([4; 32])),
                block_hash: Some(Hash32([5; 32])),
                period: Some(6),
                round: Some(7),
                step: Some(8),
                voter: Some(Address20([9; 20])),
                carries_proposed_block: true,
            },
        };

        assert_eq!(vote_event.facts.period, Some(6));
        assert!(vote_event.facts.carries_proposed_block);
    }

    #[test]
    fn plan_preserves_ordered_effects() {
        let origin = EventOrigin::IngressPayload(IngressPayloadRef(7));
        let plan = ConsensusPlan::new(
            PipelineKind::PbftVoteProgress,
            origin,
            vec![
                ConsensusEffect::MarkKnown,
                ConsensusEffect::Admit,
                ConsensusEffect::DrivePbftProgress,
                ConsensusEffect::Gossip,
            ],
        );

        assert_eq!(plan.pipeline, PipelineKind::PbftVoteProgress);
        assert_eq!(plan.origin, origin);
        assert!(plan.contains(ConsensusEffect::DrivePbftProgress));
        assert_eq!(
            plan.effects,
            vec![
                ConsensusEffect::MarkKnown,
                ConsensusEffect::Admit,
                ConsensusEffect::DrivePbftProgress,
                ConsensusEffect::Gossip,
            ]
        );
    }
}
