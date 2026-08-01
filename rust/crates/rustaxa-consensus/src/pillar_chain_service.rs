//! Native pillar-chain application ownership.
//!
//! This module owns the complete lifetime and synchronization topology for the
//! Rust pillar runtime: shared storage, startup restoration, pillar votes, the
//! canonical current-anchor snapshot, one-time vote/finalization preparation
//! registries, the outer serialization lock, and monotonic bootstrap readiness.
//! It has no CXX dependency. Task-oriented methods own storage, anchor, vote,
//! bundle, lookup, and finalization behavior. The raw state and its guard are
//! crate-private implementation details; bridge callers use only service tasks.

use crate::{
    PbftServiceReadiness, PillarBlockCreationFact, PillarBlockCreationPlan, PillarBlockLinkageFact,
    PillarBlockLinkagePlan, PillarCurrentAnchor, PillarCurrentAnchorDecisionPlan,
    PillarCurrentAnchorDecisionRequest, PillarValidatorVoteCount, PillarValidatorVoteCountChange,
    PillarVotes, load_current_pillar_block_data_storage, load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage, load_pillar_period_data_storage,
    plan_pillar_block_creation, plan_pillar_block_linkage, plan_pillar_consensus_threshold,
    plan_pillar_current_anchor_decision, plan_pillar_vote_count_changes,
    save_current_pillar_block_data_storage, save_own_pillar_block_vote_storage,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use ethereum_types::{H160, H256};
use rustaxa_storage::Storage;
use rustaxa_types::pillar::{CurrentPillarBlockDataDb, PillarBlock};
use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// Maximum unresolved pillar-finalization preparations retained by one service.
pub const MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS: usize = 16;

/// Durable pillar rows used to reconstruct the C++ manager during startup.
///
/// Missing rows are returned as empty canonical-byte vectors. The period-data
/// row, when needed, is selected from `latest_finalized.period + 1`; malformed
/// restored state and period overflow are returned as errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillarChainStartupBootstrap {
    pub own_vote_rlp: Vec<u8>,
    pub current_block_data_rlp: Vec<u8>,
    pub latest_pillar_votes_period_data_rlp: Vec<u8>,
}

/// Current-anchor decision enriched with the exact sampled native snapshot.
///
/// The generation lets callers bind later work to the decision. A missing
/// anchor is represented by `current_anchor: None`, while the planner status
/// retains the operation-specific reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillarCurrentAnchorDecisionResult {
    pub plan: PillarCurrentAnchorDecisionPlan,
    pub current_anchor: Option<PillarCurrentAnchor>,
    pub anchor_generation: u64,
}

/// Candidate fields for linkage validation against native finalized state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillarBlockLinkageRequest {
    pub pillar_block_period: u64,
    pub pillar_block_previous_hash: H256,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
}

/// External block fields used with a native validator vote-count snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillarBlockCreationRequest {
    pub pillar_block_period: u64,
    pub state_root: H256,
    pub bridge_root: H256,
    pub bridge_epoch: H256,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
}

/// Native block-creation output plus the snapshot facts retained by C++.
///
/// `anchor_generation` authenticates a later current-data apply. Current vote
/// counts preserve FinalChain order, while changes follow the deterministic
/// pillar planner's ordering and checked signed-range behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillarBlockCreationWithVoteCountsPlan {
    pub creation: PillarBlockCreationPlan,
    pub vote_count_changes: Vec<PillarValidatorVoteCountChange>,
    pub current_vote_counts: Vec<PillarValidatorVoteCount>,
    pub anchor_generation: u64,
}

/// Runtime-internal one-time preparation state for one canonical pillar vote.
///
/// The record binds canonical vote bytes and all relevance inputs to one anchor
/// generation. Trusted records are reserved for locally generated or
/// restart-restored votes; external records must be revalidated before apply.
#[derive(Debug, Clone)]
pub(crate) struct SingleVotePreparation {
    pub(crate) vote_rlp: Vec<u8>,
    pub(crate) anchor_generation: u64,
    pub(crate) period: u64,
    pub(crate) block_hash: H256,
    pub(crate) voter: H160,
    pub(crate) needs_threshold: bool,
    pub(crate) current_anchor: Option<PillarCurrentAnchor>,
    pub(crate) first_pillar_block_period: u64,
    pub(crate) pillar_blocks_interval: u64,
    pub(crate) trusted_local_or_restore: bool,
}

/// Pending single-vote preparations keyed by canonical vote hash.
///
/// Callers remove a record before applying it, making successful and rejected
/// applies one-shot. Bridge adapter code currently owns the detailed vote DTO
/// conversion while this registry and its lock are native.
#[derive(Debug, Default)]
pub(crate) struct SingleVotePreparationRegistry {
    pub(crate) entries: BTreeMap<H256, SingleVotePreparation>,
}

/// One-time prepared payload used to acknowledge pillar-block finalization.
///
/// The token registry retains this payload until storage contains the exact
/// canonical block bytes. A failed or mismatched durable lookup therefore
/// leaves the token available for a safe retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PillarBlockFinalizationPreparation {
    pub(crate) anchor_generation: u64,
    pub(crate) prepared_pillar_block_period: u64,
    pub(crate) prepared_pillar_block_rlp: Vec<u8>,
    pub(crate) matching_vote_cleanup_min_period: u64,
    pub(crate) should_emit: bool,
}

/// Runtime-owned canonical pillar-chain snapshot.
///
/// `generation` is process-local and starts at zero after restoration. Every
/// successfully persisted current-anchor replacement increments it exactly
/// once. Canonical bytes are retained for hashing, persistence parity, and
/// temporary public C++ materialization.
#[derive(Debug, Clone)]
pub(crate) struct PillarChainStateSnapshot {
    pub(crate) anchor: Option<PillarCurrentAnchor>,
    current_data_rlp: Vec<u8>,
    pub(crate) current_block_rlp: Vec<u8>,
    pub(crate) latest_finalized_block: Option<PillarBlock>,
    pub(crate) latest_finalized_block_rlp: Vec<u8>,
    pub(crate) generation: u64,
}

/// Native mutable state protected by [`PillarChainService`]'s outer mutex.
///
/// Fields are visible only to the native pillar service modules that implement
/// task-oriented operations. No raw state or guard crosses the crate boundary.
pub(crate) struct PillarChainState {
    pub(crate) storage: Arc<Storage>,
    pub(crate) votes: PillarVotes,
    pub(crate) current_anchor: RwLock<PillarChainStateSnapshot>,
    pub(crate) single_vote_preparations: Mutex<SingleVotePreparationRegistry>,
    pub(crate) pillar_block_finalization_preparations:
        Mutex<HashMap<u64, PillarBlockFinalizationPreparation>>,
    next_pillar_block_finalization_preparation_token: u64,
}

impl PillarChainState {
    /// Returns the current process-local anchor generation.
    pub(crate) fn anchor_generation(&self) -> Result<u64> {
        Ok(self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .generation)
    }

    /// Persists and publishes canonical current-pillar data for one generation.
    ///
    /// The current-anchor write lock is retained through persistence so readers
    /// never observe an in-memory anchor whose canonical storage row failed.
    /// Stale generations, malformed/noncanonical RLP, storage failures, and
    /// generation overflow leave the previous snapshot unchanged.
    pub(crate) fn apply_current_block_data(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        ensure!(
            !data_rlp.is_empty(),
            "PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"
        );
        let decoded = CurrentPillarBlockDataDb::decode_rlp(&data_rlp)
            .context("decode current pillar block data before apply")?;
        ensure!(
            decoded.encode_rlp() == data_rlp,
            "current pillar block data must use canonical RLP"
        );
        let current_block_rlp = decoded.pillar_block.encode_rlp();
        let anchor = PillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        };
        let mut snapshot = self
            .current_anchor
            .write()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        ensure!(
            snapshot.generation == expected_anchor_generation,
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );
        let generation = snapshot
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("current pillar anchor generation overflow"))?;

        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)?;
        *snapshot = PillarChainStateSnapshot {
            anchor: Some(anchor),
            current_data_rlp: data_rlp,
            current_block_rlp,
            latest_finalized_block: snapshot.latest_finalized_block.clone(),
            latest_finalized_block_rlp: snapshot.latest_finalized_block_rlp.clone(),
            generation,
        };
        Ok(())
    }

    /// Retains or reuses a generation-bound finalization preparation token.
    ///
    /// Identical canonical period/bytes at the same generation reuse their
    /// existing token. Stale-generation entries are removed first; when the
    /// bounded registry is full, the lowest token is evicted. Preserving legacy
    /// ordering, sequence overflow is checked after that cleanup and may
    /// therefore leave stale-entry pruning or one eviction applied.
    pub(crate) fn retain_finalization_preparation(
        &mut self,
        preparation: PillarBlockFinalizationPreparation,
    ) -> Result<u64> {
        let mut preparations = self
            .pillar_block_finalization_preparations
            .lock()
            .map_err(|_| anyhow!("pillar block finalization preparation lock poisoned"))?;
        preparations
            .retain(|_, retained| retained.anchor_generation == preparation.anchor_generation);
        if let Some((token, _)) = preparations.iter().find(|(_, retained)| {
            retained.anchor_generation == preparation.anchor_generation
                && retained.prepared_pillar_block_period == preparation.prepared_pillar_block_period
                && retained.prepared_pillar_block_rlp == preparation.prepared_pillar_block_rlp
        }) {
            return Ok(*token);
        }
        if preparations.len() >= MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS {
            let oldest = preparations
                .keys()
                .min()
                .copied()
                .ok_or_else(|| anyhow!("PILLAR_BLOCK_FINALIZATION_PREPARATION_CAP_EMPTY"))?;
            preparations.remove(&oldest);
        }
        let token = self
            .next_pillar_block_finalization_preparation_token
            .checked_add(1)
            .ok_or_else(|| anyhow!("PILLAR_BLOCK_FINALIZATION_TOKEN_SEQUENCE_OVERFLOW"))?;
        self.next_pillar_block_finalization_preparation_token = token;
        preparations.insert(token, preparation);
        Ok(token)
    }
}

/// Cloneable CXX-free owner of the pillar runtime.
///
/// Clones share one outer mutex and one readiness flag. `restore` validates all
/// persisted anchor bytes before publishing the service and normalizes restored
/// vote retention against the latest finalized period. Pending services reject
/// ready-required locks with `PBFT_SERVICE_PILLAR_UNAVAILABLE`.
#[derive(Clone)]
pub struct PillarChainService {
    state: Arc<Mutex<PillarChainState>>,
    readiness: PbftServiceReadiness,
}

impl PillarChainService {
    /// Restores native pillar state from shared Rust storage in pending state.
    pub fn restore(storage: Arc<Storage>) -> Result<Self> {
        let current_data_rlp = load_current_pillar_block_data_storage(storage.as_ref())?;
        let latest_finalized_block_rlp = load_latest_pillar_block_storage(storage.as_ref())?;
        let snapshot =
            decode_pillar_chain_snapshot(current_data_rlp, latest_finalized_block_rlp, 0)
                .context("restore current pillar anchor snapshot")?;
        let mut state = PillarChainState {
            storage,
            votes: PillarVotes::new(),
            current_anchor: RwLock::new(snapshot),
            single_vote_preparations: Mutex::new(SingleVotePreparationRegistry::default()),
            pillar_block_finalization_preparations: Mutex::new(HashMap::new()),
            next_pillar_block_finalization_preparation_token: 0,
        };
        {
            let snapshot = state
                .current_anchor
                .read()
                .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
            if let Some(latest) = &snapshot.latest_finalized_block {
                state.votes.erase_votes(latest.period.saturating_add(1));
            }
        }
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            readiness: PbftServiceReadiness::pending(),
        })
    }

    /// Reports whether startup restoration/replay has been published complete.
    pub fn is_ready(&self) -> bool {
        self.readiness.is_ready()
    }

    /// Publishes the monotonic pillar bootstrap transition.
    pub fn mark_ready(&self) {
        self.readiness.mark_ready();
    }

    /// Completes startup bootstrap after proving the pending state is lockable.
    ///
    /// This preserves the durable-before-publish lifecycle contract: mutex
    /// poisoning fails without publishing readiness, while success performs the
    /// monotonic release-store transition used by all later live operations.
    pub fn complete_bootstrap(&self) -> Result<()> {
        drop(self.lock(false)?);
        self.mark_ready();
        Ok(())
    }

    /// Applies canonical current-pillar data for an exact sampled generation.
    ///
    /// Live readiness is required. Persistence completes while the snapshot
    /// write lock is held and before publication; stale generations, malformed
    /// or noncanonical RLP, storage failure, and overflow leave the published
    /// snapshot unchanged.
    pub fn apply_planned_current_block_data(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        self.lock(true)?
            .apply_current_block_data(data_rlp, expected_anchor_generation)
    }

    /// Persists this node's canonical own-vote bytes for restart recovery.
    ///
    /// Live readiness is required. Empty payloads and storage errors are
    /// returned unchanged; admission into the in-memory vote index remains a
    /// separate pillar-vote task.
    pub fn apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        let state = self.lock(true)?;
        save_own_pillar_block_vote_storage(state.storage.as_ref(), &vote_rlp)
    }

    /// Loads the durable rows needed by startup reconstruction.
    ///
    /// This is the only task API that intentionally accepts pending readiness.
    /// The current row comes from the validated restored snapshot. If a latest
    /// finalized block exists, its successor period selects the opaque PBFT
    /// period-data row; checked overflow and storage errors abort bootstrap.
    pub fn load_startup_bootstrap(&self) -> Result<PillarChainStartupBootstrap> {
        let state = self.lock(false)?;
        let own_vote_rlp = load_own_pillar_block_vote_storage(state.storage.as_ref())?;
        let snapshot = state
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let current_block_data_rlp = snapshot.current_data_rlp.clone();
        let latest_pillar_votes_period_data_rlp =
            if let Some(latest_block) = &snapshot.latest_finalized_block {
                let vote_period = latest_block
                    .period
                    .checked_add(1)
                    .ok_or_else(|| anyhow!("latest pillar block period overflow"))?;
                load_pillar_period_data_storage(state.storage.as_ref(), vote_period)?
            } else {
                Vec::new()
            };
        Ok(PillarChainStartupBootstrap {
            own_vote_rlp,
            current_block_data_rlp,
            latest_pillar_votes_period_data_rlp,
        })
    }

    /// Plans one current-anchor task against a single ready snapshot.
    ///
    /// The native operation enum rejects no tags because FFI tag validation
    /// belongs in the bridge. The result always carries the sampled anchor and
    /// generation, including terminal missing/mismatch outcomes.
    pub fn plan_current_anchor_decision(
        &self,
        request: PillarCurrentAnchorDecisionRequest,
    ) -> Result<PillarCurrentAnchorDecisionResult> {
        let state = self.lock(true)?;
        let snapshot = state
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        Ok(PillarCurrentAnchorDecisionResult {
            plan: plan_pillar_current_anchor_decision(snapshot.anchor, request),
            current_anchor: snapshot.anchor,
            anchor_generation: snapshot.generation,
        })
    }

    /// Computes the strict-majority threshold for a supplied total vote count.
    ///
    /// Live readiness is required even though the arithmetic is pure, matching
    /// the previous bridge task contract. Every `u64` input is representable.
    pub fn consensus_threshold(&self, total_vote_count: u64) -> Result<u64> {
        drop(self.lock(true)?);
        Ok(plan_pillar_consensus_threshold(total_vote_count))
    }

    /// Samples the ready current-anchor generation before an external query.
    ///
    /// Callers must not retain a state guard across that query and must pass the
    /// returned generation to [`Self::plan_block_creation_for_generation`].
    pub fn sample_anchor_generation(&self) -> Result<u64> {
        self.lock(true)?.anchor_generation()
    }

    /// Plans block creation from native request and validator-count DTOs.
    ///
    /// This ready-required variant holds the outer state lock throughout the
    /// operation and returns the sampled generation for later durable apply.
    pub fn plan_block_creation(
        &self,
        request: PillarBlockCreationRequest,
        current_vote_counts: Vec<PillarValidatorVoteCount>,
    ) -> Result<PillarBlockCreationWithVoteCountsPlan> {
        let state = self.lock(true)?;
        plan_block_creation_from_state(&state, request, current_vote_counts, None)
    }

    /// Plans block creation only if an earlier external query's generation is current.
    ///
    /// The ready state is re-locked after the caller's external query. A
    /// generation mismatch returns `PILLAR_BLOCK_CREATION_STALE_ANCHOR` before
    /// any plan is returned or state is published.
    pub fn plan_block_creation_for_generation(
        &self,
        request: PillarBlockCreationRequest,
        current_vote_counts: Vec<PillarValidatorVoteCount>,
        expected_anchor_generation: u64,
    ) -> Result<PillarBlockCreationWithVoteCountsPlan> {
        let state = self.lock(true)?;
        plan_block_creation_from_state(
            &state,
            request,
            current_vote_counts,
            Some(expected_anchor_generation),
        )
    }

    /// Validates candidate parent linkage against native finalized state.
    ///
    /// Live readiness is required. Missing finalized state, interval overflow,
    /// period mismatch, and hash mismatch are represented by the native plan's
    /// stable status rather than bridge-shaped fields.
    pub fn plan_block_linkage(
        &self,
        request: PillarBlockLinkageRequest,
    ) -> Result<PillarBlockLinkagePlan> {
        let state = self.lock(true)?;
        let snapshot = state
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let (last_finalized_period, last_finalized_hash) = snapshot
            .latest_finalized_block
            .as_ref()
            .map(|block| (Some(block.period), Some(block.hash())))
            .unwrap_or((None, None));
        plan_pillar_block_linkage(PillarBlockLinkageFact {
            pillar_block_period: request.pillar_block_period,
            pillar_block_previous_hash: request.pillar_block_previous_hash,
            first_pillar_block_period: request.first_pillar_block_period,
            pillar_blocks_interval: request.pillar_blocks_interval,
            last_finalized_period,
            last_finalized_hash,
        })
    }

    /// Returns canonical latest-finalized block bytes for compatibility queries.
    ///
    /// Live readiness is required. Missing finalized state returns an empty
    /// vector, and the bytes are cloned directly from the validated snapshot.
    pub fn latest_finalized_block_rlp(&self) -> Result<Vec<u8>> {
        let state = self.lock(true)?;
        Ok(state
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .latest_finalized_block_rlp
            .clone())
    }

    /// Borrows the outer serialized state for native pillar task composition.
    ///
    /// Callers must drop the returned guard before FinalChain or C++ calls.
    /// `require_ready` rejects live work until bootstrap completion, while
    /// startup restoration may explicitly lock the pending service.
    pub(crate) fn lock(&self, require_ready: bool) -> Result<PillarChainGuard<'_>> {
        if require_ready && !self.readiness.is_ready() {
            bail!("PBFT_SERVICE_PILLAR_UNAVAILABLE");
        }
        Ok(PillarChainGuard {
            guard: self
                .state
                .lock()
                .map_err(|_| anyhow!("PBFT service pillar lock poisoned"))?,
        })
    }
}

fn plan_block_creation_from_state(
    state: &PillarChainState,
    request: PillarBlockCreationRequest,
    current_vote_counts: Vec<PillarValidatorVoteCount>,
    expected_anchor_generation: Option<u64>,
) -> Result<PillarBlockCreationWithVoteCountsPlan> {
    let snapshot = state
        .current_anchor
        .read()
        .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
    if let Some(expected) = expected_anchor_generation {
        ensure!(
            snapshot.generation == expected,
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );
    }
    let (last_finalized_period, last_finalized_hash) = snapshot
        .latest_finalized_block
        .as_ref()
        .map(|block| (Some(block.period), Some(block.hash())))
        .unwrap_or((None, None));
    let creation = plan_pillar_block_creation(PillarBlockCreationFact {
        pillar_block_period: request.pillar_block_period,
        state_root: request.state_root,
        bridge_root: request.bridge_root,
        bridge_epoch: request.bridge_epoch,
        first_pillar_block_period: request.first_pillar_block_period,
        pillar_blocks_interval: request.pillar_blocks_interval,
        last_finalized_period,
        last_finalized_hash,
    })?;
    let previous_vote_counts = if request.pillar_block_period == request.first_pillar_block_period {
        Vec::new()
    } else {
        ensure!(
            !snapshot.current_data_rlp.is_empty(),
            "current pillar vote-count snapshot is missing"
        );
        CurrentPillarBlockDataDb::decode_rlp(&snapshot.current_data_rlp)?
            .vote_counts
            .into_iter()
            .map(|value| PillarValidatorVoteCount {
                address: value.address,
                vote_count: value.vote_count,
            })
            .collect()
    };
    let vote_count_changes =
        plan_pillar_vote_count_changes(&current_vote_counts, &previous_vote_counts)?;
    Ok(PillarBlockCreationWithVoteCountsPlan {
        creation,
        vote_count_changes,
        current_vote_counts,
        anchor_generation: snapshot.generation,
    })
}

/// Native state guard used by pillar task implementations and characterization.
///
/// The guard proves the outer mutex is owned by `rustaxa-consensus`. It must
/// never survive an external executor call, and production bridge code uses
/// task-oriented service APIs instead.
pub(crate) struct PillarChainGuard<'a> {
    guard: MutexGuard<'a, PillarChainState>,
}

impl Deref for PillarChainGuard<'_> {
    type Target = PillarChainState;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl DerefMut for PillarChainGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

/// Decodes and validates canonical persisted pillar snapshot rows.
///
/// Production construction uses [`PillarChainService::restore`]. Malformed RLP,
/// noncanonical bytes, and an inconsistent current/latest relationship fail
/// without publishing state.
fn decode_pillar_chain_snapshot(
    current_data_rlp: Vec<u8>,
    latest_finalized_block_rlp: Vec<u8>,
    generation: u64,
) -> Result<PillarChainStateSnapshot> {
    let latest_finalized_block = if latest_finalized_block_rlp.is_empty() {
        None
    } else {
        let block = PillarBlock::decode_rlp(&latest_finalized_block_rlp)?;
        ensure!(
            block.encode_rlp() == latest_finalized_block_rlp,
            "latest finalized pillar block must use canonical RLP"
        );
        Some(block)
    };
    if current_data_rlp.is_empty() {
        return Ok(PillarChainStateSnapshot {
            anchor: None,
            current_data_rlp,
            current_block_rlp: Vec::new(),
            latest_finalized_block,
            latest_finalized_block_rlp,
            generation,
        });
    }
    let decoded = CurrentPillarBlockDataDb::decode_rlp(&current_data_rlp)?;
    ensure!(
        decoded.encode_rlp() == current_data_rlp,
        "current pillar block data must use canonical RLP"
    );
    let snapshot = PillarChainStateSnapshot {
        anchor: Some(PillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        }),
        current_block_rlp: decoded.pillar_block.encode_rlp(),
        current_data_rlp,
        latest_finalized_block,
        latest_finalized_block_rlp,
        generation,
    };
    validate_current_latest_relationship(&snapshot)?;
    Ok(snapshot)
}

fn validate_current_latest_relationship(snapshot: &PillarChainStateSnapshot) -> Result<()> {
    let Some(current) = snapshot.anchor else {
        return Ok(());
    };
    let Some(latest) = &snapshot.latest_finalized_block else {
        return Ok(());
    };
    if current.period < latest.period {
        bail!("PILLAR_ANCHOR_LATEST_AHEAD_OF_CURRENT");
    }
    if current.period == latest.period {
        ensure!(
            current.hash == latest.hash(),
            "PILLAR_ANCHOR_CURRENT_LATEST_HASH_MISMATCH"
        );
        return Ok(());
    }
    ensure!(
        CurrentPillarBlockDataDb::decode_rlp(&snapshot.current_data_rlp)?
            .pillar_block
            .previous_pillar_block_hash
            == latest.hash(),
        "PILLAR_ANCHOR_BROKEN_SUCCESSOR_PREVIOUS_HASH"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save_finalized_pillar_block_storage;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_types::pillar::CurrentPillarBlockDataDb;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_storage(name: &str) -> Arc<Storage> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time is available")
            .as_nanos();
        let path: PathBuf = std::env::temp_dir().join(format!("{name}_{nonce}"));
        Arc::new(Storage::new(Config::new(path)).expect("storage opens"))
    }

    fn pillar_block(period: u64, previous_pillar_block_hash: H256) -> PillarBlock {
        PillarBlock {
            period,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash,
            bridge_root: H256::from_low_u64_be(2),
            epoch: 3,
            validator_vote_count_changes: Vec::new(),
        }
    }

    fn current_data(block: PillarBlock, vote_counts: Vec<PillarValidatorVoteCount>) -> Vec<u8> {
        CurrentPillarBlockDataDb {
            pillar_block: block,
            vote_counts: vote_counts
                .into_iter()
                .map(|value| rustaxa_types::pillar::ValidatorVoteCount {
                    address: value.address,
                    vote_count: value.vote_count,
                })
                .collect(),
        }
        .encode_rlp()
    }

    #[test]
    fn snapshot_decoder_accepts_empty_current_exact_and_successor_relationships() {
        assert!(decode_pillar_chain_snapshot(Vec::new(), Vec::new(), 0).is_ok());

        let current_without_latest = pillar_block(41, H256::from_low_u64_be(10));
        assert!(
            decode_pillar_chain_snapshot(
                current_data(current_without_latest, Vec::new()),
                Vec::new(),
                0,
            )
            .is_ok()
        );

        let exact = pillar_block(41, H256::from_low_u64_be(11));
        assert!(
            decode_pillar_chain_snapshot(
                current_data(exact.clone(), Vec::new()),
                exact.encode_rlp(),
                0,
            )
            .is_ok()
        );

        let latest = pillar_block(41, H256::from_low_u64_be(12));
        let successor = pillar_block(42, latest.hash());
        assert!(
            decode_pillar_chain_snapshot(
                current_data(successor, Vec::new()),
                latest.encode_rlp(),
                0,
            )
            .is_ok()
        );

        let latest_before_gap = pillar_block(4, H256::from_low_u64_be(13));
        let current_after_gap = pillar_block(8, latest_before_gap.hash());
        assert!(
            decode_pillar_chain_snapshot(
                current_data(current_after_gap, Vec::new()),
                latest_before_gap.encode_rlp(),
                0,
            )
            .is_ok()
        );
    }

    #[test]
    fn snapshot_decoder_rejects_invalid_latest_relationships() {
        let latest_ahead = pillar_block(42, H256::from_low_u64_be(10));
        let current_behind = pillar_block(41, H256::from_low_u64_be(1));
        assert!(
            decode_pillar_chain_snapshot(
                current_data(current_behind, Vec::new()),
                latest_ahead.encode_rlp(),
                0,
            )
            .expect_err("latest period ahead of current must fail")
            .to_string()
            .contains("PILLAR_ANCHOR_LATEST_AHEAD_OF_CURRENT")
        );

        let latest_same_period = pillar_block(41, H256::from_low_u64_be(10));
        let mismatched_current = pillar_block(41, H256::from_low_u64_be(12));
        assert!(
            decode_pillar_chain_snapshot(
                current_data(mismatched_current, Vec::new()),
                latest_same_period.encode_rlp(),
                0,
            )
            .expect_err("same-period hash mismatch must fail")
            .to_string()
            .contains("PILLAR_ANCHOR_CURRENT_LATEST_HASH_MISMATCH")
        );

        for current_period in [42, 43] {
            let latest = pillar_block(41, H256::from_low_u64_be(10));
            let bad_successor = pillar_block(current_period, H256::from_low_u64_be(12));
            assert!(
                decode_pillar_chain_snapshot(
                    current_data(bad_successor, Vec::new()),
                    latest.encode_rlp(),
                    0,
                )
                .expect_err("successor with wrong previous hash must fail")
                .to_string()
                .contains("PILLAR_ANCHOR_BROKEN_SUCCESSOR_PREVIOUS_HASH")
            );
        }
    }

    #[test]
    fn restore_rejects_malformed_persisted_current_data() {
        let storage = temp_storage("pillar_owner_restore_malformed_current");
        save_current_pillar_block_data_storage(storage.as_ref(), &[0xc1, 0x01])
            .expect("malformed current bytes should persist");
        let error = match PillarChainService::restore(storage) {
            Ok(_) => panic!("malformed current data must fail"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("restore current pillar anchor snapshot")
        );
    }

    #[test]
    fn restore_rejects_malformed_latest_finalized_pillar_block() {
        let storage = temp_storage("pillar_owner_restore_malformed_latest");
        save_finalized_pillar_block_storage(storage.as_ref(), 42, &[0xc1, 0x01])
            .expect("malformed latest bytes should persist");
        let error = match PillarChainService::restore(storage) {
            Ok(_) => panic!("malformed latest block must fail"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("restore current pillar anchor snapshot")
        );
    }

    #[test]
    fn restore_starts_pending_with_empty_generation_zero_snapshot() {
        let service =
            PillarChainService::restore(temp_storage("pillar_owner_restore")).expect("restore");

        assert!(!service.is_ready());
        assert!(service.lock(true).is_err());
        let state = service.lock(false).expect("startup lock");
        assert_eq!(state.anchor_generation().expect("generation"), 0);
        assert!(
            state
                .current_anchor
                .read()
                .expect("snapshot")
                .anchor
                .is_none()
        );
    }

    #[test]
    fn clones_share_readiness_and_outer_lock_state() {
        let service =
            PillarChainService::restore(temp_storage("pillar_owner_clone")).expect("restore");
        let clone = service.clone();

        clone.mark_ready();
        assert!(service.is_ready());
        {
            let mut state = clone.lock(true).expect("clone lock");
            state.next_pillar_block_finalization_preparation_token = 41;
        }
        assert_eq!(
            service
                .lock(true)
                .expect("original lock")
                .next_pillar_block_finalization_preparation_token,
            41
        );
    }

    #[test]
    fn finalization_tokens_are_monotonic_reused_and_generation_scoped() {
        let service =
            PillarChainService::restore(temp_storage("pillar_owner_tokens")).expect("restore");
        let mut state = service.lock(false).expect("lock");
        let first = PillarBlockFinalizationPreparation {
            anchor_generation: 0,
            prepared_pillar_block_period: 10,
            prepared_pillar_block_rlp: vec![0xc0],
            matching_vote_cleanup_min_period: 11,
            should_emit: true,
        };
        assert_eq!(
            state
                .retain_finalization_preparation(first.clone())
                .expect("first token"),
            1
        );
        assert_eq!(
            state
                .retain_finalization_preparation(first)
                .expect("reused token"),
            1
        );
        assert_eq!(
            state
                .retain_finalization_preparation(PillarBlockFinalizationPreparation {
                    anchor_generation: 1,
                    prepared_pillar_block_period: 20,
                    prepared_pillar_block_rlp: vec![0xc1, 0x80],
                    matching_vote_cleanup_min_period: 21,
                    should_emit: false,
                })
                .expect("next generation token"),
            2
        );
        let preparations = state
            .pillar_block_finalization_preparations
            .lock()
            .expect("registry");
        assert_eq!(preparations.len(), 1);
        assert!(preparations.contains_key(&2));
    }

    #[test]
    fn finalization_preparation_capacity_evicts_the_lowest_token() {
        let service = PillarChainService::restore(temp_storage("pillar_owner_token_capacity"))
            .expect("restore");
        let mut state = service.lock(false).expect("lock");
        for period in 1..=MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS as u64 {
            let token = state
                .retain_finalization_preparation(PillarBlockFinalizationPreparation {
                    anchor_generation: 0,
                    prepared_pillar_block_period: period,
                    prepared_pillar_block_rlp: vec![period as u8],
                    matching_vote_cleanup_min_period: period + 1,
                    should_emit: true,
                })
                .expect("bounded preparation");
            assert_eq!(token, period);
        }

        let replacement = state
            .retain_finalization_preparation(PillarBlockFinalizationPreparation {
                anchor_generation: 0,
                prepared_pillar_block_period: 17,
                prepared_pillar_block_rlp: vec![17],
                matching_vote_cleanup_min_period: 18,
                should_emit: false,
            })
            .expect("replacement preparation");
        assert_eq!(replacement, 17);

        let preparations = state
            .pillar_block_finalization_preparations
            .lock()
            .expect("registry");
        assert_eq!(
            preparations.len(),
            MAX_PILLAR_BLOCK_FINALIZATION_PREPARATIONS
        );
        assert!(!preparations.contains_key(&1));
        assert!(preparations.contains_key(&17));
    }

    #[test]
    fn current_anchor_apply_increments_generation_and_restore_resets_process_generation() {
        let storage = temp_storage("pillar_owner_generation");
        let service = PillarChainService::restore(storage.clone()).expect("restore");
        let current = CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period: 7,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::zero(),
                bridge_root: H256::from_low_u64_be(2),
                epoch: 3,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: Vec::new(),
        }
        .encode_rlp();

        {
            let state = service.lock(false).expect("lock");
            state
                .apply_current_block_data(current.clone(), 0)
                .expect("first generation apply");
            assert_eq!(state.anchor_generation().expect("generation"), 1);
            assert_eq!(
                state
                    .apply_current_block_data(current.clone(), 0)
                    .expect_err("stale generation")
                    .to_string(),
                "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
            );
            assert_eq!(state.anchor_generation().expect("generation"), 1);
        }

        let restored = PillarChainService::restore(storage).expect("restart restore");
        let state = restored.lock(false).expect("restored lock");
        let snapshot = state.current_anchor.read().expect("restored snapshot");
        assert_eq!(snapshot.generation, 0);
        assert_eq!(snapshot.anchor.expect("anchor").period, 7);
        assert_eq!(snapshot.current_data_rlp, current);
    }

    #[test]
    fn malformed_apply_current_data_preserves_durable_and_snapshot_state() {
        let storage = temp_storage("pillar_owner_malformed_apply_unchanged");
        let service = PillarChainService::restore(storage.clone()).expect("restore");
        service.complete_bootstrap().expect("ready");
        let current = current_data(pillar_block(41, H256::from_low_u64_be(10)), Vec::new());
        service
            .apply_planned_current_block_data(current.clone(), 0)
            .expect("valid apply");

        let before = {
            let state = service.lock(false).expect("lock");
            let snapshot = state.current_anchor.read().expect("snapshot read");
            (
                state.anchor_generation().expect("generation"),
                snapshot.anchor,
                snapshot.current_data_rlp.clone(),
            )
        };

        assert!(
            service
                .apply_planned_current_block_data(vec![0xc1, 0x01], before.0)
                .is_err()
        );

        let after = {
            let state = service.lock(false).expect("lock");
            let snapshot = state.current_anchor.read().expect("snapshot read");
            (
                state.anchor_generation().expect("generation"),
                snapshot.anchor,
                snapshot.current_data_rlp.clone(),
            )
        };
        assert_eq!(before.0, after.0);
        assert_eq!(before.1, after.1);
        assert_eq!(before.2, after.2);
        assert_eq!(
            load_current_pillar_block_data_storage(storage.as_ref()).expect("durable current"),
            current
        );
    }

    #[test]
    fn live_tasks_require_ready_while_startup_bootstrap_accepts_pending() {
        let service =
            PillarChainService::restore(temp_storage("pillar_native_readiness")).expect("restore");

        assert_eq!(
            service
                .load_startup_bootstrap()
                .expect("pending startup load"),
            PillarChainStartupBootstrap {
                own_vote_rlp: Vec::new(),
                current_block_data_rlp: Vec::new(),
                latest_pillar_votes_period_data_rlp: Vec::new(),
            }
        );
        assert_eq!(
            service
                .consensus_threshold(10)
                .expect_err("live threshold must reject pending")
                .to_string(),
            "PBFT_SERVICE_PILLAR_UNAVAILABLE"
        );
        assert_eq!(
            service
                .latest_finalized_block_rlp()
                .expect_err("live latest query must reject pending")
                .to_string(),
            "PBFT_SERVICE_PILLAR_UNAVAILABLE"
        );
        for error in [
            service
                .apply_planned_current_block_data(Vec::new(), 0)
                .expect_err("live apply must reject pending"),
            service
                .apply_own_vote(vec![0xc0])
                .expect_err("live own vote must reject pending"),
            service
                .plan_current_anchor_decision(
                    PillarCurrentAnchorDecisionRequest::SelectPreviousPeriod { pbft_period: 1 },
                )
                .expect_err("live anchor decision must reject pending"),
            service
                .plan_block_linkage(PillarBlockLinkageRequest {
                    pillar_block_period: 0,
                    pillar_block_previous_hash: H256::zero(),
                    first_pillar_block_period: 0,
                    pillar_blocks_interval: 1,
                })
                .expect_err("live linkage must reject pending"),
            service
                .plan_block_creation(
                    PillarBlockCreationRequest {
                        pillar_block_period: 0,
                        state_root: H256::zero(),
                        bridge_root: H256::zero(),
                        bridge_epoch: H256::zero(),
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 1,
                    },
                    Vec::new(),
                )
                .expect_err("live creation must reject pending"),
            service
                .sample_anchor_generation()
                .expect_err("live generation sample must reject pending"),
        ] {
            assert_eq!(error.to_string(), "PBFT_SERVICE_PILLAR_UNAVAILABLE");
        }

        service.complete_bootstrap().expect("publish bootstrap");
        assert_eq!(service.consensus_threshold(10).expect("threshold"), 6);
    }

    #[test]
    fn startup_bootstrap_derives_successor_period_from_restored_latest_block() {
        let storage = temp_storage("pillar_native_bootstrap");
        let latest = pillar_block(42, H256::from_low_u64_be(9));
        let current = current_data(latest.clone(), Vec::new());
        save_current_pillar_block_data_storage(storage.as_ref(), &current)
            .expect("save current data");
        save_finalized_pillar_block_storage(storage.as_ref(), latest.period, &latest.encode_rlp())
            .expect("save latest");
        save_own_pillar_block_vote_storage(storage.as_ref(), &[0xc1, 0x01]).expect("save own vote");
        storage
            .period()
            .write(43, &[0xc1, 0x02])
            .expect("save successor period data");

        let service = PillarChainService::restore(storage).expect("restore");
        assert_eq!(
            service.load_startup_bootstrap().expect("bootstrap"),
            PillarChainStartupBootstrap {
                own_vote_rlp: vec![0xc1, 0x01],
                current_block_data_rlp: current,
                latest_pillar_votes_period_data_rlp: vec![0xc1, 0x02],
            }
        );
    }

    #[test]
    fn native_anchor_tasks_preserve_durable_generation_and_snapshot_semantics() {
        let storage = temp_storage("pillar_native_anchor_tasks");
        let service = PillarChainService::restore(storage.clone()).expect("restore");
        service.complete_bootstrap().expect("ready");
        let block = pillar_block(7, H256::zero());
        let data = current_data(block.clone(), Vec::new());

        service
            .apply_planned_current_block_data(data.clone(), 0)
            .expect("generation-bound apply");
        service
            .apply_own_vote(vec![0xc1, 0x04])
            .expect("own vote apply");
        assert_eq!(
            load_current_pillar_block_data_storage(storage.as_ref()).expect("durable current"),
            data
        );
        assert_eq!(
            load_own_pillar_block_vote_storage(storage.as_ref()).expect("durable own vote"),
            vec![0xc1, 0x04]
        );
        assert_eq!(
            service
                .apply_planned_current_block_data(current_data(block.clone(), Vec::new()), 0)
                .expect_err("stale apply")
                .to_string(),
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );

        let decision = service
            .plan_current_anchor_decision(PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: Some(block.hash()),
            })
            .expect("anchor decision");
        assert!(decision.plan.selected);
        assert_eq!(decision.current_anchor.expect("anchor").period, 7);
        assert_eq!(decision.anchor_generation, 1);
        assert!(
            service
                .latest_finalized_block_rlp()
                .expect("latest bytes")
                .is_empty()
        );
    }

    #[test]
    fn native_linkage_and_block_creation_use_restored_finalized_and_vote_count_state() {
        let storage = temp_storage("pillar_native_planning");
        let latest = pillar_block(10, H256::from_low_u64_be(9));
        let previous_counts = vec![
            PillarValidatorVoteCount {
                address: H160::from_low_u64_be(1),
                vote_count: 3,
            },
            PillarValidatorVoteCount {
                address: H160::from_low_u64_be(2),
                vote_count: 8,
            },
        ];
        save_current_pillar_block_data_storage(
            storage.as_ref(),
            &current_data(latest.clone(), previous_counts),
        )
        .expect("save current");
        save_finalized_pillar_block_storage(storage.as_ref(), 10, &latest.encode_rlp())
            .expect("save latest");
        let service = PillarChainService::restore(storage).expect("restore");
        service.complete_bootstrap().expect("ready");

        let linkage = service
            .plan_block_linkage(PillarBlockLinkageRequest {
                pillar_block_period: 20,
                pillar_block_previous_hash: latest.hash(),
                first_pillar_block_period: 10,
                pillar_blocks_interval: 10,
            })
            .expect("linkage");
        assert!(linkage.valid);
        assert_eq!(
            service.latest_finalized_block_rlp().expect("latest bytes"),
            latest.encode_rlp()
        );

        let request = PillarBlockCreationRequest {
            pillar_block_period: 20,
            state_root: H256::from_low_u64_be(0xa1),
            bridge_root: H256::from_low_u64_be(0xb2),
            bridge_epoch: H256::from_low_u64_be(0xc3),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 10,
        };
        let current_counts = vec![
            PillarValidatorVoteCount {
                address: H160::from_low_u64_be(1),
                vote_count: 3,
            },
            PillarValidatorVoteCount {
                address: H160::from_low_u64_be(3),
                vote_count: 9,
            },
        ];
        let plan = service
            .plan_block_creation_for_generation(request, current_counts, 0)
            .expect("generation-bound plan");
        assert!(plan.creation.valid);
        assert_eq!(plan.creation.previous_pillar_block_hash, latest.hash());
        assert_eq!(plan.anchor_generation, 0);
        assert_eq!(plan.vote_count_changes.len(), 2);
        assert_eq!(plan.vote_count_changes[0].address, H160::from_low_u64_be(2));
        assert_eq!(plan.vote_count_changes[0].vote_count_change, -8);
        assert_eq!(plan.vote_count_changes[1].address, H160::from_low_u64_be(3));
        assert_eq!(plan.vote_count_changes[1].vote_count_change, 9);
        assert_eq!(
            service
                .plan_block_creation_for_generation(request, Vec::new(), 1)
                .expect_err("stale generation")
                .to_string(),
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );
    }
}
