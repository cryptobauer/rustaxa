//! Native ownership for the transaction proposal-packing protocol.
//!
//! The service owns the single packing mutex, owner identity, planner, ordered
//! candidate snapshot, shard cursor, pending-estimate protocol, and selected
//! ordering. Queue mutation and gas-cache storage remain with the surrounding
//! transaction application owner; this service returns typed intents for those
//! effects and never exposes its state or lock guard.

use crate::transaction_manager::{
    TransactionPackCandidate, TransactionPackEstimate, TransactionPackingPlanner,
};
use crate::transaction_queue::TransactionQueueEntry;
use anyhow::{Context, Result, anyhow, bail, ensure};
use ethereum_types::H256;
use std::sync::{Mutex, MutexGuard};

const PACKING_LOCK_POISONED: &str = "TM_RUNTIME_PACKING_LOCK_POISONED";

/// Identity allowed to complete or abort a packing session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionPackingOwner {
    /// Legacy public `TransactionManager::packTrxs` call.
    Compatibility,
    /// DAG proposer cursor identified by its service session id.
    DagProposer(u64),
}

/// One ordered queue candidate and its native-cache snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingCandidate {
    /// Queue metadata and canonical payload.
    pub entry: TransactionQueueEntry,
    /// Cached gas used for this proposal period, when present.
    pub cached_gas_used: Option<u64>,
}

/// Immutable inputs for starting one packing session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingRequest {
    /// Caller identity that alone may finalize or abort the session.
    pub owner: TransactionPackingOwner,
    /// Maximum cumulative proposal gas weight.
    pub weight_limit: u64,
    /// Minimum gas charged when deriving the candidate snapshot bound.
    pub min_transaction_gas: u64,
    /// Proposal period used for cache and shard selection.
    pub proposal_period: u64,
    /// Declared-gas ceiling below which no external estimate is needed.
    pub estimate_gas_limit: u64,
    /// FinalChain head recorded on queue-demotion effects.
    pub last_block_number: u64,
    /// Number of configured transaction shards; must be nonzero.
    pub total_shards: u16,
    /// Local transaction shard; must be less than `total_shards`.
    pub node_shard: u16,
    /// Periods per shard rotation; must be nonzero when sharding is enabled.
    pub shard_period_interval: u64,
    /// Deterministically ordered candidate and cache snapshot.
    pub candidates: Vec<TransactionPackingCandidate>,
}

/// External EVM result supplied after the prepare step releases all locks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingEstimate {
    /// Candidate hash, which must match the pending snapshot in order.
    pub hash: H256,
    /// Externally estimated gas used by planner accounting.
    pub gas_used: u64,
    /// FinalChain head associated with any resulting queue demotion.
    pub last_block_number: u64,
    /// Opaque execution-result RLP retained by the bridge-owned native cache.
    pub result_rlp: Vec<u8>,
}

/// Candidate requiring an external EVM estimate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingEstimateRequest {
    /// Retained canonical snapshot used for legacy materialization after locks release.
    pub entry: TransactionQueueEntry,
}

/// Selected transaction in deterministic proposal order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingSelection {
    /// Selected transaction hash.
    pub hash: H256,
    /// Gas charged to proposal weight.
    pub gas_used: u64,
    /// Canonical transaction payload retained at prepare time.
    pub transaction_rlp: Vec<u8>,
}

/// Queue demotion requested by native packing policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionPackingDemotionIntent {
    /// Candidate to demote if it remains proposable in the live queue.
    pub hash: H256,
    /// FinalChain head to record with the demotion.
    pub last_block_number: u64,
}

/// Gas-cache insertion requested for an external EVM result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactionPackingCacheIntent {
    /// Estimated transaction hash.
    pub hash: H256,
    /// Proposal period forming the other cache-key component.
    pub proposal_period: u64,
    /// Estimated gas stored with the opaque result.
    pub gas_used: u64,
    /// Opaque execution-result RLP owned by the cache adapter.
    pub result_rlp: Vec<u8>,
}

/// Ordered bridge-owned effect emitted for one processed estimate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransactionPackingEffect {
    /// Demote the live queue entry before applying any cache effect for the estimate.
    Demote(TransactionPackingDemotionIntent),
    /// Insert the opaque external EVM result in the gas-estimation cache.
    CacheInsert(TransactionPackingCacheIntent),
}

/// Complete output of a prepare or finalize protocol step.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TransactionPackingStep {
    /// Ordered candidates that require unlocked external EVM estimation.
    pub request_estimates: Vec<TransactionPackingEstimateRequest>,
    /// Complete deterministic selection accumulated by this session.
    pub selected: Vec<TransactionPackingSelection>,
    /// Queue/cache effects in original per-estimate application order.
    pub effects: Vec<TransactionPackingEffect>,
    /// Demotions acknowledged by the bridge during earlier steps of this session.
    pub acknowledged_demotions: Vec<H256>,
    /// Whether planner policy stopped before exhausting the candidate snapshot.
    pub stopped: bool,
}

struct TransactionPackingSession {
    owner: TransactionPackingOwner,
    planner: TransactionPackingPlanner,
    proposal_period: u64,
    estimate_gas_limit: u64,
    last_block_number: u64,
    total_shards: u16,
    node_shard: u16,
    shard_period_interval: u64,
    candidates: Vec<TransactionPackingCandidate>,
    next_index: usize,
    current: Option<TransactionPackingCandidate>,
    pending: Vec<TransactionPackingCandidate>,
    pending_index: usize,
    selected: Vec<TransactionPackingSelection>,
    effects: Vec<TransactionPackingEffect>,
    acknowledged_demotions: Vec<H256>,
    pending_demotion_acknowledgements: Vec<H256>,
    stopped: bool,
}

/// Mutex-owning native transaction packing protocol service.
///
/// At most one compatibility or DAG-proposer session may be active. State and
/// its guard remain private; external execution crosses only typed requests and
/// effects. Poisoned locking and protocol mismatches fail with stable error
/// identifiers, while count/hash mismatches retain the matching active session.
pub struct TransactionPackingService {
    session: Mutex<Option<TransactionPackingSession>>,
}

impl Default for TransactionPackingService {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionPackingService {
    /// Creates an idle packing owner.
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, Option<TransactionPackingSession>>> {
        self.session
            .lock()
            .map_err(|_| anyhow!(PACKING_LOCK_POISONED))
    }

    /// Returns the exact candidate snapshot limit for the supplied planner inputs.
    pub fn candidate_limit(
        weight_limit: u64,
        min_transaction_gas: u64,
        total_shards: u16,
        node_shard: u16,
        shard_period_interval: u64,
    ) -> Result<u64> {
        ensure!(total_shards != 0, "TM_RUNTIME_PACK_TOTAL_SHARDS_ZERO");
        ensure!(
            node_shard < total_shards,
            "TM_RUNTIME_PACK_NODE_SHARD_OUT_OF_RANGE"
        );
        ensure!(
            total_shards <= 1 || shard_period_interval != 0,
            "TM_RUNTIME_PACK_SHARD_INTERVAL_ZERO"
        );
        Ok(
            TransactionPackingPlanner::new(weight_limit, min_transaction_gas)?
                .max_candidate_count(),
        )
    }

    /// Starts an owner-bound session and processes declared/cached gas facts.
    ///
    /// The session remains active only when external estimates are requested.
    pub fn prepare(&self, request: TransactionPackingRequest) -> Result<TransactionPackingStep> {
        let mut guard = self.lock()?;
        ensure!(guard.is_none(), "TM_RUNTIME_PACK_SESSION_ALREADY_ACTIVE");
        validate_shards(&request)?;
        let mut session = TransactionPackingSession {
            owner: request.owner,
            planner: TransactionPackingPlanner::new(
                request.weight_limit,
                request.min_transaction_gas,
            )?,
            proposal_period: request.proposal_period,
            estimate_gas_limit: request.estimate_gas_limit,
            last_block_number: request.last_block_number,
            total_shards: request.total_shards,
            node_shard: request.node_shard,
            shard_period_interval: request.shard_period_interval,
            candidates: request.candidates,
            next_index: 0,
            current: None,
            pending: Vec::new(),
            pending_index: 0,
            selected: Vec::new(),
            effects: Vec::new(),
            acknowledged_demotions: Vec::new(),
            pending_demotion_acknowledgements: Vec::new(),
            stopped: false,
        };
        let mut requests = Vec::new();
        while let Some(candidate) = next_candidate(&mut session)? {
            let last_block_number = session.last_block_number;
            if candidate.entry.gas <= session.estimate_gas_limit {
                let declared_gas = candidate.entry.gas;
                record_estimate(
                    &mut session,
                    candidate.entry.hash,
                    declared_gas,
                    last_block_number,
                    Vec::new(),
                )?;
            } else if let Some(gas_used) = candidate.cached_gas_used {
                record_estimate(
                    &mut session,
                    candidate.entry.hash,
                    gas_used,
                    last_block_number,
                    Vec::new(),
                )?;
            } else {
                requests.push(TransactionPackingEstimateRequest {
                    entry: candidate.entry.clone(),
                });
                session.pending.push(candidate);
            }
            if session.stopped {
                break;
            }
        }
        let mut step = session_step(&session);
        step.request_estimates = requests;
        if step.request_estimates.is_empty() {
            *guard = None;
        } else {
            session.pending_demotion_acknowledgements = step
                .effects
                .iter()
                .filter_map(|effect| match effect {
                    TransactionPackingEffect::Demote(intent) => Some(intent.hash),
                    TransactionPackingEffect::CacheInsert(_) => None,
                })
                .collect();
            session.effects.clear();
            *guard = Some(session);
        }
        Ok(step)
    }

    /// Completes the matching session with an exact ordered estimate batch.
    pub fn finalize(
        &self,
        owner: TransactionPackingOwner,
        inputs: Vec<TransactionPackingEstimate>,
    ) -> Result<TransactionPackingStep> {
        let mut guard = self.lock()?;
        let active_owner = guard
            .as_ref()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?
            .owner;
        ensure!(
            active_owner == owner,
            "TM_RUNTIME_PACK_SESSION_OWNER_MISMATCH"
        );
        ensure!(
            guard
                .as_ref()
                .is_some_and(|session| session.pending_demotion_acknowledgements.is_empty()),
            "TM_RUNTIME_PACK_DEMOTION_ACK_PENDING"
        );
        let mut session = guard.take().context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        if inputs.len() != session.pending.len() {
            let expected = session.pending.len();
            *guard = Some(session);
            bail!(
                "TM_RUNTIME_PACK_FINALIZE_INPUT_COUNT_MISMATCH: expected {} received {}",
                expected,
                inputs.len()
            );
        }
        for input in inputs {
            let candidate = session
                .pending
                .get(session.pending_index)
                .cloned()
                .context("TM_RUNTIME_PACK_FINALIZE_INPUT_MISSING")?;
            if candidate.entry.hash != input.hash {
                *guard = Some(session);
                bail!("TM_RUNTIME_PACK_FINALIZE_HASH_MISMATCH");
            }
            session.current = Some(candidate);
            record_estimate(
                &mut session,
                input.hash,
                input.gas_used,
                input.last_block_number,
                input.result_rlp,
            )
            .with_context(|| {
                format!(
                    "TM_RUNTIME_PACK_FINALIZE_RECORD_ESTIMATE_{}",
                    session.pending_index
                )
            })?;
            if session.stopped {
                break;
            }
            session.pending_index += 1;
        }
        if !session.stopped && session.pending_index != session.pending.len() {
            let missing = session.pending.len() - session.pending_index;
            *guard = Some(session);
            bail!(
                "TM_RUNTIME_PACK_FINALIZE_INPUT_MISSING: expected {} estimates",
                missing
            );
        }
        let step = session_step(&session);
        *guard = None;
        Ok(step)
    }

    /// Clears only a cursor owned by `owner`.
    pub fn abort(&self, owner: TransactionPackingOwner) -> Result<bool> {
        let mut guard = self.lock()?;
        if guard.as_ref().is_some_and(|session| session.owner == owner) {
            *guard = None;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Records which requested prepare-step demotions the live queue applied.
    ///
    /// Applied hashes must be an ordered subset of the pending intent hashes.
    /// This retains only the observable summary needed by a later finalize step.
    pub fn acknowledge_demotions(
        &self,
        owner: TransactionPackingOwner,
        applied_hashes: Vec<H256>,
    ) -> Result<()> {
        let mut guard = self.lock()?;
        let session = guard
            .as_mut()
            .context("TM_RUNTIME_PACK_SESSION_NOT_ACTIVE")?;
        ensure!(
            session.owner == owner,
            "TM_RUNTIME_PACK_SESSION_OWNER_MISMATCH"
        );
        let mut applied_index = 0;
        for expected in &session.pending_demotion_acknowledgements {
            if applied_hashes.get(applied_index) == Some(expected) {
                applied_index += 1;
            }
        }
        ensure!(
            applied_index == applied_hashes.len(),
            "TM_RUNTIME_PACK_DEMOTION_ACK_MISMATCH"
        );
        session.acknowledged_demotions.extend(applied_hashes);
        session.pending_demotion_acknowledgements.clear();
        Ok(())
    }

    /// Returns whether a session is active without exposing its guard.
    pub fn is_active(&self) -> Result<bool> {
        Ok(self.lock()?.is_some())
    }
}

fn validate_shards(request: &TransactionPackingRequest) -> Result<()> {
    ensure!(
        request.total_shards != 0,
        "TM_RUNTIME_PACK_TOTAL_SHARDS_ZERO"
    );
    ensure!(
        request.node_shard < request.total_shards,
        "TM_RUNTIME_PACK_NODE_SHARD_OUT_OF_RANGE"
    );
    ensure!(
        request.total_shards <= 1 || request.shard_period_interval != 0,
        "TM_RUNTIME_PACK_SHARD_INTERVAL_ZERO"
    );
    Ok(())
}

fn next_candidate(
    session: &mut TransactionPackingSession,
) -> Result<Option<TransactionPackingCandidate>> {
    if session.stopped {
        return Ok(None);
    }
    while let Some(candidate) = session.candidates.get(session.next_index).cloned() {
        session.next_index += 1;
        if !candidate_matches_shard(&candidate, session)? {
            continue;
        }
        if session
            .planner
            .consider_candidate(TransactionPackCandidate {
                hash: candidate.entry.hash,
                declared_gas: candidate.entry.gas,
            })?
            .should_estimate
        {
            session.current = Some(candidate.clone());
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn candidate_matches_shard(
    candidate: &TransactionPackingCandidate,
    session: &TransactionPackingSession,
) -> Result<bool> {
    if session.total_shards <= 1 {
        return Ok(true);
    }
    ensure!(
        session.node_shard < session.total_shards,
        "TM_RUNTIME_PACK_SHARD_OUT_OF_RANGE"
    );
    ensure!(
        session.shard_period_interval != 0,
        "TM_RUNTIME_PACK_SHARD_INTERVAL_ZERO"
    );
    let sender = candidate.entry.sender.0;
    let prefix = u64::from_be_bytes([
        0, 0, 0, sender[0], sender[1], sender[2], sender[3], sender[4],
    ]);
    let shard = prefix.wrapping_add(session.proposal_period / session.shard_period_interval)
        % u64::from(session.total_shards);
    Ok(shard == u64::from(session.node_shard))
}

fn record_estimate(
    session: &mut TransactionPackingSession,
    hash: H256,
    gas_used: u64,
    last_block_number: u64,
    result_rlp: Vec<u8>,
) -> Result<()> {
    let candidate = session
        .current
        .take()
        .context("TM_RUNTIME_PACK_NO_ACTIVE_CANDIDATE")?;
    if candidate.entry.hash != hash {
        session.current = Some(candidate);
        bail!("TM_RUNTIME_PACK_HASH_MISMATCH");
    }
    let outcome = session
        .planner
        .record_estimate(TransactionPackEstimate { hash, gas_used })?;
    if outcome.demote_to_non_proposable {
        session.effects.push(TransactionPackingEffect::Demote(
            TransactionPackingDemotionIntent {
                hash,
                last_block_number,
            },
        ));
    }
    if !result_rlp.is_empty() {
        session.effects.push(TransactionPackingEffect::CacheInsert(
            TransactionPackingCacheIntent {
                hash,
                proposal_period: session.proposal_period,
                gas_used,
                result_rlp,
            },
        ));
    }
    if outcome.selected {
        session.selected.push(TransactionPackingSelection {
            hash,
            gas_used: outcome.gas_used,
            transaction_rlp: candidate.entry.rlp,
        });
    }
    session.stopped = outcome.stop;
    Ok(())
}

fn session_step(session: &TransactionPackingSession) -> TransactionPackingStep {
    TransactionPackingStep {
        request_estimates: Vec::new(),
        selected: session.selected.clone(),
        effects: session.effects.clone(),
        acknowledged_demotions: session.acknowledged_demotions.clone(),
        stopped: session.stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::{H160, U256};
    use std::sync::Arc;

    fn entry(hash: u64, sender: H160, gas: u64) -> TransactionQueueEntry {
        TransactionQueueEntry {
            hash: H256::from_low_u64_be(hash),
            sender,
            nonce: U256::from(hash),
            gas_price: U256::from(10),
            gas,
            data_size: 3,
            rlp: vec![hash as u8, 1, 2],
            last_block_number: 0,
        }
    }

    fn request(owner: TransactionPackingOwner) -> TransactionPackingRequest {
        TransactionPackingRequest {
            owner,
            weight_limit: 1_000_000,
            min_transaction_gas: 21_000,
            proposal_period: 8,
            estimate_gas_limit: 21_000,
            last_block_number: 99,
            total_shards: 1,
            node_shard: 0,
            shard_period_interval: 1,
            candidates: vec![TransactionPackingCandidate {
                entry: entry(1, H160::from_low_u64_be(7), 100_000),
                cached_gas_used: None,
            }],
        }
    }

    #[test]
    fn owner_mismatch_preserves_active_session_for_matching_abort() {
        let service = TransactionPackingService::new();
        let owner = TransactionPackingOwner::DagProposer(9);
        assert_eq!(
            service
                .prepare(request(owner))
                .unwrap()
                .request_estimates
                .len(),
            1
        );
        assert!(
            service
                .finalize(TransactionPackingOwner::Compatibility, Vec::new())
                .unwrap_err()
                .to_string()
                .contains("TM_RUNTIME_PACK_SESSION_OWNER_MISMATCH")
        );
        assert!(
            !service
                .abort(TransactionPackingOwner::DagProposer(8))
                .unwrap()
        );
        assert!(service.is_active().unwrap());
        assert!(service.abort(owner).unwrap());
        assert!(!service.is_active().unwrap());
    }

    #[test]
    fn exact_estimate_count_and_hash_are_enforced_in_order() {
        let service = TransactionPackingService::new();
        let owner = TransactionPackingOwner::Compatibility;
        service.prepare(request(owner)).unwrap();
        assert!(
            service
                .finalize(owner, Vec::new())
                .unwrap_err()
                .to_string()
                .contains("expected 1 received 0")
        );
        assert!(
            service
                .finalize(
                    owner,
                    vec![TransactionPackingEstimate {
                        hash: H256::from_low_u64_be(2),
                        gas_used: 50_000,
                        last_block_number: 99,
                        result_rlp: vec![1],
                    }],
                )
                .unwrap_err()
                .to_string()
                .contains("TM_RUNTIME_PACK_FINALIZE_HASH_MISMATCH")
        );
        assert!(service.abort(owner).unwrap());
    }

    #[test]
    fn cached_and_declared_gas_avoid_external_estimation() {
        let service = TransactionPackingService::new();
        let mut input = request(TransactionPackingOwner::Compatibility);
        input.candidates = vec![
            TransactionPackingCandidate {
                entry: entry(1, H160::zero(), 21_000),
                cached_gas_used: None,
            },
            TransactionPackingCandidate {
                entry: entry(2, H160::zero(), 100_000),
                cached_gas_used: Some(30_000),
            },
        ];
        let step = service.prepare(input).unwrap();
        assert!(step.request_estimates.is_empty());
        assert_eq!(step.selected.len(), 2);
        assert!(!service.is_active().unwrap());
    }

    #[test]
    fn external_result_returns_cache_and_demotion_intents() {
        let service = TransactionPackingService::new();
        let owner = TransactionPackingOwner::Compatibility;
        service.prepare(request(owner)).unwrap();
        let step = service
            .finalize(
                owner,
                vec![TransactionPackingEstimate {
                    hash: H256::from_low_u64_be(1),
                    gas_used: 20_000,
                    last_block_number: 99,
                    result_rlp: vec![7, 8],
                }],
            )
            .unwrap();
        assert!(matches!(
            step.effects.as_slice(),
            [
                TransactionPackingEffect::Demote(_),
                TransactionPackingEffect::CacheInsert(_)
            ]
        ));
        assert!(!step.stopped);
    }

    #[test]
    fn shard_selection_uses_legacy_five_byte_sender_prefix() {
        let service = TransactionPackingService::new();
        let mut input = request(TransactionPackingOwner::Compatibility);
        input.total_shards = 2;
        input.node_shard = 1;
        input.proposal_period = 0;
        input.candidates[0].entry.sender =
            H160::from_slice(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(service.prepare(input).unwrap().request_estimates.len(), 1);
    }

    #[test]
    fn request_and_selected_order_follow_candidate_snapshot() {
        let service = TransactionPackingService::new();
        let owner = TransactionPackingOwner::Compatibility;
        let mut input = request(owner);
        input.candidates = vec![
            TransactionPackingCandidate {
                entry: entry(1, H160::zero(), 100_000),
                cached_gas_used: None,
            },
            TransactionPackingCandidate {
                entry: entry(2, H160::zero(), 100_000),
                cached_gas_used: None,
            },
        ];
        let prepared = service.prepare(input).unwrap();
        assert_eq!(
            prepared
                .request_estimates
                .iter()
                .map(|request| request.entry.hash)
                .collect::<Vec<_>>(),
            vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)]
        );
        let finalized = service
            .finalize(
                owner,
                vec![
                    TransactionPackingEstimate {
                        hash: H256::from_low_u64_be(1),
                        gas_used: 30_000,
                        last_block_number: 99,
                        result_rlp: vec![1],
                    },
                    TransactionPackingEstimate {
                        hash: H256::from_low_u64_be(2),
                        gas_used: 31_000,
                        last_block_number: 99,
                        result_rlp: vec![2],
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            finalized
                .selected
                .iter()
                .map(|selected| selected.hash)
                .collect::<Vec<_>>(),
            vec![H256::from_low_u64_be(1), H256::from_low_u64_be(2)]
        );
    }

    #[test]
    fn active_session_rejects_sibling_owner() {
        let service = TransactionPackingService::new();
        service
            .prepare(request(TransactionPackingOwner::Compatibility))
            .unwrap();
        assert!(
            service
                .prepare(request(TransactionPackingOwner::DagProposer(1)))
                .unwrap_err()
                .to_string()
                .contains("TM_RUNTIME_PACK_SESSION_ALREADY_ACTIVE")
        );
    }

    #[test]
    fn poisoned_mutex_uses_stable_error_identifier() {
        let service = Arc::new(TransactionPackingService::new());
        let poisoner = Arc::clone(&service);
        assert!(
            std::thread::spawn(move || {
                let _guard = poisoner.session.lock().unwrap();
                panic!("poison packing mutex");
            })
            .join()
            .is_err()
        );
        assert_eq!(
            service.is_active().unwrap_err().to_string(),
            PACKING_LOCK_POISONED
        );
    }
}
