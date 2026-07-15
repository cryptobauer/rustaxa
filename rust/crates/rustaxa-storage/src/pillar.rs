use anyhow::{Result, anyhow};
use std::sync::Arc;

use crate::Column;
use crate::SINGLE_VALUE_KEY;
use crate::db::{DbReader, DbWriter};

pub struct PillarRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> PillarRepository<D> {
    /// Creates a pillar repository over the shared database handle.
    pub fn new(db: Arc<D>) -> Self {
        PillarRepository { db }
    }

    /// Returns serialized pillar block payload for a period when available.
    /// C++ mapping: `DbStorage::getPillarBlock(PbftPeriod) const`.
    pub fn rlp(&self, period: u64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::PillarBlock, &period.to_le_bytes())?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }

    /// Returns the newest serialized pillar block payload in storage.
    /// C++ mapping: `DbStorage::getLatestPillarBlock() const`.
    pub fn latest_rlp(&self) -> Result<Option<Vec<u8>>> {
        let mut latest: Option<(u64, Vec<u8>)> = None;

        for item in self.db.iter(Column::PillarBlock) {
            let (key, value) = item?;
            if key.len() != 8 {
                continue;
            }
            let period = u64::from_le_bytes(
                key.as_ref()
                    .try_into()
                    .map_err(|_| anyhow!("pillar block period key has invalid length"))?,
            );
            if value.is_empty() {
                continue;
            }
            let should_replace = latest
                .as_ref()
                .map(|(candidate, _)| period > *candidate)
                .unwrap_or(true);
            if should_replace {
                latest = Some((period, value.to_vec()));
            }
        }

        Ok(latest.map(|(_, value)| value))
    }

    /// Returns serialized local pillar vote payload for current pillar state.
    /// C++ mapping: `DbStorage::getOwnPillarBlockVote() const`.
    pub fn own_vote_rlp(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::CurrentPillarBlockOwnVote, &SINGLE_VALUE_KEY)?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }

    /// Returns serialized current pillar-chain state payload.
    /// C++ mapping: `DbStorage::getCurrentPillarBlockData() const`.
    pub fn current_data_rlp(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::CurrentPillarBlockData, &SINGLE_VALUE_KEY)?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }
}

impl<D: DbReader + DbWriter> PillarRepository<D> {
    /// Stores serialized pillar block payload for its period.
    /// C++ mapping: `DbStorage::savePillarBlock(const std::shared_ptr<pillar_chain::PillarBlock>&)`.
    pub fn write(&self, period: u64, pillar_block_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::PillarBlock, &period.to_le_bytes(), pillar_block_rlp)
    }

    /// Stores serialized local pillar vote payload.
    /// C++ mapping: `DbStorage::saveOwnPillarBlockVote(const std::shared_ptr<PillarVote>&)`.
    pub fn write_own_vote(&self, vote_rlp: &[u8]) -> Result<()> {
        self.db.put(
            Column::CurrentPillarBlockOwnVote,
            &SINGLE_VALUE_KEY,
            vote_rlp,
        )
    }

    /// Stores serialized current pillar-chain state payload.
    /// C++ mapping: `DbStorage::saveCurrentPillarBlockData(const pillar_chain::CurrentPillarBlockDataDb&)`.
    pub fn write_current_data(&self, data_rlp: &[u8]) -> Result<()> {
        self.db
            .put(Column::CurrentPillarBlockData, &SINGLE_VALUE_KEY, data_rlp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockPillarStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockPillarStore {
        fn new() -> Self {
            MockPillarStore {
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

    impl DbReader for MockPillarStore {
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
    fn test_pillar_block_rlp() {
        let db = Arc::new(MockPillarStore::new());
        let repo = PillarRepository::new(db.clone());
        let period = 8u64;
        let block = vec![0xCA, 0x01];

        assert_eq!(repo.rlp(period).unwrap(), None);

        db.put(Column::PillarBlock, &period.to_le_bytes(), &block);
        assert_eq!(repo.rlp(period).unwrap(), Some(block));
    }

    #[test]
    fn test_latest_pillar_block_rlp() {
        let db = Arc::new(MockPillarStore::new());
        let repo = PillarRepository::new(db.clone());
        assert_eq!(repo.latest_rlp().unwrap(), None);

        db.put(Column::PillarBlock, &1u64.to_le_bytes(), &[0xA1]);
        db.put(Column::PillarBlock, &5u64.to_le_bytes(), &[0xA5]);
        db.put(Column::PillarBlock, &3u64.to_le_bytes(), &[0xA3]);

        assert_eq!(repo.latest_rlp().unwrap(), Some(vec![0xA5]));
    }

    #[test]
    fn latest_pillar_block_rlp_uses_numerically_correct_period_order() {
        let db = Arc::new(MockPillarStore::new());
        let repo = PillarRepository::new(db.clone());
        assert_eq!(repo.latest_rlp().unwrap(), None);

        db.put(Column::PillarBlock, &254u64.to_le_bytes(), &[0xA0]);
        db.put(Column::PillarBlock, &255u64.to_le_bytes(), &[0xA1]);
        db.put(Column::PillarBlock, &256u64.to_le_bytes(), &[0xA2]);
        assert_eq!(repo.latest_rlp().unwrap(), Some(vec![0xA2]));

        db.put(Column::PillarBlock, &4096u64.to_le_bytes(), &[0xA3]);
        assert_eq!(repo.latest_rlp().unwrap(), Some(vec![0xA3]));
    }

    #[test]
    fn test_own_pillar_block_vote_rlp() {
        let db = Arc::new(MockPillarStore::new());
        let repo = PillarRepository::new(db.clone());
        let vote = vec![0xD1, 0x11];

        assert_eq!(repo.own_vote_rlp().unwrap(), None);

        db.put(Column::CurrentPillarBlockOwnVote, &SINGLE_VALUE_KEY, &vote);
        assert_eq!(repo.own_vote_rlp().unwrap(), Some(vote));
    }

    #[test]
    fn test_current_pillar_block_data_rlp() {
        let db = Arc::new(MockPillarStore::new());
        let repo = PillarRepository::new(db.clone());
        let data = vec![0xC1, 0x42];

        assert_eq!(repo.current_data_rlp().unwrap(), None);

        db.put(Column::CurrentPillarBlockData, &SINGLE_VALUE_KEY, &data);
        assert_eq!(repo.current_data_rlp().unwrap(), Some(data));
    }
}
