//! Period-data sync queue metadata for PBFT rewrite mode.
//!
//! This module models the deterministic queue contract used while syncing PBFT
//! period data from peers. It deliberately owns only compact queue metadata:
//! entry ids, periods, block hashes, validation facts, effective processable
//! size, and pop/cleanup decisions. The C++ shim keeps ownership of live
//! `PeriodData`, `PbftVote`, and peer `NodeID` objects until those model types
//! are ported.

use anyhow::{Result, anyhow};
use ethereum_types::H256;
use std::collections::VecDeque;

/// Metadata for one queued period-data payload.
///
/// Inputs/outputs:
/// - `entry_id`: bridge-local id for the C++ payload object.
/// - `period`: PBFT period carried by that payload.
/// - `block_hash`: PBFT block hash carried by that payload.
/// - `prev_block_hash`: previous PBFT block hash carried by that payload.
/// - `pivot_hash`: pivot DAG block hash carried by that payload.
/// - `final_chain_hash`: final-chain hash carried by that payload's PBFT
///   block.
/// - `reward_vote_hashes`: reward-vote hashes referenced by that payload's
///   PBFT block.
/// - `pillar_vote_rlps`: canonical pillar-vote RLP payloads carried by the
///   synced period-data payload for Rust sync validation.
/// - transaction hash lists: compact sync validation facts carried by the
///   payload.
/// - previous-cert-vote flags: compact vote sidecar facts used by sync
///   admission planning.
/// - `pillar_votes_present`: compact pillar sidecar presence used by sync
///   admission planning.
/// - extra-data flags: compact PBFT block extra-data facts used by sync
///   admission planning.
///
/// Invariants:
/// - `entry_id` is unique within one queue lifetime.
/// - entries are stored in insertion order and accepted only by PBFT sync
///   period rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueueEntryRef {
    pub entry_id: u64,
    pub period: u64,
    pub block_hash: H256,
    pub prev_block_hash: H256,
    pub pivot_hash: H256,
    pub final_chain_hash: H256,
    pub reward_vote_hashes: Vec<H256>,
    pub pillar_vote_rlps: Vec<Vec<u8>>,
    pub dag_transaction_hashes: Vec<H256>,
    pub period_data_transaction_hashes: Vec<H256>,
    pub period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
    pub previous_cert_votes_present: bool,
    pub previous_cert_first_vote_has_weight: bool,
    pub pillar_votes_present: bool,
    pub extra_data_present: bool,
    pub extra_data_pillar_block_hash_present: bool,
}

/// Compact transaction identity retained for synced period-data transactions.
///
/// Inputs/outputs:
/// - `input_index`: original transaction-list index in the period data payload.
/// - `hash`: canonical transaction hash.
/// - `transaction_nonce`: declared transaction nonce as a 32-byte big-endian
///   U256 for CXX compatibility.
/// - `sender`: recovered transaction sender.
///
/// Invariants:
/// - Identities are ordered exactly like the period-data transaction list.
/// - Sender recovery and hash validation happen before this fact enters the
///   queue; malformed payloads must be rejected by the bridge/shim caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueueTransactionIdentity {
    pub input_index: u64,
    pub hash: H256,
    pub transaction_nonce: [u8; 32],
    pub sender: [u8; 20],
}

/// Result of attempting to enqueue one period-data payload.
///
/// Inputs/outputs:
/// - `accepted`: true when the entry was appended to Rust queue metadata.
/// - `clear_existing`: true when C++ must drop old live payloads before adding
///   the accepted entry because the PBFT chain moved beyond queued state.
/// - period fields expose the legacy admission calculation for diagnostics and
///   bridge tests.
/// - `effective_size`: processable queue size after the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodDataQueuePushOutcome {
    pub accepted: bool,
    pub clear_existing: bool,
    pub expected_next_period: u64,
    pub actual_period: u64,
    pub current_period: u64,
    pub effective_size: usize,
}

/// Plan returned after popping queue metadata.
///
/// Inputs/outputs:
/// - `entry_id`: C++ payload id to move out of the live payload deque.
/// - `use_last_block_cert_votes`: true when C++ must return the side-car cert
///   votes passed with the last queued block; false means cert votes come from
///   the next queued `PeriodData.previous_block_cert_votes`.
/// - `next_entry_id`: id of the next queued payload when
///   `use_last_block_cert_votes` is false.
/// - `current_period` and `effective_size` describe queue state after pop.
/// - `entry_period`, `block_hash`, `prev_block_hash`, `pivot_hash`, and
///   `final_chain_hash` are the compact PBFT block facts for the popped
///   payload.
/// - `reward_vote_hashes` are compact reward-vote references from the popped
///   PBFT block.
/// - `pillar_vote_rlps` are canonical pillar-vote payload bytes from the
///   popped period-data payload.
/// - transaction hash lists are compact sync validation facts for the popped
///   payload.
/// - transaction identities are compact finalized-status facts for the popped
///   payload's transaction list.
/// - previous-cert-vote flags are compact vote sidecar facts for the popped
///   payload.
/// - `pillar_votes_present` is the compact pillar sidecar presence fact for
///   the popped payload.
/// - extra-data flags are compact PBFT block extra-data facts for the popped
///   payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueuePopPlan {
    pub entry_id: u64,
    pub entry_period: u64,
    pub block_hash: H256,
    pub prev_block_hash: H256,
    pub pivot_hash: H256,
    pub final_chain_hash: H256,
    pub reward_vote_hashes: Vec<H256>,
    pub pillar_vote_rlps: Vec<Vec<u8>>,
    pub dag_transaction_hashes: Vec<H256>,
    pub period_data_transaction_hashes: Vec<H256>,
    pub period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
    pub previous_cert_votes_present: bool,
    pub previous_cert_first_vote_has_weight: bool,
    pub pillar_votes_present: bool,
    pub extra_data_present: bool,
    pub extra_data_pillar_block_hash_present: bool,
    pub use_last_block_cert_votes: bool,
    pub next_entry_id: u64,
    pub current_period: u64,
    pub effective_size: usize,
}

/// Rust-owned PBFT period-data queue metadata.
///
/// Behavior preserved from C++:
/// - push accepts `max(period_, max_pbft_size) + 1`
/// - an empty queue also accepts `max_pbft_size + 2`
/// - chain progress past queued state clears old queued entries on accepted push
/// - `size()` reports only entries with available cert votes, not raw length
/// - popping the last entry resets the tracked period to zero
/// - stale cleanup removes front entries but does not otherwise mutate period
///   or last-cert-vote availability
#[derive(Debug, Default, Clone)]
pub struct PeriodDataQueue {
    entries: VecDeque<PeriodDataQueueEntryRef>,
    period: u64,
    last_block_cert_votes_available: bool,
}

impl PeriodDataQueue {
    /// Creates an empty period-data queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the latest accepted queue period, or zero when reset.
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Returns the PBFT syncing period visible to network status.
    ///
    /// Inputs:
    /// - `pbft_chain_size`: local PBFT chain size supplied by the PBFT chain
    ///   compatibility executor.
    ///
    /// Outputs:
    /// - The maximum of the Rust-owned queue period and the supplied PBFT chain
    ///   size, preserving the legacy status-period calculation without making
    ///   the PBFT manager read queue metadata as an authoritative mirror.
    pub fn syncing_period(&self, pbft_chain_size: u64) -> u64 {
        self.period.max(pbft_chain_size)
    }

    /// Returns the PBFT block hash to use as the next chain-link fact.
    ///
    /// Inputs:
    /// - `current_period`: current PBFT period supplied by the PBFT chain
    ///   compatibility executor. PBFT-chain period remains authoritative at
    ///   this boundary.
    /// - `chain_last_hash`: last PBFT-chain block hash supplied by the PBFT
    ///   chain compatibility executor.
    ///
    /// Outputs:
    /// - The last queued PBFT block hash when Rust queue metadata proves the
    ///   queued period is not stale for `current_period`.
    /// - Otherwise `chain_last_hash`.
    pub fn last_block_hash_or_chain(&self, current_period: u64, chain_last_hash: H256) -> H256 {
        self.entries
            .back()
            .filter(|entry| entry.period >= current_period)
            .map(|entry| entry.block_hash)
            .unwrap_or(chain_last_hash)
    }

    /// Returns processable queue size under legacy cert-vote visibility rules.
    ///
    /// The tail entry is hidden when no side-car cert votes are available,
    /// because its cert votes may arrive only in a subsequent queued block.
    pub fn size(&self) -> usize {
        if self.last_block_cert_votes_available || self.entries.is_empty() {
            self.entries.len()
        } else {
            self.entries.len().saturating_sub(1)
        }
    }

    /// Returns true when no period-data entries are queued.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears all queue metadata and resets period state.
    pub fn clear(&mut self) {
        self.period = 0;
        self.entries.clear();
        self.last_block_cert_votes_available = false;
    }

    /// Attempts to enqueue one period-data metadata entry.
    ///
    /// Inputs:
    /// - `entry_id`: C++ payload id for the live `PeriodData` object.
    /// - `entry_period`: PBFT period of the payload block.
    /// - `block_hash`: PBFT block hash of the payload block.
    /// - `prev_block_hash`: previous PBFT block hash of the payload block.
    /// - `pivot_hash`: pivot DAG block hash of the payload block.
    /// - `final_chain_hash`: final-chain hash of the payload block.
    /// - `reward_vote_hashes`: reward-vote hashes referenced by the payload
    ///   PBFT block.
    /// - `pillar_vote_rlps`: canonical pillar-vote payload bytes carried by
    ///   the payload.
    /// - `dag_transaction_hashes`: transaction hashes referenced by finalized
    ///   DAG blocks in the payload.
    /// - `period_data_transaction_hashes`: transaction hashes supplied in the
    ///   payload transaction list.
    /// - `period_data_transaction_identities`: transaction hashes, nonces, and
    ///   senders supplied in payload transaction-list order.
    /// - `previous_cert_votes_present`: whether the payload carried previous
    ///   block cert-vote sidecars.
    /// - `previous_cert_first_vote_has_weight`: whether the first previous
    ///   cert-vote sidecar already carried a calculated weight.
    /// - `pillar_votes_present`: whether the payload carried pillar-vote
    ///   sidecar data.
    /// - `extra_data_present`: whether the PBFT block carried extra data.
    /// - `extra_data_pillar_block_hash_present`: whether that extra data
    ///   carried a pillar block hash.
    /// - `max_pbft_size`: current local PBFT chain size.
    /// - `current_block_cert_votes_count`: number of cert votes passed for the
    ///   pushed block; only the count is needed for size eligibility.
    ///
    /// Returns a push outcome. Overflow in legacy period arithmetic is reported
    /// as an error rather than wrapping.
    pub fn push(
        &mut self,
        entry_id: u64,
        entry_period: u64,
        block_hash: H256,
        prev_block_hash: H256,
        pivot_hash: H256,
        final_chain_hash: H256,
        reward_vote_hashes: Vec<H256>,
        pillar_vote_rlps: Vec<Vec<u8>>,
        dag_transaction_hashes: Vec<H256>,
        period_data_transaction_hashes: Vec<H256>,
        period_data_transaction_identities: Vec<PeriodDataQueueTransactionIdentity>,
        previous_cert_votes_present: bool,
        previous_cert_first_vote_has_weight: bool,
        pillar_votes_present: bool,
        extra_data_present: bool,
        extra_data_pillar_block_hash_present: bool,
        max_pbft_size: u64,
        current_block_cert_votes_count: usize,
    ) -> Result<PeriodDataQueuePushOutcome> {
        let expected_next_period = std::cmp::max(self.period, max_pbft_size)
            .checked_add(1)
            .ok_or_else(|| anyhow!("period data queue next-period calculation overflowed"))?;
        let empty_queue_backfill_period = max_pbft_size.checked_add(2);

        let queue_empty_backfill =
            self.entries.is_empty() && Some(entry_period) == empty_queue_backfill_period;
        if entry_period != expected_next_period && !queue_empty_backfill {
            return Ok(PeriodDataQueuePushOutcome {
                accepted: false,
                clear_existing: false,
                expected_next_period,
                actual_period: entry_period,
                current_period: self.period,
                effective_size: self.size(),
            });
        }

        let clear_existing = max_pbft_size > self.period && !self.entries.is_empty();
        if clear_existing {
            self.entries.clear();
        }

        self.period = entry_period;
        self.entries.push_back(PeriodDataQueueEntryRef {
            entry_id,
            period: entry_period,
            block_hash,
            prev_block_hash,
            pivot_hash,
            final_chain_hash,
            reward_vote_hashes,
            pillar_vote_rlps,
            dag_transaction_hashes,
            period_data_transaction_hashes,
            period_data_transaction_identities,
            previous_cert_votes_present,
            previous_cert_first_vote_has_weight,
            pillar_votes_present,
            extra_data_present,
            extra_data_pillar_block_hash_present,
        });
        self.last_block_cert_votes_available = current_block_cert_votes_count > 0;

        Ok(PeriodDataQueuePushOutcome {
            accepted: true,
            clear_existing,
            expected_next_period,
            actual_period: entry_period,
            current_period: self.period,
            effective_size: self.size(),
        })
    }

    /// Pops queue metadata and returns the C++ payload/cert-vote handoff plan.
    ///
    /// Error behavior:
    /// - returns an error when the raw queue is empty.
    pub fn pop(&mut self) -> Result<PeriodDataQueuePopPlan> {
        let Some(entry) = self.entries.pop_front() else {
            return Err(anyhow!("cannot pop from empty period data queue"));
        };

        if let Some(next) = self.entries.front() {
            return Ok(PeriodDataQueuePopPlan {
                entry_id: entry.entry_id,
                entry_period: entry.period,
                block_hash: entry.block_hash,
                prev_block_hash: entry.prev_block_hash,
                pivot_hash: entry.pivot_hash,
                final_chain_hash: entry.final_chain_hash,
                reward_vote_hashes: entry.reward_vote_hashes,
                pillar_vote_rlps: entry.pillar_vote_rlps,
                dag_transaction_hashes: entry.dag_transaction_hashes,
                period_data_transaction_hashes: entry.period_data_transaction_hashes,
                period_data_transaction_identities: entry.period_data_transaction_identities,
                previous_cert_votes_present: entry.previous_cert_votes_present,
                previous_cert_first_vote_has_weight: entry.previous_cert_first_vote_has_weight,
                pillar_votes_present: entry.pillar_votes_present,
                extra_data_present: entry.extra_data_present,
                extra_data_pillar_block_hash_present: entry.extra_data_pillar_block_hash_present,
                use_last_block_cert_votes: false,
                next_entry_id: next.entry_id,
                current_period: self.period,
                effective_size: self.size(),
            });
        }

        self.period = 0;
        self.last_block_cert_votes_available = false;
        Ok(PeriodDataQueuePopPlan {
            entry_id: entry.entry_id,
            entry_period: entry.period,
            block_hash: entry.block_hash,
            prev_block_hash: entry.prev_block_hash,
            pivot_hash: entry.pivot_hash,
            final_chain_hash: entry.final_chain_hash,
            reward_vote_hashes: entry.reward_vote_hashes,
            pillar_vote_rlps: entry.pillar_vote_rlps,
            dag_transaction_hashes: entry.dag_transaction_hashes,
            period_data_transaction_hashes: entry.period_data_transaction_hashes,
            period_data_transaction_identities: entry.period_data_transaction_identities,
            previous_cert_votes_present: entry.previous_cert_votes_present,
            previous_cert_first_vote_has_weight: entry.previous_cert_first_vote_has_weight,
            pillar_votes_present: entry.pillar_votes_present,
            extra_data_present: entry.extra_data_present,
            extra_data_pillar_block_hash_present: entry.extra_data_pillar_block_hash_present,
            use_last_block_cert_votes: true,
            next_entry_id: 0,
            current_period: self.period,
            effective_size: self.size(),
        })
    }

    /// Returns the last queued entry metadata, if any.
    pub fn last_entry(&self) -> Option<PeriodDataQueueEntryRef> {
        self.entries.back().cloned()
    }

    /// Removes queued entries with period lower than `period`.
    ///
    /// This intentionally preserves legacy behavior: only front entries are
    /// removed, while `period` and last-cert-vote availability are left intact.
    /// Removed entry ids are returned so C++ can drop matching live payloads.
    pub fn clean_old_data(&mut self, period: u64) -> Vec<PeriodDataQueueEntryRef> {
        let mut removed = Vec::new();
        while self
            .entries
            .front()
            .map(|entry| entry.period < period)
            .unwrap_or(false)
        {
            if let Some(entry) = self.entries.pop_front() {
                removed.push(entry);
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(
        queue: &mut PeriodDataQueue,
        id: u64,
        period: u64,
        max_size: u64,
        cert_votes: usize,
    ) -> bool {
        queue
            .push(
                id,
                period,
                H256::from_low_u64_be(id),
                H256::from_low_u64_be(id + 1000),
                H256::from_low_u64_be(id + 2000),
                H256::from_low_u64_be(id + 2500),
                vec![H256::from_low_u64_be(id + 2600)],
                vec![vec![id as u8, 0xa0]],
                vec![H256::from_low_u64_be(id + 3000)],
                vec![H256::from_low_u64_be(id + 4000)],
                vec![PeriodDataQueueTransactionIdentity {
                    input_index: 0,
                    hash: H256::from_low_u64_be(id + 4000),
                    transaction_nonce: [id as u8; 32],
                    sender: [id as u8; 20],
                }],
                id % 2 == 0,
                id % 3 == 0,
                id % 5 == 0,
                id % 7 == 0,
                id % 7 == 0 && id % 11 == 0,
                max_size,
                cert_votes,
            )
            .unwrap()
            .accepted
    }

    #[test]
    fn push_accepts_sequential_periods_and_rejects_gaps() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 1, 1, 0, 1));
        assert_eq!(queue.period(), 1);
        assert!(!push(&mut queue, 2, 3, 0, 1));
        assert_eq!(queue.period(), 1);
        assert!(push(&mut queue, 3, 2, 0, 1));
        assert_eq!(queue.period(), 2);
    }

    #[test]
    fn push_accepts_empty_queue_backfill_period() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 2, 2, 0, 1));
        assert_eq!(queue.period(), 2);
    }

    #[test]
    fn accepted_push_clears_existing_entries_when_chain_advances() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 2, 2, 0, 1));
        let outcome = queue
            .push(
                4,
                4,
                H256::from_low_u64_be(4),
                H256::from_low_u64_be(1004),
                H256::from_low_u64_be(2004),
                H256::from_low_u64_be(2504),
                vec![H256::from_low_u64_be(2604)],
                vec![vec![4, 0xa0]],
                vec![H256::from_low_u64_be(3004)],
                vec![H256::from_low_u64_be(4004)],
                vec![PeriodDataQueueTransactionIdentity {
                    input_index: 0,
                    hash: H256::from_low_u64_be(4004),
                    transaction_nonce: [4; 32],
                    sender: [4; 20],
                }],
                true,
                false,
                true,
                true,
                false,
                3,
                1,
            )
            .unwrap();

        assert!(outcome.accepted);
        assert!(outcome.clear_existing);
        assert_eq!(queue.period(), 4);
        assert_eq!(queue.syncing_period(3), 4);
        assert_eq!(
            queue.last_block_hash_or_chain(4, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(4)
        );
        assert_eq!(queue.last_entry().unwrap().entry_id, 4);
        assert_eq!(
            queue.last_entry().unwrap().block_hash,
            H256::from_low_u64_be(4)
        );
        assert_eq!(
            queue.last_entry().unwrap().prev_block_hash,
            H256::from_low_u64_be(1004)
        );
        assert_eq!(
            queue.last_entry().unwrap().pivot_hash,
            H256::from_low_u64_be(2004)
        );
        assert_eq!(
            queue.last_entry().unwrap().final_chain_hash,
            H256::from_low_u64_be(2504)
        );
        assert_eq!(
            queue.last_entry().unwrap().dag_transaction_hashes,
            vec![H256::from_low_u64_be(3004)]
        );
        assert_eq!(
            queue.last_entry().unwrap().pillar_vote_rlps,
            vec![vec![4, 0xa0]]
        );
        assert_eq!(
            queue.last_entry().unwrap().period_data_transaction_hashes,
            vec![H256::from_low_u64_be(4004)]
        );
        assert_eq!(
            queue
                .last_entry()
                .unwrap()
                .period_data_transaction_identities,
            vec![PeriodDataQueueTransactionIdentity {
                input_index: 0,
                hash: H256::from_low_u64_be(4004),
                transaction_nonce: [4; 32],
                sender: [4; 20]
            }]
        );
        assert!(queue.last_entry().unwrap().previous_cert_votes_present);
        assert!(
            !queue
                .last_entry()
                .unwrap()
                .previous_cert_first_vote_has_weight
        );
        assert!(queue.last_entry().unwrap().pillar_votes_present);
        assert!(queue.last_entry().unwrap().extra_data_present);
        assert!(
            !queue
                .last_entry()
                .unwrap()
                .extra_data_pillar_block_hash_present
        );
        assert_eq!(queue.size(), 1);
    }

    #[test]
    fn size_hides_tail_without_last_block_cert_votes() {
        let mut queue = PeriodDataQueue::new();

        assert!(push(&mut queue, 1, 1, 0, 0));
        assert_eq!(queue.size(), 0);
        assert!(!queue.is_empty());

        assert!(push(&mut queue, 2, 2, 0, 0));
        assert_eq!(queue.size(), 1);
    }

    #[test]
    fn pop_selects_next_entry_cert_votes_before_last_cert_votes() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 11, 1, 0, 1));
        assert!(push(&mut queue, 22, 2, 0, 1));

        let first = queue.pop().unwrap();
        assert_eq!(first.entry_id, 11);
        assert_eq!(first.entry_period, 1);
        assert_eq!(first.block_hash, H256::from_low_u64_be(11));
        assert_eq!(first.prev_block_hash, H256::from_low_u64_be(1011));
        assert_eq!(first.pivot_hash, H256::from_low_u64_be(2011));
        assert_eq!(first.final_chain_hash, H256::from_low_u64_be(2511));
        assert_eq!(first.pillar_vote_rlps, vec![vec![11, 0xa0]]);
        assert_eq!(
            first.dag_transaction_hashes,
            vec![H256::from_low_u64_be(3011)]
        );
        assert_eq!(
            first.period_data_transaction_hashes,
            vec![H256::from_low_u64_be(4011)]
        );
        assert_eq!(first.period_data_transaction_identities.len(), 1);
        assert_eq!(
            first.period_data_transaction_identities[0].hash,
            H256::from_low_u64_be(4011)
        );
        assert!(!first.previous_cert_votes_present);
        assert!(!first.previous_cert_first_vote_has_weight);
        assert!(!first.pillar_votes_present);
        assert!(!first.extra_data_present);
        assert!(!first.extra_data_pillar_block_hash_present);
        assert!(!first.use_last_block_cert_votes);
        assert_eq!(first.next_entry_id, 22);
        assert_eq!(queue.period(), 2);

        let second = queue.pop().unwrap();
        assert_eq!(second.entry_id, 22);
        assert!(second.previous_cert_votes_present);
        assert!(!second.previous_cert_first_vote_has_weight);
        assert!(!second.pillar_votes_present);
        assert!(!second.extra_data_present);
        assert!(!second.extra_data_pillar_block_hash_present);
        assert!(second.use_last_block_cert_votes);
        assert_eq!(second.next_entry_id, 0);
        assert_eq!(queue.period(), 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn clean_old_data_returns_removed_ids_and_preserves_period() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 5, 5, 4, 1));
        assert!(push(&mut queue, 6, 6, 4, 1));

        let removed = queue.clean_old_data(6);

        assert_eq!(
            removed,
            vec![PeriodDataQueueEntryRef {
                entry_id: 5,
                period: 5,
                block_hash: H256::from_low_u64_be(5),
                prev_block_hash: H256::from_low_u64_be(1005),
                pivot_hash: H256::from_low_u64_be(2005),
                final_chain_hash: H256::from_low_u64_be(2505),
                reward_vote_hashes: vec![H256::from_low_u64_be(2605)],
                pillar_vote_rlps: vec![vec![5, 0xa0]],
                dag_transaction_hashes: vec![H256::from_low_u64_be(3005)],
                period_data_transaction_hashes: vec![H256::from_low_u64_be(4005)],
                period_data_transaction_identities: vec![PeriodDataQueueTransactionIdentity {
                    input_index: 0,
                    hash: H256::from_low_u64_be(4005),
                    transaction_nonce: [5; 32],
                    sender: [5; 20]
                }],
                previous_cert_votes_present: false,
                previous_cert_first_vote_has_weight: false,
                pillar_votes_present: true,
                extra_data_present: false,
                extra_data_pillar_block_hash_present: false
            }]
        );
        assert_eq!(queue.period(), 6);
        assert_eq!(queue.syncing_period(8), 8);
        assert_eq!(
            queue.last_block_hash_or_chain(6, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(6)
        );
        assert_eq!(
            queue.last_block_hash_or_chain(7, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(99)
        );
        assert_eq!(queue.last_entry().unwrap().entry_id, 6);
    }

    #[test]
    fn clear_resets_all_state_and_pop_empty_errors() {
        let mut queue = PeriodDataQueue::new();
        assert!(push(&mut queue, 1, 1, 0, 1));

        queue.clear();

        assert_eq!(queue.period(), 0);
        assert_eq!(queue.syncing_period(7), 7);
        assert_eq!(
            queue.last_block_hash_or_chain(1, H256::from_low_u64_be(99)),
            H256::from_low_u64_be(99)
        );
        assert!(queue.is_empty());
        assert_eq!(queue.size(), 0);
        let err = queue.pop().unwrap_err().to_string();
        assert!(err.contains("empty period data queue"));
    }
}
