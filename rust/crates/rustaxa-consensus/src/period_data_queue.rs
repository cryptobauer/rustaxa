//! Period-data sync queue metadata for PBFT rewrite mode.
//!
//! This module models the deterministic queue contract used while syncing PBFT
//! period data from peers. It deliberately owns only compact queue metadata:
//! entry ids, periods, block hashes, validation facts, effective processable
//! size, and pop/cleanup decisions. The C++ shim keeps ownership of live
//! `PeriodData` and peer `NodeID` objects until those model types are ported.

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
/// - `transaction_rlps`: canonical transaction payloads carried by the synced
///   period-data payload for finalization materialization.
/// - `previous_cert_vote_rlps`: canonical PBFT cert-vote payloads carried by
///   the synced period-data payload for the previous block.
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
    pub transaction_rlps: Vec<Vec<u8>>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
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

/// Complete native request for admitting one synced period-data payload.
///
/// `entry` carries the durable-domain payload facts, `max_pbft_size` is the
/// current PBFT-chain size used by admission arithmetic, and
/// `current_block_cert_vote_rlps` supplies the final-entry certificate source.
/// The request is consumed exactly once; rejected admission does not mutate
/// queue state, while arithmetic overflow is returned as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeriodDataQueuePushRequest {
    pub entry: PeriodDataQueueEntryRef,
    pub max_pbft_size: u64,
    pub current_block_cert_vote_rlps: Vec<Vec<u8>>,
}

/// Coherent read-only view of Rust-owned period-data queue state.
///
/// The caller supplies the remaining PBFT-chain compatibility facts. The
/// snapshot derives all queue fields under the manager serialization lock and
/// never exposes the queue itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeriodDataQueueSnapshot {
    pub period: u64,
    pub syncing_period: u64,
    pub last_block_hash_or_chain: H256,
    pub size: usize,
    pub empty: bool,
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
/// - `cert_vote_rlps`: canonical PBFT cert-vote payloads selected by Rust for
///   the popped block. They are either the next queued entry's previous-cert
///   payloads or the current last-block cert-vote payloads.
/// - `use_last_block_cert_votes`: true when Rust selected the cert votes passed
///   with the last queued block; false means cert votes came from the next
///   queued entry.
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
/// - `transaction_rlps` are canonical transaction payload bytes from the
///   popped period-data payload.
/// - `previous_cert_vote_rlps` are canonical cert-vote payload bytes from the
///   popped period-data payload's previous-cert sidecar.
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
    pub transaction_rlps: Vec<Vec<u8>>,
    pub cert_vote_rlps: Vec<Vec<u8>>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
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
    last_block_cert_vote_rlps: Vec<Vec<u8>>,
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
        if !self.last_block_cert_vote_rlps.is_empty() || self.entries.is_empty() {
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
        self.last_block_cert_vote_rlps.clear();
    }

    /// Returns one coherent queue snapshot using supplied PBFT-chain facts.
    pub fn snapshot(
        &self,
        pbft_chain_size: u64,
        current_period: u64,
        chain_last_hash: H256,
    ) -> PeriodDataQueueSnapshot {
        PeriodDataQueueSnapshot {
            period: self.period(),
            syncing_period: self.syncing_period(pbft_chain_size),
            last_block_hash_or_chain: self
                .last_block_hash_or_chain(current_period, chain_last_hash),
            size: self.size(),
            empty: self.is_empty(),
        }
    }

    /// Attempts to admit one complete period-data queue request.
    ///
    /// Rejected period sequencing leaves the queue unchanged. Accepted chain
    /// advancement may clear stale entries before appending the request.
    /// Checked period arithmetic overflow is returned as an error.
    pub fn push(
        &mut self,
        request: PeriodDataQueuePushRequest,
    ) -> Result<PeriodDataQueuePushOutcome> {
        let PeriodDataQueuePushRequest {
            entry,
            max_pbft_size,
            current_block_cert_vote_rlps,
        } = request;
        let entry_period = entry.period;
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
        self.entries.push_back(entry);
        self.last_block_cert_vote_rlps = current_block_cert_vote_rlps;

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
                transaction_rlps: entry.transaction_rlps,
                cert_vote_rlps: next.previous_cert_vote_rlps.clone(),
                previous_cert_vote_rlps: entry.previous_cert_vote_rlps,
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
        let cert_vote_rlps = std::mem::take(&mut self.last_block_cert_vote_rlps);
        Ok(PeriodDataQueuePopPlan {
            entry_id: entry.entry_id,
            entry_period: entry.period,
            block_hash: entry.block_hash,
            prev_block_hash: entry.prev_block_hash,
            pivot_hash: entry.pivot_hash,
            final_chain_hash: entry.final_chain_hash,
            reward_vote_hashes: entry.reward_vote_hashes,
            pillar_vote_rlps: entry.pillar_vote_rlps,
            transaction_rlps: entry.transaction_rlps,
            cert_vote_rlps,
            previous_cert_vote_rlps: entry.previous_cert_vote_rlps,
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
    /// removed, while `period` and last-cert-vote payload availability are left intact.
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
        let previous_cert_vote_rlps = if id % 2 == 0 {
            vec![vec![id as u8, 0xc0]]
        } else {
            Vec::new()
        };
        let current_block_cert_vote_rlps = (0..cert_votes)
            .map(|idx| vec![id as u8, 0xd0 + idx as u8])
            .collect();
        queue
            .push(PeriodDataQueuePushRequest {
                entry: PeriodDataQueueEntryRef {
                    entry_id: id,
                    period,
                    block_hash: H256::from_low_u64_be(id),
                    prev_block_hash: H256::from_low_u64_be(id + 1000),
                    pivot_hash: H256::from_low_u64_be(id + 2000),
                    final_chain_hash: H256::from_low_u64_be(id + 2500),
                    reward_vote_hashes: vec![H256::from_low_u64_be(id + 2600)],
                    pillar_vote_rlps: vec![vec![id as u8, 0xa0]],
                    transaction_rlps: vec![vec![id as u8, 0xb0]],
                    previous_cert_vote_rlps,
                    dag_transaction_hashes: vec![H256::from_low_u64_be(id + 3000)],
                    period_data_transaction_hashes: vec![H256::from_low_u64_be(id + 4000)],
                    period_data_transaction_identities: vec![PeriodDataQueueTransactionIdentity {
                        input_index: 0,
                        hash: H256::from_low_u64_be(id + 4000),
                        transaction_nonce: [id as u8; 32],
                        sender: [id as u8; 20],
                    }],
                    previous_cert_votes_present: id % 2 == 0,
                    previous_cert_first_vote_has_weight: id % 3 == 0,
                    pillar_votes_present: id % 5 == 0,
                    extra_data_present: id % 7 == 0,
                    extra_data_pillar_block_hash_present: id % 7 == 0 && id % 11 == 0,
                },
                max_pbft_size: max_size,
                current_block_cert_vote_rlps,
            })
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
            .push(PeriodDataQueuePushRequest {
                entry: PeriodDataQueueEntryRef {
                    entry_id: 4,
                    period: 4,
                    block_hash: H256::from_low_u64_be(4),
                    prev_block_hash: H256::from_low_u64_be(1004),
                    pivot_hash: H256::from_low_u64_be(2004),
                    final_chain_hash: H256::from_low_u64_be(2504),
                    reward_vote_hashes: vec![H256::from_low_u64_be(2604)],
                    pillar_vote_rlps: vec![vec![4, 0xa0]],
                    transaction_rlps: vec![vec![4, 0xb0]],
                    previous_cert_vote_rlps: vec![vec![4, 0xc0]],
                    dag_transaction_hashes: vec![H256::from_low_u64_be(3004)],
                    period_data_transaction_hashes: vec![H256::from_low_u64_be(4004)],
                    period_data_transaction_identities: vec![PeriodDataQueueTransactionIdentity {
                        input_index: 0,
                        hash: H256::from_low_u64_be(4004),
                        transaction_nonce: [4; 32],
                        sender: [4; 20],
                    }],
                    previous_cert_votes_present: true,
                    previous_cert_first_vote_has_weight: false,
                    pillar_votes_present: true,
                    extra_data_present: true,
                    extra_data_pillar_block_hash_present: false,
                },
                max_pbft_size: 3,
                current_block_cert_vote_rlps: vec![vec![4, 0xd0]],
            })
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
            queue.last_entry().unwrap().transaction_rlps,
            vec![vec![4, 0xb0]]
        );
        assert_eq!(
            queue.last_entry().unwrap().previous_cert_vote_rlps,
            vec![vec![4, 0xc0]]
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
        assert_eq!(first.transaction_rlps, vec![vec![11, 0xb0]]);
        assert_eq!(first.cert_vote_rlps, vec![vec![22, 0xc0]]);
        assert!(first.previous_cert_vote_rlps.is_empty());
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
        assert_eq!(second.cert_vote_rlps, vec![vec![22, 0xd0]]);
        assert_eq!(second.previous_cert_vote_rlps, vec![vec![22, 0xc0]]);
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
                transaction_rlps: vec![vec![5, 0xb0]],
                previous_cert_vote_rlps: Vec::new(),
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
