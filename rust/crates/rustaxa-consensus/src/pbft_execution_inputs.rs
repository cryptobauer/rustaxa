//! Immutable input evidence from actual DAG, sidecar and application storage.
//! This module cannot grant execution, voting or publication authority.

use crate::{
    consensus_application_runtime::SyncedCertificateRejected,
    dag::{DagSyncBlockRlp, dag_block_transaction_hashes},
    dag_transaction_service::DagTransactionService,
    final_chain::FinalChain,
    pbft_chain::decode_persisted_pbft_chain_head_identity,
    pbft_execution_validation::{
        PbftExecutionContext, PbftExecutionInputError, PbftExecutionInputLimits,
        PbftExecutionSnapshot, PbftExecutionTransactionLookup,
    },
};
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_storage::{Column, StorageReadSnapshot};
use rustaxa_types::{
    StoredFinalChainBlockHeader,
    codec::rlp::{
        dag::FinalizedDagBlockBundleRlp, final_chain::StoredBlockHeaderRlp,
        pbft::SignedPbftBlockRlp,
    },
    concrete_state::ConcreteStateIdentity,
    pbft::PbftBlockLink,
};
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PbftInputUnavailable {
    PredecessorUnavailable,
    PublicationPending,
    LocalLimit,
    MissingDag(H256),
    MissingTransaction(H256),
    FinalizedPayloadUnavailable(H256),
    FutureLocation(H256),
}
#[derive(Debug)]
pub(crate) enum PbftInputCapture<T> {
    Ready(T),
    Unavailable(PbftInputUnavailable),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PbftCandidateInputs {
    block_rlp: Vec<u8>,
    dag: Vec<DagSyncBlockRlp>,
    lookups: Vec<PbftExecutionTransactionLookup>,
    finalized_payloads: Vec<(H256, Vec<u8>)>,
    prior: ConcreteStateIdentity,
    previous_pbft_hash: H256,
    pbft_head_rlp: Vec<u8>,
    final_chain_header_rlp: Vec<u8>,
}
impl PbftCandidateInputs {
    pub(crate) fn block_rlp(&self) -> &[u8] {
        &self.block_rlp
    }
    pub(crate) fn dag_blocks(&self) -> &[DagSyncBlockRlp] {
        &self.dag
    }
    pub(crate) fn lookups(&self) -> &[PbftExecutionTransactionLookup] {
        &self.lookups
    }
    pub(crate) fn finalized_payloads(&self) -> &[(H256, Vec<u8>)] {
        &self.finalized_payloads
    }
    pub(crate) fn prior(&self) -> ConcreteStateIdentity {
        self.prior
    }
    pub(crate) fn captured_heads(&self) -> (&[u8], &[u8]) {
        (&self.pbft_head_rlp, &self.final_chain_header_rlp)
    }

    /// Forms input structure only. The caller still needs qualified native,
    /// configuration and owner-generation authority before it can execute.
    pub(crate) fn into_snapshot(
        self,
        context: &PbftExecutionContext,
        rewards: Vec<Vec<u8>>,
        limits: PbftExecutionInputLimits,
    ) -> std::result::Result<PbftExecutionSnapshot, PbftExecutionInputError> {
        if context.facts().prior != self.prior
            || context.facts().previous_pbft_hash != self.previous_pbft_hash.0
        {
            return Err(PbftExecutionInputError::InvalidStructure(
                "PBFT_INPUT_CAPTURE_CONTEXT_MISMATCH".into(),
            ));
        }
        PbftExecutionSnapshot::new(
            self.block_rlp,
            self.dag,
            self.lookups,
            rewards,
            context,
            limits,
        )
    }
}

pub(crate) struct PbftInputBudget {
    used: u64,
    maximum: u64,
}
pub(crate) type PbftInputLookups = (Vec<PbftExecutionTransactionLookup>, Vec<(H256, Vec<u8>)>);
impl PbftInputBudget {
    pub(crate) fn charge(&mut self, bytes: usize) -> bool {
        let Some(next) = self.used.checked_add(bytes as u64) else {
            return false;
        };
        if next > self.maximum {
            return false;
        }
        self.used = next;
        true
    }
}

pub(crate) fn input_hash(bytes: &[u8]) -> H256 {
    let mut out = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut out);
    H256(out)
}

pub(crate) fn exact_input_rlp(bytes: &[u8]) -> Result<Rlp<'_>> {
    let value = Rlp::new(bytes);
    ensure!(
        value.payload_info()?.total() == bytes.len(),
        "PBFT_INPUT_DURABLE_TRAILING_BYTES"
    );
    Ok(value)
}

fn canonical_dag(snapshot: &StorageReadSnapshot<'_>, hash: H256) -> Result<Option<Vec<u8>>> {
    if let Some(bytes) = snapshot.get(Column::DagBlocks, hash.as_bytes())? {
        return Ok(Some(bytes));
    }
    let Some(location) = snapshot.get(Column::DagBlockPeriod, hash.as_bytes())? else {
        return Ok(None);
    };
    let location = exact_input_rlp(&location)?;
    ensure!(location.item_count()? == 2, "PBFT_INPUT_DAG_LOCATION_SHAPE");
    let period: u64 = location.val_at(0)?;
    let position: usize = location.val_at(1)?;
    let Some(data) = snapshot.get(Column::PeriodData, &period.to_le_bytes())? else {
        return Ok(None);
    };
    let data = exact_input_rlp(&data)?;
    Ok(Some(
        FinalizedDagBlockBundleRlp::new(data.at(2)?.as_raw()).canonical_block_rlp(position)?,
    ))
}

impl DagTransactionService {
    /// Captures one exact candidate without the historical omission behavior.
    /// Locks never cross an executor, signer, CXX or publication boundary.
    pub(crate) fn snapshot_pbft_execution_inputs(
        &self,
        block_rlp: &[u8],
        limits: PbftExecutionInputLimits,
    ) -> Result<PbftInputCapture<PbftCandidateInputs>> {
        self.capture_pbft_execution_inputs(block_rlp, None, limits)
    }

    /// Uses exact decoded PeriodData bytes as the DAG and transaction source
    /// while a lagging node lacks the local proposal graph.
    pub(crate) fn snapshot_synced_pbft_execution_inputs(
        &self,
        block_rlp: &[u8],
        period_data: &[u8],
        limits: PbftExecutionInputLimits,
    ) -> Result<PbftInputCapture<PbftCandidateInputs>> {
        self.capture_pbft_execution_inputs(block_rlp, Some(period_data), limits)
    }

    fn capture_pbft_execution_inputs(
        &self,
        block_rlp: &[u8],
        period_data: Option<&[u8]>,
        limits: PbftExecutionInputLimits,
    ) -> Result<PbftInputCapture<PbftCandidateInputs>> {
        let unavailable = |reason| Ok(PbftInputCapture::Unavailable(reason));
        let mut budget = PbftInputBudget {
            used: 0,
            maximum: limits.maximum_bytes,
        };
        if !budget.charge(block_rlp.len()) {
            return unavailable(PbftInputUnavailable::LocalLimit);
        }
        let block = exact_input_rlp(block_rlp)?;
        ensure!(
            matches!(block.item_count()?, 8 | 9),
            "PBFT_INPUT_BLOCK_SHAPE"
        );
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(block_rlp))?;
        let Some(prior_period) = link.period.checked_sub(1) else {
            return unavailable(PbftInputUnavailable::PredecessorUnavailable);
        };
        let (dag, transaction) = self.lock_dag_and_transaction()?;
        ensure!(
            Arc::ptr_eq(&dag.storage, &transaction.storage),
            "PBFT_INPUT_STORAGE_OWNER_MISMATCH"
        );
        let snapshot = dag.storage.read_snapshot();
        let Some(pbft_head_rlp) = snapshot.get(Column::PbftHead, H256::zero().as_bytes())? else {
            return unavailable(PbftInputUnavailable::PredecessorUnavailable);
        };
        let head = decode_persisted_pbft_chain_head_identity(&pbft_head_rlp)?;
        let Some(number) = snapshot.get(
            Column::FinalChainMeta,
            &FinalChain::DB_META_LAST_NUMBER.to_le_bytes(),
        )?
        else {
            return unavailable(PbftInputUnavailable::PredecessorUnavailable);
        };
        ensure!(number.len() == 8, "PBFT_INPUT_FINAL_CHAIN_NUMBER_WIDTH");
        let number = u64::from_le_bytes(number.as_slice().try_into().expect("checked width"));
        if dag.state.period() != prior_period
            || head.size != prior_period
            || number != prior_period
            || head.last_pbft_block_hash != link.prev_block_hash
        {
            return unavailable(PbftInputUnavailable::PredecessorUnavailable);
        }
        if snapshot.external_evm_pending_publication_raw()?.is_some() {
            return unavailable(PbftInputUnavailable::PublicationPending);
        }
        let Some(final_chain_header_rlp) =
            snapshot.get(Column::FinalChainBlkByNumber, &prior_period.to_le_bytes())?
        else {
            return unavailable(PbftInputUnavailable::PredecessorUnavailable);
        };
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(
            &final_chain_header_rlp,
        ))
        .context("PBFT_INPUT_FINAL_CHAIN_HEADER")?;
        if !budget.charge(pbft_head_rlp.len()) || !budget.charge(final_chain_header_rlp.len()) {
            return unavailable(PbftInputUnavailable::LocalLimit);
        }
        let (blocks, supplied_transactions) = if let Some(bytes) = period_data {
            if bytes.len() as u64 > limits.maximum_bytes {
                return unavailable(PbftInputUnavailable::LocalLimit);
            }
            let data = exact_input_rlp(bytes)?;
            ensure!(
                data.at(0)?.as_raw() == block_rlp,
                "PBFT_INPUT_SYNC_BLOCK_MISMATCH"
            );
            let dag = data.at(2)?;
            let count = if dag.is_empty() {
                0
            } else {
                dag.at(2)?.item_count()?
            };
            let transactions = data.at(3)?;
            if count as u64 > limits.maximum_dag_blocks
                || transactions.item_count()? as u64 > limits.maximum_transactions
            {
                return unavailable(PbftInputUnavailable::LocalLimit);
            }
            let bundle = FinalizedDagBlockBundleRlp::new(dag.as_raw());
            let blocks = (0..count)
                .map(|position| {
                    let block_rlp = bundle.canonical_block_rlp(position)?;
                    Ok(DagSyncBlockRlp {
                        hash: input_hash(&block_rlp),
                        block_rlp,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let ordered = transactions
                .iter()
                .map(|tx| tx.as_raw())
                .collect::<Vec<_>>();
            let by_hash: std::collections::HashMap<_, _> = ordered
                .iter()
                .map(|&bytes| (input_hash(bytes), bytes))
                .collect();
            if by_hash.len() != ordered.len() {
                return Err(
                    SyncedCertificateRejected("duplicate ordered transaction".into()).into(),
                );
            }
            (blocks, Some((ordered, by_hash)))
        } else {
            let order = if link.pivot_dag_block_hash.is_zero() {
                Vec::new()
            } else {
                if link.pivot_dag_block_hash == dag.state.anchor() {
                    return unavailable(PbftInputUnavailable::PredecessorUnavailable);
                }
                let Some(order) = dag.state.compute_order(link.pivot_dag_block_hash) else {
                    return unavailable(PbftInputUnavailable::MissingDag(
                        link.pivot_dag_block_hash,
                    ));
                };
                if order.is_empty() {
                    return unavailable(PbftInputUnavailable::MissingDag(
                        link.pivot_dag_block_hash,
                    ));
                }
                order
            };
            if order.len() as u64 > limits.maximum_dag_blocks {
                return unavailable(PbftInputUnavailable::LocalLimit);
            }
            let mut blocks = Vec::with_capacity(order.len());
            for hash in order {
                let Some(bytes) = canonical_dag(&snapshot, hash)? else {
                    return unavailable(PbftInputUnavailable::MissingDag(hash));
                };
                if !budget.charge(bytes.len()) {
                    return unavailable(PbftInputUnavailable::LocalLimit);
                }
                blocks.push(DagSyncBlockRlp {
                    hash,
                    block_rlp: bytes,
                });
            }
            (blocks, None)
        };
        let mut by_block = Vec::with_capacity(blocks.len());
        let mut encoded_order = RlpStream::new_list(1);
        encoded_order.begin_list(blocks.len());
        for block in &blocks {
            ensure!(
                input_hash(&block.block_rlp) == block.hash,
                "PBFT_INPUT_DAG_HASH_MISMATCH"
            );
            if period_data.is_some() && !budget.charge(block.block_rlp.len()) {
                return unavailable(PbftInputUnavailable::LocalLimit);
            }
            encoded_order.append(&block.hash.as_bytes());
            by_block.push(dag_block_transaction_hashes(&block.block_rlp)?);
        }
        let actual_order = if blocks.is_empty() {
            H256::zero()
        } else {
            input_hash(&encoded_order.out())
        };
        ensure!(
            block.val_at::<H256>(2)? == actual_order,
            "PBFT_INPUT_CANDIDATE_ORDER_MISMATCH"
        );
        let query = crate::dag::plan_non_finalized_transaction_query(&by_block);
        if query.query_hashes.len() as u64 > limits.maximum_transactions {
            return unavailable(PbftInputUnavailable::LocalLimit);
        }
        let (lookups, finalized_payloads) = match transaction.pbft_execution_input_lookups(
            &snapshot,
            &query.query_hashes,
            prior_period,
            supplied_transactions.as_ref().map(|(_, by_hash)| by_hash),
            &mut budget,
        )? {
            PbftInputCapture::Ready(data) => data,
            PbftInputCapture::Unavailable(reason) => return unavailable(reason),
        };
        if let Some((ordered, _)) = supplied_transactions {
            let actual = lookups.iter().filter_map(|lookup| match lookup {
                PbftExecutionTransactionLookup::Included { rlp, .. } => Some(rlp.as_slice()),
                _ => None,
            });
            if !ordered.into_iter().eq(actual) {
                return Err(SyncedCertificateRejected(
                    "transaction order differs from the certified DAG".into(),
                )
                .into());
            }
        }
        Ok(PbftInputCapture::Ready(PbftCandidateInputs {
            block_rlp: block_rlp.to_vec(),
            dag: blocks,
            lookups,
            finalized_payloads,
            prior: ConcreteStateIdentity {
                period: prior_period.into(),
                state_root: header.state_root.0,
            },
            previous_pbft_hash: head.last_pbft_block_hash,
            pbft_head_rlp,
            final_chain_header_rlp,
        }))
    }
}
