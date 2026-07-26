//! CXX bridge wrappers for deterministic pillar-chain planning.
//!
//! This bridge composes pillar-chain state with borrowed Rust FinalChain reads
//! and exposes stable CXX plans to the compatibility shim. Production callers
//! no longer supply Pillar-specific DPoS vote-count or eligibility facts;
//! low-level fact planners remain module-internal test seams. C++ remains
//! responsible for temporary `PillarBlock` object construction, event
//! emission, and network side effects. Rust consensus owns the pillar storage
//! writes routed through `rustaxa-storage`, including generation-checked apply
//! of a planned current block.

use crate::ffi::rustaxa_ffi::{
    PillarBlockCreationRequest as FfiPillarBlockCreationRequest,
    PillarBlockCreationWithVoteCountsPlan as FfiPillarBlockCreationWithVoteCountsPlan,
    PillarBlockLinkagePlan as FfiPillarBlockLinkagePlan,
    PillarBlockLinkageRequest as FfiPillarBlockLinkageRequest,
    PillarChainStartupBootstrap as FfiPillarChainStartupBootstrap,
    PillarCurrentAnchorDecisionRequest as FfiPillarCurrentAnchorDecisionRequest,
    PillarCurrentAnchorDecisionResult as FfiPillarCurrentAnchorDecisionResult,
    PillarValidatorVoteCount as FfiPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as FfiPillarValidatorVoteCountChange,
};
use crate::ffi::{
    BridgeFinalChain, BridgePbftService, BridgePillarChainStorage, BridgeStorage, PillarChainState,
    PillarChainStateSnapshot, SingleVotePreparationRegistry,
};
use anyhow::{anyhow, bail, ensure, Context, Result};
#[cfg(test)]
use ethereum_types::H160;
use ethereum_types::H256;
#[cfg(test)]
use rustaxa_consensus::plan_pillar_consensus_threshold as consensus_plan_pillar_consensus_threshold;
use rustaxa_consensus::{
    load_current_pillar_block_data_storage as consensus_load_current_pillar_block_data_storage,
    load_latest_pillar_block_storage as consensus_load_latest_pillar_block_storage,
    load_own_pillar_block_vote_storage as consensus_load_own_pillar_block_vote_storage,
    load_pillar_period_data_storage as consensus_load_pillar_period_data_storage,
    plan_pillar_block_creation as consensus_plan_pillar_block_creation,
    plan_pillar_block_linkage as consensus_plan_pillar_block_linkage,
    plan_pillar_current_anchor_decision as consensus_plan_pillar_current_anchor_decision,
    plan_pillar_vote_count_changes as consensus_plan_vote_count_changes,
    save_current_pillar_block_data_storage, save_finalized_pillar_block_storage,
    save_own_pillar_block_vote_storage,
    PillarBlockCreationFact as ConsensusPillarBlockCreationFact,
    PillarBlockCreationPlan as ConsensusPillarBlockCreationPlan,
    PillarBlockLinkageFact as ConsensusPillarBlockLinkageFact,
    PillarBlockLinkagePlan as ConsensusPillarBlockLinkagePlan,
    PillarCurrentAnchor as ConsensusPillarCurrentAnchor,
    PillarCurrentAnchorDecisionRequest as ConsensusPillarCurrentAnchorDecisionRequest,
    PillarValidatorVoteCount as ConsensusPillarValidatorVoteCount,
    PillarValidatorVoteCountChange as ConsensusPillarValidatorVoteCountChange,
};
use rustaxa_types::pillar::{CurrentPillarBlockDataDb, PillarBlock};
use std::sync::RwLock;

/// Creates a typed pillar-chain storage handle from the generic CXX storage
/// facade.
///
/// The returned handle clones the underlying `Arc<rustaxa_storage::Storage>`.
/// Production C++ pillar-chain code should keep this typed handle and use its
/// methods instead of retaining or passing `BridgeStorage` after construction.
pub fn create_pillar_chain_storage(storage: &BridgeStorage) -> Box<BridgePillarChainStorage> {
    Box::new(BridgePillarChainStorage {
        storage: storage.0.clone(),
    })
}

/// Creates a Rust-owned pillar-chain runtime for the C++ PillarChainManager
/// shim.
///
/// The runtime owns both the pillar-vote aggregation state and the typed
/// storage handle needed by finalization, so live pillar-manager routes do not
/// pass one bridge handle into another to execute internal consensus behavior.
/// Missing and restored-present snapshots both start at generation zero; zero
/// is a valid process-local baseline, and each successful apply increments it.
/// Malformed persisted current data makes construction fail before a runtime is
/// published.
pub(crate) fn restore_pillar_chain_state(storage: &BridgeStorage) -> Result<PillarChainState> {
    let current_data_rlp = consensus_load_current_pillar_block_data_storage(storage.0.as_ref())?;
    let latest_finalized_block_rlp =
        consensus_load_latest_pillar_block_storage(storage.0.as_ref())?;
    let current_anchor =
        decode_current_anchor_snapshot(current_data_rlp, latest_finalized_block_rlp, 0)
            .context("restore current pillar anchor snapshot")?;
    let mut runtime = PillarChainState {
        storage: storage.0.clone(),
        votes: rustaxa_consensus::PillarVotes::new(),
        current_anchor: RwLock::new(current_anchor),
        single_vote_preparations: std::sync::Mutex::new(SingleVotePreparationRegistry {
            entries: std::collections::BTreeMap::new(),
        }),
        pillar_block_finalization_preparations: std::sync::Mutex::new(
            std::collections::HashMap::new(),
        ),
        next_pillar_block_finalization_preparation_token: 0,
    };
    {
        let current_anchor = runtime
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        if let Some(latest) = &current_anchor.latest_finalized_block {
            runtime.votes.erase_votes(latest.period.saturating_add(1));
        }
    }
    Ok(runtime)
}

/// Creates a test-only ready PBFT service using the production composition.
///
/// The production constructor restores every PBFT capability, including pillar
/// state. This wrapper supplies deterministic test configuration and completes
/// the pillar bootstrap gate; it does not create a partial service topology.
#[cfg(test)]
pub(crate) fn create_pillar_test_service_from_storage(
    storage: &BridgeStorage,
) -> Result<Box<BridgePbftService>> {
    let service = crate::pbft_manager::create_pbft_service_from_storage(
        storage,
        crate::ffi::rustaxa_ffi::PbftServiceConfig {
            genesis_lambda_ms: 100,
            cacti_lambda_max_ms: 100,
            cacti_lambda_default_ms: 100,
            cacti_block: u64::MAX,
            max_exponential_lambda_ms: 60_000,
            max_steps: 13,
            deadline_ms: 400,
            polling_interval_ms: 100,
            report_malicious_behaviour: true,
            magnolia_activation_period: 0,
        },
    )?;
    service.pbft_service_complete_pillar_bootstrap()?;
    Ok(service)
}

impl BridgePillarChainStorage {
    /// Persists current pillar-block sidecar data through consensus-owned
    /// storage.
    pub fn pillar_chain_storage_apply_current_block_data(&self, data_rlp: Vec<u8>) -> Result<()> {
        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)
    }

    /// Persists this node's own pillar-block vote through consensus-owned
    /// storage.
    pub fn pillar_chain_storage_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        save_own_pillar_block_vote_storage(self.storage.as_ref(), &vote_rlp)
    }

    /// Persists a finalized pillar block through consensus-owned storage.
    pub fn pillar_chain_storage_apply_finalized_block(
        &self,
        period: u64,
        pillar_block_rlp: Vec<u8>,
    ) -> Result<()> {
        save_finalized_pillar_block_storage(self.storage.as_ref(), period, &pillar_block_rlp)
    }

    /// Loads this node's own pillar-block vote bytes, returning empty bytes when
    /// no vote is stored.
    pub fn pillar_chain_storage_load_own_vote(&self) -> Result<Vec<u8>> {
        consensus_load_own_pillar_block_vote_storage(self.storage.as_ref())
    }

    /// Loads current pillar-block sidecar bytes, returning empty bytes when
    /// missing.
    pub fn pillar_chain_storage_load_current_block_data(&self) -> Result<Vec<u8>> {
        consensus_load_current_pillar_block_data_storage(self.storage.as_ref())
    }

    /// Loads the latest finalized pillar block bytes, returning empty bytes when
    /// no finalized pillar block is stored.
    pub fn pillar_chain_storage_load_latest_block(&self) -> Result<Vec<u8>> {
        consensus_load_latest_pillar_block_storage(self.storage.as_ref())
    }

    /// Loads a finalized pillar block by period, returning empty bytes when no
    /// block is stored for that period.
    pub fn pillar_chain_storage_load_block(&self, period: u64) -> Result<Vec<u8>> {
        Ok(self.storage.pillar().rlp(period)?.unwrap_or_default())
    }
}

impl PillarChainState {
    /// Applies one block-creation result only if its sampled anchor is current.
    pub fn pillar_state_apply_planned_current_block_data(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        self.pillar_state_apply_current_block_data_inner(data_rlp, expected_anchor_generation)
    }

    fn pillar_state_apply_current_block_data_inner(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        ensure!(
            !data_rlp.is_empty(),
            "PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"
        );
        let decoded = CurrentPillarBlockDataDb::decode_rlp(&data_rlp)
            .context("decode current pillar block data before apply")?;
        ensure!(
            decoded.encode_rlp() == data_rlp,
            "current pillar block data must use canonical RLP"
        );
        let current_block_rlp = decoded.pillar_block.encode_rlp();
        let anchor = ConsensusPillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        };
        let mut snapshot = self
            .current_anchor
            .write()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        ensure!(
            snapshot.generation == expected_anchor_generation,
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );
        let generation = snapshot
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow!("current pillar anchor generation overflow"))?;

        // Keep the lock across persistence and publication so no consumer can
        // observe bytes that are not yet durable. A failed write leaves the
        // prior in-memory snapshot unchanged.
        save_current_pillar_block_data_storage(self.storage.as_ref(), &data_rlp)?;
        *snapshot = PillarChainStateSnapshot {
            anchor: Some(anchor),
            current_data_rlp: data_rlp,
            current_block_rlp,
            latest_finalized_block: snapshot.latest_finalized_block.clone(),
            latest_finalized_block_rlp: snapshot.latest_finalized_block_rlp.clone(),
            generation,
        };
        Ok(())
    }

    /// Persists this node's own pillar-block vote through the runtime-owned
    /// native Rust storage handle.
    ///
    /// Inputs:
    /// - `vote_rlp` is the canonical legacy `PillarVote` payload selected by the
    ///   temporary C++ vote materializer.
    ///
    /// Outputs:
    /// - Commits the own-vote row used for restart recovery.
    ///
    /// Invariants and edge behavior:
    /// - Empty payloads are rejected by the consensus storage helper.
    /// - Vote admission into the live runtime index remains a separate operation;
    ///   this API only owns the persistence write that used to route through the
    ///   storage-only bridge handle.
    pub fn pillar_state_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        save_own_pillar_block_vote_storage(self.storage.as_ref(), &vote_rlp)
    }

    /// Loads the durable rows required to reconstruct pillar-manager state.
    ///
    /// Inputs:
    /// - Uses the native Rust storage handle owned by this runtime; C++ does not
    ///   choose storage keys or compose a separate storage bridge handle.
    ///
    /// Outputs:
    /// - Returns this node's own vote, current pillar sidecar, and the
    ///   period-data row following the runtime-owned latest finalized block.
    /// - Missing rows are represented by empty byte vectors.
    ///
    /// Invariants and edge behavior:
    /// - When a latest block exists, Rust decodes its canonical RLP and derives
    ///   the pillar-vote recovery lookup as `latest.period + 1`.
    /// - Malformed latest-block RLP and period overflow are returned as errors,
    ///   preventing startup from silently reconstructing inconsistent state.
    /// - When no latest block exists, no period-data lookup is needed and the
    ///   corresponding output is empty.
    pub fn pillar_state_load_startup_bootstrap(&self) -> Result<FfiPillarChainStartupBootstrap> {
        let own_vote_rlp = consensus_load_own_pillar_block_vote_storage(self.storage.as_ref())?;
        let current_block_data_rlp = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .current_data_rlp
            .clone();
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let latest_pillar_votes_period_data_rlp =
            if let Some(latest_block) = &snapshot.latest_finalized_block {
                let vote_period = latest_block
                    .period
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("latest pillar block period overflow"))?;
                consensus_load_pillar_period_data_storage(self.storage.as_ref(), vote_period)?
            } else {
                Vec::new()
            };

        Ok(FfiPillarChainStartupBootstrap {
            own_vote_rlp,
            current_block_data_rlp,
            latest_pillar_votes_period_data_rlp,
        })
    }

    /// Plans one operation against the runtime-owned current anchor snapshot.
    ///
    /// The operation tag selects candidate validation, previous-period anchor
    /// selection, or restart-due selection. Unknown tags return an error. The
    /// result includes the exact anchor generation used for the decision.
    pub fn pillar_state_plan_current_anchor_decision(
        &self,
        request: FfiPillarCurrentAnchorDecisionRequest,
    ) -> Result<FfiPillarCurrentAnchorDecisionResult> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let consensus_request = match request.operation {
            0 => ConsensusPillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: request
                    .has_candidate_hash
                    .then_some(H256::from(request.candidate_hash)),
            },
            1 => ConsensusPillarCurrentAnchorDecisionRequest::SelectPreviousPeriod {
                pbft_period: request.pbft_period,
            },
            2 => ConsensusPillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: request.pbft_period,
                pillar_blocks_interval: request.pillar_blocks_interval,
            },
            operation => bail!("unknown current pillar anchor operation: {operation}"),
        };
        let plan =
            consensus_plan_pillar_current_anchor_decision(snapshot.anchor, consensus_request);
        let (has_current_anchor, current_period, current_hash) = snapshot
            .anchor
            .map(|anchor| (true, anchor.period, anchor.hash.into()))
            .unwrap_or((false, 0, [0; 32]));
        Ok(FfiPillarCurrentAnchorDecisionResult {
            status: plan.status.as_u8(),
            selected: plan.selected,
            has_current_anchor,
            current_period,
            current_hash,
            anchor_generation: snapshot.generation,
        })
    }

    /// Computes the strict-majority threshold from an external total-vote fact.
    #[cfg(test)]
    pub fn pillar_state_consensus_threshold(&self, total_vote_count: u64) -> u64 {
        consensus_plan_pillar_consensus_threshold(total_vote_count)
    }

    /// Plans pillar-block construction from external FinalChain facts and the
    /// runtime-owned current/latest pillar snapshots.
    pub fn pillar_state_plan_block_creation(
        &self,
        request: FfiPillarBlockCreationRequest,
        current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
    ) -> Result<FfiPillarBlockCreationWithVoteCountsPlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let (last_finalized_period, last_finalized_hash) = snapshot
            .latest_finalized_block
            .as_ref()
            .map(|block| (Some(block.period), Some(block.hash())))
            .unwrap_or((None, None));
        let creation_plan =
            consensus_plan_pillar_block_creation(ConsensusPillarBlockCreationFact {
                pillar_block_period: request.pillar_block_period,
                state_root: H256::from(request.state_root),
                bridge_root: H256::from(request.bridge_root),
                bridge_epoch: H256::from(request.bridge_epoch),
                first_pillar_block_period: request.first_pillar_block_period,
                pillar_blocks_interval: request.pillar_blocks_interval,
                last_finalized_period,
                last_finalized_hash,
            })?;
        let consensus_vote_counts = current_vote_counts
            .iter()
            .map(|value| ConsensusPillarValidatorVoteCount {
                address: value.address.into(),
                vote_count: value.vote_count,
            })
            .collect::<Vec<_>>();
        let previous_vote_counts =
            if request.pillar_block_period == request.first_pillar_block_period {
                Vec::new()
            } else {
                ensure!(
                    !snapshot.current_data_rlp.is_empty(),
                    "current pillar vote-count snapshot is missing"
                );
                CurrentPillarBlockDataDb::decode_rlp(&snapshot.current_data_rlp)?
                    .vote_counts
                    .into_iter()
                    .map(|vote_count| ConsensusPillarValidatorVoteCount {
                        address: vote_count.address,
                        vote_count: vote_count.vote_count,
                    })
                    .collect()
            };
        let vote_count_changes = consensus_plan_vote_count_changes(
            consensus_vote_counts.as_slice(),
            previous_vote_counts.as_slice(),
        )?
        .into_iter()
        .map(FfiPillarValidatorVoteCountChange::from)
        .collect();
        Ok(
            FfiPillarBlockCreationWithVoteCountsPlan::from_creation_plan(
                creation_plan,
                vote_count_changes,
                current_vote_counts,
                snapshot.generation,
            ),
        )
    }

    /// Validates candidate linkage against the runtime-owned latest finalized
    /// pillar block.
    pub fn pillar_state_plan_block_linkage(
        &self,
        request: FfiPillarBlockLinkageRequest,
    ) -> Result<FfiPillarBlockLinkagePlan> {
        let snapshot = self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?;
        let (last_finalized_period, last_finalized_hash) = snapshot
            .latest_finalized_block
            .as_ref()
            .map(|block| (Some(block.period), Some(block.hash())))
            .unwrap_or((None, None));
        Ok(FfiPillarBlockLinkagePlan::from(
            consensus_plan_pillar_block_linkage(ConsensusPillarBlockLinkageFact {
                pillar_block_period: request.pillar_block_period,
                pillar_block_previous_hash: H256::from(request.pillar_block_previous_hash),
                first_pillar_block_period: request.first_pillar_block_period,
                pillar_blocks_interval: request.pillar_blocks_interval,
                last_finalized_period,
                last_finalized_hash,
            })?,
        ))
    }

    /// Returns canonical latest-finalized pillar bytes solely for the public
    /// C++ compatibility getter.
    pub fn pillar_state_latest_finalized_block_rlp(&self) -> Result<Vec<u8>> {
        Ok(self
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .latest_finalized_block_rlp
            .clone())
    }
}

impl BridgePbftService {
    pub fn pbft_service_has_pillar(&self) -> bool {
        self.pillar.is_some()
    }

    pub fn pbft_service_pillar_ready(&self) -> bool {
        self.pillar.is_some() && self.pillar_readiness.is_ready()
    }

    pub fn pbft_service_complete_pillar_bootstrap(&self) -> Result<()> {
        drop(self.pillar_state(false)?);
        self.pillar_readiness.mark_ready();
        Ok(())
    }

    /// Installs test setup data through the same generation check as production.
    ///
    /// This helper is compiled only for crate tests. It samples the current
    /// anchor generation immediately before delegating to the planned apply;
    /// malformed payloads and persistence failures are returned unchanged.
    #[cfg(test)]
    pub fn pbft_service_pillar_apply_current_block_data(&self, data_rlp: Vec<u8>) -> Result<()> {
        let state = self.pillar_state(true)?;
        let expected_anchor_generation = state
            .current_anchor
            .read()
            .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
            .generation;
        state.pillar_state_apply_planned_current_block_data(data_rlp, expected_anchor_generation)
    }

    /// Publishes a block-creation payload only against its sampled generation.
    pub fn pbft_service_pillar_apply_planned_current_block_data(
        &self,
        data_rlp: Vec<u8>,
        expected_anchor_generation: u64,
    ) -> Result<()> {
        self.pillar_state(true)?
            .pillar_state_apply_planned_current_block_data(data_rlp, expected_anchor_generation)
    }

    pub fn pbft_service_pillar_apply_own_vote(&self, vote_rlp: Vec<u8>) -> Result<()> {
        self.pillar_state(true)?
            .pillar_state_apply_own_vote(vote_rlp)
    }

    pub fn pbft_service_pillar_load_startup_bootstrap(
        &self,
    ) -> Result<FfiPillarChainStartupBootstrap> {
        self.pillar_state(false)?
            .pillar_state_load_startup_bootstrap()
    }

    pub fn pbft_service_pillar_plan_current_anchor_decision(
        &self,
        request: FfiPillarCurrentAnchorDecisionRequest,
    ) -> Result<FfiPillarCurrentAnchorDecisionResult> {
        self.pillar_state(true)?
            .pillar_state_plan_current_anchor_decision(request)
    }

    #[cfg(test)]
    pub fn pbft_service_pillar_consensus_threshold(&self, total_vote_count: u64) -> Result<u64> {
        let state = self.pillar_state(true)?;
        Ok(state.pillar_state_consensus_threshold(total_vote_count))
    }

    #[cfg(test)]
    pub fn pbft_service_pillar_plan_block_creation(
        &self,
        request: FfiPillarBlockCreationRequest,
        current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
    ) -> Result<FfiPillarBlockCreationWithVoteCountsPlan> {
        self.pillar_state(true)?
            .pillar_state_plan_block_creation(request, current_vote_counts)
    }

    /// Plans a pillar block while keeping the validator snapshot query inside Rust.
    ///
    /// The pillar generation is sampled before the potentially blocking
    /// FinalChain read and verified again after reacquiring the pillar mutex.
    /// No pillar lock is held while FinalChain selects the period snapshot.
    /// `request.pillar_block_period` is the exact DPoS snapshot height. The
    /// result contains both Rust-planned deltas and the canonical current counts
    /// temporarily required by C++ persistence materialization. FinalChain
    /// errors and generation drift reject without publishing pillar state.
    pub fn pbft_service_pillar_plan_block_creation_with_final_chain(
        &self,
        final_chain: &BridgeFinalChain,
        request: FfiPillarBlockCreationRequest,
    ) -> Result<FfiPillarBlockCreationWithVoteCountsPlan> {
        let generation = {
            let state = self.pillar_state(true)?;
            let generation = state
                .current_anchor
                .read()
                .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
                .generation;
            generation
        };
        let current_vote_counts = final_chain
            .0
            .dpos_validators_eligible_vote_counts(request.pillar_block_period.into())?
            .into_iter()
            .map(|value| FfiPillarValidatorVoteCount {
                address: value.address,
                vote_count: value.vote_count,
            })
            .collect();
        let state = self.pillar_state(true)?;
        ensure!(
            state
                .current_anchor
                .read()
                .map_err(|_| anyhow!("current pillar anchor lock poisoned"))?
                .generation
                == generation,
            "PILLAR_BLOCK_CREATION_STALE_ANCHOR"
        );
        state.pillar_state_plan_block_creation(request, current_vote_counts)
    }

    pub fn pbft_service_pillar_plan_block_linkage(
        &self,
        request: FfiPillarBlockLinkageRequest,
    ) -> Result<FfiPillarBlockLinkagePlan> {
        self.pillar_state(true)?
            .pillar_state_plan_block_linkage(request)
    }

    pub fn pbft_service_pillar_latest_finalized_block_rlp(&self) -> Result<Vec<u8>> {
        self.pillar_state(true)?
            .pillar_state_latest_finalized_block_rlp()
    }
}

fn decode_current_anchor_snapshot(
    current_data_rlp: Vec<u8>,
    latest_finalized_block_rlp: Vec<u8>,
    generation: u64,
) -> Result<PillarChainStateSnapshot> {
    let latest_finalized_block = if latest_finalized_block_rlp.is_empty() {
        None
    } else {
        let block = PillarBlock::decode_rlp(&latest_finalized_block_rlp)?;
        ensure!(
            block.encode_rlp() == latest_finalized_block_rlp,
            "latest finalized pillar block must use canonical RLP"
        );
        Some(block)
    };
    if current_data_rlp.is_empty() {
        return Ok(PillarChainStateSnapshot {
            anchor: None,
            current_data_rlp,
            current_block_rlp: Vec::new(),
            latest_finalized_block,
            latest_finalized_block_rlp,
            generation,
        });
    }
    let decoded = CurrentPillarBlockDataDb::decode_rlp(&current_data_rlp)?;
    ensure!(
        decoded.encode_rlp() == current_data_rlp,
        "current pillar block data must use canonical RLP"
    );
    let current_block_rlp = decoded.pillar_block.encode_rlp();
    let snapshot = PillarChainStateSnapshot {
        anchor: Some(ConsensusPillarCurrentAnchor {
            period: decoded.pillar_block.period,
            hash: decoded.pillar_block.hash(),
        }),
        current_data_rlp,
        current_block_rlp,
        latest_finalized_block,
        latest_finalized_block_rlp,
        generation,
    };

    validate_current_latest_pillar_anchor_relationship(&snapshot)?;
    Ok(snapshot)
}

fn validate_current_latest_pillar_anchor_relationship(
    snapshot: &PillarChainStateSnapshot,
) -> Result<()> {
    let Some(current_anchor) = snapshot.anchor else {
        return Ok(());
    };
    let Some(latest_finalized_block) = &snapshot.latest_finalized_block else {
        return Ok(());
    };

    let current_period = current_anchor.period;
    let current_hash = current_anchor.hash;
    let latest_period = latest_finalized_block.period;
    let latest_hash = latest_finalized_block.hash();

    if current_period < latest_period {
        bail!("PILLAR_ANCHOR_LATEST_AHEAD_OF_CURRENT");
    }

    if current_period == latest_period && current_hash != latest_hash {
        bail!("PILLAR_ANCHOR_CURRENT_LATEST_HASH_MISMATCH");
    }

    if current_period == latest_period {
        return Ok(());
    }

    ensure!(
        decoded_current_pillar_previous_hash(snapshot)? == latest_hash,
        "PILLAR_ANCHOR_BROKEN_SUCCESSOR_PREVIOUS_HASH"
    );
    Ok(())
}

fn decoded_current_pillar_previous_hash(
    snapshot: &PillarChainStateSnapshot,
) -> Result<ethereum_types::H256> {
    if snapshot.current_data_rlp.is_empty() {
        bail!("PILLAR_ANCHOR_CURRENT_DATA_MISSING");
    }
    let decoded = CurrentPillarBlockDataDb::decode_rlp(&snapshot.current_data_rlp)?;
    Ok(decoded.pillar_block.previous_pillar_block_hash)
}

impl From<ConsensusPillarValidatorVoteCountChange> for FfiPillarValidatorVoteCountChange {
    fn from(value: ConsensusPillarValidatorVoteCountChange) -> Self {
        Self {
            address: value.address.into(),
            vote_count_change: value.vote_count_change,
        }
    }
}

impl From<ConsensusPillarBlockLinkagePlan> for FfiPillarBlockLinkagePlan {
    fn from(value: ConsensusPillarBlockLinkagePlan) -> Self {
        Self {
            status: value.status.as_u8(),
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
        }
    }
}

impl FfiPillarBlockCreationWithVoteCountsPlan {
    fn from_creation_plan(
        value: ConsensusPillarBlockCreationPlan,
        vote_count_changes: Vec<FfiPillarValidatorVoteCountChange>,
        current_vote_counts: Vec<FfiPillarValidatorVoteCount>,
        anchor_generation: u64,
    ) -> Self {
        Self {
            status: value.status as u8,
            valid: value.valid,
            expected_previous_period: value.expected_previous_period,
            previous_pillar_block_hash: value.previous_pillar_block_hash.0,
            state_root: value.state_root.0,
            bridge_root: value.bridge_root.0,
            bridge_epoch: value.bridge_epoch.0,
            vote_count_changes,
            current_vote_counts,
            anchor_generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi;
    use crate::final_chain::create_final_chain;
    use crate::storage::create_storage;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn u256_be(value: u64) -> Vec<u8> {
        ethereum_types::U256::from(value).to_big_endian().to_vec()
    }

    fn final_chain_with_validator(
        storage: &BridgeStorage,
        address: [u8; 20],
    ) -> Box<BridgeFinalChain> {
        create_final_chain(
            storage,
            0,
            0,
            Vec::new(),
            vec![rustaxa_ffi::GenesisValidator {
                address,
                owner: address,
                vrf_key: [7; 32],
                commission: 0,
                description: String::new(),
                endpoint: String::new(),
                total_stake: u256_be(5_000),
                delegations: vec![rustaxa_ffi::GenesisDelegation {
                    delegator: address,
                    stake: u256_be(5_000),
                }],
            }],
            rustaxa_ffi::GenesisDposConfig {
                eligibility_balance_threshold: u256_be(1_000),
                vote_eligibility_balance_step: u256_be(1_000),
                validator_maximum_stake: u256_be(30_000),
                minimum_deposit: Vec::new(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 0,
                dag_vdf_sortition_total_vote_count_until_period: 0,
            },
        )
        .expect("FinalChain should initialize")
    }

    fn canonical_current_data(period: u64) -> Vec<u8> {
        CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::from_low_u64_be(2),
                bridge_root: H256::from_low_u64_be(3),
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: Vec::new(),
        }
        .encode_rlp()
    }

    fn canonical_current_data_with_vote_counts(period: u64) -> Vec<u8> {
        CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::from_low_u64_be(2),
                bridge_root: H256::from_low_u64_be(3),
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: vec![
                rustaxa_types::pillar::ValidatorVoteCount {
                    address: H160::from_low_u64_be(1),
                    vote_count: 3,
                },
                rustaxa_types::pillar::ValidatorVoteCount {
                    address: H160::from_low_u64_be(2),
                    vote_count: 8,
                },
                rustaxa_types::pillar::ValidatorVoteCount {
                    address: H160::from_low_u64_be(3),
                    vote_count: 5,
                },
            ],
        }
        .encode_rlp()
    }

    #[test]
    fn full_pbft_fixture_exposes_ready_pillar_capability() {
        let temp_dir = unique_temp_dir("pillar_full_service_capability");
        let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
        let service = create_pillar_test_service_from_storage(&storage).unwrap();
        assert!(service.pbft_service_has_pillar());
        assert!(service.pbft_service_pillar_ready());
        assert_eq!(service.manager.lock().state.snapshot().period, 1);
        assert!(service.verified_votes.is_some());
        assert!(service.slashing.is_some());
        assert_eq!(
            service.pbft_service_pillar_consensus_threshold(10).unwrap(),
            6
        );
        let _ = fs::remove_dir_all(temp_dir);
    }

    fn decode_current_anchor_snapshot_for_test(
        current_data_rlp: Vec<u8>,
        latest_finalized_block_rlp: Vec<u8>,
    ) -> anyhow::Result<PillarChainStateSnapshot> {
        decode_current_anchor_snapshot(current_data_rlp, latest_finalized_block_rlp, 0)
    }

    fn canonical_current_block_with_previous(period: u64, previous: H256) -> Vec<u8> {
        CurrentPillarBlockDataDb {
            pillar_block: PillarBlock {
                period,
                state_root: H256::from_low_u64_be(period.saturating_add(2)),
                previous_pillar_block_hash: previous,
                bridge_root: H256::from_low_u64_be(period.saturating_add(3)),
                epoch: period.saturating_add(4),
                validator_vote_count_changes: Vec::new(),
            },
            vote_counts: Vec::new(),
        }
        .encode_rlp()
    }

    fn canonical_pillar_block(period: u64, previous: H256, state_root_offset: u64) -> PillarBlock {
        PillarBlock {
            period,
            state_root: H256::from_low_u64_be(state_root_offset),
            previous_pillar_block_hash: previous,
            bridge_root: H256::from_low_u64_be(period.saturating_add(state_root_offset)),
            epoch: state_root_offset,
            validator_vote_count_changes: Vec::new(),
        }
    }

    #[test]
    fn decode_current_anchor_snapshot_validates_allowed_relationships() {
        assert!(decode_current_anchor_snapshot_for_test(Vec::new(), Vec::new()).is_ok());

        assert!(
            decode_current_anchor_snapshot_for_test(canonical_current_data(41), Vec::new(),)
                .is_ok()
        );

        let exact_latest = canonical_pillar_block(41, H256::from_low_u64_be(10), 11);
        let exact_current = CurrentPillarBlockDataDb {
            pillar_block: exact_latest.clone(),
            vote_counts: Vec::new(),
        }
        .encode_rlp();
        assert!(
            decode_current_anchor_snapshot_for_test(exact_current, exact_latest.encode_rlp(),)
                .is_ok()
        );

        let latest = canonical_pillar_block(41, H256::from_low_u64_be(12), 13);
        let successor = canonical_current_block_with_previous(42, latest.hash());
        assert!(decode_current_anchor_snapshot_for_test(successor, latest.encode_rlp(),).is_ok());

        let latest_for_gap = canonical_pillar_block(4, H256::from_low_u64_be(10), 11);
        let current_with_gap = canonical_current_block_with_previous(8, latest_for_gap.hash());
        assert!(decode_current_anchor_snapshot_for_test(
            current_with_gap,
            latest_for_gap.encode_rlp(),
        )
        .is_ok());
    }

    #[test]
    fn decode_current_anchor_snapshot_rejects_invalid_latest_relationships() {
        let latest_ahead = canonical_pillar_block(42, H256::from_low_u64_be(10), 11).encode_rlp();
        assert!(decode_current_anchor_snapshot_for_test(
            canonical_current_block_with_previous(41, H256::from_low_u64_be(1)),
            latest_ahead.clone(),
        )
        .unwrap_err()
        .to_string()
        .contains("PILLAR_ANCHOR_LATEST_AHEAD_OF_CURRENT"));

        let latest_same_period =
            canonical_pillar_block(41, H256::from_low_u64_be(10), 11).encode_rlp();
        let mismatched_current =
            canonical_current_block_with_previous(41, H256::from_low_u64_be(12));
        assert!(decode_current_anchor_snapshot_for_test(
            mismatched_current,
            latest_same_period.clone(),
        )
        .unwrap_err()
        .to_string()
        .contains("PILLAR_ANCHOR_CURRENT_LATEST_HASH_MISMATCH"));

        let latest_gap = canonical_pillar_block(41, H256::from_low_u64_be(10), 11).encode_rlp();
        let successor_gap = canonical_current_block_with_previous(43, H256::from_low_u64_be(1));
        assert!(
            decode_current_anchor_snapshot_for_test(successor_gap, latest_gap.clone(),)
                .unwrap_err()
                .to_string()
                .contains("PILLAR_ANCHOR_BROKEN_SUCCESSOR_PREVIOUS_HASH")
        );

        let latest_with_previous =
            canonical_pillar_block(41, H256::from_low_u64_be(10), 11).encode_rlp();
        let successor_bad_previous =
            canonical_current_block_with_previous(42, H256::from_low_u64_be(12));
        assert!(decode_current_anchor_snapshot_for_test(
            successor_bad_previous,
            latest_with_previous,
        )
        .unwrap_err()
        .to_string()
        .contains("PILLAR_ANCHOR_BROKEN_SUCCESSOR_PREVIOUS_HASH"));
    }

    #[test]
    fn runtime_owns_latest_finalized_linkage_and_creation_facts() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_creation");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let latest = PillarBlock {
                period: 10,
                state_root: H256::from_low_u64_be(1),
                previous_pillar_block_hash: H256::from_low_u64_be(2),
                bridge_root: H256::from_low_u64_be(3),
                epoch: 4,
                validator_vote_count_changes: Vec::new(),
            };
            save_current_pillar_block_data_storage(
                storage.0.as_ref(),
                &canonical_current_data_with_vote_counts(10),
            )
            .unwrap();
            save_finalized_pillar_block_storage(
                storage.0.as_ref(),
                latest.period,
                &latest.encode_rlp(),
            )
            .unwrap();

            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let creation = runtime
                .pbft_service_pillar_plan_block_creation(
                    FfiPillarBlockCreationRequest {
                        pillar_block_period: 20,
                        state_root: H256::from_low_u64_be(0xA1).into(),
                        bridge_root: H256::from_low_u64_be(0xB2).into(),
                        bridge_epoch: H256::from_low_u64_be(0xC3).into(),
                        first_pillar_block_period: 10,
                        pillar_blocks_interval: 10,
                    },
                    vec![
                        FfiPillarValidatorVoteCount {
                            address: H160::from_low_u64_be(1).into(),
                            vote_count: 3,
                        },
                        FfiPillarValidatorVoteCount {
                            address: H160::from_low_u64_be(3).into(),
                            vote_count: 9,
                        },
                        FfiPillarValidatorVoteCount {
                            address: H160::from_low_u64_be(4).into(),
                            vote_count: 4,
                        },
                    ],
                )
                .unwrap();
            assert!(creation.valid);
            assert_eq!(creation.previous_pillar_block_hash, latest.hash().0);
            assert_eq!(creation.vote_count_changes.len(), 3);
            assert_eq!(
                creation.vote_count_changes[0].address,
                H160::from_low_u64_be(2).0
            );
            assert_eq!(creation.vote_count_changes[0].vote_count_change, -8);
            assert_eq!(
                creation.vote_count_changes[1].address,
                H160::from_low_u64_be(3).0
            );
            assert_eq!(creation.vote_count_changes[1].vote_count_change, 4);
            assert_eq!(
                creation.vote_count_changes[2].address,
                H160::from_low_u64_be(4).0
            );
            assert_eq!(creation.vote_count_changes[2].vote_count_change, 4);

            let linkage = runtime
                .pbft_service_pillar_plan_block_linkage(FfiPillarBlockLinkageRequest {
                    pillar_block_period: 20,
                    pillar_block_previous_hash: latest.hash().0,
                    first_pillar_block_period: 10,
                    pillar_blocks_interval: 10,
                })
                .unwrap();
            assert!(linkage.valid);
            let wrong_linkage = runtime
                .pbft_service_pillar_plan_block_linkage(FfiPillarBlockLinkageRequest {
                    pillar_block_period: 20,
                    pillar_block_previous_hash: H256::from_low_u64_be(999).0,
                    first_pillar_block_period: 10,
                    pillar_blocks_interval: 10,
                })
                .unwrap();
            assert!(!wrong_linkage.valid);
            assert_eq!(wrong_linkage.status, 4);
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_applies_pillar_storage_writes_through_consensus() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_helpers");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            pillar_storage
                .pillar_chain_storage_apply_current_block_data(vec![0xC1, 0x01])
                .expect("current pillar data should persist");
            pillar_storage
                .pillar_chain_storage_apply_own_vote(vec![0xC1, 0x02])
                .expect("own pillar vote should persist");
            pillar_storage
                .pillar_chain_storage_apply_finalized_block(42, vec![0xC1, 0x03])
                .expect("finalized pillar block should persist");

            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should load"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should load"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_block(42)
                    .expect("pillar block should load"),
                vec![0xC1, 0x03],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should read"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should read"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_latest_block()
                    .expect("latest pillar block should read"),
                vec![0xC1, 0x03],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_applies_manager_pillar_storage_writes() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_storage_helpers");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);
            let pillar_runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            let current_data = canonical_current_data(41);

            pillar_runtime
                .pbft_service_pillar_apply_current_block_data(current_data.clone())
                .expect("runtime current pillar data should persist");
            pillar_runtime
                .pbft_service_pillar_apply_own_vote(vec![0xC2, 0x02])
                .expect("runtime own pillar vote should persist");

            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_current_block_data()
                    .expect("current pillar data should load"),
                current_data,
            );
            assert_eq!(
                pillar_storage
                    .pillar_chain_storage_load_own_vote()
                    .expect("own pillar vote should load"),
                vec![0xC2, 0x02],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_loads_pillar_startup_bootstrap_after_restart() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_startup_bootstrap");
        let latest_block = PillarBlock {
            period: 42,
            state_root: H256::from_low_u64_be(1),
            previous_pillar_block_hash: H256::from_low_u64_be(2),
            bridge_root: H256::from_low_u64_be(3),
            epoch: 4,
            validator_vote_count_changes: Vec::new(),
        }
        .encode_rlp();
        let current_data = canonical_current_data(42);

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            runtime
                .pbft_service_pillar_apply_current_block_data(current_data.clone())
                .expect("current pillar data should persist");
            runtime
                .pbft_service_pillar_apply_own_vote(vec![0xC3, 0x02])
                .expect("own vote should persist");
            save_finalized_pillar_block_storage(storage.0.as_ref(), 42, &latest_block)
                .expect("latest pillar block should persist");
            storage
                .0
                .period()
                .write(43, &[0xC3, 0x04])
                .expect("following period data should persist");
        }

        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should reopen");
            let runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            let bootstrap = runtime
                .pbft_service_pillar_load_startup_bootstrap()
                .expect("runtime should load restart bootstrap");

            assert_eq!(bootstrap.own_vote_rlp, vec![0xC3, 0x02]);
            assert_eq!(bootstrap.current_block_data_rlp, current_data);
            assert_eq!(
                runtime
                    .pbft_service_pillar_latest_finalized_block_rlp()
                    .expect("latest finalized block should load from runtime"),
                latest_block
            );
            assert_eq!(
                bootstrap.latest_pillar_votes_period_data_rlp,
                vec![0xC3, 0x04]
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_startup_bootstrap_is_empty_for_new_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_empty_bootstrap");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            let bootstrap = runtime
                .pbft_service_pillar_load_startup_bootstrap()
                .expect("empty bootstrap should load");

            assert!(bootstrap.own_vote_rlp.is_empty());
            assert!(bootstrap.current_block_data_rlp.is_empty());
            assert!(runtime
                .pbft_service_pillar_latest_finalized_block_rlp()
                .expect("empty latest finalized block should load")
                .is_empty());
            assert!(bootstrap.latest_pillar_votes_period_data_rlp.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn runtime_restores_anchor_and_preserves_decisions_across_restart() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_anchor_restart");
        let current_data = canonical_current_data(41);
        let before;
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            runtime
                .pbft_service_pillar_apply_current_block_data(current_data.clone())
                .unwrap();
            before = runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert!(before.selected);
            assert_eq!(before.anchor_generation, 1);
        }
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let after = runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(after.status, before.status);
            assert_eq!(after.selected, before.selected);
            assert_eq!(after.current_period, before.current_period);
            assert_eq!(after.current_hash, before.current_hash);
            assert_eq!(after.anchor_generation, 0);
            assert_eq!(
                runtime
                    .pbft_service_pillar_load_startup_bootstrap()
                    .unwrap()
                    .current_block_data_rlp,
                current_data
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn malformed_persisted_current_data_rejects_runtime_factory() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_malformed_current_restore");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            save_current_pillar_block_data_storage(storage.0.as_ref(), &[0xC1, 0x01]).unwrap();
            let error = match create_pillar_test_service_from_storage(&storage) {
                Ok(_) => panic!("malformed current data must reject construction"),
                Err(error) => error,
            };
            assert!(error
                .to_string()
                .contains("restore current pillar anchor snapshot"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn malformed_current_apply_leaves_storage_and_snapshot_unchanged() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_malformed_current_apply");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let current_data = canonical_current_data(41);
            runtime
                .pbft_service_pillar_apply_current_block_data(current_data.clone())
                .unwrap();
            let before = runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();

            assert!(runtime
                .pbft_service_pillar_apply_current_block_data(vec![0xC1, 0x01])
                .is_err());
            let after = runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 1,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 42,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(after.anchor_generation, before.anchor_generation);
            assert_eq!(after.current_hash, before.current_hash);
            assert_eq!(
                consensus_load_current_pillar_block_data_storage(storage.0.as_ref()).unwrap(),
                current_data
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn current_anchor_bridge_rejects_unknown_tag_and_maps_threshold() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_anchor_tags");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let runtime = create_pillar_test_service_from_storage(&storage).unwrap();
            let missing = runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 0,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 0,
                        pillar_blocks_interval: 0,
                    },
                )
                .unwrap();
            assert_eq!(missing.status, 1);
            assert!(!missing.has_current_anchor);
            assert!(runtime
                .pbft_service_pillar_plan_current_anchor_decision(
                    FfiPillarCurrentAnchorDecisionRequest {
                        operation: 99,
                        has_candidate_hash: false,
                        candidate_hash: [0; 32],
                        pbft_period: 0,
                        pillar_blocks_interval: 0,
                    },
                )
                .is_err());
            assert_eq!(
                runtime.pbft_service_pillar_consensus_threshold(0).unwrap(),
                1
            );
            assert_eq!(
                runtime
                    .pbft_service_pillar_consensus_threshold(u64::MAX)
                    .unwrap(),
                (u64::MAX / 2) + 1
            );
        }
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_runtime_factory_rejects_malformed_latest_block() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_runtime_malformed_bootstrap");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            save_finalized_pillar_block_storage(storage.0.as_ref(), 42, &[0xC1, 0x01])
                .expect("malformed latest bytes should persist opaquely");
            let error = match create_pillar_test_service_from_storage(&storage) {
                Ok(_) => panic!("malformed latest block should reject runtime construction"),
                Err(error) => error,
            };
            assert!(format!("{error:#}").contains("six items"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_pillar_storage_reads_return_empty_when_missing() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_missing_reads");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            assert!(pillar_storage
                .pillar_chain_storage_load_current_block_data()
                .expect("current pillar data should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_own_vote()
                .expect("own pillar vote should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_latest_block()
                .expect("latest pillar block should read")
                .is_empty());
            assert!(pillar_storage
                .pillar_chain_storage_load_block(42)
                .expect("pillar block should read")
                .is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn bridge_rejects_empty_pillar_storage_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pillar_storage_empty_payloads");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pillar_storage = create_pillar_chain_storage(&storage);

            assert!(pillar_storage
                .pillar_chain_storage_apply_current_block_data(Vec::new())
                .expect_err("empty current data should reject")
                .to_string()
                .contains("PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"));
            assert!(pillar_storage
                .pillar_chain_storage_apply_own_vote(Vec::new())
                .expect_err("empty own vote should reject")
                .to_string()
                .contains("PILLAR_OWN_VOTE_EMPTY_PAYLOAD"));
            let pillar_runtime = create_pillar_test_service_from_storage(&storage)
                .expect("pillar runtime should initialize");
            assert!(pillar_runtime
                .pbft_service_pillar_apply_current_block_data(Vec::new())
                .expect_err("empty runtime current data should reject")
                .to_string()
                .contains("PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"));
            assert!(pillar_runtime
                .pbft_service_pillar_apply_own_vote(Vec::new())
                .expect_err("empty runtime own vote should reject")
                .to_string()
                .contains("PILLAR_OWN_VOTE_EMPTY_PAYLOAD"));
            assert!(pillar_storage
                .pillar_chain_storage_apply_finalized_block(42, Vec::new())
                .expect_err("empty pillar block should reject")
                .to_string()
                .contains("PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn composed_final_chain_threshold_and_creation_keep_validator_facts_in_rust() {
        let temp_dir = unique_temp_dir("pillar_composed_final_chain");
        {
            let storage = create_storage(temp_dir.to_str().unwrap()).unwrap();
            let final_chain = final_chain_with_validator(&storage, [9; 20]);
            let service = create_pillar_test_service_from_storage(&storage).unwrap();
            service.pbft_service_complete_pillar_bootstrap().unwrap();

            let threshold = service
                .pbft_service_pillar_consensus_threshold_with_final_chain(&final_chain, 0)
                .unwrap();
            assert!(threshold.available);
            assert!(threshold.threshold > 0);

            let plan = service
                .pbft_service_pillar_plan_block_creation_with_final_chain(
                    &final_chain,
                    FfiPillarBlockCreationRequest {
                        pillar_block_period: 0,
                        state_root: [1; 32],
                        bridge_root: [2; 32],
                        bridge_epoch: [0; 32],
                        first_pillar_block_period: 0,
                        pillar_blocks_interval: 10,
                    },
                )
                .unwrap();
            assert!(plan.valid);
            assert_eq!(plan.current_vote_counts.len(), 1);
            assert_eq!(plan.current_vote_counts[0].address, [9; 20]);
            assert!(plan.current_vote_counts[0].vote_count > 0);
            assert_eq!(plan.vote_count_changes.len(), 1);

            service
                .pbft_service_pillar_apply_current_block_data(canonical_current_data(1))
                .unwrap();
            let stale = service
                .pbft_service_pillar_apply_planned_current_block_data(
                    canonical_current_data(0),
                    plan.anchor_generation,
                )
                .unwrap_err();
            assert!(format!("{stale:#}").contains("PILLAR_BLOCK_CREATION_STALE_ANCHOR"));
        }
        let _ = fs::remove_dir_all(temp_dir);
    }
}
