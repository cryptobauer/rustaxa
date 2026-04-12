use anyhow::Result;
use std::sync::Arc;

use crate::Column;
use crate::SINGLE_VALUE_KEY;
use crate::StorageError;
use crate::db::DbReader;

pub struct MetadataRepository<D: DbReader> {
    db: Arc<D>,
}

impl<D: DbReader> MetadataRepository<D> {
    pub fn new(db: Arc<D>) -> Self {
        MetadataRepository { db }
    }

    /// Implements getGenesisHash() -> optional(bytes)
    pub fn genesis_hash_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(self
            .db
            .get(Column::Genesis, &SINGLE_VALUE_KEY)?
            .map(|value| value.as_ref().to_vec())
            .filter(|value| !value.is_empty()))
    }

    /// Implements getLastSortitionParams(count) -> [rlp(sortition_params_change)]
    ///
    /// The returned list is ordered oldest to newest, matching the C++ storage API.
    /// We iterate the column in reverse so we can stop after collecting the latest
    /// `count` entries, then reverse the collected slice to restore the expected
    /// chronological order.
    pub fn last_sortition_params_changes_rlp(&self, count: usize) -> Result<Vec<Vec<u8>>> {
        let mut changes = Vec::new();
        for item in self.db.iter_rev(Column::SortitionParamsChange) {
            if changes.len() >= count {
                break;
            }
            let (_, value) = item?;
            changes.push(value.into_vec());
        }

        changes.reverse();
        Ok(changes)
    }

    /// Implements getParamsChangeForPeriod(period) -> optional(rlp(sortition_params_change))
    pub fn params_change_for_period_rlp(&self, period: u64) -> Result<Option<Vec<u8>>> {
        let Some((_, value)) = self
            .db
            .get_at_or_before(Column::SortitionParamsChange, &period.to_le_bytes())?
        else {
            return Ok(None);
        };

        Ok(Some(value.into_vec()))
    }

    /// Implements getStatusField(field) -> uint64_t
    pub fn status_field(&self, field: u8) -> Result<u64> {
        let Some(value) = self.db.get(Column::Status, &[field])? else {
            return Ok(0);
        };

        Self::decode_u64_value(value.as_ref(), Column::Status)
    }

    /// Implements getPeriodLambda(period, find_closest) -> optional(uint32_t)
    pub fn period_lambda(&self, period: u64, find_closest: bool) -> Result<Option<u32>> {
        if find_closest {
            let Some((_, value)) = self
                .db
                .get_at_or_before(Column::PeriodLambda, &period.to_le_bytes())?
            else {
                return Ok(None);
            };
            return Self::decode_u32_value(value.as_ref(), Column::PeriodLambda).map(Some);
        }

        let Some(value) = self.db.get(Column::PeriodLambda, &period.to_le_bytes())? else {
            return Ok(None);
        };

        Self::decode_u32_value(value.as_ref(), Column::PeriodLambda).map(Some)
    }

    /// Implements getRoundsCountDynamicLambda() -> uint32_t
    pub fn rounds_count_dynamic_lambda(&self) -> Result<u32> {
        let Some(value) = self
            .db
            .get(Column::RoundsCountDynamicLambda, &SINGLE_VALUE_KEY)?
        else {
            return Ok(0);
        };

        Self::decode_u32_value(value.as_ref(), Column::RoundsCountDynamicLambda)
    }

    /// Implements getBlocksRewardsStats() -> [(period, rlp(block_stats))]
    pub fn block_rewards_stats_rlp(&self) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut stats = Vec::new();
        for item in self.db.iter(Column::BlockRewardsStats) {
            let (key, value) = item?;
            let period = Self::decode_u64_key(&key, Column::BlockRewardsStats)?;
            stats.push((period, value.into_vec()));
        }

        Ok(stats)
    }

    fn decode_u64_key(bytes: &[u8], column: Column) -> Result<u64> {
        if bytes.len() != std::mem::size_of::<u64>() {
            return Err(StorageError::Read(format!(
                "Invalid key size in {}: expected {}, got {}",
                column.name(),
                std::mem::size_of::<u64>(),
                bytes.len()
            ))
            .into());
        }

        let mut arr = [0u8; 8];
        arr.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(arr))
    }

    fn decode_u64_value(bytes: &[u8], column: Column) -> Result<u64> {
        if bytes.len() != std::mem::size_of::<u64>() {
            return Err(StorageError::Read(format!(
                "Invalid value size in {}: expected {}, got {}",
                column.name(),
                std::mem::size_of::<u64>(),
                bytes.len()
            ))
            .into());
        }

        let mut arr = [0u8; 8];
        arr.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(arr))
    }

    fn decode_u32_value(bytes: &[u8], column: Column) -> Result<u32> {
        if bytes.len() != std::mem::size_of::<u32>() {
            return Err(StorageError::Read(format!(
                "Invalid value size in {}: expected {}, got {}",
                column.name(),
                std::mem::size_of::<u32>(),
                bytes.len()
            ))
            .into());
        }

        let mut arr = [0u8; 4];
        arr.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbIterator;
    use std::collections::{BTreeMap, HashMap};
    use std::sync::RwLock;

    struct MockMetadataStore {
        data: RwLock<HashMap<String, BTreeMap<Vec<u8>, Vec<u8>>>>,
    }

    impl MockMetadataStore {
        fn new() -> Self {
            MockMetadataStore {
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

    impl DbReader for MockMetadataStore {
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
    fn test_genesis_hash_bytes() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());
        let genesis = vec![0xAB; 32];

        assert_eq!(repo.genesis_hash_bytes().unwrap(), None);
        db.put(Column::Genesis, &SINGLE_VALUE_KEY, &genesis);
        assert_eq!(repo.genesis_hash_bytes().unwrap(), Some(genesis));
    }

    #[test]
    fn test_last_sortition_params_changes_rlp_order_and_limit() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        db.put(Column::SortitionParamsChange, &1u64.to_le_bytes(), &[0xA1]);
        db.put(Column::SortitionParamsChange, &3u64.to_le_bytes(), &[0xA3]);
        db.put(Column::SortitionParamsChange, &5u64.to_le_bytes(), &[0xA5]);

        assert_eq!(
            repo.last_sortition_params_changes_rlp(2).unwrap(),
            vec![vec![0xA3], vec![0xA5]]
        );
        assert!(
            repo.last_sortition_params_changes_rlp(0)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_params_change_for_period_rlp() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        db.put(Column::SortitionParamsChange, &2u64.to_le_bytes(), &[0xB2]);
        db.put(Column::SortitionParamsChange, &4u64.to_le_bytes(), &[0xB4]);

        assert_eq!(repo.params_change_for_period_rlp(1).unwrap(), None);
        assert_eq!(
            repo.params_change_for_period_rlp(2).unwrap(),
            Some(vec![0xB2])
        );
        assert_eq!(
            repo.params_change_for_period_rlp(3).unwrap(),
            Some(vec![0xB2])
        );
        assert_eq!(
            repo.params_change_for_period_rlp(4).unwrap(),
            Some(vec![0xB4])
        );
        assert_eq!(
            repo.params_change_for_period_rlp(7).unwrap(),
            Some(vec![0xB4])
        );
    }

    #[test]
    fn test_status_field() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        assert_eq!(repo.status_field(1).unwrap(), 0);
        db.put(Column::Status, &[1], &42u64.to_le_bytes());
        assert_eq!(repo.status_field(1).unwrap(), 42);
    }

    #[test]
    fn test_period_lambda() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        db.put(
            Column::PeriodLambda,
            &2u64.to_le_bytes(),
            &11u32.to_le_bytes(),
        );
        db.put(
            Column::PeriodLambda,
            &5u64.to_le_bytes(),
            &22u32.to_le_bytes(),
        );

        assert_eq!(repo.period_lambda(2, false).unwrap(), Some(11));
        assert_eq!(repo.period_lambda(3, false).unwrap(), None);

        assert_eq!(repo.period_lambda(1, true).unwrap(), None);
        assert_eq!(repo.period_lambda(2, true).unwrap(), Some(11));
        assert_eq!(repo.period_lambda(4, true).unwrap(), Some(11));
        assert_eq!(repo.period_lambda(5, true).unwrap(), Some(22));
        assert_eq!(repo.period_lambda(9, true).unwrap(), Some(22));
    }

    #[test]
    fn test_rounds_count_dynamic_lambda() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        assert_eq!(repo.rounds_count_dynamic_lambda().unwrap(), 0);
        db.put(
            Column::RoundsCountDynamicLambda,
            &SINGLE_VALUE_KEY,
            &9u32.to_le_bytes(),
        );
        assert_eq!(repo.rounds_count_dynamic_lambda().unwrap(), 9);
    }

    #[test]
    fn test_block_rewards_stats_rlp() {
        let db = Arc::new(MockMetadataStore::new());
        let repo = MetadataRepository::new(db.clone());

        db.put(Column::BlockRewardsStats, &3u64.to_le_bytes(), &[0xC3]);
        db.put(Column::BlockRewardsStats, &7u64.to_le_bytes(), &[0xC7]);

        assert_eq!(
            repo.block_rewards_stats_rlp().unwrap(),
            vec![(3u64, vec![0xC3]), (7u64, vec![0xC7])]
        );
    }
}
