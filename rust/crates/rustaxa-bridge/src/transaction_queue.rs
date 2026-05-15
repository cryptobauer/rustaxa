//! CXX bridge adapters for Rust-owned transaction queue metadata and payload bytes.
//!
//! C++ shims pass fixed metadata and canonical RLP into this module. Rust returns hash and payload plans so C++ can
//! update known-transaction cache state and materialize legacy `Transaction` objects for API callers.

use crate::ffi::rustaxa_ffi::{
    TransactionQueueAddress, TransactionQueueConfig, TransactionQueueDemotePlan,
    TransactionQueueErasePlan, TransactionQueueHash, TransactionQueueHashGroup,
    TransactionQueueInsertInput, TransactionQueueInsertOutcome, TransactionQueueOrderedHashesPlan,
    TransactionQueuePurgePlan, TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::BridgeTransactionQueue;
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueDemoteOutcome, TransactionQueueDemoteStatus,
    TransactionQueueEntry, TransactionQueueEraseOutcome, TransactionQueueInsertStatus,
    TransactionQueueOrderedHashesPlan as ConsensusOrderedHashesPlan, TransactionQueuePurgeOutcome,
};

const TRANSACTION_QUEUE_STATUS_INSERTED: u8 = 0;
const TRANSACTION_QUEUE_STATUS_INSERTED_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_STATUS_KNOWN: u8 = 2;
const TRANSACTION_QUEUE_STATUS_OVERFLOW: u8 = 3;
const TRANSACTION_QUEUE_DEMOTE_STATUS_NOT_FOUND: u8 = 0;
const TRANSACTION_QUEUE_DEMOTE_STATUS_ALREADY_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_DEMOTE_STATUS_DEMOTED: u8 = 2;
const ZERO_HASH: H256 = H256::zero();

fn tx_queue_sender_for_entry(entry: &TransactionQueueEntry) -> [u8; 20] {
    entry.sender.0
}

fn tx_queue_nonce_for_entry(entry: &TransactionQueueEntry) -> [u8; 32] {
    entry.nonce.to_big_endian()
}

fn tx_queue_gas_price_for_entry(entry: &TransactionQueueEntry) -> [u8; 32] {
    entry.gas_price.to_big_endian()
}

fn tx_queue_empty_entry() -> TransactionQueueEntry {
    TransactionQueueEntry {
        hash: ZERO_HASH,
        sender: H160::zero(),
        nonce: U256::zero(),
        gas_price: U256::zero(),
        gas: 0,
        data_size: 0,
        rlp: Vec::new(),
        last_block_number: 0,
    }
}

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

fn tx_queue_erase_plan_from_consensus(
    outcome: TransactionQueueEraseOutcome,
) -> TransactionQueueErasePlan {
    let entry = outcome.removed_entry.unwrap_or_else(tx_queue_empty_entry);
    TransactionQueueErasePlan {
        removed: outcome.removed,
        removed_hash: entry.hash.0,
        removed_sender: tx_queue_sender_for_entry(&entry),
        removed_nonce: tx_queue_nonce_for_entry(&entry),
        removed_gas_price: tx_queue_gas_price_for_entry(&entry),
        removed_gas: entry.gas,
        removed_data_size: entry.data_size as usize,
        removed_last_block_number: entry.last_block_number,
        removed_proposable: outcome.removed_proposable,
    }
}

fn tx_queue_demote_plan_from_consensus(
    hash: H256,
    outcome: TransactionQueueDemoteOutcome,
) -> TransactionQueueDemotePlan {
    let hash_found = outcome.entry.is_some();
    let entry = outcome.entry.unwrap_or_else(tx_queue_empty_entry);
    TransactionQueueDemotePlan {
        status: match outcome.status {
            TransactionQueueDemoteStatus::NotFound => TRANSACTION_QUEUE_DEMOTE_STATUS_NOT_FOUND,
            TransactionQueueDemoteStatus::AlreadyNonProposable => {
                TRANSACTION_QUEUE_DEMOTE_STATUS_ALREADY_NON_PROPOSABLE
            }
            TransactionQueueDemoteStatus::Demoted => TRANSACTION_QUEUE_DEMOTE_STATUS_DEMOTED,
        },
        hash: hash.0,
        hash_found,
        sender: tx_queue_sender_for_entry(&entry),
        nonce: tx_queue_nonce_for_entry(&entry),
        gas_price: tx_queue_gas_price_for_entry(&entry),
        gas: entry.gas,
        data_size: entry.data_size as usize,
        last_block_number: entry.last_block_number,
        proposable_before: matches!(outcome.status, TransactionQueueDemoteStatus::Demoted),
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

fn tx_queue_ordered_hashes_plan_from_consensus(
    outcome: ConsensusOrderedHashesPlan,
) -> TransactionQueueOrderedHashesPlan {
    TransactionQueueOrderedHashesPlan {
        hashes: hashes_to_bridge(outcome.hashes),
        requested_count: outcome.requested_count,
        complete: outcome.complete,
    }
}

pub fn create_transaction_queue(config: TransactionQueueConfig) -> Box<BridgeTransactionQueue> {
    Box::new(BridgeTransactionQueue(TransactionQueue::new(
        config.max_size as u64,
    )))
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
        let outcome = self.0.insert(entry, input.proposable)?;
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
        self.transaction_queue_erase_plan(hash).removed
    }

    /// Removes a transaction and returns a C++ mirror update plan.
    pub fn transaction_queue_erase_plan(&mut self, hash: &[u8; 32]) -> TransactionQueueErasePlan {
        tx_queue_erase_plan_from_consensus(self.0.erase_plan(H256::from(*hash)))
    }

    /// Returns true when a transaction hash is known to proposer or non-proposer queue metadata.
    pub fn transaction_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.0.contains(H256::from(*hash))
    }

    /// Returns queued transaction bytes for one hash.
    pub fn transaction_queue_get_transaction(
        &self,
        hash: &[u8; 32],
    ) -> TransactionQueueStoredTransaction {
        tx_queue_stored_transaction_from_entry(self.0.transaction(H256::from(*hash)))
    }

    /// Returns the number of proposable transactions.
    pub fn transaction_queue_size(&self) -> usize {
        self.0.size() as usize
    }

    /// Returns proposer-ordered transaction hashes.
    pub fn transaction_queue_ordered_hashes(&self, count: u64) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(self.0.ordered_hashes(count))
    }

    /// Returns proposer-ordered transaction payloads.
    pub fn transaction_queue_ordered_transactions(
        &self,
        count: u64,
    ) -> Vec<TransactionQueueStoredTransaction> {
        self.0
            .ordered_transactions(count)
            .into_iter()
            .map(Some)
            .map(tx_queue_stored_transaction_from_entry)
            .collect()
    }

    /// Returns proposer-ordered hashes with plan metadata.
    pub fn transaction_queue_ordered_hashes_plan(
        &self,
        count: u64,
    ) -> TransactionQueueOrderedHashesPlan {
        tx_queue_ordered_hashes_plan_from_consensus(self.0.ordered_hashes_plan(count))
    }

    /// Returns proposable transaction hashes grouped by sender and ordered by nonce.
    pub fn transaction_queue_all_hash_groups(&self) -> Vec<TransactionQueueHashGroup> {
        self.0
            .all_transactions_grouped()
            .into_iter()
            .map(|hashes| TransactionQueueHashGroup {
                hashes: hashes_to_bridge(hashes),
            })
            .collect()
    }

    /// Returns proposable transaction payloads grouped by sender and ordered by nonce.
    pub fn transaction_queue_all_transaction_groups(
        &self,
    ) -> Vec<TransactionQueueTransactionGroup> {
        self.0
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
        hashes_to_bridge(self.0.block_finalized(block_number))
    }

    /// Expires old non-proposable transaction hashes and returns a mutation plan.
    pub fn transaction_queue_block_finalized_plan(
        &mut self,
        block_number: u64,
    ) -> TransactionQueuePurgePlan {
        tx_queue_purge_plan_from_consensus(self.0.block_finalized_plan(block_number))
    }

    /// Returns proposer accounts that C++ should query from FinalChain for purge.
    pub fn transaction_queue_proposable_accounts(&self) -> Vec<TransactionQueueAddress> {
        self.0
            .proposable_accounts()
            .into_iter()
            .map(|address| TransactionQueueAddress { address: address.0 })
            .collect()
    }

    /// Removes hashes for one account whose nonce is below the finalized account nonce.
    pub fn transaction_queue_purge_account(
        &mut self,
        sender: &[u8; 20],
        account_nonce: &[u8; 32],
    ) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(
            self.0
                .purge_account(H160::from(*sender), U256::from_big_endian(account_nonce)),
        )
    }

    /// Removes hashes for one account and returns a mutation plan.
    pub fn transaction_queue_purge_account_plan(
        &mut self,
        sender: &[u8; 20],
        account_nonce: &[u8; 32],
    ) -> TransactionQueuePurgePlan {
        tx_queue_purge_plan_from_consensus(
            self.0
                .purge_account_plan(H160::from(*sender), U256::from_big_endian(account_nonce)),
        )
    }

    /// Attempts to demote one queue hash to non-proposable metadata.
    pub fn transaction_queue_demote_to_non_proposable(
        &mut self,
        hash: &[u8; 32],
        last_block_number: u64,
    ) -> TransactionQueueDemotePlan {
        let parsed_hash = H256::from(*hash);
        tx_queue_demote_plan_from_consensus(
            parsed_hash,
            self.0.demote(parsed_hash, last_block_number),
        )
    }

    /// Returns true when non-proposable transactions reached their limit.
    pub fn transaction_queue_non_proposable_over_limit(&self) -> bool {
        self.0.non_proposable_transactions_over_the_limit()
    }

    /// Returns the minimum big-endian gas price needed for next-block inclusion.
    pub fn transaction_queue_min_gas_price_for_block_inclusion(&self, limit: u64) -> [u8; 32] {
        self.0
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
        assert_eq!(queue.transaction_queue_ordered_hashes(10)[0].hash, [2; 32]);
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
    fn bridge_erase_plan_reports_removed_entry_metadata() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 1, 1, 1)).unwrap();

        let erase_plan = queue.transaction_queue_erase_plan(&[1; 32]);
        assert!(erase_plan.removed);
        assert_eq!(erase_plan.removed_hash, [1; 32]);
        assert_eq!(erase_plan.removed_sender, [1; 20]);
    }

    #[test]
    fn bridge_ordered_hashes_plan_tracks_completion() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 1, 1, 1)).unwrap();
        queue.transaction_queue_insert(input(1, 2, 2, 2)).unwrap();
        queue.transaction_queue_insert(input(2, 1, 3, 3)).unwrap();

        let partial = queue.transaction_queue_ordered_hashes_plan(2);
        assert_eq!(partial.hashes.len(), 2);
        assert!(!partial.complete);

        let full = queue.transaction_queue_ordered_hashes_plan(10);
        assert!(full.complete);
        assert_eq!(full.requested_count, 10);
    }

    #[test]
    fn bridge_purge_account_plan_and_block_finalized_plan_expose_hashes() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 1, 1, 1)).unwrap();
        let plan = queue.transaction_queue_block_finalized_plan(11);
        assert_eq!(plan.removed_count, 0);

        let plan = queue.transaction_queue_purge_account_plan(&[1; 20], &be(2));
        assert_eq!(plan.removed_count, 1);
        assert_eq!(plan.removed_hashes[0].hash, [1; 32]);
    }

    #[test]
    fn bridge_demote_plan_reports_status() {
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 1, 1, 1)).unwrap();

        let demote = queue.transaction_queue_demote_to_non_proposable(&[1; 32], 9);
        assert_eq!(demote.status, TRANSACTION_QUEUE_DEMOTE_STATUS_DEMOTED);
        assert_eq!(demote.last_block_number, 9);
        assert!(demote.hash_found);

        let not_found = queue.transaction_queue_demote_to_non_proposable(&[2; 32], 5);
        assert_eq!(not_found.status, TRANSACTION_QUEUE_DEMOTE_STATUS_NOT_FOUND);
    }
}
