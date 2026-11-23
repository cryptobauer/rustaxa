use crate::native::ProbeNativeExecutionPort;
use crate::production_transition::{Hashes, Natives, historical_profile};
use anyhow::{Context, Result, ensure};
use ethereum_types::{H160, H256};
use rlp::Rlp;
use rustaxa_consensus::consensus_application_runtime::{OwnedDagGasEstimate, OwnedDagGasReport};
use rustaxa_consensus::{DagGasEstimateRequest, DagGasState};
use rustaxa_evm::{
    contracts::{
        CodeExecutionError, CodeExecutionStatus, ConsensusFailure, ExecutionBlockContext,
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
        TransactionExecutionResult,
    },
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    simulation::{CorrectedSimulation, SimulationBinding, simulate_with_native},
};
use rustaxa_storage::ConcreteCheckpointReaders;
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};
use rustaxa_types::{
    FinalChainCallLog, FinalChainCallOutcome, FinalChainCallRequest, FinalChainTransactionPosition,
};

pub(crate) fn estimate(
    state: &DagGasState<'_>,
    request: &DagGasEstimateRequest,
) -> Result<OwnedDagGasReport> {
    let estimates = request
        .transactions
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let position = FinalChainTransactionPosition::from(u32::try_from(index)?);
            let transaction =
                decode_legacy_input(position, &item.transaction_rlp, LegacyInputKind::Signed)?;
            ensure!(
                transaction.hash == item.hash
                    && transaction.canonical_rlp.as_deref()
                        == Some(item.transaction_rlp.as_slice()),
                "PRODUCTION_DAG_GAS_TRANSACTION_MISMATCH"
            );
            let gas_used = if state.is_direct_dpos(transaction.receiver) {
                estimate_dpos(state, transaction)?
            } else {
                crate::period_probe::gas_used(&simulate_evm(state, &transaction)?)
            };
            Ok(OwnedDagGasEstimate {
                hash: item.hash,
                gas_used,
            })
        })
        .collect::<Result<_>>()?;
    Ok(OwnedDagGasReport {
        effect_id: request.effect_id,
        estimates,
    })
}

fn estimate_dpos(state: &DagGasState<'_>, transaction: ExecutionTransaction) -> Result<u64> {
    let result = state.dpos_call(FinalChainCallRequest {
        block_number: state.proposal_period(),
        sender: transaction.sender,
        receiver: transaction.receiver,
        value: transaction
            .value
            .value()
            .to_bytes_be()
            .as_slice()
            .try_into()?,
        gas_price: transaction
            .gas_price
            .value()
            .to_bytes_be()
            .as_slice()
            .try_into()?,
        gas_limit: transaction.gas_limit,
        input: transaction.input,
    })?;
    Ok(result.gas_used.as_u64())
}

fn simulate_evm(
    state: &DagGasState<'_>,
    transaction: &ExecutionTransaction,
) -> Result<TransactionExecutionResult> {
    let header = state.block_header()?;
    let header = Rlp::new(&header);
    let period = state.period();
    let block = ExecutionBlockContext {
        period,
        author: header.val_at::<H160>(2)?.0,
        timestamp: header.val_at(10)?,
        gas_limit: header.val_at::<u64>(8)?.into(),
        chain_id: state.chain_id(),
        difficulty: Default::default(),
    };
    let state_root = header.val_at::<H256>(3)?.0;
    ensure!(
        header.val_at::<u64>(7)? == period.as_u64(),
        "PRODUCTION_DAG_GAS_HEADER_PERIOD_MISMATCH"
    );
    let profile = historical_profile(state.instruction_phase())
        .with_correction_stage(state.correction_stage());
    let native = state.begin_native_simulation()?;
    state.with_evm_checkpoint(|readers, identity| {
        ensure!(
            identity.state_root == state_root,
            "PRODUCTION_DAG_GAS_HEADER_ROOT_MISMATCH"
        );
        simulate_at_checkpoint(
            readers,
            SimulationBinding {
                state: identity,
                block,
                profile,
                envelope_rules: EnvelopeRules {
                    cornus: state.cornus(),
                },
            },
            native,
            |number| state.block_hash(number),
            |period, address| state.is_native(period, address),
            transaction,
        )
    })
}

pub(crate) fn simulate_at_checkpoint(
    readers: &ConcreteCheckpointReaders,
    binding: SimulationBinding,
    native: rustaxa_consensus::native_session::FinalChainNativeSimulation<'_>,
    block_hash: impl Fn(rustaxa_types::FinalChainBlockNumber) -> Result<Option<Vec<u8>>>,
    classifier: impl Fn(rustaxa_types::FinalChainBlockNumber, [u8; 20]) -> bool + Copy,
    transaction: &ExecutionTransaction,
) -> Result<TransactionExecutionResult> {
    let checkpoint = Checkpoint {
        readers,
        identity: binding.state,
    };
    let all_native = Natives(classifier, binding.profile, true);
    let consensus_native = Natives(classifier, binding.profile, false);
    let block_hashes = Hashes(block_hash);
    let outcome = if binding.profile.correction_stage()
        == rustaxa_evm::profile::CorrectionStage::Historical
    {
        simulate_with_native(
            &checkpoint,
            &block_hashes,
            &all_native,
            &consensus_native,
            |_| Ok(ProbeNativeExecutionPort::historical(native)),
            &binding.block,
            transaction,
            binding.envelope_rules,
            binding.profile,
        )
    } else {
        CorrectedSimulation::new(&checkpoint, binding.clone()).and_then(|simulation| {
            simulation.with_native(
                &block_hashes,
                &all_native,
                &consensus_native,
                |_| {
                    Ok(ProbeNativeExecutionPort::historical(native)
                        .bind_finalized_simulation(binding))
                },
                transaction,
            )
        })
    };
    outcome
        .map(|result| result.execution)
        .map_err(|error| anyhow::anyhow!("PRODUCTION_FINALIZED_EVM_SIMULATION: {error:?}"))
}

pub(crate) fn trace(state: &DagGasState<'_>, request_rlp: &[u8]) -> Result<Vec<u8>> {
    let request = Rlp::new(request_rlp);
    use rustaxa_evm::trace_runner::HistoricalTraceFormat;
    let format = match request.item_count()? {
        2 => HistoricalTraceFormat::Debug,
        3 => {
            let options = request.at(2)?;
            ensure!(options.item_count()? == 3, "PRODUCTION_TRACE_OPTIONS_SHAPE");
            HistoricalTraceFormat::OpenEthereum {
                vm_trace: options.val_at(0)?,
                trace: options.val_at(1)?,
            }
        }
        _ => anyhow::bail!("PRODUCTION_TRACE_REQUEST_SHAPE"),
    };
    let prefix = decode_trace_transactions(request.at(0)?, 0)?;
    let targets = decode_trace_transactions(request.at(1)?, prefix.len())?;
    let period = state.proposal_period();
    let header = match format {
        HistoricalTraceFormat::Debug => state.block_header()?,
        HistoricalTraceFormat::OpenEthereum { .. } => {
            ensure!(
                period == state.period(),
                "FinalChain::trace is not implemented above EVM head"
            );
            state.published_block_header()?.context("Future block")?
        }
    };
    let header = Rlp::new(&header);
    ensure!(
        header.val_at::<u64>(7)? == period.as_u64(),
        "PRODUCTION_TRACE_HEADER_PERIOD"
    );
    let profile = historical_profile(state.instruction_phase())
        .with_correction_stage(state.correction_stage());
    let block = ExecutionBlockContext {
        period,
        author: header.val_at::<H160>(2)?.0,
        timestamp: header.val_at(10)?,
        gas_limit: header.val_at::<u64>(8)?.into(),
        chain_id: state.chain_id(),
        difficulty: Default::default(),
    };
    state.with_trace_checkpoint(|readers, identity, native| {
        let checkpoint = Checkpoint { readers, identity };
        let classifier = |period, address| state.is_native(period, address);
        rustaxa_evm::trace_runner::run_trace_with_native(
            &checkpoint,
            &Hashes(|number| state.block_hash(number)),
            &Natives(classifier, profile, true),
            &Natives(classifier, profile, false),
            |parent, execution| {
                debug_assert_eq!((parent, execution), (identity, period));
                Ok(ProbeNativeExecutionPort::historical(native))
            },
            &block,
            &prefix,
            &targets,
            EnvelopeRules {
                cornus: state.cornus(),
            },
            profile,
            format,
        )
        .map(|result| result.output)
        .map_err(Into::into)
    })
}

fn decode_trace_transactions(
    sequence: Rlp<'_>,
    offset: usize,
) -> Result<Vec<ExecutionTransaction>> {
    ensure!(sequence.is_list(), "PRODUCTION_TRACE_SEQUENCE_SHAPE");
    (0..sequence.item_count()?)
        .map(|index| {
            let item = sequence.at(index)?;
            ensure!(
                item.item_count()? == 7,
                "PRODUCTION_TRACE_TRANSACTION_SHAPE"
            );
            let receiver = item.at(2)?.data()?;
            let receiver = (!receiver.is_empty())
                .then(|| receiver.try_into())
                .transpose()?;
            let price: ethereum_types::U256 = item.val_at(1)?;
            let value: ethereum_types::U256 = item.val_at(4)?;
            Ok(ExecutionTransaction {
                position: u32::try_from(offset + index)?.into(),
                hash: [0; 32],
                sender: item.val_at::<H160>(0)?.0,
                receiver,
                nonce: rustaxa_types::FinalChainNonce::from_bytes(item.at(3)?.data()?)?,
                gas_price: ExecutionGasPrice::new(num_bigint::BigUint::from_bytes_be(
                    &price.to_big_endian(),
                )),
                gas_limit: item.val_at::<u64>(5)?.into(),
                value: ExecutionValue::new(num_bigint::BigUint::from_bytes_be(
                    &value.to_big_endian(),
                )),
                input: item.val_at(6)?,
                canonical_rlp: None,
                kind: if receiver.is_some() {
                    ExecutionTransactionKind::Call
                } else {
                    ExecutionTransactionKind::Create
                },
            })
        })
        .collect()
}

pub(crate) fn call(
    state: &DagGasState<'_>,
    request: FinalChainCallRequest,
) -> Result<FinalChainCallOutcome> {
    if state.is_direct_dpos(request.receiver) {
        return state.dpos_call(request);
    }
    let transaction = ExecutionTransaction {
        position: 0_u32.into(),
        hash: [0; 32],
        sender: request.sender,
        receiver: request.receiver,
        nonce: Default::default(),
        gas_price: ExecutionGasPrice::new(num_bigint::BigUint::from_bytes_be(
            &request.gas_price.to_fixed_be_bytes(),
        )),
        gas_limit: request.gas_limit,
        value: ExecutionValue::new(num_bigint::BigUint::from_bytes_be(
            &request.value.to_fixed_be_bytes(),
        )),
        input: request.input,
        canonical_rlp: None,
        kind: if request.receiver.is_some() {
            ExecutionTransactionKind::Call
        } else {
            ExecutionTransactionKind::Create
        },
    };
    let mut result = FinalChainCallOutcome::default();
    match simulate_evm(state, &transaction)? {
        TransactionExecutionResult::Executed(execution) => {
            result.gas_used = execution.gas_used;
            result.code_retval = execution.output;
            result.logs = execution
                .logs
                .into_iter()
                .map(|log| FinalChainCallLog {
                    address: log.address,
                    topics: log.topics,
                    data: log.data,
                })
                .collect();
            if let CodeExecutionStatus::Failure(error) = execution.status {
                result.code_err = call_code_error(error, &result.code_retval);
            }
        }
        TransactionExecutionResult::ConsensusFailure(failure) => {
            result.gas_used = failure.gas_used;
            result.code_retval = failure.output;
            result.consensus_err = match failure.error {
                ConsensusFailure::InsufficientBalanceForGas => {
                    "insufficient balance to pay for gas"
                }
                ConsensusFailure::NonceTooLow => "nonce too low",
                ConsensusFailure::IntrinsicGasOverflow => "gas uint64 overflow",
                ConsensusFailure::IntrinsicGas => "intrinsic gas too low",
                ConsensusFailure::InsufficientBalanceForTransfer => {
                    "insufficient balance for transfer"
                }
            }
            .into();
        }
    }
    Ok(result)
}

/// Reads both pillar fields at the same committed checkpoint.
pub(crate) fn pillar_bridge_values(state: &DagGasState<'_>) -> Result<([u8; 32], [u8; 32])> {
    use tiny_keccak::{Hasher, Keccak};
    let bridge = state.bridge_address();
    let has_code = state.with_evm_checkpoint(|readers, identity| {
        Ok(matches!(readers.account_at(identity, bridge)?, ConcreteRead::Present(row) if row.account.code_size > 0))
    })?;
    if !has_code {
        return Ok(([0; 32], [0; 32]));
    }
    let read = |method: &[u8]| -> Result<[u8; 32]> {
        let mut keccak = Keccak::v256();
        keccak.update(method);
        let mut digest = [0; 32];
        keccak.finalize(&mut digest);
        let result = call(
            state,
            FinalChainCallRequest {
                block_number: state.period(),
                sender: [0; 20],
                receiver: Some(bridge),
                gas_price: rustaxa_types::FinalChainGasPrice::from_u256(1.into()),
                value: rustaxa_types::FinalChainTransactionValue::zero(),
                gas_limit: 10_000_000.into(),
                input: digest[..4].to_vec(),
            },
        )?;
        if result.code_retval.is_empty() {
            Ok([0; 32])
        } else {
            result
                .code_retval
                .try_into()
                .map_err(|_| anyhow::anyhow!("PRODUCTION_BRIDGE_ANCHOR_RESULT_WIDTH"))
        }
    };
    Ok((read(b"getBridgeRoot()")?, read(b"finalizedEpoch()")?))
}

fn call_code_error(error: CodeExecutionError, data: &[u8]) -> String {
    let diagnostic = match error {
        CodeExecutionError::Revert => {
            return String::from_utf8_lossy(&rustaxa_evm::revert::dry_run_revert_diagnostic(data))
                .into_owned();
        }
        CodeExecutionError::InvalidOpcode(opcode) => {
            return format!("invalid opcode: 0x{opcode:x}");
        }
        CodeExecutionError::Native(error) => return error.error,
        CodeExecutionError::OutOfGas => "out of gas",
        CodeExecutionError::StackUnderflow => "stack underflow",
        CodeExecutionError::StackOverflow => "stack limit reached",
        CodeExecutionError::StaticViolation => "write protection",
        CodeExecutionError::Depth => "max call depth exceeded",
        CodeExecutionError::CreateCollision => "contract address collision",
        CodeExecutionError::ContractSize => "max code size exceeded",
        CodeExecutionError::CodeDepositOutOfGas => "contract creation code storage out of gas",
        CodeExecutionError::InvalidJump => "invalid jump destination",
        CodeExecutionError::ReturnDataOutOfBounds => "return data out of bounds",
        CodeExecutionError::GasUintOverflow => "gas uint64 overflow",
    };
    diagnostic.into()
}

struct Checkpoint<'a> {
    readers: &'a ConcreteCheckpointReaders,
    identity: ConcreteStateIdentity,
}

impl ConcreteStateRead for Checkpoint<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.readers.account_at(self.identity, address)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.readers.storage_at(self.identity, address, key)
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.readers.code_at(self.identity, hash)
    }
}
