//! Bounded corrected simulation contracts with independently specified bytecode.
//! Native tests use real disposable FinalChain sessions with a synthetic seed.
//! They do not prove production root selection or RPC routing.

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::FinalChain;
use rustaxa_evm::{
    contracts::*,
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    estimate::{EstimateError, EstimateProbe},
    profile::{CorrectionStage, TaraxaProfile},
    simulation::{
        CorrectedSimulation, CorrectedSimulationNative, SimulationBinding, SimulationError,
        simulate_ordinary, simulate_with_native,
    },
};
use rustaxa_storage::{Config, Storage};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainRewardsConfig, GenesisDposConfig, concrete_state::*,
};
use std::{
    cell::Cell,
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[path = "../../rustaxa-storage/src/closeout_test_scratch.rs"]
mod closeout_test_scratch;
#[allow(dead_code)]
#[path = "support/mixed_native.rs"]
mod mixed_native;

const SENDER: [u8; 20] = [0xaa; 20];
const PARENT: [u8; 20] = [0xbb; 20];
const CHILD: [u8; 20] = [0xcc; 20];
const SLASH: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
];
const STAGE: CorrectionStage = CorrectionStage::SlashingHardError;

struct Reader {
    identity: Cell<ConcreteStateIdentity>,
    reads: Cell<usize>,
    identity_reads: Cell<usize>,
    accounts: BTreeMap<[u8; 20], ConcreteAccountRecord>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
}
impl Reader {
    fn new() -> Self {
        let mut result = Self {
            identity: Cell::new(ConcreteStateIdentity {
                period: 0.into(),
                state_root: [0x44; 32],
            }),
            reads: Cell::new(0),
            identity_reads: Cell::new(0),
            accounts: BTreeMap::new(),
            codes: BTreeMap::new(),
        };
        result.add_account(SENDER, 10_000_000, None);
        result.add_account(SLASH, 0, None);
        result
    }
    fn add_account(&mut self, address: [u8; 20], balance: u64, code: Option<Vec<u8>>) {
        let (code_hash, code_size) = if let Some(code) = code {
            let hash = keccak256(&code).0;
            let size = code.len() as u64;
            self.codes.insert(hash, code);
            (Some(hash), size)
        } else {
            (None, 0)
        };
        let account = ConcreteAccount {
            nonce: 1.into(),
            balance: ConcreteAccountBalance::new(balance.into()),
            storage_root: None,
            code_hash,
            code_size,
        };
        let mut raw = RlpStream::new_list(5);
        raw.append(&account.nonce.to_bytes());
        if balance == 0 {
            raw.append_empty_data();
        } else {
            raw.append(&BigUint::from(balance).to_bytes_be());
        }
        raw.append_empty_data();
        if let Some(hash) = code_hash {
            raw.append(&hash.as_slice());
        } else {
            raw.append_empty_data();
        }
        raw.append(&code_size);
        self.accounts.insert(
            address,
            ConcreteAccountRecord {
                account,
                physical_rlp: raw.out().to_vec(),
            },
        );
    }
}
impl ConcreteStateRead for Reader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity_reads.set(self.identity_reads.get() + 1);
        self.identity.get()
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.reads.set(self.reads.get() + 1);
        Ok(self
            .accounts
            .get(&address)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}
struct External;
impl BlockHashRead for External {
    fn block_hash(&self, _: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        panic!("unexpected history read")
    }
}
impl NativeAddressClassifier for External {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address == SLASH
    }
}
fn binding(reader: &Reader, stage: CorrectionStage) -> SimulationBinding {
    SimulationBinding {
        state: reader.identity(),
        block: ExecutionBlockContext {
            period: 0.into(),
            author: [0; 20],
            timestamp: 1,
            gas_limit: 1_000_000.into(),
            chain_id: 666,
            difficulty: BigUint::default(),
        },
        profile: TaraxaProfile::new(false).with_correction_stage(stage),
        envelope_rules: EnvelopeRules { cornus: true },
    }
}
fn tx(create: bool, input: Vec<u8>) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [1; 32],
        sender: SENDER,
        receiver: if create { None } else { Some(PARENT) },
        nonce: 999.into(),
        gas_price: ExecutionGasPrice::new(1_u8.into()),
        gas_limit: 200_000.into(),
        value: ExecutionValue::default(),
        input,
        canonical_rlp: None,
        kind: if create {
            ExecutionTransactionKind::Create
        } else {
            ExecutionTransactionKind::Call
        },
    }
}
fn executed(result: TransactionExecutionResult) -> ExecutedTransactionResult {
    match result {
        TransactionExecutionResult::Executed(value) => value,
        other => panic!("expected executed transaction: {other:?}"),
    }
}

struct Port<P> {
    inner: P,
    binding: SimulationBinding,
    stage: Option<CorrectionStage>,
    sequences: Arc<Mutex<Vec<u64>>>,
}
impl<P: NativeExecutionPort> NativeExecutionPort for Port<P> {
    fn correction_stage(&self) -> Option<CorrectionStage> {
        self.stage
    }
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.sequences.lock().unwrap().push(invocation.id.sequence);
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.inner.invoke(invocation, quote, journal)
    }
    fn input_aliases(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<NativeInputAlias>, NativePortError> {
        self.inner.input_aliases(id)
    }
    fn input_mutations(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<NativeInputMutation>, NativePortError> {
        self.inner.input_mutations(id)
    }
}
impl<P: NativeExecutionPort> CorrectedSimulationNative for Port<P> {
    fn simulation_binding(&self) -> &SimulationBinding {
        &self.binding
    }
}
struct Unreachable;
impl NativeExecutionPort for Unreachable {
    fn prepare(
        &mut self,
        _: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        panic!("unexpected prepare")
    }
    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        panic!("unexpected invoke")
    }
}

#[test]
fn historical_wrappers_and_corrected_constructor_reject_before_reads_or_factory() {
    let reader = Reader::new();
    let selected = binding(&reader, STAGE);
    let transaction = tx(false, vec![]);
    let historical = binding(&reader, CorrectionStage::Historical);
    let identity_reads = reader.identity_reads.get();
    assert!(matches!(
        simulate_ordinary(
            &reader,
            &External,
            &External,
            &selected.block,
            &transaction,
            selected.envelope_rules,
            selected.profile
        ),
        Err(SimulationError::UnsupportedCorrectionStage(STAGE))
    ));
    assert!(matches!(
        simulate_with_native(
            &reader,
            &External,
            &External,
            &External,
            |_| -> Result<Unreachable, NativePortError> { panic!("unexpected factory") },
            &selected.block,
            &transaction,
            selected.envelope_rules,
            selected.profile
        ),
        Err(SimulationError::UnsupportedCorrectionStage(STAGE))
    ));
    assert!(matches!(
        CorrectedSimulation::new(&reader, historical),
        Err(SimulationError::HistoricalCorrectionStage)
    ));
    assert_eq!(reader.reads.get(), 0);
    assert_eq!(reader.identity_reads.get(), identity_reads);
}

#[test]
fn corrected_selection_rejects_root_period_and_changed_reader() {
    let reader = Reader::new();
    let mut selected = binding(&reader, STAGE);
    selected.state.state_root = [0x55; 32];
    assert!(matches!(
        CorrectedSimulation::new(&reader, selected),
        Err(SimulationError::StateIdentityMismatch { .. })
    ));
    let mut selected = binding(&reader, STAGE);
    selected.block.period = 1.into();
    assert!(matches!(
        CorrectedSimulation::new(&reader, selected),
        Err(SimulationError::StatePeriodMismatch { .. })
    ));
    let sim = CorrectedSimulation::new(&reader, binding(&reader, STAGE)).unwrap();
    reader.identity.set(ConcreteStateIdentity {
        period: 0.into(),
        state_root: [0x66; 32],
    });
    assert!(matches!(
        sim.ordinary(&External, &External, &tx(false, vec![])),
        Err(SimulationError::StateIdentityMismatch { .. })
    ));
    assert_eq!(reader.reads.get(), 0);
}

#[test]
fn native_binding_and_all_correction_stage_mismatches_precede_execution() {
    let reader = Reader::new();
    for stage in [CorrectionStage::TransientRollback, STAGE] {
        let sim = CorrectedSimulation::new(&reader, binding(&reader, stage)).unwrap();
        for actual in [None, Some(CorrectionStage::Historical)] {
            let error = sim
                .with_native(
                    &External,
                    &External,
                    &External,
                    |origin| {
                        Ok(Port {
                            inner: Unreachable,
                            binding: origin.clone(),
                            stage: actual,
                            sequences: Arc::default(),
                        })
                    },
                    &tx(false, vec![]),
                )
                .unwrap_err();
            assert_eq!(
                error,
                SimulationError::NativeCorrectionStageMismatch {
                    expected: stage,
                    actual
                }
            );
        }
        for field in 0..5 {
            let error = sim
                .with_native(
                    &External,
                    &External,
                    &External,
                    |origin| {
                        let mut changed = origin.clone();
                        match field {
                            0 => changed.state.state_root = [0x55; 32],
                            1 => changed.block.chain_id += 1,
                            2 => {
                                changed.profile =
                                    TaraxaProfile::new(true).with_correction_stage(stage)
                            }
                            3 => changed.envelope_rules.cornus = false,
                            _ => changed.state.period = 1.into(),
                        }
                        Ok(Port {
                            inner: Unreachable,
                            binding: changed,
                            stage: Some(stage),
                            sequences: Arc::default(),
                        })
                    },
                    &tx(false, vec![]),
                )
                .unwrap_err();
            assert_eq!(error, SimulationError::NativeBindingMismatch);
        }
    }
}

fn rollback_parent() -> Vec<u8> {
    let mut code = vec![
        0x60, 7, 0x60, 1, 0xb4, 0x60, 0, 0x60, 0, 0x60, 0, 0x60, 0, 0x73,
    ];
    code.extend_from_slice(&CHILD);
    code.extend_from_slice(&[
        0x61, 0xff, 0xff, 0xf4, 0x50, 0x60, 1, 0xb3, 0x60, 0, 0x52, 0x60, 32, 0x60, 0, 0xf3,
    ]);
    code
}
#[test]
fn corrected_call_and_create_catch_child_revert_and_restore_transient_value() {
    let mut reader = Reader::new();
    reader.add_account(
        CHILD,
        0,
        Some(vec![0x60, 9, 0x60, 1, 0xb4, 0x60, 0, 0x60, 0, 0xfd]),
    );
    reader.add_account(PARENT, 0, Some(rollback_parent()));
    let before = reader.accounts.clone();
    let sim = CorrectedSimulation::new(&reader, binding(&reader, STAGE)).unwrap();
    for create in [false, true] {
        let request = tx(create, if create { rollback_parent() } else { vec![] });
        let first = sim.ordinary(&External, &External, &request).unwrap();
        let second = sim.ordinary(&External, &External, &request).unwrap();
        assert_eq!(first, second);
        let result = executed(first.execution);
        assert_eq!(result.status, CodeExecutionStatus::Success);
        let mut expected = vec![0; 32];
        expected[31] = 7;
        assert_eq!(result.output, expected);
        assert_eq!(result.attempted_contract_address.is_some(), create);
        let mut different_nonce = request.clone();
        different_nonce.nonce = 0.into();
        assert_eq!(
            sim.ordinary(&External, &External, &different_nonce)
                .unwrap()
                .execution,
            sim.ordinary(&External, &External, &request)
                .unwrap()
                .execution
        );
    }
    assert_eq!(reader.accounts, before);
}

#[test]
fn corrected_envelope_and_revert_keep_gas_value_output_and_input() {
    let mut reader = Reader::new();
    reader.add_account(
        PARENT,
        0,
        Some(vec![0x60, 0xab, 0x60, 0, 0x53, 0x60, 1, 0x60, 0, 0xfd]),
    );
    let sim = CorrectedSimulation::new(&reader, binding(&reader, STAGE)).unwrap();
    let request = tx(false, vec![]);
    let result = executed(
        sim.ordinary(&External, &External, &request)
            .unwrap()
            .execution,
    );
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::Revert)
    );
    assert_eq!(result.output, vec![0xab]);
    assert!(result.gas_used.as_u64() >= 21_000 && result.gas_used < request.gas_limit);
    assert_eq!(request.nonce, rustaxa_types::FinalChainNonce::from_u64(999));
    let mut low = request.clone();
    low.gas_limit = 20_000.into();
    assert!(matches!(
        sim.ordinary(&External, &External, &low).unwrap().execution,
        TransactionExecutionResult::ConsensusFailure(ConsensusFailureResult {
            error: ConsensusFailure::IntrinsicGas,
            ..
        })
    ));
    let mut expensive = request.clone();
    expensive.value = ExecutionValue::new(20_000_000_u64.into());
    assert!(matches!(
        sim.ordinary(&External, &External, &expensive)
            .unwrap()
            .execution,
        TransactionExecutionResult::ConsensusFailure(ConsensusFailureResult {
            error: ConsensusFailure::InsufficientBalanceForTransfer,
            ..
        })
    ));
    let code_error = sim
        .estimate_with_native(
            &External,
            &External,
            &External,
            |origin| {
                Ok(Port {
                    inner: Unreachable,
                    binding: origin.clone(),
                    stage: Some(STAGE),
                    sequences: Arc::default(),
                })
            },
            &request,
            |result| {
                assert!(matches!(
                    result,
                    TransactionExecutionResult::Executed(ExecutedTransactionResult {
                        status: CodeExecutionStatus::Failure(CodeExecutionError::Revert),
                        ..
                    })
                ));
                EstimateProbe::CodeFailure {
                    error: "caller code sentinel".into(),
                }
            },
        )
        .unwrap_err();
    assert_eq!(
        code_error,
        EstimateError::CodeFailure {
            error: "caller code sentinel".into()
        }
    );
    let consensus_error = sim
        .estimate_with_native(
            &External,
            &External,
            &External,
            |origin| {
                Ok(Port {
                    inner: Unreachable,
                    binding: origin.clone(),
                    stage: Some(STAGE),
                    sequences: Arc::default(),
                })
            },
            &low,
            |result| {
                assert!(matches!(
                    result,
                    TransactionExecutionResult::ConsensusFailure(ConsensusFailureResult {
                        error: ConsensusFailure::IntrinsicGas,
                        ..
                    })
                ));
                EstimateProbe::ConsensusFailure {
                    error: "caller consensus sentinel".into(),
                }
            },
        )
        .unwrap_err();
    assert_eq!(
        consensus_error,
        EstimateError::ConsensusFailure {
            error: "caller consensus sentinel".into()
        }
    );
}

fn with_chain(test: impl FnOnce(&FinalChain)) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!(
            "corrected-simulation-{}-{stamp}",
            std::process::id()
        )),
        true,
    );
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    closeout_test_scratch::created(&path);
    let chain = FinalChain::new_with_rewards_config(
        storage.clone(),
        1_000_000.into(),
        0,
        vec![],
        vec![],
        GenesisDposConfig::default(),
        FinalChainRewardsConfig {
            cornus_period: 0.into(),
            ..Default::default()
        },
    )
    .unwrap()
    .with_correction_stage(STAGE);
    test(&chain);
    drop(chain);
    drop(storage);
    closeout_test_scratch::dispose(&path).unwrap();
}
fn malformed() -> Vec<u8> {
    let mut result = vec![0xfa, 0xc7, 0xc9, 0x4a];
    for value in [64_u64, 128, 1] {
        let mut word = [0; 32];
        word[24..].copy_from_slice(&value.to_be_bytes());
        result.extend_from_slice(&word);
    }
    result.push(0xc0);
    result.resize(4 + 128, 0);
    let mut word = [0; 32];
    word[31] = 1;
    result.extend_from_slice(&word);
    result.push(0xc0);
    result.resize(4 + 192, 0);
    result
}
fn native_parent(gas: u16, input: &[u8], create: bool) -> Vec<u8> {
    // Store proof bytes in memory so the same fixture also runs as CREATE initcode.
    let mut code = Vec::new();
    for (index, chunk) in input.chunks(32).enumerate() {
        let mut word = [0; 32];
        word[..chunk.len()].copy_from_slice(chunk);
        code.push(0x7f);
        code.extend_from_slice(&word);
        code.push(0x61);
        code.extend_from_slice(&((index * 32) as u16).to_be_bytes());
        code.push(0x52);
    }
    code.extend_from_slice(&[0x60, 0, 0x60, 0, 0x61]);
    code.extend_from_slice(&(input.len() as u16).to_be_bytes());
    code.extend_from_slice(&[0x60, 0, 0x60, 0, 0x73]);
    code.extend_from_slice(&SLASH);
    code.push(0x61);
    code.extend_from_slice(&gas.to_be_bytes());
    code.extend_from_slice(&[0xf1, 0x50, 0x60, 9, 0x60, 1, 0x55]);
    if create {
        code.extend_from_slice(&[0x60, 0, 0x60, 0, 0xf3]);
    } else {
        code.push(0);
    }
    code
}
fn native_factory<'a>(
    chain: &'a FinalChain,
    selected: &SimulationBinding,
    sequences: Arc<Mutex<Vec<u64>>>,
) -> Result<Port<mixed_native::SimulationNativeExecutionPort<'a>>, NativePortError> {
    // This synthetic owner fixes configuration and state selection. Production
    // adapters must authenticate their concrete root against the native owner.
    assert_eq!(selected.state.period, FinalChainBlockNumber::GENESIS);
    assert_eq!(selected.state.state_root, [0x44; 32]);
    assert_eq!(selected.block.chain_id, 666);
    assert_eq!(selected.profile, binding(&Reader::new(), STAGE).profile);
    let inner = mixed_native::SimulationNativeExecutionPort::new(
        chain
            .begin_native_simulation(selected.state.period)
            .map_err(|error| NativePortError::Domain(format!("{error:?}")))?,
    );
    let stage = inner.correction_stage();
    Ok(Port {
        inner,
        binding: selected.clone(),
        stage,
        sequences,
    })
}

#[test]
fn actual_nested_native_hard_error_stops_call_create_and_estimate_mapper() {
    with_chain(|chain| {
        for create in [false, true] {
            let mut reader = Reader::new();
            let code = native_parent(20_000, &malformed(), create);
            reader.add_account(PARENT, 0, Some(code.clone()));
            let before = reader.accounts.clone();
            let sim = CorrectedSimulation::new(&reader, binding(&reader, STAGE)).unwrap();
            let request = tx(create, if create { code } else { vec![] });
            let sequences = Arc::new(Mutex::new(Vec::new()));
            let error = sim
                .with_native(
                    &External,
                    &External,
                    &External,
                    |origin| native_factory(chain, origin, sequences.clone()),
                    &request,
                )
                .unwrap_err();
            assert!(matches!(
                error,
                SimulationError::Execution(rustaxa_evm::driver::ExecutionDriverError::Native(
                    rustaxa_evm::native::NativeAdapterError::Port(
                        NativePortError::MalformedNestedSlashingProof(_)
                    )
                ))
            ));
            assert_eq!(*sequences.lock().unwrap(), vec![0]);
            let factories = Cell::new(0);
            let error = sim
                .estimate_with_native(
                    &External,
                    &External,
                    &External,
                    |origin| {
                        factories.set(factories.get() + 1);
                        native_factory(chain, origin, sequences.clone())
                    },
                    &request,
                    |_| -> EstimateProbe { panic!("hard error entered mapper") },
                )
                .unwrap_err();
            assert!(matches!(
                error,
                EstimateError::Callback(SimulationError::Execution(
                    rustaxa_evm::driver::ExecutionDriverError::Native(
                        rustaxa_evm::native::NativeAdapterError::Port(
                            NativePortError::MalformedNestedSlashingProof(_)
                        )
                    )
                ))
            ));
            assert_eq!(factories.get(), 1);
            assert_eq!(*sequences.lock().unwrap(), vec![0, 0]);
            assert_eq!(reader.accounts, before);
        }
    });
}

#[test]
fn actual_native_estimate_probes_and_retry_use_fresh_sequence_and_state() {
    with_chain(|chain| {
        let mut reader = Reader::new();
        let mut code = native_parent(19_999, &malformed(), false);
        assert_eq!(code.pop(), Some(0));
        // Nonempty calldata reverts after the native attempt and storage write.
        let revert_target = u16::try_from(code.len() + 6).unwrap();
        code.extend_from_slice(&[0x36, 0x61]);
        code.extend_from_slice(&revert_target.to_be_bytes());
        code.extend_from_slice(&[0x57, 0, 0x5b, 0x60, 0, 0x60, 0, 0xfd]);
        reader.add_account(PARENT, 0, Some(code));
        let before = reader.accounts.clone();
        let sim = CorrectedSimulation::new(&reader, binding(&reader, STAGE)).unwrap();
        let sequences = Arc::new(Mutex::new(Vec::new()));
        let factories = Cell::new(0);
        let estimate = sim
            .estimate_with_native(
                &External,
                &External,
                &External,
                |origin| {
                    factories.set(factories.get() + 1);
                    native_factory(chain, origin, sequences.clone())
                },
                &tx(false, vec![]),
                |result| match result {
                    TransactionExecutionResult::Executed(result) => match result.status {
                        CodeExecutionStatus::Success => EstimateProbe::Success {
                            gas_used: result.gas_used.as_u64(),
                        },
                        CodeExecutionStatus::Failure(_) => EstimateProbe::CodeFailure {
                            error: "candidate failed".into(),
                        },
                    },
                    TransactionExecutionResult::ConsensusFailure(_) => {
                        EstimateProbe::ConsensusFailure {
                            error: "admission failed".into(),
                        }
                    }
                },
            )
            .unwrap();
        assert!(estimate >= 21_000);
        assert!(factories.get() > 1);
        assert!(sequences.lock().unwrap().len() >= 2);
        assert!(
            sequences
                .lock()
                .unwrap()
                .iter()
                .all(|sequence| *sequence == 0)
        );
        let failed = tx(false, vec![1]);
        let prepares_before_failure = sequences.lock().unwrap().len();
        let failed = sim
            .with_native(
                &External,
                &External,
                &External,
                |origin| native_factory(chain, origin, sequences.clone()),
                &failed,
            )
            .unwrap();
        assert_eq!(sequences.lock().unwrap().len(), prepares_before_failure + 1);
        assert_eq!(
            executed(failed.execution).status,
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        );
        let first = sim
            .with_native(
                &External,
                &External,
                &External,
                |origin| native_factory(chain, origin, sequences.clone()),
                &tx(false, vec![]),
            )
            .unwrap();
        let second = sim
            .with_native(
                &External,
                &External,
                &External,
                |origin| native_factory(chain, origin, sequences.clone()),
                &tx(false, vec![]),
            )
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            executed(first.execution).status,
            CodeExecutionStatus::Success
        );
        assert!(
            sequences
                .lock()
                .unwrap()
                .iter()
                .all(|sequence| *sequence == 0)
        );
        assert_eq!(reader.accounts, before);
    });
}
