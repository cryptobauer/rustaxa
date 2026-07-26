//! Native pillar-chain application ownership.
//!
//! This module owns the complete lifetime and synchronization topology for the
//! Rust pillar runtime: shared storage, startup restoration, pillar votes, the
//! canonical current-anchor snapshot, one-time vote/finalization preparation
//! registries, the outer serialization lock, and monotonic bootstrap readiness.
//! It has no CXX dependency. Bridge code may temporarily borrow
//! [`PillarChainGuard`] to adapt existing FFI DTOs, but must release that guard
//! before calling FinalChain or any C++ executor.

use crate::{
    PbftServiceReadiness, PillarCurrentAnchor, PillarVotes, load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage, save_current_pillar_block_data_storage,
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

/// Runtime-internal one-time preparation state for one canonical pillar vote.
///
/// The record binds canonical vote bytes and all relevance inputs to one anchor
/// generation. Trusted records are reserved for locally generated or
/// restart-restored votes; external records must be revalidated before apply.
#[derive(Debug, Clone)]
pub struct SingleVotePreparation {
    pub vote_rlp: Vec<u8>,
    pub anchor_generation: u64,
    pub period: u64,
    pub block_hash: H256,
    pub voter: H160,
    pub needs_threshold: bool,
    pub current_anchor: Option<PillarCurrentAnchor>,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
    pub trusted_local_or_restore: bool,
}

/// Pending single-vote preparations keyed by canonical vote hash.
///
/// Callers remove a record before applying it, making successful and rejected
/// applies one-shot. Bridge adapter code currently owns the detailed vote DTO
/// conversion while this registry and its lock are native.
#[derive(Debug, Default)]
pub struct SingleVotePreparationRegistry {
    pub entries: BTreeMap<H256, SingleVotePreparation>,
}

/// One-time prepared payload used to acknowledge pillar-block finalization.
///
/// The token registry retains this payload until storage contains the exact
/// canonical block bytes. A failed or mismatched durable lookup therefore
/// leaves the token available for a safe retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PillarBlockFinalizationPreparation {
    pub anchor_generation: u64,
    pub prepared_pillar_block_period: u64,
    pub prepared_pillar_block_rlp: Vec<u8>,
    pub matching_vote_cleanup_min_period: u64,
    pub should_emit: bool,
}

/// Runtime-owned canonical pillar-chain snapshot.
///
/// `generation` is process-local and starts at zero after restoration. Every
/// successfully persisted current-anchor replacement increments it exactly
/// once. Canonical bytes are retained for hashing, persistence parity, and
/// temporary public C++ materialization.
#[derive(Debug, Clone)]
pub struct PillarChainStateSnapshot {
    pub anchor: Option<PillarCurrentAnchor>,
    pub current_data_rlp: Vec<u8>,
    pub current_block_rlp: Vec<u8>,
    pub latest_finalized_block: Option<PillarBlock>,
    pub latest_finalized_block_rlp: Vec<u8>,
    pub generation: u64,
}

/// Native mutable state protected by [`PillarChainService`]'s outer mutex.
///
/// The public fields are a temporary bridge-adapter escape hatch. They do not
/// cross CXX and must only be accessed through [`PillarChainGuard`]. New native
/// behavior should prefer task-oriented service methods so this surface can be
/// narrowed when the remaining bridge orchestration moves into this crate.
pub struct PillarChainState {
    pub storage: Arc<Storage>,
    pub votes: PillarVotes,
    pub current_anchor: RwLock<PillarChainStateSnapshot>,
    pub single_vote_preparations: Mutex<SingleVotePreparationRegistry>,
    pub pillar_block_finalization_preparations:
        Mutex<HashMap<u64, PillarBlockFinalizationPreparation>>,
    pub next_pillar_block_finalization_preparation_token: u64,
}

impl PillarChainState {
    /// Returns the current process-local anchor generation.
    pub fn anchor_generation(&self) -> Result<u64> {
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
    pub fn apply_current_block_data(
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
    pub fn retain_finalization_preparation(
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

    /// Borrows the outer serialized state for temporary bridge adaptation.
    ///
    /// Callers must drop the returned guard before FinalChain or C++ calls.
    /// `require_ready` rejects live work until bootstrap completion, while
    /// startup restoration may explicitly lock the pending service.
    pub fn lock(&self, require_ready: bool) -> Result<PillarChainGuard<'_>> {
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

/// Temporary native state guard used by the bridge adapter.
///
/// The guard proves the outer mutex is owned by `rustaxa-consensus`, not by the
/// CXX bridge. It dereferences to native state only to support the bounded
/// bridge migration and must never survive an external executor call.
pub struct PillarChainGuard<'a> {
    guard: MutexGuard<'a, PillarChainState>,
}

impl PillarChainGuard<'_> {
    /// Returns the guarded state for temporary bridge field adaptation.
    pub fn state(&self) -> &PillarChainState {
        &self.guard
    }

    /// Returns mutable guarded state for temporary bridge field adaptation.
    pub fn state_mut(&mut self) -> &mut PillarChainState {
        &mut self.guard
    }
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
/// This is exposed for focused bridge boundary characterization while those
/// tests migrate to the native owner. Production construction should use
/// [`PillarChainService::restore`]. Malformed RLP, noncanonical bytes, and an
/// inconsistent current/latest relationship fail without publishing state.
pub fn decode_pillar_chain_snapshot(
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
}
