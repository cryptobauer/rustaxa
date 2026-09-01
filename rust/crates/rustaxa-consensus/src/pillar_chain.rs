//! Deterministic pillar-chain planning helpers.
//!
//! This module owns side-effect-free pillar-chain rules that do not require
//! storage, networking, live C++ objects, or FinalChain calls. C++ shims still
//! source DPoS vote-count snapshots, construct `PillarBlock` objects, persist
//! current/finalized pillar state, and emit events. Rust owns the deterministic
//! vote-count delta ordering, pillar-block parent/period linkage decision, and
//! storage commits for pillar rows that the Rust-mode pillar manager routes
//! through `rustaxa-storage`.

use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256};
use rustaxa_storage::Storage;
use std::collections::BTreeMap;

/// Immutable current pillar anchor used by deterministic manager decisions.
///
/// The anchor deliberately contains only the canonical block identity. Vote
/// counts remain persistence/materialization compatibility data and cannot
/// influence the current-anchor rules.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarCurrentAnchor {
    /// PBFT period summarized by the current pillar block.
    pub period: u64,
    /// Canonical hash of the current pillar block.
    pub hash: H256,
}

/// One operation requested from the current pillar anchor planner.
///
/// Variants prevent unrelated booleans from being combined into an invalid
/// request. Every operation is side-effect free and consumes the same optional
/// current anchor snapshot.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PillarCurrentAnchorDecisionRequest {
    /// Validates an optional candidate hash against the current anchor.
    ValidateCandidate { candidate_hash: Option<H256> },
    /// Selects the anchor only when it belongs to `pbft_period - 1`.
    SelectPreviousPeriod { pbft_period: u64 },
    /// Selects the anchor when restart post-processing is due.
    RestartPostProcessing {
        pbft_period: u64,
        pillar_blocks_interval: u64,
    },
}

/// Stable result status for a current pillar anchor decision.
///
/// Numeric values are part of the CXX bridge contract and must remain stable.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PillarCurrentAnchorDecisionStatus {
    Selected,
    MissingCurrentAnchor,
    MissingCandidate,
    CandidateHashMismatch,
    PbftPeriodUnderflow,
    CurrentPeriodMismatch,
    InvalidInterval,
    IntervalOverflow,
    RestartNotDue,
}

impl PillarCurrentAnchorDecisionStatus {
    /// Returns the stable CXX status code.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Selected => 0,
            Self::MissingCurrentAnchor => 1,
            Self::MissingCandidate => 2,
            Self::CandidateHashMismatch => 3,
            Self::PbftPeriodUnderflow => 4,
            Self::CurrentPeriodMismatch => 5,
            Self::InvalidInterval => 6,
            Self::IntervalOverflow => 7,
            Self::RestartNotDue => 8,
        }
    }
}

/// Deterministic current pillar anchor selection result.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarCurrentAnchorDecisionPlan {
    /// First terminal status reached by the selected operation.
    pub status: PillarCurrentAnchorDecisionStatus,
    /// Whether the current anchor was selected for the requested operation.
    pub selected: bool,
}

/// Computes the strict-majority pillar consensus threshold.
///
/// The formula is `total_vote_count / 2 + 1`. Division occurs before addition,
/// so every `u64` input, including `u64::MAX`, is representable without
/// saturation or wrapping.
pub fn plan_pillar_consensus_threshold(total_vote_count: u64) -> u64 {
    total_vote_count / 2 + 1
}

/// Plans one deterministic operation against the current pillar anchor.
///
/// Missing current state always takes precedence. Candidate validation then
/// distinguishes a missing candidate from a mismatch. Previous-period
/// selection uses checked subtraction, while restart selection rejects a zero
/// interval and uses checked addition before comparing the due period.
pub fn plan_pillar_current_anchor_decision(
    current_anchor: Option<PillarCurrentAnchor>,
    request: PillarCurrentAnchorDecisionRequest,
) -> PillarCurrentAnchorDecisionPlan {
    let Some(current_anchor) = current_anchor else {
        return current_anchor_plan(PillarCurrentAnchorDecisionStatus::MissingCurrentAnchor);
    };

    let status = match request {
        PillarCurrentAnchorDecisionRequest::ValidateCandidate { candidate_hash } => {
            let Some(candidate_hash) = candidate_hash else {
                return current_anchor_plan(PillarCurrentAnchorDecisionStatus::MissingCandidate);
            };
            if candidate_hash == current_anchor.hash {
                PillarCurrentAnchorDecisionStatus::Selected
            } else {
                PillarCurrentAnchorDecisionStatus::CandidateHashMismatch
            }
        }
        PillarCurrentAnchorDecisionRequest::SelectPreviousPeriod { pbft_period } => {
            let Some(expected_period) = pbft_period.checked_sub(1) else {
                return current_anchor_plan(PillarCurrentAnchorDecisionStatus::PbftPeriodUnderflow);
            };
            if current_anchor.period == expected_period {
                PillarCurrentAnchorDecisionStatus::Selected
            } else {
                PillarCurrentAnchorDecisionStatus::CurrentPeriodMismatch
            }
        }
        PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
            pbft_period,
            pillar_blocks_interval,
        } => {
            if pillar_blocks_interval == 0 {
                return current_anchor_plan(PillarCurrentAnchorDecisionStatus::InvalidInterval);
            }
            let Some(due_period) = current_anchor.period.checked_add(pillar_blocks_interval) else {
                return current_anchor_plan(PillarCurrentAnchorDecisionStatus::IntervalOverflow);
            };
            if pbft_period == due_period {
                PillarCurrentAnchorDecisionStatus::Selected
            } else {
                PillarCurrentAnchorDecisionStatus::RestartNotDue
            }
        }
    };

    current_anchor_plan(status)
}

fn current_anchor_plan(
    status: PillarCurrentAnchorDecisionStatus,
) -> PillarCurrentAnchorDecisionPlan {
    PillarCurrentAnchorDecisionPlan {
        status,
        selected: status == PillarCurrentAnchorDecisionStatus::Selected,
    }
}

/// Validator vote-count snapshot fact supplied by the C++ manager shim.
///
/// Inputs:
/// - `address` is the validator account.
/// - `vote_count` is the eligible vote count at the queried DPoS period.
///
/// Invariants:
/// - The vector order is preserved by first-pillar-block planning for byte
///   compatibility with the legacy manager's direct transform path.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarValidatorVoteCount {
    pub address: H160,
    pub vote_count: u64,
}

/// One ordered validator vote-count delta for a pillar block.
///
/// Outputs:
/// - `vote_count_change` mirrors the C++ `int32_t` pillar-block field.
///
/// Edge behavior:
/// - Deltas outside the signed 32-bit field range are rejected instead of
///   silently truncating before C++ materializes a `PillarBlock`.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarValidatorVoteCountChange {
    pub address: H160,
    pub vote_count_change: i32,
}

/// Computes legacy-compatible validator vote-count changes for a new pillar block.
///
/// Inputs:
/// - `current_vote_counts` is the latest DPoS eligible-vote snapshot.
/// - `previous_vote_counts` is the snapshot stored with the current pillar
///   block when planning a non-first pillar block. Pass an empty slice for the
///   first pillar block.
///
/// Outputs:
/// - For the first pillar block, returns every current vote count as a positive
///   change in the caller-provided order.
/// - For later pillar blocks, returns address-ordered non-zero deltas for
///   changed/new validators plus negative deltas for validators absent from the
///   current snapshot.
pub fn plan_pillar_vote_count_changes(
    current_vote_counts: &[PillarValidatorVoteCount],
    previous_vote_counts: &[PillarValidatorVoteCount],
) -> Result<Vec<PillarValidatorVoteCountChange>> {
    if previous_vote_counts.is_empty() {
        return current_vote_counts
            .iter()
            .map(|vote_count| {
                Ok(PillarValidatorVoteCountChange {
                    address: vote_count.address,
                    vote_count_change: u64_to_i32(vote_count.vote_count)?,
                })
            })
            .collect();
    }

    let current_by_address = vote_counts_by_address(current_vote_counts);
    let mut previous_by_address = vote_counts_by_address(previous_vote_counts);
    let mut changes = BTreeMap::<H160, i128>::new();

    for current in current_by_address.values() {
        match previous_by_address.remove(&current.address) {
            Some(previous) => {
                let delta = i128::from(current.vote_count) - i128::from(previous.vote_count);
                if delta != 0 {
                    changes.insert(current.address, delta);
                }
            }
            None => {
                changes.insert(current.address, i128::from(current.vote_count));
            }
        }
    }

    for (address, previous) in previous_by_address {
        changes.insert(address, -i128::from(previous.vote_count));
    }

    changes
        .into_iter()
        .map(|(address, delta)| {
            Ok(PillarValidatorVoteCountChange {
                address,
                vote_count_change: i128_to_i32(delta)?,
            })
        })
        .collect()
}

/// Pillar block linkage validation status.
///
/// These statuses are exposed as stable bridge codes for C++ logging/tests.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PillarBlockLinkageStatus {
    Valid,
    FirstPillarBlock,
    MissingLastFinalizedBlock,
    PeriodMismatch,
    PreviousHashMismatch,
    IntervalOverflow,
}

impl PillarBlockLinkageStatus {
    /// Returns the stable CXX bridge status code.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Valid => 0,
            Self::FirstPillarBlock => 1,
            Self::MissingLastFinalizedBlock => 2,
            Self::PeriodMismatch => 3,
            Self::PreviousHashMismatch => 4,
            Self::IntervalOverflow => 5,
        }
    }
}

/// Linkage facts for validating one proposed pillar block.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockLinkageFact {
    pub pillar_block_period: u64,
    pub pillar_block_previous_hash: H256,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
    pub last_finalized_period: Option<u64>,
    pub last_finalized_hash: Option<H256>,
}

/// Result of validating pillar-block parent linkage.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockLinkagePlan {
    pub status: PillarBlockLinkageStatus,
    pub valid: bool,
    pub expected_previous_period: u64,
}

/// Typed facts needed to plan the materialized shell of a new pillar block.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockCreationFact {
    pub pillar_block_period: u64,
    pub state_root: H256,
    pub bridge_root: H256,
    pub bridge_epoch: H256,
    pub first_pillar_block_period: u64,
    pub pillar_blocks_interval: u64,
    pub last_finalized_period: Option<u64>,
    pub last_finalized_hash: Option<H256>,
}

/// Deterministic plan for the C++ pillar-block materialization boundary.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockCreationPlan {
    pub status: PillarBlockLinkageStatus,
    pub valid: bool,
    pub expected_previous_period: u64,
    pub previous_pillar_block_hash: H256,
    pub state_root: H256,
    pub bridge_root: H256,
    pub bridge_epoch: H256,
}

/// Pillar-block finalization preflight status.
///
/// These statuses are exposed as stable bridge codes so the C++ executor can
/// log and apply effects without re-owning the finalization decision.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PillarBlockFinalizationStatus {
    Ready,
    MissingCurrentBlock,
    CurrentBlockHashMismatch,
    MissingVotes,
    AlreadyFinalized,
}

impl PillarBlockFinalizationStatus {
    /// Returns the stable CXX bridge status code.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Ready => 0,
            Self::MissingCurrentBlock => 1,
            Self::CurrentBlockHashMismatch => 2,
            Self::MissingVotes => 3,
            Self::AlreadyFinalized => 4,
        }
    }
}

/// Compact facts for planning a pillar-block finalization attempt.
///
/// Inputs:
/// - `requested_pillar_block_hash` is the hash requested by the PBFT/finalizer
///   boundary.
/// - `has_current_pillar_block`, `current_period`, and `current_hash` describe
///   the current local pillar block without transferring a live C++ object.
/// - `threshold_met`, `block_weight`, `selected_weight`, and
///   `selected_vote_count` are Rust pillar-vote lookup facts for
///   `current_period + 1` and the requested hash.
/// - `has_last_finalized_pillar_block` and `last_finalized_hash` describe the
///   compact latest-finalized sidecar.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockFinalizationFact {
    pub requested_pillar_block_hash: H256,
    pub has_current_pillar_block: bool,
    pub current_period: u64,
    pub current_hash: H256,
    pub threshold_met: bool,
    pub block_weight: u64,
    pub selected_weight: u64,
    pub selected_vote_count: u64,
    pub has_last_finalized_pillar_block: bool,
    pub last_finalized_hash: H256,
}

/// Deterministic plan for the C++ pillar-finalization executor.
///
/// Outputs:
/// - `return_votes` tells C++ whether the already-fetched vote payloads should
///   be returned to the caller.
/// - `should_request_votes`, `should_persist`, and `should_emit` are executor
///   effects. Rust chooses them; C++ performs the network/storage/event work.
/// - `current_period` echoes the compact current-block period used for storage
///   and cleanup in the ready path.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct PillarBlockFinalizationPlan {
    pub status: PillarBlockFinalizationStatus,
    pub return_votes: bool,
    pub should_request_votes: bool,
    /// Vote period selected for the missing-votes network effect.
    ///
    /// `None` suppresses the effect, including when `current_period + 1`
    /// overflows.
    pub request_votes_period: Option<u64>,
    pub should_persist: bool,
    pub should_emit: bool,
    pub current_period: u64,
    pub block_weight: u64,
    pub selected_weight: u64,
    pub selected_vote_count: u64,
}

/// Validates pillar-block period and parent-hash linkage.
///
/// Behavior mirrors the legacy manager:
/// - The first configured pillar block is always valid.
/// - Other blocks require a last finalized pillar block at
///   `period - pillar_blocks_interval` and a matching previous hash.
pub fn plan_pillar_block_linkage(fact: PillarBlockLinkageFact) -> Result<PillarBlockLinkagePlan> {
    ensure!(
        fact.pillar_blocks_interval > 0,
        "pillar blocks interval must be greater than zero"
    );
    ensure!(
        fact.last_finalized_period.is_some() == fact.last_finalized_hash.is_some(),
        "last finalized period/hash options must be provided together"
    );

    if fact.pillar_block_period == fact.first_pillar_block_period {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::FirstPillarBlock,
            valid: true,
            expected_previous_period: 0,
        });
    }

    let Some(last_period) = fact.last_finalized_period else {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::MissingLastFinalizedBlock,
            valid: false,
            expected_previous_period: 0,
        });
    };
    let Some(last_hash) = fact.last_finalized_hash else {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::MissingLastFinalizedBlock,
            valid: false,
            expected_previous_period: last_period,
        });
    };

    let Some(expected_period) = last_period.checked_add(fact.pillar_blocks_interval) else {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::IntervalOverflow,
            valid: false,
            expected_previous_period: last_period,
        });
    };

    if fact.pillar_block_period != expected_period {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::PeriodMismatch,
            valid: false,
            expected_previous_period: expected_period,
        });
    }

    if fact.pillar_block_previous_hash != last_hash {
        return Ok(PillarBlockLinkagePlan {
            status: PillarBlockLinkageStatus::PreviousHashMismatch,
            valid: false,
            expected_previous_period: expected_period,
        });
    }

    Ok(PillarBlockLinkagePlan {
        status: PillarBlockLinkageStatus::Valid,
        valid: true,
        expected_previous_period: expected_period,
    })
}

/// Plans one pillar-block finalization attempt from compact manager facts.
///
/// Behavior mirrors the legacy manager while keeping the deterministic branch
/// ordering in Rust:
/// - Missing current block and current-hash mismatch reject without effects.
/// - Missing threshold votes requests the vote bundle and rejects.
/// - Already-finalized blocks return the selected votes without re-persisting.
/// - Ready blocks return votes and request storage, cleanup, and event effects.
pub fn plan_pillar_block_finalization(
    fact: PillarBlockFinalizationFact,
) -> PillarBlockFinalizationPlan {
    if !fact.has_current_pillar_block {
        return PillarBlockFinalizationPlan {
            status: PillarBlockFinalizationStatus::MissingCurrentBlock,
            return_votes: false,
            should_request_votes: false,
            request_votes_period: None,
            should_persist: false,
            should_emit: false,
            current_period: fact.current_period,
            block_weight: fact.block_weight,
            selected_weight: fact.selected_weight,
            selected_vote_count: fact.selected_vote_count,
        };
    }

    if fact.current_hash != fact.requested_pillar_block_hash {
        return PillarBlockFinalizationPlan {
            status: PillarBlockFinalizationStatus::CurrentBlockHashMismatch,
            return_votes: false,
            should_request_votes: false,
            request_votes_period: None,
            should_persist: false,
            should_emit: false,
            current_period: fact.current_period,
            block_weight: fact.block_weight,
            selected_weight: fact.selected_weight,
            selected_vote_count: fact.selected_vote_count,
        };
    }

    if fact.has_last_finalized_pillar_block
        && fact.last_finalized_hash == fact.requested_pillar_block_hash
    {
        return PillarBlockFinalizationPlan {
            status: PillarBlockFinalizationStatus::AlreadyFinalized,
            return_votes: true,
            should_request_votes: false,
            request_votes_period: None,
            should_persist: false,
            should_emit: false,
            current_period: fact.current_period,
            block_weight: fact.block_weight,
            selected_weight: fact.selected_weight,
            selected_vote_count: fact.selected_vote_count,
        };
    }

    if !fact.threshold_met || fact.selected_vote_count == 0 {
        let request_votes_period = fact.current_period.checked_add(1);
        return PillarBlockFinalizationPlan {
            status: PillarBlockFinalizationStatus::MissingVotes,
            return_votes: false,
            should_request_votes: request_votes_period.is_some(),
            request_votes_period,
            should_persist: false,
            should_emit: false,
            current_period: fact.current_period,
            block_weight: fact.block_weight,
            selected_weight: fact.selected_weight,
            selected_vote_count: fact.selected_vote_count,
        };
    }

    PillarBlockFinalizationPlan {
        status: PillarBlockFinalizationStatus::Ready,
        return_votes: true,
        should_request_votes: false,
        request_votes_period: None,
        should_persist: true,
        should_emit: true,
        current_period: fact.current_period,
        block_weight: fact.block_weight,
        selected_weight: fact.selected_weight,
        selected_vote_count: fact.selected_vote_count,
    }
}

/// Plans the deterministic shell fields for a new pillar block.
///
/// Inputs:
/// - `fact`: pillar period/config, finalized parent context, final-chain state
///   root, and bridge root/epoch facts supplied by the boundary that still owns
///   FinalChain/EVM calls.
///
/// Outputs:
/// - Returns the parent hash, state root, bridge root, and bridge epoch that
///   C++ should use when materializing the temporary `PillarBlock` object.
/// - Returns an explicit linkage status when the candidate period cannot follow
///   the finalized pillar context.
///
/// Invariants and edge behavior:
/// - Bridge root and epoch are consumed as typed facts here so C++ does not pass
///   them ad hoc directly into pillar-block construction.
/// - The first pillar block always uses the null previous-pillar hash.
/// - Non-first blocks use the last finalized pillar hash and validate the
///   expected period interval before allowing materialization.
pub fn plan_pillar_block_creation(
    fact: PillarBlockCreationFact,
) -> Result<PillarBlockCreationPlan> {
    ensure!(
        fact.last_finalized_period.is_some() == fact.last_finalized_hash.is_some(),
        "last finalized period/hash options must be provided together"
    );

    let previous_pillar_block_hash = if fact.pillar_block_period == fact.first_pillar_block_period {
        H256::zero()
    } else {
        fact.last_finalized_hash.unwrap_or_else(H256::zero)
    };

    let linkage = plan_pillar_block_linkage(PillarBlockLinkageFact {
        pillar_block_period: fact.pillar_block_period,
        pillar_block_previous_hash: previous_pillar_block_hash,
        first_pillar_block_period: fact.first_pillar_block_period,
        pillar_blocks_interval: fact.pillar_blocks_interval,
        last_finalized_period: fact.last_finalized_period,
        last_finalized_hash: fact.last_finalized_hash,
    })?;

    Ok(PillarBlockCreationPlan {
        status: linkage.status,
        valid: linkage.valid,
        expected_previous_period: linkage.expected_previous_period,
        previous_pillar_block_hash,
        state_root: fact.state_root,
        bridge_root: fact.bridge_root,
        bridge_epoch: fact.bridge_epoch,
    })
}

/// Persists the current pillar block sidecar data through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `data_rlp`: legacy-compatible `CurrentPillarBlockDataDb` RLP bytes
///   produced by the current C++ materialization boundary.
///
/// Outputs:
/// - Writes the current-pillar singleton row.
///
/// Invariants and edge behavior:
/// - This helper owns only the durable storage row. C++ still owns
///   `PillarBlock` materialization and live manager mirrors for now.
/// - Empty payloads are rejected to avoid replacing a live current-pillar row
///   with an undecodable value.
pub fn save_current_pillar_block_data_storage(storage: &Storage, data_rlp: &[u8]) -> Result<()> {
    ensure!(
        !data_rlp.is_empty(),
        "PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD"
    );
    storage
        .pillar()
        .write_current_data(data_rlp)
        .context("PILLAR_CURRENT_BLOCK_DATA_WRITE")
}

/// Persists the local node's own pillar-block vote through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `vote_rlp`: legacy-compatible `PillarVote` RLP bytes.
///
/// Outputs:
/// - Writes the current own-pillar-vote singleton row.
///
/// Invariants and edge behavior:
/// - Vote signing, validation, gossip, and live vote aggregation remain C++
///   executor boundaries for this slice.
/// - Empty payloads are rejected because the row is later decoded as a concrete
///   `PillarVote`.
pub fn save_own_pillar_block_vote_storage(storage: &Storage, vote_rlp: &[u8]) -> Result<()> {
    ensure!(!vote_rlp.is_empty(), "PILLAR_OWN_VOTE_EMPTY_PAYLOAD");
    storage
        .pillar()
        .write_own_vote(vote_rlp)
        .context("PILLAR_OWN_VOTE_WRITE")
}

/// Persists one finalized pillar block through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `period`: pillar block period used as the storage key.
/// - `pillar_block_rlp`: canonical pillar-block RLP bytes from the C++
///   materialized block.
///
/// Outputs:
/// - Writes the finalized pillar-block row for `period`.
///
/// Invariants and edge behavior:
/// - This helper owns only finalized pillar-block persistence. The native
///   pillar services own vote lookup and current/finalized state; application
///   observers own post-ack event delivery.
/// - Empty block payloads are rejected to avoid creating an undecodable pillar
///   block record.
pub fn save_finalized_pillar_block_storage(
    storage: &Storage,
    period: u64,
    pillar_block_rlp: &[u8],
) -> Result<()> {
    ensure!(
        !pillar_block_rlp.is_empty(),
        "PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD"
    );
    storage
        .pillar()
        .write(period, pillar_block_rlp)
        .context("PILLAR_FINALIZED_BLOCK_WRITE")
}

/// Loads the local node's own pillar-block vote bytes through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Returns legacy-compatible `PillarVote` RLP bytes, or an empty vector when
///   no own vote is stored.
///
/// Invariants and edge behavior:
/// - This helper owns only the storage read. Native pillar services decode and
///   validate the canonical vote bytes.
pub fn load_own_pillar_block_vote_storage(storage: &Storage) -> Result<Vec<u8>> {
    Ok(storage
        .pillar()
        .own_vote_rlp()
        .context("PILLAR_OWN_VOTE_READ")?
        .unwrap_or_default())
}

/// Loads current pillar-block sidecar bytes through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Returns legacy-compatible `CurrentPillarBlockDataDb` RLP bytes, or an
///   empty vector when no current pillar block is stored.
///
/// Invariants and edge behavior:
/// - This helper owns only the durable storage read. C++ still decodes the
///   sidecar and updates live manager mirrors in this slice.
pub fn load_current_pillar_block_data_storage(storage: &Storage) -> Result<Vec<u8>> {
    Ok(storage
        .pillar()
        .current_data_rlp()
        .context("PILLAR_CURRENT_BLOCK_DATA_READ")?
        .unwrap_or_default())
}

/// Loads the latest finalized pillar-block bytes through Rust storage.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
///
/// Outputs:
/// - Returns the newest stored pillar-block RLP bytes, or an empty vector when
///   no finalized pillar block is stored.
///
/// Invariants and edge behavior:
/// - This helper owns storage lookup ordering through `rustaxa-storage`.
/// - C++ still decodes the returned bytes into a `PillarBlock` object.
pub fn load_latest_pillar_block_storage(storage: &Storage) -> Result<Vec<u8>> {
    Ok(storage
        .pillar()
        .latest_rlp()
        .context("PILLAR_LATEST_BLOCK_READ")?
        .unwrap_or_default())
}

/// Loads finalized period-data bytes used for pillar-vote recovery.
///
/// Inputs:
/// - `storage`: native Rust storage handle.
/// - `period`: finalized PBFT period whose period-data row may contain pillar
///   votes for the previous pillar block.
///
/// Outputs:
/// - Returns raw period-data RLP bytes, or an empty vector when no row exists.
///
/// Invariants and edge behavior:
/// - This helper intentionally returns raw period data because period-data
///   decoding is still shared with other consensus paths in C++ for this slice.
pub fn load_pillar_period_data_storage(storage: &Storage, period: u64) -> Result<Vec<u8>> {
    storage
        .period()
        .data_raw(period)
        .context("PILLAR_PERIOD_DATA_READ")
}

fn vote_counts_by_address(
    vote_counts: &[PillarValidatorVoteCount],
) -> BTreeMap<H160, PillarValidatorVoteCount> {
    let mut by_address = BTreeMap::new();
    for vote_count in vote_counts {
        by_address.entry(vote_count.address).or_insert(*vote_count);
    }
    by_address
}

fn u64_to_i32(value: u64) -> Result<i32> {
    ensure!(
        value <= i32::MAX as u64,
        "pillar validator vote count change exceeds i32 range: {value}"
    );
    Ok(value as i32)
}

fn i128_to_i32(value: i128) -> Result<i32> {
    ensure!(
        value >= i128::from(i32::MIN) && value <= i128::from(i32::MAX),
        "pillar validator vote count delta exceeds i32 range: {value}"
    );
    Ok(value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_storage::{Config, Storage};
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

    fn addr(value: u64) -> H160 {
        H160::from_low_u64_be(value)
    }

    fn hash(value: u64) -> H256 {
        H256::from_low_u64_be(value)
    }

    fn vote_count(address: u64, vote_count: u64) -> PillarValidatorVoteCount {
        PillarValidatorVoteCount {
            address: addr(address),
            vote_count,
        }
    }

    #[test]
    fn pillar_storage_helpers_persist_current_vote_and_finalized_block() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pillar_storage_helpers");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            save_current_pillar_block_data_storage(&storage, &[0xC1, 0x01])
                .expect("current pillar data should persist");
            save_own_pillar_block_vote_storage(&storage, &[0xC1, 0x02])
                .expect("own pillar vote should persist");
            save_finalized_pillar_block_storage(&storage, 42, &[0xC1, 0x03])
                .expect("finalized pillar block should persist");

            assert_eq!(
                storage
                    .pillar()
                    .current_data_rlp()
                    .expect("current pillar data should load"),
                Some(vec![0xC1, 0x01]),
            );
            assert_eq!(
                storage
                    .pillar()
                    .own_vote_rlp()
                    .expect("own pillar vote should load"),
                Some(vec![0xC1, 0x02]),
            );
            assert_eq!(
                storage.pillar().rlp(42).expect("pillar block should load"),
                Some(vec![0xC1, 0x03]),
            );
            assert_eq!(
                load_current_pillar_block_data_storage(&storage)
                    .expect("current pillar data should read"),
                vec![0xC1, 0x01],
            );
            assert_eq!(
                load_own_pillar_block_vote_storage(&storage).expect("own pillar vote should read"),
                vec![0xC1, 0x02],
            );
            assert_eq!(
                load_latest_pillar_block_storage(&storage)
                    .expect("latest pillar block should read"),
                vec![0xC1, 0x03],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn pillar_storage_read_helpers_return_empty_when_missing() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pillar_storage_missing_reads");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            assert!(
                load_current_pillar_block_data_storage(&storage)
                    .expect("current pillar data should read")
                    .is_empty()
            );
            assert!(
                load_own_pillar_block_vote_storage(&storage)
                    .expect("own pillar vote should read")
                    .is_empty()
            );
            assert!(
                load_latest_pillar_block_storage(&storage)
                    .expect("latest pillar block should read")
                    .is_empty()
            );
            assert!(
                load_pillar_period_data_storage(&storage, 42)
                    .expect("period data should read")
                    .is_empty()
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn pillar_storage_helpers_reject_empty_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_consensus_pillar_storage_empty_payloads");
        {
            let storage =
                Storage::new(Config::new(temp_dir.clone())).expect("storage should initialize");

            assert!(
                save_current_pillar_block_data_storage(&storage, &[])
                    .expect_err("empty current data should reject")
                    .to_string()
                    .contains("PILLAR_CURRENT_BLOCK_DATA_EMPTY_PAYLOAD")
            );
            assert!(
                save_own_pillar_block_vote_storage(&storage, &[])
                    .expect_err("empty own vote should reject")
                    .to_string()
                    .contains("PILLAR_OWN_VOTE_EMPTY_PAYLOAD")
            );
            assert!(
                save_finalized_pillar_block_storage(&storage, 42, &[])
                    .expect_err("empty pillar block should reject")
                    .to_string()
                    .contains("PILLAR_FINALIZED_BLOCK_EMPTY_PAYLOAD")
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn vote_count_changes_preserve_first_block_order() {
        let changes = plan_pillar_vote_count_changes(
            &[vote_count(3, 30), vote_count(1, 10), vote_count(2, 20)],
            &[],
        )
        .unwrap();

        assert_eq!(
            changes,
            vec![
                PillarValidatorVoteCountChange {
                    address: addr(3),
                    vote_count_change: 30,
                },
                PillarValidatorVoteCountChange {
                    address: addr(1),
                    vote_count_change: 10,
                },
                PillarValidatorVoteCountChange {
                    address: addr(2),
                    vote_count_change: 20,
                },
            ]
        );
    }

    #[test]
    fn vote_count_changes_are_address_ordered_for_later_blocks() {
        let changes = plan_pillar_vote_count_changes(
            &[vote_count(5, 9), vote_count(2, 1), vote_count(4, 4)],
            &[vote_count(5, 3), vote_count(1, 7), vote_count(2, 2)],
        )
        .unwrap();

        assert_eq!(
            changes,
            vec![
                PillarValidatorVoteCountChange {
                    address: addr(1),
                    vote_count_change: -7,
                },
                PillarValidatorVoteCountChange {
                    address: addr(2),
                    vote_count_change: -1,
                },
                PillarValidatorVoteCountChange {
                    address: addr(4),
                    vote_count_change: 4,
                },
                PillarValidatorVoteCountChange {
                    address: addr(5),
                    vote_count_change: 6,
                },
            ]
        );
    }

    #[test]
    fn vote_count_changes_reject_out_of_range_deltas() {
        let err =
            plan_pillar_vote_count_changes(&[vote_count(1, i32::MAX as u64 + 1)], &[]).unwrap_err();
        assert!(err.to_string().contains("exceeds i32 range"));
    }

    #[test]
    fn pillar_block_linkage_accepts_first_and_valid_next_block() {
        let first = plan_pillar_block_linkage(PillarBlockLinkageFact {
            pillar_block_period: 4,
            pillar_block_previous_hash: H256::zero(),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            last_finalized_period: None,
            last_finalized_hash: None,
        })
        .unwrap();
        assert!(first.valid);
        assert_eq!(first.status, PillarBlockLinkageStatus::FirstPillarBlock);

        let next = plan_pillar_block_linkage(PillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(44),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            last_finalized_period: Some(4),
            last_finalized_hash: Some(hash(44)),
        })
        .unwrap();
        assert!(next.valid);
        assert_eq!(next.status, PillarBlockLinkageStatus::Valid);
    }

    #[test]
    fn pillar_block_linkage_reports_mismatches() {
        let wrong_period = plan_pillar_block_linkage(PillarBlockLinkageFact {
            pillar_block_period: 9,
            pillar_block_previous_hash: hash(44),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            last_finalized_period: Some(4),
            last_finalized_hash: Some(hash(44)),
        })
        .unwrap();
        assert!(!wrong_period.valid);
        assert_eq!(
            wrong_period.status,
            PillarBlockLinkageStatus::PeriodMismatch
        );
        assert_eq!(wrong_period.expected_previous_period, 8);

        let wrong_hash = plan_pillar_block_linkage(PillarBlockLinkageFact {
            pillar_block_period: 8,
            pillar_block_previous_hash: hash(45),
            first_pillar_block_period: 4,
            pillar_blocks_interval: 4,
            last_finalized_period: Some(4),
            last_finalized_hash: Some(hash(44)),
        })
        .unwrap();
        assert!(!wrong_hash.valid);
        assert_eq!(
            wrong_hash.status,
            PillarBlockLinkageStatus::PreviousHashMismatch
        );
    }

    #[test]
    fn pillar_block_creation_consumes_bridge_facts_and_parent_context() {
        let plan = plan_pillar_block_creation(PillarBlockCreationFact {
            pillar_block_period: 18,
            state_root: H256::from_low_u64_be(0xA1),
            bridge_root: H256::from_low_u64_be(0xB2),
            bridge_epoch: H256::from_low_u64_be(0xC3),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 8,
            last_finalized_period: Some(10),
            last_finalized_hash: Some(H256::from_low_u64_be(0xD4)),
        })
        .expect("creation planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, PillarBlockLinkageStatus::Valid);
        assert_eq!(plan.previous_pillar_block_hash, H256::from_low_u64_be(0xD4));
        assert_eq!(plan.state_root, H256::from_low_u64_be(0xA1));
        assert_eq!(plan.bridge_root, H256::from_low_u64_be(0xB2));
        assert_eq!(plan.bridge_epoch, H256::from_low_u64_be(0xC3));
    }

    #[test]
    fn pillar_block_creation_uses_null_parent_for_first_block() {
        let plan = plan_pillar_block_creation(PillarBlockCreationFact {
            pillar_block_period: 10,
            state_root: H256::from_low_u64_be(0xA1),
            bridge_root: H256::from_low_u64_be(0xB2),
            bridge_epoch: H256::from_low_u64_be(0xC3),
            first_pillar_block_period: 10,
            pillar_blocks_interval: 8,
            last_finalized_period: None,
            last_finalized_hash: None,
        })
        .expect("first block planning should succeed");

        assert!(plan.valid);
        assert_eq!(plan.status, PillarBlockLinkageStatus::FirstPillarBlock);
        assert_eq!(plan.previous_pillar_block_hash, H256::zero());
    }

    #[test]
    fn pillar_block_finalization_plans_executor_effects() {
        let requested = hash(10);
        let ready = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 20,
            current_hash: requested,
            threshold_met: true,
            block_weight: 7,
            selected_weight: 5,
            selected_vote_count: 3,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: H256::zero(),
        });
        assert_eq!(ready.status, PillarBlockFinalizationStatus::Ready);
        assert!(ready.return_votes);
        assert!(ready.should_persist);
        assert!(ready.should_emit);

        let missing_votes = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 20,
            current_hash: requested,
            threshold_met: false,
            block_weight: 2,
            selected_weight: 0,
            selected_vote_count: 0,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: H256::zero(),
        });
        assert_eq!(
            missing_votes.status,
            PillarBlockFinalizationStatus::MissingVotes
        );
        assert!(missing_votes.should_request_votes);
        assert!(!missing_votes.return_votes);
    }

    #[test]
    fn pillar_block_finalization_preserves_reject_and_already_finalized_paths() {
        let requested = hash(10);
        let missing_current = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: false,
            current_period: 0,
            current_hash: H256::zero(),
            threshold_met: false,
            block_weight: 0,
            selected_weight: 0,
            selected_vote_count: 0,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: H256::zero(),
        });
        assert_eq!(
            missing_current.status,
            PillarBlockFinalizationStatus::MissingCurrentBlock
        );
        assert!(!missing_current.return_votes);

        let mismatch = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 20,
            current_hash: hash(11),
            threshold_met: true,
            block_weight: 5,
            selected_weight: 5,
            selected_vote_count: 2,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: H256::zero(),
        });
        assert_eq!(
            mismatch.status,
            PillarBlockFinalizationStatus::CurrentBlockHashMismatch
        );
        assert!(!mismatch.return_votes);

        let already_finalized = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: 20,
            current_hash: requested,
            threshold_met: true,
            block_weight: 5,
            selected_weight: 5,
            selected_vote_count: 2,
            has_last_finalized_pillar_block: true,
            last_finalized_hash: requested,
        });
        assert_eq!(
            already_finalized.status,
            PillarBlockFinalizationStatus::AlreadyFinalized
        );
        assert!(already_finalized.return_votes);
        assert!(!already_finalized.should_persist);
        assert!(!already_finalized.should_emit);
    }

    #[test]
    fn current_anchor_candidate_validation_has_stable_precedence_and_codes() {
        let anchor = PillarCurrentAnchor {
            period: 40,
            hash: hash(10),
        };
        let missing = plan_pillar_current_anchor_decision(
            None,
            PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: None,
            },
        );
        assert_eq!(missing.status.as_u8(), 1);
        assert!(!missing.selected);

        let no_candidate = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: None,
            },
        );
        assert_eq!(no_candidate.status.as_u8(), 2);

        let mismatch = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: Some(hash(11)),
            },
        );
        assert_eq!(mismatch.status.as_u8(), 3);

        let matched = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::ValidateCandidate {
                candidate_hash: Some(hash(10)),
            },
        );
        assert_eq!(matched.status.as_u8(), 0);
        assert!(matched.selected);
    }

    #[test]
    fn current_anchor_previous_period_selection_checks_underflow_and_period() {
        let anchor = PillarCurrentAnchor {
            period: 40,
            hash: hash(10),
        };
        let underflow = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::SelectPreviousPeriod { pbft_period: 0 },
        );
        assert_eq!(underflow.status.as_u8(), 4);

        let mismatch = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::SelectPreviousPeriod { pbft_period: 40 },
        );
        assert_eq!(mismatch.status.as_u8(), 5);

        let selected = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::SelectPreviousPeriod { pbft_period: 41 },
        );
        assert_eq!(selected.status.as_u8(), 0);
        assert!(selected.selected);
    }

    #[test]
    fn current_anchor_restart_selection_checks_interval_due_and_overflow() {
        let anchor = PillarCurrentAnchor {
            period: 40,
            hash: hash(10),
        };
        let zero = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: 50,
                pillar_blocks_interval: 0,
            },
        );
        assert_eq!(zero.status.as_u8(), 6);

        let not_due = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: 49,
                pillar_blocks_interval: 10,
            },
        );
        assert_eq!(not_due.status.as_u8(), 8);

        let due = plan_pillar_current_anchor_decision(
            Some(anchor),
            PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: 50,
                pillar_blocks_interval: 10,
            },
        );
        assert_eq!(due.status.as_u8(), 0);
        assert!(due.selected);

        let overflow = plan_pillar_current_anchor_decision(
            Some(PillarCurrentAnchor {
                period: u64::MAX,
                hash: hash(10),
            }),
            PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
                pbft_period: u64::MAX,
                pillar_blocks_interval: 1,
            },
        );
        assert_eq!(overflow.status.as_u8(), 7);
    }

    #[test]
    fn pillar_consensus_threshold_handles_full_u64_domain() {
        assert_eq!(plan_pillar_consensus_threshold(0), 1);
        assert_eq!(plan_pillar_consensus_threshold(4), 3);
        assert_eq!(plan_pillar_consensus_threshold(5), 3);
        assert_eq!(
            plan_pillar_consensus_threshold(u64::MAX),
            (u64::MAX / 2) + 1
        );
    }

    #[test]
    fn pillar_finalization_suppresses_overflowing_vote_request_period() {
        let requested = hash(12);
        let plan = plan_pillar_block_finalization(PillarBlockFinalizationFact {
            requested_pillar_block_hash: requested,
            has_current_pillar_block: true,
            current_period: u64::MAX,
            current_hash: requested,
            threshold_met: false,
            block_weight: 0,
            selected_weight: 0,
            selected_vote_count: 0,
            has_last_finalized_pillar_block: false,
            last_finalized_hash: H256::zero(),
        });
        assert_eq!(plan.status, PillarBlockFinalizationStatus::MissingVotes);
        assert!(!plan.should_request_votes);
        assert_eq!(plan.request_votes_period, None);
    }
}
