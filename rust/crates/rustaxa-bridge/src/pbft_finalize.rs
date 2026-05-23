//! Bridge wrapper for PBFT finalization intent planning.
//!
//! C++ passes a compact, plain fact bundle gathered from existing execute/finalize
//! flow steps (validation, pillar-finalization check, anchor classification, etc.).
//! Rust performs deterministic intent planning and returns bridge-safe flags and
//! status codes so C++ can apply side effects explicitly.
//!
//! The finalized-period storage appender is the first native persistence cutover
//! for this path. It appends the storage records that are already represented as
//! stable keys and canonical bytes to a Rust storage batch supplied by the shim.
//! Live VoteManager, sortition manager mutation, FinalChain, and PBFT runtime
//! side effects remain caller-owned until their Rust transition APIs exist.

use crate::ffi::rustaxa_ffi::{
    PbftFinalizationCleanupPlan as FfiPbftFinalizationCleanupPlan,
    PbftFinalizationIntentFact as FfiPbftFinalizationIntentFact,
    PbftFinalizationIntentPlan as FfiPbftFinalizationIntentPlan,
    PbftFinalizationPositionedHash as FfiPbftFinalizationPositionedHash,
    PbftFinalizationStorageWritePlan as FfiPbftFinalizationStorageWritePlan,
    PbftFinalizedPeriodApplyResult as FfiPbftFinalizedPeriodApplyResult,
};
use crate::ffi::BridgeStorage;
use anyhow::{anyhow, Context, Result};
use ethereum_types::H256;
use rustaxa_consensus::pbft_finalize::{
    plan_pbft_finalization_intent as plan_domain_pbft_finalization_intent,
    PbftFinalizationCleanupIntent, PbftFinalizationIntentFact, PbftFinalizationPlan,
    PbftFinalizationPositionedHash, PbftFinalizationStorageWriteIntent,
};
use rustaxa_storage::Column;

const APPLY_STATUS_APPLIED: u8 = 0;
const APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES: u8 = 1;
const APPLY_STATUS_REJECTED_WRITE_SET: u8 = 2;
const APPLY_STATUS_MISSING_REQUIRED_PAYLOAD: u8 = 3;
const APPLY_STATUS_CONFLICTING_EXISTING_WRITE: u8 = 4;
const PBFT_MGR_FIELD_LAMBDA: u8 = 2;
const PBFT_MGR_STATUS_EXECUTED_BLOCK: u8 = 0;
const SINGLE_VALUE_KEY: [u8; 4] = 0i32.to_le_bytes();

/// C++/Rust bridge entry for one deterministic PBFT finalization intent.
pub fn plan_pbft_finalization_intent(
    fact: FfiPbftFinalizationIntentFact,
) -> FfiPbftFinalizationIntentPlan {
    plan_domain_pbft_finalization_intent(fact.into()).into()
}

/// Appends Rust-owned finalized-period storage writes to an existing bridge batch.
///
/// Inputs:
/// - `storage`: shared Rust storage bridge used by the C++ storage shim.
/// - `batch_id`: an existing bridge batch id owned by the caller's `Batch`.
/// - `write_set`: accepted PBFT finalization storage intent from the Rust planner.
///
/// Outputs:
/// - `status` reports whether writes were appended, already present, rejected, or conflicted.
/// - count fields report how many finalized DAG/transaction indexes were appended.
///
/// Invariants and edge behavior:
/// - The function appends primary finalized-period records: PBFT head,
///   PBFT hash-to-period, period-data RLP, DAG finalized indexes, transaction
///   finalized indexes, and deletes of pending DAG/transaction rows.
/// - It does not commit the batch. C++ commits the same Rust-backed batch after
///   adding still-C++-owned reward-vote and sortition writes, preserving the
///   current atomic commit boundary.
/// - Missing required payloads or conflicting existing immutable finalized
///   records return a non-applied status and do not mutate the batch. `PbftHead`
///   is mutable chain-head state and is intentionally replaced when present.
/// - Storage backend or unknown-batch failures are returned as bridge errors.
pub fn append_pbft_finalized_period_storage_writes(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.persist_pbft_head && !write_set.persist_period_data {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_REJECTED_WRITE_SET",
        ));
    }

    if write_set.persist_pbft_head && write_set.pbft_head_payload.is_empty() {
        return Ok(apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_MISSING_PBFT_HEAD_PAYLOAD",
        ));
    }

    if write_set.persist_period_data && write_set.period_data_rlp.is_empty() {
        return Ok(apply_result(
            APPLY_STATUS_MISSING_REQUIRED_PAYLOAD,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_MISSING_PERIOD_DATA_RLP",
        ));
    }

    let mut already_applied = true;
    let mut pending_deletes_absent = true;
    let pbft_block_hash = H256::from(write_set.pbft_block_hash);
    let pbft_head_hash = H256::from(write_set.pbft_head_hash);

    if write_set.persist_pbft_head {
        already_applied &= storage
            .0
            .get_raw(Column::PbftHead, pbft_head_hash.as_bytes())?
            .as_deref()
            == Some(write_set.pbft_head_payload.as_slice());
    }

    if write_set.persist_period_data {
        let period_key = write_set.block_period.to_le_bytes();
        let period_value = write_set.block_period.to_le_bytes();
        if check_existing_value(
            storage,
            Column::PbftBlockPeriod,
            pbft_block_hash.as_bytes(),
            &period_value,
            "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PbftBlockPeriod, pbft_block_hash.as_bytes())?
            .is_some();

        if check_existing_value(
            storage,
            Column::PeriodData,
            &period_key,
            &write_set.period_data_rlp,
            "PBFT_FINALIZE_CONFLICTING_PERIOD_DATA",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_DATA",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PeriodData, &period_key)?
            .is_some();

        for write in &write_set.dag_block_period_writes {
            let hash = H256::from(write.hash);
            let value = block_position_rlp(write_set.block_period, write.position);
            if check_existing_value(
                storage,
                Column::DagBlockPeriod,
                hash.as_bytes(),
                &value,
                "PBFT_FINALIZE_CONFLICTING_DAG_PERIOD",
            )? {
                return Ok(apply_result(
                    APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                    write_set,
                    0,
                    0,
                    "PBFT_FINALIZE_CONFLICTING_DAG_PERIOD",
                ));
            }
            already_applied &= storage
                .0
                .get_raw(Column::DagBlockPeriod, hash.as_bytes())?
                .is_some();
            pending_deletes_absent &= storage
                .0
                .get_raw(Column::DagBlocks, hash.as_bytes())?
                .is_none();
        }

        for write in &write_set.transaction_location_writes {
            let hash = H256::from(write.hash);
            let value = block_position_rlp(write_set.block_period, write.position);
            if check_existing_value(
                storage,
                Column::TrxPeriod,
                hash.as_bytes(),
                &value,
                "PBFT_FINALIZE_CONFLICTING_TRANSACTION_LOCATION",
            )? {
                return Ok(apply_result(
                    APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                    write_set,
                    0,
                    0,
                    "PBFT_FINALIZE_CONFLICTING_TRANSACTION_LOCATION",
                ));
            }
            already_applied &= storage
                .0
                .get_raw(Column::TrxPeriod, hash.as_bytes())?
                .is_some();
            pending_deletes_absent &= storage
                .0
                .get_raw(Column::Transactions, hash.as_bytes())?
                .is_none();
        }
    }

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;

        if write_set.persist_pbft_head {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PbftHead,
                    pbft_head_hash.as_bytes(),
                    &write_set.pbft_head_payload,
                )
                .context("PBFT_FINALIZE_BATCH_PBFT_HEAD")?;
        }

        if write_set.persist_period_data {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PbftBlockPeriod,
                    pbft_block_hash.as_bytes(),
                    &write_set.block_period.to_le_bytes(),
                )
                .context("PBFT_FINALIZE_BATCH_PBFT_PERIOD")?;
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PeriodData,
                    &write_set.block_period.to_le_bytes(),
                    &write_set.period_data_rlp,
                )
                .context("PBFT_FINALIZE_BATCH_PERIOD_DATA")?;

            for write in &write_set.dag_block_period_writes {
                let hash = H256::from(write.hash);
                storage
                    .0
                    .batch_delete_raw(batch, Column::DagBlocks, hash.as_bytes())
                    .context("PBFT_FINALIZE_BATCH_DELETE_PENDING_DAG")?;
                storage
                    .0
                    .batch_put_raw(
                        batch,
                        Column::DagBlockPeriod,
                        hash.as_bytes(),
                        &block_position_rlp(write_set.block_period, write.position),
                    )
                    .context("PBFT_FINALIZE_BATCH_DAG_PERIOD")?;
            }

            for write in &write_set.transaction_location_writes {
                let hash = H256::from(write.hash);
                storage
                    .0
                    .batch_delete_raw(batch, Column::Transactions, hash.as_bytes())
                    .context("PBFT_FINALIZE_BATCH_DELETE_PENDING_TRANSACTION")?;
                storage
                    .0
                    .batch_put_raw(
                        batch,
                        Column::TrxPeriod,
                        hash.as_bytes(),
                        &block_position_rlp(write_set.block_period, write.position),
                    )
                    .context("PBFT_FINALIZE_BATCH_TRANSACTION_LOCATION")?;
            }
        }
    }

    let status = if already_applied && pending_deletes_absent {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(apply_result(
        status,
        write_set,
        write_set.dag_block_period_writes.len(),
        write_set.transaction_location_writes.len(),
        "",
    ))
}

/// Appends dynamic-lambda persistence after C++ has applied the existing lambda
/// adjustment policy to its live `PbftManager` fields.
///
/// Inputs:
/// - `storage` and `batch_id` identify the Rust-backed batch owned by C++.
/// - `write_set` is the accepted PBFT finalization storage intent.
/// - `rounds_count_dynamic_lambda` and `dynamic_lambda` are the post-adjust live
///   values that must become durable with the optional period-lambda row.
///
/// Outputs and invariants:
/// - Returns the same apply status envelope as the primary appender.
/// - Rejects write sets that did not request dynamic-lambda persistence.
/// - Treats `period_lambda` as immutable for a finalized period and reports a
///   conflict when an existing value differs. Manager lambda and round-count
///   fields are mutable PBFT manager state and are overwritten.
pub fn append_pbft_finalization_dynamic_lambda_storage_writes(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    rounds_count_dynamic_lambda: u32,
    dynamic_lambda: u32,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.apply_dynamic_lambda_update {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_DYNAMIC_LAMBDA_NOT_REQUESTED",
        ));
    }

    let mut already_applied = true;
    if write_set.persist_period_lambda {
        let period_key = write_set.block_period.to_le_bytes();
        let period_lambda = write_set.period_lambda.to_le_bytes();
        if check_existing_value(
            storage,
            Column::PeriodLambda,
            &period_key,
            &period_lambda,
            "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA",
        )? {
            return Ok(apply_result(
                APPLY_STATUS_CONFLICTING_EXISTING_WRITE,
                write_set,
                0,
                0,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA",
            ));
        }
        already_applied &= storage
            .0
            .get_raw(Column::PeriodLambda, &period_key)?
            .is_some();
    }
    already_applied &= storage
        .0
        .get_raw(Column::RoundsCountDynamicLambda, &SINGLE_VALUE_KEY)?
        .as_deref()
        == Some(&rounds_count_dynamic_lambda.to_le_bytes());
    already_applied &= storage
        .0
        .get_raw(Column::PbftMgrRoundStep, &[PBFT_MGR_FIELD_LAMBDA])?
        .as_deref()
        == Some(&dynamic_lambda.to_le_bytes());

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;

        if write_set.persist_period_lambda {
            storage
                .0
                .batch_put_raw(
                    batch,
                    Column::PeriodLambda,
                    &write_set.block_period.to_le_bytes(),
                    &write_set.period_lambda.to_le_bytes(),
                )
                .context("PBFT_FINALIZE_BATCH_PERIOD_LAMBDA")?;
        }
        storage
            .0
            .batch_put_raw(
                batch,
                Column::RoundsCountDynamicLambda,
                &SINGLE_VALUE_KEY,
                &rounds_count_dynamic_lambda.to_le_bytes(),
            )
            .context("PBFT_FINALIZE_BATCH_DYNAMIC_LAMBDA_ROUNDS")?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::PbftMgrRoundStep,
                &[PBFT_MGR_FIELD_LAMBDA],
                &dynamic_lambda.to_le_bytes(),
            )
            .context("PBFT_FINALIZE_BATCH_DYNAMIC_LAMBDA_FIELD")?;
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

/// Appends the PBFT manager executed-block status after FinalChain finalization
/// has been dispatched.
///
/// This preserves the legacy ordering where durable `ExecutedBlock=true` is not
/// written before the final-chain path is invoked, while keeping the byte-level
/// persistence in Rust.
pub fn append_pbft_finalization_executed_status_storage_write(
    storage: &BridgeStorage,
    batch_id: u64,
    write_set: &FfiPbftFinalizationStorageWritePlan,
) -> Result<FfiPbftFinalizedPeriodApplyResult> {
    if !write_set.persist_executed_pbft_status {
        return Ok(apply_result(
            APPLY_STATUS_REJECTED_WRITE_SET,
            write_set,
            0,
            0,
            "PBFT_FINALIZE_EXECUTED_STATUS_NOT_REQUESTED",
        ));
    }

    let status_key = [PBFT_MGR_STATUS_EXECUTED_BLOCK];
    let already_applied = storage
        .0
        .get_raw(Column::PbftMgrStatus, &status_key)?
        .as_deref()
        == Some(&[u8::from(write_set.executed_pbft_status)]);

    {
        let mut batches = storage
            .1
            .lock()
            .map_err(|_| anyhow!("batch registry lock poisoned"))?;
        let batch = batches
            .get_mut(&batch_id)
            .ok_or_else(|| anyhow!("unknown batch id: {batch_id}"))?;
        storage
            .0
            .batch_put_raw(
                batch,
                Column::PbftMgrStatus,
                &status_key,
                &[u8::from(write_set.executed_pbft_status)],
            )
            .context("PBFT_FINALIZE_BATCH_EXECUTED_STATUS")?;
    }

    let status = if already_applied {
        APPLY_STATUS_ALREADY_APPLIED_SAME_VALUES
    } else {
        APPLY_STATUS_APPLIED
    };
    Ok(sidecar_apply_result(status, write_set, ""))
}

impl From<FfiPbftFinalizationIntentFact> for PbftFinalizationIntentFact {
    fn from(value: FfiPbftFinalizationIntentFact) -> Self {
        Self {
            block_hash: H256::from(value.block_hash),
            pbft_head_hash: H256::from(value.pbft_head_hash),
            block_period: value.block_period,
            block_prev_hash: H256::from(value.block_prev_hash),
            chain_last_hash: H256::from(value.chain_last_hash),
            chain_last_period: value.chain_last_period,
            block_in_chain: value.block_in_chain,
            pivot_dag_anchor_hash: H256::from(value.pivot_dag_anchor_hash),
            has_pillar_block: value.has_pillar_block,
            pillar_block_finalized: value.pillar_block_finalized,
            request_dynamic_lambda_update: value.request_dynamic_lambda_update,
            cert_vote_count: value.cert_vote_count,
            sample_cert_vote_block_hash: H256::from(value.sample_cert_vote_block_hash),
            sample_cert_vote_period: value.sample_cert_vote_period,
            sample_cert_vote_round: value.sample_cert_vote_round,
            sample_cert_vote_step: value.sample_cert_vote_step,
            block_lambda: value.block_lambda,
            last_saved_period_lambda_found: value.last_saved_period_lambda_found,
            last_saved_period_lambda: value.last_saved_period_lambda,
            dynamic_blocks_per_year: value.dynamic_blocks_per_year,
            dpos_blocks_per_year: value.dpos_blocks_per_year,
            pbft_head_payload: value.pbft_head_payload,
            period_data_rlp: value.period_data_rlp,
            ordered_dag_block_hashes: value
                .ordered_dag_block_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
            ordered_transaction_hashes: value
                .ordered_transaction_hashes
                .into_iter()
                .map(|hash| H256::from(hash.hash))
                .collect(),
        }
    }
}

impl From<PbftFinalizationCleanupIntent> for FfiPbftFinalizationCleanupPlan {
    fn from(value: PbftFinalizationCleanupIntent) -> Self {
        Self {
            persist_pbft_block_metadata: value.persist_pbft_block_metadata,
            reset_reward_votes: value.reset_reward_votes,
            set_dag_block_order: value.set_dag_block_order,
            update_sortition_params: value.update_sortition_params,
            update_finalized_transactions_status: value.update_finalized_transactions_status,
            update_pbft_chain: value.update_pbft_chain,
            clear_anchor_dag_cache: value.clear_anchor_dag_cache,
            finalize_final_chain: value.finalize_final_chain,
            maybe_update_dynamic_lambda: value.maybe_update_dynamic_lambda,
            advance_period: value.advance_period,
        }
    }
}

impl From<PbftFinalizationStorageWriteIntent> for FfiPbftFinalizationStorageWritePlan {
    fn from(value: PbftFinalizationStorageWriteIntent) -> Self {
        Self {
            persist_pbft_head: value.persist_pbft_head,
            persist_period_data: value.persist_period_data,
            reset_reward_votes: value.reset_reward_votes,
            update_sortition_params: value.update_sortition_params,
            apply_dynamic_lambda_update: value.apply_dynamic_lambda_update,
            persist_period_lambda: value.persist_period_lambda,
            persist_executed_pbft_status: value.persist_executed_pbft_status,
            pbft_block_hash: value.pbft_block_hash.0,
            pbft_head_hash: value.pbft_head_hash.0,
            block_period: value.block_period,
            null_anchor: value.null_anchor,
            reward_vote_period: value.reward_vote_period,
            reward_vote_round: value.reward_vote_round,
            reward_vote_step: value.reward_vote_step,
            reward_vote_block_hash: value.reward_vote_block_hash.0,
            period_lambda: value.period_lambda,
            blocks_per_year: value.blocks_per_year,
            executed_pbft_status: value.executed_pbft_status,
            pbft_head_payload: value.pbft_head_payload,
            period_data_rlp: value.period_data_rlp,
            dag_block_period_writes: value
                .dag_block_period_writes
                .into_iter()
                .map(Into::into)
                .collect(),
            transaction_location_writes: value
                .transaction_location_writes
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
}

fn block_position_rlp(period: u64, position: u32) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(2);
    stream.append(&period);
    stream.append(&position);
    stream.out().to_vec()
}

fn check_existing_value(
    storage: &BridgeStorage,
    column: Column,
    key: &[u8],
    expected: &[u8],
    error_code: &str,
) -> Result<bool> {
    if let Some(existing) = storage.0.get_raw(column, key)? {
        if existing != expected {
            let _ = error_code;
            return Ok(true);
        }
    }
    Ok(false)
}

fn apply_result(
    status: u8,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    dag_index_writes: usize,
    transaction_location_writes: usize,
    error_code: &str,
) -> FfiPbftFinalizedPeriodApplyResult {
    FfiPbftFinalizedPeriodApplyResult {
        status,
        wrote_pbft_head: status != APPLY_STATUS_REJECTED_WRITE_SET
            && status != APPLY_STATUS_MISSING_REQUIRED_PAYLOAD
            && status != APPLY_STATUS_CONFLICTING_EXISTING_WRITE
            && write_set.persist_pbft_head,
        wrote_period_data: status != APPLY_STATUS_REJECTED_WRITE_SET
            && status != APPLY_STATUS_MISSING_REQUIRED_PAYLOAD
            && status != APPLY_STATUS_CONFLICTING_EXISTING_WRITE
            && write_set.persist_period_data,
        dag_index_writes,
        transaction_location_writes,
        block_period: write_set.block_period,
        pbft_block_hash: write_set.pbft_block_hash,
        error_code: error_code.to_string(),
    }
}

fn sidecar_apply_result(
    status: u8,
    write_set: &FfiPbftFinalizationStorageWritePlan,
    error_code: &str,
) -> FfiPbftFinalizedPeriodApplyResult {
    FfiPbftFinalizedPeriodApplyResult {
        status,
        wrote_pbft_head: false,
        wrote_period_data: false,
        dag_index_writes: 0,
        transaction_location_writes: 0,
        block_period: write_set.block_period,
        pbft_block_hash: write_set.pbft_block_hash,
        error_code: error_code.to_string(),
    }
}

impl From<PbftFinalizationPositionedHash> for FfiPbftFinalizationPositionedHash {
    fn from(value: PbftFinalizationPositionedHash) -> Self {
        Self {
            hash: value.hash.0,
            position: value.position,
        }
    }
}

impl From<PbftFinalizationPlan> for FfiPbftFinalizationIntentPlan {
    fn from(value: PbftFinalizationPlan) -> Self {
        Self {
            finalize_block: value.finalize_block,
            anchor: value.anchor.as_u8(),
            executed_pbft_block: value.executed_pbft_block,
            status: value.status.as_u8(),
            cleanup: value.cleanup.into(),
            storage_write_intent: value.storage_write_intent.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftFinalizationHash as FfiPbftFinalizationHash;
    use crate::storage::create_storage;
    use rustaxa_consensus::pbft_finalize::PbftFinalizationAnchor::{Anchored, Null};
    use rustaxa_consensus::pbft_finalize::PbftFinalizationStatus;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const APPLY_STATUS_APPLIED_TEST: u8 = 0;
    const APPLY_STATUS_ALREADY_APPLIED_TEST: u8 = 1;
    const APPLY_STATUS_MISSING_PAYLOAD_TEST: u8 = 3;
    const APPLY_STATUS_CONFLICT_TEST: u8 = 4;
    const EXECUTED_BLOCK_STATUS_FIELD: u8 = 0;

    fn fact() -> FfiPbftFinalizationIntentFact {
        FfiPbftFinalizationIntentFact {
            block_hash: [7; 32],
            pbft_head_hash: [8; 32],
            block_period: 10,
            block_prev_hash: [3; 32],
            chain_last_hash: [3; 32],
            chain_last_period: 9,
            block_in_chain: false,
            pivot_dag_anchor_hash: [4; 32],
            has_pillar_block: false,
            pillar_block_finalized: false,
            request_dynamic_lambda_update: true,
            cert_vote_count: 3,
            sample_cert_vote_block_hash: [7; 32],
            sample_cert_vote_period: 10,
            sample_cert_vote_round: 2,
            sample_cert_vote_step: 5,
            block_lambda: 1_500,
            last_saved_period_lambda_found: false,
            last_saved_period_lambda: 0,
            dynamic_blocks_per_year: 1_000,
            dpos_blocks_per_year: 500,
            pbft_head_payload: br#"{"last":true}"#.to_vec(),
            period_data_rlp: vec![0xc0],
            ordered_dag_block_hashes: vec![
                FfiPbftFinalizationHash { hash: [1; 32] },
                FfiPbftFinalizationHash { hash: [2; 32] },
            ],
            ordered_transaction_hashes: vec![FfiPbftFinalizationHash { hash: [3; 32] }],
        }
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!("{name}_{nonce}"))
    }

    #[test]
    fn bridge_bridge_accepts_anchored_block_and_maps_cleanup_intent() {
        let plan = plan_pbft_finalization_intent(fact());

        assert!(plan.finalize_block);
        assert_eq!(plan.anchor, Anchored.as_u8());
        assert_eq!(plan.status, PbftFinalizationStatus::Accepted.as_u8());
        assert!(plan.executed_pbft_block);
        assert!(plan.cleanup.persist_pbft_block_metadata);
        assert!(plan.cleanup.update_sortition_params);
        assert!(plan.cleanup.set_dag_block_order);
        assert!(plan.storage_write_intent.persist_pbft_head);
        assert!(plan.storage_write_intent.persist_period_data);
        assert!(plan.storage_write_intent.reset_reward_votes);
        assert!(plan.storage_write_intent.update_sortition_params);
        assert!(plan.storage_write_intent.apply_dynamic_lambda_update);
        assert!(plan.storage_write_intent.persist_period_lambda);
        assert!(plan.storage_write_intent.persist_executed_pbft_status);
        assert_eq!(plan.storage_write_intent.pbft_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.pbft_head_hash, [8; 32]);
        assert_eq!(plan.storage_write_intent.reward_vote_block_hash, [7; 32]);
        assert_eq!(plan.storage_write_intent.period_lambda, 1_500);
        assert_eq!(plan.storage_write_intent.blocks_per_year, 1_000);
        assert_eq!(
            plan.storage_write_intent.pbft_head_payload,
            br#"{"last":true}"#.to_vec()
        );
        assert_eq!(plan.storage_write_intent.period_data_rlp, vec![0xc0]);
        assert_eq!(plan.storage_write_intent.dag_block_period_writes.len(), 2);
        assert_eq!(
            plan.storage_write_intent.dag_block_period_writes[1].position,
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes.len(),
            1
        );
        assert_eq!(
            plan.storage_write_intent.transaction_location_writes[0].hash,
            [3; 32]
        );
    }

    #[test]
    fn bridge_maps_anchor_and_status_for_null_and_rejects() {
        let mut rejected = fact();
        rejected.pivot_dag_anchor_hash = [0; 32];
        rejected.has_pillar_block = true;
        rejected.pillar_block_finalized = false;

        let rejected_plan = plan_pbft_finalization_intent(rejected);
        assert!(!rejected_plan.finalize_block);
        assert_eq!(rejected_plan.anchor, Null.as_u8());
        assert_eq!(
            rejected_plan.status,
            PbftFinalizationStatus::PillarDependencyMissing.as_u8()
        );
    }

    #[test]
    fn appends_finalized_period_storage_writes_to_existing_batch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");

            let mut seed = storage.0.create_write_batch();
            storage
                .0
                .batch_put_raw(&mut seed, Column::DagBlocks, &[2u8; 32], &[0xDA])
                .expect("pending DAG block should seed");
            storage
                .0
                .batch_put_raw(&mut seed, Column::Transactions, &[3u8; 32], &[0xD0])
                .expect("pending transaction should seed");
            storage
                .0
                .commit_write_batch_with_sync(seed, false)
                .expect("seed batch should commit");

            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("bridge batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
            )
            .expect("append should succeed");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(result.wrote_pbft_head);
            assert!(result.wrote_period_data);
            assert_eq!(result.dag_index_writes, 2);
            assert_eq!(result.transaction_location_writes, 1);
            storage
                .commit_write_batch(batch_id, false)
                .expect("append batch should commit");

            assert_eq!(
                storage
                    .get_pbft_head(&[8; 32])
                    .expect("pbft head should load"),
                br#"{"last":true}"#.to_vec()
            );
            assert_eq!(
                storage
                    .get_period_data_raw(10)
                    .expect("period data should load"),
                vec![0xc0]
            );
            assert!(storage
                .0
                .get_raw(Column::DagBlocks, &[2; 32])
                .expect("pending DAG row lookup should succeed")
                .is_none());
            assert!(storage
                .get_transaction(&[3; 32])
                .expect("pending transaction row should be deleted")
                .is_empty());
            assert_eq!(
                storage
                    .get_dag_block_period_lookup(&[2; 32])
                    .expect("DAG period lookup should load")
                    .position,
                1
            );
            assert!(!storage
                .get_transaction_location(&[3; 32])
                .expect("transaction location should load")
                .is_empty());
            assert!(
                !storage
                    .get_period_lambda(10, false)
                    .expect("period lambda should remain sidecar-owned")
                    .found
            );
            assert!(!storage
                .get_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD)
                .expect("executed status should remain sidecar-owned"));

            let retry_batch = storage
                .create_write_batch()
                .expect("retry batch should be created");
            let retry_result = append_pbft_finalized_period_storage_writes(
                &storage,
                retry_batch,
                &plan.storage_write_intent,
            )
            .expect("idempotent append should succeed");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            storage
                .drop_write_batch(retry_batch)
                .expect("retry batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_missing_or_conflicting_finalized_period_payloads() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_finalization_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let mut missing_plan = plan_pbft_finalization_intent(fact());
            missing_plan.storage_write_intent.pbft_head_payload.clear();
            let batch_id = storage
                .create_write_batch()
                .expect("bridge batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &missing_plan.storage_write_intent,
            )
            .expect("missing payload should return status");
            assert_eq!(result.status, APPLY_STATUS_MISSING_PAYLOAD_TEST);
            assert!(!result.wrote_pbft_head);
            storage
                .drop_write_batch(batch_id)
                .expect("missing-payload batch should drop");

            storage
                .save_pbft_block_period(&[7; 32], 99)
                .expect("conflicting PBFT block period should seed");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("conflict batch should be created");
            let result = append_pbft_finalized_period_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
            )
            .expect("conflict should return status");
            assert_eq!(result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(result.error_code, "PBFT_FINALIZE_CONFLICTING_PBFT_PERIOD");
            storage
                .drop_write_batch(batch_id)
                .expect("conflict batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_dynamic_lambda_storage_writes_after_live_adjustment() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_dynamic_lambda_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            let batch_id = storage
                .create_write_batch()
                .expect("dynamic-lambda batch should be created");
            let result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                batch_id,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("dynamic-lambda append should succeed");
            assert_eq!(result.status, APPLY_STATUS_APPLIED_TEST);
            assert!(!result.wrote_pbft_head);
            assert!(!result.wrote_period_data);
            storage
                .commit_write_batch(batch_id, false)
                .expect("dynamic-lambda batch should commit");

            let period_lambda = storage
                .get_period_lambda(10, false)
                .expect("period lambda should load");
            assert!(period_lambda.found);
            assert_eq!(period_lambda.value, 1_500);
            assert_eq!(
                storage
                    .get_rounds_count_dynamic_lambda()
                    .expect("rounds count should load"),
                7
            );
            assert_eq!(
                storage
                    .get_pbft_mgr_field(PBFT_MGR_FIELD_LAMBDA)
                    .expect("lambda field should load"),
                1_450
            );

            let retry_batch = storage
                .create_write_batch()
                .expect("retry dynamic-lambda batch should be created");
            let retry_result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                retry_batch,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("dynamic-lambda retry should succeed");
            assert_eq!(retry_result.status, APPLY_STATUS_ALREADY_APPLIED_TEST);
            storage
                .drop_write_batch(retry_batch)
                .expect("retry dynamic-lambda batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn rejects_conflicting_dynamic_lambda_period_value() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_dynamic_lambda_reject");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            storage
                .save_period_lambda(10, 1_600)
                .expect("lambda mismatch should seed");
            let lambda_batch = storage
                .create_write_batch()
                .expect("conflicting-lambda batch should be created");
            let lambda_result = append_pbft_finalization_dynamic_lambda_storage_writes(
                &storage,
                lambda_batch,
                &plan.storage_write_intent,
                7,
                1_450,
            )
            .expect("lambda mismatch should return status");
            assert_eq!(lambda_result.status, APPLY_STATUS_CONFLICT_TEST);
            assert_eq!(
                lambda_result.error_code,
                "PBFT_FINALIZE_CONFLICTING_PERIOD_LAMBDA"
            );
            storage
                .drop_write_batch(lambda_batch)
                .expect("lambda-conflict batch should drop");
        }

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn appends_executed_status_after_final_chain_dispatch() {
        let temp_dir = unique_temp_dir("rustaxa_bridge_pbft_executed_status_apply");
        {
            let storage =
                create_storage(temp_dir.to_str().expect("temp path should be valid UTF-8"))
                    .expect("storage should initialize");
            let plan = plan_pbft_finalization_intent(fact());
            storage
                .save_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD, false)
                .expect("previous executed status should seed");
            let status_batch = storage
                .create_write_batch()
                .expect("status overwrite batch should be created");
            let status_result = append_pbft_finalization_executed_status_storage_write(
                &storage,
                status_batch,
                &plan.storage_write_intent,
            )
            .expect("status overwrite should append");
            assert_eq!(status_result.status, APPLY_STATUS_APPLIED_TEST);
            storage
                .commit_write_batch(status_batch, false)
                .expect("status overwrite batch should commit");
            assert!(storage
                .get_pbft_mgr_status(EXECUTED_BLOCK_STATUS_FIELD)
                .expect("executed status should load"));
        }

        let _ = fs::remove_dir_all(temp_dir);
    }
}
