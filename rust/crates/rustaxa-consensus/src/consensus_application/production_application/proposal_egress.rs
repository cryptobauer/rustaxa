use super::*;
use crate::consensus_application_runtime::{
    ConsensusEffectId, ConsensusTransportPort, ConsensusTransportReport, GossipDagBlockRequest,
    GossipPillarVoteRequest, GossipVoteBundleRequest, GossipVoteRequest,
};
use std::cell::RefCell;

#[derive(Clone)]
pub(crate) enum ProposalEgressPacket {
    Dag(GossipDagBlockRequest),
    Vote(GossipVoteRequest),
    VoteBundle(GossipVoteBundleRequest),
    Pillar(GossipPillarVoteRequest),
}

impl ProposalEgressPacket {
    pub(crate) fn effect_id(&self) -> ConsensusEffectId {
        match self {
            Self::Dag(request) => request.effect_id,
            Self::Vote(request) => request.effect_id,
            Self::VoteBundle(request) => request.effect_id,
            Self::Pillar(request) => request.effect_id,
        }
    }

    pub(crate) fn send(
        &self,
        transport: &impl ConsensusTransportPort,
    ) -> Result<ConsensusTransportReport> {
        match self {
            Self::Dag(request) => transport.gossip_dag_block(request),
            Self::Vote(request) => transport.gossip_vote(request),
            Self::VoteBundle(request) => transport.gossip_vote_bundle(request),
            Self::Pillar(request) => transport.gossip_pillar_vote(request),
        }
    }

    fn matches(&self, request: &NetworkEgressPrepareRequest) -> bool {
        if request.source_payload_id != self.effect_id().sequence
            || request.source_peer_id != [0; 64]
        {
            return false;
        }
        match self {
            Self::Dag(dag) => {
                request.family == NETWORK_EGRESS_FAMILY_DAG_BLOCK
                    && !request.rebroadcast
                    && request.object_hash == dag.block_hash
                    && request.payload_bytes == dag.block_rlp
                    && request.related_payload_bytes.is_empty()
            }
            Self::Vote(vote) => {
                request.family == NETWORK_EGRESS_FAMILY_PBFT_VOTE
                    && request.object_hash == [0; 32]
                    && request.rebroadcast == vote.rebroadcast
                    && request.payload_bytes == vote.vote_rlp
                    && request.related_payload_bytes == vote.proposed_block_rlp
            }
            Self::VoteBundle(bundle) => {
                request.family == NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE
                    && request.object_hash == [0; 32]
                    && request.rebroadcast == bundle.rebroadcast
                    && request.payload_bytes == bundle.votes_bundle_rlp
                    && request.related_payload_bytes.is_empty()
            }
            Self::Pillar(vote) => {
                request.family == crate::network_api::NETWORK_EGRESS_FAMILY_PILLAR_VOTE
                    && request.object_hash == [0; 32]
                    && request.rebroadcast == vote.rebroadcast
                    && request.payload_bytes == vote.pillar_vote_rlp
                    && request.related_payload_bytes.is_empty()
            }
        }
    }
}

struct ProposalEgressLease {
    issuer: Arc<()>,
    packet: ProposalEgressPacket,
    generation: u64,
    pending: Option<(u64, u32)>,
    planned_lanes: Vec<u32>,
    completed_lanes: usize,
    drained: Vec<u64>,
    started: bool,
    stale: bool,
}

impl ProposalEgressLease {
    fn follows(&mut self, generation: u64) -> bool {
        self.stale |= self.generation.checked_add(1) != Some(generation);
        !self.stale
    }
}

pub(crate) struct ProposalEgressCompletion {
    pub(crate) generation: u64,
    pub(crate) pending: Option<u64>,
    pub(crate) abandoned_lane: Option<u32>,
    pub(crate) complete: bool,
    pub(crate) started: bool,
    pub(crate) stale: bool,
}

thread_local! {
    static LEASE: RefCell<Option<ProposalEgressLease>> = const { RefCell::new(None) };
}

pub(crate) struct ProposalEgressGuard<'a> {
    source: &'a ApplicationScopeSource,
    // The callback re-enters Rust on this thread.
    _thread_local: std::marker::PhantomData<std::rc::Rc<()>>,
}

impl Drop for ProposalEgressGuard<'_> {
    fn drop(&mut self) {
        LEASE.with_borrow_mut(|slot| {
            if slot
                .as_ref()
                .is_some_and(|lease| Arc::ptr_eq(&lease.issuer, &self.source.identity))
            {
                *slot = None;
            }
        });
    }
}

impl ProposalEgressGuard<'_> {
    pub(crate) fn finish(self) -> Result<ProposalEgressCompletion> {
        self.source.with_proposal_lease(|lease| {
            Ok(ProposalEgressCompletion {
                generation: lease.generation,
                pending: lease.pending.map(|(token, _)| token),
                abandoned_lane: (lease.completed_lanes != lease.planned_lanes.len())
                    .then(|| lease.planned_lanes.last().copied())
                    .flatten(),
                complete: lease.pending.is_none()
                    && lease.completed_lanes != 0
                    && lease.completed_lanes == lease.planned_lanes.len()
                    && lease.drained.is_empty(),
                started: lease.started,
                stale: lease.stale,
            })
        })
    }
}

pub(super) fn requires_proposal_egress_lease(request: &NetworkEgressPrepareRequest) -> bool {
    request.source_peer_id == [0; 64]
        && matches!(
            request.family,
            NETWORK_EGRESS_FAMILY_PBFT_VOTE
                | NETWORK_EGRESS_FAMILY_PBFT_VOTES_BUNDLE
                | NETWORK_EGRESS_FAMILY_DAG_BLOCK
                | crate::network_api::NETWORK_EGRESS_FAMILY_PILLAR_VOTE
        )
}

impl ApplicationScopeSource {
    pub(crate) fn begin_proposal_egress(
        &self,
        generation: u64,
        packet: ProposalEgressPacket,
    ) -> Result<Option<ProposalEgressGuard<'_>>> {
        let action = self.read()?;
        if action.generation != generation {
            return Ok(None);
        }
        LEASE.with_borrow_mut(|slot| {
            ensure!(slot.is_none(), "PRODUCTION_PROPOSAL_EGRESS_ALREADY_ACTIVE");
            *slot = Some(ProposalEgressLease {
                issuer: Arc::clone(&self.identity),
                packet,
                generation,
                pending: None,
                planned_lanes: Vec::new(),
                completed_lanes: 0,
                drained: Vec::new(),
                started: false,
                stale: false,
            });
            Ok(Some(ProposalEgressGuard {
                source: self,
                _thread_local: std::marker::PhantomData,
            }))
        })
    }

    fn with_proposal_lease<T>(
        &self,
        operation: impl FnOnce(&mut ProposalEgressLease) -> Result<T>,
    ) -> Result<T> {
        LEASE.with_borrow_mut(|slot| {
            let lease = slot
                .as_mut()
                .filter(|lease| Arc::ptr_eq(&lease.issuer, &self.identity))
                .context("PRODUCTION_PROPOSAL_EGRESS_AUTHORITY_REQUIRED")?;
            operation(lease)
        })
    }

    pub(super) fn authorize_proposal_egress_preparation(
        &self,
        generation: u64,
        request: &NetworkEgressPrepareRequest,
    ) -> Result<()> {
        self.with_proposal_lease(|lease| {
            lease.started = true;
            ensure!(
                lease.packet.matches(request)
                    && lease.follows(generation)
                    && lease.pending.is_none()
                    && lease.drained.is_empty()
                    && lease.planned_lanes.len() == lease.completed_lanes
                    && lease.planned_lanes.len() < 16
                    && !lease.planned_lanes.contains(&request.transport_lane),
                "PRODUCTION_PROPOSAL_EGRESS_PREPARATION_MISMATCH"
            );
            Ok(())
        })
    }

    pub(super) fn note_proposal_egress_preparation(
        &self,
        generation: u64,
        token: u64,
        lane: u32,
    ) -> Result<()> {
        self.with_proposal_lease(|lease| {
            lease.pending = Some((token, lane));
            lease.generation = generation;
            Ok(())
        })
    }

    pub(super) fn authorize_proposal_egress_plan(&self, generation: u64, token: u64) -> Result<()> {
        self.with_proposal_lease(|lease| {
            ensure!(
                lease.pending.is_some_and(|(pending, _)| pending == token)
                    && lease.follows(generation),
                "PRODUCTION_PROPOSAL_EGRESS_PLAN_MISMATCH"
            );
            Ok(())
        })
    }

    pub(super) fn note_proposal_egress_plan(&self, generation: u64) -> Result<()> {
        self.with_proposal_lease(|lease| {
            let (_, lane) = lease
                .pending
                .take()
                .context("PRODUCTION_PROPOSAL_EGRESS_TOKEN_MISSING")?;
            lease.planned_lanes.push(lane);
            lease.generation = generation;
            Ok(())
        })
    }

    pub(super) fn note_proposal_egress_drain(
        &self,
        generation: u64,
        lane: u32,
        payload: u64,
        source_scoped: bool,
        batch: &NetworkEffectBatch,
    ) {
        LEASE.with_borrow_mut(|slot| {
            if let Some(lease) = slot.as_mut().filter(|lease| {
                Arc::ptr_eq(&lease.issuer, &self.identity)
                    && source_scoped
                    && lease.planned_lanes.last() == Some(&lane)
                    && lease.planned_lanes.len() == lease.completed_lanes + 1
                    && lease.packet.effect_id().sequence == payload
                    && lease.generation.checked_add(1) == Some(generation)
                    && lease.drained.is_empty()
                    && batch.status == 0
                    && batch.effects.iter().all(|effect| {
                        effect.transport_lane == lane && effect.source_payload_id == payload
                    })
            }) {
                lease.generation = generation;
                lease
                    .drained
                    .extend(batch.effects.iter().map(|effect| effect.effect_id));
                if batch.effects.is_empty() && !batch.more_available {
                    lease.completed_lanes += 1;
                }
            }
        });
    }

    pub(super) fn authorize_proposal_egress_drain(
        &self,
        generation: u64,
        lane: u32,
        payload: u64,
        source_scoped: bool,
    ) -> Option<bool> {
        LEASE.with_borrow_mut(|slot| {
            let lease = slot.as_mut().filter(|lease| {
                Arc::ptr_eq(&lease.issuer, &self.identity)
                    && source_scoped
                    && lease.packet.effect_id().sequence == payload
                    && lease.planned_lanes.last() == Some(&lane)
            })?;
            Some(
                lease.follows(generation)
                    && lease.pending.is_none()
                    && lease.completed_lanes + 1 == lease.planned_lanes.len()
                    && lease.drained.is_empty(),
            )
        })
    }

    pub(super) fn note_proposal_egress_acknowledgement(
        &self,
        generation: u64,
        ids: &[u64],
        ack: &NetworkEffectAck,
    ) {
        LEASE.with_borrow_mut(|slot| {
            if let Some(lease) = slot.as_mut().filter(|lease| {
                Arc::ptr_eq(&lease.issuer, &self.identity)
                    && !ids.is_empty()
                    && lease.drained == ids
                    && ack.status == 0
                    && ack.accepted_results as usize == ids.len()
            }) {
                if lease.follows(generation) {
                    lease.generation = generation;
                    lease.drained.clear();
                }
            }
        });
    }
}
