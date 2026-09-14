//! First complete mixed-workload period through the existing FinalChain owner.
//!
//! This finite fixture creates both databases exclusively, hydrates the Rust
//! genesis snapshot from the independently generated concrete rows, executes
//! four periods through the real EVM driver and bound native rewards
//! session, and commits via ordered concrete lifecycle phases. It then closes
//! and reopens both databases and compares receipts, roots, catalog values and CF1--CF5 to
//! the pinned Go observer. This is test composition, not production routing or
//! imported-state qualification.

#[path = "support/mixed_genesis.rs"]
mod mixed_genesis;
#[path = "support/mixed_native.rs"]
mod mixed_native;
#[path = "support/state_api_epoch.rs"]
mod state_api_epoch;

use anyhow::{Context, Result, bail, ensure};
use ethereum_types::{H160, H256, U256};
use k256::ecdsa::SigningKey;
use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::{Rlp, RlpStream};
use rustaxa_consensus::concrete_state_projection::*;
use rustaxa_consensus::final_chain_execution::*;
use rustaxa_consensus::{
    ConsensusExecutionPort, FinalChain, PillarAnchorStateReport, PillarAnchorStateRequest,
    RewardCertVoteFact,
    native_projection_context::{
        FinalChainNativeInvocationContext, FinalChainNativeProjectionContext,
        FinalChainNativeRewardsContext,
    },
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ConsensusNativeDisposition, ConsensusNativeObservation, ExecutionBlockContext,
        ExecutionTransactionKind, NativeCallKind, NativeOutcome, NativeRawOperation,
        TransactionExecutionResult,
    },
    driver::{
        NativeAddressClassifier, PeriodConsensusSequence, execute_top_level_call_with_native,
        execute_top_level_create_with_native,
    },
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    journal::{ExecutionJournal, JournalAccountOperation},
    profile::TaraxaProfile,
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteStateReader, ConcreteStorageMutation,
    Storage,
};
use rustaxa_storage::{
    ConcreteCommitApproval, ConcreteObserverPhaseDelta, ConcreteObserverPhaseOutput,
    ConcreteStateLifecycle, PreparedConcreteView,
};
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    decode_concrete_storage_catalog, encode_concrete_storage_catalog,
};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
use rustaxa_types::concrete_state::*;
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce, FinalizationDagBlock, FinalizationTransaction,
    LegacyTransactionEnvelope, StoredFinalChainBlockHeader,
};
use serde_json::Value;
use state_api_epoch::StateApiEpoch;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mixed_genesis::{fresh_concrete, open_chain};
use mixed_native::MixedNativeExecutionPort;

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("fixture hex string")).unwrap()
}
fn fixed<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().unwrap()
}
fn number(value: &Value) -> u64 {
    value.as_u64().expect("fixture integer")
}
fn string(value: &Value) -> &str {
    value.as_str().expect("fixture string")
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

/// A fixed durable or unpublished base; unpublished views expose execution
/// reads only and cannot impersonate a committed descriptor.
enum FixturePrior<'a> {
    Committed(&'a dyn ConcreteStateRead),
    Prepared(PreparedConcreteView<'a>),
}
/// Immutable per-transaction view over the fixture's exclusively created state.
/// Missing physical slots are known absent only because this test owns the full
/// finite creation/mutation history across both closes. No imported reader can
/// obtain this capability. Account/code corruption continues to fail normally.
/// The ordered path bypasses the cumulative adapter's account/slot/code caches.
struct FixtureView<'a> {
    prior: FixturePrior<'a>,
    identity: ConcreteStateIdentity,
}
impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for FixtureView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        match &self.prior {
            FixturePrior::Committed(prior) => prior.account(address),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                    prior, address,
                )
            }
        }
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let result = match &self.prior {
            FixturePrior::Committed(prior) => prior.storage(address, key),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    prior, address, key,
                )
            }
        };
        match result {
            Err(ConcreteReadError::HistoryUnavailable(_)) => Ok(ConcreteRead::Absent),
            result => result,
        }
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match &self.prior {
            FixturePrior::Committed(prior) => prior.code(hash),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::code(prior, hash)
            }
        }
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
/// Complete post-Ficus, pre-Cacti native registry used by the Go workload.
struct AllNatives;
impl NativeAddressClassifier for AllNatives {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address[..19] == [0; 19] && matches!(address[19], 1..=9 | 0xee | 0xfe)
    }
}
/// Consensus-owned subset; stateless addresses are handled directly by the driver.
struct ConsensusNatives;
impl NativeAddressClassifier for ConsensusNatives {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address[..19] == [0; 19] && address[19] == 0xfe
    }
}
struct Staged {
    prepared: ConcreteObserverPhaseOutput,
    projection: FinalChainConcreteStateProjection,
    provenance: Vec<u8>,
    catalog_rlp: Vec<u8>,
}

struct NativeContextParts {
    request_id: [u8; 32],
    projection_hash: [u8; 32],
    invocations: Vec<FinalChainNativeInvocationContext>,
    rewards: FinalChainNativeRewardsContext,
    raw_slots: BTreeSet<ConcreteStorageSlot>,
}

struct Adapter<'a> {
    chain: &'a FinalChain,
    application: &'a Storage,
    concrete: RefCell<Option<ConcreteStateLifecycle>>,
    staged: RefCell<Option<Staged>>,
    native: RefCell<Option<MixedNativeExecutionPort<'a>>>,
    native_context: RefCell<Option<NativeContextParts>>,
    state_api_epoch: StateApiEpoch,
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
            state_api_epoch: self.state_api_epoch.current(),
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
        self.state_api_epoch.validate(request.state_api_epoch)?;
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable"))?;
        concrete.stage_execution(&request.concrete_marker_rlp)?;
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        let expected = self.fixture["transactions"]
            .as_array()
            .context("period transactions")?;
        ensure!(
            request.transactions.len() == expected.len(),
            "fixture transaction count"
        );

        let session = self.chain.begin_native_session_bound_at_state_api_epoch(
            request.request_id,
            request.state_api_epoch,
            request.period,
            request.prior_state.period,
        )?;
        let mut native = MixedNativeExecutionPort::new(session);
        let mut sequence = PeriodConsensusSequence::new(request.period);
        let mut prepared: Option<ConcreteObserverPhaseOutput> = None;
        let mut identity = concrete.observation()?.committed;
        let mut raw_catalog =
            decode_concrete_storage_catalog(&concrete.observation()?.catalog_rlp)?
                .into_iter()
                .collect::<BTreeSet<_>>();
        let mut native_contexts = Vec::new();
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
                prior: match prepared.as_ref() {
                    Some(output) => FixturePrior::Prepared(concrete.prepared_view(output)?),
                    None => FixturePrior::Committed(concrete.prior_reader()),
                },
                identity,
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
                ExecutionTransactionKind::Call => execute_top_level_call_with_native(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &AllNatives,
                    &ConsensusNatives,
                    &mut native,
                    &mut sequence,
                    &block,
                    &decoded,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )?,
                ExecutionTransactionKind::Create => execute_top_level_create_with_native(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &AllNatives,
                    &ConsensusNatives,
                    &mut native,
                    &mut sequence,
                    &block,
                    &decoded,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )?,
                other => bail!("fixture unsupported transaction kind: {other:?}"),
            };
            let TransactionExecutionResult::Executed(result) = result else {
                bail!("fixture consensus failure")
            };
            let expected_error = string(&expected["execution_error"]);
            ensure!(
                code_error(&result.status) == expected_error,
                "Go execution status"
            );
            ensure!(
                result.output == bytes(&expected["output"]),
                "Go execution output"
            );
            ensure!(
                result.attempted_contract_address.unwrap_or([0; 20])
                    == fixed_or_zero(&expected["created"]),
                "Go created address"
            );
            ensure!(
                result.gas_used.as_u64() == number(&expected["gas_used"]),
                "Go gas used"
            );
            ensure!(
                result.refund_applied.as_u64() == number(&expected["gas_refund"]["applied"]),
                "Go applied refund"
            );
            let settled = journal.settle_transaction()?;
            drop(journal);
            ensure!(
                settled.logs == result.logs,
                "settled logs differ from result"
            );
            ensure!(
                settled.refund == number(&expected["gas_refund"]["counter"]),
                "Go refund counter"
            );

            let call_contexts = native.take_contexts();
            ensure!(
                call_contexts.len() == settled.native_invocations.len(),
                "native context/observation count"
            );
            compare_raw_trace(&call_contexts, &expected["ordered_raw_writes"])?;
            for context in &call_contexts {
                for mutation in &context.raw_mutations {
                    raw_catalog.insert(ConcreteStorageSlot {
                        address: mutation.address,
                        key: mutation.key.0,
                    });
                }
            }
            native_contexts.extend(call_contexts);

            cumulative = cumulative
                .checked_add(result.gas_used.as_u64())
                .context("cumulative gas overflow")?;
            ensure!(
                cumulative == number(&expected["cumulative_gas_used"]),
                "Go cumulative gas"
            );
            let mut report = FinalChainEvmTransactionResult {
                position: transaction.position,
                hash: transaction.hash,
                status: u8::from(result.status == CodeExecutionStatus::Success),
                gas_used: result.gas_used,
                cumulative_gas_used: cumulative.into(),
                receipt_rlp: Vec::new(),
                logs: result.logs.iter().map(project_log).collect(),
                new_contract_address: result.attempted_contract_address,
                output: result.output.clone(),
                code_error: expected_error.to_owned(),
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
                "Go StateAPI transaction bytes"
            );
            ensure!(
                encode_concrete_execution_result(&report)
                    == bytes(&expected["state_api_execution_result_rlp"]),
                "Go StateAPI execution-result bytes"
            );

            let mut changed = BTreeSet::new();
            let mut accounts = Vec::new();
            let mut storage = Vec::new();
            let mut code = Vec::new();
            for write in settled.writes.accounts {
                changed.insert(write.address);
                accounts.push(match write.operation {
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
                });
            }
            for write in settled.writes.ordinary_storage {
                changed.insert(write.address);
                storage.push(ConcreteStorageMutation {
                    address: write.address,
                    key: write.key,
                    value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
                });
            }
            let mut raw_touched = BTreeSet::new();
            for write in settled.writes.raw_storage {
                changed.insert(write.address);
                raw_touched.insert((write.address, write.key));
                raw_catalog.insert(ConcreteStorageSlot {
                    address: write.address,
                    key: write.key.0,
                });
                storage.push(ConcreteStorageMutation {
                    address: write.address,
                    key: write.key,
                    value: raw_operation_value(&write.operation),
                });
            }
            for write in settled.writes.code {
                code.push(ConcreteCodeInsertion {
                    code_hash: write.code_hash,
                    code: write.code,
                });
            }
            let next = concrete.apply_observer_phase(ConcreteObserverPhaseDelta {
                accounts,
                storage,
                code,
            })?;
            identity = next.identity();
            ensure!(
                identity.state_root == fixed(&expected["observer"]["root"]),
                "Go intermediate root"
            );
            let observed_changed = next
                .changed_accounts()
                .iter()
                .map(|entry| entry.address)
                .collect::<BTreeSet<_>>();
            ensure!(
                observed_changed.is_subset(&changed),
                "writer reported an account absent from the settled delta period={} tx={}: observed={observed_changed:?} settled={changed:?}",
                request.period.as_u64(),
                transaction.position.as_u32(),
            );
            let view = FixtureView {
                prior: FixturePrior::Prepared(concrete.prepared_view(&next)?),
                identity: next.identity(),
            };
            let effect_accounts = changed
                .iter()
                .map(|address| {
                    Ok(FinalChainConcreteAccountProjection {
                        address: *address,
                        raw_account_rlp: account_bytes(
                            &rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                                &view, *address,
                            )?,
                        ),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let effect_storage = raw_touched
                .into_iter()
                .map(|(address, key)| {
                    Ok(FinalChainConcreteStorageProjection {
                        contract: address,
                        key: key.0,
                        value: read_raw_projection(&view, address, key)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let invocations = settled
                .native_invocations
                .iter()
                .map(project_invocation)
                .collect::<Result<Vec<_>>>()?;
            effects.push(FinalChainConcreteTransactionEffect {
                index: u64::from(transaction.position.as_u32()),
                transaction_rlp: encode_concrete_evm_transaction(transaction),
                execution_result_rlp: encode_concrete_execution_result(&report),
                intermediate_state: FinalChainConcreteState {
                    period: request.period.as_u64(),
                    root: identity.state_root,
                },
                accounts: effect_accounts,
                storage: effect_storage,
                invocations,
            });
            prepared = Some(next);
            results.push(report);
        }

        let prepared = prepared.context("fixture period has no transaction preparation")?;
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
            accounts: Vec::new(),
            storage: Vec::new(),
            invocations: Vec::new(),
            rewards_input: Vec::new(),
            catalog_hash: concrete_storage_catalog_hash(&[]),
        };
        let period_raw_slots = native_contexts
            .iter()
            .flat_map(|context| {
                context
                    .raw_reads
                    .iter()
                    .map(|read| (read.address, read.key))
                    .chain(
                        context
                            .raw_mutations
                            .iter()
                            .map(|mutation| (mutation.address, mutation.key)),
                    )
            })
            .map(|(address, key)| ConcreteStorageSlot {
                address,
                key: key.0,
            })
            .collect();
        *self.native.borrow_mut() = Some(native);
        *self.native_context.borrow_mut() = Some(NativeContextParts {
            request_id: request.request_id,
            projection_hash: [0; 32],
            invocations: native_contexts,
            rewards: FinalChainNativeRewardsContext::default(),
            raw_slots: period_raw_slots,
        });
        *self.staged.borrow_mut() = Some(Staged {
            prepared,
            projection,
            provenance: Vec::new(),
            catalog_rlp: encode_concrete_storage_catalog(raw_catalog.iter().copied()),
        });
        Ok(FinalChainEvmExecutionReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
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
        _request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        bail!("mixed fixture requires the opaque native rewards plan")
    }

    fn distribute_final_chain_rewards_with_native_plan(
        &self,
        request: &FinalChainEvmRewardsRequest,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
    ) -> Result<FinalChainEvmRewardsReport> {
        self.state_api_epoch.validate(request.state_api_epoch)?;
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .context("fixture concrete handle unavailable")?;
        let mut staged = self.staged.borrow_mut();
        let staged = staged.as_mut().context("fixture execution is not staged")?;
        ensure!(
            staged.projection.post_transaction_state.root == request.post_transaction_state_root,
            "rewards prior root"
        );
        let prior_identity = staged.prepared.identity();
        let view = FixtureView {
            prior: FixturePrior::Prepared(concrete.prepared_view(&staged.prepared)?),
            identity: prior_identity,
        };
        let mut journal = ExecutionJournal::new(view);
        let mut native = self.native.borrow_mut();
        let native = native
            .as_mut()
            .context("bound native session unavailable")?;
        let (native_rewards, projected, reward_context) = native.finish(plan, &journal)?;
        ensure!(projected.status == rustaxa_evm::contracts::NativeStatus::Success);
        apply_native_outcome(&mut journal, &projected)?;
        let settled = journal.settle_transaction()?;
        drop(journal);
        ensure!(settled.logs.is_empty() && settled.native_invocations.is_empty());
        ensure!(
            native_rewards.total_reward.as_u256()
                == decimal_u256(&self.fixture["reward_output"]["minted_reward"]),
            "Go minted reward"
        );
        compare_reward_raw_trace(
            &reward_context,
            &self.fixture["ordered_raw_writes"]["rewards"],
            &self.fixture["ordered_raw_writes"]["end_block"],
        )?;

        let mut changed = BTreeSet::new();
        let mut accounts = Vec::new();
        let mut storage = Vec::new();
        let mut code = Vec::new();
        for write in settled.writes.accounts {
            changed.insert(write.address);
            accounts.push(match write.operation {
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
            });
        }
        for write in settled.writes.ordinary_storage {
            changed.insert(write.address);
            storage.push(ConcreteStorageMutation {
                address: write.address,
                key: write.key,
                value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
            });
        }
        for write in settled.writes.raw_storage {
            changed.insert(write.address);
            storage.push(ConcreteStorageMutation {
                address: write.address,
                key: write.key,
                value: raw_operation_value(&write.operation),
            });
        }
        for write in settled.writes.code {
            code.push(ConcreteCodeInsertion {
                code_hash: write.code_hash,
                code: write.code,
            });
        }
        let final_output = concrete.apply_observer_phase(ConcreteObserverPhaseDelta {
            accounts,
            storage,
            code,
        })?;
        ensure!(
            final_output.identity().state_root == fixed(&self.fixture["final"]["root"]),
            "Go post-rewards root"
        );
        ensure!(
            final_output
                .changed_accounts()
                .iter()
                .map(|entry| entry.address)
                .collect::<BTreeSet<_>>()
                == changed,
            "reward changed-account set"
        );

        let mut account_identities = staged
            .projection
            .transaction_effects
            .iter()
            .flat_map(|effect| effect.accounts.iter().map(|entry| entry.address))
            .collect::<BTreeSet<_>>();
        account_identities.extend(changed);
        let expected_accounts =
            catalog_account_identities(&self.fixture["period_catalog_identities"]["accounts"])?;
        ensure!(
            account_identities == expected_accounts,
            "Go period account catalog period={}: actual={account_identities:?} expected={expected_accounts:?}",
            request.period.as_u64(),
        );

        let mut catalog = decode_concrete_storage_catalog(&staged.catalog_rlp)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut period_raw_slots = self
            .native_context
            .borrow()
            .as_ref()
            .context("native context accumulator unavailable")?
            .raw_slots
            .clone();
        for read in &reward_context.raw_reads {
            let slot = ConcreteStorageSlot {
                address: read.address,
                key: read.key.0,
            };
            catalog.insert(slot);
            period_raw_slots.insert(slot);
        }
        for mutation in &native_rewards.raw_mutations {
            let slot = ConcreteStorageSlot {
                address: mutation.address,
                key: mutation.key.0,
            };
            catalog.insert(slot);
            period_raw_slots.insert(slot);
        }
        let expected_slots =
            catalog_storage_identities(&self.fixture["period_catalog_identities"]["slots"])?;
        let declared_absence_probes = catalog_storage_identities(
            &self.fixture["final"]["native_catalog"]["coverage"]["absent_slot_identities"],
        )?;
        // The Go period catalog also carries declared absence probes. They are
        // lifecycle coverage facts and need not equal the implementation's
        // consumed reward reads, which the native sidecar validates directly.
        let mut explained_period_slots = catalog.clone();
        explained_period_slots.extend(period_raw_slots.iter().copied());
        explained_period_slots.extend(declared_absence_probes.iter().copied());
        ensure!(
            expected_slots.is_subset(&explained_period_slots),
            "Go period catalog identity is neither prior, consumed, mutated nor a declared absence probe"
        );
        catalog.extend(declared_absence_probes.iter().copied());
        catalog.extend(period_raw_slots);
        let expected_catalog =
            catalog_storage_identities(&self.fixture["final"]["native_catalog"]["slots"])?;
        ensure!(catalog == expected_catalog, "Go full raw catalog");
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        // Replace the temporary pre-rewards catalog hash only after every raw
        // key is known and every final value can be read from one prepared view.
        let view = FixtureView {
            prior: FixturePrior::Prepared(concrete.prepared_view(&final_output)?),
            identity: final_output.identity(),
        };
        for slot in &declared_absence_probes {
            ensure!(
                matches!(
                    rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                        &view,
                        slot.address,
                        ConcreteStorageKey(slot.key),
                    )?,
                    ConcreteRead::Absent | ConcreteRead::Tombstone
                ),
                "fixture-declared raw absence probe is present"
            );
        }
        let projection_accounts = account_identities
            .iter()
            .map(|address| {
                Ok(FinalChainConcreteAccountProjection {
                    address: *address,
                    raw_account_rlp: account_bytes(
                        &rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                            &view, *address,
                        )?,
                    ),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let projection_storage = catalog
            .iter()
            .map(|slot| {
                let key = ConcreteStorageKey(slot.key);
                Ok(FinalChainConcreteStorageProjection {
                    contract: slot.address,
                    key: slot.key,
                    value: read_raw_projection(&view, slot.address, key)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let catalog_hash = concrete_storage_catalog_hash(&projection_storage);
        let projection_invocations = staged
            .projection
            .transaction_effects
            .iter()
            .flat_map(|effect| effect.invocations.iter().cloned())
            .collect();
        staged.projection.post_rewards_state = FinalChainConcreteState {
            period: request.period.as_u64(),
            root: final_output.identity().state_root,
        };
        staged.projection.accounts = projection_accounts;
        staged.projection.storage = projection_storage;
        staged.projection.invocations = projection_invocations;
        staged.projection.rewards_input =
            encode_concrete_rewards_input(&request.distribution_stats);
        staged.projection.catalog_hash = catalog_hash;
        staged.prepared = final_output;
        staged.catalog_rlp = catalog_rlp;

        let projection = encode_concrete_state_projection(&staged.projection);
        let projection_hash = concrete_state_bytes_digest(&projection);
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        staged.provenance = encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: staged.projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        });
        {
            let mut stored = self.native_context.borrow_mut();
            let stored = stored
                .as_mut()
                .context("native context accumulator unavailable")?;
            stored.rewards = reward_context;
            stored.projection_hash = projection_hash;
        }
        let mut wrong_request = request.request_id;
        wrong_request[0] ^= 1;
        ensure!(
            ConsensusExecutionPort::final_chain_native_projection_context(
                self,
                wrong_request,
                projection_hash,
            )
            .is_err(),
            "wrong native context request identity was accepted"
        );
        let mut wrong_projection = projection_hash;
        wrong_projection[0] ^= 1;
        ensure!(
            ConsensusExecutionPort::final_chain_native_projection_context(
                self,
                request.request_id,
                wrong_projection,
            )
            .is_err(),
            "wrong native context projection hash was accepted"
        );
        Ok(FinalChainEvmRewardsReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            period: request.period,
            status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: staged.projection.post_rewards_state.root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            concrete_projection_rlp: projection,
            concrete_projection_hash: projection_hash,
            concrete_provenance_rlp: staged.provenance.clone(),
            total_reward: minimal_u256(native_rewards.total_reward.as_u256()),
        })
    }

    fn final_chain_native_projection_context(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<FinalChainNativeProjectionContext>> {
        let mut stored = self.native_context.borrow_mut();
        let parts = stored
            .as_ref()
            .context("native context requested before rewards")?;
        ensure!(
            parts.request_id == request_id,
            "native context request identity mismatch"
        );
        ensure!(
            parts.projection_hash == projection_hash,
            "native context projection hash mismatch"
        );
        let parts = stored.take().expect("validated stored native context");
        Ok(Some(FinalChainNativeProjectionContext {
            request_id: parts.request_id,
            projection_hash: parts.projection_hash,
            invocations: parts.invocations,
            rewards: parts.rewards,
        }))
    }
    fn commit_final_chain_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        self.state_api_epoch.validate(request.state_api_epoch)?;
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
        let mut missing_epoch = request.clone();
        missing_epoch.state_api_epoch = 0;
        ensure!(
            self.chain
                .validate_pending_external_evm_commit(&missing_epoch)
                .is_err(),
            "zero-epoch intent accepted"
        );
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
        let approval = ConcreteCommitApproval {
            marker_rlp: request.concrete_marker_rlp.clone(),
            provenance_rlp: request.concrete_provenance_rlp.clone(),
            catalog_rlp: staged.catalog_rlp,
            projection_hash: request.concrete_projection_hash,
            catalog_hash: staged.projection.catalog_hash,
        };
        let observed = concrete.commit_observer_approved(staged.prepared, approval)?;
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.provenance_rlp == request.concrete_provenance_rlp,
            "concrete commit observation"
        );
        Ok(FinalChainExternalEvmStateCommitResult {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
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
        self.state_api_epoch
            .validate(request.expected_state_api_epoch)?;
        self.staged.borrow_mut().take();
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture ambiguous consumed commit"))?;
        ensure!(
            concrete_state_bytes_digest(&request.concrete_marker_rlp) == request.marker_hash,
            "fixture discard marker hash"
        );
        concrete.discard_execution(&request.concrete_marker_rlp)?;
        let observed = concrete.observation()?;
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.committed.period == request.prior_state.period
                && observed.committed.state_root == request.prior_state.state_root,
            "fixture discard reopened prior descriptor"
        );
        let (previous_state_api_epoch, state_api_epoch) = self
            .state_api_epoch
            .replace_after_discard(request.expected_state_api_epoch)?;
        Ok(FinalChainExternalEvmDiscardReport {
            request_id: request.request_id,
            previous_state_api_epoch,
            state_api_epoch,
            period: request.period,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            marker_hash: request.marker_hash,
            prior_state: request.prior_state,
            committed_state: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
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

fn project_log(log: &rustaxa_evm::contracts::ExecutionLog) -> FinalChainEvmLog {
    FinalChainEvmLog {
        address: log.address,
        topics: log
            .topics
            .iter()
            .map(|topic| FinalChainEvmLogTopic { topic: *topic })
            .collect(),
        data: log.data.clone(),
    }
}

fn project_invocation(
    observation: &ConsensusNativeObservation,
) -> Result<FinalChainConcreteInvocation> {
    let error = code_error(&observation.status);
    let call_type = match observation.invocation.kind {
        NativeCallKind::Call => 0,
        NativeCallKind::CallCode => 1,
        NativeCallKind::DelegateCall => 2,
        NativeCallKind::StaticCall => 3,
    };
    let disposition = match observation.disposition {
        ConsensusNativeDisposition::Normal => FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL,
        ConsensusNativeDisposition::OwnFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED
        }
        ConsensusNativeDisposition::OuterFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
        }
    };
    Ok(FinalChainConcreteInvocation {
        transaction_index: u64::from(observation.invocation.id.transaction.as_u32()),
        sequence: observation.invocation.id.sequence,
        depth: observation.invocation.depth,
        call_type,
        caller: observation.invocation.caller,
        contract: observation.invocation.contract,
        value: observation.invocation.value.value().to_bytes_be(),
        input: observation.invocation.input.clone(),
        output: observation.output.clone(),
        supplied_gas: observation.invocation.supplied_gas.as_u64(),
        required_gas: observation.required_gas.as_u64(),
        gas_used: observation.gas_used.as_u64(),
        error,
        logs: observation.logs.iter().map(project_log).collect(),
        disposition,
    })
}

fn code_error(status: &CodeExecutionStatus) -> String {
    match status {
        CodeExecutionStatus::Success => String::new(),
        CodeExecutionStatus::Failure(CodeExecutionError::Revert) => "execution reverted".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) => error.error.clone(),
        CodeExecutionStatus::Failure(other) => format!("{other:?}"),
    }
}

fn raw_operation_value(operation: &NativeRawOperation) -> Option<Vec<u8>> {
    match operation {
        NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
        NativeRawOperation::Delete => None,
    }
}

fn apply_native_outcome<R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    outcome: &NativeOutcome,
) -> Result<()> {
    journal.apply_native_account_mutations(&outcome.account_mutations)?;
    for mutation in &outcome.raw_mutations {
        ensure!(
            journal.raw_storage(mutation.address, mutation.key)? == mutation.expected,
            "reward raw mutation expected classification"
        );
        journal.set_raw_storage(mutation.address, mutation.key, mutation.operation.clone())?;
    }
    Ok(())
}

fn account_bytes(read: &ConcreteRead<ConcreteAccountRecord>) -> Vec<u8> {
    match read {
        ConcreteRead::Present(record) => record.physical_rlp.clone(),
        ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
    }
}

fn read_raw_projection<R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead>(
    view: &R,
    address: [u8; 20],
    key: ConcreteStorageKey,
) -> Result<Vec<u8>> {
    Ok(
        match rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            view, address, key,
        )? {
            ConcreteRead::Present(value) => value,
            ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
        },
    )
}

fn compare_raw_trace(
    contexts: &[FinalChainNativeInvocationContext],
    expected: &Value,
) -> Result<()> {
    let actual = contexts
        .iter()
        .flat_map(|context| context.raw_mutations.iter());
    compare_raw_mutations(actual, expected)
}

fn compare_reward_raw_trace(
    context: &FinalChainNativeRewardsContext,
    rewards: &Value,
    end_block: &Value,
) -> Result<()> {
    let expected = rewards
        .as_array()
        .context("reward raw trace")?
        .iter()
        .chain(end_block.as_array().context("end-block raw trace")?);
    let actual = context.raw_mutations.iter();
    ensure!(
        actual.len() == expected.clone().count(),
        "reward raw trace count"
    );
    for (actual, expected) in actual.zip(expected) {
        compare_raw_mutation(actual, expected)?;
    }
    Ok(())
}

fn compare_raw_mutations<'a>(
    actual: impl Iterator<Item = &'a rustaxa_consensus::native_session::FinalChainNativeRawMutation>,
    expected: &Value,
) -> Result<()> {
    let expected = expected.as_array().context("transaction raw trace")?;
    let actual = actual.collect::<Vec<_>>();
    ensure!(
        actual.len() == expected.len(),
        "transaction raw trace count"
    );
    for (actual, expected) in actual.into_iter().zip(expected) {
        compare_raw_mutation(actual, expected)?;
    }
    Ok(())
}

fn compare_raw_mutation(
    actual: &rustaxa_consensus::native_session::FinalChainNativeRawMutation,
    expected: &Value,
) -> Result<()> {
    ensure!(
        actual.address == fixed(&expected["address"]),
        "raw trace address"
    );
    ensure!(actual.key.0 == fixed(&expected["key"]), "raw trace key");
    let value = bytes(&expected["value"]);
    match &actual.operation {
        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Put(actual) => {
            ensure!(actual.as_bytes() == value, "raw trace put bytes")
        }
        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Delete => {
            ensure!(value.is_empty(), "raw trace delete bytes")
        }
    }
    Ok(())
}

fn catalog_account_identities(value: &Value) -> Result<BTreeSet<[u8; 20]>> {
    value
        .as_array()
        .context("account catalog")?
        .iter()
        .map(|value| fixed_prefixed(value.as_str().context("account identity")?))
        .collect()
}

fn catalog_storage_identities(value: &Value) -> Result<BTreeSet<ConcreteStorageSlot>> {
    value
        .as_array()
        .context("storage catalog")?
        .iter()
        .map(|entry| {
            Ok(ConcreteStorageSlot {
                address: fixed_prefixed(string(&entry["address"]))?,
                key: fixed_prefixed(string(&entry["key"]))?,
            })
        })
        .collect()
}

fn fixed_prefixed<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = hex::decode(value.strip_prefix("0x").unwrap_or(value))?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow::anyhow!("expected {N} bytes, got {}", bytes.len()))
}

fn decimal_u256(value: &Value) -> U256 {
    U256::from_dec_str(string(value)).expect("fixture uint256")
}

fn minimal_u256(value: U256) -> Vec<u8> {
    value
        .to_big_endian()
        .into_iter()
        .skip_while(|byte| *byte == 0)
        .collect()
}

fn fixed_or_zero<const N: usize>(value: &Value) -> [u8; N] {
    let value = string(value);
    if value.is_empty() || value.chars().all(|character| character == '0') {
        [0; N]
    } else {
        fixed_prefixed(value).expect("fixture fixed-width hex")
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
    let key = SigningKey::from_slice(&[9; 32]).expect("fixture signing key");
    let (signature, recovery) = key
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .expect("fixture PBFT signature");
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut signed = RlpStream::new_list(8);
    fields(&mut signed, period);
    signed.append(&signature);
    signed.out().to_vec()
}

fn period_data(request: &FinalChainExecutionRequest) -> Vec<u8> {
    let mut stream = RlpStream::new_list(4);
    stream.append_raw(&request.pbft_block_rlp, 1);
    stream.begin_list(0);
    stream.begin_list(0);
    stream.begin_list(request.transactions.len());
    for transaction in &request.transactions {
        stream.append_raw(&transaction.rlp, 1);
    }
    stream.out().to_vec()
}

fn request(root: &Value, period: &Value) -> Result<FinalChainExecutionRequest> {
    let transactions = period["transactions"]
        .as_array()
        .context("period transactions")?
        .iter()
        .map(|transaction| {
            let raw = bytes(&transaction["signed_rlp"]);
            let decoded = LegacyTransactionEnvelope::decode(&raw)?;
            ensure!(decoded.signature_valid, "fixture signature");
            Ok(FinalizationTransaction {
                hash: decoded.hash.0,
                sender: decoded.sender.context("signed sender")?.0,
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
    let planner = &period["planner_facts"];
    let dag = planner["dag_blocks"]
        .as_array()
        .context("planner DAG blocks")?
        .iter()
        .map(|block| {
            Ok(FinalizationDagBlock {
                author: fixed(&block["author"]),
                difficulty: number(&block["difficulty"])
                    .try_into()
                    .context("DAG difficulty fits u16")?,
                transaction_hashes: block["transaction_hashes"]
                    .as_array()
                    .context("DAG transaction hashes")?
                    .iter()
                    .map(fixed)
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        dag.iter()
            .flat_map(|block| block.transaction_hashes.iter())
            .copied()
            .collect::<Vec<_>>()
            == transactions
                .iter()
                .map(|transaction| transaction.hash)
                .collect::<Vec<_>>(),
        "planner DAG transaction order"
    );
    let cert_votes = planner["certificate_votes"]
        .as_array()
        .context("certificate votes")?
        .iter()
        .map(|vote| RewardCertVoteFact {
            voter: H160(fixed(&vote["validator"])),
            weight: number(&vote["weight"]),
            period: number(&vote["period"]),
        })
        .collect();
    Ok(FinalChainExecutionRequest {
        pbft_block_rlp: pbft(number(&period["number"])),
        transactions,
        finalized_dag_blocks: dag,
        blocks_per_year: number(&root["configuration"]["dpos"]["blocks_per_year"])
            .try_into()
            .context("blocks per year fits u32")?,
        cert_votes,
        block_gas_limit: number(&root["configuration"]["block_gas_limit"]).into(),
        mode: FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
    })
}

fn load_fixture() -> Result<Value> {
    let path = std::env::var_os("RUSTAXA_MIXED_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../../experiments/evm_feasibility/fixtures/mixed_workload_local_observer.json",
            )
        });
    serde_json::from_str(&std::fs::read_to_string(path)?).context("mixed workload fixture")
}

fn verify_receipts(storage: &Storage, period: &Value) -> Result<()> {
    for transaction in period["transactions"].as_array().context("transactions")? {
        ensure!(
            storage
                .final_chain()
                .receipt_by_trx_hash(H256(fixed(&transaction["hash"])))?
                == Some(bytes(&transaction["receipt_rlp"])),
            "durable receipt differs from Go"
        );
    }
    Ok(())
}

fn verify_physical_rows(path: &Path, expected: &Value) -> Result<()> {
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, path)?;
    let descriptors = columns
        .iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    for column in 1..=5 {
        let handle = db.cf_handle(&column.to_string()).context("concrete CF")?;
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

fn verify_final_reader(reader: &ConcreteStateReader, final_state: &Value) -> Result<()> {
    let view = FixtureView {
        prior: FixturePrior::Committed(reader),
        identity: ConcreteStateRead::identity(reader),
    };
    for account in final_state["accounts"]
        .as_array()
        .context("final accounts")?
    {
        let actual = rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
            &view,
            fixed(&account["address"]),
        )?;
        if account["present"] == Value::Bool(true) {
            let ConcreteRead::Present(actual) = actual else {
                bail!("Go-present final account is missing")
            };
            ensure!(
                actual.physical_rlp == bytes(&account["raw_account"]),
                "final account RLP"
            );
            if number(&account["code_size"]) != 0 {
                ensure!(
                    reader.code(fixed(&account["code_hash"]))?
                        == ConcreteRead::Present(bytes(&account["code"])),
                    "final code bytes"
                );
            }
        } else {
            ensure!(matches!(
                actual,
                ConcreteRead::Absent | ConcreteRead::Tombstone
            ));
        }
    }
    for slot in final_state["native_catalog"]["slots"]
        .as_array()
        .context("final native slots")?
    {
        let actual = rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            &view,
            fixed(&slot["address"]),
            ConcreteStorageKey(fixed(&slot["key"])),
        )?;
        if slot["present"] == Value::Bool(true) {
            ensure!(
                actual == ConcreteRead::Present(bytes(&slot["value"])),
                "final raw slot"
            );
        } else {
            ensure!(
                matches!(actual, ConcreteRead::Absent | ConcreteRead::Tombstone)
                    || actual == ConcreteRead::Present(Vec::new()),
                "final raw deletion"
            );
        }
    }
    Ok(())
}

fn verify_historical_account(reader: &ConcreteStateReader, expected: &Value) -> Result<()> {
    let actual = reader.account(fixed(&expected["address"]))?;
    if expected["present"] == Value::Bool(true) {
        let ConcreteRead::Present(actual) = actual else {
            bail!("Go-present historical account is missing")
        };
        ensure!(
            actual.physical_rlp == bytes(&expected["raw_account"]),
            "historical account RLP"
        );
        if number(&expected["code_size"]) != 0 {
            ensure!(
                reader.code(fixed(&expected["code_hash"]))?
                    == ConcreteRead::Present(bytes(&expected["code"])),
                "historical code bytes"
            );
        }
    } else {
        ensure!(
            matches!(actual, ConcreteRead::Absent | ConcreteRead::Tombstone),
            "Go-absent historical account is present"
        );
    }
    Ok(())
}

fn verify_historical_slot(reader: &ConcreteStateReader, expected: &Value) -> Result<()> {
    let actual = reader.storage(
        fixed(&expected["address"]),
        ConcreteStorageKey(fixed(&expected["key"])),
    )?;
    ensure!(
        actual == ConcreteRead::Present(bytes(&expected["raw_value"])),
        "historical physical slot"
    );
    Ok(())
}

fn historical_reader(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
    period: usize,
) -> Result<ConcreteStateReader> {
    ConcreteStateReader::open_historical_read_only(
        path,
        committed,
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(period as u64),
            state_root: fixed(&fixture["periods"][period - 1]["final"]["root"]),
        },
    )
    .map_err(Into::into)
}

fn verify_historical_reads(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
) -> Result<()> {
    let head = committed.period.as_u64();
    let expected = &fixture["historical_reads"];
    let period_one = historical_reader(path, committed, fixture, 1)?;
    verify_historical_account(&period_one, &expected["period_1_lifecycle_account"])?;
    verify_historical_slot(&period_one, &expected["period_1_lifecycle_slot_0"])?;
    verify_historical_slot(
        &period_one,
        &expected["restored_slot_contract"]["period_1_slot_0"],
    )?;
    verify_historical_slot(
        &period_one,
        &expected["restored_slot_contract"]["period_1_slot_1"],
    )?;
    verify_historical_account(&period_one, &expected["reverted_selfdestruct"]["child"])?;
    verify_historical_account(
        &period_one,
        &expected["reverted_selfdestruct"]["beneficiary"],
    )?;
    drop(period_one);

    if head >= 2 {
        let period_two = historical_reader(path, committed, fixture, 2)?;
        verify_historical_account(&period_two, &expected["period_2_lifecycle_account"])?;
        // Raw physical history deliberately survives after the account is
        // deleted and its logical trie path is absent.
        verify_historical_slot(
            &period_two,
            &expected["period_2_orphaned_lifecycle_storage_row"],
        )?;
        verify_historical_account(
            &period_two,
            &expected["native_custody"]["dispatcher_period_2"],
        )?;
        drop(period_two);

        let retained = &expected["retained_period_1_lifecycle_cf5"];
        ensure!(
            bytes(&retained["physical_key"])
                == [
                    bytes(&retained["logical_key"]),
                    number(&retained["period"]).to_be_bytes().to_vec(),
                ]
                .concat(),
            "retained CF5 physical-key layout"
        );
        let options = rocksdb::Options::default();
        let columns = rocksdb::DB::list_cf(&options, path)?;
        let descriptors = columns
            .iter()
            .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
        let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
        let cf5 = db.cf_handle("5").context("concrete CF5")?;
        ensure!(
            db.get_cf(cf5, bytes(&retained["physical_key"]))? == Some(bytes(&retained["value"])),
            "retained period-one CF5 row"
        );
        drop(db);
    }

    if head >= 4 {
        let period_four = historical_reader(path, committed, fixture, 4)?;
        verify_historical_slot(
            &period_four,
            &expected["restored_slot_contract"]["period_4_slot_0"],
        )?;
        verify_historical_slot(
            &period_four,
            &expected["restored_slot_contract"]["period_4_slot_1"],
        )?;
        verify_historical_account(
            &period_four,
            &expected["native_custody"]["dispatcher_period_4"],
        )?;
    }
    Ok(())
}

fn run_period(
    fixture: &Value,
    period: &Value,
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    expected_chain_identity: Option<[u8; 32]>,
) -> Result<[u8; 32]> {
    let (application, mut chain) = open_chain(application_path, fixture)?;
    ensure!(
        application
            .final_chain()
            .external_evm_pending_publication_raw()?
            .is_none(),
        "application unexpectedly needs recovery"
    );
    let chain_identity = chain.concrete_chain_identity()?;
    if let Some(expected) = expected_chain_identity {
        ensure!(
            chain_identity == expected,
            "reopened concrete chain identity"
        );
    }
    let concrete = if fresh {
        let concrete = fresh_concrete(concrete_path, &chain, fixture)?;
        chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
        concrete
    } else {
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, chain_identity)?;
        let genesis_identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: fixed(&fixture["genesis"]["root"]),
        };
        let genesis_reader = ConcreteStateReader::open_historical_read_only(
            concrete_path,
            observed.committed,
            genesis_identity,
        )?;
        chain.hydrate_concrete_genesis_accounts(&genesis_reader)?;
        drop(genesis_reader);
        ConcreteStateLifecycle::open(concrete_path, chain_identity, observed.committed)?
    };
    let adapter = Adapter {
        chain: &chain,
        application: &application,
        concrete: RefCell::new(Some(concrete)),
        staged: RefCell::new(None),
        native: RefCell::new(None),
        native_context: RefCell::new(None),
        state_api_epoch: StateApiEpoch::new(),
        fixture: period,
    };
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &adapter)?;
    let author = fixed(&period["reward_input"]["block_author"]);
    let execution_request = request(fixture, period)?;
    chain.ensure_period_data(
        FinalChainBlockNumber::new(number(&period["number"])),
        &period_data(&execution_request),
    )?;
    let report = execute_final_chain_application_task(
        &chain,
        execution_request,
        FinalChainProposalPeriodDagLevelUpdate::default(),
        false,
        fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
        &adapter,
    )?;
    let period_number = FinalChainBlockNumber::new(number(&period["number"]));
    ensure!(
        period["planner_facts"]["is_pillar_boundary"] == Value::Bool(false),
        "mixed period unexpectedly configured as a pillar boundary"
    );
    ensure!(report.period == period_number);
    ensure!(author == fixed(&period["planner_facts"]["dag_blocks"][0]["author"]));
    let header = application
        .final_chain()
        .block_header_raw(period_number.as_u64())?
        .context("published mixed-period header")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header))?;
    let public_header = chain
        .block_header(period_number)?
        .context("published materialized mixed-period header")?;
    ensure!(
        Rlp::new(&public_header).val_at::<H160>(2)?.0 == author,
        "published validator author"
    );
    ensure!(
        header.state_root.0 == fixed(&period["final"]["root"]),
        "published root"
    );
    ensure!(
        header.gas_used.as_u64()
            == period["transactions"]
                .as_array()
                .context("transactions")?
                .iter()
                .map(|transaction| number(&transaction["gas_used"]))
                .sum::<u64>(),
        "published gas"
    );
    ensure!(
        header.total_reward.as_u256() == decimal_u256(&period["reward_output"]["minted_reward"]),
        "published reward"
    );
    verify_receipts(&application, period)?;
    drop(adapter);
    drop(chain);
    drop(application);

    verify_physical_rows(concrete_path, &period["final"]["rows"])?;
    let (application, mut chain) = open_chain(application_path, fixture)?;
    let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, chain_identity)?;
    ensure!(observed.committed.period == period_number);
    ensure!(observed.committed.state_root == fixed(&period["final"]["root"]));
    ensure!(observed.pending_marker_rlp.is_empty());
    let genesis_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::GENESIS,
        state_root: fixed(&fixture["genesis"]["root"]),
    };
    let genesis_reader = ConcreteStateReader::open_historical_read_only(
        concrete_path,
        observed.committed,
        genesis_identity,
    )?;
    chain.hydrate_concrete_genesis_accounts(&genesis_reader)?;
    drop(genesis_reader);
    let concrete = ConcreteStateLifecycle::open(concrete_path, chain_identity, observed.committed)?;
    ensure!(chain.last_block_number_typed()? == period_number);
    let validator = fixed(&period["planner_facts"]["certificate_votes"][0]["validator"]);
    ensure!(
        chain.dpos_eligible_vote_count(period_number, validator)?
            == number(&period["planner_facts"]["validator_eligible_vote_count"]),
        "delayed validator vote count"
    );
    ensure!(
        chain.dpos_eligible_total_vote_count(period_number)?
            == number(&period["planner_facts"]["total_eligible_vote_count"]),
        "delayed total vote count"
    );
    verify_receipts(&application, period)?;
    drop(concrete);
    let reader = ConcreteStateReader::open_read_only(concrete_path, observed.committed)?;
    verify_final_reader(&reader, &period["final"])?;
    drop(reader);
    verify_historical_reads(concrete_path, observed.committed, fixture)?;
    drop(chain);
    drop(application);
    verify_physical_rows(concrete_path, &period["final"]["rows"])?;
    Ok(chain_identity)
}

#[test]
fn four_mixed_periods_commit_and_reopen_exactly() -> Result<()> {
    let fixture = load_fixture()?;
    ensure!(
        fixture["execution_mode"] == "observer",
        "observer fixture required"
    );
    ensure!(
        number(&fixture["configuration"]["chain_id"]) == 841,
        "fixture chain id"
    );
    let periods = fixture["periods"].as_array().context("fixture periods")?;
    ensure!(periods.len() == 4, "four-period fixture required");
    let expected_invocations = [0, 11, 4, 3];
    for (index, (period, count)) in periods.iter().zip(expected_invocations).enumerate() {
        ensure!(
            number(&period["number"]) == (index + 1) as u64,
            "consecutive mixed period"
        );
        ensure!(
            period["period_catalog_identities"]["invocations"]
                .as_array()
                .context("period invocation identities")?
                .len()
                == count,
            "frozen native invocation count"
        );
    }

    let root =
        std::env::temp_dir().join(format!("rustaxa-evm-mixed-periods-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    let application_path = root.join("application");
    let concrete_path = root.join("state_db");
    let mut chain_identity = None;
    for (index, period) in periods.iter().enumerate() {
        chain_identity = Some(run_period(
            &fixture,
            period,
            &application_path,
            &concrete_path,
            index == 0,
            chain_identity,
        )?);
    }
    std::fs::remove_dir_all(root)?;
    Ok(())
}

#[path = "support/mixed_recovery.rs"]
mod mixed_recovery;
