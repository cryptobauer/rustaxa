use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeStorage;
use crate::ffi::BridgeStorageBatch;
use anyhow::Context;
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_consensus::{
    clear_own_verified_votes as domain_clear_own_verified_votes,
    persist_pbft_vote_progress as domain_persist_pbft_vote_progress,
    remove_extra_reward_votes as domain_remove_extra_reward_votes,
    save_non_finalized_transactions as domain_save_non_finalized_transactions,
    save_own_verified_vote as domain_save_own_verified_vote, NonFinalizedTransactionStoragePayload,
    PbftTwoTPlusOneVoteBundle as DomainPbftTwoTPlusOneVoteBundle,
    PbftVotePersistenceResult as DomainPbftVotePersistenceResult, PbftVotePersistenceStatus,
    PbftVoteProgressPersistenceWrite as DomainPbftVoteProgressPersistenceWrite,
    PbftVoteStorageRecord as DomainPbftVoteStorageRecord,
};
use rustaxa_storage::Config;
use rustaxa_storage::Storage;
use rustaxa_types::pillar::RawPillarBlockData;
use std::path::PathBuf;
use std::sync::Arc;

const PILLAR_VOTES_POS_IN_PERIOD_DATA: usize = 4;

fn pbft_vote_persistence_from_domain(
    value: DomainPbftVotePersistenceResult,
) -> rustaxa_ffi::PbftVotePersistenceResult {
    rustaxa_ffi::PbftVotePersistenceResult {
        status: value.status.as_u8(),
        applied_writes: value.applied_writes,
        error_code: value.error_code,
    }
}

fn require_pbft_vote_persistence_applied(
    result: DomainPbftVotePersistenceResult,
) -> Result<(), anyhow::Error> {
    if result.status == PbftVotePersistenceStatus::Applied {
        return Ok(());
    }
    Err(anyhow::anyhow!(result.error_code))
}

fn vote_storage_record_to_domain(
    value: rustaxa_ffi::PbftVoteStorageRecord,
) -> DomainPbftVoteStorageRecord {
    DomainPbftVoteStorageRecord {
        hash: H256::from(value.hash),
        vote_rlp: value.vote_rlp,
    }
}

fn two_t_plus_one_bundle_to_domain(
    value: rustaxa_ffi::PbftTwoTPlusOneVoteBundle,
) -> DomainPbftTwoTPlusOneVoteBundle {
    DomainPbftTwoTPlusOneVoteBundle {
        kind: value.kind,
        period: value.period,
        round: value.round,
        step: value.step,
        block_hash: H256::from(value.block_hash),
        votes_bundle_rlp: value.votes_bundle_rlp,
    }
}

fn vote_progress_write_to_domain(
    value: rustaxa_ffi::PbftVoteProgressPersistenceWrite,
) -> DomainPbftVoteProgressPersistenceWrite {
    DomainPbftVoteProgressPersistenceWrite {
        extra_reward_vote: value
            .has_extra_reward_vote
            .then(|| vote_storage_record_to_domain(value.extra_reward_vote)),
        two_t_plus_one_bundle: value
            .has_two_t_plus_one_bundle
            .then(|| two_t_plus_one_bundle_to_domain(value.two_t_plus_one_bundle)),
    }
}

pub fn create_storage(path: &str) -> Result<Box<BridgeStorage>, anyhow::Error> {
    let path_buf = PathBuf::from(path);
    let config = Config::new(path_buf);
    let storage = Arc::new(Storage::new(config)?);
    Ok(Box::new(BridgeStorage(storage)))
}

/// Creates a Rust-owned storage batch for the C++ `DbStorage` shim.
///
/// The returned object owns a native `rustaxa-storage` write batch and the shared
/// storage handle needed to append and commit it. This replaces the previous
/// bridge-global integer batch registry while the public C++ `Batch&` surface is
/// still being retired.
pub fn create_storage_shim_batch(storage: &BridgeStorage) -> Box<BridgeStorageBatch> {
    Box::new(BridgeStorageBatch {
        storage: storage.0.clone(),
        batch: Some(storage.0.create_write_batch()),
    })
}

fn storage_shim_batch_mut(
    batch: &mut BridgeStorageBatch,
) -> Result<&mut rustaxa_storage::StorageWriteBatch, anyhow::Error> {
    batch
        .batch
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("storage shim batch already committed"))
}

/// Appends one raw legacy column put to a Rust-owned storage shim batch.
///
/// This is a storage-shim compatibility API, not a production consensus storage
/// API. Migrated Rust runtimes should use typed storage repositories or
/// operation-specific apply functions that own their full atomic write group.
pub fn storage_shim_batch_put(
    batch: &mut BridgeStorageBatch,
    column: u8,
    key: Vec<u8>,
    value: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let column = rustaxa_storage::Column::from_index(column)?;
    let storage = batch.storage.clone();
    storage.batch_put_raw(storage_shim_batch_mut(batch)?, column, &key, &value)
}

/// Appends one raw legacy column delete to a Rust-owned storage shim batch.
///
/// This exists only for the C++ `DbStorage` compatibility shim while remaining
/// public callers are moved to typed Rust storage paths.
pub fn storage_shim_batch_delete(
    batch: &mut BridgeStorageBatch,
    column: u8,
    key: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let column = rustaxa_storage::Column::from_index(column)?;
    let storage = batch.storage.clone();
    storage.batch_delete_raw(storage_shim_batch_mut(batch)?, column, &key)
}

/// Appends a typed status-field write to a Rust-owned storage shim batch.
///
/// This keeps the legacy C++ batch commit/drop boundary while moving the
/// status-column key/value encoding into `rustaxa-storage`.
pub fn storage_shim_save_status_field(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: u64,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .metadata()
        .write_status_field_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed sortition-params change write to a Rust-owned storage shim batch.
///
/// The payload must already be the legacy RLP bytes. The Rust metadata
/// repository owns the target column and period-key encoding.
pub fn storage_shim_save_sortition_params_change(
    batch: &mut BridgeStorageBatch,
    period: u64,
    params_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_sortition_params_change_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        &params_rlp,
    )
}

/// Appends a typed period-lambda write to a Rust-owned storage shim batch.
pub fn storage_shim_save_period_lambda(
    batch: &mut BridgeStorageBatch,
    period: u64,
    period_lambda: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_period_lambda_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        period_lambda,
    )
}

/// Appends a typed dynamic-lambda rounds-count write to a Rust-owned storage shim batch.
pub fn storage_shim_save_rounds_count_dynamic_lambda(
    batch: &mut BridgeStorageBatch,
    rounds_count: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .metadata()
        .write_rounds_count_dynamic_lambda_in_batch(storage_shim_batch_mut(batch)?, rounds_count)
}

/// Appends typed block reward statistics bytes to a Rust-owned storage shim batch.
///
/// The caller supplies legacy-compatible encoded block-stats RLP; Rust owns the
/// period-key encoding and `block_rewards_stats` column selection.
pub fn storage_shim_save_block_rewards_stats(
    batch: &mut BridgeStorageBatch,
    period: u64,
    stats_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.metadata().write_block_rewards_stats_in_batch(
        storage_shim_batch_mut(batch)?,
        period,
        &stats_rlp,
    )
}

/// Appends a typed PBFT manager numeric-field write to a Rust-owned storage shim batch.
///
/// The C++ shim supplies legacy enum discriminants and values; `rustaxa-storage`
/// owns the PBFT manager column and little-endian value encoding.
pub fn storage_shim_save_pbft_mgr_field(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: u32,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_manager_field_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed PBFT manager status write to a Rust-owned storage shim batch.
///
/// The C++ shim supplies the legacy status discriminant while Rust owns the
/// status-column key and bool encoding.
pub fn storage_shim_save_pbft_mgr_status(
    batch: &mut BridgeStorageBatch,
    field: u8,
    value: bool,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_manager_status_in_batch(storage_shim_batch_mut(batch)?, field, value)
}

/// Appends a typed cert-voted block cleanup to a Rust-owned storage shim batch.
pub fn storage_shim_remove_cert_voted_block_in_round(
    batch: &mut BridgeStorageBatch,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_cert_voted_block_in_round_in_batch(storage_shim_batch_mut(batch)?)
}

/// Appends a typed PBFT head write to a Rust-owned storage shim batch.
///
/// The head payload remains opaque legacy bytes while Rust owns the PBFT head
/// column and hash-key layout.
pub fn storage_shim_save_pbft_head(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
    head: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .write_head_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash), &head)
}

/// Appends a typed own verified vote cleanup to a Rust-owned storage shim batch.
pub fn storage_shim_remove_own_verified_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_own_verified_vote_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Appends a typed 2t+1 vote bundle replacement to a Rust-owned storage shim batch.
///
/// Rust validates the legacy vote-type discriminant and owns the delete-then-put
/// ordering inside the caller-owned batch.
pub fn storage_shim_replace_two_t_plus_one_votes(
    batch: &mut BridgeStorageBatch,
    vote_type: u8,
    votes_bundle_rlp: Vec<u8>,
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage.pbft().replace_two_t_plus_one_votes_in_batch(
        storage_shim_batch_mut(batch)?,
        vote_type,
        &votes_bundle_rlp,
    )
}

/// Appends a typed extra reward vote cleanup to a Rust-owned storage shim batch.
pub fn storage_shim_remove_extra_reward_vote(
    batch: &mut BridgeStorageBatch,
    hash: &[u8; 32],
) -> Result<(), anyhow::Error> {
    let storage = batch.storage.clone();
    storage
        .pbft()
        .remove_extra_reward_vote_in_batch(storage_shim_batch_mut(batch)?, H256::from(*hash))
}

/// Commits a Rust-owned storage shim batch and consumes it.
///
/// Dropping a `BridgeStorageBatch` without calling this method discards staged
/// writes, matching legacy dropped-batch behavior without a bridge-side batch
/// registry.
pub fn storage_shim_commit_batch(
    mut batch: Box<BridgeStorageBatch>,
    sync: bool,
) -> Result<(), anyhow::Error> {
    let storage_batch = batch
        .batch
        .take()
        .ok_or_else(|| anyhow::anyhow!("storage shim batch already committed"))?;
    batch
        .storage
        .commit_write_batch_with_sync(storage_batch, sync)
}

/// Batch-loads transaction RLP payloads by hash using Rust storage semantics shared by consensus bridges.
///
/// Inputs are canonical transaction hashes in caller-requested order. Outputs preserve
/// that order, return the original hash, mark whether a payload was found, and identify
/// whether the payload came from finalized storage. Lookup checks pending/non-finalized
/// transactions first, then finalized transaction-location metadata, including system
/// transactions. Missing hashes are returned as `found = false` rather than errors;
/// storage/codec failures are propagated with stable context labels.
pub(crate) fn transaction_rlp_lookups(
    storage: &Storage,
    hashes: Vec<H256>,
) -> Result<Vec<rustaxa_ffi::DagTransactionRlpLookup>, anyhow::Error> {
    let transaction = storage.transaction();
    let mut out = Vec::with_capacity(hashes.len());

    for hash in hashes {
        let (tx_rlp, finalized) = if let Some(tx_rlp) = transaction
            .rlp(hash)
            .context("DAG_TRANSACTION_RLP_PENDING_LOOKUP")?
        {
            (Some(tx_rlp), false)
        } else if let Some(location_rlp) = transaction
            .location_rlp(hash)
            .context("DAG_TRANSACTION_RLP_LOCATION_LOOKUP")?
        {
            let location = rlp::Rlp::new(&location_rlp);
            let period = location
                .val_at::<u64>(0)
                .context("DAG_TRANSACTION_RLP_LOCATION_PERIOD")?;
            let position = location
                .val_at::<u32>(1)
                .context("DAG_TRANSACTION_RLP_LOCATION_POSITION")?;
            let is_system = location
                .item_count()
                .context("DAG_TRANSACTION_RLP_LOCATION_SHAPE")?
                == 3
                && location
                    .val_at::<bool>(2)
                    .context("DAG_TRANSACTION_RLP_LOCATION_SYSTEM_FLAG")?;
            let tx_rlp = if is_system {
                transaction
                    .system_rlp(hash)
                    .context("DAG_TRANSACTION_RLP_SYSTEM_LOOKUP")?
            } else {
                transaction
                    .by_period_position_rlp(period, position)
                    .context("DAG_TRANSACTION_RLP_FINALIZED_LOOKUP")?
            };
            (tx_rlp, true)
        } else {
            (None, false)
        };

        out.push(rustaxa_ffi::DagTransactionRlpLookup {
            hash: hash.0,
            found: tx_rlp.is_some(),
            finalized,
            tx_rlp: tx_rlp.unwrap_or_default(),
        });
    }

    Ok(out)
}

impl BridgeStorage {
    /// Returns canonical `PillarBlockData` RLP for RPC/query materialization.
    ///
    /// Inputs:
    /// - `period`: pillar block period requested by the caller.
    ///
    /// Outputs:
    /// - Empty bytes when either the pillar block or the following period's
    ///   finalized pillar-vote bundle is absent.
    /// - Otherwise `[pillar_block_rlp, optimized_pillar_votes_bundle_rlp]`
    ///   encoded with the compatibility shape used by C++ `PillarBlockData`.
    ///
    /// Invariants and edge behavior:
    /// - Reads go directly through `rustaxa-storage`; no `DbStorage` query API
    ///   participates in Rust mode.
    /// - Vote payloads are preserved as canonical bytes and decoded only by the
    ///   RPC materialization boundary.
    pub fn get_pillar_block_data_rlp(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        let Some(pillar_block_rlp) = self.0.pillar().rlp(period)? else {
            return Ok(Vec::new());
        };
        let period_data = self
            .0
            .period()
            .data_raw(period + 1)
            .context("PILLAR_BLOCK_DATA_PERIOD_DATA")?;
        if period_data.is_empty() {
            return Ok(Vec::new());
        }

        let period_rlp = Rlp::new(&period_data);
        if period_rlp.item_count()? <= PILLAR_VOTES_POS_IN_PERIOD_DATA {
            return Ok(Vec::new());
        }
        let votes = period_rlp
            .at(PILLAR_VOTES_POS_IN_PERIOD_DATA)
            .context("PILLAR_BLOCK_DATA_VOTES")?;
        if votes.item_count()? == 0 {
            return Ok(Vec::new());
        }

        RawPillarBlockData {
            pillar_block_rlp,
            pillar_votes_bundle_rlp: votes.as_raw().to_vec(),
        }
        .encode_rlp()
    }

    pub fn dag_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0
            .dag()
            .exists(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_dag_block(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .dag()
            .by_hash_rlp_optional(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?
            .unwrap_or_default())
    }

    pub fn get_dag_block_period(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::BlockPeriod, anyhow::Error> {
        let (period, position) = self
            .0
            .dag()
            .period(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rustaxa_ffi::BlockPeriod { period, position })
    }

    pub fn get_dag_block_period_lookup(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::BlockPeriodLookup, anyhow::Error> {
        let lookup = self
            .0
            .dag()
            .period_optional(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(match lookup {
            Some((period, position)) => rustaxa_ffi::BlockPeriodLookup {
                found: true,
                period,
                position,
            },
            None => rustaxa_ffi::BlockPeriodLookup {
                found: false,
                period: 0,
                position: 0,
            },
        })
    }

    pub fn get_last_blocks_level(&self) -> Result<u64, anyhow::Error> {
        self.0.dag().last_level().map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_blocks_by_level(&self, level: u64) -> Result<Vec<u8>, anyhow::Error> {
        let hashes = self
            .0
            .dag()
            .hashes_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        let mut bytes = Vec::with_capacity(hashes.len() * 32);
        for h in hashes {
            bytes.extend_from_slice(h.as_bytes());
        }
        Ok(bytes)
    }

    pub fn get_dag_blocks_at_level(
        &self,
        level: u64,
        number_of_levels: u32,
    ) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        let rlps = self
            .0
            .dag()
            .at_level_range(level, number_of_levels)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(rlps
            .into_iter()
            .map(|data| rustaxa_ffi::BlockRlp { data })
            .collect())
    }

    pub fn get_nonfinalized_dag_blocks(
        &self,
    ) -> Result<Vec<rustaxa_ffi::LevelBlocks>, anyhow::Error> {
        let map = self
            .0
            .dag()
            .non_finalized()
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(map
            .into_iter()
            .map(|(level, blocks)| rustaxa_ffi::LevelBlocks {
                level,
                blocks: blocks
                    .into_iter()
                    .map(|data| rustaxa_ffi::BlockRlp { data })
                    .collect(),
            })
            .collect())
    }

    pub fn get_proposal_period_for_dag_level(
        &self,
        level: u64,
    ) -> Result<rustaxa_ffi::PeriodLookup, anyhow::Error> {
        let period = self
            .0
            .dag()
            .proposal_period_at_level(level)
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(match period {
            Some(period) => rustaxa_ffi::PeriodLookup {
                found: true,
                period,
            },
            None => rustaxa_ffi::PeriodLookup {
                found: false,
                period: 0,
            },
        })
    }

    pub fn save_dag_block(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .write(H256::from(*hash), level, tips_count, &block_rlp)
    }

    pub fn update_dag_block_counter(
        &self,
        hash: &[u8; 32],
        level: u64,
        tips_count: u64,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .update_counter(H256::from(*hash), level, tips_count)
    }

    pub fn remove_dag_block(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.dag().remove(H256::from(*hash))
    }

    pub fn save_proposal_period_dag_levels_map(
        &self,
        level: u64,
        period: u64,
    ) -> Result<(), anyhow::Error> {
        self.0.dag().write_proposal_period_at_level(level, period)
    }

    pub fn save_dag_block_period(
        &self,
        hash: &[u8; 32],
        period: u64,
        position: u32,
    ) -> Result<(), anyhow::Error> {
        self.0
            .dag()
            .write_period(H256::from(*hash), period, position)
    }

    pub fn get_period_data_raw(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .period()
            .data_raw(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_period_from_pbft_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<rustaxa_ffi::PeriodLookup, anyhow::Error> {
        let lookup = self
            .0
            .period()
            .by_pbft_hash(H256::from(*hash))
            .map_err(|e| anyhow::anyhow!(e))?;

        match lookup {
            Some(period) => Ok(rustaxa_ffi::PeriodLookup {
                found: true,
                period,
            }),
            None => Ok(rustaxa_ffi::PeriodLookup {
                found: false,
                period: 0,
            }),
        }
    }

    pub fn get_block_receipt(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        self.0
            .period()
            .receipt(period)
            .map_err(|e| anyhow::anyhow!(e))
    }

    pub fn get_final_chain_meta_value(&self, key: u32) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.final_chain().meta_value(key)?.unwrap_or_default())
    }

    pub fn get_final_chain_block_header(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_header_raw(block_number)?
            .unwrap_or_default())
    }

    pub fn get_final_chain_block_hash_by_number(
        &self,
        block_number: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_hash_by_number(block_number)?
            .unwrap_or_default())
    }

    pub fn get_final_chain_block_number_by_hash(
        &self,
        hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .block_number_by_hash(H256::from(*hash))?
            .unwrap_or_default())
    }

    pub fn get_final_chain_log_blooms_chunk(
        &self,
        chunk_id: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .log_blooms_chunk_raw(H256::from(*chunk_id))?
            .unwrap_or_default())
    }

    pub fn get_final_chain_receipt_by_trx_hash(
        &self,
        trx_hash: &[u8; 32],
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .final_chain()
            .receipt_by_trx_hash(H256::from(*trx_hash))?
            .unwrap_or_default())
    }

    /// Seeds the exact FinalChain lookup rows required by storage conformance.
    ///
    /// Inputs are legacy-compatible raw bytes supplied by the conformance runner;
    /// output is only durable storage mutation. The native storage repository
    /// owns one atomic write batch for all rows so the CXX bridge does not expose
    /// generic batch staging for this fixture.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_final_chain_conformance_lookup_rows(
        &self,
        meta_key: u32,
        meta_value: Vec<u8>,
        block_number: u64,
        block_hash: &[u8; 32],
        block_header_rlp: Vec<u8>,
        receipt_hash: &[u8; 32],
        receipt_rlp: Vec<u8>,
        blooms_chunk: &[u8; 32],
        blooms_rlp: Vec<u8>,
        receipt_period: u64,
        receipts_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0.final_chain().write_conformance_lookup_rows(
            meta_key,
            &meta_value,
            block_number,
            H256::from(*block_hash),
            &block_header_rlp,
            H256::from(*receipt_hash),
            &receipt_rlp,
            H256::from(*blooms_chunk),
            &blooms_rlp,
            receipt_period,
            &receipts_rlp,
        )
    }

    pub fn save_period_data(
        &self,
        period: u64,
        period_data_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0.period().write(period, &period_data_rlp)
    }

    pub fn save_pbft_block_period(
        &self,
        hash: &[u8; 32],
        period: u64,
    ) -> Result<(), anyhow::Error> {
        self.0.period().write_pbft_period(H256::from(*hash), period)
    }

    pub fn get_pillar_block(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.pillar().rlp(period)?.unwrap_or_default())
    }

    pub fn get_latest_pillar_block(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.pillar().latest_rlp()?.unwrap_or_default())
    }

    pub fn get_own_pillar_block_vote(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.pillar().own_vote_rlp()?.unwrap_or_default())
    }

    pub fn get_current_pillar_block_data(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.pillar().current_data_rlp()?.unwrap_or_default())
    }

    pub fn save_pillar_block(
        &self,
        period: u64,
        pillar_block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0.pillar().write(period, &pillar_block_rlp)
    }

    pub fn save_own_pillar_block_vote(&self, vote_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pillar().write_own_vote(&vote_rlp)
    }

    pub fn save_current_pillar_block_data(&self, data_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pillar().write_current_data(&data_rlp)
    }

    pub fn get_genesis_hash(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.metadata().genesis_hash()?.unwrap_or_default())
    }

    pub fn set_genesis_hash(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.metadata().set_genesis_hash_if_empty(hash)
    }

    pub fn get_last_sortition_params(
        &self,
        count: u64,
    ) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        // C++ passes size_t across the bridge; on the same architecture, size_t and usize are equal.
        // This conversion should never fail on 32-bit or 64-bit systems.
        let count = usize::try_from(count).unwrap_or(usize::MAX);
        let changes = self.0.metadata().last_sortition_params_changes_rlp(count)?;
        Ok(changes
            .into_iter()
            .map(|data| rustaxa_ffi::BlockRlp { data })
            .collect())
    }

    pub fn get_params_change_for_period(&self, period: u64) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .metadata()
            .params_change_for_period_rlp(period)?
            .unwrap_or_default())
    }

    pub fn get_status_field(&self, field: u8) -> Result<u64, anyhow::Error> {
        self.0.metadata().status_field(field)
    }

    pub fn save_status_field(&self, field: u8, value: u64) -> Result<(), anyhow::Error> {
        self.0.metadata().write_status_field(field, value)
    }

    pub fn save_sortition_params_change(
        &self,
        period: u64,
        params_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .write_sortition_params_change(period, &params_rlp)
    }

    pub fn get_period_lambda(
        &self,
        period: u64,
        find_closest: bool,
    ) -> Result<rustaxa_ffi::PeriodLambda, anyhow::Error> {
        let value = self.0.metadata().period_lambda(period, find_closest)?;
        Ok(match value {
            Some(value) => rustaxa_ffi::PeriodLambda { found: true, value },
            None => rustaxa_ffi::PeriodLambda {
                found: false,
                value: 0,
            },
        })
    }

    pub fn get_rounds_count_dynamic_lambda(&self) -> Result<u32, anyhow::Error> {
        self.0.metadata().rounds_count_dynamic_lambda()
    }

    pub fn save_period_lambda(&self, period: u64, period_lambda: u32) -> Result<(), anyhow::Error> {
        self.0.metadata().write_period_lambda(period, period_lambda)
    }

    pub fn save_rounds_count_dynamic_lambda(&self, rounds_count: u32) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .write_rounds_count_dynamic_lambda(rounds_count)
    }

    pub fn get_blocks_rewards_stats(&self) -> Result<Vec<rustaxa_ffi::PeriodRlp>, anyhow::Error> {
        let stats = self.0.metadata().block_rewards_stats_rlp()?;
        Ok(stats
            .into_iter()
            .map(|(period, data)| rustaxa_ffi::PeriodRlp { period, data })
            .collect())
    }

    pub fn save_block_rewards_stats(
        &self,
        period: u64,
        stats_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .metadata()
            .write_block_rewards_stats(period, &stats_rlp)
    }

    pub fn clear_block_rewards_stats(&self) -> Result<(), anyhow::Error> {
        self.0.metadata().clear_block_rewards_stats()
    }

    pub fn pbft_block_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.pbft().exists(H256::from(*hash))
    }

    pub fn get_pbft_mgr_field(&self, field: u8) -> Result<u32, anyhow::Error> {
        Ok(self.0.pbft().manager_field(field)?.unwrap_or(1))
    }

    pub fn get_pbft_mgr_status(&self, field: u8) -> Result<bool, anyhow::Error> {
        Ok(self.0.pbft().manager_status(field)?.unwrap_or(false))
    }

    pub fn get_cert_voted_block_in_round(&self) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .pbft()
            .cert_voted_block_in_round_rlp()?
            .unwrap_or_default())
    }

    pub fn get_proposed_pbft_blocks(&self) -> Result<Vec<rustaxa_ffi::BlockRlp>, anyhow::Error> {
        let blocks = self.0.pbft().proposed_rlp()?;
        Ok(blocks
            .into_iter()
            .map(|data| rustaxa_ffi::BlockRlp { data })
            .collect())
    }

    pub fn get_pbft_head(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self.0.pbft().head(H256::from(*hash))?.unwrap_or_default())
    }

    pub fn get_own_verified_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().own_verified_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| rustaxa_ffi::VoteRlp { data })
            .collect())
    }

    pub fn get_all_two_t_plus_one_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().all_two_t_plus_one_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| rustaxa_ffi::VoteRlp { data })
            .collect())
    }

    pub fn get_reward_votes(&self) -> Result<Vec<rustaxa_ffi::VoteRlp>, anyhow::Error> {
        let votes = self.0.pbft().reward_votes_rlp()?;
        Ok(votes
            .into_iter()
            .map(|data| rustaxa_ffi::VoteRlp { data })
            .collect())
    }

    pub fn save_cert_voted_block_in_round(
        &self,
        round: u64,
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .pbft()
            .write_cert_voted_block_in_round(round, &block_rlp)
    }

    pub fn save_proposed_pbft_block(
        &self,
        hash: &[u8; 32],
        block_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0.pbft().write_proposed(H256::from(*hash), &block_rlp)
    }

    pub fn save_pbft_mgr_field(&self, field: u8, value: u32) -> Result<(), anyhow::Error> {
        self.0.pbft().write_manager_field(field, value)
    }

    pub fn save_pbft_mgr_status(&self, field: u8, value: bool) -> Result<(), anyhow::Error> {
        self.0.pbft().write_manager_status(field, value)
    }

    pub fn save_pbft_head(&self, hash: &[u8; 32], head: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.pbft().write_head(H256::from(*hash), &head)
    }

    pub fn save_own_verified_vote(
        &self,
        hash: &[u8; 32],
        vote_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        require_pbft_vote_persistence_applied(domain_save_own_verified_vote(
            &self.0,
            DomainPbftVoteStorageRecord {
                hash: H256::from(*hash),
                vote_rlp,
            },
        )?)
    }

    pub fn remove_cert_voted_block_in_round(&self) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_cert_voted_block_in_round()
    }

    pub fn remove_proposed_pbft_block(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.pbft().remove_proposed(H256::from(*hash))
    }

    pub fn remove_own_verified_vote(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        require_pbft_vote_persistence_applied(domain_clear_own_verified_votes(
            &self.0,
            vec![H256::from(*hash)],
        )?)
    }

    pub fn remove_extra_reward_vote(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        require_pbft_vote_persistence_applied(domain_remove_extra_reward_votes(
            &self.0,
            vec![H256::from(*hash)],
        )?)
    }

    pub fn replace_two_t_plus_one_votes(
        &self,
        vote_type: u8,
        votes_bundle_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        require_pbft_vote_persistence_applied(domain_persist_pbft_vote_progress(
            &self.0,
            DomainPbftVoteProgressPersistenceWrite {
                extra_reward_vote: None,
                two_t_plus_one_bundle: Some(DomainPbftTwoTPlusOneVoteBundle {
                    kind: vote_type,
                    period: 0,
                    round: 0,
                    step: 0,
                    block_hash: H256::zero(),
                    votes_bundle_rlp,
                }),
            },
        )?)
    }

    pub fn save_extra_reward_vote(
        &self,
        hash: &[u8; 32],
        vote_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        require_pbft_vote_persistence_applied(domain_persist_pbft_vote_progress(
            &self.0,
            DomainPbftVoteProgressPersistenceWrite {
                extra_reward_vote: Some(DomainPbftVoteStorageRecord {
                    hash: H256::from(*hash),
                    vote_rlp,
                }),
                two_t_plus_one_bundle: None,
            },
        )?)
    }

    /// Persists VoteManager durable effects for one accepted PBFT vote.
    ///
    /// Inputs:
    /// - `write`: optional extra reward-vote record and optional latest-round
    ///   2t+1 vote bundle selected by the VoteManager progress planner.
    ///
    /// Outputs:
    /// - A bridge result with `status = 0` on success or `status = 1` plus a
    ///   stable error code on validation/storage failure.
    ///
    /// Invariants and edge behavior:
    /// - Both optional effects are applied through one Rust storage batch.
    /// - 2t+1 bundle replacement is delete-plus-put atomic.
    /// - Vote bytes are persisted as provided by C++ and are not decoded into
    ///   C++ vote objects in Rust storage.
    pub fn persist_pbft_vote_progress(
        &self,
        write: rustaxa_ffi::PbftVoteProgressPersistenceWrite,
    ) -> Result<rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        Ok(pbft_vote_persistence_from_domain(
            domain_persist_pbft_vote_progress(&self.0, vote_progress_write_to_domain(write))?,
        ))
    }

    /// Clears VoteManager own-vote rows through a Rust-owned storage batch.
    ///
    /// Inputs:
    /// - `vote_hashes`: exact latest-round own-vote keys to delete.
    ///
    /// Outputs:
    /// - A bridge result with `status = 0` after the Rust-owned batch commits
    ///   or `status = 1` plus a stable error code if storage rejects the write.
    ///
    /// Invariants and edge behavior:
    /// - The bridge does not expose or consume a C++ batch id for this path.
    /// - Missing keys are RocksDB delete no-ops, matching legacy semantics.
    pub fn clear_own_verified_votes(
        &self,
        vote_hashes: Vec<rustaxa_ffi::PbftFinalizationHash>,
    ) -> Result<rustaxa_ffi::PbftVotePersistenceResult, anyhow::Error> {
        let hashes = vote_hashes
            .into_iter()
            .map(|hash| H256::from(hash.hash))
            .collect();
        Ok(pbft_vote_persistence_from_domain(
            domain_clear_own_verified_votes(&self.0, hashes)?,
        ))
    }

    pub fn transaction_in_db(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.transaction().exists(H256::from(*hash))
    }

    pub fn transaction_finalized(&self, hash: &[u8; 32]) -> Result<bool, anyhow::Error> {
        self.0.transaction().finalized(H256::from(*hash))
    }

    pub fn get_transaction_location(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .location_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    pub fn get_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    pub fn get_transaction_by_period_position(
        &self,
        period: u64,
        position: u32,
    ) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .by_period_position_rlp(period, position)?
            .unwrap_or_default())
    }

    pub fn get_transaction_count(&self, period: u64) -> Result<u64, anyhow::Error> {
        self.0.transaction().count(period)
    }

    pub fn get_system_transaction(&self, hash: &[u8; 32]) -> Result<Vec<u8>, anyhow::Error> {
        Ok(self
            .0
            .transaction()
            .system_rlp(H256::from(*hash))?
            .unwrap_or_default())
    }

    pub fn get_all_nonfinalized_transactions(
        &self,
    ) -> Result<Vec<rustaxa_ffi::TxRlp>, anyhow::Error> {
        let trxs = self.0.transaction().all_nonfinalized_rlp()?;
        Ok(trxs
            .into_iter()
            .map(|data| rustaxa_ffi::TxRlp { data })
            .collect())
    }

    /// Batch-loads canonical transaction RLP payloads by hash through Rust
    /// storage.
    ///
    /// Inputs are transaction hashes in caller-requested order. Each output entry
    /// preserves that order and carries the queried hash, a presence flag,
    /// whether the bytes came from finalized storage, and raw transaction RLP
    /// bytes. Lookup mirrors the storage shim's hash lookup:
    /// pending/non-finalized transactions first, then finalized transaction
    /// location metadata, including system transactions.
    pub fn get_transaction_rlps_by_hashes(
        &self,
        hashes: Vec<rustaxa_ffi::DagTransactionHash>,
    ) -> Result<Vec<rustaxa_ffi::DagTransactionRlpLookup>, anyhow::Error> {
        transaction_rlp_lookups(
            &self.0,
            hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        )
    }

    pub fn get_all_transaction_period(
        &self,
    ) -> Result<Vec<rustaxa_ffi::HashPeriod>, anyhow::Error> {
        let periods = self.0.transaction().all_with_period()?;
        Ok(periods
            .into_iter()
            .map(|(hash, period)| {
                let mut h = [0u8; 32];
                h.copy_from_slice(hash.as_bytes());
                rustaxa_ffi::HashPeriod { hash: h, period }
            })
            .collect())
    }

    pub fn get_period_system_transactions_hashes(
        &self,
        period: u64,
    ) -> Result<Vec<u8>, anyhow::Error> {
        self.0.transaction().period_system_hashes_rlp(period)
    }

    pub fn save_transaction(&self, hash: &[u8; 32], trx_rlp: Vec<u8>) -> Result<(), anyhow::Error> {
        self.0.transaction().write(H256::from(*hash), &trx_rlp)
    }

    /// Persists TransactionManager-accepted non-finalized transactions with one
    /// atomic write batch and writes the manager-owned `StatusField::TrxCount`.
    ///
    /// The caller owns transaction selection, duplicate filtering, finalized
    /// checks, and the in-memory transaction-count value. This method is a
    /// storage boundary only: every supplied payload is written under its hash,
    /// and the provided `transaction_count` is stored as the target count in the
    /// same batch.
    pub fn save_non_finalized_transactions(
        &self,
        transactions: Vec<rustaxa_ffi::NonFinalizedTransactionPayload>,
        transaction_count: u64,
    ) -> Result<(), anyhow::Error> {
        domain_save_non_finalized_transactions(
            &self.0,
            transactions
                .into_iter()
                .map(|transaction| NonFinalizedTransactionStoragePayload {
                    hash: H256::from(transaction.hash),
                    trx_rlp: transaction.trx_rlp,
                })
                .collect(),
            transaction_count,
        )
    }

    pub fn remove_transaction(&self, hash: &[u8; 32]) -> Result<(), anyhow::Error> {
        self.0.transaction().remove(H256::from(*hash))
    }

    pub fn save_transaction_location(
        &self,
        hash: &[u8; 32],
        period: u64,
        position: u32,
        is_system: bool,
    ) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .write_location(H256::from(*hash), period, position, is_system)
    }

    pub fn save_system_transaction(
        &self,
        hash: &[u8; 32],
        trx_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .write_system(H256::from(*hash), &trx_rlp)
    }

    pub fn save_period_system_transactions_hashes(
        &self,
        period: u64,
        hashes_rlp: Vec<u8>,
    ) -> Result<(), anyhow::Error> {
        self.0
            .transaction()
            .write_period_system_hashes(period, &hashes_rlp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    fn tx_hash(byte: u8) -> rustaxa_ffi::DagTransactionHash {
        rustaxa_ffi::DagTransactionHash { hash: [byte; 32] }
    }

    fn non_finalized_tx_payload(hash: u8, data: u8) -> rustaxa_ffi::NonFinalizedTransactionPayload {
        rustaxa_ffi::NonFinalizedTransactionPayload {
            hash: [hash; 32],
            trx_rlp: vec![data],
        }
    }

    fn period_data_rlp(transaction_rlps: &[Vec<u8>]) -> Vec<u8> {
        let mut transactions = rlp::RlpStream::new_list(transaction_rlps.len());
        for transaction_rlp in transaction_rlps {
            transactions.append_raw(transaction_rlp, 1);
        }

        let mut period_data = rlp::RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&transactions.out(), 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.out().to_vec()
    }

    fn period_data_with_pillar_votes_rlp(votes_bundle_rlp: &[u8]) -> Vec<u8> {
        let mut period_data = rlp::RlpStream::new_list(5);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(&[0xC0], 1);
        period_data.append_raw(votes_bundle_rlp, 1);
        period_data.out().to_vec()
    }

    #[test]
    fn pillar_block_data_query_reads_raw_components_from_rust_storage() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_pillar_block_data");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            assert!(storage
                .get_pillar_block_data_rlp(10)
                .expect("missing query should succeed")
                .is_empty());

            let pillar_block_rlp = vec![0xC1, 0xA1];
            let mut votes_bundle = rlp::RlpStream::new_list(1);
            votes_bundle.append(&vec![0xB0]);
            let votes_bundle_rlp = votes_bundle.out().to_vec();

            storage
                .save_pillar_block(10, pillar_block_rlp.clone())
                .expect("pillar block should persist");
            storage
                .save_period_data(11, period_data_with_pillar_votes_rlp(&votes_bundle_rlp))
                .expect("period data should persist");

            let encoded = storage
                .get_pillar_block_data_rlp(10)
                .expect("query should succeed");
            let decoded = RawPillarBlockData::decode_rlp(&encoded).expect("wrapper should decode");
            assert_eq!(decoded.pillar_block_rlp, pillar_block_rlp);
            assert_eq!(decoded.pillar_votes_bundle_rlp, votes_bundle_rlp);
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn transaction_rlp_batch_lookup_reads_pending_finalized_system_and_missing() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_transaction_rlps");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let pending = vec![0xC1, 0xA1];
            let finalized = vec![0xC1, 0xA2];
            let system = vec![0xC1, 0xA3];

            storage
                .save_transaction(&[1u8; 32], pending.clone())
                .expect("pending transaction should save");
            storage
                .save_period_data(7, period_data_rlp(std::slice::from_ref(&finalized)))
                .expect("period data should save");
            storage
                .save_transaction_location(&[2u8; 32], 7, 0, false)
                .expect("regular finalized location should save");
            storage
                .save_system_transaction(&[3u8; 32], system.clone())
                .expect("system transaction should save");
            storage
                .save_transaction_location(&[3u8; 32], 8, 0, true)
                .expect("system finalized location should save");

            let lookup = storage
                .get_transaction_rlps_by_hashes(vec![
                    tx_hash(1),
                    tx_hash(2),
                    tx_hash(3),
                    tx_hash(4),
                ])
                .expect("batch lookup should succeed");

            assert_eq!(lookup.len(), 4);
            assert_eq!(lookup[0].hash, [1u8; 32]);
            assert!(lookup[0].found);
            assert!(!lookup[0].finalized);
            assert_eq!(lookup[0].tx_rlp, pending);
            assert_eq!(lookup[1].hash, [2u8; 32]);
            assert!(lookup[1].found);
            assert!(lookup[1].finalized);
            assert_eq!(lookup[1].tx_rlp, finalized);
            assert_eq!(lookup[2].hash, [3u8; 32]);
            assert!(lookup[2].found);
            assert!(lookup[2].finalized);
            assert_eq!(lookup[2].tx_rlp, system);
            assert_eq!(lookup[3].hash, [4u8; 32]);
            assert!(!lookup[3].found);
            assert!(!lookup[3].finalized);
            assert!(lookup[3].tx_rlp.is_empty());
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn save_non_finalized_transactions_batch_updates_trx_count_status() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_storage_save_non_finalized_transactions");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            let existing_tx_count = 3u64;
            storage
                .save_status_field(
                    rustaxa_storage::StatusField::TrxCount as u8,
                    existing_tx_count,
                )
                .expect("pre-seeded transaction count should persist");

            storage
                .save_non_finalized_transactions(
                    vec![
                        non_finalized_tx_payload(10, 1),
                        non_finalized_tx_payload(11, 2),
                    ],
                    existing_tx_count + 2,
                )
                .expect("batch write should persist accepted transactions");

            assert_eq!(
                storage
                    .get_status_field(rustaxa_storage::StatusField::TrxCount as u8)
                    .expect("trx count status should load"),
                existing_tx_count + 2,
            );
            assert_eq!(
                storage
                    .get_transaction(&[10u8; 32])
                    .expect("tx 10 should be retrievable"),
                vec![1],
            );

            storage
                .save_non_finalized_transactions(
                    vec![non_finalized_tx_payload(13, 5)],
                    existing_tx_count + 3,
                )
                .expect("second batch write should persist accepted tx");

            assert_eq!(
                storage
                    .get_status_field(rustaxa_storage::StatusField::TrxCount as u8)
                    .expect("trx count status should load"),
                existing_tx_count + 3,
            );
            assert_eq!(
                storage
                    .get_transaction(&[13u8; 32])
                    .expect("tx 13 should be persisted"),
                vec![5],
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
