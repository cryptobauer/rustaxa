//! Deterministic transaction-pool metadata for Rust-backed transaction queue shims.
//!
//! The queue stores transaction metadata and canonical transaction bytes needed for consensus-facing pool decisions.
//! C++ supplies validated RLP at insertion time and materializes `Transaction` objects on demand for legacy API callers.
//! C++ still owns signature/state validation, known-cache expiration, event dispatch, overflow wall-clock state, and
//! FinalChain account reads. Rust owns deterministic insertion, same-sender nonce replacement, priority ordering,
//! non-proposable expiry planning, queued payload retention, and gas-price threshold accounting.

use anyhow::{Result, ensure};
use ethereum_types::{H160, H256, U256};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Metadata for a transaction known to the Rust queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionQueueEntry {
    /// Canonical transaction hash.
    pub hash: H256,
    /// Transaction sender.
    pub sender: H160,
    /// Transaction nonce.
    pub nonce: U256,
    /// Transaction gas price.
    pub gas_price: U256,
    /// Transaction gas limit.
    pub gas: u64,
    /// Raw transaction data size in bytes.
    pub data_size: u64,
    /// Canonical transaction RLP bytes retained while the transaction is queued.
    pub rlp: Vec<u8>,
    /// Final-chain block number observed when the transaction became non-proposable.
    pub last_block_number: u64,
}

/// Insert status mirroring C++ `TransactionStatus`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionQueueInsertStatus {
    #[default]
    Inserted,
    InsertedNonProposable,
    Known,
    Overflow,
}

/// Result of inserting a transaction into [`TransactionQueue`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionQueueInsertOutcome {
    /// Insert status.
    pub status: TransactionQueueInsertStatus,
    /// Hash accepted by Rust, when an entry was accepted.
    pub inserted_hash: Option<H256>,
    /// Hashes that stayed known but moved from proposer ordering to non-proposer state.
    pub demoted_hashes: Vec<H256>,
    /// Hashes dropped entirely because proposer overflow eviction ran.
    pub overflow_removed_hashes: Vec<H256>,
}

/// Result of erasing one hash from Rust queue metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionQueueEraseOutcome {
    /// Whether the hash was removed.
    pub removed: bool,
    /// Whether the removed entry was proposable.
    pub removed_proposable: bool,
    /// Removed live metadata, when the hash existed.
    pub removed_entry: Option<TransactionQueueEntry>,
}

/// Result of explicit proposer -> non-proposer demotion for one known transaction.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransactionQueueDemoteStatus {
    #[default]
    NotFound,
    AlreadyNonProposable,
    Demoted,
}

/// Result of an explicit demotion attempt.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionQueueDemoteOutcome {
    /// Outcome status for the requested hash.
    pub status: TransactionQueueDemoteStatus,
    /// Final entry metadata after demotion (when known).
    pub entry: Option<TransactionQueueEntry>,
}

/// Deterministic result for ordered-read operations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionQueueOrderedHashesPlan {
    /// Ordered hashes that match the request.
    pub hashes: Vec<H256>,
    /// Caller requested cardinality.
    pub requested_count: u64,
    /// True when all proposer entries were returned.
    pub complete: bool,
}

/// Result of purge-like operations.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TransactionQueuePurgeOutcome {
    /// Hashes removed from Rust queue metadata.
    pub removed_hashes: Vec<H256>,
}

#[derive(Clone, Debug)]
struct StoredTransaction {
    entry: TransactionQueueEntry,
    proposable: bool,
}

/// Rust-owned transaction queue metadata and deterministic ordering state.
///
/// Invariants:
/// - a hash appears at most once across proposer and non-proposer indexes
/// - each `(sender, nonce)` proposer slot points to the highest-gas-price proposable transaction for that slot
/// - gas-price totals include only proposer transactions
/// - `data_size` includes proposer and non-proposer payload sizes
pub struct TransactionQueue {
    entries: HashMap<H256, StoredTransaction>,
    account_nonce_transactions: BTreeMap<H160, BTreeMap<U256, H256>>,
    queue_transactions_gas_prices: BTreeMap<Reverse<U256>, u64>,
    non_proposable_transactions: BTreeMap<H256, u64>,
    data_size: u64,
    max_size: u64,
    max_data_size: u64,
    max_non_proposable_size: u64,
    max_single_account_transactions_size: u64,
    non_proposable_expiry_limit: u64,
}

impl TransactionQueue {
    /// Creates an empty transaction queue with the same derived limits as the legacy C++ queue.
    pub fn new(max_size: u64) -> Self {
        Self {
            entries: HashMap::new(),
            account_nonce_transactions: BTreeMap::new(),
            queue_transactions_gas_prices: BTreeMap::new(),
            non_proposable_transactions: BTreeMap::new(),
            data_size: 0,
            max_size,
            max_data_size: max_size.saturating_mul(1024),
            max_non_proposable_size: max_size * 20 / 100,
            max_single_account_transactions_size: max_size * 5 / 100,
            non_proposable_expiry_limit: 10,
        }
    }

    /// Returns the number of proposable transactions.
    pub fn size(&self) -> u64 {
        self.entries
            .values()
            .filter(|entry| entry.proposable)
            .count() as u64
    }

    /// Returns true when either proposer or non-proposer indexes contain `hash`.
    pub fn contains(&self, hash: H256) -> bool {
        self.entries.contains_key(&hash)
    }

    /// Returns canonical transaction bytes and metadata for a queued hash.
    ///
    /// The returned entry is cloned because the CXX bridge transfers owned bytes
    /// to C++ for legacy `Transaction` materialization. Missing hashes return `None`.
    pub fn transaction(&self, hash: H256) -> Option<TransactionQueueEntry> {
        self.entries.get(&hash).map(|stored| stored.entry.clone())
    }

    /// Returns true when non-proposer transactions reached their configured limit.
    pub fn non_proposable_transactions_over_the_limit(&self) -> bool {
        self.non_proposable_transactions.len() as u64 >= self.max_non_proposable_size
    }

    /// Inserts a transaction and returns the pointer/index updates C++ must mirror.
    pub fn insert(
        &mut self,
        entry: TransactionQueueEntry,
        proposable: bool,
    ) -> Result<TransactionQueueInsertOutcome> {
        ensure!(
            !entry.hash.is_zero(),
            "transaction queue entry hash cannot be zero"
        );
        if self.contains(entry.hash) {
            return Ok(TransactionQueueInsertOutcome {
                status: TransactionQueueInsertStatus::Known,
                ..Default::default()
            });
        }

        if self.data_size > self.max_data_size {
            return Ok(TransactionQueueInsertOutcome {
                status: TransactionQueueInsertStatus::Overflow,
                ..Default::default()
            });
        }

        if proposable {
            self.insert_proposable(entry)
        } else {
            self.insert_non_proposable(entry)
        }
    }

    /// Removes a transaction by hash from any queue index.
    pub fn erase(&mut self, hash: H256) -> bool {
        self.erase_plan(hash).removed
    }

    /// Removes a transaction by hash and returns a C++ mirror mutation plan.
    pub fn erase_plan(&mut self, hash: H256) -> TransactionQueueEraseOutcome {
        let Some(stored) = self.entries.remove(&hash) else {
            return TransactionQueueEraseOutcome {
                removed: false,
                removed_proposable: false,
                removed_entry: None,
            };
        };
        self.data_size = self.data_size.saturating_sub(stored.entry.data_size);
        if stored.proposable {
            self.remove_proposable_indexes(&stored.entry);
        } else {
            self.non_proposable_transactions.remove(&hash);
        }
        TransactionQueueEraseOutcome {
            removed: true,
            removed_proposable: stored.proposable,
            removed_entry: Some(stored.entry),
        }
    }

    /// Attempts to demote one transaction to non-proposable.
    pub fn demote(&mut self, hash: H256, last_block_number: u64) -> TransactionQueueDemoteOutcome {
        let Some(mut stored) = self.entries.remove(&hash) else {
            return TransactionQueueDemoteOutcome {
                status: TransactionQueueDemoteStatus::NotFound,
                entry: None,
            };
        };

        if !stored.proposable {
            let entry = stored.entry.clone();
            self.entries.insert(hash, stored);
            return TransactionQueueDemoteOutcome {
                status: TransactionQueueDemoteStatus::AlreadyNonProposable,
                entry: Some(entry),
            };
        }

        self.remove_proposable_indexes(&stored.entry);
        stored.entry.last_block_number = last_block_number;
        stored.proposable = false;
        self.non_proposable_transactions
            .insert(hash, stored.entry.last_block_number);
        let entry = stored.entry.clone();
        self.entries.insert(hash, stored);
        TransactionQueueDemoteOutcome {
            status: TransactionQueueDemoteStatus::Demoted,
            entry: Some(entry),
        }
    }

    /// Removes expired non-proposer transactions for a newly finalized block number.
    pub fn block_finalized(&mut self, block_number: u64) -> Vec<H256> {
        self.block_finalized_plan(block_number).removed_hashes
    }

    /// Removes expired non-proposer transactions and returns a C++ mirror mutation plan.
    pub fn block_finalized_plan(&mut self, block_number: u64) -> TransactionQueuePurgeOutcome {
        let expired = self
            .non_proposable_transactions
            .iter()
            .filter_map(|(hash, last_block_number)| {
                (last_block_number + self.non_proposable_expiry_limit < block_number)
                    .then_some(*hash)
            })
            .collect::<Vec<_>>();
        for hash in &expired {
            self.erase(*hash);
        }
        TransactionQueuePurgeOutcome {
            removed_hashes: expired,
        }
    }

    /// Removes proposer transactions whose nonce is lower than the finalized account nonce.
    pub fn purge_account(&mut self, sender: H160, account_nonce: U256) -> Vec<H256> {
        self.purge_account_plan(sender, account_nonce)
            .removed_hashes
    }

    /// Removes proposer transactions for `sender` below `account_nonce` and returns a mutation plan.
    pub fn purge_account_plan(
        &mut self,
        sender: H160,
        account_nonce: U256,
    ) -> TransactionQueuePurgeOutcome {
        let removed = self
            .account_nonce_transactions
            .get(&sender)
            .map(|nonces| {
                nonces
                    .range(..account_nonce)
                    .map(|(_, hash)| *hash)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for hash in &removed {
            self.erase(*hash);
        }
        TransactionQueuePurgeOutcome {
            removed_hashes: removed,
        }
    }

    /// Returns all accounts that currently own proposer transactions.
    pub fn proposable_accounts(&self) -> Vec<H160> {
        self.account_nonce_transactions.keys().copied().collect()
    }

    /// Returns proposer ordering with per-account nonce gating and deterministic gas-price priority.
    pub fn ordered_hashes(&self, count: u64) -> Vec<H256> {
        self.ordered_hashes_plan(count).hashes
    }

    /// Returns proposer ordering metadata for ordered-read callers.
    pub fn ordered_hashes_plan(&self, count: u64) -> TransactionQueueOrderedHashesPlan {
        let mut iterators = self
            .account_nonce_transactions
            .iter()
            .map(|(sender, transactions)| (*sender, transactions.iter()))
            .collect::<Vec<_>>();
        let mut heads = BTreeSet::<(Reverse<U256>, H160, U256, H256)>::new();

        for (sender, iterator) in &mut iterators {
            if let Some((nonce, hash)) = iterator.next()
                && let Some(entry) = self.entries.get(hash)
            {
                heads.insert((Reverse(entry.entry.gas_price), *sender, *nonce, *hash));
            }
        }

        let mut selected = Vec::new();
        while let Some((_, sender, _, hash)) = heads.pop_first() {
            selected.push(hash);
            if selected.len() == count as usize {
                break;
            }
            if let Some((_, iterator)) = iterators
                .iter_mut()
                .find(|(candidate, _)| *candidate == sender)
                && let Some((next_nonce, next_hash)) = iterator.next()
                && let Some(entry) = self.entries.get(next_hash)
            {
                heads.insert((
                    Reverse(entry.entry.gas_price),
                    sender,
                    *next_nonce,
                    *next_hash,
                ));
            }
        }
        TransactionQueueOrderedHashesPlan {
            hashes: selected,
            requested_count: count,
            complete: self.size() <= count,
        }
    }

    /// Returns proposer transaction hashes grouped per sender and ordered by nonce within each sender.
    pub fn all_transactions_grouped(&self) -> Vec<Vec<H256>> {
        self.account_nonce_transactions
            .values()
            .map(|transactions| transactions.values().copied().collect())
            .collect()
    }

    /// Returns proposer-ordered queued transactions with canonical bytes.
    pub fn ordered_transactions(&self, count: u64) -> Vec<TransactionQueueEntry> {
        self.ordered_hashes(count)
            .into_iter()
            .filter_map(|hash| self.transaction(hash))
            .collect()
    }

    /// Returns proposer transactions grouped per sender and ordered by nonce within each sender.
    pub fn all_transaction_groups(&self) -> Vec<Vec<TransactionQueueEntry>> {
        self.account_nonce_transactions
            .values()
            .map(|transactions| {
                transactions
                    .values()
                    .filter_map(|hash| self.transaction(*hash))
                    .collect()
            })
            .collect()
    }

    /// Returns the minimum gas price needed for inclusion under a gas limit.
    pub fn min_gas_price_for_block_inclusion(&self, limit: u64) -> U256 {
        let mut total_gas = 0_u64;
        for (gas_price, gas) in &self.queue_transactions_gas_prices {
            total_gas = total_gas.saturating_add(*gas);
            if total_gas >= limit {
                return gas_price.0 + U256::one();
            }
        }
        U256::one()
    }

    fn insert_proposable(
        &mut self,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionQueueInsertOutcome> {
        if self
            .account_nonce_transactions
            .get(&entry.sender)
            .map(|transactions| {
                transactions.len() as u64 == self.max_single_account_transactions_size
            })
            .unwrap_or(false)
        {
            return Ok(TransactionQueueInsertOutcome {
                status: TransactionQueueInsertStatus::Overflow,
                ..Default::default()
            });
        }

        let mut outcome = TransactionQueueInsertOutcome {
            status: TransactionQueueInsertStatus::Inserted,
            inserted_hash: Some(entry.hash),
            ..Default::default()
        };

        let existing_hash = self
            .account_nonce_transactions
            .get(&entry.sender)
            .and_then(|transactions| transactions.get(&entry.nonce))
            .copied();
        if let Some(existing_hash) = existing_hash {
            let existing_gas_price = self.entries[&existing_hash].entry.gas_price;
            if entry.gas_price > existing_gas_price {
                self.demote_proposable(existing_hash, entry.last_block_number)?;
                outcome.demoted_hashes.push(existing_hash);
                self.add_proposable(entry)?;
            } else {
                self.add_non_proposable(entry)?;
            }
        } else {
            self.add_proposable(entry)?;
        }

        if self.size() > self.max_size {
            let queue_size = self.size();
            let ordered = self.ordered_hashes(queue_size);
            let mut removed_count = 0_u64;
            for hash in ordered.into_iter().rev() {
                self.erase(hash);
                outcome.overflow_removed_hashes.push(hash);
                removed_count += 1;
                if removed_count >= queue_size / 100 {
                    break;
                }
            }
            if let Some(inserted_hash) = outcome.inserted_hash
                && !self.contains(inserted_hash)
            {
                outcome.status = TransactionQueueInsertStatus::Overflow;
            }
        }
        Ok(outcome)
    }

    fn insert_non_proposable(
        &mut self,
        entry: TransactionQueueEntry,
    ) -> Result<TransactionQueueInsertOutcome> {
        if self.non_proposable_transactions.len() as u64 <= self.max_non_proposable_size {
            let hash = entry.hash;
            self.add_non_proposable(entry)?;
            Ok(TransactionQueueInsertOutcome {
                status: TransactionQueueInsertStatus::InsertedNonProposable,
                inserted_hash: Some(hash),
                ..Default::default()
            })
        } else {
            Ok(TransactionQueueInsertOutcome {
                status: TransactionQueueInsertStatus::Overflow,
                ..Default::default()
            })
        }
    }

    fn add_proposable(&mut self, entry: TransactionQueueEntry) -> Result<()> {
        ensure!(
            !self.entries.contains_key(&entry.hash),
            "transaction queue proposable hash already exists"
        );
        self.data_size = self.data_size.saturating_add(entry.data_size);
        *self
            .queue_transactions_gas_prices
            .entry(Reverse(entry.gas_price))
            .or_default() += entry.gas;
        self.account_nonce_transactions
            .entry(entry.sender)
            .or_default()
            .insert(entry.nonce, entry.hash);
        self.entries.insert(
            entry.hash,
            StoredTransaction {
                entry,
                proposable: true,
            },
        );
        Ok(())
    }

    fn add_non_proposable(&mut self, entry: TransactionQueueEntry) -> Result<()> {
        ensure!(
            !self.entries.contains_key(&entry.hash),
            "transaction queue non-proposer hash already exists"
        );
        self.data_size = self.data_size.saturating_add(entry.data_size);
        self.non_proposable_transactions
            .insert(entry.hash, entry.last_block_number);
        self.entries.insert(
            entry.hash,
            StoredTransaction {
                entry,
                proposable: false,
            },
        );
        Ok(())
    }

    fn demote_proposable(&mut self, hash: H256, last_block_number: u64) -> Result<()> {
        let Some(mut stored) = self.entries.remove(&hash) else {
            return Ok(());
        };
        ensure!(
            stored.proposable,
            "only proposer transactions can be demoted"
        );
        self.remove_proposable_indexes(&stored.entry);
        stored.entry.last_block_number = last_block_number;
        stored.proposable = false;
        self.non_proposable_transactions
            .insert(hash, stored.entry.last_block_number);
        self.entries.insert(hash, stored);
        Ok(())
    }

    fn remove_proposable_indexes(&mut self, entry: &TransactionQueueEntry) {
        if let Some(transactions) = self.account_nonce_transactions.get_mut(&entry.sender) {
            transactions.remove(&entry.nonce);
            if transactions.is_empty() {
                self.account_nonce_transactions.remove(&entry.sender);
            }
        }
        if let Some(gas) = self
            .queue_transactions_gas_prices
            .get_mut(&Reverse(entry.gas_price))
        {
            *gas = gas.saturating_sub(entry.gas);
            if *gas == 0 {
                self.queue_transactions_gas_prices
                    .remove(&Reverse(entry.gas_price));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(sender: u8, nonce: u64, gas_price: u64, hash: u8) -> TransactionQueueEntry {
        TransactionQueueEntry {
            hash: H256::from([hash; 32]),
            sender: H160::from([sender; 20]),
            nonce: U256::from(nonce),
            gas_price: U256::from(gas_price),
            gas: 100,
            data_size: 4,
            rlp: vec![hash; 4],
            last_block_number: 1,
        }
    }

    #[test]
    fn orders_by_account_nonce_before_later_higher_price() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 5, 1), true).unwrap();
        queue.insert(entry(1, 2, 6, 2), true).unwrap();
        queue.insert(entry(2, 1, 4, 3), true).unwrap();

        assert_eq!(
            queue.ordered_hashes(3),
            vec![
                H256::from([1; 32]),
                H256::from([2; 32]),
                H256::from([3; 32])
            ]
        );
    }

    #[test]
    fn replaces_same_sender_nonce_with_higher_gas_price() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 1, 1), true).unwrap();
        let outcome = queue.insert(entry(1, 1, 10, 2), true).unwrap();

        assert_eq!(queue.size(), 1);
        assert_eq!(outcome.demoted_hashes, vec![H256::from([1; 32])]);
        assert_eq!(queue.ordered_hashes(10), vec![H256::from([2; 32])]);
        assert!(queue.contains(H256::from([1; 32])));
    }

    #[test]
    fn erase_plan_reports_removed_entry_metadata() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 1, 1), true).unwrap();

        let outcome = queue.erase_plan(H256::from([1; 32]));
        assert!(outcome.removed);
        assert!(outcome.removed_proposable);
        assert_eq!(outcome.removed_entry.unwrap().hash, H256::from([1; 32]));
        assert!(!queue.contains(H256::from([1; 32])));
    }

    #[test]
    fn demote_returns_explicit_status() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 10, 1), true).unwrap();

        let outcome = queue.demote(H256::from([1; 32]), 33);
        assert_eq!(outcome.status, TransactionQueueDemoteStatus::Demoted);
        assert_eq!(outcome.entry.unwrap().last_block_number, 33);
    }

    #[test]
    fn ordered_hashes_plan_reports_completion_state() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 5, 1), true).unwrap();
        queue.insert(entry(1, 2, 6, 2), true).unwrap();
        queue.insert(entry(2, 1, 4, 3), true).unwrap();

        let partial = queue.ordered_hashes_plan(2);
        assert_eq!(
            partial.hashes,
            vec![H256::from([1; 32]), H256::from([2; 32])]
        );
        assert!(!partial.complete);
        assert_eq!(partial.requested_count, 2);

        let all = queue.ordered_hashes_plan(10);
        assert!(all.complete);
        assert_eq!(
            all.hashes,
            vec![
                H256::from([1; 32]),
                H256::from([2; 32]),
                H256::from([3; 32])
            ]
        );
    }

    #[test]
    fn block_finalized_and_purge_account_plans_return_removed_lists() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 1, 1), false).unwrap();
        assert_eq!(
            queue.block_finalized_plan(12).removed_hashes,
            vec![H256::from([1; 32])]
        );

        queue.insert(entry(1, 1, 5, 1), true).unwrap();
        queue.insert(entry(1, 2, 6, 2), true).unwrap();
        queue.insert(entry(2, 1, 4, 3), true).unwrap();
        assert_eq!(
            queue
                .purge_account_plan(H160::from([1; 20]), U256::from(2u8))
                .removed_hashes,
            vec![H256::from([1; 32])]
        );
    }

    #[test]
    fn expires_non_proposable_transactions_after_finalized_block_limit() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 1, 1), false).unwrap();

        assert!(queue.block_finalized(11).is_empty());
        assert_eq!(queue.block_finalized(12), vec![H256::from([1; 32])]);
        assert!(!queue.contains(H256::from([1; 32])));
    }

    #[test]
    fn gas_price_threshold_uses_proposable_gas_only() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 5, 1), true).unwrap();
        queue.insert(entry(2, 1, 7, 2), true).unwrap();
        queue.insert(entry(3, 1, 100, 3), false).unwrap();

        assert_eq!(queue.min_gas_price_for_block_inclusion(150), U256::from(6));
        assert_eq!(queue.min_gas_price_for_block_inclusion(300), U256::one());
    }

    #[test]
    fn queued_transactions_return_canonical_payloads() {
        let mut queue = TransactionQueue::new(100);
        queue.insert(entry(1, 1, 5, 1), true).unwrap();
        queue.insert(entry(2, 1, 7, 2), true).unwrap();

        let ordered = queue.ordered_transactions(2);
        assert_eq!(ordered[0].hash, H256::from([2; 32]));
        assert_eq!(ordered[0].rlp, vec![2; 4]);
        assert_eq!(
            queue.transaction(H256::from([1; 32])).unwrap().rlp,
            vec![1; 4]
        );

        let groups = queue.all_transaction_groups();
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().any(|group| group[0].rlp == vec![1; 4]));
        assert!(groups.iter().any(|group| group[0].rlp == vec![2; 4]));
    }
}
