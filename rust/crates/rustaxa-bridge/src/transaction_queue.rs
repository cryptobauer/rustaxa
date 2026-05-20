//! CXX bridge adapters for Rust-owned transaction queue metadata and payload bytes.
//!
//! C++ shims pass fixed metadata and canonical RLP into this module. Rust returns hash and payload plans so C++ can
//! update known-transaction cache state and materialize legacy `Transaction` objects for API callers. Production purge
//! routing sources account nonce facts from the Rust FinalChain handle so C++ does not materialize account facts.

use crate::ffi::rustaxa_ffi::{
    TransactionQueueConfig, TransactionQueueErasePlan, TransactionQueueHash,
    TransactionQueueHashGroup, TransactionQueueInsertInput, TransactionQueueInsertOutcome,
    TransactionQueueOrderedHashesPlan, TransactionQueuePurgePlan,
    TransactionQueueStoredTransaction, TransactionQueueTransactionGroup,
};
use crate::ffi::{BridgeFinalChain, BridgeTransactionQueue};
use anyhow::{Context, Result};
use ethereum_types::{H160, H256, U256};
use rustaxa_consensus::transaction_queue::{
    TransactionQueue, TransactionQueueAccountNonceFact, TransactionQueueEntry,
    TransactionQueueEraseOutcome, TransactionQueueInsertStatus,
    TransactionQueueOrderedHashesPlan as ConsensusOrderedHashesPlan, TransactionQueuePurgeOutcome,
};
use std::time::{Duration, Instant};

const TRANSACTION_QUEUE_STATUS_INSERTED: u8 = 0;
const TRANSACTION_QUEUE_STATUS_INSERTED_NON_PROPOSABLE: u8 = 1;
const TRANSACTION_QUEUE_STATUS_KNOWN: u8 = 2;
const TRANSACTION_QUEUE_STATUS_OVERFLOW: u8 = 3;
const TRANSACTION_QUEUE_DROP_WINDOW: Duration = Duration::from_secs(600);
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

fn tx_queue_purge_plan_from_consensus(
    outcome: TransactionQueuePurgeOutcome,
) -> TransactionQueuePurgePlan {
    TransactionQueuePurgePlan {
        removed_count: outcome.removed_hashes.len(),
        removed_hashes: hashes_to_bridge(outcome.removed_hashes),
    }
}

fn tx_queue_account_nonce_facts_from_final_chain(
    final_chain: &BridgeFinalChain,
    proposable_accounts: Vec<H160>,
) -> Result<Vec<TransactionQueueAccountNonceFact>> {
    proposable_accounts
        .into_iter()
        .map(|sender| {
            let lookup = final_chain
                .get_account(&sender.0)
                .context("TRANSACTION_QUEUE_PURGE_ACCOUNT_LOOKUP_FAILED")?;
            Ok(TransactionQueueAccountNonceFact {
                sender,
                account_found: lookup.found,
                account_nonce: U256::from(lookup.nonce),
            })
        })
        .collect()
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
        self.transaction_queue_erase_plan(hash).removed
    }

    /// Removes a transaction and returns a C++ mirror update plan.
    pub fn transaction_queue_erase_plan(&mut self, hash: &[u8; 32]) -> TransactionQueueErasePlan {
        tx_queue_erase_plan_from_consensus(self.queue.erase_plan(H256::from(*hash)))
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

    /// Returns proposer-ordered transaction hashes.
    pub fn transaction_queue_ordered_hashes(&self, count: u64) -> Vec<TransactionQueueHash> {
        hashes_to_bridge(self.queue.ordered_hashes(count))
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

    /// Returns proposer-ordered hashes with plan metadata.
    pub fn transaction_queue_ordered_hashes_plan(
        &self,
        count: u64,
    ) -> TransactionQueueOrderedHashesPlan {
        tx_queue_ordered_hashes_plan_from_consensus(self.queue.ordered_hashes_plan(count))
    }

    /// Returns proposable transaction hashes grouped by sender and ordered by nonce.
    pub fn transaction_queue_all_hash_groups(&self) -> Vec<TransactionQueueHashGroup> {
        self.queue
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

    /// Expires old non-proposable transaction hashes and returns a mutation plan.
    pub fn transaction_queue_block_finalized_plan(
        &mut self,
        block_number: u64,
    ) -> TransactionQueuePurgePlan {
        tx_queue_purge_plan_from_consensus(self.queue.block_finalized_plan(block_number))
    }

    /// Removes proposer transactions whose nonce is below the latest FinalChain account nonce.
    ///
    /// Inputs:
    /// - `final_chain`: Rust FinalChain runtime used to read the latest account
    ///   state for each currently proposable queue sender.
    ///
    /// Output:
    /// - a deterministic purge plan containing all removed transaction hashes.
    ///
    /// Behavior:
    /// - collects proposable senders from the Rust queue
    /// - reads each sender account from Rust FinalChain
    /// - treats missing accounts as nonce zero, matching the consensus queue
    ///   planner's account-fact semantics
    /// - mutates only Rust-owned queue state and does not materialize C++
    ///   account facts or transaction objects
    pub fn transaction_queue_purge_with_final_chain(
        &mut self,
        final_chain: &BridgeFinalChain,
    ) -> Result<TransactionQueuePurgePlan> {
        let facts = tx_queue_account_nonce_facts_from_final_chain(
            final_chain,
            self.queue.proposable_accounts(),
        )?;
        Ok(tx_queue_purge_plan_from_consensus(
            self.queue.purge_accounts_plan(&facts),
        ))
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
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let now_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after UNIX_EPOCH")
            .as_nanos();
        let process_id = std::process::id();
        std::env::temp_dir().join(format!("{prefix}_{process_id}_{now_ns}"))
    }

    fn final_chain_with_genesis_account(
        storage_path: &str,
        address: [u8; 20],
    ) -> Box<BridgeFinalChain> {
        let storage =
            crate::storage::create_storage(storage_path).expect("storage should initialize");
        crate::final_chain::create_final_chain(
            &storage,
            1_000_000,
            1,
            vec![crate::ffi::rustaxa_ffi::GenesisAccount {
                address,
                balance: vec![1],
            }],
            Vec::new(),
            crate::ffi::rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: vec![1],
                vote_eligibility_balance_step: vec![1],
                validator_maximum_stake: vec![1],
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("final chain should initialize")
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
    fn bridge_purge_with_final_chain_sources_account_facts_in_rust() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_tx_queue_purge_fc");
        let final_chain = final_chain_with_genesis_account(
            temp_dir.to_str().expect("temp path should be valid UTF-8"),
            [1; 20],
        );
        let mut queue = create_transaction_queue(TransactionQueueConfig { max_size: 100 });
        queue.transaction_queue_insert(input(1, 0, 5, 1)).unwrap();
        queue.transaction_queue_insert(input(2, 0, 6, 2)).unwrap();

        let plan = queue
            .transaction_queue_purge_with_final_chain(&final_chain)
            .expect("FinalChain-backed purge should succeed");

        assert_eq!(plan.removed_count, 0);
        assert!(queue.transaction_queue_contains(&[1; 32]));
        assert!(queue.transaction_queue_contains(&[2; 32]));

        let _ = fs::remove_dir_all(temp_dir);
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
