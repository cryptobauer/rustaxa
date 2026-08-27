//! Application-owned PBFT finalization composition.
//!
//! This module converts canonical `PeriodData` into the manager executor's
//! domain plan, drives every Rust-owned sibling action, and exposes only the
//! retained FinalChain/EVM call. The same operation serves live certified
//! blocks and already-admitted sync entries.

use crate::FinalChain;
use crate::consensus_application_startup::{StartupPillarVoteDraft, apply_startup_pillar_vote};
use crate::dag_transaction_service::DagTransactionService;
use crate::pbft_finalize::{
    PbftDynamicLambdaConfig, PbftDynamicLambdaFact, PbftFinalizationRuntimeAction,
    PbftFinalizationRuntimeStatus, PbftFinalizationStorageWriteStage,
};
use crate::pbft_manager::{PbftFinalizationExecutorBoundary, PbftFinalizationExecutorStartMode};
use crate::pbft_service::{PbftFinalizationIntent, PbftService};
use crate::pbft_vote_payload::build_optimized_pbft_vote_bundle;
use crate::pbft_vote_validation::{PbftCanonicalVoteInspectionStatus, inspect_canonical_pbft_vote};
use crate::pillar_vote_service::{
    PillarBlockFinalizationAcknowledgeRequest, PillarBlockFinalizationRequest,
};
use crate::transaction_service::TransactionServiceAccountNonceFact;
use anyhow::{Context, Result, bail, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_types::codec::rlp::dag::{DagBlockRlp, FinalizedDagBlockBundleRlp};
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use rustaxa_types::{
    CurrentPillarBlockDataDb, DagBlock, FinalChainGas, FinalizationDagBlock,
    FinalizationTransaction, LegacyTransactionEnvelope, PillarBlock, PillarBlockData, PillarVote,
    ValidatorVoteCount, ValidatorVoteCountChange, encode_optimized_pillar_votes_bundle_rlp,
};
use std::collections::HashMap;
use tiny_keccak::{Hasher, Keccak};

/// Canonical input shared by synced and live-certified finalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationFinalizationRequest {
    pub period_data_rlp: Vec<u8>,
    pub current_cert_vote_rlps: Vec<Vec<u8>>,
    pub synchronous: bool,
}

/// The sole external effect retained by application finalization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationEvmRequest {
    pub cursor: u32,
    pub period_data_rlp: Vec<u8>,
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
    pub finalized_dag_hashes: Vec<[u8; 32]>,
    pub blocks_per_year: u32,
    pub synchronous: bool,
    pub anchor_block_rlp: Vec<u8>,
}

/// Exact report for the retained FinalChain/EVM effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationEvmReport {
    pub cursor: u32,
    pub succeeded: bool,
    pub status: u8,
    pub last_block_number: u64,
    pub error_code: String,
}

/// Exact account rows needed to update finalized transaction queue state.
///
/// The request is returned only while the native finalization cursor owns
/// `UpdateFinalizedTransactions`. Addresses are unique and preserve their
/// first appearance in canonical period data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationAccountFactsRequest {
    pub cursor: u32,
    pub addresses: Vec<[u8; 20]>,
    /// Exact dynamic-lambda result retained across the account-facts leaf so
    /// the subsequent FinalChain dispatch uses the same finalization plan.
    pub blocks_per_year: u32,
    /// Exact weighted previous-certificate payloads selected while preparing
    /// this finalization. Decoding PeriodData alone cannot reconstruct them.
    pub previous_cert_vote_rlps: Vec<Vec<u8>>,
}

/// External-EVM account facts reported for one retained finalization cursor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationAccountFact {
    pub address: [u8; 20],
    pub found: bool,
    pub nonce: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationAccountFactsReport {
    pub cursor: u32,
    pub succeeded: bool,
    pub observed_block: u64,
    pub accounts: Vec<PbftApplicationAccountFact>,
    pub error_code: String,
}

/// Exact FinalChain header/bridge-state read required by pillar post-processing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationPillarAnchorRequest {
    pub cursor: u32,
    pub period: u64,
    pub pillar_block_period: u64,
}

/// Result of the exact pillar anchor-state leaf.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationPillarAnchorReport {
    pub cursor: u32,
    pub succeeded: bool,
    pub block_header_rlp: Vec<u8>,
    pub state_root: [u8; 32],
    pub bridge_root: [u8; 32],
    pub bridge_epoch: [u8; 32],
    pub validator_vote_counts: Vec<crate::pillar_chain::PillarValidatorVoteCount>,
    pub signer_vote_counts: Vec<u64>,
    pub total_eligible_vote_count: u64,
    pub error_code: String,
}

/// One exact pillar-vote signing request plus remaining native drafts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationPillarSigningRequest {
    pub cursor: u32,
    pub draft: StartupPillarVoteDraft,
    pub remaining_drafts: Vec<StartupPillarVoteDraft>,
}

/// Exact transport publication required after native pillar-vote admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationPillarGossipRequest {
    pub cursor: u32,
    pub pillar_vote_rlp: Vec<u8>,
    pub remaining_drafts: Vec<StartupPillarVoteDraft>,
}

/// Successful terminal outcome of one finalization operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationFinalizationOutcome {
    pub period: u64,
    pub block_hash: H256,
}

/// Public pillar event made available only after durable native acknowledgement.
///
/// The canonical payload is the legacy-compatible `PillarBlockData` envelope,
/// not a manager object: it contains the finalized pillar block and exactly the
/// selected threshold votes. Application runtimes may publish this event on a
/// best-effort observer leaf without affecting committed consensus state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PbftApplicationPillarObservation {
    pub block_hash: [u8; 32],
    pub block_data_rlp: Vec<u8>,
}

/// Initial native finalization boundary plus any post-commit public event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedPbftApplicationFinalization {
    pub step: PbftApplicationFinalizationStep,
    pub pillar_observation: Option<PbftApplicationPillarObservation>,
}

/// Operation-shaped boundary returned to the application runner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PbftApplicationFinalizationStep {
    Evm(PbftApplicationEvmRequest),
    AccountFacts(PbftApplicationAccountFactsRequest),
    PillarAnchor(PbftApplicationPillarAnchorRequest),
    PillarSign(PbftApplicationPillarSigningRequest),
    PillarGossip(PbftApplicationPillarGossipRequest),
    Complete(PbftApplicationFinalizationOutcome),
    Rejected { error_code: String },
}

#[derive(Clone)]
struct DecodedFinalization {
    link: PbftBlockLink,
    period_data_rlp: Vec<u8>,
    ordered_dag_hashes: Vec<H256>,
    ordered_transaction_hashes: Vec<H256>,
    transaction_senders: Vec<[u8; 20]>,
    cert: crate::pbft_vote_validation::PbftCanonicalVoteInspection,
    has_pillar_block: bool,
    pillar_hash: Option<H256>,
    previous_cert_vote_rlps: Vec<Vec<u8>>,
}

fn keccak(bytes: &[u8]) -> H256 {
    let mut output = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut output);
    output.into()
}

fn process_pillar_after_finalization(
    ficus_activation: u64,
    pillar_interval: u64,
    finalized_period: u64,
) -> bool {
    if ficus_activation == u64::MAX || pillar_interval == 0 {
        return false;
    }
    let first_pillar_period = if ficus_activation == 0 {
        pillar_interval
    } else {
        ficus_activation
    };
    finalized_period >= first_pillar_period && finalized_period % pillar_interval == 0
}

fn pillar_anchor_state_period(finalized_period: u64, delegation_delay: u64) -> Result<u64> {
    finalized_period
        .checked_sub(delegation_delay)
        .filter(|period| *period > 0)
        .context("PBFT_APPLICATION_FINALIZATION_PILLAR_STATE_PERIOD")
}

fn decode_finalization(
    request: &PbftApplicationFinalizationRequest,
) -> Result<DecodedFinalization> {
    let period_data = Rlp::new(&request.period_data_rlp);
    ensure!(
        matches!(period_data.item_count()?, 4 | 5),
        "PBFT_APPLICATION_FINALIZATION_PERIOD_DATA_SHAPE"
    );
    let block = period_data
        .at(0)
        .context("PBFT_APPLICATION_FINALIZATION_BLOCK")?;
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(block.as_raw()))?;
    let cert = request
        .current_cert_vote_rlps
        .first()
        .context("PBFT_APPLICATION_FINALIZATION_CERT_EMPTY")
        .and_then(|vote| inspect_canonical_pbft_vote(vote))?;
    ensure!(
        cert.status == PbftCanonicalVoteInspectionStatus::Valid,
        "PBFT_APPLICATION_FINALIZATION_CERT_INVALID"
    );

    let dag_bundle = period_data.at(2)?;
    let mut ordered_dag_hashes = Vec::new();
    if !dag_bundle.is_empty() {
        let count = dag_bundle.at(2)?.item_count()?;
        let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
        for index in 0..count {
            ordered_dag_hashes.push(keccak(&bundle.canonical_block_rlp(index)?));
        }
    }
    let transactions = period_data.at(3)?;
    let mut ordered_transaction_hashes = Vec::new();
    let mut transaction_senders = Vec::new();
    for tx in transactions.iter() {
        let envelope = LegacyTransactionEnvelope::decode(tx.as_raw())?;
        ordered_transaction_hashes.push(envelope.hash);
        transaction_senders.push(
            envelope
                .sender
                .context("PBFT_APPLICATION_FINALIZATION_TRANSACTION_SENDER")?
                .into(),
        );
    }
    let pillar_hash = if block.item_count()? == 9 {
        let extra_bytes = block.at(7)?.data()?;
        let extra = Rlp::new(extra_bytes);
        extra
            .at(5)
            .ok()
            .and_then(|value| value.data().ok())
            .filter(|bytes| bytes.len() == 32)
            .map(H256::from_slice)
    } else {
        None
    };
    Ok(DecodedFinalization {
        link,
        period_data_rlp: request.period_data_rlp.clone(),
        ordered_dag_hashes,
        ordered_transaction_hashes,
        transaction_senders,
        cert,
        has_pillar_block: pillar_hash.is_some(),
        pillar_hash,
        previous_cert_vote_rlps: Vec::new(),
    })
}

fn previous_cert_vote_rlps(
    pbft: &PbftService,
    decoded: &DecodedFinalization,
) -> Result<Vec<Vec<u8>>> {
    if decoded.link.period <= 1 {
        return Ok(Vec::new());
    }
    let period_data = Rlp::new(&decoded.period_data_rlp);
    let block = period_data.at(0)?;
    let rewards = block.at(6)?;
    ensure!(
        rewards.is_list(),
        "PBFT_APPLICATION_FINALIZATION_REWARD_HASHES_NOT_LIST"
    );
    let mut hashes = Vec::with_capacity(rewards.item_count()?);
    for index in 0..rewards.item_count()? {
        hashes.push(rewards.val_at(index)?);
    }
    let selection = pbft.select_reward_vote_payloads(decoded.link.period, hashes)?;
    ensure!(
        selection.accepted,
        "PBFT_APPLICATION_FINALIZATION_REWARD_VOTES_UNAVAILABLE"
    );
    Ok(selection
        .selected_records
        .into_iter()
        .map(|record| record.vote_rlp)
        .collect())
}

/// Decodes the canonical PBFT finalization payload into the complete native
/// FinalChain execution request.
///
/// Transaction, DAG-author, VDF-difficulty, and certificate-weight facts are
/// derived directly from canonical bytes. No C++ `PeriodData`, `Transaction`,
/// `DagBlock`, or `PbftVote` object participates in consensus execution.
pub(crate) fn final_chain_execution_request_from_period_data(
    period_data_rlp: &[u8],
    previous_cert_vote_rlps: &[Vec<u8>],
    previous_cert_vote_weights: &[u64],
    blocks_per_year: u32,
    block_gas_limit: FinalChainGas,
) -> Result<crate::FinalChainExecutionRequest> {
    let period_data = Rlp::new(period_data_rlp);
    ensure!(
        matches!(period_data.item_count()?, 4 | 5),
        "FINAL_CHAIN_PERIOD_DATA_SHAPE"
    );
    let pbft_block_rlp = period_data.at(0)?.as_raw().to_vec();

    let transactions = period_data
        .at(3)?
        .iter()
        .map(|transaction| {
            let envelope = LegacyTransactionEnvelope::decode(transaction.as_raw())?;
            Ok(FinalizationTransaction {
                hash: envelope.hash.into(),
                sender: envelope
                    .sender
                    .context("FINAL_CHAIN_TRANSACTION_SENDER_MISSING")?
                    .into(),
                receiver: envelope.receiver.map(Into::into),
                nonce: rustaxa_types::FinalChainNonce::from_bytes(
                    &crate::final_chain_execution::u256_to_nonce_bytes(envelope.nonce),
                )?,
                value: envelope.value.into(),
                gas_price: envelope.gas_price.into(),
                gas_limit: envelope.gas.into(),
                data: envelope.data,
                rlp: envelope.rlp,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let dag_bundle = period_data.at(2)?;
    let finalized_dag_blocks = if dag_bundle.is_empty() {
        Vec::new()
    } else {
        let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
        (0..dag_bundle.at(2)?.item_count()?)
            .map(|position| {
                let canonical_rlp = bundle.canonical_block_rlp(position)?;
                let block = DagBlock::try_from(DagBlockRlp::new(&canonical_rlp))?;
                let author = block
                    .recover_sender()
                    .context("FINAL_CHAIN_DAG_AUTHOR_RECOVERY_FAILED")?;
                let difficulty = Rlp::new(&block.vdf).val_at(3)?;
                Ok(FinalizationDagBlock {
                    author: author.into(),
                    difficulty,
                    transaction_hashes: block.transactions.into_iter().map(Into::into).collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?
    };

    ensure!(
        previous_cert_vote_weights.is_empty()
            || previous_cert_vote_weights.len() == previous_cert_vote_rlps.len(),
        "FINAL_CHAIN_REWARD_CERT_VOTE_WEIGHT_COUNT_MISMATCH"
    );
    let cert_votes = previous_cert_vote_rlps
        .iter()
        .enumerate()
        .map(|(index, vote_rlp)| {
            let vote = inspect_canonical_pbft_vote(vote_rlp)?;
            ensure!(
                vote.status == PbftCanonicalVoteInspectionStatus::Valid && vote.signature_valid,
                "FINAL_CHAIN_REWARD_CERT_VOTE_INVALID"
            );
            let weight = previous_cert_vote_weights
                .get(index)
                .copied()
                .or_else(|| vote.has_embedded_weight.then_some(vote.embedded_weight))
                .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_REWARD_CERT_VOTE_WEIGHT_MISSING"))?;
            Ok(crate::RewardCertVoteFact {
                voter: vote.recovered_voter,
                weight,
                period: vote.period,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(crate::FinalChainExecutionRequest {
        pbft_block_rlp,
        transactions,
        finalized_dag_blocks,
        blocks_per_year,
        cert_votes,
        block_gas_limit,
        mode: crate::FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
    })
}

fn drive_boundary(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    decoded: &DecodedFinalization,
    synchronous: bool,
    blocks_per_year: Option<u32>,
    mut boundary: PbftFinalizationExecutorBoundary,
) -> Result<PbftApplicationFinalizationStep> {
    loop {
        if boundary.next_step.runtime_status == PbftFinalizationRuntimeStatus::Complete {
            return Ok(PbftApplicationFinalizationStep::Complete(
                PbftApplicationFinalizationOutcome {
                    period: decoded.link.period,
                    block_hash: decoded.link.block_hash,
                },
            ));
        }
        if boundary.next_step.runtime_status != PbftFinalizationRuntimeStatus::Active
            || !boundary.next_step.has_action
        {
            return Ok(PbftApplicationFinalizationStep::Rejected {
                error_code: if boundary.error_code.is_empty() {
                    boundary.next_step.error_code
                } else {
                    boundary.error_code
                },
            });
        }
        let cursor = boundary.next_step.action_index;
        match boundary
            .next_step
            .action
            .context("PBFT_APPLICATION_FINALIZATION_ACTION_MISSING")?
        {
            PbftFinalizationRuntimeAction::SetDagBlockOrder => {
                boundary = pbft.advance_finalization_dag_order(dag, cursor)?;
            }
            PbftFinalizationRuntimeAction::CommitSortitionRuntime => {
                boundary = pbft.advance_finalization_sortition_commit(dag, cursor)?;
            }
            PbftFinalizationRuntimeAction::CommitRewardVotesResetRuntime => {
                boundary = pbft.advance_finalization_reward_votes_reset(cursor)?;
            }
            PbftFinalizationRuntimeAction::AdvancePeriod => {
                boundary = pbft.advance_period_after_finalization(cursor, decoded.link.period)?;
            }
            PbftFinalizationRuntimeAction::UpdateFinalizedTransactions => {
                let mut addresses = Vec::new();
                for sender in &decoded.transaction_senders {
                    if !addresses.contains(sender) {
                        addresses.push(*sender);
                    }
                }
                let blocks_per_year = blocks_per_year
                    .context("PBFT_APPLICATION_FINALIZATION_BLOCKS_PER_YEAR_MISSING")?;
                return Ok(PbftApplicationFinalizationStep::AccountFacts(
                    PbftApplicationAccountFactsRequest {
                        cursor,
                        addresses,
                        blocks_per_year,
                        previous_cert_vote_rlps: decoded.previous_cert_vote_rlps.clone(),
                    },
                ));
            }
            PbftFinalizationRuntimeAction::FinalizeFinalChain => {
                let anchor_block_rlp = if decoded.link.pivot_dag_block_hash.is_zero() {
                    Vec::new()
                } else {
                    dag.canonical_dag_block_rlp(decoded.link.pivot_dag_block_hash)?
                        .context("PBFT_APPLICATION_FINALIZATION_ANCHOR_MISSING")?
                };
                return Ok(PbftApplicationFinalizationStep::Evm(
                    PbftApplicationEvmRequest {
                        cursor,
                        period_data_rlp: decoded.period_data_rlp.clone(),
                        previous_cert_vote_rlps: decoded.previous_cert_vote_rlps.clone(),
                        finalized_dag_hashes: decoded
                            .ordered_dag_hashes
                            .iter()
                            .map(|hash| hash.0)
                            .collect(),
                        blocks_per_year: blocks_per_year
                            .context("PBFT_APPLICATION_FINALIZATION_BLOCKS_PER_YEAR_MISSING")?,
                        synchronous,
                        anchor_block_rlp,
                    },
                ));
            }
            PbftFinalizationRuntimeAction::ProcessPillarBlock => {
                let state_period = pillar_anchor_state_period(
                    decoded.link.period,
                    final_chain.dpos_delegation_delay(),
                )?;
                return Ok(PbftApplicationFinalizationStep::PillarAnchor(
                    PbftApplicationPillarAnchorRequest {
                        cursor,
                        period: state_period,
                        pillar_block_period: decoded.link.period,
                    },
                ));
            }
            PbftFinalizationRuntimeAction::ApplyPrimaryStorage => {
                bail!("PBFT_APPLICATION_FINALIZATION_PRIMARY_ESCAPED_START")
            }
            PbftFinalizationRuntimeAction::UpdatePbftChain
            | PbftFinalizationRuntimeAction::ClearAnchorDagCache
            | PbftFinalizationRuntimeAction::ApplyDynamicLambda
            | PbftFinalizationRuntimeAction::PersistExecutedStatus
            | PbftFinalizationRuntimeAction::SetExecutedFlag => {
                bail!("PBFT_APPLICATION_FINALIZATION_OWNED_ACTION_ESCAPED_DRAIN")
            }
            PbftFinalizationRuntimeAction::Complete => {
                bail!("PBFT_APPLICATION_FINALIZATION_COMPLETE_ACTION_INVALID")
            }
        }
    }
}

/// Prepares and drives one canonical PeriodData finalization to EVM or terminal state.
pub fn prepare_pbft_application_finalization(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    request: PbftApplicationFinalizationRequest,
) -> Result<PreparedPbftApplicationFinalization> {
    let mut decoded = decode_finalization(&request)?;
    decoded.previous_cert_vote_rlps = previous_cert_vote_rlps(pbft, &decoded)?;
    let snapshot = pbft.manager_snapshot();
    let policy = pbft.process_synced_policy();
    let (lambda_max, lambda_default) = pbft.cacti_lambda_policy();
    let dynamic = pbft.plan_finalization_dynamic_lambda(PbftDynamicLambdaFact {
        dynamic_lambda_active: decoded.link.period >= pbft.cacti_block(),
        finalized_period: decoded.link.period,
        finalized_round: decoded.cert.round,
        pre_adjust_rounds_count_dynamic_lambda: snapshot.rounds_count_dynamic_lambda,
        pre_adjust_dynamic_lambda: snapshot.dynamic_lambda_ms,
        config: PbftDynamicLambdaConfig {
            cacti_block_num: pbft.cacti_block(),
            lambda_min: policy.lambda_min_ms,
            lambda_max,
            lambda_default,
            lambda_change_interval: policy.lambda_change_interval,
            lambda_change: policy.lambda_change_ms,
            consensus_delay: policy.consensus_delay_ms,
            dpos_blocks_per_year: policy.dpos_blocks_per_year,
        },
    })?;
    ensure!(
        dynamic.plan.status == crate::pbft_finalize::PbftFinalizationStatus::Accepted,
        "PBFT_APPLICATION_FINALIZATION_DYNAMIC_LAMBDA_REJECTED"
    );
    let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
    let process_pillar =
        process_pillar_after_finalization(ficus_activation, pillar_interval, decoded.link.period);
    let resume = pbft.pbft_chain_block_exists(decoded.link.block_hash)?;
    let pillar_preparation = (!resume)
        .then_some(
            decoded
                .pillar_hash
                .map(|hash| {
                    pbft.prepare_pillar_block_finalization(PillarBlockFinalizationRequest {
                        requested_pillar_block_hash: hash.0,
                    })
                })
                .transpose()?,
        )
        .flatten();
    if let Some(prepared) = &pillar_preparation {
        ensure!(
            prepared.success
                && prepared.has_prepared_pillar_block
                && prepared.selected_vote_count > 0,
            "PBFT_APPLICATION_FINALIZATION_PILLAR_PREFLIGHT_REJECTED"
        );
    }
    let intent = PbftFinalizationIntent {
        block_hash: decoded.link.block_hash,
        block_period: decoded.link.period,
        block_prev_hash: decoded.link.prev_block_hash,
        pivot_dag_anchor_hash: decoded.link.pivot_dag_block_hash,
        has_pillar_block: decoded.has_pillar_block,
        pillar_block_finalized: pillar_preparation.is_some() || !decoded.has_pillar_block,
        request_dynamic_lambda_update: dynamic.plan.apply_dynamic_lambda_update,
        cert_vote_count: request.current_cert_vote_rlps.len() as u64,
        sample_cert_vote_block_hash: decoded.cert.block_hash,
        sample_cert_vote_period: decoded.cert.period,
        sample_cert_vote_round: decoded.cert.round,
        sample_cert_vote_step: decoded.cert.step,
        block_lambda: dynamic.plan.period_lambda,
        last_saved_period_lambda_found: dynamic.last_saved_period_lambda.found,
        last_saved_period_lambda: dynamic.last_saved_period_lambda.value,
        dynamic_blocks_per_year: dynamic.plan.blocks_per_year,
        rounds_count_dynamic_lambda: dynamic.plan.rounds_count_dynamic_lambda,
        dynamic_lambda: dynamic.plan.dynamic_lambda,
        dpos_blocks_per_year: policy.dpos_blocks_per_year,
        period_data_rlp: request.period_data_rlp,
        ordered_dag_block_hashes: decoded.ordered_dag_hashes.clone(),
        ordered_transaction_hashes: decoded.ordered_transaction_hashes.clone(),
        process_pillar_block_after_advance: process_pillar,
    };
    let plan = if resume {
        pbft.plan_finalization_resume_intent(intent)?
    } else {
        pbft.plan_finalization_intent(intent)?
    };
    if !plan.finalize_block {
        return Ok(PreparedPbftApplicationFinalization {
            step: PbftApplicationFinalizationStep::Rejected {
                error_code: format!(
                    "PBFT_APPLICATION_FINALIZATION_PLAN_REJECTED:{:?}",
                    plan.status
                ),
            },
            pillar_observation: None,
        });
    }
    let mut primary_stage = PbftFinalizationStorageWriteStage::default();
    if let Some(prepared) = &pillar_preparation {
        primary_stage.has_prepared_pillar_block = true;
        primary_stage.prepared_pillar_block_period = prepared.prepared_pillar_block_period;
        primary_stage.prepared_pillar_block_rlp = prepared.prepared_pillar_block_rlp.clone();
    }
    let boundary = pbft.start_finalization_executor(
        dag,
        crate::pbft_manager::PbftFinalizationExecutorStartRequest {
            plan,
            mode: if resume {
                PbftFinalizationExecutorStartMode::Resume {
                    final_chain_last_block: final_chain.last_block_number()?,
                }
            } else {
                PbftFinalizationExecutorStartMode::Fresh {
                    primary_stages: vec![primary_stage],
                    sync: request.synchronous,
                }
            },
        },
    )?;
    let pillar_observation = if let Some(prepared) = pillar_preparation {
        let block_data_rlp = if prepared.should_emit {
            Some(
                PillarBlockData {
                    pillar_block: PillarBlock::decode_rlp(&prepared.prepared_pillar_block_rlp)?,
                    pillar_votes: prepared
                        .votes
                        .iter()
                        .map(|vote| PillarVote::decode_rlp(&vote.vote_rlp))
                        .collect::<Result<Vec<_>>>()?,
                }
                .encode_rlp()?,
            )
        } else {
            None
        };
        let acknowledged = pbft.acknowledge_pillar_block_finalization(
            PillarBlockFinalizationAcknowledgeRequest {
                anchor_generation: prepared.preparation_anchor_generation,
                preparation_token: prepared.preparation_token,
            },
        )?;
        ensure!(
            acknowledged.should_emit == block_data_rlp.is_some(),
            "PBFT_APPLICATION_FINALIZATION_PILLAR_OBSERVER_FLAG_MISMATCH"
        );
        block_data_rlp.map(|block_data_rlp| PbftApplicationPillarObservation {
            block_hash: acknowledged.latest_finalized_hash,
            block_data_rlp,
        })
    } else {
        None
    };
    Ok(PreparedPbftApplicationFinalization {
        step: drive_boundary(
            pbft,
            dag,
            final_chain,
            &decoded,
            request.synchronous,
            Some(dynamic.plan.blocks_per_year),
            boundary,
        )?,
        pillar_observation,
    })
}

/// Reports the exact EVM result and drives the retained executor to its next boundary.
pub fn report_pbft_application_finalization_evm(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    request: &PbftApplicationFinalizationRequest,
    report: PbftApplicationEvmReport,
) -> Result<PbftApplicationFinalizationStep> {
    let decoded = decode_finalization(request)?;
    let boundary = if report.succeeded {
        pbft.advance_finalization_final_chain_dispatch(report.cursor, report.last_block_number)?
    } else {
        pbft.fail_finalization_external_effect(report.cursor, report.status, report.error_code)?
    };
    drive_boundary(
        pbft,
        dag,
        final_chain,
        &decoded,
        request.synchronous,
        None,
        boundary,
    )
}

/// Applies exact external-EVM account rows and resumes the retained transaction
/// finalization cursor.
///
/// Reports must contain every requested address exactly once and in request
/// order. A failed report terminates the native executor without mutating
/// transaction queue state.
pub fn report_pbft_application_finalization_account_facts(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    request: &PbftApplicationFinalizationRequest,
    facts_request: &PbftApplicationAccountFactsRequest,
    report: PbftApplicationAccountFactsReport,
) -> Result<PbftApplicationFinalizationStep> {
    let mut decoded = decode_finalization(request)?;
    pbft.ensure_finalization_action(
        report.cursor,
        PbftFinalizationRuntimeAction::UpdateFinalizedTransactions,
    )?;
    ensure!(
        report.cursor == facts_request.cursor,
        "PBFT_APPLICATION_FINALIZATION_ACCOUNT_CURSOR_MISMATCH"
    );
    decoded.previous_cert_vote_rlps = facts_request.previous_cert_vote_rlps.clone();
    if !report.succeeded {
        let boundary =
            pbft.fail_finalization_external_effect(report.cursor, 1, report.error_code)?;
        return drive_boundary(
            pbft,
            dag,
            final_chain,
            &decoded,
            request.synchronous,
            None,
            boundary,
        );
    }
    ensure!(
        account_facts_match_pre_finalization_head(decoded.link.period, report.observed_block),
        "PBFT_APPLICATION_FINALIZATION_ACCOUNT_BLOCK_STALE"
    );
    ensure!(
        report.accounts.len() == facts_request.addresses.len(),
        "PBFT_APPLICATION_FINALIZATION_ACCOUNT_COUNT_MISMATCH"
    );
    let mut account_nonce_facts = Vec::with_capacity(report.accounts.len());
    for (expected, account) in facts_request.addresses.iter().zip(report.accounts) {
        ensure!(
            account.address == *expected,
            "PBFT_APPLICATION_FINALIZATION_ACCOUNT_ORDER_MISMATCH"
        );
        account_nonce_facts.push(TransactionServiceAccountNonceFact {
            sender: account.address,
            account_found: account.found,
            account_nonce: account.nonce,
        });
    }
    let retention = pbft
        .process_synced_policy()
        .recently_finalized_factor
        .saturating_mul(final_chain.dpos_delegation_delay());
    let boundary = pbft.advance_finalization_transaction_status(
        dag,
        report.cursor,
        retention,
        account_nonce_facts,
    )?;
    drive_boundary(
        pbft,
        dag,
        final_chain,
        &decoded,
        request.synchronous,
        Some(facts_request.blocks_per_year),
        boundary,
    )
}

/// Returns whether account facts were read from the exact FinalChain head that
/// precedes `period`.
///
/// Transaction-status processing intentionally runs before the period's EVM
/// finalization. Resume plans do not replay that transaction action, so neither
/// an older head nor an already-finalized current-period head is valid here.
fn account_facts_match_pre_finalization_head(period: u64, observed_block: u64) -> bool {
    period.checked_sub(1) == Some(observed_block)
}

/// Applies an exact pillar anchor-state report and returns signing work or the
/// next finalization boundary. The executor cursor is not acknowledged until
/// the pillar block is durably applied and every eligible local vote is signed,
/// admitted, and persisted.
pub fn report_pbft_application_finalization_pillar_anchor(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    request: &PbftApplicationFinalizationRequest,
    signing_identities: &[(u64, [u8; 20])],
    report: PbftApplicationPillarAnchorReport,
) -> Result<PbftApplicationFinalizationStep> {
    pbft.ensure_finalization_action(
        report.cursor,
        PbftFinalizationRuntimeAction::ProcessPillarBlock,
    )?;
    let decoded = decode_finalization(request)?;
    if !report.succeeded {
        let boundary =
            pbft.fail_finalization_external_effect(report.cursor, 1, report.error_code)?;
        return drive_boundary(
            pbft,
            dag,
            final_chain,
            &decoded,
            request.synchronous,
            None,
            boundary,
        );
    }
    ensure!(
        !report.block_header_rlp.is_empty(),
        "PBFT_APPLICATION_FINALIZATION_PILLAR_HEADER_EMPTY"
    );
    let period = decoded.link.period;
    let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
    let first_pillar_period = if ficus_activation == 0 {
        pillar_interval
    } else {
        ficus_activation
    };
    let creation = pbft.plan_pillar_block_creation_with_vote_counts(
        crate::pillar_chain_service::PillarBlockCreationRequest {
            pillar_block_period: period,
            state_root: report.state_root.into(),
            bridge_root: report.bridge_root.into(),
            bridge_epoch: report.bridge_epoch.into(),
            first_pillar_block_period: first_pillar_period,
            pillar_blocks_interval: pillar_interval,
        },
        report.validator_vote_counts,
    )?;
    ensure!(
        creation.creation.valid,
        "PBFT_APPLICATION_FINALIZATION_PILLAR_LINKAGE_REJECTED"
    );
    let block = PillarBlock {
        period,
        state_root: creation.creation.state_root,
        previous_pillar_block_hash: creation.creation.previous_pillar_block_hash,
        bridge_root: creation.creation.bridge_root,
        epoch: ethereum_types::U256::from_big_endian(&report.bridge_epoch),
        validator_vote_count_changes: creation
            .vote_count_changes
            .into_iter()
            .map(|change| ValidatorVoteCountChange {
                address: change.address,
                vote_count_change: change.vote_count_change,
            })
            .collect(),
    };
    let block_hash = block.hash();
    let data = CurrentPillarBlockDataDb {
        pillar_block: block,
        vote_counts: creation
            .current_vote_counts
            .into_iter()
            .map(|count| ValidatorVoteCount {
                address: count.address,
                vote_count: count.vote_count,
            })
            .collect(),
    };
    pbft.apply_pillar_current_block_data_for_generation(
        data.encode_rlp(),
        creation.anchor_generation,
    )?;

    let vote_period = period
        .checked_add(1)
        .context("PBFT_APPLICATION_FINALIZATION_PILLAR_VOTE_PERIOD_OVERFLOW")?;
    ensure!(
        report.signer_vote_counts.len() == signing_identities.len(),
        "PBFT_APPLICATION_FINALIZATION_PILLAR_SIGNER_FACT_COUNT_MISMATCH"
    );
    let mut drafts = signing_identities
        .iter()
        .zip(report.signer_vote_counts)
        .filter(|(_, vote_count)| *vote_count > 0)
        .map(|((wallet_index, address), validator_vote_count)| {
            let unsigned = PillarVote {
                period: vote_period,
                block_hash,
                signature: [0; 65],
            };
            StartupPillarVoteDraft {
                period: vote_period,
                block_hash,
                digest: unsigned.hash(false),
                wallet_index: *wallet_index,
                expected_voter: *address,
                validator_vote_count,
                total_eligible_vote_count: report.total_eligible_vote_count,
            }
        })
        .collect::<Vec<_>>();
    if !drafts.is_empty() {
        let draft = drafts.remove(0);
        return Ok(PbftApplicationFinalizationStep::PillarSign(
            PbftApplicationPillarSigningRequest {
                cursor: report.cursor,
                draft,
                remaining_drafts: drafts,
            },
        ));
    }
    let boundary = pbft.advance_finalization_pillar_post_processing(
        report.cursor,
        pillar_anchor_state_period(decoded.link.period, final_chain.dpos_delegation_delay())?,
    )?;
    drive_boundary(
        pbft,
        dag,
        final_chain,
        &decoded,
        request.synchronous,
        None,
        boundary,
    )
}

/// Applies one host-signed pillar vote and advances only after all drafts have
/// completed native verification, admission, and persistence.
pub fn report_pbft_application_finalization_pillar_signature(
    pbft: &PbftService,
    _dag: &DagTransactionService,
    _final_chain: &FinalChain,
    _request: &PbftApplicationFinalizationRequest,
    signing: PbftApplicationPillarSigningRequest,
    signature: Vec<u8>,
) -> Result<PbftApplicationFinalizationStep> {
    pbft.ensure_finalization_action(
        signing.cursor,
        PbftFinalizationRuntimeAction::ProcessPillarBlock,
    )?;
    let pillar_vote_rlp = apply_startup_pillar_vote(&signing.draft, &signature, pbft)?;
    Ok(PbftApplicationFinalizationStep::PillarGossip(
        PbftApplicationPillarGossipRequest {
            cursor: signing.cursor,
            pillar_vote_rlp,
            remaining_drafts: signing.remaining_drafts,
        },
    ))
}

/// Acknowledges exact pillar-vote gossip before requesting the next signature
/// or reporting completed native pillar post-processing.
pub fn report_pbft_application_finalization_pillar_gossip(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    request: &PbftApplicationFinalizationRequest,
    mut gossip: PbftApplicationPillarGossipRequest,
    succeeded: bool,
    status: u8,
    error_code: String,
) -> Result<PbftApplicationFinalizationStep> {
    pbft.ensure_finalization_action(
        gossip.cursor,
        PbftFinalizationRuntimeAction::ProcessPillarBlock,
    )?;
    let decoded = decode_finalization(request)?;
    if !succeeded {
        let boundary = pbft.fail_finalization_external_effect(gossip.cursor, status, error_code)?;
        return drive_boundary(
            pbft,
            dag,
            final_chain,
            &decoded,
            request.synchronous,
            None,
            boundary,
        );
    }
    if !gossip.remaining_drafts.is_empty() {
        let draft = gossip.remaining_drafts.remove(0);
        return Ok(PbftApplicationFinalizationStep::PillarSign(
            PbftApplicationPillarSigningRequest {
                cursor: gossip.cursor,
                draft,
                remaining_drafts: gossip.remaining_drafts,
            },
        ));
    }
    let state_period =
        pillar_anchor_state_period(decoded.link.period, final_chain.dpos_delegation_delay())?;
    let boundary = pbft.advance_finalization_pillar_post_processing(gossip.cursor, state_period)?;
    drive_boundary(
        pbft,
        dag,
        final_chain,
        &decoded,
        request.synchronous,
        None,
        boundary,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pillar_postprocess_uses_ficus_pillar_period_not_pbft_payload_period() {
        assert!(!process_pillar_after_finalization(u64::MAX, 10, 20));
        assert!(!process_pillar_after_finalization(11, 10, 11));
        assert!(!process_pillar_after_finalization(11, 10, 19));
        assert!(process_pillar_after_finalization(11, 10, 20));
        assert!(process_pillar_after_finalization(0, 10, 10));
    }

    #[test]
    fn pillar_anchor_state_period_applies_checked_delegation_delay() {
        assert_eq!(pillar_anchor_state_period(20, 3).unwrap(), 17);
        assert!(pillar_anchor_state_period(3, 3).is_err());
        assert!(pillar_anchor_state_period(2, 3).is_err());
    }

    #[test]
    fn account_facts_accept_exact_pre_finalization_head() {
        assert!(account_facts_match_pre_finalization_head(1, 0));
        assert!(account_facts_match_pre_finalization_head(2, 1));

        assert!(!account_facts_match_pre_finalization_head(0, 0));
        assert!(!account_facts_match_pre_finalization_head(1, 1));
        assert!(!account_facts_match_pre_finalization_head(2, 0));
        assert!(!account_facts_match_pre_finalization_head(2, 2));
    }

    #[test]
    fn account_facts_request_retains_evm_finalization_context() {
        let previous_cert_vote_rlps = vec![vec![0x11, 0x22], vec![0x33, 0x44]];
        let request = PbftApplicationAccountFactsRequest {
            cursor: 4,
            addresses: vec![[7; 20]],
            blocks_per_year: 1_234_567,
            previous_cert_vote_rlps: previous_cert_vote_rlps.clone(),
        };

        assert_eq!(request.blocks_per_year, 1_234_567);
        assert_eq!(request.previous_cert_vote_rlps, previous_cert_vote_rlps);
    }
}

fn encode_finalized_dag_bundle(blocks: &[crate::dag::DagSyncBlockRlp]) -> Result<Vec<u8>> {
    let mut transaction_positions = HashMap::<H256, usize>::new();
    let mut transaction_hashes = Vec::new();
    let mut per_block = Vec::new();
    for stored in blocks {
        let block = DagBlock::try_from(DagBlockRlp::new(&stored.block_rlp))?;
        let mut indexes = Vec::new();
        for hash in block.transactions {
            let next = transaction_positions.len();
            let index = *transaction_positions.entry(hash).or_insert_with(|| {
                transaction_hashes.push(hash);
                next
            });
            indexes.push(index);
        }
        per_block.push(indexes);
    }
    let mut stream = RlpStream::new_list(3);
    stream.begin_list(transaction_hashes.len());
    for hash in transaction_hashes {
        stream.append(&hash);
    }
    stream.begin_list(per_block.len());
    for indexes in per_block {
        stream.begin_list(indexes.len());
        for index in indexes {
            stream.append(&index);
        }
    }
    stream.begin_list(blocks.len());
    for stored in blocks {
        let block = Rlp::new(&stored.block_rlp);
        stream.begin_list(7);
        for field in 0..5 {
            stream.append_raw(block.at(field)?.as_raw(), 1);
        }
        stream.append_raw(block.at(6)?.as_raw(), 1);
        stream.append_raw(block.at(7)?.as_raw(), 1);
    }
    Ok(stream.out().to_vec())
}

/// Builds canonical PeriodData from a live certified proposal and enters the shared operation.
pub fn prepare_certified_pbft_application_finalization(
    pbft: &PbftService,
    dag: &DagTransactionService,
    final_chain: &FinalChain,
    proposed_block_rlp: Vec<u8>,
    current_cert_vote_rlps: Vec<Vec<u8>>,
    synchronous: bool,
) -> Result<(
    PbftApplicationFinalizationRequest,
    PreparedPbftApplicationFinalization,
)> {
    let block = Rlp::new(&proposed_block_rlp);
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&proposed_block_rlp))?;
    let reward_hashes: Vec<H256> = block.list_at(6)?;
    let previous_cert = if link.period == 1 {
        vec![0x80]
    } else {
        let selected = pbft.select_reward_vote_payloads(link.period, reward_hashes)?;
        ensure!(
            selected.accepted,
            "PBFT_APPLICATION_FINALIZATION_REWARD_VOTES"
        );
        build_optimized_pbft_vote_bundle(
            &selected.selected_records,
            selected.selected_block_hash,
            selected.selected_period,
            selected.selected_round,
            3,
        )?
        .bundle_rlp
    };
    let prepared = if link.pivot_dag_block_hash.is_zero() {
        None
    } else {
        dag.prepare_pbft_candidate_payload(link.period, link.pivot_dag_block_hash)?
    };
    let hydrated = prepared
        .map(|prepared| dag.hydrate_pbft_candidate_transactions(prepared.payload))
        .transpose()?;
    let dag_bundle = hydrated
        .as_ref()
        .map(|payload| encode_finalized_dag_bundle(&payload.storage.blocks))
        .transpose()?
        .unwrap_or_else(|| vec![0x80]);
    let transactions: Vec<Vec<u8>> = hydrated
        .as_ref()
        .map(|payload| {
            payload
                .storage
                .transactions
                .iter()
                .map(|tx| tx.tx_rlp.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let pillar_bundle = if block.item_count()? == 9 {
        let extra = Rlp::new(block.at(7)?.data()?);
        let bytes = extra.at(5)?.data()?;
        if bytes.len() == 32 {
            let lookup = pbft.pillar_verified_vote_payloads(
                link.period,
                bytes.try_into().expect("length checked"),
                true,
            )?;
            ensure!(
                lookup.threshold_met && !lookup.votes.is_empty(),
                "PBFT_APPLICATION_FINALIZATION_PILLAR_VOTES"
            );
            let votes = lookup
                .votes
                .iter()
                .map(|record| PillarVote::decode_rlp(&record.vote_rlp))
                .collect::<Result<Vec<_>>>()?;
            Some(encode_optimized_pillar_votes_bundle_rlp(&votes)?)
        } else {
            None
        }
    } else {
        None
    };
    let mut period = RlpStream::new_list(4 + usize::from(pillar_bundle.is_some()));
    period.append_raw(&proposed_block_rlp, 1);
    period.append_raw(&previous_cert, 1);
    period.append_raw(&dag_bundle, 1);
    period.begin_list(transactions.len());
    for tx in transactions {
        period.append_raw(&tx, 1);
    }
    if let Some(bundle) = pillar_bundle {
        period.append_raw(&bundle, 1);
    }
    let request = PbftApplicationFinalizationRequest {
        period_data_rlp: period.out().to_vec(),
        current_cert_vote_rlps,
        synchronous,
    };
    let prepared = prepare_pbft_application_finalization(pbft, dag, final_chain, request.clone())?;
    Ok((request, prepared))
}
