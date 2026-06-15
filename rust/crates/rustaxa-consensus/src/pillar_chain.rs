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
/// - This helper owns only finalized pillar-block persistence. Above-threshold
///   vote lookup, event emission, and live finalized/current mirrors remain at
///   the shim boundary until the full manager executor moves to Rust.
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
}
