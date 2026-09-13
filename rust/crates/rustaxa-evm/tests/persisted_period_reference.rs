//! First isolated persisted execution through the existing FinalChain owner.
//!
//! The fixture starts from exclusively created empty databases, executes signed
//! transfer/creation inputs, closes both owners, then calls the created contract
//! after recovery. No imported-state coverage or production routing is implied.
//! The cumulative mutation adapter is deliberately limited to these three
//! inputs: it does not implement account deletion after earlier slot writes or
//! general raw/native same-block cache behavior. Intermediate views follow the
//! concrete observer boundary; the oracle separately checks batched agreement
//! for this finite fixture.

use anyhow::{Result, bail, ensure};
use ethereum_types::{H256, U256};
use k256::ecdsa::SigningKey;
use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::concrete_state_projection::*;
use rustaxa_consensus::final_chain_execution::*;
use rustaxa_consensus::{
    ConsensusExecutionPort, FinalChain, PillarAnchorStateReport, PillarAnchorStateRequest,
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionTransactionKind, TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call, execute_top_level_create},
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    journal::{ExecutionJournal, JournalAccountOperation},
    profile::TaraxaProfile,
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteStateMutationBatch,
    ConcreteStateReader, ConcreteStorageMutation, Config, PreparedConcreteState, Storage,
};
use rustaxa_storage::{ConcreteCommitApproval, ConcreteStateLifecycle};
use rustaxa_types::codec::rlp::concrete_lifecycle::decode_concrete_storage_catalog;
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_state::*;
use rustaxa_types::{
    FinalChainAccountBalance, FinalChainBlockNumber, FinalChainNonce, FinalChainRewardsConfig,
    FinalizationDagBlock, FinalizationTransaction, GenesisAccount, LegacyTransactionEnvelope,
    StoredFinalChainBlockHeader,
};
use serde_json::Value;
use std::{cell::RefCell, collections::BTreeMap, path::Path, sync::Arc};

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("fixture hex string")).unwrap()
}
fn fixed<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().unwrap()
}
fn number(value: &Value) -> u64 {
    value.as_u64().expect("fixture integer")
}
fn record(
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
) -> ConcreteAccountRecord {
    let mut raw = RlpStream::new_list(5);
    raw.append(&nonce.to_bytes());
    let balance_bytes = if balance.value() == &BigUint::default() {
        vec![]
    } else {
        balance.value().to_bytes_be()
    };
    raw.append(&balance_bytes);
    raw.append_empty_data();
    match code_hash {
        Some(hash) => {
            raw.append(&hash.as_slice());
        }
        None => {
            raw.append_empty_data();
        }
    }
    raw.append(&code_size);
    ConcreteAccountRecord {
        account: ConcreteAccount {
            nonce,
            balance,
            storage_root: None,
            code_hash,
            code_size,
        },
        physical_rlp: raw.out().to_vec(),
    }
}

/// Immutable per-transaction view over the fixture's exclusively created state.
/// Missing physical slots are known absent only because this test owns the full
/// finite creation/mutation history across both closes. No imported reader can
/// obtain this capability. Account/code corruption continues to fail normally.
struct FixtureView<'a> {
    prior: &'a dyn ConcreteStateRead,
    identity: ConcreteStateIdentity,
    accounts: &'a BTreeMap<[u8; 20], ConcreteRead<ConcreteAccountRecord>>,
    slots: &'a BTreeMap<([u8; 20], ConcreteStorageKey), ConcreteRead<Vec<u8>>>,
    code: &'a BTreeMap<[u8; 32], Vec<u8>>,
}
impl ConcreteStateRead for FixtureView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.accounts
            .get(&address)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| self.prior.account(address))
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        if let Some(value) = self.slots.get(&(address, key)) {
            return Ok(value.clone());
        }
        match self.prior.storage(address, key) {
            Err(ConcreteReadError::HistoryUnavailable(_)) => Ok(ConcreteRead::Absent),
            result => result,
        }
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.code
            .get(&hash)
            .cloned()
            .map(|value| Ok(ConcreteRead::Present(value)))
            .unwrap_or_else(|| self.prior.code(hash))
    }
}
struct ChainHashes<'a>(&'a FinalChain);
impl BlockHashRead for ChainHashes<'_> {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        self.0
            .block_hash(number)
            .map_err(|e| BlockHashReadError::Io(e.to_string()))?
            .ok_or(BlockHashReadError::HistoryUnavailable(number))?
            .try_into()
            .map_err(|_| BlockHashReadError::Corrupt("block hash width".into()))
    }
}
/// The fixture's explicit pre-Ficus/pre-Cacti native registry. No native target
/// is used, but unexpected native dispatch must still refuse ordinary execution.
struct FixtureNatives;
impl NativeAddressClassifier for FixtureNatives {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address[..19] == [0; 19] && matches!(address[19], 1..=8 | 0xee | 0xfe)
    }
}
struct Staged {
    prepared: PreparedConcreteState,
    projection: FinalChainConcreteStateProjection,
    provenance: Vec<u8>,
}
struct Adapter<'a> {
    chain: &'a FinalChain,
    application: &'a Storage,
    concrete: RefCell<Option<ConcreteStateLifecycle>>,
    staged: RefCell<Option<Staged>>,
    fixture: &'a Value,
}
impl ConsensusExecutionPort for Adapter<'_> {
    fn load_final_chain_committed_state(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let concrete = self.concrete.borrow();
        let observed = concrete
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable after commit"))?
            .observation()?;
        ensure!(
            observed.identity.chain_id == request.concrete_chain_identity,
            "fixture chain identity"
        );
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            committed: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            concrete_provenance_rlp: observed.provenance_rlp,
            pending_concrete_marker_rlp: observed.pending_marker_rlp,
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_system_transaction_facts(
        &self,
        request: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact> {
        ensure!(
            !request.is_pillar_block_period,
            "fixture cannot finalize a bridge epoch"
        );
        let concrete = self.concrete.borrow();
        let account = concrete
            .as_ref()
            .unwrap()
            .prior_reader()
            .account(request.bridge_contract_address)?;
        ensure!(
            matches!(account, ConcreteRead::Absent | ConcreteRead::Tombstone),
            "fixture bridge is not absent"
        );
        Ok(FinalChainSystemTransactionPlanFact {
            request_id: request.request_id,
            period: request.period,
            is_pillar_block_period: false,
            bridge_contract_address: request.bridge_contract_address,
            bridge_contract_found: false,
            bridge_contract_has_code: false,
            should_finalize_epoch: false,
            system_account_nonce: FinalChainNonce::zero(),
            block_gas_limit: request.block_gas_limit,
        })
    }
    fn execute_final_chain_transactions(
        &self,
        request: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport> {
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable"))?;
        concrete.stage_execution(&request.concrete_marker_rlp)?;
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        let expected = self.fixture["transactions"].as_array().unwrap();
        ensure!(
            request.transactions.len() == expected.len(),
            "fixture transaction count"
        );
        let mut account_changes = BTreeMap::new();
        let mut slot_changes = BTreeMap::new();
        let mut code_changes = BTreeMap::new();
        let mut accounts = BTreeMap::new();
        let mut slots = BTreeMap::new();
        let mut code = BTreeMap::new();
        let mut identity = concrete.observation()?.committed;
        let mut prepared = None;
        let mut effects = Vec::new();
        let mut results = Vec::new();
        let mut cumulative = 0_u64;
        for (transaction, expected) in request.transactions.iter().zip(expected) {
            ensure!(
                transaction.rlp == bytes(&expected["signed_rlp"]),
                "fixture signed input"
            );
            ensure!(!transaction.is_system, "fixture unexpected system input");
            let decoded = decode_legacy_input(
                transaction.position,
                &transaction.rlp,
                LegacyInputKind::Signed,
            )?;
            ensure!(
                decoded.hash == transaction.hash && decoded.sender == transaction.sender,
                "selected transaction identity"
            );
            let view = FixtureView {
                prior: concrete.prior_reader(),
                identity,
                accounts: &accounts,
                slots: &slots,
                code: &code,
            };
            let mut journal = ExecutionJournal::new(view);
            let block = ExecutionBlockContext {
                period: request.period,
                author: request.block_author,
                timestamp: request.timestamp,
                gas_limit: request.block_gas_limit,
                chain_id: 841,
                difficulty: BigUint::default(),
            };
            let result = match decoded.kind {
                ExecutionTransactionKind::Call => execute_top_level_call(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &FixtureNatives,
                    &block,
                    &decoded,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )?,
                ExecutionTransactionKind::Create => execute_top_level_create(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &block,
                    &decoded,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )?,
                _ => bail!("fixture unsupported transaction kind"),
            };
            let TransactionExecutionResult::Executed(result) = result else {
                bail!("fixture consensus failure")
            };
            ensure!(
                result.status == CodeExecutionStatus::Success,
                "fixture code failure"
            );
            let settled = journal.settle_transaction()?;
            drop(journal);
            ensure!(
                settled.writes.raw_storage.is_empty(),
                "fixture cannot omit native effects"
            );
            cumulative = cumulative.checked_add(result.gas_used.as_u64()).unwrap();
            let mut report = FinalChainEvmTransactionResult {
                position: transaction.position,
                hash: transaction.hash,
                status: 1,
                gas_used: result.gas_used,
                cumulative_gas_used: cumulative.into(),
                receipt_rlp: Vec::new(),
                logs: result
                    .logs
                    .into_iter()
                    .map(|log| FinalChainEvmLog {
                        address: log.address,
                        topics: log
                            .topics
                            .into_iter()
                            .map(|topic| FinalChainEvmLogTopic { topic })
                            .collect(),
                        data: log.data,
                    })
                    .collect(),
                new_contract_address: result.attempted_contract_address,
                output: result.output,
                code_error: String::new(),
                consensus_error: String::new(),
            };
            report.receipt_rlp = encode_external_evm_receipt(&report);
            ensure!(
                report.receipt_rlp == bytes(&expected["receipt_rlp"]),
                "Go receipt bytes"
            );
            ensure!(
                encode_concrete_evm_transaction(transaction)
                    == bytes(&expected["state_api_transaction_rlp"]),
                "Go StateAPI input bytes"
            );
            ensure!(
                encode_concrete_execution_result(&report)
                    == bytes(&expected["state_api_execution_result_rlp"]),
                "Go execution result bytes"
            );
            let mut changed = std::collections::BTreeSet::new();
            for write in settled.writes.accounts {
                changed.insert(write.address);
                let mutation = match write.operation {
                    JournalAccountOperation::Upsert {
                        nonce,
                        balance,
                        code_hash,
                        code_size,
                    } => ConcreteAccountMutation::Upsert {
                        address: write.address,
                        record: record(nonce, balance, code_hash, code_size),
                    },
                    JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                        address: write.address,
                    },
                };
                account_changes.insert(write.address, mutation);
            }
            for write in settled.writes.ordinary_storage {
                changed.insert(write.address);
                let value = if write.value == BigUint::default() {
                    None
                } else {
                    Some(write.value.to_bytes_be())
                };
                slots.insert(
                    (write.address, write.key),
                    value
                        .clone()
                        .map(ConcreteRead::Present)
                        .unwrap_or(ConcreteRead::Tombstone),
                );
                slot_changes.insert(
                    (write.address, write.key),
                    ConcreteStorageMutation {
                        address: write.address,
                        key: write.key,
                        value,
                    },
                );
            }
            for write in settled.writes.code {
                code.insert(write.code_hash, write.code.clone());
                code_changes.insert(
                    write.code_hash,
                    ConcreteCodeInsertion {
                        code_hash: write.code_hash,
                        code: write.code,
                    },
                );
            }
            let next = concrete.prepare(
                request.period,
                ConcreteStateMutationBatch {
                    accounts: account_changes.values().cloned().collect(),
                    storage: slot_changes.values().cloned().collect(),
                    code: code_changes.values().cloned().collect(),
                },
            )?;
            identity = next.next_identity();
            ensure!(
                identity.state_root == fixed::<32>(&expected["intermediate_root"]),
                "Go intermediate root"
            );
            let mut effect_accounts = Vec::new();
            for address in changed {
                let account = concrete.prepared_account(&next, address)?;
                effect_accounts.push(FinalChainConcreteAccountProjection {
                    address,
                    raw_account_rlp: match &account {
                        ConcreteRead::Present(record) => record.physical_rlp.clone(),
                        _ => Vec::new(),
                    },
                });
                accounts.insert(address, account);
            }
            effects.push(FinalChainConcreteTransactionEffect {
                index: u64::from(transaction.position.as_u32()),
                transaction_rlp: encode_concrete_evm_transaction(transaction),
                execution_result_rlp: encode_concrete_execution_result(&report),
                intermediate_state: FinalChainConcreteState {
                    period: request.period.as_u64(),
                    root: identity.state_root,
                },
                accounts: effect_accounts,
                storage: Vec::new(),
                invocations: Vec::new(),
            });
            prepared = Some(next);
            results.push(report);
        }
        let prior_catalog = decode_concrete_storage_catalog(&concrete.observation()?.catalog_rlp)?;
        ensure!(
            prior_catalog.is_empty(),
            "fixture has no native genesis or mutations"
        );
        let projection = FinalChainConcreteStateProjection {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            prior_state: marker.prior_state,
            post_transaction_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            post_rewards_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            transaction_effects: effects,
            accounts: accounts
                .into_iter()
                .map(|(address, row)| FinalChainConcreteAccountProjection {
                    address,
                    raw_account_rlp: match row {
                        ConcreteRead::Present(record) => record.physical_rlp,
                        _ => Vec::new(),
                    },
                })
                .collect(),
            storage: Vec::new(),
            invocations: Vec::new(),
            rewards_input: Vec::new(),
            catalog_hash: concrete_storage_catalog_hash(&[]),
        };
        *self.staged.borrow_mut() = Some(Staged {
            prepared: prepared.unwrap(),
            projection,
            provenance: Vec::new(),
        });
        Ok(FinalChainEvmExecutionReport {
            request_id: request.request_id,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            post_transaction_state_root: identity.state_root,
            cumulative_gas_used: cumulative.into(),
            results,
        })
    }
    fn distribute_final_chain_rewards(
        &self,
        request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        ensure!(
            request
                .transaction_fees
                .iter()
                .flatten()
                .all(|byte| *byte == 0),
            "fixture requires zero fees"
        );
        let mut staged = self.staged.borrow_mut();
        let staged = staged
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture execution is not staged"))?;
        ensure!(
            staged.projection.post_transaction_state.root == request.post_transaction_state_root,
            "rewards prior root"
        );
        staged.projection.rewards_input =
            encode_concrete_rewards_input(&request.distribution_stats);
        let projection = encode_concrete_state_projection(&staged.projection);
        let projection_hash = concrete_state_bytes_digest(&projection);
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        // The real no-validator/yield-zero/price-zero configuration is checked
        // by existing FinalChain reward/native kernels during commit preparation.
        // No adapter-defined reward result can bypass that validator.
        staged.provenance = encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: staged.projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash: staged.projection.catalog_hash,
        });
        Ok(FinalChainEvmRewardsReport {
            request_id: request.request_id,
            period: request.period,
            status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: request.post_transaction_state_root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            concrete_projection_rlp: projection,
            concrete_projection_hash: projection_hash,
            concrete_provenance_rlp: staged.provenance.clone(),
            total_reward: Vec::new(),
        })
    }
    fn commit_final_chain_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        ensure!(
            self.chain.last_block_number_typed()? == request.prior_state.period,
            "application published before concrete commit"
        );
        ensure!(
            self.application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_some(),
            "application intent must be durable first"
        );
        self.chain.validate_pending_external_evm_commit(request)?;
        let mut foreign = request.clone();
        foreign.concrete_projection_hash[0] ^= 1;
        ensure!(
            self.chain
                .validate_pending_external_evm_commit(&foreign)
                .is_err(),
            "foreign intent accepted"
        );
        let staged = self
            .staged
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("fixture execution is not staged"))?;
        ensure!(
            encode_concrete_state_projection(&staged.projection) == request.concrete_projection_rlp
                && staged.provenance == request.concrete_provenance_rlp,
            "accepted projection/provenance differs"
        );
        let concrete = self
            .concrete
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable"))?;
        let catalog = concrete.observation()?.catalog_rlp;
        let observed = concrete.commit_approved(
            staged.prepared,
            ConcreteCommitApproval {
                marker_rlp: request.concrete_marker_rlp.clone(),
                provenance_rlp: request.concrete_provenance_rlp.clone(),
                catalog_rlp: catalog,
                projection_hash: request.concrete_projection_hash,
                catalog_hash: staged.projection.catalog_hash,
            },
        )?;
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.provenance_rlp == request.concrete_provenance_rlp,
            "concrete commit observation"
        );
        Ok(FinalChainExternalEvmStateCommitResult {
            request_id: request.request_id,
            plan_id: request.plan_id,
            period: request.period,
            publication_block_hash: request.publication_block_hash,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: request.post_rewards_state_root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_projection_rlp: request.concrete_projection_rlp.clone(),
            concrete_projection_hash: request.concrete_projection_hash,
            concrete_provenance_rlp: request.concrete_provenance_rlp.clone(),
            committed_state: Some(FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            }),
            status: FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            error_code: String::new(),
        })
    }
    fn discard_final_chain_state(
        &self,
        request: &FinalChainExternalEvmDiscardRequest,
    ) -> Result<FinalChainExternalEvmDiscardReport> {
        self.staged.borrow_mut().take();
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture ambiguous consumed commit"))?;
        concrete.discard_execution(&request.concrete_marker_rlp)?;
        Ok(FinalChainExternalEvmDiscardReport {
            request_id: request.request_id,
            period: request.period,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            marker_hash: request.marker_hash,
            prior_state: request.prior_state,
            committed_state: request.prior_state,
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_pillar_anchor_state(
        &self,
        _: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        bail!("fixture has no pillar operation")
    }
}

fn pbft(period: u64) -> Vec<u8> {
    fn fields(out: &mut RlpStream, period: u64) {
        for value in 10..14 {
            out.append(&H256::from_low_u64_be(value));
        }
        out.append(&period).append(&(1234 + period));
        out.begin_list(0);
    }
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned, period);
    let key = SigningKey::from_slice(&[9; 32]).unwrap();
    let (signature, recovery) = key
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .unwrap();
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut signed = RlpStream::new_list(8);
    fields(&mut signed, period);
    signed.append(&signature);
    signed.out().to_vec()
}
fn request(fixture: &Value) -> Result<FinalChainExecutionRequest> {
    let transactions = fixture["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tx| {
            let raw = bytes(&tx["signed_rlp"]);
            let decoded = LegacyTransactionEnvelope::decode(&raw)?;
            ensure!(decoded.signature_valid, "fixture signature");
            Ok(FinalizationTransaction {
                hash: decoded.hash.0,
                sender: decoded.sender.unwrap().0,
                receiver: decoded.receiver.map(|address| address.0),
                nonce: FinalChainNonce::from_bytes(
                    &decoded
                        .nonce
                        .to_big_endian()
                        .into_iter()
                        .skip_while(|byte| *byte == 0)
                        .collect::<Vec<_>>(),
                )?,
                value: decoded.value.into(),
                gas_price: decoded.gas_price.into(),
                gas_limit: decoded.gas.into(),
                data: decoded.data,
                rlp: raw,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let dag = FinalizationDagBlock {
        author: [0x77; 20],
        difficulty: 0,
        transaction_hashes: transactions.iter().map(|tx| tx.hash).collect(),
    };
    Ok(FinalChainExecutionRequest {
        pbft_block_rlp: pbft(number(&fixture["period"])),
        transactions,
        finalized_dag_blocks: vec![dag],
        blocks_per_year: 0,
        cert_votes: Vec::new(),
        block_gas_limit: 1_000_000_u64.into(),
        mode: FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
    })
}
fn open_chain(path: &Path, fixture: &Value) -> Result<(Arc<Storage>, FinalChain)> {
    let storage = Arc::new(Storage::new(Config::new(path.to_path_buf()))?);
    storage.metadata().set_genesis_hash_if_empty(&[7; 32])?;
    let rewards = FinalChainRewardsConfig {
        yield_percentage: 0,
        magnolia_period: 0.into(),
        cornus_period: 0.into(),
        aspen_part_two_period: FinalChainBlockNumber::MAX,
        cacti_period: FinalChainBlockNumber::MAX,
        fix_redelegate_block_num: FinalChainBlockNumber::MAX,
        ..Default::default()
    };
    let chain = FinalChain::new_with_genesis_state_root(
        storage.clone(),
        1_000_000_u64.into(),
        0,
        H256(fixed(&fixture["genesis"]["root"])),
        true,
        vec![GenesisAccount {
            address: fixed(&fixture["inputs"]["sender"]),
            balance: FinalChainAccountBalance::new_account(U256::from(1_000_000)),
        }],
        Vec::new(),
        Default::default(),
        rewards,
    )?;
    Ok((storage, chain))
}

fn verify_receipts(storage: &Storage, period: &Value) -> Result<()> {
    for transaction in period["transactions"].as_array().unwrap() {
        ensure!(
            storage
                .final_chain()
                .receipt_by_trx_hash(H256(fixed(&transaction["hash"])))?
                == Some(bytes(&transaction["receipt_rlp"])),
            "durable receipt differs from Go execution facts"
        );
    }
    Ok(())
}

/// Read-only diagnostic of this tiny exclusively created fixture. This is not
/// an execution/storage port: it compares every historical row, including
/// intermediate nodes and period suffixes, with the independently projected Go
/// PendingBlockState writes. Actual Go RocksDB reopen remains a separate gate.
fn verify_physical_rows(path: &Path, expected: &Value) -> Result<()> {
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, path)?;
    let descriptors = columns
        .iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    for column in 1..=5 {
        let handle = db.cf_handle(&column.to_string()).unwrap();
        let actual = db
            .iterator_cf(handle, rocksdb::IteratorMode::Start)
            .map(|row| {
                row.map(|(key, value)| (hex::encode(key), Value::String(hex::encode(value))))
            })
            .collect::<Result<serde_json::Map<String, Value>, _>>()?;
        ensure!(
            Value::Object(actual) == expected[column - 1],
            "Go physical CF{column} rows differ"
        );
    }
    Ok(())
}

#[test]
fn signed_periods_commit_through_final_chain_and_continue_after_reopen() -> Result<()> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/s4_public.json"
    ))?;
    let path = std::env::temp_dir().join(format!("rustaxa-evm-s4-period-{}", std::process::id()));
    std::fs::create_dir(&path)?;
    let application_path = path.join("application");
    let concrete_path = path.join("state_db");
    let mut expected_identity = None;
    let mut database_identity = None;
    for (index, period) in fixture["periods"].as_array().unwrap().iter().enumerate() {
        let (application, chain) = open_chain(&application_path, &fixture)?;
        if index > 0 {
            verify_receipts(&application, &fixture["periods"][index - 1])?;
        }
        let concrete = if let Some(expected) = expected_identity {
            ConcreteStateLifecycle::open(
                &concrete_path,
                chain.concrete_chain_identity()?,
                expected,
            )?
        } else {
            ConcreteStateLifecycle::create_fresh_exclusive(
                &concrete_path,
                chain.concrete_chain_identity()?,
                ConcreteStateMutationBatch {
                    accounts: vec![ConcreteAccountMutation::Upsert {
                        address: fixed(&fixture["inputs"]["sender"]),
                        record: record(
                            FinalChainNonce::zero(),
                            ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                            None,
                            0,
                        ),
                    }],
                    ..Default::default()
                },
                Vec::new(),
            )?
        };
        let observed = concrete.observation()?;
        if let Some(identity) = database_identity {
            ensure!(
                observed.identity == identity,
                "database identity changed on reopen"
            );
        } else {
            database_identity = Some(observed.identity);
        }
        ensure!(
            observed.committed.state_root == fixed::<32>(&period["prior_root"]),
            "Go prior root"
        );
        verify_physical_rows(
            &concrete_path,
            if index == 0 {
                &fixture["genesis"]["rows"]
            } else {
                &fixture["periods"][index - 1]["rows"]
            },
        )?;
        let adapter = Adapter {
            chain: &chain,
            application: &application,
            concrete: RefCell::new(Some(concrete)),
            staged: RefCell::new(None),
            fixture: period,
        };
        recover_final_chain_application_state(&chain, &adapter)?;
        ensure!(
            chain
                .validate_pending_external_evm_commit(
                    &FinalChainExternalEvmStateCommitIntent::default()
                )
                .is_err(),
            "missing durable intent accepted"
        );
        let report = execute_final_chain_application_task(
            &chain,
            request(period)?,
            FinalChainProposalPeriodDagLevelUpdate::default(),
            false,
            [0x99; 20],
            &adapter,
        )?;
        ensure!(
            report.period.as_u64() == number(&period["period"]),
            "published period"
        );
        ensure!(
            application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none(),
            "publication marker remains"
        );
        let header = application
            .final_chain()
            .block_header_raw(report.period.as_u64())?
            .unwrap();
        let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header))?;
        ensure!(
            header.state_root.0 == fixed::<32>(&period["root"]),
            "published Go root"
        );
        ensure!(
            header.gas_used.as_u64()
                == period["transactions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|tx| number(&tx["gas_used"]))
                    .sum::<u64>(),
            "published gas"
        );
        ensure!(
            header.total_reward == rustaxa_types::DposTokenAmount::zero(),
            "neutral reward fixture"
        );
        ensure!(
            chain.block_hash(report.period)?.as_deref() == Some(report.block_hash.as_slice()),
            "published block hash"
        );
        verify_receipts(&application, period)?;
        expected_identity = Some(ConcreteStateIdentity {
            period: report.period,
            state_root: header.state_root.0,
        });
        drop(adapter);
        drop(chain);
        drop(application);
        verify_physical_rows(&concrete_path, &period["rows"])?;
        let reader =
            ConcreteStateReader::open_read_only(&concrete_path, expected_identity.unwrap())?;
        for account in period["accounts"].as_array().unwrap() {
            let ConcreteRead::Present(actual) = reader.account(fixed(&account["address"]))? else {
                bail!("fixture account missing after reopen")
            };
            ensure!(
                actual.physical_rlp == bytes(&account["raw_account"]),
                "Go physical account bytes"
            );
            if number(&account["code_size"]) != 0 {
                ensure!(
                    reader.code(fixed(&account["code_hash"]))?
                        == ConcreteRead::Present(bytes(&account["code"])),
                    "Go runtime code after reopen"
                );
                ensure!(
                    reader.storage(fixed(&account["address"]), ConcreteStorageKey([0; 32]))?
                        == ConcreteRead::Present(bytes(&account["slot_zero"])),
                    "Go slot bytes after reopen"
                );
            }
        }
    }
    std::fs::remove_dir_all(path)?;
    Ok(())
}
