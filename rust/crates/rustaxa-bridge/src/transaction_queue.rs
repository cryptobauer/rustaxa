//! CXX bridge adapters for Rust-owned transaction queue metadata and payload bytes.
//!
//! C++ shims pass fixed metadata and canonical RLP into this module. Rust returns hash and payload plans so C++ can
//! update known-transaction cache state and materialize legacy `Transaction` objects for API callers. Production purge
//! routing sources account nonce facts from the C++-supplied bridge API so queue pruning is minimal at the boundary.

use crate::ffi::rustaxa_ffi::{
    TransactionQueueConfig, TransactionQueueHash, TransactionQueueInsertInput,
    TransactionQueueInsertOutcome, TransactionQueueProposableAccountFact,
    TransactionQueuePurgePlan, TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::{
    rustaxa_ffi::TransactionQueueAccountNonceFact as BridgeTransactionQueueAccountNonceFact,
    BridgeTransactionQueue,
};
use anyhow::Result;
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueEntry,
    TransactionQueueInsertStatus, TransactionQueuePurgeOutcome,
};
use std::time::{Duration, Instant};

const TRANSACTION_QUEUE_STATUS_INSERTED: u8 = 0;
const TRANSACTION_QUEUE_STATUS_INSERTED_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_STATUS_KNOWN: u8 = 2;
const TRANSACTION_QUEUE_STATUS_OVERFLOW: u8 = 3;
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);
const ZERO_HASH: H256 = H256::zero();

fn tx_queue_stored_transaction_from_entry(
    entry: Option<TransactionQueueEntry>,
) -> TransactionQueueStoredTransaction {
    if let Some(entry) = entry {
        TransactionQueueStoredTransaction {
            found: true,
            hash: entry.hash.0,
            tx_rlp: entry.rlp,
        }
    } else {
        TransactionQueueStoredTransaction {
            found: false,
            hash: ZERO_HASH.0,
            tx_rlp: Vec::new(),
        }
    }
}

fn tx_queue_purge_plan_from_consensus(
    outcome: TransactionQueuePurgeOutcome,
) -> TransactionQueuePurgePlan {
    TransactionQueuePurgePlan {
        removed_count: outcome.removed_hashes.len(),
        removed_hashes: hashes_to_bridge(outcome.removed_hashes),
    }
}

fn tx_queue_account_nonce_facts_from_bridge(
    account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
) -> Vec<TransactionQueueAccountNonceFact> {
    account_nonce_facts
        .into_iter()
        .map(|fact| TransactionQueueAccountNonceFact {
            sender: H160::from(fact.sender),
            account_found: fact.account_found,
            account_nonce: U256::from_big_endian(&fact.account_nonce),
        })
        .collect()
}

pub fn create_transaction_queue(config: TransactionQueueConfig) -> Box<BridgeTransactionQueue> {
    Box::new(BridgeTransactionQueue {
        queue: TransactionQueue::new(config.max_size as u64),
        last_drop_observed: None,
    })
}

impl BridgeTransactionQueue {
    /// Inserts transaction metadata into the Rust queue and returns C++ mirror updates.
    pub fn transaction_queue_insert(
        &mut self,
        input: TransactionQueueInsertInput,
    ) -> anyhow::Result<TransactionQueueInsertOutcome> {
        let entry = TransactionQueueEntry {
            hash: H256::from(input.hash),
            sender: H160::from(input.sender),
            nonce: U256::from_big_endian(&input.nonce),
            gas_price: U256::from_big_endian(&input.gas_price),
            gas: input.gas,
            data_size: input.data_size as u64,
            rlp: input.tx_rlp,
            last_block_number: input.last_block_number,
        };
        let outcome = self.queue.insert(entry, input.proposable)?;
        if matches!(outcome.status, TransactionQueueInsertStatus::Overflow)
            || !outcome.overflow_removed_hashes.is_empty()
        {
            self.last_drop_observed = Some(Instant::now());
        }
        Ok(TransactionQueueInsertOutcome {
            status: status_to_bridge(outcome.status),
            inserted_hash_found: outcome.inserted_hash.is_some(),
            inserted_hash: outcome.inserted_hash.unwrap_or_default().0,
            demoted_hashes: hashes_to_bridge(outcome.demoted_hashes),
            overflow_removed_hashes: hashes_to_bridge(outcome.overflow_removed_hashes),
        })
    }

    /// Removes a transaction from any Rust queue index.
    pub fn transaction_queue_erase(&mut self, hash: &[u8; 32]) -> bool {
        self.queue.erase(H256::from(*hash))
    }

    /// Returns true when a transaction hash is known to proposer or non-proposer queue metadata.
    pub fn transaction_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.queue.contains(H256::from(*hash))
    }

    /// Marks a transaction hash in the Rust-owned known-admission cache.
    pub fn transaction_queue_mark_transaction_known(&mut self, hash: &[u8; 32]) -> bool {
        self.queue.mark_transaction_known(H256::from(*hash))
    }

    /// Returns true when the Rust-owned known-admission cache contains a hash.
    pub fn transaction_queue_is_transaction_known(&self, hash: &[u8; 32]) -> bool {
        self.queue.is_transaction_known(H256::from(*hash))
    }

    /// Returns true while the overflow/drop observation window remains active.
    pub fn transaction_queue_transactions_dropped(&self) -> bool {
        self.last_drop_observed
            .is_some_and(|observed| observed.elapsed() < TRANSACTION_QUEUE_DROP_WINDOW)
    }

    /// Returns queued transaction bytes for one hash.
    pub fn transaction_queue_get_transaction(
        &self,
        hash: &[u8; 32],
    ) -> TransactionQueueStoredTransaction {
        tx_queue_stored_transaction_from_entry(self.queue.transaction(H256::from(*hash)))
    }

    /// Returns the number of proposable transactions.
    pub fn transaction_queue_size(&self) -> usize {
        self.queue.size() as usize
    }

    /// Returns proposer-ordered transaction payloads.
    pub fn transaction_queue_ordered_transactions(
        &self,
        count: u64,
    ) -> Vec<TransactionQueueStoredTransaction> {
        self.queue
            .ordered_transactions(count)
            .into_iter()
            .map(Some)
            .map(tx_queue_stored_transaction_from_entry)
            .collect()
    }

    /// Returns proposable transaction payloads grouped by sender and ordered by nonce.
    pub fn transaction_queue_all_transaction_groups(
        &self,
    ) -> Vec<TransactionQueueTransactionGroup> {
        self.queue
            .all_transaction_groups()
            .into_iter()
            .map(|transactions| TransactionQueueTransactionGroup {
                transactions: transactions
                    .into_iter()
                    .map(Some)
                    .map(tx_queue_stored_transaction_from_entry)
                    .collect(),
            })
            .collect()
    }

    /// Expires old non-proposable transaction hashes after finalization advances.
    pub fn transaction_queue_block_finalized(
        &mut self,
        block_number: u64,
    ) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(self.queue.block_finalized(block_number))
    }

    /// Removes proposer transactions whose nonce is below supplied account facts.
    ///
    /// Inputs:
    /// - `account_nonce_facts`: caller-supplied account facts for each
    ///   currently proposable queue sender.
    ///
    /// Output:
    /// - a deterministic purge plan containing all removed transaction hashes.
    pub fn transaction_queue_purge_with_account_nonce_facts(
        &mut self,
        account_nonce_facts: Vec<BridgeTransactionQueueAccountNonceFact>,
    ) -> Result<TransactionQueuePurgePlan> {
        Ok(tx_queue_purge_plan_from_consensus(
            self.queue
                .purge_accounts_plan(&tx_queue_account_nonce_facts_from_bridge(
                    account_nonce_facts,
                )),
        ))
    }

    /// Returns proposable senders currently queued, as raw addresses.
    pub fn transaction_queue_proposable_accounts(
        &self,
    ) -> Vec<TransactionQueueProposableAccountFact> {
        self.queue
            .proposable_accounts()
            .into_iter()
            .map(|sender| TransactionQueueProposableAccountFact { sender: sender.0 })
            .collect()
    }

    /// Returns true when non-proposable transactions reached their limit.
    pub fn transaction_queue_non_proposable_over_limit(&self) -> bool {
        self.queue.non_proposable_transactions_over_the_limit()
    }

    /// Returns the minimum big-endian gas price needed for next-block inclusion.
    pub fn transaction_queue_min_gas_price_for_block_inclusion(&self, limit: u64) -> [u8; 32] {
        self.queue
            .min_gas_price_for_block_inclusion(limit)
            .to_big_endian()
    }
}

fn hashes_to_bridge(hashes: Vec<H256>) -> Vec<TransactionQueueHash> {
    hashes
        .into_iter()
        .map(|hash| TransactionQueueHash { hash: hash.0 })
        .collect()
}

fn status_to_bridge(status: TransactionQueueInsertStatus) -> u8 {
    match status {
        TransactionQueueInsertStatus::Inserted => TRANSACTION_QUEUE_STATUS_INSERTED,
        TransactionQueueInsertStatus::InsertedNonProposable => {
            TRANSACTION_QUEUE_STATUS_INSERTED_NON_PROPOSABLE
        }
        TransactionQueueInsertStatus::Known => TRANSACTION_QUEUE_STATUS_KNOWN,
        TransactionQueueInsertStatus::Overflow => TRANSACTION_QUEUE_STATUS_OVERFLOW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be(value: u64) -> [u8; 32] {
        U256::from(value).to_big_endian()
    }

    fn input(sender: u8, nonce: u64, gas_price: u64, hash: u8) -> TransactionQueueInsertInput {
        TransactionQueueInsertInput {
            hash: [hash; 32],
            sender: [sender; 20],
            nonce: be(nonce),
            gas_price: be(gas_price),
            gas: 100,
            data_size: 4,
            tx_rlp: vec![hash; 4],
            proposable: true,
            last_block_number: 1,
        }
    }

    #[test]
    fn bridge_orders_hashes_and_reports_replacement_updates() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });

        assert_eq!(
            queue
                .transaction_queue_insert(input(1, 1, 1, 1))
                .unwrap()
                .status,
            TRANSACTION_QUEUE_STATUS_INSERTED
        );
        let replacement = queue.transaction_queue_insert(input(1, 1, 2, 2)).unwrap();
        assert_eq!(replacement.demoted_hashes[0].hash, [1; 32]);

        assert_eq!(queue.transaction_queue_size(), 1);
        assert_eq!(
            queue.transaction_queue_ordered_transactions(10)[0].hash,
            [2; 32]
        );
        assert!(queue.transaction_queue_contains(&[1; 32]));
    }

    #[test]
    fn bridge_returns_rust_owned_queue_payloads() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 1, 1, 1)).unwrap();
        queue.transaction_queue_insert(input(2, 1, 2, 2)).unwrap();

        let stored = queue.transaction_queue_get_transaction(&[1; 32]);
        assert!(stored.found);
        assert_eq!(stored.tx_rlp, vec![1; 4]);

        let ordered = queue.transaction_queue_ordered_transactions(2);
        assert_eq!(ordered[0].hash, [2; 32]);
        assert_eq!(ordered[0].tx_rlp, vec![2; 4]);

        let groups = queue.transaction_queue_all_transaction_groups();
        assert_eq!(groups.len(), 2);
        assert!(groups
            .iter()
            .any(|group| group.transactions[0].tx_rlp == vec![1; 4]));
        assert!(groups
            .iter()
            .any(|group| group.transactions[0].tx_rlp == vec![2; 4]));
    }

    #[test]
    fn bridge_queue_purge_with_account_nonce_facts_in_rust() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 0, 5, 1)).unwrap();
        queue.transaction_queue_insert(input(2, 0, 6, 2)).unwrap();

        let plan = queue
            .transaction_queue_purge_with_account_nonce_facts(vec![
                BridgeTransactionQueueAccountNonceFact {
                    sender: [1; 20],
                    account_found: true,
                    account_nonce: U256::from(0u8).to_big_endian(),
                },
                BridgeTransactionQueueAccountNonceFact {
                    sender: [2; 20],
                    account_found: true,
                    account_nonce: U256::from(0u8).to_big_endian(),
                },
            ])
            .expect("Queue purge should execute with caller-provided account facts");

        assert_eq!(plan.removed_count, 0);
        assert!(queue.transaction_queue_contains(&[1; 32]));
        assert!(queue.transaction_queue_contains(&[2; 32]));
    }

    #[test]
    fn bridge_queue_proposable_accounts_tracks_senders() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 0, 5, 1)).unwrap();
        queue.transaction_queue_insert(input(2, 1, 6, 2)).unwrap();

        let proposable = queue.transaction_queue_proposable_accounts();
        assert_eq!(proposable.len(), 2);
        assert!(proposable.iter().any(|fact| fact.sender == [1; 20]));
        assert!(proposable.iter().any(|fact| fact.sender == [2; 20]));
    }

    #[test]
    fn bridge_routes_known_cache_and_drop_window_to_rust_queue() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 5 });

        for hash in 1..=10 {
            assert!(queue.transaction_queue_mark_transaction_known(&[hash; 32]));
        }
        assert!(!queue.transaction_queue_mark_transaction_known(&[1; 32]));
        assert!(queue.transaction_queue_is_transaction_known(&[1; 32]));

        assert!(queue.transaction_queue_mark_transaction_known(&[11; 32]));
        assert!(!queue.transaction_queue_is_transaction_known(&[1; 32]));
        assert!(queue.transaction_queue_is_transaction_known(&[11; 32]));

        assert!(!queue.transaction_queue_transactions_dropped());
        for hash in 20..=25 {
            queue
                .transaction_queue_insert(input(hash, 1, 1, hash))
                .unwrap();
        }
        assert!(queue.transaction_queue_transactions_dropped());
    }
}
