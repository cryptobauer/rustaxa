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
}

impl<D: DbReader + DbWriter> PbftRepository<D> {
    /// Persists a PBFT manager numeric field value.
    /// C++ mapping: `DbStorage::savePbftMgrField(PbftMgrField, uint32_t)`.
    pub fn write_manager_field(&self, field: u8, value: u32) -> Result<()> {
        self.db
            .put(Column::PbftMgrRoundStep, &[field], &value.to_le_bytes())
    }

    /// Persists a PBFT manager status flag.
    /// C++ mapping: `DbStorage::savePbftMgrStatus(PbftMgrStatus, bool const&)`.
    pub fn write_manager_status(&self, field: u8, value: bool) -> Result<()> {
        self.db
            .put(Column::PbftMgrStatus, &[field], &[u8::from(value)])
    }

    /// Stores serialized PBFT head bytes for a head hash.
    /// C++ mapping: `DbStorage::savePbftHead(blk_hash_t const&, std::string const&)`.
    pub fn write_head(&self, pbft_hash: H256, head_bytes: &[u8]) -> Result<()> {
        self.db
            .put(Column::PbftHead, pbft_hash.as_bytes(), head_bytes)
    }

    /// Stores one locally produced verified vote payload.
    /// C++ mapping: `DbStorage::saveOwnVerifiedVote(const std::shared_ptr<PbftVote>&)`.
    pub fn write_own_verified_vote(&self, vote_hash: H256, vote_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::LatestRoundOwnVotes, vote_hash.as_bytes(), vote_rlp)
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

    /// Stores one extra reward vote payload.
    /// C++ mapping: `DbStorage::saveExtraRewardVote(const std::shared_ptr<PbftVote>&)`.
    pub fn write_extra_reward_vote(&self, vote_hash: H256, vote_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::ExtraRewardVotes, vote_hash.as_bytes(), vote_rlp)
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

    /// Removes one proposed PBFT block by hash.
    /// C++ mapping: `DbStorage::removeProposedPbftBlock(const blk_hash_t&, Batch&)`.
    pub fn remove_proposed(&self, block_hash: H256) -> Result<()> {
        self.db
            .delete(Column::ProposedPbftBlocks, block_hash.as_bytes())
    }

    /// Removes one cached own verified vote by vote hash.
    /// C++ mapping: `DbStorage::clearOwnVerifiedVotes(Batch&, const std::vector<std::shared_ptr<PbftVote>>&)`.
    pub fn remove_own_verified_vote(&self, vote_hash: H256) -> Result<()> {
        self.db
            .delete(Column::LatestRoundOwnVotes, vote_hash.as_bytes())
    }

    /// Removes one cached extra reward vote by vote hash.
    /// C++ mapping: `DbStorage::removeExtraRewardVotes(const std::vector<vote_hash_t>&, Batch&)`.
    pub fn remove_extra_reward_vote(&self, vote_hash: H256) -> Result<()> {
        self.db
            .delete(Column::ExtraRewardVotes, vote_hash.as_bytes())
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
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

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
}
