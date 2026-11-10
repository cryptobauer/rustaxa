//! Isolated correction expectations from the approved frame and lifetime model.
//! These memory-only tests do not use the historical Go defects as an oracle.

use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutedTransactionResult, ExecutionBlockContext, ExecutionGasPrice, ExecutionLog,
        ExecutionTransaction, ExecutionTransactionKind, ExecutionValue, NativeCallKind,
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
        NativeJournalRead, NativePortError, NativeRawOperation, NativeRawValue, NativeStatus,
        StatelessInvocation, StatelessInvocationId, TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, PeriodConsensusSequence,
        execute_top_level_call, execute_top_level_call_with_native,
        execute_top_level_call_with_trace, execute_top_level_create,
        execute_top_level_create_with_native, execute_top_level_create_with_trace,
    },
    envelope::EnvelopeRules,
    falcon::{FALCON_VERIFY_ADDRESS, PreparedFalconCall},
    journal::ExecutionJournal,
    profile::{CorrectionStage, TaraxaPhase, TaraxaProfile},
    simulation::{SimulationError, simulate_ordinary, simulate_with_native},
    trace::{TraceCollector, TraceEvent, TraceOpcodePhase},
    trace_runner::{
        StructuredTraceRunnerError, run_structured_claim_all_prefix_trace, run_structured_trace,
        run_structured_trace_with_native,
    },
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};

const SENDER: [u8; 20] = [0xaa; 20];
const PARENT: [u8; 20] = [0xbb; 20];
const CHILD: [u8; 20] = [0xcc; 20];
const SIBLING: [u8; 20] = [0xdd; 20];
const KEY: ConcreteStorageKey = ConcreteStorageKey([1; 32]);
const ZERO: [u8; 32] = [0; 32];
const RULES: EnvelopeRules = EnvelopeRules { cornus: true };

#[derive(Clone, Default)]
struct Reader {
    accounts: BTreeMap<[u8; 20], ConcreteAccount>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    reads: Arc<AtomicUsize>,
}

impl Reader {
    fn funded() -> Self {
        let mut reader = Self::default();
        reader.accounts.insert(
            SENDER,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(10_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        );
        reader
    }

    fn add_code(&mut self, address: [u8; 20], code: Vec<u8>) {
        let hash = keccak256(&code).0;
        self.accounts.insert(
            address,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: Some(hash),
                code_size: code.len() as u64,
            },
        );
        self.codes.insert(hash, code);
    }
}

impl ConcreteStateRead for Reader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.reads.fetch_add(1, Ordering::Relaxed);
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(7),
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
            .map_or(ConcreteRead::Absent, |account| {
                ConcreteRead::Present(ConcreteAccountRecord {
                    account,
                    physical_rlp: vec![0xc0],
                })
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

struct NoExternal;
impl BlockHashRead for NoExternal {
    fn block_hash(&self, _: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        panic!("unexpected block hash read")
    }
}
impl NativeAddressClassifier for NoExternal {
    fn is_native_address(&self, _: FinalChainBlockNumber, _: [u8; 20]) -> bool {
        false
    }
}
struct UnusedPort;
impl NativeExecutionPort for UnusedPort {
    fn prepare(
        &mut self,
        _: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        panic!("native preparation must not run")
    }
    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        panic!("native invocation must not run")
    }
}

fn profile(stage: CorrectionStage, phase: TaraxaPhase) -> TaraxaProfile {
    TaraxaProfile::for_phase(phase).with_correction_stage(stage)
}
fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(7),
        author: [0; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}
fn transaction(create: bool) -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: (!create).then_some(PARENT),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(500_000),
        value: ExecutionValue::new(BigUint::default()),
        input: Vec::new(),
        canonical_rlp: None,
        kind: if create {
            ExecutionTransactionKind::Create
        } else {
            ExecutionTransactionKind::Call
        },
    }
}
fn execute(
    journal: &mut ExecutionJournal<Reader>,
    tx: &ExecutionTransaction,
    phase: TaraxaPhase,
) -> ExecutedTransactionResult {
    let run = if tx.kind == ExecutionTransactionKind::Create {
        execute_top_level_create
    } else {
        execute_top_level_call
    };
    let result = run(
        journal,
        &NoExternal,
        &NoExternal,
        &block(),
        tx,
        RULES,
        profile(journal.correction_stage(), phase),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("funded test transaction rejected")
    };
    result
}

#[test]
fn corrected_undo_restores_presence_values_and_nested_commits() {
    let mut journal = ExecutionJournal::new_with_correction_stage(
        Reader::funded(),
        CorrectionStage::TransientRollback,
    );
    journal.set_transient_storage(PARENT, KEY, [1; 32]);
    let outer = journal.checkpoint();
    journal.set_transient_storage(PARENT, KEY, ZERO);
    let inner = journal.checkpoint();
    journal.set_transient_storage(PARENT, KEY, [2; 32]);
    journal.set_transient_storage(CHILD, KEY, ZERO);
    journal.set_transient_storage(CHILD, KEY, [3; 32]);
    journal.set_transient_storage(CHILD, KEY, [4; 32]);
    journal.commit_checkpoint(inner).unwrap();
    assert_eq!(journal.transient_storage(CHILD, KEY), [4; 32]);
    journal.revert_checkpoint(outer).unwrap();
    assert_eq!(journal.transient_storage(PARENT, KEY), [1; 32]);
    assert_eq!(journal.transient_storage(CHILD, KEY), ZERO);

    let outer = journal.checkpoint();
    journal.set_transient_storage(PARENT, KEY, [5; 32]);
    let inner = journal.checkpoint();
    journal.set_transient_storage(PARENT, KEY, ZERO);
    journal.revert_checkpoint(inner).unwrap();
    assert_eq!(journal.transient_storage(PARENT, KEY), [5; 32]);
    let sibling = journal.checkpoint();
    journal.set_transient_storage(CHILD, KEY, [6; 32]);
    journal.commit_checkpoint(sibling).unwrap();
    journal.commit_checkpoint(outer).unwrap();
    assert_eq!(journal.transient_storage(CHILD, KEY), [6; 32]);
    let settled = journal.settle_transaction().unwrap();
    assert!(settled.writes.accounts.is_empty());
    assert!(settled.writes.ordinary_storage.is_empty());
    assert!(settled.writes.raw_storage.is_empty());
    assert_eq!(journal.transient_storage(PARENT, KEY), ZERO);
    assert_eq!(journal.transient_storage(CHILD, KEY), ZERO);
    assert_eq!(
        journal.correction_stage(),
        CorrectionStage::TransientRollback
    );
}

#[test]
fn corrected_undo_keeps_raw_storage_and_log_refund_lifetimes_separate() {
    let mut journal = ExecutionJournal::new_with_correction_stage(
        Reader::funded(),
        CorrectionStage::TransientRollback,
    );
    let outer = journal.checkpoint();
    journal
        .set_ordinary_storage(SENDER, KEY, BigUint::from(7_u8))
        .unwrap();
    journal
        .set_raw_storage(
            SENDER,
            KEY,
            NativeRawOperation::Put(NativeRawValue::new(vec![8]).unwrap()),
        )
        .unwrap();
    journal.set_transient_storage(SENDER, KEY, [9; 32]);
    journal.push_log(ExecutionLog {
        address: SENDER,
        topics: vec![],
        data: vec![1],
    });
    journal.add_refund(12).unwrap();
    journal.revert_checkpoint(outer).unwrap();
    assert_eq!(journal.transient_storage(SENDER, KEY), ZERO);
    assert_eq!(
        journal.raw_storage(SENDER, KEY).unwrap(),
        ConcreteRead::Present(vec![8])
    );
    assert!(journal.logs().is_empty());
    assert_eq!(journal.refund(), 0);
    let writes = journal.settle_transaction().unwrap().writes;
    assert_eq!(writes.ordinary_storage.len(), 1);
    assert_eq!(writes.ordinary_storage[0].value, BigUint::default());
    assert_eq!(writes.raw_storage.len(), 1);
}

#[test]
fn all_entry_variants_reject_both_mismatch_directions_before_effects() {
    for (journal_stage, requested) in [
        (
            CorrectionStage::Historical,
            CorrectionStage::TransientRollback,
        ),
        (
            CorrectionStage::TransientRollback,
            CorrectionStage::Historical,
        ),
        (
            CorrectionStage::Historical,
            CorrectionStage::SingleSelfDestructRefund,
        ),
        (
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::Historical,
        ),
        (
            CorrectionStage::TransientRollback,
            CorrectionStage::SingleSelfDestructRefund,
        ),
        (
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::TransientRollback,
        ),
        (
            CorrectionStage::Historical,
            CorrectionStage::CheckedReturnDataCopy,
        ),
        (
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::Historical,
        ),
        (
            CorrectionStage::TransientRollback,
            CorrectionStage::CheckedReturnDataCopy,
        ),
        (
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::TransientRollback,
        ),
        (
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::CheckedReturnDataCopy,
        ),
        (
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::SingleSelfDestructRefund,
        ),
        (
            CorrectionStage::Historical,
            CorrectionStage::BoundedFalconAbi,
        ),
        (
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::Historical,
        ),
        (
            CorrectionStage::TransientRollback,
            CorrectionStage::BoundedFalconAbi,
        ),
        (
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::TransientRollback,
        ),
        (
            CorrectionStage::SingleSelfDestructRefund,
            CorrectionStage::BoundedFalconAbi,
        ),
        (
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::SingleSelfDestructRefund,
        ),
        (
            CorrectionStage::CheckedReturnDataCopy,
            CorrectionStage::BoundedFalconAbi,
        ),
        (
            CorrectionStage::BoundedFalconAbi,
            CorrectionStage::CheckedReturnDataCopy,
        ),
    ] {
        for variant in 0..6 {
            let mut journal =
                ExecutionJournal::new_with_correction_stage(Reader::funded(), journal_stage);
            journal.set_transient_storage(PARENT, KEY, [7; 32]);
            let mut observer = TraceCollector::default();
            let mut sequence = PeriodConsensusSequence::new(block().period);
            let mut port = UnusedPort;
            let tx = transaction(variant >= 3);
            let p = profile(requested, TaraxaPhase::Cacti);
            let result = match variant {
                0 => execute_top_level_call(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &block(),
                    &tx,
                    RULES,
                    p,
                ),
                1 => execute_top_level_call_with_trace(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &block(),
                    &tx,
                    RULES,
                    p,
                    &mut observer,
                ),
                2 => execute_top_level_call_with_native(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &NoExternal,
                    &mut port,
                    &mut sequence,
                    &block(),
                    &tx,
                    RULES,
                    p,
                ),
                3 => execute_top_level_create(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &block(),
                    &tx,
                    RULES,
                    p,
                ),
                4 => execute_top_level_create_with_trace(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &block(),
                    &tx,
                    RULES,
                    p,
                    &mut observer,
                ),
                5 => execute_top_level_create_with_native(
                    &mut journal,
                    &NoExternal,
                    &NoExternal,
                    &NoExternal,
                    &mut port,
                    &mut sequence,
                    &block(),
                    &tx,
                    RULES,
                    p,
                ),
                _ => unreachable!(),
            };
            assert_eq!(
                result,
                Err(ExecutionDriverError::CorrectionStageMismatch {
                    journal: journal_stage,
                    profile: requested
                })
            );
            assert_eq!(journal.reader().reads.load(Ordering::Relaxed), 0);
            assert_eq!(sequence.next_sequence(), 0);
            assert!(observer.events().is_empty());
            assert!(journal.logs().is_empty());
            assert_eq!(journal.refund(), 0);
            assert_eq!(journal.transient_storage(PARENT, KEY), [7; 32]);
            let sender = journal.account(SENDER).unwrap();
            assert_eq!(sender.nonce, FinalChainNonce::from_u64(1));
            assert_eq!(
                sender.balance.value(),
                &num_bigint::BigInt::from(10_000_000_u64)
            );
            assert!(
                journal
                    .settle_transaction()
                    .unwrap()
                    .writes
                    .accounts
                    .is_empty()
            );
        }
    }
}

#[test]
fn historical_wrappers_reject_corrections_before_reads_or_native_factory() {
    for stage in [
        CorrectionStage::TransientRollback,
        CorrectionStage::SingleSelfDestructRefund,
        CorrectionStage::CheckedReturnDataCopy,
        CorrectionStage::BoundedFalconAbi,
    ] {
        let reader = Reader::funded();
        let p = profile(stage, TaraxaPhase::Ficus);
        let tx = transaction(false);
        assert_eq!(
            simulate_ordinary(&reader, &NoExternal, &NoExternal, &block(), &tx, RULES, p),
            Err(SimulationError::UnsupportedCorrectionStage(stage))
        );
        assert_eq!(
            simulate_with_native(
                &reader,
                &NoExternal,
                &NoExternal,
                &NoExternal,
                |_| -> Result<UnusedPort, NativePortError> { panic!("factory must not run") },
                &block(),
                &tx,
                RULES,
                p
            ),
            Err(SimulationError::UnsupportedCorrectionStage(stage))
        );
        let expected = StructuredTraceRunnerError::UnsupportedCorrectionStage(stage);
        assert_eq!(
            run_structured_trace(
                &reader,
                &NoExternal,
                &NoExternal,
                &block(),
                &[],
                std::slice::from_ref(&tx),
                RULES,
                p
            )
            .unwrap_err(),
            expected
        );
        assert_eq!(
            run_structured_trace_with_native(
                &reader,
                &NoExternal,
                &NoExternal,
                &NoExternal,
                |_, _| -> Result<UnusedPort, NativePortError> { panic!("factory must not run") },
                &block(),
                &[],
                std::slice::from_ref(&tx),
                RULES,
                p
            )
            .unwrap_err(),
            expected
        );
        // The exact retained claim-all prefix profile is rejected before its inputs are inspected.
        assert_eq!(
            run_structured_claim_all_prefix_trace(
                &reader,
                &NoExternal,
                &NoExternal,
                &NoExternal,
                |_| -> Result<UnusedPort, NativePortError> { panic!("factory must not run") },
                &block(),
                &[],
                std::slice::from_ref(&tx),
                RULES,
                p
            )
            .unwrap_err(),
            expected
        );
        assert_eq!(reader.reads.load(Ordering::Relaxed), 0);
    }
}

fn store(value: u8, opcode: u8) -> Vec<u8> {
    vec![0x60, value, 0x60, 1, opcode]
}
fn load_return(opcode: u8) -> Vec<u8> {
    vec![0x60, 1, opcode, 0x60, 0, 0x52, 0x60, 32, 0x60, 0, 0xf3]
}
fn call(address: [u8; 20], opcode: u8) -> Vec<u8> {
    let mut code = vec![0x60, 32, 0x60, 0, 0x60, 0, 0x60, 0];
    if opcode == 0xf1 || opcode == 0xf2 {
        code.extend([0x60, 0]);
    }
    code.push(0x73);
    code.extend(address);
    code.extend([0x62, 0x01, 0x86, 0xa0, opcode, 0x50]);
    code
}
fn slot_one() -> ConcreteStorageKey {
    let mut key = ZERO;
    key[31] = 1;
    ConcreteStorageKey(key)
}
fn word(value: u8) -> [u8; 32] {
    let mut word = ZERO;
    word[31] = value;
    word
}

#[test]
fn frame_rollback_and_address_scope_hold_for_all_call_kinds_and_aliases() {
    for phase in [
        TaraxaPhase::Californicum,
        TaraxaPhase::Ficus,
        TaraxaPhase::Cacti,
    ] {
        let aliases: &[(u8, u8)] = if phase == TaraxaPhase::Cacti {
            &[(0xb3, 0xb4), (0x5c, 0x5d)]
        } else {
            &[(0xb3, 0xb4)]
        };
        for &(load, write) in aliases {
            for opcode in [0xf1, 0xf2, 0xf4] {
                for revert in [false, true] {
                    let mut reader = Reader::funded();
                    let mut parent = store(1, write);
                    parent.extend(call(CHILD, opcode));
                    parent.extend(load_return(load));
                    let mut child = store(2, write);
                    child.extend(if revert {
                        vec![0x60, 0, 0x60, 0, 0xfd]
                    } else {
                        vec![0x00]
                    });
                    reader.add_code(PARENT, parent);
                    reader.add_code(CHILD, child);
                    let mut journal = ExecutionJournal::new_with_correction_stage(
                        reader,
                        CorrectionStage::TransientRollback,
                    );
                    let result = execute(&mut journal, &transaction(false), phase);
                    assert_eq!(result.status, CodeExecutionStatus::Success);
                    let expected = if opcode == 0xf1 || revert { 1 } else { 2 };
                    assert_eq!(result.output, word(expected));
                    assert_eq!(
                        journal.transient_storage(PARENT, slot_one()),
                        word(expected)
                    );
                    assert_eq!(
                        journal.transient_storage(CHILD, slot_one()),
                        if opcode == 0xf1 && !revert {
                            word(2)
                        } else {
                            ZERO
                        }
                    );
                    journal.settle_transaction().unwrap();
                    assert_eq!(journal.transient_storage(PARENT, slot_one()), ZERO);
                }
            }
        }
    }
}

#[test]
fn parent_revert_undoes_committed_delegate_write_and_sibling_sees_restored_value() {
    for parent_revert in [false, true] {
        let mut reader = Reader::funded();
        let mut parent = store(1, 0xb4);
        parent.extend(call(CHILD, 0xf4));
        parent.extend(call(SIBLING, 0xf4));
        parent.extend([0x60, 32, 0x60, 0, if parent_revert { 0xfd } else { 0xf3 }]);
        let mut child = store(2, 0xb4);
        child.extend(if parent_revert {
            vec![0]
        } else {
            vec![0x60, 0, 0x60, 0, 0xfd]
        });
        reader.add_code(PARENT, parent);
        reader.add_code(CHILD, child);
        reader.add_code(SIBLING, load_return(0xb3));
        let mut journal =
            ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
        journal.set_transient_storage(PARENT, slot_one(), word(7));
        let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
        assert_eq!(result.output, word(if parent_revert { 2 } else { 1 }));
        assert_eq!(
            result.status,
            if parent_revert {
                CodeExecutionStatus::Failure(CodeExecutionError::Revert)
            } else {
                CodeExecutionStatus::Success
            }
        );
        assert_eq!(
            journal.transient_storage(PARENT, slot_one()),
            word(if parent_revert { 7 } else { 1 })
        );
        journal.settle_transaction().unwrap();
        assert_eq!(journal.transient_storage(PARENT, slot_one()), ZERO);
    }
}

#[test]
fn static_child_loads_but_cannot_store_transient_values() {
    for write in [false, true] {
        let mut reader = Reader::funded();
        let mut parent = call(CHILD, 0xfa);
        parent.extend([0x60, 32, 0x60, 0, 0xf3]);
        let child = if write {
            store(2, 0xb4)
        } else {
            load_return(0xb3)
        };
        reader.add_code(PARENT, parent);
        reader.add_code(CHILD, child);
        let mut journal =
            ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
        journal.set_transient_storage(CHILD, slot_one(), word(7));
        let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
        assert_eq!(result.status, CodeExecutionStatus::Success);
        assert_eq!(result.output, if write { ZERO } else { word(7) });
        assert_eq!(journal.transient_storage(CHILD, slot_one()), word(7));
    }
}

#[test]
fn historical_defaults_keep_the_accepted_transient_lifetime() {
    assert_eq!(
        TaraxaProfile::default().correction_stage(),
        CorrectionStage::Historical
    );
    for p in [
        TaraxaProfile::new(false),
        TaraxaProfile::new(true),
        TaraxaProfile::for_phase(TaraxaPhase::Ficus),
    ] {
        assert_eq!(p.correction_stage(), CorrectionStage::Historical);
    }
    let mut journal = ExecutionJournal::new(Reader::funded());
    let checkpoint = journal.checkpoint();
    journal.set_transient_storage(PARENT, KEY, [8; 32]);
    journal.revert_checkpoint(checkpoint).unwrap();
    assert_eq!(journal.transient_storage(PARENT, KEY), [8; 32]);
    journal.settle_transaction().unwrap();
    assert_eq!(journal.transient_storage(PARENT, KEY), ZERO);
}

#[test]
fn top_level_creation_restores_transient_state_on_failure_and_resets_on_settlement() {
    // CREATE(sender, nonce=1) uses the transaction nonce, encoded independently here.
    let mut rlp = rlp::RlpStream::new_list(2);
    rlp.append(&SENDER.as_slice());
    rlp.append(&1_u64);
    let hash = keccak256(rlp.out());
    let created: [u8; 20] = hash[12..].try_into().unwrap();
    for terminal in [0xf3, 0xfd, 0xfe] {
        let mut tx = transaction(true);
        tx.input = store(2, 0xb4);
        tx.input.extend([0x60, 0, 0x60, 0, terminal]);
        let mut journal = ExecutionJournal::new_with_correction_stage(
            Reader::funded(),
            CorrectionStage::TransientRollback,
        );
        journal.set_transient_storage(created, slot_one(), word(7));
        let result = execute(&mut journal, &tx, TaraxaPhase::Cacti);
        assert_eq!(result.attempted_contract_address, Some(created));
        assert_eq!(
            result.status,
            match terminal {
                0xf3 => CodeExecutionStatus::Success,
                0xfd => CodeExecutionStatus::Failure(CodeExecutionError::Revert),
                _ => CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(0xfe)),
            }
        );
        assert_eq!(
            journal.transient_storage(created, slot_one()),
            word(if terminal == 0xf3 { 2 } else { 7 })
        );
        assert_eq!(journal.account(created).unwrap().exists, terminal == 0xf3);
        let settled = journal.settle_transaction().unwrap();
        assert!(settled.writes.ordinary_storage.is_empty());
        assert!(settled.writes.raw_storage.is_empty());
        assert_eq!(journal.transient_storage(created, slot_one()), ZERO);
        assert_eq!(
            journal.correction_stage(),
            CorrectionStage::TransientRollback
        );
    }
}

#[test]
fn reverted_create2_can_retry_without_retaining_the_first_initcode_write() {
    // Initcode returns the value seen before its write as REVERT data.
    let mut initcode = vec![0x60, 1, 0xb3, 0x60, 0, 0x52];
    initcode.extend(store(2, 0xb4));
    initcode.extend([0x60, 32, 0x60, 0, 0xfd]);
    let mut parent = vec![0x60, initcode.len() as u8, 0x60, 0, 0x60, 0, 0x39];
    for _ in 0..2 {
        parent.extend([
            0x60,
            0,
            0x60,
            initcode.len() as u8,
            0x60,
            0,
            0x60,
            0,
            0xf5,
            0x50,
        ]);
    }
    parent.extend([0x60, 32, 0x60, 0, 0x60, 0, 0x3e, 0x60, 32, 0x60, 0, 0xf3]);
    parent[3] = parent.len() as u8;
    parent.extend(&initcode);
    let mut preimage = vec![0xff];
    preimage.extend(PARENT);
    preimage.extend(ZERO);
    preimage.extend(keccak256(&initcode));
    let hash = keccak256(preimage);
    let created: [u8; 20] = hash[12..].try_into().unwrap();
    let mut reader = Reader::funded();
    reader.add_code(PARENT, parent);
    let mut journal =
        ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
    let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.output, ZERO);
    assert_eq!(journal.transient_storage(created, slot_one()), ZERO);
    assert!(!journal.account(created).unwrap().exists);
}

#[test]
fn both_cacti_aliases_share_one_transient_slot() {
    for (write, load) in [(0xb4, 0x5c), (0x5d, 0xb3)] {
        let mut code = store(3, write);
        code.extend(load_return(load));
        let mut reader = Reader::funded();
        reader.add_code(PARENT, code);
        let mut journal =
            ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
        assert_eq!(
            execute(&mut journal, &transaction(false), TaraxaPhase::Cacti).output,
            word(3)
        );
        journal.settle_transaction().unwrap();
        assert_eq!(journal.transient_storage(PARENT, slot_one()), ZERO);
    }
}

#[test]
fn next_transaction_reads_zero_after_success_or_failure_settlement() {
    for terminal in [0xf3, 0xfd, 0xfe] {
        let mut code = vec![0x60, 1, 0xb3, 0x60, 0, 0x52];
        code.extend(store(3, 0xb4));
        code.extend([0x60, 32, 0x60, 0, terminal]);
        let mut reader = Reader::funded();
        reader.add_code(PARENT, code);
        let mut journal =
            ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
        for nonce in [1, 2] {
            let mut tx = transaction(false);
            tx.nonce = FinalChainNonce::from_u64(nonce);
            let result = execute(&mut journal, &tx, TaraxaPhase::Cacti);
            assert_eq!(
                result.status,
                match terminal {
                    0xf3 => CodeExecutionStatus::Success,
                    0xfd => CodeExecutionStatus::Failure(CodeExecutionError::Revert),
                    _ => CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(0xfe)),
                }
            );
            assert_eq!(
                result.output,
                if terminal == 0xfe {
                    Vec::new()
                } else {
                    ZERO.to_vec()
                }
            );
            assert_eq!(
                journal.transient_storage(PARENT, slot_one()),
                if terminal == 0xf3 { word(3) } else { ZERO }
            );
            journal.settle_transaction().unwrap();
            assert_eq!(journal.transient_storage(PARENT, slot_one()), ZERO);
        }
    }
}

#[test]
fn direct_corrected_trace_uses_the_same_rollback_and_result_as_execution() {
    for terminal in [0xf3, 0xfd] {
        let mut code = store(3, 0xb4);
        code.extend(load_return(0xb3));
        *code.last_mut().unwrap() = terminal;
        let mut reader = Reader::funded();
        reader.add_code(PARENT, code);
        let mut plain = ExecutionJournal::new_with_correction_stage(
            reader.clone(),
            CorrectionStage::TransientRollback,
        );
        let mut traced =
            ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::TransientRollback);
        plain.set_transient_storage(PARENT, slot_one(), word(7));
        traced.set_transient_storage(PARENT, slot_one(), word(7));
        let expected = execute(&mut plain, &transaction(false), TaraxaPhase::Cacti);
        let mut observer = TraceCollector::default();
        let result = execute_top_level_call_with_trace(
            &mut traced,
            &NoExternal,
            &NoExternal,
            &block(),
            &transaction(false),
            RULES,
            profile(CorrectionStage::TransientRollback, TaraxaPhase::Cacti),
            &mut observer,
        )
        .unwrap();
        assert_eq!(result, TransactionExecutionResult::Executed(expected));
        assert!(!observer.events().is_empty());
        assert_eq!(
            traced.transient_storage(PARENT, slot_one()),
            word(if terminal == 0xf3 { 3 } else { 7 })
        );
        assert_eq!(
            plain.settle_transaction().unwrap(),
            traced.settle_transaction().unwrap()
        );
    }
}

fn selfdestruct_code(beneficiary: [u8; 20]) -> Vec<u8> {
    let mut code = vec![0x73];
    code.extend(beneficiary);
    code.push(0xff);
    code
}

fn selfdestruct_reader(parent: Vec<u8>, beneficiary: [u8; 20]) -> Reader {
    let mut reader = Reader::funded();
    reader.add_code(PARENT, parent);
    reader.add_code(CHILD, selfdestruct_code(beneficiary));
    reader.accounts.get_mut(&CHILD).unwrap().balance =
        ConcreteAccountBalance::new(BigUint::from(7_u8));
    if beneficiary != CHILD {
        let mut account = reader.accounts[&CHILD].clone();
        account.balance = ConcreteAccountBalance::default();
        account.code_hash = None;
        account.code_size = 0;
        reader.accounts.insert(beneficiary, account);
    }
    reader
}

#[test]
fn refund_stage_grants_one_refund_per_account_and_preserves_the_existing_cap() {
    let beneficiary = [0x77; 20];
    let mut parent = vec![0x5b; 30_000];
    parent.extend(call(CHILD, 0xf1));
    parent.extend(call(CHILD, 0xf1));
    parent.push(0);
    for stage in [
        CorrectionStage::Historical,
        CorrectionStage::TransientRollback,
        CorrectionStage::SingleSelfDestructRefund,
        CorrectionStage::CheckedReturnDataCopy,
        CorrectionStage::BoundedFalconAbi,
    ] {
        let mut journal = ExecutionJournal::new_with_correction_stage(
            selfdestruct_reader(parent.clone(), beneficiary),
            stage,
        );
        let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
        assert_eq!(result.status, CodeExecutionStatus::Success);
        // Intrinsic 21000 + 30000 JUMPDEST + 2*(7 PUSH*3 + CALL700 + POP2)
        // + one-word memory3 + 2*(PUSH20*3 + SELFDESTRUCT5000).
        let spent = 62_455;
        let refund = if matches!(
            stage,
            CorrectionStage::SingleSelfDestructRefund
                | CorrectionStage::CheckedReturnDataCopy
                | CorrectionStage::BoundedFalconAbi
        ) {
            24_000
        } else {
            48_000
        };
        assert_eq!(journal.refund(), refund);
        assert_eq!(result.refund_applied.as_u64(), refund.min(spent / 2));
        assert_eq!(result.gas_used.as_u64(), spent - refund.min(spent / 2));
        assert_eq!(
            journal.account(beneficiary).unwrap().balance.value(),
            &num_bigint::BigInt::from(7)
        );
        let settled = journal.settle_transaction().unwrap();
        assert!(
            settled
                .writes
                .accounts
                .iter()
                .any(|write| write.address == CHILD
                    && matches!(
                        write.operation,
                        rustaxa_evm::journal::JournalAccountOperation::Delete
                    ))
        );
        assert_eq!(journal.refund(), 0);
    }
}

#[test]
fn refund_stage_keeps_distinct_account_eligibility_and_self_beneficiary_rules() {
    for self_beneficiary in [false, true] {
        let beneficiary = if self_beneficiary { CHILD } else { [0x77; 20] };
        let mut parent = call(CHILD, 0xf1);
        parent.extend(call(if self_beneficiary { CHILD } else { SIBLING }, 0xf1));
        let mut reader = selfdestruct_reader(parent, beneficiary);
        if !self_beneficiary {
            reader.add_code(SIBLING, selfdestruct_code(beneficiary));
            reader.accounts.get_mut(&SIBLING).unwrap().balance =
                ConcreteAccountBalance::new(BigUint::from(7_u8));
        }
        let mut journal = ExecutionJournal::new_with_correction_stage(
            reader,
            CorrectionStage::SingleSelfDestructRefund,
        );
        assert_eq!(
            execute(&mut journal, &transaction(false), TaraxaPhase::Cacti).status,
            CodeExecutionStatus::Success
        );
        assert_eq!(
            journal.refund(),
            if self_beneficiary { 24_000 } else { 48_000 }
        );
        assert_eq!(
            journal.account(CHILD).unwrap().balance.value(),
            &num_bigint::BigInt::default()
        );
        if !self_beneficiary {
            assert_eq!(
                journal.account(beneficiary).unwrap().balance.value(),
                &num_bigint::BigInt::from(14)
            );
        }
    }
}

#[test]
fn refund_stage_restores_eligibility_balance_and_refund_after_parent_revert() {
    let beneficiary = [0x77; 20];
    let mut parent = call(SIBLING, 0xf1);
    parent.extend(call(CHILD, 0xf1));
    let mut middle = call(CHILD, 0xf1);
    middle.extend(store(3, 0xb4));
    middle.extend([0x60, 0, 0x60, 0, 0xfd]);
    let mut reader = selfdestruct_reader(parent, beneficiary);
    reader.add_code(SIBLING, middle);
    let mut journal = ExecutionJournal::new_with_correction_stage(
        reader,
        CorrectionStage::SingleSelfDestructRefund,
    );
    assert_eq!(
        execute(&mut journal, &transaction(false), TaraxaPhase::Cacti).status,
        CodeExecutionStatus::Success
    );
    assert_eq!(journal.refund(), 24_000);
    assert_eq!(journal.transient_storage(SIBLING, slot_one()), ZERO);
    assert_eq!(
        journal.account(beneficiary).unwrap().balance.value(),
        &num_bigint::BigInt::from(7)
    );
    assert_eq!(
        journal.account(CHILD).unwrap().balance.value(),
        &num_bigint::BigInt::default()
    );
}

#[test]
fn refund_stage_static_and_unfunded_destruction_leave_no_effects() {
    for opcode in [0xf1, 0xfa] {
        let beneficiary = [0x77; 20];
        let mut parent = call(CHILD, opcode);
        if opcode == 0xf1 {
            let length = parent.len();
            parent[length - 5..length - 2].copy_from_slice(&[0, 0x13, 0x8a]); // 5002 gas.
        }
        let mut reader = selfdestruct_reader(parent, beneficiary);
        reader.accounts.remove(&beneficiary);
        let mut journal = ExecutionJournal::new_with_correction_stage(
            reader,
            CorrectionStage::SingleSelfDestructRefund,
        );
        assert_eq!(
            execute(&mut journal, &transaction(false), TaraxaPhase::Cacti).status,
            CodeExecutionStatus::Success
        );
        assert_eq!(journal.refund(), 0);
        assert_eq!(
            journal.account(CHILD).unwrap().balance.value(),
            &num_bigint::BigInt::from(7)
        );
        assert!(!journal.account(beneficiary).unwrap().exists);
        assert!(
            !journal
                .settle_transaction()
                .unwrap()
                .writes
                .accounts
                .iter()
                .any(|write| write.address == beneficiary || write.address == CHILD)
        );
    }
}

#[test]
fn refund_stage_preserves_zero_and_signed_balance_rules() {
    for balance in [0, -7] {
        let beneficiary = [0x77; 20];
        let mut parent = call(CHILD, 0xf1);
        parent.extend(call(CHILD, 0xf1));
        if balance < 0 {
            parent.extend([0x60, 0, 0x60, 0, 0xfd]);
        }
        let mut journal = ExecutionJournal::new_with_correction_stage(
            selfdestruct_reader(parent, beneficiary),
            CorrectionStage::SingleSelfDestructRefund,
        );
        journal
            .set_balance(
                CHILD,
                rustaxa_evm::contracts::ExecutionBalance::new(num_bigint::BigInt::from(balance)),
            )
            .unwrap();
        let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
        if balance < 0 {
            assert_eq!(
                result.status,
                CodeExecutionStatus::Failure(CodeExecutionError::Revert)
            );
            assert_eq!(journal.refund(), 0);
            assert_eq!(
                journal.account(CHILD).unwrap().balance.value(),
                &num_bigint::BigInt::from(-7)
            );
        } else {
            assert_eq!(result.status, CodeExecutionStatus::Success);
            assert_eq!(journal.refund(), 24_000);
            assert_eq!(
                journal.account(CHILD).unwrap().balance.value(),
                &num_bigint::BigInt::default()
            );
        }
        assert_eq!(
            journal.account(beneficiary).unwrap().balance.value(),
            &num_bigint::BigInt::default()
        );
    }
}

fn integer_word(value: u64) -> [u8; 32] {
    let mut word = ZERO;
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

fn return_data_copy(destination: [u8; 32], source: [u8; 32], length: [u8; 32]) -> Vec<u8> {
    let mut code = Vec::new();
    for value in [length, source, destination] {
        code.push(0x7f);
        code.extend(value);
    }
    code.extend([0x3e, 0]);
    code
}

#[test]
fn corrected_copy_bounds_gas_order_and_fault_traces_follow_the_approved_model() {
    use CodeExecutionError as E;
    use revm::interpreter::InstructionResult as I;
    // Expected opcode costs: base3 + 3*ceil(length/32) + memory expansion.
    // Failed gas admission changes no gas; a later memory failure keeps copy cost.
    let cases = [
        (
            "wrap256",
            [255; 32],
            ZERO,
            integer_word(1),
            500_000,
            Some(E::GasUintOverflow),
            I::InvalidOperandOOG,
            0,
            0,
        ),
        (
            "above64",
            integer_word(u64::MAX),
            ZERO,
            integer_word(1),
            500_000,
            Some(E::GasUintOverflow),
            I::InvalidOperandOOG,
            0,
            0,
        ),
        (
            "round64",
            integer_word(u64::MAX - 1),
            ZERO,
            integer_word(1),
            500_000,
            Some(E::GasUintOverflow),
            I::InvalidOperandOOG,
            0,
            0,
        ),
        (
            "ceiling",
            integer_word(0x00ff_ffff_ffe0),
            ZERO,
            integer_word(1),
            500_000,
            Some(E::OutOfGas),
            I::OutOfGas,
            0,
            0,
        ),
        (
            "zero-wide-destination",
            [255; 32],
            ZERO,
            ZERO,
            500_000,
            None,
            I::Stop,
            3,
            0,
        ),
        (
            "zero-source-beyond-end",
            ZERO,
            integer_word(1),
            ZERO,
            500_000,
            Some(E::ReturnDataOutOfBounds),
            I::OutOfOffset,
            3,
            0,
        ),
        (
            "source256",
            ZERO,
            [255; 32],
            integer_word(1),
            500_000,
            Some(E::ReturnDataOutOfBounds),
            I::OutOfOffset,
            9,
            32,
        ),
        (
            "source64-overflow",
            ZERO,
            integer_word(u64::MAX),
            integer_word(1),
            500_000,
            Some(E::ReturnDataOutOfBounds),
            I::OutOfOffset,
            9,
            32,
        ),
        (
            "base-unfunded-before-source",
            ZERO,
            integer_word(1),
            ZERO,
            21_011,
            Some(E::OutOfGas),
            I::OutOfGas,
            0,
            0,
        ),
        (
            "memory-unfunded-before-source",
            ZERO,
            integer_word(1),
            integer_word(1),
            21_017,
            Some(E::OutOfGas),
            I::MemoryOOG,
            6,
            0,
        ),
        (
            "funded-before-source",
            ZERO,
            integer_word(1),
            integer_word(1),
            21_018,
            Some(E::ReturnDataOutOfBounds),
            I::OutOfOffset,
            9,
            32,
        ),
        (
            "two-memory-words",
            integer_word(31),
            ZERO,
            integer_word(2),
            500_000,
            Some(E::ReturnDataOutOfBounds),
            I::OutOfOffset,
            12,
            64,
        ),
    ];
    for (name, destination, source, length, gas, error, fault, cost, memory_len) in cases {
        let mut reader = Reader::funded();
        reader.add_code(PARENT, return_data_copy(destination, source, length));
        let mut plain = ExecutionJournal::new_with_correction_stage(
            reader.clone(),
            CorrectionStage::CheckedReturnDataCopy,
        );
        let mut traced = ExecutionJournal::new_with_correction_stage(
            reader,
            CorrectionStage::CheckedReturnDataCopy,
        );
        let mut tx = transaction(false);
        tx.gas_limit = FinalChainGas::new(gas);
        let result = execute(&mut plain, &tx, TaraxaPhase::Cacti);
        let mut observer = TraceCollector::default();
        let actual = execute_top_level_call_with_trace(
            &mut traced,
            &NoExternal,
            &NoExternal,
            &block(),
            &tx,
            RULES,
            profile(CorrectionStage::CheckedReturnDataCopy, TaraxaPhase::Cacti),
            &mut observer,
        )
        .unwrap();
        assert_eq!(
            actual,
            TransactionExecutionResult::Executed(result.clone()),
            "{name}"
        );
        assert_eq!(
            result.status,
            error
                .clone()
                .map_or(CodeExecutionStatus::Success, CodeExecutionStatus::Failure),
            "{name}"
        );
        assert_eq!(
            result.gas_used.as_u64(),
            if error.is_some() {
                gas
            } else {
                21_000 + 9 + cost
            },
            "{name}"
        );
        assert!(result.output.is_empty());
        let rows: Vec<_> = observer
            .events()
            .iter()
            .filter_map(|event| match event {
                TraceEvent::Opcode(row) if row.opcode == 0x3e => Some(row),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), if error.is_some() { 2 } else { 1 }, "{name}");
        assert_eq!(rows[0].phase, TraceOpcodePhase::BeforeExecution);
        assert_eq!(rows[0].gas, gas - 21_009);
        assert_eq!(rows[0].gas_cost, cost, "{name}");
        assert_eq!(rows[0].memory, vec![0; memory_len]);
        if error.is_some() {
            assert_eq!(rows[1].phase, TraceOpcodePhase::Fault(fault));
            assert_eq!(rows[1].gas_cost, cost);
            assert!(rows[1].stack.is_empty());
            assert_eq!(rows[1].memory, vec![0; memory_len]);
        }
        assert_eq!(
            plain.settle_transaction().unwrap(),
            traced.settle_transaction().unwrap()
        );
    }
}

#[test]
fn corrected_copy_stack_underflow_has_a_defined_trace_before_gas_checks() {
    let mut reader = Reader::funded();
    reader.add_code(PARENT, vec![0x3e]);
    let mut journal =
        ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::CheckedReturnDataCopy);
    let mut tx = transaction(false);
    tx.gas_limit = FinalChainGas::new(21_000);
    let mut observer = TraceCollector::default();
    let result = execute_top_level_call_with_trace(
        &mut journal,
        &NoExternal,
        &NoExternal,
        &block(),
        &tx,
        RULES,
        profile(CorrectionStage::CheckedReturnDataCopy, TaraxaPhase::Cacti),
        &mut observer,
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("admission")
    };
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::StackUnderflow)
    );
    assert_eq!(result.gas_used.as_u64(), 21_000);
    let rows: Vec<_> = observer
        .events()
        .iter()
        .filter_map(|event| {
            if let TraceEvent::Opcode(row) = event {
                Some(row)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1].phase,
        TraceOpcodePhase::Fault(revm::interpreter::InstructionResult::StackUnderflow)
    );
    assert_eq!(rows[1].gas_cost, 0);
    assert!(rows[1].memory.is_empty());
}

#[test]
fn corrected_copy_nested_failure_rolls_back_and_parent_continues() {
    let mut parent = store(1, 0xb4);
    parent.extend(call(CHILD, 0xf4));
    parent.extend(load_return(0xb3));
    let mut child = store(3, 0xb4);
    child.extend([0x60, 9, 0x60, 1, 0x55]);
    child.extend(return_data_copy([255; 32], ZERO, integer_word(1)));
    let mut reader = Reader::funded();
    reader.add_code(PARENT, parent);
    reader.add_code(CHILD, child);
    let mut journal =
        ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::CheckedReturnDataCopy);
    let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.output, word(1));
    assert_eq!(journal.transient_storage(PARENT, slot_one()), word(1));
    assert_eq!(
        journal.ordinary_storage(PARENT, slot_one()).unwrap().1,
        BigUint::default()
    );
}

#[test]
fn corrected_copy_top_level_failure_settles_fees_and_allows_the_next_transaction() {
    let mut reader = Reader::funded();
    reader.add_code(PARENT, return_data_copy([255; 32], ZERO, integer_word(1)));
    let mut journal =
        ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::CheckedReturnDataCopy);
    let result = execute(&mut journal, &transaction(false), TaraxaPhase::Cacti);
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::GasUintOverflow)
    );
    assert_eq!(
        journal.account(SENDER).unwrap().nonce,
        FinalChainNonce::from_u64(2)
    );
    assert_eq!(
        journal.account(SENDER).unwrap().balance.value(),
        &num_bigint::BigInt::from(9_500_000)
    );
    let mut tx = transaction(false);
    tx.receiver = Some([0x88; 20]);
    tx.nonce = FinalChainNonce::from_u64(2);
    let next = execute(&mut journal, &tx, TaraxaPhase::Cacti);
    assert_eq!(next.status, CodeExecutionStatus::Success);
    assert_eq!(next.gas_used.as_u64(), 21_000);
    assert_eq!(
        journal.account(SENDER).unwrap().nonce,
        FinalChainNonce::from_u64(3)
    );
    assert_eq!(
        journal.account(SENDER).unwrap().balance.value(),
        &num_bigint::BigInt::from(9_479_000)
    );
}

struct IdentityPrecompile;
impl NativeAddressClassifier for IdentityPrecompile {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address[..19] == [0; 19] && address[19] == 4
    }
}

#[test]
fn corrected_copy_valid_range_returns_independently_seeded_bytes() {
    let mut identity = [0; 20];
    identity[19] = 4;
    let mut code = vec![0x60, 0x2a, 0x60, 0, 0x52];
    let mut invoke = call(identity, 0xf1);
    invoke[5] = 32;
    code.extend(invoke);
    code.extend(return_data_copy(integer_word(32), ZERO, integer_word(32)));
    code.pop(); // Continue after the copy instead of STOP.
    code.extend([0x60, 32, 0x60, 32, 0xf3]);
    for phase in [
        TaraxaPhase::Californicum,
        TaraxaPhase::Ficus,
        TaraxaPhase::Cacti,
    ] {
        let mut reader = Reader::funded();
        reader.add_code(PARENT, code.clone());
        let mut journal = ExecutionJournal::new_with_correction_stage(
            reader,
            CorrectionStage::CheckedReturnDataCopy,
        );
        let mut sequence = PeriodConsensusSequence::new(block().period);
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoExternal,
            &IdentityPrecompile,
            &NoExternal,
            &mut UnusedPort,
            &mut sequence,
            &block(),
            &transaction(false),
            RULES,
            profile(CorrectionStage::CheckedReturnDataCopy, phase),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("admission")
        };
        assert_eq!(result.status, CodeExecutionStatus::Success);
        assert_eq!(result.output, word(0x2a));
        assert_eq!(sequence.next_sequence(), 0);
    }
}

fn falcon_rows() -> Vec<serde_json::Value> {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/falcon/local.json"
    ))
    .unwrap();
    corpus["falcon"].as_array().unwrap().clone()
}

fn falcon_invocation(input: Vec<u8>, supplied: u64, kind: NativeCallKind) -> StatelessInvocation {
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 0_u32.into(),
            ordinal: 0,
        },
        period: block().period,
        depth: 0,
        kind,
        is_static: kind == NativeCallKind::StaticCall,
        caller: PARENT,
        contract: FALCON_VERIFY_ADDRESS,
        state_address: FALCON_VERIFY_ADDRESS,
        value: ExecutionValue::new(BigUint::from(7_u8)),
        input,
        supplied_gas: FinalChainGas::new(supplied),
    }
}

fn bounded_falcon(input: Vec<u8>) -> Vec<u8> {
    let quote = 1_465 + 6 * input.len().div_ceil(32) as u64;
    let result =
        PreparedFalconCall::prepare_bounded(falcon_invocation(input, quote, NativeCallKind::Call))
            .unwrap()
            .invoke()
            .unwrap();
    let NativeInvocationResult::Completed(outcome) = result else {
        panic!("funding")
    };
    assert_eq!(outcome.status, NativeStatus::Success);
    assert_eq!(outcome.gas_used.as_u64(), quote);
    assert!(outcome.account_mutations.is_empty());
    assert!(outcome.raw_mutations.is_empty());
    assert!(outcome.logs.is_empty());
    outcome.output
}

#[test]
fn bounded_falcon_uses_separate_corrected_expectations_and_the_same_quote() {
    let rows = falcon_rows();
    assert_eq!(rows.len(), 41);
    // Only these complete, bounded inputs contain retained valid crypto vectors.
    // The expected set follows the approved range policy, not the Go result field.
    let valid = [
        "historical-valid",
        "historical-valid-long-message",
        "reordered-fields",
        "unaligned-fields",
        "trailing-bytes",
    ];
    for row in rows {
        let name = row["name"].as_str().unwrap();
        let input = hex::decode(row["input"].as_str().unwrap()).unwrap();
        let quote = 1_465 + 6 * input.len().div_ceil(32) as u64;
        for kind in [
            NativeCallKind::Call,
            NativeCallKind::CallCode,
            NativeCallKind::DelegateCall,
            NativeCallKind::StaticCall,
        ] {
            for supplied in [quote - 1, quote, quote + 1] {
                let request = falcon_invocation(input.clone(), supplied, kind);
                let prepared = PreparedFalconCall::prepare_bounded(request.clone()).unwrap();
                assert_eq!(prepared.invocation(), &request);
                let gas_quote = prepared.quote();
                assert_eq!(gas_quote.required_gas.as_u64(), quote);
                let result = prepared.invoke().unwrap();
                result.validate(&request, gas_quote).unwrap();
                if supplied < quote {
                    assert_eq!(
                        result,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: quote.into()
                        }
                    );
                    continue;
                }
                let NativeInvocationResult::Completed(outcome) = result else {
                    panic!("funded")
                };
                assert_eq!(outcome.gas_used.as_u64(), quote);
                if input.len() < 4 || input[..4] != [0xde, 0x8f, 0x50, 0xa1] {
                    let NativeStatus::ContractFailure(failure) = outcome.status else {
                        panic!("selector")
                    };
                    assert_eq!(
                        failure.error,
                        if input.len() < 4 {
                            "invalid input format"
                        } else {
                            "invalid method signature"
                        }
                    );
                    assert!(outcome.output.is_empty());
                } else {
                    assert_eq!(outcome.status, NativeStatus::Success, "{name}");
                    assert_eq!(
                        outcome.output,
                        word(u8::from(!valid.contains(&name))),
                        "{name}"
                    );
                }
                assert!(outcome.account_mutations.is_empty());
                assert!(outcome.raw_mutations.is_empty());
                assert!(outcome.logs.is_empty());
            }
        }
    }
}

fn abi_word_index(input: &[u8], index: usize) -> usize {
    u64::from_be_bytes(
        input[4 + index * 32 + 24..4 + (index + 1) * 32]
            .try_into()
            .unwrap(),
    ) as usize
}

#[test]
fn bounded_falcon_checks_each_full_width_field_without_declared_length_allocation() {
    let row = falcon_rows()
        .into_iter()
        .find(|row| row["name"] == "historical-valid")
        .unwrap();
    let valid = hex::decode(row["input"].as_str().unwrap()).unwrap();
    assert_eq!(bounded_falcon(valid.clone()), ZERO);
    for index in 0..3 {
        let offset = abi_word_index(&valid, index);
        for target in [4 + 32 * index, 4 + offset] {
            for length in [
                [255; 32],
                integer_word(i64::MAX as u64),
                integer_word(1_u64 << 63),
                ZERO,
            ] {
                let mut input = valid.clone();
                input[target..target + 32].copy_from_slice(&length);
                assert_eq!(bounded_falcon(input), word(1));
            }
            let mut input = valid.clone();
            input[target] = 1;
            assert_eq!(bounded_falcon(input), word(1));
        }
        let length = u64::from_be_bytes(valid[4 + offset + 24..4 + offset + 32].try_into().unwrap())
            as usize;
        assert_eq!(
            bounded_falcon(valid[..4 + offset + 32 + length - 1].to_vec()),
            word(1)
        );
    }
    let offset = abi_word_index(&valid, 2);
    let length =
        u64::from_be_bytes(valid[4 + offset + 24..4 + offset + 32].try_into().unwrap()) as usize;
    assert_eq!(
        bounded_falcon(valid[..4 + offset + 32 + length].to_vec()),
        ZERO
    );
    let mut request = falcon_invocation(valid, 100_000, NativeCallKind::Call);
    request.contract[0] = 1;
    assert!(matches!(
        PreparedFalconCall::prepare_bounded(request),
        Err(NativePortError::Domain(_))
    ));
}

struct FalconPrecompile;
impl NativeAddressClassifier for FalconPrecompile {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address == FALCON_VERIFY_ADDRESS
    }
}

fn falcon_frame(input: &[u8], opcode: u8, supplied: u32, revert: bool) -> Vec<u8> {
    let mut code = vec![0x61];
    code.extend((input.len() as u16).to_be_bytes());
    code.extend([0x61, 0, 0, 0x60, 0, 0x39]);
    code.extend([0x60, 32, 0x60, 0, 0x61]);
    code.extend((input.len() as u16).to_be_bytes());
    code.extend([0x60, 0]);
    if opcode == 0xf1 || opcode == 0xf2 {
        code.extend([0x60, 7]);
    }
    code.push(0x73);
    code.extend(FALCON_VERIFY_ADDRESS);
    // CALL and CALLCODE add the existing 2300 value stipend to the operand.
    let operand = supplied
        - if opcode == 0xf1 || opcode == 0xf2 {
            2_300
        } else {
            0
        };
    code.push(0x62);
    code.extend(&operand.to_be_bytes()[1..]);
    code.extend([
        opcode,
        0x50,
        0x60,
        32,
        0x60,
        0,
        if revert { 0xfd } else { 0xf3 },
    ]);
    let offset = code.len() as u16;
    code[4..6].copy_from_slice(&offset.to_be_bytes());
    code.extend(input);
    code
}

#[test]
fn bounded_falcon_frames_keep_value_for_invalid_success_and_revert_it_on_parent_failure() {
    // The wrapped field must become a normal invalid result through the driver.
    // Trailing bytes raise the quote above the existing value stipend.
    let row = falcon_rows()
        .into_iter()
        .find(|row| row["name"] == "wrapped-message-length-panic")
        .unwrap();
    let mut input = hex::decode(row["input"].as_str().unwrap()).unwrap();
    input.resize(6_144, 0);
    let quote = 2_617;
    for opcode in [0xf1, 0xf2, 0xf4, 0xfa] {
        for revert in [false, true] {
            for supplied in [quote - 1, quote, quote + 1] {
                let mut reader = Reader::funded();
                reader.add_code(PARENT, falcon_frame(&input, opcode, supplied, revert));
                reader.accounts.get_mut(&PARENT).unwrap().balance =
                    ConcreteAccountBalance::new(BigUint::from(10_u8));
                let mut journal = ExecutionJournal::new_with_correction_stage(
                    reader,
                    CorrectionStage::BoundedFalconAbi,
                );
                let mut sequence = PeriodConsensusSequence::new(block().period);
                let result = execute_top_level_call_with_native(
                    &mut journal,
                    &NoExternal,
                    &FalconPrecompile,
                    &NoExternal,
                    &mut UnusedPort,
                    &mut sequence,
                    &block(),
                    &transaction(false),
                    RULES,
                    profile(CorrectionStage::BoundedFalconAbi, TaraxaPhase::Cacti),
                )
                .unwrap();
                let TransactionExecutionResult::Executed(result) = result else {
                    panic!("admission")
                };
                assert_eq!(
                    result.status,
                    if revert {
                        CodeExecutionStatus::Failure(CodeExecutionError::Revert)
                    } else {
                        CodeExecutionStatus::Success
                    }
                );
                let funded = supplied >= quote;
                let mut expected = ZERO;
                if funded {
                    expected = word(1);
                } else {
                    expected.copy_from_slice(&input[..32]);
                }
                assert_eq!(result.output, expected);
                let transfer = if opcode == 0xf1 && funded && !revert {
                    7
                } else {
                    0
                };
                assert_eq!(
                    journal
                        .account(FALCON_VERIFY_ADDRESS)
                        .unwrap()
                        .balance
                        .value(),
                    &num_bigint::BigInt::from(transfer)
                );
                assert_eq!(
                    journal.account(PARENT).unwrap().balance.value(),
                    &num_bigint::BigInt::from(10 - transfer)
                );
                assert_eq!(sequence.next_sequence(), 0);
                assert!(result.logs.is_empty());
                assert!(journal.logs().is_empty());
            }
        }
    }
}

#[test]
fn bounded_falcon_preserves_phase_selection_and_prior_corrected_arithmetic() {
    for phase in [TaraxaPhase::Californicum, TaraxaPhase::Ficus] {
        let mut journal = ExecutionJournal::new_with_correction_stage(
            Reader::funded(),
            CorrectionStage::BoundedFalconAbi,
        );
        let mut tx = transaction(false);
        tx.receiver = Some(FALCON_VERIFY_ADDRESS);
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoExternal,
            &FalconPrecompile,
            &NoExternal,
            &mut UnusedPort,
            &mut PeriodConsensusSequence::new(block().period),
            &block(),
            &tx,
            RULES,
            profile(CorrectionStage::BoundedFalconAbi, phase),
        );
        assert!(
            matches!(result,Err(ExecutionDriverError::NativeCallUnavailable {address}) if address==FALCON_VERIFY_ADDRESS)
        );
    }
    let mut reader = Reader::funded();
    reader.add_code(PARENT, return_data_copy([255; 32], ZERO, integer_word(1)));
    let mut journal =
        ExecutionJournal::new_with_correction_stage(reader, CorrectionStage::BoundedFalconAbi);
    assert_eq!(
        execute(&mut journal, &transaction(false), TaraxaPhase::Cacti).status,
        CodeExecutionStatus::Failure(CodeExecutionError::GasUintOverflow)
    );
}
