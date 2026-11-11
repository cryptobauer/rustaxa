//! Independent hard-error checks through the real consensus native port.
//! Lifecycle checks cover concrete staging; FinalChain publication has a separate owner test.

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::FinalChain;
use rustaxa_evm::{
    contracts::*,
    driver::{
        ExecutionDriverError, NativeAddressClassifier, PeriodConsensusSequence,
        execute_top_level_call_with_native, execute_top_level_create_with_native,
    },
    envelope::EnvelopeRules,
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
    native::{NativeAdapterError, invoke_native},
    profile::{CorrectionStage, TaraxaProfile},
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteCommitApproval, ConcreteStateLifecycle,
    ConcreteStateMutationBatch, ConcreteStorageMutation, Config, Storage,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce, FinalChainRewardsConfig, GenesisDposConfig,
    codec::rlp::concrete_lifecycle::*, concrete_lifecycle::*, concrete_state::*,
};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[path = "../../rustaxa-storage/src/closeout_test_scratch.rs"]
pub mod closeout_test_scratch;
#[allow(dead_code)]
#[path = "support/mixed_native.rs"]
mod mixed_native;

const SENDER: [u8; 20] = [0xaa; 20];
const PARENT: [u8; 20] = [0xbb; 20];
const PREFIX: [u8; 20] = [0xcc; 20];
const SUFFIX: [u8; 20] = [0xdd; 20];
const SLASH: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
];
const STAGE: CorrectionStage = CorrectionStage::SlashingHardError;
const RULES: EnvelopeRules = EnvelopeRules { cornus: true };

#[derive(Clone, Default)]
struct Reader {
    accounts: BTreeMap<[u8; 20], ConcreteAccount>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    reads: Arc<AtomicUsize>,
}
impl Reader {
    fn funded() -> Self {
        let mut result = Self::default();
        result.accounts.insert(SENDER, account(1, 10_000_000));
        result.accounts.insert(SLASH, account(1, 0));
        result
    }
    fn add_code(&mut self, address: [u8; 20], code: Vec<u8>) {
        let hash = keccak256(&code).0;
        let mut value = account(1, 0);
        value.code_hash = Some(hash);
        value.code_size = code.len() as u64;
        self.accounts.insert(address, value);
        self.codes.insert(hash, code);
    }
}
impl ConcreteStateRead for Reader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.reads.fetch_add(1, Ordering::Relaxed);
        ConcreteStateIdentity {
            period: 0.into(),
            state_root: [0x44; 32],
        }
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .accounts
            .get(&address)
            .cloned()
            .map_or(ConcreteRead::Absent, |value| {
                ConcreteRead::Present(record(value))
            }))
    }
    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(ConcreteRead::Absent)
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}
fn account(nonce: u64, balance: u64) -> ConcreteAccount {
    ConcreteAccount {
        nonce: nonce.into(),
        balance: ConcreteAccountBalance::new(balance.into()),
        storage_root: None,
        code_hash: None,
        code_size: 0,
    }
}
fn record(value: ConcreteAccount) -> ConcreteAccountRecord {
    let mut raw = RlpStream::new_list(5);
    let balance = if value.balance.value() == &BigUint::default() {
        vec![]
    } else {
        value.balance.value().to_bytes_be()
    };
    raw.append(&value.nonce.to_bytes()).append(&balance);
    if let Some(root) = value.storage_root {
        raw.append(&root.as_slice());
    } else {
        raw.append_empty_data();
    }
    if let Some(hash) = value.code_hash {
        raw.append(&hash.as_slice());
    } else {
        raw.append_empty_data();
    }
    raw.append(&value.code_size);
    ConcreteAccountRecord {
        account: value,
        physical_rlp: raw.out().to_vec(),
    }
}
struct External;
impl BlockHashRead for External {
    fn block_hash(&self, _: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        panic!("unexpected block hash read")
    }
}
impl NativeAddressClassifier for External {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address == SLASH
    }
}
fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: 1.into(),
        author: [0; 20],
        timestamp: 1,
        gas_limit: 1_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}
fn profile(stage: CorrectionStage) -> TaraxaProfile {
    TaraxaProfile::new(false).with_correction_stage(stage)
}
fn tx(address: [u8; 20], nonce: u64, input: Vec<u8>) -> ExecutionTransaction {
    ExecutionTransaction {
        position: (nonce as u32 - 1).into(),
        hash: [nonce as u8; 32],
        sender: SENDER,
        receiver: Some(address),
        nonce: nonce.into(),
        gas_price: ExecutionGasPrice::new(1_u8.into()),
        gas_limit: 150_000.into(),
        value: ExecutionValue::default(),
        input,
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}
fn invocation(input: Vec<u8>, gas: u64) -> NativeInvocation {
    NativeInvocation {
        id: NativeInvocationId {
            transaction: 0.into(),
            sequence: 0,
        },
        period: 1.into(),
        depth: 0,
        kind: NativeCallKind::Call,
        is_static: false,
        caller: SENDER,
        contract: SLASH,
        state_address: SLASH,
        value: ExecutionValue::default(),
        input,
        supplied_gas: gas.into(),
    }
}
fn scratch(name: &str) -> PathBuf {
    scratch_path(name, true)
}
fn scratch_path(name: &str, create: bool) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!(
            "slashing-hard-error-{name}-{}-{stamp}",
            std::process::id()
        )),
        create,
    )
}
struct Owner {
    chain: FinalChain,
    storage: Arc<Storage>,
    path: PathBuf,
}
impl Owner {
    fn new() -> Self {
        let path = scratch("native");
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
                cornus_period: FinalChainBlockNumber::GENESIS,
                ..Default::default()
            },
        )
        .unwrap()
        .with_correction_stage(STAGE);
        Self {
            chain,
            storage,
            path,
        }
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = &self.storage;
        closeout_test_scratch::dispose(&self.path).unwrap();
    }
}
// Fields have the approved vote and sortition widths; zero signatures are semantic failures.
fn vote(sortition: &[u8], block: u8) -> Vec<u8> {
    let mut out = RlpStream::new_list(3);
    out.append(&vec![block; 32])
        .append(&sortition)
        .append(&vec![0_u8; 65]);
    out.out().to_vec()
}
fn sortition() -> Vec<u8> {
    let mut out = RlpStream::new_list(4);
    out.append(&1_u64)
        .append(&2_u32)
        .append(&4_u32)
        .append(&vec![0_u8; 80]);
    out.out().to_vec()
}
fn abi(a: &[u8], b: &[u8]) -> Vec<u8> {
    fn word(value: usize) -> [u8; 32] {
        let mut out = [0; 32];
        out[24..].copy_from_slice(&(value as u64).to_be_bytes());
        out
    }
    let padded = |size: usize| size.div_ceil(32) * 32;
    let mut out = vec![0xfa, 0xc7, 0xc9, 0x4a];
    out.extend_from_slice(&word(64));
    out.extend_from_slice(&word(96 + padded(a.len())));
    for value in [a, b] {
        out.extend_from_slice(&word(value.len()));
        out.extend_from_slice(value);
        out.resize(out.len() + padded(value.len()) - value.len(), 0);
    }
    out
}
fn malformed_inputs() -> Vec<Vec<u8>> {
    let good = vote(&sortition(), 1);
    let bad_sortition = vote(&[0xc0], 2);
    vec![
        abi(&[0xc0], &good),
        abi(&good, &[0xc0]),
        abi(&bad_sortition, &good),
        abi(&good, &bad_sortition),
    ]
}
fn assert_hard(error: NativeAdapterError) {
    assert!(
        matches!(error, NativeAdapterError::Port(NativePortError::Domain(message)) if message.contains("malformed nested slashing proof RLP"))
    );
}
fn run<
    R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead,
    P: NativeExecutionPort,
>(
    journal: &mut ExecutionJournal<R>,
    port: &mut P,
    sequence: &mut PeriodConsensusSequence,
    transaction: &ExecutionTransaction,
) -> Result<TransactionExecutionResult, ExecutionDriverError> {
    execute_top_level_call_with_native(
        journal,
        &External,
        &External,
        &External,
        port,
        sequence,
        &block(),
        transaction,
        RULES,
        profile(STAGE),
    )
}
fn completed(value: TransactionExecutionResult) -> ExecutedTransactionResult {
    let TransactionExecutionResult::Executed(value) = value else {
        panic!("transaction must execute")
    };
    value
}

#[test]
fn actual_adapter_hard_error_and_action_gas_precedence() {
    let owner = Owner::new();
    for input in malformed_inputs() {
        let mut journal = ExecutionJournal::new_with_correction_stage(Reader::funded(), STAGE);
        let mut port = mixed_native::MixedNativeExecutionPort::new(
            owner
                .chain
                .begin_native_session(1.into(), 0.into())
                .unwrap(),
        );
        let low =
            invoke_native(&mut journal, &mut port, &invocation(input.clone(), 19_999)).unwrap();
        assert_eq!(
            low.status,
            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
        );
        assert_eq!(low.gas_left.as_u64(), 19_999);
        let mut request = invocation(input, 20_000);
        request.id.sequence = 1;
        assert_hard(invoke_native(&mut journal, &mut port, &request).unwrap_err());
        request.id.sequence = 2;
        assert!(
            matches!(invoke_native(&mut journal, &mut port, &request), Err(NativeAdapterError::Port(NativePortError::Domain(message))) if message.contains("Aborted"))
        );
    }
}

fn parent_code(child_gas: u16) -> Vec<u8> {
    // Copy calldata, invoke slashing, then change a slot and emit a log.
    let mut code = vec![
        0x36, 0x60, 0, 0x60, 0, 0x37, 0x60, 0, 0x60, 0, 0x36, 0x60, 0, 0x60, 0, 0x73,
    ];
    code.extend_from_slice(&SLASH);
    code.push(0x61);
    code.extend_from_slice(&child_gas.to_be_bytes());
    code.extend_from_slice(&[
        0xf1, 0x50, 0x60, 9, 0x60, 1, 0x55, 0x60, 0, 0x60, 0, 0xa0, 0,
    ]);
    code
}
#[test]
fn actual_nested_call_hard_error_prevents_parent_continuation() {
    let owner = Owner::new();
    for input in malformed_inputs() {
        let mut state = Reader::funded();
        state.add_code(PARENT, parent_code(20_000));
        let mut journal = ExecutionJournal::new_with_correction_stage(state, STAGE);
        let mut port = mixed_native::MixedNativeExecutionPort::new(
            owner
                .chain
                .begin_native_session(1.into(), 0.into())
                .unwrap(),
        );
        let mut sequence = PeriodConsensusSequence::new(1.into());
        let error = run(
            &mut journal,
            &mut port,
            &mut sequence,
            &tx(PARENT, 1, input),
        )
        .unwrap_err();
        let ExecutionDriverError::Native(error) = error else {
            panic!("native hard error expected: {error:?}")
        };
        assert_hard(error);
        assert_eq!(sequence.next_sequence(), 1);
        assert!(journal.logs().is_empty());
        assert_eq!(
            journal
                .ordinary_storage(
                    PARENT,
                    ConcreteStorageKey({
                        let mut k = [0; 32];
                        k[31] = 1;
                        k
                    })
                )
                .unwrap()
                .1,
            BigUint::default()
        );
    }
}

#[test]
fn actual_nested_normal_failures_allow_parent_continuation() {
    let owner = Owner::new();
    let good_a = vote(&sortition(), 1);
    let good_b = vote(&sortition(), 2);
    for (input, gas) in [
        (vec![0xfa, 0xc7, 0xc9, 0x4a], 20_000),
        (abi(&good_a, &good_b), 20_000),
        (malformed_inputs()[0].clone(), 19_999),
    ] {
        let mut state = Reader::funded();
        state.add_code(PARENT, parent_code(gas));
        let mut journal = ExecutionJournal::new_with_correction_stage(state, STAGE);
        let mut port = mixed_native::MixedNativeExecutionPort::new(
            owner
                .chain
                .begin_native_session(1.into(), 0.into())
                .unwrap(),
        );
        let mut sequence = PeriodConsensusSequence::new(1.into());
        let result = completed(
            run(
                &mut journal,
                &mut port,
                &mut sequence,
                &tx(PARENT, 1, input),
            )
            .unwrap(),
        );
        assert_eq!(result.status, CodeExecutionStatus::Success);
        assert_eq!(result.logs.len(), 1);
        let settled = journal.settle_transaction().unwrap();
        assert_eq!(settled.native_invocations.len(), 1);
        let native = &settled.native_invocations[0];
        assert_eq!(native.required_gas.as_u64(), 20_000);
        assert_eq!(
            native.gas_used.as_u64(),
            if gas == 19_999 { 0 } else { 20_000 }
        );
        assert!(native.output.is_empty());
        assert!(native.logs.is_empty());
        assert_eq!(settled.writes.ordinary_storage.len(), 1);
        assert_eq!(
            settled.writes.ordinary_storage[0].value,
            BigUint::from(9_u8)
        );
        assert!(settled.writes.raw_storage.is_empty());
    }
}

struct StagePort(Option<CorrectionStage>);
impl NativeExecutionPort for StagePort {
    fn correction_stage(&self) -> Option<CorrectionStage> {
        self.0
    }
    fn prepare(
        &mut self,
        _: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        panic!("mismatch must precede prepare")
    }
    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        panic!("mismatch must precede invoke")
    }
}
#[test]
fn port_identity_mismatch_has_no_reader_sequence_or_journal_effects() {
    for (stage, port_stage) in [
        (STAGE, Some(CorrectionStage::Historical)),
        (STAGE, None),
        (CorrectionStage::Historical, Some(STAGE)),
    ] {
        for create in [false, true] {
            let mut journal = ExecutionJournal::new_with_correction_stage(Reader::funded(), stage);
            let mut port = StagePort(port_stage);
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let mut transaction = tx(PARENT, 1, vec![]);
            if create {
                transaction.kind = ExecutionTransactionKind::Create;
                transaction.receiver = None;
            }
            let execute = if create {
                execute_top_level_create_with_native
            } else {
                execute_top_level_call_with_native
            };
            let error = execute(
                &mut journal,
                &External,
                &External,
                &External,
                &mut port,
                &mut sequence,
                &block(),
                &transaction,
                RULES,
                profile(stage),
            )
            .unwrap_err();
            assert!(
                matches!(error,ExecutionDriverError::Native(NativeAdapterError::CorrectionStageMismatch {journal: got,port: got_port}) if got==stage && got_port==port_stage)
            );
            assert_eq!(journal.reader().reads.load(Ordering::Relaxed), 0);
            assert_eq!(sequence.next_sequence(), 0);
            assert!(journal.logs().is_empty());
            assert_eq!(journal.refund(), 0);
            let error =
                invoke_native(&mut journal, &mut port, &invocation(vec![], 20_000)).unwrap_err();
            assert_eq!(
                error,
                NativeAdapterError::CorrectionStageMismatch {
                    journal: stage,
                    port: port_stage
                }
            );
            assert_eq!(journal.reader().reads.load(Ordering::Relaxed), 0);
            assert!(
                journal
                    .settle_transaction()
                    .unwrap()
                    .native_invocations
                    .is_empty()
            );
        }
    }
}

fn period_state() -> Reader {
    let mut state = Reader::funded();
    let code = vec![0x60, 9, 0x60, 1, 0x55, 0x60, 0, 0x60, 0, 0xa0, 0];
    state.add_code(PREFIX, code.clone());
    state.add_code(SUFFIX, code);
    state
}
#[test]
fn memory_prefix_is_discarded_and_fresh_attempt_succeeds() {
    let owner = Owner::new();
    let state = period_state();
    let before = state.accounts.clone();
    let mut journal = ExecutionJournal::new_with_correction_stage(state.clone(), STAGE);
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(1.into());
    let prefix = completed(
        run(
            &mut journal,
            &mut port,
            &mut sequence,
            &tx(PREFIX, 1, vec![]),
        )
        .unwrap(),
    );
    assert_eq!(prefix.status, CodeExecutionStatus::Success);
    assert_eq!(prefix.logs.len(), 1);
    let receipt = journal.settle_transaction().unwrap();
    assert_eq!(receipt.logs.len(), 1);
    assert!(!receipt.writes.accounts.is_empty());
    assert_eq!(receipt.writes.ordinary_storage.len(), 1);
    let error = run(
        &mut journal,
        &mut port,
        &mut sequence,
        &tx(SLASH, 2, malformed_inputs()[0].clone()),
    )
    .unwrap_err();
    let ExecutionDriverError::Native(error) = error else {
        panic!("expected native hard error")
    };
    assert_hard(error);
    assert_eq!(sequence.next_sequence(), 1);
    assert_eq!(
        journal
            .ordinary_storage(
                SUFFIX,
                ConcreteStorageKey({
                    let mut k = [0; 32];
                    k[31] = 1;
                    k
                })
            )
            .unwrap()
            .1,
        BigUint::default()
    );
    drop((receipt, journal, port, sequence));
    assert_eq!(state.accounts, before);
    let mut journal = ExecutionJournal::new_with_correction_stage(state, STAGE);
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(1.into());
    for (nonce, address) in [(1, PREFIX), (2, SUFFIX)] {
        let result = completed(
            run(
                &mut journal,
                &mut port,
                &mut sequence,
                &tx(address, nonce, vec![]),
            )
            .unwrap(),
        );
        assert_eq!(result.status, CodeExecutionStatus::Success);
        assert_eq!(result.logs.len(), 1);
        journal.settle_transaction().unwrap();
    }
    assert_eq!(sequence.next_sequence(), 0);
}

fn seed(state: &Reader) -> ConcreteStateMutationBatch {
    ConcreteStateMutationBatch {
        accounts: state
            .accounts
            .iter()
            .map(|(address, value)| ConcreteAccountMutation::Upsert {
                address: *address,
                record: record(value.clone()),
            })
            .collect(),
        code: state
            .codes
            .iter()
            .map(|(hash, code)| ConcreteCodeInsertion {
                code_hash: *hash,
                code: code.clone(),
            })
            .collect(),
        ..Default::default()
    }
}
fn delta(plan: JournalWritePlan) -> ConcreteStateMutationBatch {
    assert!(plan.raw_storage.is_empty());
    ConcreteStateMutationBatch {
        accounts: plan
            .accounts
            .into_iter()
            .map(|entry| match entry.operation {
                JournalAccountOperation::Upsert {
                    nonce,
                    balance,
                    code_hash,
                    code_size,
                } => ConcreteAccountMutation::Upsert {
                    address: entry.address,
                    record: record(ConcreteAccount {
                        nonce,
                        balance,
                        storage_root: None,
                        code_hash,
                        code_size,
                    }),
                },
                JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                    address: entry.address,
                },
            })
            .collect(),
        storage: plan
            .ordinary_storage
            .into_iter()
            .map(|entry| ConcreteStorageMutation {
                address: entry.address,
                key: entry.key,
                value: (entry.value != BigUint::default()).then(|| entry.value.to_bytes_be()),
            })
            .collect(),
        code: plan
            .code
            .into_iter()
            .map(|entry| ConcreteCodeInsertion {
                code_hash: entry.code_hash,
                code: entry.code,
            })
            .collect(),
    }
}
type ConcreteDatabaseRows = Vec<(String, Vec<(Vec<u8>, Vec<u8>)>)>;
fn rows(path: &std::path::Path) -> ConcreteDatabaseRows {
    let options = rocksdb::Options::default();
    let mut columns = rocksdb::DB::list_cf(&options, path).unwrap();
    columns.sort();
    let db = rocksdb::DB::open_cf_for_read_only(&options, path, &columns, false).unwrap();
    columns
        .into_iter()
        .map(|column| {
            let values = db
                .iterator_cf(
                    &db.cf_handle(&column).unwrap(),
                    rocksdb::IteratorMode::Start,
                )
                .map(|row| {
                    let (key, value) = row.unwrap();
                    (key.to_vec(), value.to_vec())
                })
                .collect();
            (column, values)
        })
        .collect()
}

fn marker(
    prior: &rustaxa_storage::ConcreteLifecycleObservation,
) -> FinalChainConcreteExecutionMarker {
    FinalChainConcreteExecutionMarker {
        identity: prior.identity,
        generation: prior.generation + 1,
        plan_hash: [0x11; 32],
        period: 1,
        prior_state: FinalChainConcreteState {
            period: 0,
            root: prior.committed.state_root,
        },
        transactions_hash: [0x22; 32],
        rewards_hash: [0x33; 32],
    }
}
enum ConcreteView<'a> {
    Prior(&'a ConcreteStateLifecycle),
    Prepared(rustaxa_storage::PreparedConcreteView<'a>),
}
impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for ConcreteView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        match self {
            Self::Prior(owner) => owner.prior_reader().identity(),
            Self::Prepared(view) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::identity(view)
            }
        }
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        match self {
            Self::Prior(owner) => owner.prior_reader().account(address),
            Self::Prepared(view) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                    view, address,
                )
            }
        }
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let read = match self {
            Self::Prior(owner) => owner.prior_reader().storage(address, key),
            Self::Prepared(view) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    view, address, key,
                )
            }
        };
        if let Err(ConcreteReadError::HistoryUnavailable(identity)) = &read {
            if *identity != self.identity() {
                return read;
            }
            let proof = match self {
                Self::Prior(owner) => owner.verify_prior_storage_path(address, key),
                Self::Prepared(view) => view.verify_storage_path(address, key),
            };
            if matches!(proof, Ok(rustaxa_storage::ConcreteStoragePath::NonMember)) {
                return Ok(ConcreteRead::Absent);
            }
        }
        read
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self {
            Self::Prior(owner) => owner.prior_reader().code(hash),
            Self::Prepared(view) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::code(view, hash)
            }
        }
    }
}
fn observer_delta(plan: JournalWritePlan) -> rustaxa_storage::ConcreteObserverPhaseDelta {
    let batch = delta(plan);
    rustaxa_storage::ConcreteObserverPhaseDelta {
        accounts: batch.accounts,
        storage: batch.storage,
        code: batch.code,
    }
}
fn log(address: [u8; 20]) -> ExecutionLog {
    ExecutionLog {
        address,
        topics: vec![],
        data: vec![],
    }
}
fn slot_one() -> ConcreteStorageKey {
    let mut key = [0; 32];
    key[31] = 1;
    ConcreteStorageKey(key)
}
#[test]
fn concrete_staged_prefix_abort_reopen_and_valid_control() {
    let owner = Owner::new();
    let state = period_state();
    let path = scratch_path("concrete", false);
    let chain_id = [0x77; 32];
    let lifecycle =
        ConcreteStateLifecycle::create_fresh_exclusive(&path, chain_id, seed(&state), vec![])
            .unwrap();
    closeout_test_scratch::created(&path);
    let prior = lifecycle.observation().unwrap();
    drop(lifecycle);
    let before = rows(&path);
    let mut lifecycle = ConcreteStateLifecycle::open(&path, chain_id, prior.committed).unwrap();
    let marker = marker(&prior);
    let marker_rlp = encode_concrete_execution_marker(&marker);
    lifecycle.stage_execution(&marker_rlp).unwrap();
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(1.into());
    let receipt = {
        let mut journal =
            ExecutionJournal::new_with_correction_stage(ConcreteView::Prior(&lifecycle), STAGE);
        let prefix = completed(
            run(
                &mut journal,
                &mut port,
                &mut sequence,
                &tx(PREFIX, 1, vec![]),
            )
            .unwrap(),
        );
        assert_eq!(prefix.status, CodeExecutionStatus::Success);
        assert_eq!(prefix.gas_used.as_u64(), 21_000 + 20_000 + 4 * 3 + 375);
        assert_eq!(prefix.logs, vec![log(PREFIX)]);
        journal.settle_transaction().unwrap()
    };
    assert_eq!(receipt.logs, vec![log(PREFIX)]);
    let pending = lifecycle
        .apply_observer_phase(observer_delta(receipt.writes))
        .unwrap();
    assert_ne!(pending.identity().state_root, prior.committed.state_root);
    {
        let view = lifecycle.prepared_view(&pending).unwrap();
        let ConcreteRead::Present(sender) =
            rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(&view, SENDER)
                .unwrap()
        else {
            panic!("sender must exist")
        };
        assert_eq!(sender.account.nonce, FinalChainNonce::from_u64(2));
        assert_eq!(
            sender.account.balance.value(),
            &BigUint::from(10_000_000_u64 - 41_387)
        );
    }
    let mut attempted = 0;
    for transaction in [
        tx(SLASH, 2, malformed_inputs()[0].clone()),
        tx(SUFFIX, 3, vec![]),
    ] {
        attempted += 1;
        let mut journal = ExecutionJournal::new_with_correction_stage(
            ConcreteView::Prepared(lifecycle.prepared_view(&pending).unwrap()),
            STAGE,
        );
        match run(&mut journal, &mut port, &mut sequence, &transaction) {
            Err(ExecutionDriverError::Native(error)) => {
                assert_hard(error);
                break;
            }
            Ok(_) => {}
            other => panic!("malformed middle must stop the period: {other:?}"),
        }
    }
    assert_eq!(attempted, 1);
    assert_eq!(sequence.next_sequence(), 1);
    drop((port, sequence, pending));
    lifecycle.discard_execution(&marker_rlp).unwrap();
    assert_eq!(lifecycle.observation().unwrap(), prior);
    drop(lifecycle);
    assert_eq!(
        rows(&path),
        before,
        "all concrete columns must remain byte-identical"
    );
    let mut lifecycle = ConcreteStateLifecycle::open(&path, chain_id, prior.committed).unwrap();
    assert_eq!(lifecycle.observation().unwrap(), prior);
    lifecycle.stage_execution(&marker_rlp).unwrap();
    let mut port = mixed_native::MixedNativeExecutionPort::new(
        owner
            .chain
            .begin_native_session(1.into(), 0.into())
            .unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(1.into());
    let mut output: Option<rustaxa_storage::ConcreteObserverPhaseOutput> = None;
    let mut logs = vec![];
    for (nonce, address, expected_gas) in [
        (1, PREFIX, 41_387),
        (2, SUFFIX, 41_387),
        (3, PREFIX, 21_587),
    ] {
        let receipt = {
            let view = match &output {
                None => ConcreteView::Prior(&lifecycle),
                Some(previous) => {
                    ConcreteView::Prepared(lifecycle.prepared_view(previous).unwrap())
                }
            };
            let mut journal = ExecutionJournal::new_with_correction_stage(view, STAGE);
            let result = completed(
                run(
                    &mut journal,
                    &mut port,
                    &mut sequence,
                    &tx(address, nonce, vec![]),
                )
                .unwrap(),
            );
            assert_eq!(result.status, CodeExecutionStatus::Success);
            assert_eq!(result.gas_used.as_u64(), expected_gas);
            assert_eq!(result.logs, vec![log(address)]);
            logs.extend(result.logs);
            journal.settle_transaction().unwrap()
        };
        output = Some(
            lifecycle
                .apply_observer_phase(observer_delta(receipt.writes))
                .unwrap(),
        );
    }
    assert_eq!(logs, vec![log(PREFIX), log(SUFFIX), log(PREFIX)]);
    assert_eq!(sequence.next_sequence(), 0);
    drop((port, sequence));
    let output = output.unwrap();
    let next = output.identity();
    let catalog_hash = concrete_storage_slot_catalog_hash(std::iter::empty());
    let projection_hash = [0x55; 32];
    let provenance = FinalChainConcreteStateProvenance {
        identity: marker.identity,
        generation: marker.generation,
        plan_hash: marker.plan_hash,
        committed_state: FinalChainConcreteState {
            period: 1,
            root: next.state_root,
        },
        transactions_hash: marker.transactions_hash,
        rewards_hash: marker.rewards_hash,
        projection_hash,
        catalog_hash,
    };
    let approval = ConcreteCommitApproval {
        marker_rlp,
        provenance_rlp: encode_concrete_state_provenance(&provenance),
        catalog_rlp: encode_concrete_storage_catalog(std::iter::empty()),
        projection_hash,
        catalog_hash,
    };
    let published = lifecycle
        .commit_observer_approved(output, approval)
        .unwrap();
    assert_eq!(published.committed, next);
    assert_eq!(published.generation, prior.generation + 1);
    assert!(published.pending_marker_rlp.is_empty());
    assert_ne!(rows(&path), before);
    let reopened = ConcreteStateLifecycle::open(&path, chain_id, next).unwrap();
    assert_eq!(reopened.observation().unwrap(), published);
    let ConcreteRead::Present(sender) = reopened.prior_reader().account(SENDER).unwrap() else {
        panic!("sender must exist")
    };
    assert_eq!(sender.account.nonce, FinalChainNonce::from_u64(4));
    assert_eq!(
        sender.account.balance.value(),
        &BigUint::from(10_000_000_u64 - 41_387 - 41_387 - 21_587)
    );
    for address in [PREFIX, SUFFIX] {
        let ConcreteRead::Present(record) = reopened.prior_reader().account(address).unwrap()
        else {
            panic!("contract must exist")
        };
        assert_eq!(record.account.nonce, FinalChainNonce::from_u64(1));
        assert_eq!(record.account.balance.value(), &BigUint::default());
        assert!(record.account.storage_root.is_some());
        assert_eq!(
            reopened
                .prior_reader()
                .storage(address, slot_one())
                .unwrap(),
            ConcreteRead::Present(vec![9])
        );
    }
    let ConcreteRead::Present(slash) = reopened.prior_reader().account(SLASH).unwrap() else {
        panic!("native account must exist")
    };
    assert_eq!(slash.account, account(1, 0));
    drop(reopened);
    closeout_test_scratch::dispose(&path).unwrap();
}

#[test]
fn actual_driver_nonce_and_full_gas_cap_checks_precede_nested_decode() {
    let owner = Owner::new();
    for nested in [false, true] {
        for stale in [false, true] {
            let mut state = Reader::funded();
            if nested {
                state.add_code(PARENT, parent_code(20_000));
            }
            if !stale {
                state.accounts.get_mut(&SENDER).unwrap().balance =
                    ConcreteAccountBalance::new(149_999_u64.into());
            }
            let mut journal = ExecutionJournal::new_with_correction_stage(state, STAGE);
            let mut port = mixed_native::MixedNativeExecutionPort::new(
                owner
                    .chain
                    .begin_native_session(1.into(), 0.into())
                    .unwrap(),
            );
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let mut transaction = tx(
                if nested { PARENT } else { SLASH },
                1,
                malformed_inputs()[0].clone(),
            );
            if stale {
                transaction.nonce = FinalChainNonce::zero();
            }
            let result = run(&mut journal, &mut port, &mut sequence, &transaction).unwrap();
            let TransactionExecutionResult::ConsensusFailure(result) = result else {
                panic!("admission must fail")
            };
            assert_eq!(
                result.error,
                if stale {
                    ConsensusFailure::NonceTooLow
                } else {
                    ConsensusFailure::InsufficientBalanceForGas
                }
            );
            assert_eq!(
                result.gas_used.as_u64(),
                if stale { 150_000 } else { 149_999 }
            );
            assert!(result.output.is_empty());
            assert_eq!(sequence.next_sequence(), 0);
            let receipt = journal.settle_transaction().unwrap();
            assert!(receipt.native_invocations.is_empty());
            assert!(receipt.logs.is_empty());
            assert!(receipt.writes.raw_storage.is_empty());
        }
    }
}
