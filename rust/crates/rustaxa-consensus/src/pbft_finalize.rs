//! Deterministic PBFT finalization intent planning.
//!
//! This module receives plain, C++-computed facts from the execute/finalize
//! boundary and returns a bridge-safe intent that only captures runtime
//! side-effects. No storage mutation, I/O, scheduling, locking, or DB reads are
//! performed in this planner.
//!
//! Inputs:
//! - `block_period`, `block_prev_hash`, `chain_last_hash`, `chain_last_period`:
//!   used only for deterministic candidate acceptance checks.
//! - `block_in_chain`: if true, the candidate was already written previously.
//! - `pivot_dag_anchor_hash`: determines anchored vs null-anchor behavior.
//! - `has_pillar_block` + `pillar_block_finalized`: controls acceptance of
//!   pillar-linked PBFT blocks in Ficus-era hardfork paths.
//! - certified-vote and dynamic-lambda facts let Rust prepare a storage-write
//!   intent that can be executed natively by Rust in the next slice.
//!
//! Outputs:
//! - `finalize_block`: whether the PBFT block should continue through execute/
//!   finalize side-effects.
//! - `anchor`: null-anchor vs anchored classification.
//! - `executed_pbft_block`: intent for setting the manager's executed flag.
//! - `cleanup`: a bounded cleanup intent used by C++ to schedule deterministic
//!   in-memory/storage-facing updates in a fixed order.
//! - `storage_write_intent`: the PBFT persistence command shape. C++ still
//!   applies the writes in this slice, but Rust owns the decision and facts.
//! - `status`: explicit decision status code for metrics/logging/telemetry.
use ethereum_types::H256;

/// Null-anchor / anchored status reported in a planner plan.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationAnchor {
    /// PBFT block has `kNullBlockHash` anchor and should follow null-anchor
    /// finalization semantics.
    Null,
    /// PBFT block has a concrete DAG pivot anchor hash.
    Anchored,
    /// Input encoded an unknown anchor code while coming from bridge payloads.
    Unknown,
}

impl PbftFinalizationAnchor {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Anchored => 1,
            Self::Unknown => 255,
        }
    }

    /// Decodes a bridge code from C++.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Null,
            1 => Self::Anchored,
            _ => Self::Unknown,
        }
    }
}

/// Finalization status result codes used by both Rust and C++.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PbftFinalizationStatus {
    /// The block is accepted for finalization execution.
    Accepted,
    /// Block is already present in chain / storage.
    BlockAlreadyInChain,
    /// Candidate is stale for the current head period and cannot be finalized.
    StalePeriod,
    /// The candidate prev hash mismatches chain head for a non-stale block.
    PreviousHashMismatch,
    /// A pillar-linked PBFT block requires pillar-finalization input that was not provided.
    PillarDependencyMissing,
    /// A non-duplicate finalization path was called without certified votes.
    EmptyCertVotes,
    /// The sample certified vote does not certify the PBFT block hash.
    CertVoteBlockMismatch,
    /// The caller omitted storage payload facts required for accepted writes.
    StorageFactsIncomplete,
    /// Internal contract error or impossible status in transport facts.
    ContractError,
    /// Unknown status code produced from legacy inputs.
    Unknown,
}

impl PbftFinalizationStatus {
    /// Stable bridge code for C++.
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::BlockAlreadyInChain => 1,
            Self::StalePeriod => 2,
            Self::PreviousHashMismatch => 3,
            Self::PillarDependencyMissing => 4,
            Self::EmptyCertVotes => 5,
            Self::CertVoteBlockMismatch => 6,
            Self::StorageFactsIncomplete => 7,
            Self::ContractError => 255,
            Self::Unknown => 254,
        }
    }

    /// Decodes a bridge code from C++.
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Accepted,
            1 => Self::BlockAlreadyInChain,
            2 => Self::StalePeriod,
            3 => Self::PreviousHashMismatch,
            4 => Self::PillarDependencyMissing,
            5 => Self::EmptyCertVotes,
            6 => Self::CertVoteBlockMismatch,
            7 => Self::StorageFactsIncomplete,
            255 => Self::ContractError,
            _ => Self::Unknown,
        }
    }
}

/// Minimal bounded cleanup intent for deterministic finalize-path side-effects.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PbftFinalizationCleanupIntent {
    /// Persist selected PBFT block metadata (`pbft_blocks` and PBFT head entry).
    pub persist_pbft_block_metadata: bool,
    /// Persist per-period reward vote state.
    pub reset_reward_votes: bool,
    /// Apply finalized DAG ordering/anchor changes for this block.
    pub set_dag_block_order: bool,
    /// Update sortition parameters for anchored PBFT finalization.
    pub update_sortition_params: bool,
    /// Update transaction manager finalized-transaction bookkeeping.
    pub update_finalized_transactions_status: bool,
    /// Update PBFT head runtime chain state.
    pub update_pbft_chain: bool,
    /// Clear one-period cache of anchored DAG order lookups.
    pub clear_anchor_dag_cache: bool,
    /// Execute final-chain finalize path for the block.
    pub finalize_final_chain: bool,
    /// Persist lambda/period bookkeeping for Cacti-era blocks.
    pub maybe_update_dynamic_lambda: bool,
    /// Advance PBFT manager consensus period.
    pub advance_period: bool,
}

impl PbftFinalizationCleanupIntent {
    const fn reject() -> Self {
        Self {
            persist_pbft_block_metadata: false,
            reset_reward_votes: false,
            set_dag_block_order: false,
            update_sortition_params: false,
            update_finalized_transactions_status: false,
            update_pbft_chain: false,
            clear_anchor_dag_cache: false,
            finalize_final_chain: false,
            maybe_update_dynamic_lambda: false,
            advance_period: false,
        }
    }
}

/// Hash plus finalized-position metadata for a planned storage index write.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationPositionedHash {
    /// Hash to index in finalized period storage.
    pub hash: H256,
    /// Zero-based position in the accepted period ordering.
    pub position: u32,
}

/// Explicit storage-write intent used as a transition plan before native Rust DB writes are enabled.
///
/// The booleans identify the PBFT persistence operations the caller should
/// execute. The scalar and hash fields carry the exact facts a native Rust DB
/// writer needs next: PBFT head key/value identity, reward-vote reset identity,
/// dynamic-lambda persistence decision, reward calculation block rate, and
/// executed-status value. Opaque legacy payloads such as `PeriodData` are still
/// materialized by C++ in this slice.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationStorageWriteIntent {
    /// Persist selected PBFT block metadata (`pbft_blocks` and PBFT head entry).
    pub persist_pbft_head: bool,
    /// Persist period-data payload (`savePeriodData`).
    pub persist_period_data: bool,
    /// Persist per-period reward vote state update path.
    pub reset_reward_votes: bool,
    /// Persist sortition parameters for anchored PBFT finalization.
    pub update_sortition_params: bool,
    /// Persist lambda/period bookkeeping path for dynamic-lambda-enabled blocks.
    pub apply_dynamic_lambda_update: bool,
    /// Persist period lambda because storage is missing it or contains a different value.
    pub persist_period_lambda: bool,
    /// Persist `PbftMgrStatus::ExecutedBlock`.
    pub persist_executed_pbft_status: bool,
    /// Accepted PBFT block hash.
    pub pbft_block_hash: H256,
    /// PBFT head storage key that should receive the projected head payload.
    pub pbft_head_hash: H256,
    /// Accepted PBFT block period.
    pub block_period: u64,
    /// Whether PBFT-head metadata should encode a null-anchor block.
    pub null_anchor: bool,
    /// Certified-vote period used by reward-vote reset.
    pub reward_vote_period: u64,
    /// Certified-vote round used by reward-vote reset.
    pub reward_vote_round: u64,
    /// Certified-vote step used by reward-vote reset.
    pub reward_vote_step: u64,
    /// Certified-vote block hash used by reward-vote reset.
    pub reward_vote_block_hash: H256,
    /// Lambda value to persist when `persist_period_lambda` is true.
    pub period_lambda: u32,
    /// Blocks-per-year value that must be passed to FinalChain finalization.
    pub blocks_per_year: u32,
    /// Executed status value to persist.
    pub executed_pbft_status: bool,
    /// Canonical period-data RLP payload to write.
    pub period_data_rlp: Vec<u8>,
    /// Finalized DAG block period index writes in storage order.
    pub dag_block_period_writes: Vec<PbftFinalizationPositionedHash>,
    /// Finalized transaction location writes in storage order.
    pub transaction_location_writes: Vec<PbftFinalizationPositionedHash>,
}

impl PbftFinalizationStorageWriteIntent {
    fn reject() -> Self {
        Self {
            persist_pbft_head: false,
            persist_period_data: false,
            reset_reward_votes: false,
            update_sortition_params: false,
            apply_dynamic_lambda_update: false,
            persist_period_lambda: false,
            persist_executed_pbft_status: false,
            pbft_block_hash: H256::zero(),
            pbft_head_hash: H256::zero(),
            block_period: 0,
            null_anchor: false,
            reward_vote_period: 0,
            reward_vote_round: 0,
            reward_vote_step: 0,
            reward_vote_block_hash: H256::zero(),
            period_lambda: 0,
            blocks_per_year: 0,
            executed_pbft_status: false,
            period_data_rlp: Vec::new(),
            dag_block_period_writes: Vec::new(),
            transaction_location_writes: Vec::new(),
        }
    }
}

/// Input facts from C++ execute/finalize path.
#[derive(Debug, Clone)]
pub struct PbftFinalizationIntentFact {
    /// PBFT candidate block hash.
    pub block_hash: H256,
    /// PBFT head storage key used by legacy `addPbftHeadToBatch`.
    pub pbft_head_hash: H256,
    /// PBFT candidate period.
    pub block_period: u64,
    /// PBFT candidate prev hash.
    pub block_prev_hash: H256,
    /// Current chain head hash at intent time.
    pub chain_last_hash: H256,
    /// Current chain last period at intent time.
    pub chain_last_period: u64,
    /// True when `pbftBlock` is already in chain/storage.
    pub block_in_chain: bool,
    /// PBFT block pivot DAG anchor hash.
    pub pivot_dag_anchor_hash: H256,
    /// Block carries a Pillar block hash and therefore requires pillar-chain finalize.
    pub has_pillar_block: bool,
    /// Pillar finalization result supplied by C++ for this candidate.
    pub pillar_block_finalized: bool,
    /// C++ precomputed dynamic-lambda path requirement.
    pub request_dynamic_lambda_update: bool,
    /// Number of certified votes supplied for this non-duplicate finalization.
    pub cert_vote_count: u64,
    /// Sample certified-vote block hash.
    pub sample_cert_vote_block_hash: H256,
    /// Sample certified-vote period.
    pub sample_cert_vote_period: u64,
    /// Sample certified-vote round.
    pub sample_cert_vote_round: u64,
    /// Sample certified-vote step.
    pub sample_cert_vote_step: u64,
    /// Lambda used by this PBFT block round.
    pub block_lambda: u32,
    /// Whether storage already has the previous saved period lambda.
    pub last_saved_period_lambda_found: bool,
    /// Last saved period lambda when present.
    pub last_saved_period_lambda: u32,
    /// C++-computed Cacti-era blocks-per-year value for `block_lambda`.
    pub dynamic_blocks_per_year: u32,
    /// Genesis-configured pre-Cacti blocks-per-year value.
    pub dpos_blocks_per_year: u32,
    /// Canonical period-data RLP payload that native Rust storage will write.
    pub period_data_rlp: Vec<u8>,
    /// Ordered finalized DAG block hashes.
    pub ordered_dag_block_hashes: Vec<H256>,
    /// Ordered finalized transaction hashes after legacy nonce reordering.
    pub ordered_transaction_hashes: Vec<H256>,
}

/// Deterministic finalization runtime intent returned to C++ for one certified PBFT
/// block path.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PbftFinalizationPlan {
    /// True when this candidate should continue into C++ execute/finalize effects.
    pub finalize_block: bool,
    /// Null-anchor or anchored intent.
    pub anchor: PbftFinalizationAnchor,
    /// `db_->savePbftMgrStatus(PbftMgrStatus::ExecutedBlock, true)` intent.
    pub executed_pbft_block: bool,
    /// Cleanup intent flags for the caller.
    pub cleanup: PbftFinalizationCleanupIntent,
    /// Storage-write intent planned for Rust native persistence.
    pub storage_write_intent: PbftFinalizationStorageWriteIntent,
    /// Explicit status reason for telemetry and error-path handling.
    pub status: PbftFinalizationStatus,
}

impl PbftFinalizationPlan {
    fn accept(anchor: PbftFinalizationAnchor, fact: PbftFinalizationIntentFact) -> Self {
        let anchored = anchor == PbftFinalizationAnchor::Anchored;
        let persist_period_lambda = fact.request_dynamic_lambda_update
            && (!fact.last_saved_period_lambda_found
                || fact.last_saved_period_lambda != fact.block_lambda);
        let blocks_per_year = if fact.request_dynamic_lambda_update {
            fact.dynamic_blocks_per_year
        } else {
            fact.dpos_blocks_per_year
        };
        Self {
            finalize_block: true,
            anchor,
            executed_pbft_block: true,
            cleanup: PbftFinalizationCleanupIntent {
                persist_pbft_block_metadata: true,
                reset_reward_votes: true,
                set_dag_block_order: true,
                update_sortition_params: anchored,
                update_finalized_transactions_status: true,
                update_pbft_chain: true,
                clear_anchor_dag_cache: true,
                finalize_final_chain: true,
                maybe_update_dynamic_lambda: fact.request_dynamic_lambda_update,
                advance_period: true,
            },
            storage_write_intent: PbftFinalizationStorageWriteIntent {
                persist_pbft_head: true,
                persist_period_data: true,
                reset_reward_votes: true,
                update_sortition_params: anchored,
                apply_dynamic_lambda_update: fact.request_dynamic_lambda_update,
                persist_period_lambda,
                persist_executed_pbft_status: true,
                pbft_block_hash: fact.block_hash,
                pbft_head_hash: fact.pbft_head_hash,
                block_period: fact.block_period,
                null_anchor: anchor == PbftFinalizationAnchor::Null,
                reward_vote_period: fact.sample_cert_vote_period,
                reward_vote_round: fact.sample_cert_vote_round,
                reward_vote_step: fact.sample_cert_vote_step,
                reward_vote_block_hash: fact.sample_cert_vote_block_hash,
                period_lambda: fact.block_lambda,
                blocks_per_year,
                executed_pbft_status: true,
                period_data_rlp: fact.period_data_rlp,
                dag_block_period_writes: positioned_hashes(fact.ordered_dag_block_hashes),
                transaction_location_writes: positioned_hashes(fact.ordered_transaction_hashes),
            },
            status: PbftFinalizationStatus::Accepted,
        }
    }

    fn reject(status: PbftFinalizationStatus, anchor: PbftFinalizationAnchor) -> Self {
        Self {
            finalize_block: false,
            anchor,
            executed_pbft_block: false,
            cleanup: PbftFinalizationCleanupIntent::reject(),
            storage_write_intent: PbftFinalizationStorageWriteIntent::reject(),
            status,
        }
    }
}

/// Builds a deterministic finalization plan from plain facts.
///
/// Ordering and contracts are intentionally side-effect-free:
/// - `block_in_chain` and stale/prev-hash conflicts reject without state change.
/// - pillar-linked blocks require explicit success from C++ pillar-domain checks.
/// - accepted plans mirror legacy non-null anchored cleanup behavior (`sortitionParamsManager`
///   update only for non-null anchors).
pub fn plan_pbft_finalization_intent(fact: PbftFinalizationIntentFact) -> PbftFinalizationPlan {
    let anchor = if fact.pivot_dag_anchor_hash.is_zero() {
        PbftFinalizationAnchor::Null
    } else {
        PbftFinalizationAnchor::Anchored
    };

    if fact.block_in_chain {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::BlockAlreadyInChain, anchor);
    }

    if fact.cert_vote_count == 0 {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::EmptyCertVotes, anchor);
    }

    if fact.sample_cert_vote_block_hash != fact.block_hash {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::CertVoteBlockMismatch, anchor);
    }

    if fact.period_data_rlp.is_empty() {
        return PbftFinalizationPlan::reject(
            PbftFinalizationStatus::StorageFactsIncomplete,
            anchor,
        );
    }

    if fact.block_prev_hash != fact.chain_last_hash && fact.block_period <= fact.chain_last_period {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::StalePeriod, anchor);
    }

    if fact.block_prev_hash != fact.chain_last_hash {
        return PbftFinalizationPlan::reject(PbftFinalizationStatus::PreviousHashMismatch, anchor);
    }

    if fact.has_pillar_block && !fact.pillar_block_finalized {
        return PbftFinalizationPlan::reject(
            PbftFinalizationStatus::PillarDependencyMissing,
            anchor,
        );
    }

    PbftFinalizationPlan::accept(anchor, fact)
}

fn positioned_hashes(hashes: Vec<H256>) -> Vec<PbftFinalizationPositionedHash> {
    hashes
        .into_iter()
        .enumerate()
        .map(|(position, hash)| PbftFinalizationPositionedHash {
            hash,
            position: position as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(v: u64) -> H256 {
        H256::from_low_u64_be(v)
    }

    fn accepted_fact() -> PbftFinalizationIntentFact {
        PbftFinalizationIntentFact {
            block_hash: hash(99),
            pbft_head_hash: hash(88),
            block_period: 10,
            block_prev_hash: hash(42),
            chain_last_hash: hash(42),
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: hash(123),
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
            cert_vote_count: 3,
            sample_cert_vote_block_hash: hash(99),
            sample_cert_vote_period: 10,
            sample_cert_vote_round: 2,
            sample_cert_vote_step: 5,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            dpos_blocks_per_year: 500,
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![hash(1), hash(2)],
            ordered_transaction_hashes: vec![hash(3), hash(4)],
        }
    }

    #[test]
    fn accepts_anchored_block_and_raises_expected_cleanup_intent() {
        let fact = accepted_fact();
        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, PbftFinalizationAnchor::Anchored);
        assert!(plan.executed_pbft_block);
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted);
        assert!(plan.cleanup.persist_pbft_block_metadata);
        assert!(plan.storage_write_intent.persist_pbft_head);
        assert!(plan.storage_write_intent.persist_period_data);
        assert!(plan.storage_write_intent.reset_reward_votes);
        assert!(plan.cleanup.update_sortition_params);
        assert!(plan.storage_write_intent.update_sortition_params);
        assert!(plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.persist_executed_pbft_status);
        assert_eq!(plan.storage_write_intent.pbft_block_hash, hash(99));
        assert_eq!(plan.storage_write_intent.pbft_head_hash, hash(88));
        assert_eq!(plan.storage_write_intent.block_period, 10);
        assert!(!plan.storage_write_intent.null_anchor);
        assert_eq!(plan.storage_write_intent.reward_vote_period, 10);
        assert_eq!(plan.storage_write_intent.reward_vote_round, 2);
        assert_eq!(plan.storage_write_intent.reward_vote_step, 5);
        assert_eq!(plan.storage_write_intent.reward_vote_block_hash, hash(99));
        assert_eq!(plan.storage_write_intent.period_lambda, 1_500);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 1_000);
        assert!(plan.storage_write_intent.executed_pbft_status);
        assert_eq!(plan.storage_write_intent.period_data_rlp, vec![0xc0]);
        assert_eq!(
            plan.storage_write_intent.dag_block_period_writes,
            vec![
                PbftFinalizationPositionedHash {
                    hash: hash(1),
                    position: 0
                },
                PbftFinalizationPositionedHash {
                    hash: hash(2),
                    position: 1
                }
            ]
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes,
            vec![
                PbftFinalizationPositionedHash {
                    hash: hash(3),
                    position: 0
                },
                PbftFinalizationPositionedHash {
                    hash: hash(4),
                    position: 1
                }
            ]
        );
        assert!(plan.cleanup.finalize_final_chain);
        assert!(plan.cleanup.advance_period);
        assert!(plan.cleanup.set_dag_block_order);
        assert!(plan.cleanup.update_finalized_transactions_status);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
    }

    #[test]
    fn null_anchor_is_skipped_from_sortition_update_cleanup() {
        let mut fact = accepted_fact();
        fact.pivot_dag_anchor_hash = H256::zero();
        fact.request_dynamic_lambda_update = false;
        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, PbftFinalizationAnchor::Null);
        assert!(!plan.cleanup.update_sortition_params);
        assert!(!plan.storage_write_intent.update_sortition_params);
        assert!(!plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(!plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.null_anchor);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 500);
    }

    #[test]
    fn rejects_duplicate_blocks() {
        let mut fact = accepted_fact();
        fact.block_in_chain = true;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::BlockAlreadyInChain);
        assert!(!plan.executed_pbft_block);
        assert!(!plan.cleanup.advance_period);
        assert!(!plan.storage_write_intent.persist_pbft_head);
        assert!(!plan.storage_write_intent.persist_period_data);
        assert!(!plan.storage_write_intent.reset_reward_votes);
        assert!(!plan.storage_write_intent.persist_executed_pbft_status);
    }

    #[test]
    fn rejects_missing_and_mismatched_cert_vote_facts() {
        let mut fact = accepted_fact();
        fact.cert_vote_count = 0;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::EmptyCertVotes);
        assert!(!plan.storage_write_intent.persist_pbft_head);

        fact = accepted_fact();
        fact.sample_cert_vote_block_hash = hash(100);

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::CertVoteBlockMismatch);
        assert!(!plan.storage_write_intent.persist_period_data);
    }

    #[test]
    fn skips_period_lambda_storage_when_existing_value_matches() {
        let mut fact = accepted_fact();
        fact.last_saved_period_lambda_found = true;
        fact.last_saved_period_lambda = fact.block_lambda;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(plan.finalize_block);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(!plan.storage_write_intent.persist_period_lambda);
    }

    #[test]
    fn rejects_missing_storage_payload_facts_for_accepted_blocks() {
        let mut fact = accepted_fact();
        fact.period_data_rlp.clear();

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::StorageFactsIncomplete);
        assert!(plan.storage_write_intent.period_data_rlp.is_empty());
        assert!(plan.storage_write_intent.dag_block_period_writes.is_empty());
    }

    #[test]
    fn rejects_pillar_blocks_without_finalized_pillar() {
        let mut fact = accepted_fact();
        fact.has_pillar_block = true;
        fact.pillar_block_finalized = false;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::PillarDependencyMissing);
    }

    #[test]
    fn rejects_stale_prev_hash_conflicts() {
        let mut fact = accepted_fact();
        fact.block_prev_hash = hash(41);
        fact.chain_last_period = 12;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::StalePeriod);
    }

    #[test]
    fn rejects_non_stale_prev_hash_mismatch_with_previous_hash_status() {
        let mut fact = accepted_fact();
        fact.block_prev_hash = hash(41);
        fact.chain_last_period = 9;

        let plan = plan_pbft_finalization_intent(fact);

        assert!(!plan.finalize_block);
        assert_eq!(plan.status, PbftFinalizationStatus::PreviousHashMismatch);
    }
}
