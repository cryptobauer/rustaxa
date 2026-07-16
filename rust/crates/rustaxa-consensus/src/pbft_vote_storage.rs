//! Rust-owned PBFT vote persistence runtime.
//!
//! Vote admission and progress planning decide which durable vote rows must
//! change, but storage commit ordering belongs here. This module receives
//! canonical vote payloads and compact hashes, validates storage-only facts,
//! creates one `rustaxa-storage` batch per logical operation, and commits or
//! rejects the operation without routing through C++ batch ids.

use anyhow::Result;
use ethereum_types::H256;
use rustaxa_storage::Storage;

/// Stable status for PBFT vote persistence operations.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftVotePersistenceStatus {
    /// The operation committed successfully.
    Applied,
    /// The operation was rejected before commit.
    Rejected,
}

impl PbftVotePersistenceStatus {
    /// Stable bridge status code.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Applied => 0,
            Self::Rejected => 1,
        }
    }
}

/// Weighted PBFT vote payload keyed by canonical vote hash.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVoteStorageRecord {
    /// Canonical PBFT vote hash used as the storage key.
    pub hash: H256,
    /// Weighted PBFT vote RLP bytes.
    pub vote_rlp: Vec<u8>,
}

/// Latest-round `2t+1` vote bundle selected by vote progress.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftTwoTPlusOneVoteBundle {
    /// C++-compatible `TwoTPlusOneVotedBlockType` discriminant.
    pub kind: u8,
    /// PBFT period represented by the bundle.
    pub period: u64,
    /// PBFT round represented by the bundle.
    pub round: u64,
    /// PBFT step that triggered persistence.
    pub step: u64,
    /// Block hash selected by the threshold family.
    pub block_hash: H256,
    /// RLP list of weighted PBFT vote payloads.
    pub votes_bundle_rlp: Vec<u8>,
}

/// Operation-level VoteManager persistence request for one accepted vote.
#[derive(Debug, Clone, Eq, PartialEq, Default)]
pub struct PbftVoteProgressPersistenceWrite {
    /// Optional accepted stale cert vote to store as an extra reward vote.
    pub extra_reward_vote: Option<PbftVoteStorageRecord>,
    /// Optional latest-round `2t+1` bundle replacement.
    pub two_t_plus_one_bundle: Option<PbftTwoTPlusOneVoteBundle>,
}

/// Result returned after one Rust-owned PBFT vote persistence operation.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftVotePersistenceResult {
    /// Stable operation status.
    pub status: PbftVotePersistenceStatus,
    /// Number of logical vote-family writes accepted into the committed batch.
    pub applied_writes: u64,
    /// Stable error code for rejected operations.
    pub error_code: String,
}

fn applied(applied_writes: u64) -> PbftVotePersistenceResult {
    PbftVotePersistenceResult {
        status: PbftVotePersistenceStatus::Applied,
        applied_writes,
        error_code: String::new(),
    }
}

fn rejected(error_code: &str) -> PbftVotePersistenceResult {
    PbftVotePersistenceResult {
        status: PbftVotePersistenceStatus::Rejected,
        applied_writes: 0,
        error_code: error_code.to_string(),
    }
}

fn validate_two_t_plus_one_kind(kind: u8) -> bool {
    kind <= 3
}

fn validate_votes_bundle_rlp(bytes: &[u8]) -> bool {
    let rlp = rlp::Rlp::new(bytes);
    rlp.is_list() && rlp.item_count().is_ok()
}

/// Persists VoteManager durable effects for one accepted PBFT vote.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `write`: optional extra reward-vote record and optional latest-round
///   `2t+1` bundle selected by vote progress.
///
/// Outputs:
/// - `Applied` with a logical write count after the Rust-owned batch commits.
/// - `Rejected` with a stable error code when validation or storage fails.
///
/// Invariants and edge behavior:
/// - Both optional effects are applied through one Rust storage batch.
/// - `2t+1` bundle replacement is delete-plus-put atomic.
/// - Vote bytes are preserved as canonical storage bytes and not decoded into
///   C++ vote objects.
/// - Extra reward-vote writes and cert-bundle replacements hold the shared
///   reward-storage guard through commit, preventing interleaving with
///   finalization reset or one-time legacy cursor bootstrap.
pub fn persist_pbft_vote_progress(
    storage: &Storage,
    write: PbftVoteProgressPersistenceWrite,
) -> Result<PbftVotePersistenceResult> {
    let _extra_reward_guard = if write.extra_reward_vote.is_some()
        || write
            .two_t_plus_one_bundle
            .as_ref()
            .is_some_and(|bundle| bundle.kind == 1)
    {
        Some(storage.lock_extra_reward_votes()?)
    } else {
        None
    };
    let mut batch = storage.create_write_batch();
    let mut applied_writes = 0;

    if let Some(vote) = write.extra_reward_vote {
        if storage
            .pbft()
            .write_extra_reward_vote_in_batch(&mut batch, vote.hash, &vote.vote_rlp)
            .is_err()
        {
            return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
        }
        applied_writes += 1;
    }

    if let Some(bundle) = write.two_t_plus_one_bundle {
        if !validate_two_t_plus_one_kind(bundle.kind) {
            return Ok(rejected("PBFT_VOTE_PERSIST_INVALID_TWO_T_PLUS_ONE_KIND"));
        }
        if !validate_votes_bundle_rlp(&bundle.votes_bundle_rlp) {
            return Ok(rejected(
                "PBFT_VOTE_PERSIST_MALFORMED_TWO_T_PLUS_ONE_BUNDLE",
            ));
        }
        if storage
            .pbft()
            .replace_two_t_plus_one_votes_in_batch(
                &mut batch,
                bundle.kind,
                &bundle.votes_bundle_rlp,
            )
            .is_err()
        {
            return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
        }
        applied_writes += 1;
    }

    if storage.commit_write_batch_with_sync(batch, false).is_err() {
        return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
    }

    Ok(applied(applied_writes))
}

/// Clears latest-round own verified votes through a Rust-owned batch.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - `Applied` with the number of delete intents committed.
/// - `Rejected` with a stable storage error code when a delete or commit fails.
///
/// Invariants and edge behavior:
/// - The operation creates and commits its own Rust storage batch; no C++ batch
///   id participates.
/// - Keys are enumerated directly from the native latest-round own-vote column
///   before the batch is constructed, so callers cannot accidentally leave
///   rows behind by supplying an incomplete compatibility-sidecar list.
/// - A storage-owned mutex serializes enumeration and commit with production
///   saves and PBFT-service verified-vote queries sharing the `Storage` instance.
/// - Missing keys are RocksDB delete no-ops, matching legacy semantics.
pub fn clear_own_verified_votes(storage: &Storage) -> Result<PbftVotePersistenceResult> {
    let _guard = storage.lock_own_verified_votes()?;
    let vote_hashes = storage.pbft().own_verified_vote_hashes()?;
    let mut batch = storage.create_write_batch();
    let applied_writes = vote_hashes.len() as u64;

    for hash in vote_hashes {
        if storage
            .pbft()
            .remove_own_verified_vote_in_batch(&mut batch, hash)
            .is_err()
        {
            return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
        }
    }

    if storage.commit_write_batch_with_sync(batch, false).is_err() {
        return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
    }

    Ok(applied(applied_writes))
}

/// Persists one locally generated own verified vote through a Rust-owned batch.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `vote`: weighted own-vote payload keyed by canonical vote hash.
///
/// Outputs:
/// - `Applied` after the latest-round own-vote row commits.
/// - `Rejected` with a storage error code when the write or commit fails.
///
/// Invariants and edge behavior:
/// - Existing own-vote rows for the same hash are overwritten, matching legacy
///   RocksDB put semantics.
/// - A storage-owned mutex serializes the full save with lifecycle/direct
///   clears and PBFT-service verified-vote production queries sharing the storage.
pub fn save_own_verified_vote(
    storage: &Storage,
    vote: PbftVoteStorageRecord,
) -> Result<PbftVotePersistenceResult> {
    let _guard = storage.lock_own_verified_votes()?;
    let mut batch = storage.create_write_batch();
    if storage
        .pbft()
        .write_own_verified_vote_in_batch(&mut batch, vote.hash, &vote.vote_rlp)
        .is_err()
    {
        return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
    }
    if storage.commit_write_batch_with_sync(batch, false).is_err() {
        return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
    }
    Ok(applied(1))
}

/// Removes extra reward votes through a Rust-owned batch.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `vote_hashes`: extra reward-vote keys to delete.
///
/// Outputs:
/// - `Applied` after deletes commit.
/// - `Rejected` with a storage error code on storage failure.
///
/// Invariants and edge behavior:
/// - Missing keys are delete no-ops, matching legacy behavior.
pub fn remove_extra_reward_votes(
    storage: &Storage,
    vote_hashes: Vec<H256>,
) -> Result<PbftVotePersistenceResult> {
    let _extra_reward_guard = storage.lock_extra_reward_votes()?;
    let mut batch = storage.create_write_batch();
    let applied_writes = vote_hashes.len() as u64;
    for hash in vote_hashes {
        if storage
            .pbft()
            .remove_extra_reward_vote_in_batch(&mut batch, hash)
            .is_err()
        {
            return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
        }
    }
    if storage.commit_write_batch_with_sync(batch, false).is_err() {
        return Ok(rejected("PBFT_VOTE_PERSIST_STORAGE_FAILURE"));
    }
    Ok(applied(applied_writes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Column, Config};

    fn temp_storage(name: &str) -> Storage {
        let dir = std::env::temp_dir().join(format!(
            "{name}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        Storage::new(Config::new(dir)).unwrap()
    }

    #[test]
    fn progress_persistence_groups_reward_and_two_t_plus_one_writes() {
        let storage = temp_storage("rustaxa_consensus_pbft_vote_progress");
        let reward_hash = H256::from([0x44; 32]);
        let result = persist_pbft_vote_progress(
            &storage,
            PbftVoteProgressPersistenceWrite {
                extra_reward_vote: Some(PbftVoteStorageRecord {
                    hash: reward_hash,
                    vote_rlp: vec![0x71],
                }),
                two_t_plus_one_bundle: Some(PbftTwoTPlusOneVoteBundle {
                    kind: 0,
                    period: 10,
                    round: 2,
                    step: 3,
                    block_hash: H256::from([0x55; 32]),
                    votes_bundle_rlp: vec![0xC2, 0x01, 0x02],
                }),
            },
        )
        .unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Applied);
        assert_eq!(result.applied_writes, 2);
        let persisted = storage
            .get_raw(Column::ExtraRewardVotes, reward_hash.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(persisted.as_slice(), &[0x71]);
    }

    #[test]
    fn progress_persistence_rejects_invalid_bundle_kind_without_commit() {
        let storage = temp_storage("rustaxa_consensus_pbft_vote_progress_reject");
        let result = persist_pbft_vote_progress(
            &storage,
            PbftVoteProgressPersistenceWrite {
                extra_reward_vote: None,
                two_t_plus_one_bundle: Some(PbftTwoTPlusOneVoteBundle {
                    kind: 99,
                    period: 0,
                    round: 0,
                    step: 0,
                    block_hash: H256::zero(),
                    votes_bundle_rlp: vec![0xC1, 0x01],
                }),
            },
        )
        .unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Rejected);
        assert_eq!(
            result.error_code,
            "PBFT_VOTE_PERSIST_INVALID_TWO_T_PLUS_ONE_KIND"
        );
        assert!(
            storage
                .pbft()
                .all_two_t_plus_one_votes_rlp()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn clear_own_verified_votes_commits_deletes() {
        let storage = temp_storage("rustaxa_consensus_pbft_vote_clear_own");
        let own_hash = H256::from([0x66; 32]);
        storage
            .pbft()
            .write_own_verified_vote(own_hash, &[0x72])
            .unwrap();

        let result = clear_own_verified_votes(&storage).unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Applied);
        assert_eq!(result.applied_writes, 1);
        assert!(storage.pbft().own_verified_votes_rlp().unwrap().is_empty());
    }

    #[test]
    fn clear_waits_for_in_flight_shared_handle_save_and_removes_its_row() {
        use std::sync::{Arc, mpsc};
        use std::time::Duration;

        let storage = Arc::new(temp_storage(
            "rustaxa_consensus_pbft_vote_clear_serializes_save",
        ));
        let own_hash = H256::from([0x67; 32]);
        let (save_locked_tx, save_locked_rx) = mpsc::channel();
        let (continue_save_tx, continue_save_rx) = mpsc::channel();
        let save_storage = storage.clone();
        let save = std::thread::spawn(move || {
            let _guard = save_storage.lock_own_verified_votes().unwrap();
            save_locked_tx.send(()).unwrap();
            continue_save_rx.recv().unwrap();
            save_storage
                .pbft()
                .write_own_verified_vote(own_hash, &[0x74])
                .unwrap();
        });
        save_locked_rx.recv().unwrap();

        let (clear_done_tx, clear_done_rx) = mpsc::channel();
        let clear_storage = storage.clone();
        let clear = std::thread::spawn(move || {
            let result = clear_own_verified_votes(&clear_storage).unwrap();
            clear_done_tx.send(result).unwrap();
        });
        assert!(
            clear_done_rx
                .recv_timeout(Duration::from_millis(50))
                .is_err(),
            "clear must block while the shared save operation owns the guard"
        );

        continue_save_tx.send(()).unwrap();
        save.join().unwrap();
        let result = clear_done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        clear.join().unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Applied);
        assert_eq!(result.applied_writes, 1);
        assert!(
            storage
                .pbft()
                .own_verified_vote_hashes()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn save_own_verified_vote_commits_payload() {
        let storage = temp_storage("rustaxa_consensus_pbft_vote_save_own");
        let own_hash = H256::from([0x77; 32]);

        let result = save_own_verified_vote(
            &storage,
            PbftVoteStorageRecord {
                hash: own_hash,
                vote_rlp: vec![0x73],
            },
        )
        .unwrap();

        assert_eq!(result.status, PbftVotePersistenceStatus::Applied);
        let own_votes = storage.pbft().own_verified_votes_rlp().unwrap();
        assert_eq!(own_votes, vec![vec![0x73]]);
    }
}
