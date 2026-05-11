//! CXX bridge adapters for Rust-owned transaction queue metadata.
//!
//! C++ shims keep live `Transaction` objects and pass only fixed metadata into this module. Returned hash plans tell C++
//! which local pointers and known-transaction cache entries to update.

use crate::ffi::rustaxa_ffi::{
    TransactionQueueAddress, TransactionQueueConfig, TransactionQueueHash,
    TransactionQueueHashGroup, TransactionQueueInsertInput, TransactionQueueInsertOutcome,
};
use crate::ffi::BridgeTransactionQueue;
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueEntry, TransactionQueueInsertStatus,
};

const TRANSACTION_QUEUE_STATUS_INSERTED: u8 = 0;
const TRANSACTION_QUEUE_STATUS_INSERTED_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_STATUS_KNOWN: u8 = 2;
const TRANSACTION_QUEUE_STATUS_OVERFLOW: u8 = 3;

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
        self.0.erase(H256::from(*hash))
    }

    /// Returns true when a transaction hash is known to proposer or non-proposer queue metadata.
    pub fn transaction_queue_contains(&self, hash: &[u8; 32]) -> bool {
        self.0.contains(H256::from(*hash))
    }

    /// Returns the number of proposable transactions.
    pub fn transaction_queue_size(&self) -> usize {
        self.0.size() as usize
    }

    /// Returns proposer-ordered transaction hashes.
    pub fn transaction_queue_ordered_hashes(&self, count: u64) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(self.0.ordered_hashes(count))
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

    /// Expires old non-proposable transaction hashes after finalization advances.
    pub fn transaction_queue_block_finalized(
        &mut self,
        block_number: u64,
    ) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(self.0.block_finalized(block_number))
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
}
