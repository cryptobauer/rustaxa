//! Shared ordered transaction computation. This module grants no publication authority.
use crate::period_probe::{PbftExecutionObserver, gas_used};
use anyhow::Result;
use num_bigint::BigUint;
use rlp::RlpStream;
use rustaxa_evm::{
    contracts::*,
    driver::{self, NativeAddressClassifier, PeriodConsensusSequence},
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
    native::NativeAdapterError,
    profile::{CorrectionStage, TaraxaProfile},
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteObserverPhaseDelta,
    ConcreteStorageMutation,
};
use rustaxa_types::{
    FinalChainNonce,
    concrete_state::execution::ConcreteExecutionRead,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteStateIdentity,
    },
};
use tiny_keccak::{Hasher, Keccak};

pub(super) trait PeriodState {
    type View<'a>: ConcreteExecutionRead
    where
        Self: 'a;
    fn view(&self) -> Result<Self::View<'_>>;
    fn apply_phase(&mut self, delta: ConcreteObserverPhaseDelta) -> Result<()>;
    fn identity(&self) -> Result<ConcreteStateIdentity>;
    fn records_lifecycle(&self) -> bool {
        false
    }
    fn transaction_completed(&mut self, _output: &PeriodTransactionOutput) -> Result<()> {
        Ok(())
    }
}
impl PeriodState for rustaxa_storage::ConcreteTransitionBuilder<'_> {
    type View<'a>
        = rustaxa_storage::ConcreteExecutionProbeView<'a>
    where
        Self: 'a;
    fn view(&self) -> Result<Self::View<'_>> {
        Ok(self.current_view()?)
    }
    fn apply_phase(&mut self, delta: ConcreteObserverPhaseDelta) -> Result<()> {
        rustaxa_storage::ConcreteTransitionBuilder::apply_phase(self, delta)?;
        Ok(())
    }
    fn identity(&self) -> Result<ConcreteStateIdentity> {
        Ok(self.current_view()?.identity())
    }
}

pub(super) struct NativeCodeUpdate {
    pub address: [u8; 20],
    pub code: Vec<u8>,
}
pub(super) struct PeriodTransactionOutput {
    pub transaction: ExecutionTransaction,
    pub result: TransactionExecutionResult,
    pub native: Vec<ConsensusNativeObservation>,
    pub raw_writes: usize,
    pub state: ConcreteStateIdentity,
    pub raw_slots: Vec<rustaxa_types::concrete_lifecycle::ConcreteStorageSlot>,
    pub lifecycle_receipt: Option<rustaxa_evm::journal::JournalLifecycleReceipt>,
}
pub(super) enum PeriodKernelOutcome {
    Completed(Vec<PeriodTransactionOutput>),
    MalformedProof {
        transaction_position: u64,
        native_sequence: Option<u64>,
        message: String,
    },
}
pub(super) fn prepare_native_code(
    concrete: &mut impl PeriodState,
    updates: impl IntoIterator<Item = NativeCodeUpdate>,
) -> Result<()> {
    for update in updates {
        let prior = concrete.view()?.account(update.address)?;
        let (nonce, balance) = match prior {
            ConcreteRead::Present(row) => (row.account.nonce, row.account.balance),
            _ => (
                FinalChainNonce::zero(),
                ConcreteAccountBalance::new(BigUint::default()),
            ),
        };
        let code_hash = hash(&update.code);
        concrete.apply_phase(ConcreteObserverPhaseDelta {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address: update.address,
                record: record(nonce, balance, Some(code_hash), update.code.len() as u64),
            }],
            storage: vec![],
            code: vec![ConcreteCodeInsertion {
                code_hash,
                code: update.code,
            }],
        })?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_transaction_stream<'a>(
    concrete: &mut impl PeriodState,
    native: &mut impl NativeExecutionPort,
    block: &ExecutionBlockContext,
    profile: TaraxaProfile,
    cornus: bool,
    credit_fees: bool,
    transactions: impl IntoIterator<Item = (&'a [u8], LegacyInputKind)>,
    hashes: &impl BlockHashRead,
    all_natives: &impl NativeAddressClassifier,
    consensus_natives: &impl NativeAddressClassifier,
    observer: &mut impl PbftExecutionObserver,
) -> Result<PeriodKernelOutcome> {
    let mut sequence = PeriodConsensusSequence::new(block.period);
    let mut completed = Vec::new();
    for (index, (bytes, kind)) in transactions.into_iter().enumerate() {
        let position = u32::try_from(index)
            .map_err(|_| anyhow::anyhow!("PBFT_PROBE_POSITION_OVERFLOW"))?
            .into();
        let is_system = matches!(kind, LegacyInputKind::System);
        let transaction = decode_legacy_input(position, bytes, kind)?;
        observer.transaction_started(&transaction);
        let mut journal = ExecutionJournal::new_with_correction_stage(
            concrete.view()?,
            profile.correction_stage(),
        );
        let lifecycle_token = concrete
            .records_lifecycle()
            .then(|| journal.begin_lifecycle_receipt())
            .transpose()?;
        let result = if is_system {
            driver::execute_top_level_system_call_with_native(
                &mut journal,
                hashes,
                all_natives,
                consensus_natives,
                native,
                &mut sequence,
                block,
                &transaction,
                EnvelopeRules { cornus },
                profile,
                bytes,
            )
        } else if transaction.kind == ExecutionTransactionKind::Create {
            driver::execute_top_level_create_with_native(
                &mut journal,
                hashes,
                all_natives,
                consensus_natives,
                native,
                &mut sequence,
                block,
                &transaction,
                EnvelopeRules { cornus },
                profile,
            )
        } else {
            driver::execute_top_level_call_with_native(
                &mut journal,
                hashes,
                all_natives,
                consensus_natives,
                native,
                &mut sequence,
                block,
                &transaction,
                EnvelopeRules { cornus },
                profile,
            )
        };
        let result = match result {
            Ok(result) => result,
            Err(driver::ExecutionDriverError::Native(NativeAdapterError::Port(
                NativePortError::MalformedNestedSlashingProof(message),
            ))) => {
                return Ok(PeriodKernelOutcome::MalformedProof {
                    transaction_position: index as u64,
                    native_sequence: sequence.next_sequence().checked_sub(1),
                    message,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let settled = journal.settle_transaction()?;
        drop(journal);
        if let Some(token) = lifecycle_token {
            let receipt = settled
                .lifecycle_receipt
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("PRODUCTION_LIFECYCLE_RECEIPT_MISSING"))?;
            rustaxa_evm::journal::validate_lifecycle_receipt_token(token, receipt)?;
        }
        let raw_slots = if lifecycle_token.is_some() {
            settled
                .writes
                .raw_storage
                .iter()
                .map(
                    |write| rustaxa_types::concrete_lifecycle::ConcreteStorageSlot {
                        address: write.address,
                        key: write.key.0,
                    },
                )
                .collect()
        } else {
            Vec::new()
        };
        let raw_writes = settled.writes.raw_storage.len();
        concrete.apply_phase(settlement_delta(settled.writes))?;
        if credit_fees {
            apply_transaction_fee(
                concrete,
                block.author,
                transaction.gas_price.value(),
                gas_used(&result),
                profile.correction_stage(),
            )?;
        }
        let output = PeriodTransactionOutput {
            transaction,
            result,
            native: settled.native_invocations,
            raw_writes,
            state: concrete.identity()?,
            raw_slots,
            lifecycle_receipt: settled.lifecycle_receipt,
        };
        concrete.transaction_completed(&output)?;
        observer.transaction_completed(
            &output.transaction,
            &output.result,
            &output.native,
            output.raw_writes,
            output.state,
        );
        completed.push(output);
    }
    Ok(PeriodKernelOutcome::Completed(completed))
}

pub(super) fn apply_transaction_fee(
    concrete: &mut impl PeriodState,
    author: [u8; 20],
    price: &BigUint,
    gas: u64,
    stage: CorrectionStage,
) -> Result<()> {
    let modulus = BigUint::from(1_u8) << 256_usize;
    let fee = ((price % &modulus) * BigUint::from(gas)) % modulus;
    let mut journal = ExecutionJournal::new_with_correction_stage(concrete.view()?, stage);
    journal.add_balance(author, &fee)?;
    let settled = journal.settle_transaction()?;
    drop(journal);
    concrete.apply_phase(settlement_delta(settled.writes))?;
    Ok(())
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut result = [0; 32];
    let mut h = Keccak::v256();
    h.update(bytes);
    h.finalize(&mut result);
    result
}
fn record(
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
) -> ConcreteAccountRecord {
    let mut rlp = RlpStream::new_list(5);
    let balance_bytes = if balance.value() == &BigUint::default() {
        vec![]
    } else {
        balance.value().to_bytes_be()
    };
    rlp.append(&nonce.to_bytes())
        .append(&balance_bytes)
        .append_empty_data();
    match code_hash {
        Some(hash) => {
            rlp.append(&hash.as_slice());
        }
        None => {
            rlp.append_empty_data();
        }
    }
    rlp.append(&code_size);
    ConcreteAccountRecord {
        account: ConcreteAccount {
            nonce,
            balance,
            storage_root: None,
            code_hash,
            code_size,
        },
        physical_rlp: rlp.out().to_vec(),
    }
}
pub(super) fn settlement_delta(writes: JournalWritePlan) -> ConcreteObserverPhaseDelta {
    ConcreteObserverPhaseDelta {
        accounts: writes
            .accounts
            .into_iter()
            .map(|write| match write.operation {
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
            })
            .collect(),
        storage: writes
            .ordinary_storage
            .into_iter()
            .map(|write| ConcreteStorageMutation {
                address: write.address,
                key: write.key,
                value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
            })
            .chain(
                writes
                    .raw_storage
                    .into_iter()
                    .map(|write| ConcreteStorageMutation {
                        address: write.address,
                        key: write.key,
                        value: match write.operation {
                            NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
                            NativeRawOperation::Delete => None,
                        },
                    }),
            )
            .collect(),
        code: writes
            .code
            .into_iter()
            .map(|write| ConcreteCodeInsertion {
                code_hash: write.code_hash,
                code: write.code,
            })
            .collect(),
    }
}
