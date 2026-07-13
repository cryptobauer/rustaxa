use anyhow::Result;
use ethereum_types::H256;
use std::sync::Arc;

use crate::Column;
use crate::SINGLE_VALUE_KEY;
use crate::StorageError;
use crate::db::{DbReader, DbWriter};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TwoTPlusOneVotedBlockType {
    SoftVoted = 0,
    CertVoted = 1,
    NextVoted = 2,
    NextVotedNull = 3,
}

impl TwoTPlusOneVotedBlockType {
    const ALL: [TwoTPlusOneVotedBlockType; 4] = [
        TwoTPlusOneVotedBlockType::SoftVoted,
        TwoTPlusOneVotedBlockType::CertVoted,
        TwoTPlusOneVotedBlockType::NextVoted,
        TwoTPlusOneVotedBlockType::NextVotedNull,
    ];
}

pub struct PbftRepository<D: DbReader> {
    db: Arc<D>,
}

/// One latest-round `2t+1` storage slot with its voted-block category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTwoTPlusOneVotesBundle {
    /// Stable legacy category key (`0..=3`).
    pub kind: u8,
    /// Canonical RLP list of weighted PBFT vote payloads.
    pub votes_bundle_rlp: Vec<u8>,
}

/// One persisted locally produced PBFT vote with its canonical storage key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredOwnVerifiedVote {
    /// Canonical signed-vote hash stored as the RocksDB key.
    pub vote_hash: H256,
    /// Persisted weighted PBFT vote RLP stored as the row value.
    pub vote_rlp: Vec<u8>,
}

/// One persisted extra reward PBFT vote with its canonical storage key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExtraRewardVote {
    /// Canonical signed-vote hash stored as the RocksDB key.
    pub vote_hash: H256,
    /// Persisted weighted PBFT vote RLP stored as the row value.
    pub vote_rlp: Vec<u8>,
}

/// Durable identity of the cert-vote bundle selected by finalization rewards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFinalizedRewardVoteCursor {
    /// Finalized PBFT period whose cert votes are reward-eligible.
    pub period: u64,
    /// Certified round selected for that period.
    pub round: u64,
    /// Certified step selected for that round.
    pub step: u64,
    /// Certified PBFT block hash.
    pub block_hash: H256,
    /// Canonical weighted cert-vote bundle selected for reward processing.
    pub votes_bundle_rlp: Vec<u8>,
}

impl<D: DbReader> PbftRepository<D> {
    /// Creates a PBFT repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        PbftRepository { db }
    }

    /// Returns true when a PBFT block hash has a finalized period index entry.
    /// C++ mapping: `DbStorage::pbftBlockInDb(blk_hash_t const&)`.
    pub fn exists(&self, pbft_hash: H256) -> Result<bool> {
        self.db.exist(Column::PbftBlockPeriod, pbft_hash.as_bytes())
    }

    /// Reads a PBFT manager numeric field and validates fixed-width encoding.
    /// C++ mapping: `DbStorage::getPbftMgrField(PbftMgrField)`.
    pub fn manager_field(&self, field: u8) -> Result<Option<u32>> {
        let Some(value) = self.db.get(Column::PbftMgrRoundStep, &[field])? else {
            return Ok(None);
        };
        let value = value.as_ref();
        if value.is_empty() {
            return Ok(None);
        }
        if value.len() != std::mem::size_of::<u32>() {
            return Err(StorageError::Read(format!(
                "Invalid pbft_mgr_round_step value size: expected {}, got {}",
                std::mem::size_of::<u32>(),
                value.len()
            ))
            .into());
        }

        let mut num = [0u8; 4];
        num.copy_from_slice(value);
        Ok(Some(u32::from_le_bytes(num)))
    }

    /// Reads a PBFT manager status flag.
    /// C++ mapping: `DbStorage::getPbftMgrStatus(PbftMgrStatus)`.
    pub fn manager_status(&self, field: u8) -> Result<Option<bool>> {
        let Some(value) = self.db.get(Column::PbftMgrStatus, &[field])? else {
            return Ok(None);
        };
        let value = value.as_ref();
        if value.is_empty() {
            return Ok(None);
        }

        Ok(Some(value[0] != 0))
    }

    /// Returns serialized cert-voted block-with-round payload for the latest round.
    /// C++ mapping: `DbStorage::getCertVotedBlockInRound() const`.
    pub fn cert_voted_block_in_round_rlp(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::CertVotedBlockInRound, &SINGLE_VALUE_KEY)?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }

    /// Returns all cached proposed PBFT blocks as raw RLP payloads.
    /// C++ mapping: `DbStorage::getProposedPbftBlocks()`.
    pub fn proposed_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for item in self.db.iter(Column::ProposedPbftBlocks) {
            let (_, value) = item?;
            result.push(value.into_vec());
        }
        Ok(result)
    }

    /// Returns serialized PBFT head bytes associated with a head hash.
    /// C++ mapping: `DbStorage::getPbftHead(blk_hash_t const&)`.
    pub fn head(&self, pbft_hash: H256) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::PbftHead, pbft_hash.as_bytes())?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }

    /// Returns all locally stored verified votes for the latest round.
    /// C++ mapping: `DbStorage::getOwnVerifiedVotes()`.
    pub fn own_verified_votes_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for item in self.db.iter(Column::LatestRoundOwnVotes) {
            let (_, value) = item?;
            result.push(value.into_vec());
        }
        Ok(result)
    }

    /// Returns locally produced votes with their storage keys in hash order.
    ///
    /// Each key must be exactly 32 bytes. Payload semantics are deliberately
    /// validated by the consensus layer, which owns the PBFT codec and checks
    /// that the decoded canonical hash matches `vote_hash`.
    pub fn own_verified_vote_records(&self) -> Result<Vec<StoredOwnVerifiedVote>> {
        let mut result = Vec::new();
        for item in self.db.iter(Column::LatestRoundOwnVotes) {
            let (key, value) = item?;
            if key.len() != 32 {
                return Err(StorageError::Read(format!(
                    "Invalid latest_round_own_votes key size: expected 32, got {}",
                    key.len()
                ))
                .into());
            }
            result.push(StoredOwnVerifiedVote {
                vote_hash: H256::from_slice(&key),
                vote_rlp: value.into_vec(),
            });
        }
        result.sort_unstable_by_key(|record| record.vote_hash);
        Ok(result)
    }

    /// Returns canonical hashes used as keys for all locally produced votes.
    pub fn own_verified_vote_hashes(&self) -> Result<Vec<H256>> {
        Ok(self
            .own_verified_vote_records()?
            .into_iter()
            .map(|record| record.vote_hash)
            .collect())
    }

    /// Returns flattened votes from all stored 2t+1 vote bundles.
    /// C++ mapping: `DbStorage::getAllTwoTPlusOneVotes()`.
    pub fn all_two_t_plus_one_votes_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for vote_type in TwoTPlusOneVotedBlockType::ALL.iter() {
            let Some(votes_bundle_rlp) = self
                .db
                .get(Column::LatestRoundTwoTPlusOneVotes, &[*vote_type as u8])?
            else {
                continue;
            };
            let votes_bundle_rlp = votes_bundle_rlp.as_ref();
            if votes_bundle_rlp.is_empty() {
                continue;
            }

            let votes = rlp::Rlp::new(votes_bundle_rlp);
            for vote in votes.iter() {
                result.push(vote.as_raw().to_vec());
            }
        }
        Ok(result)
    }

    /// Returns non-empty latest-round `2t+1` slots without discarding category.
    ///
    /// Results follow stable category order. Keeping the key attached lets
    /// consensus restoration validate each bundle's vote type and reconstruct
    /// the authoritative voted-block mapping rather than merely flattening
    /// votes into compatibility sidecars.
    pub fn two_t_plus_one_votes_bundles(&self) -> Result<Vec<StoredTwoTPlusOneVotesBundle>> {
        let mut result = Vec::new();
        for vote_type in TwoTPlusOneVotedBlockType::ALL {
            let kind = vote_type as u8;
            let Some(value) = self.db.get(Column::LatestRoundTwoTPlusOneVotes, &[kind])? else {
                continue;
            };
            let votes_bundle_rlp = value.as_ref().to_vec();
            if !votes_bundle_rlp.is_empty() {
                result.push(StoredTwoTPlusOneVotesBundle {
                    kind,
                    votes_bundle_rlp,
                });
            }
        }
        Ok(result)
    }

    /// Returns all stored extra reward votes for the latest finalized block.
    /// C++ mapping: `DbStorage::getRewardVotes()`.
    pub fn reward_votes_rlp(&self) -> Result<Vec<Vec<u8>>> {
        let mut result = Vec::new();
        for item in self.db.iter(Column::ExtraRewardVotes) {
            let (_, value) = item?;
            result.push(value.into_vec());
        }
        Ok(result)
    }

    /// Returns extra reward votes with their keys in canonical hash order.
    ///
    /// Keys must be exactly 32 bytes. Consensus validates weighted PBFT payload
    /// semantics and decoded hash/key equality before using these records.
    pub fn extra_reward_vote_records(&self) -> Result<Vec<StoredExtraRewardVote>> {
        let mut result = Vec::new();
        for item in self.db.iter(Column::ExtraRewardVotes) {
            let (key, value) = item?;
            if key.len() != 32 {
                return Err(StorageError::Read(format!(
                    "Invalid extra_reward_votes key size: expected 32, got {}",
                    key.len()
                ))
                .into());
            }
            result.push(StoredExtraRewardVote {
                vote_hash: H256::from_slice(&key),
                vote_rlp: value.into_vec(),
            });
        }
        result.sort_unstable_by_key(|record| record.vote_hash);
        Ok(result)
    }

    /// Returns canonical keys for all extra reward votes in hash order.
    pub fn extra_reward_vote_hashes(&self) -> Result<Vec<H256>> {
        Ok(self
            .extra_reward_vote_records()?
            .into_iter()
            .map(|record| record.vote_hash)
            .collect())
    }

    /// Loads the finalized reward-vote cursor from its dedicated durable row.
    ///
    /// Returns `None` before the first reward reset. The fixed-width encoding
    /// begins with three little-endian `u64` values and a 32-byte hash, followed
    /// by the non-empty canonical cert-vote bundle. Malformed rows fail instead
    /// of falling back to the mutable latest cert bundle.
    pub fn finalized_reward_vote_cursor(&self) -> Result<Option<StoredFinalizedRewardVoteCursor>> {
        let Some(value) = self
            .db
            .get(Column::FinalizedRewardVoteCursor, &SINGLE_VALUE_KEY)?
        else {
            return Ok(None);
        };
        let value = value.as_ref();
        if value.is_empty() {
            return Ok(None);
        }
        if value.len() <= 56 {
            return Err(StorageError::Read(format!(
                "Invalid finalized_reward_vote_cursor value size: expected more than 56, got {}",
                value.len()
            ))
            .into());
        }
        let decode_u64 = |offset: usize| {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(&value[offset..offset + 8]);
            u64::from_le_bytes(bytes)
        };
        Ok(Some(StoredFinalizedRewardVoteCursor {
            period: decode_u64(0),
            round: decode_u64(8),
            step: decode_u64(16),
            block_hash: H256::from_slice(&value[24..56]),
            votes_bundle_rlp: value[56..].to_vec(),
        }))
    }
}

impl<D: DbReader + DbWriter> PbftRepository<D> {
    /// Appends the finalized reward-vote cursor to a caller-owned batch.
    ///
    /// The cursor is encoded canonically as fixed-width scalar fields and must
    /// be committed atomically with the reward-reset cert bundle and stale
    /// extra-vote deletions. Repeated writes replace the single durable row.
    pub fn write_finalized_reward_vote_cursor_in_batch(
        &self,
        batch: &mut D::Batch,
        cursor: StoredFinalizedRewardVoteCursor,
    ) -> Result<()> {
        if cursor.votes_bundle_rlp.is_empty() {
            return Err(StorageError::Read(
                "finalized reward vote cursor bundle must not be empty".into(),
            )
            .into());
        }
        let mut value = Vec::with_capacity(56 + cursor.votes_bundle_rlp.len());
        value.extend_from_slice(&cursor.period.to_le_bytes());
        value.extend_from_slice(&cursor.round.to_le_bytes());
        value.extend_from_slice(&cursor.step.to_le_bytes());
        value.extend_from_slice(cursor.block_hash.as_bytes());
        value.extend_from_slice(&cursor.votes_bundle_rlp);
        self.db.batch_put(
            batch,
            Column::FinalizedRewardVoteCursor,
            &SINGLE_VALUE_KEY,
            &value,
        )
    }

    /// Persists a PBFT manager numeric field value.
    /// C++ mapping: `DbStorage::savePbftMgrField(PbftMgrField, uint32_t)`.
    pub fn write_manager_field(&self, field: u8, value: u32) -> Result<()> {
        self.db
            .put(Column::PbftMgrRoundStep, &[field], &value.to_le_bytes())
    }

    /// Appends a PBFT manager numeric field write to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `field`: C++-compatible `PbftMgrField` discriminant.
    /// - `value`: absolute field value encoded as little-endian `uint32_t`.
    ///
    /// Outputs:
    /// - Appends a put in `pbft_mgr_round_step`.
    ///
    /// Invariants and edge behavior:
    /// - Missing values are not interpreted here; restart/default behavior is a
    ///   read-side contract.
    /// - Existing keys are overwritten, matching legacy RocksDB put semantics.
    pub fn write_manager_field_in_batch(
        &self,
        batch: &mut D::Batch,
        field: u8,
        value: u32,
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::PbftMgrRoundStep,
            &[field],
            &value.to_le_bytes(),
        )
    }

    /// Persists a PBFT manager status flag.
    /// C++ mapping: `DbStorage::savePbftMgrStatus(PbftMgrStatus, bool const&)`.
    pub fn write_manager_status(&self, field: u8, value: bool) -> Result<()> {
        self.db
            .put(Column::PbftMgrStatus, &[field], &[u8::from(value)])
    }

    /// Appends a PBFT manager status flag write to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `field`: C++-compatible `PbftMgrStatus` discriminant.
    /// - `value`: status value encoded with Rust/C++ bool compatibility
    ///   (`0` for false, `1` for true).
    ///
    /// Outputs:
    /// - Appends a put in `pbft_mgr_status`.
    ///
    /// Invariants and edge behavior:
    /// - Existing keys are overwritten, matching legacy RocksDB put semantics.
    pub fn write_manager_status_in_batch(
        &self,
        batch: &mut D::Batch,
        field: u8,
        value: bool,
    ) -> Result<()> {
        self.db
            .batch_put(batch, Column::PbftMgrStatus, &[field], &[u8::from(value)])
    }

    /// Stores serialized PBFT head bytes for a head hash.
    /// C++ mapping: `DbStorage::savePbftHead(blk_hash_t const&, std::string const&)`.
    pub fn write_head(&self, pbft_hash: H256, head_bytes: &[u8]) -> Result<()> {
        self.db
            .put(Column::PbftHead, pbft_hash.as_bytes(), head_bytes)
    }

    /// Appends serialized PBFT head bytes to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `pbft_hash`: canonical PBFT block hash used as the head key.
    /// - `head_bytes`: legacy PBFT chain head bytes stored without decoding.
    ///
    /// Outputs:
    /// - Appends a put in `pbft_head`.
    ///
    /// Invariants and edge behavior:
    /// - Existing head bytes for the same hash are overwritten, matching legacy
    ///   RocksDB put semantics.
    /// - The payload is intentionally opaque here; PBFT chain compatibility
    ///   owns the encoded head shape until that object is fully Rust-owned.
    pub fn write_head_in_batch(
        &self,
        batch: &mut D::Batch,
        pbft_hash: H256,
        head_bytes: &[u8],
    ) -> Result<()> {
        self.db
            .batch_put(batch, Column::PbftHead, pbft_hash.as_bytes(), head_bytes)
    }

    /// Stores one locally produced verified vote payload.
    /// C++ mapping: `DbStorage::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>&)`.
    pub fn write_own_verified_vote(&self, vote_hash: H256, vote_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::LatestRoundOwnVotes, vote_hash.as_bytes(), vote_rlp)
    }

    /// Appends one locally produced verified vote payload to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `vote_hash`: canonical PBFT vote hash used as the latest own-vote key.
    /// - `vote_rlp`: weighted PBFT vote bytes, matching `PbftVote::rlp(true, true)`.
    ///
    /// Outputs:
    /// - Appends a put in `latest_round_own_votes`.
    ///
    /// Invariants and edge behavior:
    /// - Existing keys are overwritten, matching RocksDB put semantics used by
    ///   the legacy C++ path.
    pub fn write_own_verified_vote_in_batch(
        &self,
        batch: &mut D::Batch,
        vote_hash: H256,
        vote_rlp: &[u8],
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::LatestRoundOwnVotes,
            vote_hash.as_bytes(),
            vote_rlp,
        )
    }

    /// Replaces a full 2t+1 vote bundle for the given vote type.
    /// C++ mapping: `DbStorage::replaceTwoTPlusOneVotes(TwoTPlusOneVotedBlockType, const std::vector<std::shared_ptr<PbftVote>>&)`.
    pub fn replace_two_t_plus_one_votes(
        &self,
        vote_type: u8,
        votes_bundle_rlp: &[u8],
    ) -> Result<()> {
        Self::validate_two_t_plus_one_vote_type(vote_type)?;
        self.db
            .delete(Column::LatestRoundTwoTPlusOneVotes, &[vote_type])?;
        self.db.put(
            Column::LatestRoundTwoTPlusOneVotes,
            &[vote_type],
            votes_bundle_rlp,
        )
    }

    /// Appends a full 2t+1 vote bundle replacement to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `vote_type`: C++-compatible `TwoTPlusOneVotedBlockType` discriminant.
    /// - `votes_bundle_rlp`: RLP list containing weighted PBFT vote payloads.
    ///
    /// Outputs:
    /// - Appends delete-then-put operations for the latest-round 2t+1 vote slot.
    ///
    /// Invariants and edge behavior:
    /// - The vote type must be one of the C++ discriminants `0..=3`.
    /// - The bundle is keyed only by vote type to preserve legacy
    ///   "latest-round" semantics.
    pub fn replace_two_t_plus_one_votes_in_batch(
        &self,
        batch: &mut D::Batch,
        vote_type: u8,
        votes_bundle_rlp: &[u8],
    ) -> Result<()> {
        Self::validate_two_t_plus_one_vote_type(vote_type)?;
        self.db
            .batch_delete(batch, Column::LatestRoundTwoTPlusOneVotes, &[vote_type])?;
        self.db.batch_put(
            batch,
            Column::LatestRoundTwoTPlusOneVotes,
            &[vote_type],
            votes_bundle_rlp,
        )
    }

    /// Stores one extra reward vote payload.
    /// C++ mapping: `DbStorage::saveExtraRewardVote(const std::shared_ptr<PbftVote>&)`.
    pub fn write_extra_reward_vote(&self, vote_hash: H256, vote_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::ExtraRewardVotes, vote_hash.as_bytes(), vote_rlp)
    }

    /// Appends one extra reward vote payload to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `vote_hash`: canonical PBFT vote hash used as the extra reward-vote key.
    /// - `vote_rlp`: weighted PBFT vote bytes, matching `PbftVote::rlp(true, true)`.
    ///
    /// Outputs:
    /// - Appends a put in `extra_reward_votes`.
    ///
    /// Invariants and edge behavior:
    /// - Existing keys are overwritten, matching RocksDB put semantics used by
    ///   the legacy C++ path.
    pub fn write_extra_reward_vote_in_batch(
        &self,
        batch: &mut D::Batch,
        vote_hash: H256,
        vote_rlp: &[u8],
    ) -> Result<()> {
        self.db.batch_put(
            batch,
            Column::ExtraRewardVotes,
            vote_hash.as_bytes(),
            vote_rlp,
        )
    }

    /// Stores the latest cert-voted block together with the round number.
    /// C++ mapping: `DbStorage::saveCertVotedBlockInRound(PbftRound, const std::shared_ptr<PbftBlock>&)`.
    pub fn write_cert_voted_block_in_round(&self, round: u64, block_rlp: &[u8]) -> Result<()> {
        let mut stream = rlp::RlpStream::new_list(2);
        stream.append(&round);
        stream.append_raw(block_rlp, 1);
        self.db.put(
            Column::CertVotedBlockInRound,
            &SINGLE_VALUE_KEY,
            &stream.out(),
        )
    }

    /// Appends the latest cert-voted block and its round to a caller-owned
    /// storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic
    ///   commit.
    /// - `round`: PBFT round associated with the cert-voted block.
    /// - `block_rlp`: Canonical PBFT block bytes matching
    ///   `PbftBlock::rlp(true)`.
    ///
    /// Outputs:
    /// - Appends a put in `cert_voted_block_in_round` at the legacy
    ///   single-value key.
    ///
    /// Invariants and edge behavior:
    /// - The stored value is the legacy two-item RLP list `[round, block]`.
    /// - `block_rlp` is embedded as raw RLP bytes and is not decoded or
    ///   normalized by storage.
    pub fn write_cert_voted_block_in_round_in_batch(
        &self,
        batch: &mut D::Batch,
        round: u64,
        block_rlp: &[u8],
    ) -> Result<()> {
        let mut stream = rlp::RlpStream::new_list(2);
        stream.append(&round);
        stream.append_raw(block_rlp, 1);
        self.db.batch_put(
            batch,
            Column::CertVotedBlockInRound,
            &SINGLE_VALUE_KEY,
            &stream.out(),
        )
    }

    /// Stores a proposed PBFT block payload by its hash.
    /// C++ mapping: `DbStorage::saveProposedPbftBlock(const std::shared_ptr<PbftBlock>&)`.
    pub fn write_proposed(&self, block_hash: H256, block_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::ProposedPbftBlocks, block_hash.as_bytes(), block_rlp)
    }

    /// Removes the cached cert-voted block for the latest round.
    /// C++ mapping: `DbStorage::removeCertVotedBlockInRound(Batch&)`.
    pub fn remove_cert_voted_block_in_round(&self) -> Result<()> {
        self.db
            .delete(Column::CertVotedBlockInRound, &SINGLE_VALUE_KEY)
    }

    /// Appends removal of the cached cert-voted block to a caller-owned batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    ///
    /// Outputs:
    /// - Appends a delete in `cert_voted_block_in_round` at the legacy
    ///   single-value key.
    ///
    /// Invariants and edge behavior:
    /// - Missing keys are RocksDB delete no-ops, matching legacy storage
    ///   behavior.
    pub fn remove_cert_voted_block_in_round_in_batch(&self, batch: &mut D::Batch) -> Result<()> {
        self.db
            .batch_delete(batch, Column::CertVotedBlockInRound, &SINGLE_VALUE_KEY)
    }

    /// Removes one proposed PBFT block by hash.
    /// C++ mapping: `DbStorage::removeProposedPbftBlock(const blk_hash_t&, Batch&)`.
    pub fn remove_proposed(&self, block_hash: H256) -> Result<()> {
        self.db
            .delete(Column::ProposedPbftBlocks, block_hash.as_bytes())
    }

    /// Appends removal of one proposed PBFT block to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `block_hash`: canonical proposed PBFT block hash used as the cache key.
    ///
    /// Outputs:
    /// - Appends a delete in `proposed_pbft_blocks`.
    ///
    /// Invariants and edge behavior:
    /// - Missing keys are RocksDB delete no-ops, matching legacy storage
    ///   behavior.
    pub fn remove_proposed_in_batch(&self, batch: &mut D::Batch, block_hash: H256) -> Result<()> {
        self.db
            .batch_delete(batch, Column::ProposedPbftBlocks, block_hash.as_bytes())
    }

    /// Removes one cached own verified vote by vote hash.
    /// C++ mapping: `DbStorage::clearOwnVerifiedVotes(Batch&, const std::vector<std::shared_ptr<PbftVote>>&)`.
    pub fn remove_own_verified_vote(&self, vote_hash: H256) -> Result<()> {
        self.db
            .delete(Column::LatestRoundOwnVotes, vote_hash.as_bytes())
    }

    /// Appends removal of one cached own verified vote to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `vote_hash`: canonical PBFT vote hash used as the latest own-vote key.
    ///
    /// Outputs:
    /// - Appends a delete in `latest_round_own_votes`.
    ///
    /// Invariants and edge behavior:
    /// - Missing keys are treated as RocksDB delete no-ops, matching legacy
    ///   storage behavior.
    pub fn remove_own_verified_vote_in_batch(
        &self,
        batch: &mut D::Batch,
        vote_hash: H256,
    ) -> Result<()> {
        self.db
            .batch_delete(batch, Column::LatestRoundOwnVotes, vote_hash.as_bytes())
    }

    /// Removes one cached extra reward vote by vote hash.
    /// C++ mapping: `DbStorage::removeExtraRewardVotes(const std::vector<vote_hash_t>&, Batch&)`.
    pub fn remove_extra_reward_vote(&self, vote_hash: H256) -> Result<()> {
        self.db
            .delete(Column::ExtraRewardVotes, vote_hash.as_bytes())
    }

    /// Appends removal of one cached extra reward vote to a caller-owned storage batch.
    ///
    /// Inputs:
    /// - `batch`: Rust storage write batch that owns the eventual atomic commit.
    /// - `vote_hash`: canonical PBFT vote hash used as the extra reward-vote key.
    ///
    /// Outputs:
    /// - Appends a delete in `extra_reward_votes`.
    ///
    /// Invariants and edge behavior:
    /// - Missing keys are treated as RocksDB delete no-ops, matching legacy
    ///   storage behavior.
    pub fn remove_extra_reward_vote_in_batch(
        &self,
        batch: &mut D::Batch,
        vote_hash: H256,
    ) -> Result<()> {
        self.db
            .batch_delete(batch, Column::ExtraRewardVotes, vote_hash.as_bytes())
    }

    fn validate_two_t_plus_one_vote_type(vote_type: u8) -> Result<()> {
        if TwoTPlusOneVotedBlockType::ALL
            .iter()
            .any(|item| *item as u8 == vote_type)
        {
            return Ok(());
        }
        Err(StorageError::Read(format!("Invalid two_t_plus_one vote type: {vote_type}")).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DbIterator, DbWriter};
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    enum MockBatchOp {
        Put(Column, Vec<u8>, Vec<u8>),
        Delete(Column, Vec<u8>),
    }

    struct MockPbftStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockPbftStore {
        fn new() -> Self {
            MockPbftStore {
                data: RwLock::new(HashMap::new()),
            }
        }

        fn put(&self, col: Column, key: &[u8], value: &[u8]) {
            let mut data = self.data.write().unwrap();
            let cf = data
                .entry(col.name().to_string())
                .or_insert_with(BTreeMap::new);
            cf.insert(key.to_vec(), value.to_vec());
        }

        fn delete(&self, col: Column, key: &[u8]) {
            let mut data = self.data.write().unwrap();
            if let Some(cf) = data.get_mut(col.name()) {
                cf.remove(key);
            }
        }
    }

    impl DbReader for MockPbftStore {
        type Slice<'a> = Vec<u8>;

        fn exist(&self, col: Column, key: &[u8]) -> Result<bool> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.contains_key(key))
            } else {
                Ok(false)
            }
        }

        fn get<'a>(&'a self, col: Column, key: &[u8]) -> Result<Option<Self::Slice<'a>>> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                Ok(cf.get(key).cloned())
            } else {
                Ok(None)
            }
        }

        fn get_at_or_before(
            &self,
            col: Column,
            key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            let data = self.data.read().unwrap();
            let Some(cf) = data.get(col.name()) else {
                return Ok(None);
            };
            let key = key.to_vec();
            Ok(cf
                .range(..=key)
                .next_back()
                .map(|(k, v)| (k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
        }

        fn get_at_or_after(
            &self,
            col: Column,
            key: &[u8],
        ) -> Result<Option<(Box<[u8]>, Box<[u8]>)>> {
            let data = self.data.read().unwrap();
            let Some(cf) = data.get(col.name()) else {
                return Ok(None);
            };
            let key = key.to_vec();
            Ok(cf
                .range(key..)
                .next()
                .map(|(k, v)| (k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
        }

        fn iter<'a>(&'a self, col: Column) -> DbIterator<'a> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                let items: Vec<_> = cf
                    .iter()
                    .map(|(k, v)| Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
                    .collect();
                Box::new(items.into_iter())
            } else {
                Box::new(std::iter::empty())
            }
        }

        fn iter_rev<'a>(&'a self, col: Column) -> DbIterator<'a> {
            let data = self.data.read().unwrap();
            if let Some(cf) = data.get(col.name()) {
                let items: Vec<_> = cf
                    .iter()
                    .rev()
                    .map(|(k, v)| Ok((k.clone().into_boxed_slice(), v.clone().into_boxed_slice())))
                    .collect();
                Box::new(items.into_iter())
            } else {
                Box::new(std::iter::empty())
            }
        }
    }

    impl DbWriter for MockPbftStore {
        type Batch = Vec<MockBatchOp>;

        fn create_batch(&self) -> Self::Batch {
            Vec::new()
        }

        fn batch_put(
            &self,
            batch: &mut Self::Batch,
            col: Column,
            key: &[u8],
            value: &[u8],
        ) -> Result<()> {
            batch.push(MockBatchOp::Put(col, key.to_vec(), value.to_vec()));
            Ok(())
        }

        fn batch_delete(&self, batch: &mut Self::Batch, col: Column, key: &[u8]) -> Result<()> {
            batch.push(MockBatchOp::Delete(col, key.to_vec()));
            Ok(())
        }

        fn commit_batch(&self, batch: Self::Batch) -> Result<()> {
            for op in batch {
                match op {
                    MockBatchOp::Put(col, key, value) => self.put(col, &key, &value),
                    MockBatchOp::Delete(col, key) => self.delete(col, &key),
                }
            }
            Ok(())
        }

        fn put(&self, col: Column, key: &[u8], value: &[u8]) -> Result<()> {
            MockPbftStore::put(self, col, key, value);
            Ok(())
        }

        fn delete(&self, col: Column, key: &[u8]) -> Result<()> {
            MockPbftStore::delete(self, col, key);
            Ok(())
        }
    }

    #[test]
    fn test_mock_period_store_exist() {
        let db = MockPbftStore::new();
        let hash = H256::from_low_u64_be(3);

        assert!(!db.exist(Column::PbftBlockPeriod, hash.as_bytes()).unwrap());

        db.put(Column::PbftBlockPeriod, hash.as_bytes(), &[0xAA]);

        assert!(db.exist(Column::PbftBlockPeriod, hash.as_bytes()).unwrap());
        assert!(
            !db.exist(Column::ProposedPbftBlocks, hash.as_bytes())
                .unwrap()
        );
    }

    #[test]
    fn test_pbft_mgr_field() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        assert_eq!(repo.manager_field(0).unwrap(), None);

        db.put(Column::PbftMgrRoundStep, &[0], &7u32.to_le_bytes());
        assert_eq!(repo.manager_field(0).unwrap(), Some(7));
    }

    #[test]
    fn test_pbft_mgr_status() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        assert_eq!(repo.manager_status(1).unwrap(), None);

        db.put(Column::PbftMgrStatus, &[1], &[1]);
        assert_eq!(repo.manager_status(1).unwrap(), Some(true));
    }

    #[test]
    fn test_cert_voted_block_in_round_rlp() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let value = vec![0xC2, 0x01, 0x02];

        assert!(repo.cert_voted_block_in_round_rlp().unwrap().is_none());

        db.put(Column::CertVotedBlockInRound, &SINGLE_VALUE_KEY, &value);
        assert_eq!(repo.cert_voted_block_in_round_rlp().unwrap(), Some(value));
    }

    #[test]
    fn test_proposed_pbft_blocks_rlp() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        db.put(
            Column::ProposedPbftBlocks,
            H256::from_low_u64_be(1).as_bytes(),
            &[0xAA],
        );
        db.put(
            Column::ProposedPbftBlocks,
            H256::from_low_u64_be(2).as_bytes(),
            &[0xBB],
        );

        let mut res = repo.proposed_rlp().unwrap();
        res.sort();
        assert_eq!(res, vec![vec![0xAA], vec![0xBB]]);
    }

    #[test]
    fn test_proposed_pbft_block_batch_delete_waits_for_commit() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let hash = H256::from_low_u64_be(3);

        db.put(Column::ProposedPbftBlocks, hash.as_bytes(), &[0xCC]);

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.remove_proposed_in_batch(&mut batch, hash).unwrap();

        assert_eq!(repo.proposed_rlp().unwrap(), vec![vec![0xCC]]);

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert!(repo.proposed_rlp().unwrap().is_empty());
    }

    #[test]
    fn test_pbft_head() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let hash = H256::from_low_u64_be(9);
        let head = b"head-data".to_vec();

        assert_eq!(repo.head(hash).unwrap(), None);
        db.put(Column::PbftHead, hash.as_bytes(), &head);
        assert_eq!(repo.head(hash).unwrap(), Some(head));
    }

    #[test]
    fn test_own_verified_votes_rlp() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        db.put(
            Column::LatestRoundOwnVotes,
            H256::from_low_u64_be(11).as_bytes(),
            &[0xA1],
        );
        db.put(
            Column::LatestRoundOwnVotes,
            H256::from_low_u64_be(12).as_bytes(),
            &[0xA2],
        );

        let mut res = repo.own_verified_votes_rlp().unwrap();
        res.sort();
        assert_eq!(res, vec![vec![0xA1], vec![0xA2]]);
    }

    #[test]
    fn own_verified_vote_records_reject_non_hash_key() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        db.put(Column::LatestRoundOwnVotes, &[0x11; 31], &[0xA1]);

        let error = repo.own_verified_vote_records().unwrap_err().to_string();
        assert!(error.contains("expected 32, got 31"));
    }

    #[test]
    fn extra_reward_vote_records_reject_non_hash_key() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        db.put(Column::ExtraRewardVotes, &[0x22; 31], &[0xB1]);

        let error = repo.extra_reward_vote_records().unwrap_err().to_string();
        assert!(error.contains("expected 32, got 31"));
    }

    #[test]
    fn finalized_reward_vote_cursor_round_trips_through_batch() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let cursor = StoredFinalizedRewardVoteCursor {
            period: 12,
            round: 2,
            step: 3,
            block_hash: H256::from([0x44; 32]),
            votes_bundle_rlp: vec![0xc1, 0x01],
        };
        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_finalized_reward_vote_cursor_in_batch(&mut batch, cursor.clone())
            .unwrap();
        assert!(repo.finalized_reward_vote_cursor().unwrap().is_none());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();
        assert_eq!(repo.finalized_reward_vote_cursor().unwrap(), Some(cursor));
    }

    #[test]
    fn test_all_two_t_plus_one_votes_rlp() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        let mut soft_votes = rlp::RlpStream::new_list(1);
        soft_votes.append_raw(&[0xC1, 0xA1], 1);
        db.put(
            Column::LatestRoundTwoTPlusOneVotes,
            &[TwoTPlusOneVotedBlockType::SoftVoted as u8],
            soft_votes.out().as_ref(),
        );

        let mut cert_votes = rlp::RlpStream::new_list(2);
        cert_votes.append_raw(&[0xC1, 0xB1], 1);
        cert_votes.append_raw(&[0xC1, 0xB2], 1);
        db.put(
            Column::LatestRoundTwoTPlusOneVotes,
            &[TwoTPlusOneVotedBlockType::CertVoted as u8],
            cert_votes.out().as_ref(),
        );

        let res = repo.all_two_t_plus_one_votes_rlp().unwrap();
        assert_eq!(
            res,
            vec![vec![0xC1, 0xA1], vec![0xC1, 0xB1], vec![0xC1, 0xB2]]
        );
    }

    #[test]
    fn test_reward_votes_rlp() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        db.put(
            Column::ExtraRewardVotes,
            H256::from_low_u64_be(21).as_bytes(),
            &[0xF1],
        );
        db.put(
            Column::ExtraRewardVotes,
            H256::from_low_u64_be(22).as_bytes(),
            &[0xF2],
        );

        let mut res = repo.reward_votes_rlp().unwrap();
        res.sort();
        assert_eq!(res, vec![vec![0xF1], vec![0xF2]]);
    }

    #[test]
    fn test_vote_writes_wait_for_batch_commit() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let own_hash = H256::from_low_u64_be(31);
        let reward_hash = H256::from_low_u64_be(32);

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_own_verified_vote_in_batch(&mut batch, own_hash, &[0xA1])
            .unwrap();
        repo.write_extra_reward_vote_in_batch(&mut batch, reward_hash, &[0xB1])
            .unwrap();

        assert!(repo.own_verified_votes_rlp().unwrap().is_empty());
        assert!(repo.reward_votes_rlp().unwrap().is_empty());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(repo.own_verified_votes_rlp().unwrap(), vec![vec![0xA1]]);
        assert_eq!(repo.reward_votes_rlp().unwrap(), vec![vec![0xB1]]);
    }

    #[test]
    fn test_cert_voted_block_write_waits_for_batch_commit() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_cert_voted_block_in_round_in_batch(&mut batch, 12, &[0xC0])
            .unwrap();

        assert!(repo.cert_voted_block_in_round_rlp().unwrap().is_none());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        let mut expected = rlp::RlpStream::new_list(2);
        expected.append(&12u64);
        expected.append_raw(&[0xC0], 1);
        assert_eq!(
            repo.cert_voted_block_in_round_rlp().unwrap(),
            Some(expected.out().to_vec())
        );
    }

    #[test]
    fn test_manager_transition_writes_wait_for_batch_commit() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());

        db.put(Column::PbftMgrRoundStep, &[0], &1u32.to_le_bytes());
        db.put(Column::PbftMgrRoundStep, &[1], &1u32.to_le_bytes());
        db.put(Column::PbftMgrStatus, &[2], &[1]);
        db.put(
            Column::CertVotedBlockInRound,
            &SINGLE_VALUE_KEY,
            &[0xC2, 0x01, 0x02],
        );

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.write_manager_field_in_batch(&mut batch, 0, 7).unwrap();
        repo.write_manager_field_in_batch(&mut batch, 1, 4).unwrap();
        repo.write_manager_status_in_batch(&mut batch, 2, false)
            .unwrap();
        repo.write_head_in_batch(&mut batch, H256::from_low_u64_be(51), &[0xA5])
            .unwrap();
        repo.remove_cert_voted_block_in_round_in_batch(&mut batch)
            .unwrap();

        assert_eq!(repo.manager_field(0).unwrap(), Some(1));
        assert_eq!(repo.manager_field(1).unwrap(), Some(1));
        assert_eq!(repo.manager_status(2).unwrap(), Some(true));
        assert_eq!(repo.head(H256::from_low_u64_be(51)).unwrap(), None);
        assert!(repo.cert_voted_block_in_round_rlp().unwrap().is_some());

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(repo.manager_field(0).unwrap(), Some(7));
        assert_eq!(repo.manager_field(1).unwrap(), Some(4));
        assert_eq!(repo.manager_status(2).unwrap(), Some(false));
        assert_eq!(
            repo.head(H256::from_low_u64_be(51)).unwrap(),
            Some(vec![0xA5])
        );
        assert!(repo.cert_voted_block_in_round_rlp().unwrap().is_none());
    }

    #[test]
    fn test_vote_deletes_wait_for_batch_commit() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let own_hash = H256::from_low_u64_be(41);
        let reward_hash = H256::from_low_u64_be(42);
        db.put(Column::LatestRoundOwnVotes, own_hash.as_bytes(), &[0xA1]);
        db.put(Column::ExtraRewardVotes, reward_hash.as_bytes(), &[0xB1]);

        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.remove_own_verified_vote_in_batch(&mut batch, own_hash)
            .unwrap();
        repo.remove_extra_reward_vote_in_batch(&mut batch, reward_hash)
            .unwrap();

        assert_eq!(repo.own_verified_votes_rlp().unwrap(), vec![vec![0xA1]]);
        assert_eq!(repo.reward_votes_rlp().unwrap(), vec![vec![0xB1]]);

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert!(repo.own_verified_votes_rlp().unwrap().is_empty());
        assert!(repo.reward_votes_rlp().unwrap().is_empty());
    }

    #[test]
    fn test_two_t_plus_one_batch_replaces_bundle() {
        let db = Arc::new(MockPbftStore::new());
        let repo = PbftRepository::new(db.clone());
        let vote_type = TwoTPlusOneVotedBlockType::SoftVoted as u8;

        let mut old_bundle = rlp::RlpStream::new_list(1);
        old_bundle.append_raw(&[0xC1, 0xA1], 1);
        db.put(
            Column::LatestRoundTwoTPlusOneVotes,
            &[vote_type],
            old_bundle.out().as_ref(),
        );

        let mut new_bundle = rlp::RlpStream::new_list(1);
        new_bundle.append_raw(&[0xC1, 0xB1], 1);
        let mut batch = DbWriter::create_batch(db.as_ref());
        repo.replace_two_t_plus_one_votes_in_batch(
            &mut batch,
            vote_type,
            new_bundle.out().as_ref(),
        )
        .unwrap();

        assert_eq!(
            repo.all_two_t_plus_one_votes_rlp().unwrap(),
            vec![vec![0xC1, 0xA1]]
        );

        DbWriter::commit_batch(db.as_ref(), batch).unwrap();

        assert_eq!(
            repo.all_two_t_plus_one_votes_rlp().unwrap(),
            vec![vec![0xC1, 0xB1]]
        );
    }
}
