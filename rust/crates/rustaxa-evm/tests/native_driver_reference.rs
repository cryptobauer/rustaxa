//! Driver/native boundary checks for explicit opt-in consensus-native routing.
//!
//! A scripted consensus port isolates frame bookkeeping. Pinned Go fixtures
//! additionally cover stateless primitive/frame integration and RETURNDATACOPY
//! ordering. These tests do not register any production route.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, NativeContractFailure, NativeExecutionPort, NativeGasQuote,
        NativeInvocation, NativeInvocationResult, NativeJournalRead, NativeOutcome,
        NativePortError, NativeStatus, TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, PeriodConsensusSequence,
        execute_top_level_call_with_native, execute_top_level_create_with_native,
    },
    envelope::EnvelopeRules,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
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
const NATIVE: [u8; 20] = [0xcc; 20];

struct Reader {
    accounts: BTreeMap<[u8; 20], ConcreteAccount>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    code_reads: Arc<AtomicUsize>,
}

impl ConcreteStateRead for Reader {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(7),
            state_root: [0x44; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
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
        _address: [u8; 20],
        _key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.code_reads.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}

struct NoHistory;

impl BlockHashRead for NoHistory {
    fn block_hash(&self, _number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("test bytecode does not execute BLOCKHASH")
    }
}

struct AddressSet(Vec<[u8; 20]>);

impl NativeAddressClassifier for AddressSet {
    fn is_native_address(&self, _period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.0.contains(&address)
    }
}

struct ScriptedPort {
    required_gas: FinalChainGas,
    result: NativeInvocationResult,
    invocations: Mutex<Vec<NativeInvocation>>,
}

impl ScriptedPort {
    fn completed(status: NativeStatus, output: Vec<u8>) -> Self {
        Self {
            required_gas: FinalChainGas::new(20),
            result: NativeInvocationResult::Completed(NativeOutcome {
                status,
                gas_used: FinalChainGas::new(20),
                output,
                account_mutations: Vec::new(),
                raw_mutations: Vec::new(),
                logs: Vec::new(),
                diagnostic: None,
            }),
            invocations: Mutex::new(Vec::new()),
        }
    }

    fn insufficient(required_gas: u64) -> Self {
        Self {
            required_gas: FinalChainGas::new(required_gas),
            result: NativeInvocationResult::InsufficientGas {
                required_gas: FinalChainGas::new(required_gas),
            },
            invocations: Mutex::new(Vec::new()),
        }
    }
}

impl NativeExecutionPort for ScriptedPort {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        _journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        Ok(NativeGasQuote {
            invocation: invocation.id,
            required_gas: self.required_gas,
        })
    }

    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        _quote: NativeGasQuote,
        _journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.invocations.lock().unwrap().push(invocation.clone());
        Ok(self.result.clone())
    }
}

#[test]
fn native_failure_keeps_returndata_without_copying_output_memory() {
    let parent = parent_call_then_return_memory(NATIVE);
    let mut journal = journal_with_parent(parent);
    let mut port = ScriptedPort::completed(
        NativeStatus::ContractFailure(NativeContractFailure {
            error: "execution reverted".into(),
        }),
        hex::decode("deadbeef").unwrap(),
    );
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute parent")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    let failure_gas_used = result.gas_used;
    assert_eq!(
        result.output,
        hex::decode("eeeeeeee deadbeef".replace(' ', "")).unwrap()
    );
    assert_eq!(sequence.next_sequence(), 1);
    let invocations = port.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].id.sequence, 0);
    assert_eq!(invocations[0].depth, 1);
    assert_eq!(invocations[0].contract, NATIVE);
    assert_eq!(invocations[0].state_address, NATIVE);
    drop(invocations);

    let mut journal = journal_with_parent(parent_call_then_return_memory(NATIVE));
    let mut port = ScriptedPort::completed(NativeStatus::Success, hex::decode("deadbeef").unwrap());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute parent")
    };
    assert_eq!(
        result.output,
        hex::decode("deadeeee deadbeef".replace(' ', "")).unwrap()
    );
    assert_eq!(result.gas_used, failure_gas_used);
}

#[test]
fn consensus_classifier_must_be_a_subset_of_all_native_addresses() {
    let mut journal = journal_with_parent(parent_call_then_stop(NATIVE, 0, false));
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    assert_eq!(
        execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(Vec::new()),
            &AddressSet(vec![NATIVE]),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(PARENT, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::NativeClassifierMismatch { address: NATIVE })
    );
    assert_eq!(sequence.next_sequence(), 0);
    assert!(port.invocations.lock().unwrap().is_empty());
}

#[test]
fn sequence_survives_outer_revert_but_not_funds_rejection() {
    let mut journal = journal_with_parent(parent_call_then_stop(NATIVE, 0, true));
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Failure(CodeExecutionError::Revert)
    ));
    assert_eq!(sequence.next_sequence(), 1);

    let mut journal = journal_with_parent(parent_call_then_stop(NATIVE, 1, false));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    assert_eq!(sequence.next_sequence(), 1);
    assert_eq!(port.invocations.lock().unwrap().len(), 1);
}

#[test]
fn depth_rejection_does_not_allocate_a_native_sequence() {
    let addresses: Vec<[u8; 20]> = (0..1_025).map(chain_address).collect();
    let code_reads = Arc::new(AtomicUsize::new(0));
    let mut reader = Reader {
        accounts: BTreeMap::from([(
            SENDER,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        )]),
        codes: BTreeMap::new(),
        code_reads: Arc::clone(&code_reads),
    };
    for (index, address) in addresses.iter().enumerate() {
        let callee = addresses.get(index + 1).copied().unwrap_or(NATIVE);
        let mut code = hex::decode("6000600060006000600073").unwrap();
        code.extend_from_slice(&callee);
        code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
        let hash = keccak256(&code).0;
        reader.accounts.insert(
            *address,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: Some(hash),
                code_size: code.len() as u64,
            },
        );
        reader.codes.insert(hash, code);
    }
    let mut journal = ExecutionJournal::new(reader);
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let mut transaction = transaction(addresses[0], ExecutionValue::default());
    transaction.gas_limit = FinalChainGas::new(u64::MAX);
    transaction.gas_price = ExecutionGasPrice::new(BigUint::default());
    let mut block = block();
    block.gas_limit = FinalChainGas::new(u64::MAX);
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block,
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    assert_eq!(sequence.next_sequence(), 0);
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(code_reads.load(Ordering::Relaxed), 1_025);
}

#[test]
fn direct_native_failure_settles_exact_output_gas_and_depth_zero() {
    let mut journal = journal_without_parent();
    let failure = NativeContractFailure {
        error: "native failure".into(),
    };
    let mut port = ScriptedPort::completed(
        NativeStatus::ContractFailure(failure.clone()),
        vec![0x44, 0x55],
    );
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(NATIVE, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute native")
    };
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::Native(failure))
    );
    assert_eq!(result.output, vec![0x44, 0x55]);
    assert_eq!(result.gas_used, FinalChainGas::new(21_020));
    assert_eq!(port.invocations.lock().unwrap()[0].depth, 0);
    assert_eq!(sequence.next_sequence(), 1);
}

#[test]
fn native_quote_out_of_gas_consumes_sequence_and_returns_all_supplied_gas() {
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::insufficient(80_000);
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(NATIVE, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute native gas admission")
    };
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
    );
    assert_eq!(result.gas_used, FinalChainGas::new(21_000));
    assert!(result.output.is_empty());
    assert_eq!(sequence.next_sequence(), 1);
    assert_eq!(port.invocations.lock().unwrap().len(), 1);
}

#[test]
fn native_delegatecall_keeps_code_state_caller_and_full_value_distinct() {
    let parent = parent_delegate_then_stop(NATIVE);
    let value = (BigUint::from(1_u8) << 264) + BigUint::from(9_u8);
    let mut journal = journal_with_parent_balance(parent, &value + BigUint::from(1_000_000_u64));
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let value = ExecutionValue::new(value);
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, value.clone()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    let invocations = port.invocations.lock().unwrap();
    assert_eq!(invocations.len(), 1);
    assert_eq!(
        invocations[0].kind,
        rustaxa_evm::contracts::NativeCallKind::DelegateCall
    );
    assert_eq!(invocations[0].caller, SENDER);
    assert_eq!(invocations[0].contract, NATIVE);
    assert_eq!(invocations[0].state_address, PARENT);
    assert_eq!(invocations[0].value, value);
    assert_eq!(invocations[0].depth, 1);
}

#[test]
fn unsupported_native_subset_and_wrong_period_fail_without_allocation() {
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    assert_eq!(
        execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![NATIVE]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(NATIVE, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::NativeCallUnavailable { address: NATIVE })
    );
    assert_eq!(sequence.next_sequence(), 0);

    let mut wrong_period = PeriodConsensusSequence::new(FinalChainBlockNumber::new(6));
    assert_eq!(
        execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![NATIVE]),
            &AddressSet(vec![NATIVE]),
            &mut port,
            &mut wrong_period,
            &block(),
            &transaction(NATIVE, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::NativeSequencePeriod {
            expected: FinalChainBlockNumber::new(6),
            observed: FinalChainBlockNumber::new(7),
        })
    );
    assert_eq!(wrong_period.next_sequence(), 0);
    assert!(port.invocations.lock().unwrap().is_empty());
}

#[test]
fn create_initcode_can_use_the_explicit_native_port() {
    let initcode = parent_call_then_stop(NATIVE, 0, false);
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(FinalChainBlockNumber::new(7));
    let transaction = ExecutionTransaction {
        position: FinalChainTransactionPosition::from(4_u32),
        hash: [0x22; 32],
        sender: SENDER,
        receiver: None,
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: initcode,
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Create,
    };
    let result = execute_top_level_create_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![NATIVE]),
        &AddressSet(vec![NATIVE]),
        &mut port,
        &mut sequence,
        &block(),
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    assert_eq!(sequence.next_sequence(), 1);
    let invocations = port.invocations.lock().unwrap();
    assert_eq!(invocations[0].id.transaction, transaction.position);
    assert_eq!(invocations[0].depth, 1);
}

fn journal_without_parent() -> ExecutionJournal<Reader> {
    ExecutionJournal::new(Reader {
        accounts: BTreeMap::from([(
            SENDER,
            ConcreteAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        )]),
        codes: BTreeMap::new(),
        code_reads: Arc::new(AtomicUsize::new(0)),
    })
}

fn journal_with_parent(code: Vec<u8>) -> ExecutionJournal<Reader> {
    journal_with_parent_balance(code, BigUint::from(1_000_000_u64))
}

fn journal_with_parent_balance(code: Vec<u8>, sender_balance: BigUint) -> ExecutionJournal<Reader> {
    let hash = keccak256(&code).0;
    let reader = Reader {
        accounts: BTreeMap::from([
            (
                SENDER,
                ConcreteAccount {
                    nonce: FinalChainNonce::from_u64(1),
                    balance: ConcreteAccountBalance::new(sender_balance),
                    storage_root: None,
                    code_hash: None,
                    code_size: 0,
                },
            ),
            (
                PARENT,
                ConcreteAccount {
                    nonce: FinalChainNonce::from_u64(1),
                    balance: ConcreteAccountBalance::default(),
                    storage_root: None,
                    code_hash: Some(hash),
                    code_size: code.len() as u64,
                },
            ),
        ]),
        codes: BTreeMap::from([(hash, code)]),
        code_reads: Arc::new(AtomicUsize::new(0)),
    };
    ExecutionJournal::new(reader)
}

fn parent_call_then_return_memory(target: [u8; 20]) -> Vec<u8> {
    let mut code =
        hex::decode("60ee60005360ee60015360ee60025360ee60035360026000600060006000").unwrap();
    code.push(0x73);
    code.extend_from_slice(&target);
    code.extend_from_slice(&[
        0x61, 0x03, 0xe8, 0xf1, 0x50, 0x3d, 0x60, 0x00, 0x60, 0x04, 0x3e, 0x60, 0x08, 0x60, 0x00,
        0xf3,
    ]);
    code
}

fn parent_call_then_stop(target: [u8; 20], value: u8, revert: bool) -> Vec<u8> {
    let mut code = hex::decode("6000600060006000").unwrap();
    code.extend_from_slice(&[0x60, value, 0x73]);
    code.extend_from_slice(&target);
    code.extend_from_slice(&[0x61, 0x03, 0xe8, 0xf1, 0x50]);
    if revert {
        code.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xfd]);
    } else {
        code.push(0x00);
    }
    code
}

fn parent_delegate_then_stop(target: [u8; 20]) -> Vec<u8> {
    let mut code = hex::decode("6000600060006000").unwrap();
    code.push(0x73);
    code.extend_from_slice(&target);
    code.extend_from_slice(&[0x61, 0x03, 0xe8, 0xf4, 0x50, 0x00]);
    code
}

fn chain_address(index: usize) -> [u8; 20] {
    let mut address = [0xdd; 20];
    address[12..].copy_from_slice(&(index as u64).to_be_bytes());
    address
}

fn transaction(receiver: [u8; 20], value: ExecutionValue) -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(3_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(receiver),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value,
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(7),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn primitive_address(number: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = number;
    address
}

#[test]
fn bls_frame_dispatch_preserves_both_registries_outputs_and_errors() {
    use rustaxa_evm::profile::TaraxaPhase;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/bls/public.json"
    ))
    .unwrap();
    for row in fixture["bls"].as_array().unwrap() {
        let target = primitive_address(row["address"].as_u64().unwrap().try_into().unwrap());
        let phase = match row["registry"].as_str().unwrap() {
            "ficus" => TaraxaPhase::Ficus,
            "cacti" => TaraxaPhase::Cacti,
            _ => unreachable!(),
        };
        let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
        tx.gas_price = ExecutionGasPrice::new(BigUint::default());
        tx.input = if row["repeat"].is_object() {
            hex::decode(row["repeat"]["element"].as_str().unwrap())
                .unwrap()
                .repeat(row["repeat"]["count"].as_u64().unwrap().try_into().unwrap())
        } else {
            hex::decode(row["input"].as_str().unwrap()).unwrap()
        };
        let intrinsic = rustaxa_types::transaction::intrinsic_gas(&tx.input, false).unwrap();
        let quote = row["required_gas"].as_u64().unwrap();
        tx.gas_limit = (intrinsic + quote + 100).into();
        let mut context = block();
        context.gas_limit = 10_000_000_u64.into();
        let mut journal = journal_without_parent();
        let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
        let mut sequence = PeriodConsensusSequence::new(context.period);
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &context,
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(phase),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!(
                "BLS admission: {} {} {result:?}",
                row["registry"], row["name"]
            )
        };
        let error = row["error"].as_str().unwrap();
        assert_eq!(
            result.status,
            if error.is_empty() {
                CodeExecutionStatus::Success
            } else {
                CodeExecutionStatus::Failure(CodeExecutionError::Native(NativeContractFailure {
                    error: error.into(),
                }))
            },
            "{} {}",
            row["registry"],
            row["name"]
        );
        assert_eq!(
            result.output,
            hex::decode(row["output"].as_str().unwrap()).unwrap()
        );
        assert_eq!(result.gas_used.as_u64(), intrinsic + quote);
        if error.is_empty() {
            assert_eq!(
                journal.account(target).unwrap().balance.value(),
                &num_bigint::BigInt::from(7)
            );
        } else {
            assert!(!journal.account(target).unwrap().exists);
        }
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
    }
}

#[test]
fn cacti_p256_frame_preserves_quotes_value_and_consensus_sequence() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/p256/public.json"
    ))
    .unwrap();
    let target = rustaxa_evm::p256::P256_VERIFY_ADDRESS;
    for row in fixture["p256"].as_array().unwrap() {
        for action_gas in [6_899_u64, 6_900, 6_901] {
            let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
            tx.input = hex::decode(row["input"].as_str().unwrap()).unwrap();
            let intrinsic = rustaxa_types::transaction::intrinsic_gas(&tx.input, false).unwrap();
            tx.gas_limit = (intrinsic + action_gas).into();
            let mut journal = journal_without_parent();
            let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
            let mut sequence = PeriodConsensusSequence::new(7_u64.into());
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &AddressSet(vec![target]),
                &AddressSet(Vec::new()),
                &mut port,
                &mut sequence,
                &block(),
                &tx,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(true),
            )
            .unwrap();
            let TransactionExecutionResult::Executed(result) = result else {
                panic!("admitted frame")
            };
            if action_gas < 6_900 {
                assert_eq!(
                    result.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                );
                assert_eq!(result.gas_used.as_u64(), intrinsic);
                assert!(!journal.account(target).unwrap().exists);
            } else {
                assert_eq!(result.status, CodeExecutionStatus::Success);
                assert_eq!(result.gas_used.as_u64(), intrinsic + 6_900);
                assert_eq!(
                    result.output,
                    hex::decode(row["output"].as_str().unwrap()).unwrap()
                );
                assert_eq!(
                    journal.account(target).unwrap().balance.value(),
                    &num_bigint::BigInt::from(7)
                );
            }
            assert!(port.invocations.lock().unwrap().is_empty());
            assert_eq!(sequence.next_sequence(), 0);
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

#[test]
fn p256_routing_requires_cacti_and_rejects_consensus_overlap() {
    let target = rustaxa_evm::p256::P256_VERIFY_ADDRESS;
    for (cacti, all, consensus, expected) in [
        (
            false,
            vec![target],
            vec![],
            Some(ExecutionDriverError::NativeCallUnavailable { address: target }),
        ),
        (
            true,
            vec![target],
            vec![target],
            Some(ExecutionDriverError::NativeClassifierOverlap { address: target }),
        ),
        // Before activation an unregistered empty address is an ordinary call.
        (false, vec![], vec![], None),
    ] {
        let mut journal = journal_without_parent();
        let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
        let mut sequence = PeriodConsensusSequence::new(7_u64.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(all),
            &AddressSet(consensus),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(target, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(cacti),
        );
        if let Some(expected) = expected {
            assert_eq!(result, Err(expected));
        } else {
            let TransactionExecutionResult::Executed(result) = result.unwrap() else {
                panic!("ordinary call")
            };
            assert_eq!(result.status, CodeExecutionStatus::Success);
            assert!(result.output.is_empty());
        }
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
    }
}

#[test]
fn nested_p256_executes_each_call_kind_without_consensus_dispatch() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/p256/public.json"
    ))
    .unwrap();
    let row = fixture["p256"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "valid")
        .unwrap();
    let input = hex::decode(row["input"].as_str().unwrap()).unwrap();
    let expected = hex::decode(row["output"].as_str().unwrap()).unwrap();
    let target = rustaxa_evm::p256::P256_VERIFY_ADDRESS;
    for opcode in [0xf1_u8, 0xf2, 0xf4, 0xfa] {
        // Copy the actual Go signature input from code into memory, execute
        // the child, and return both its 32-byte output and CALL success flag.
        let mut code = vec![0x61, 0, 160, 0x61, 0, 0, 0x60, 0, 0x39];
        code.extend_from_slice(&[0x60, 32, 0x60, 0, 0x60, 160, 0x60, 0]);
        if opcode == 0xf1 || opcode == 0xf2 {
            code.extend_from_slice(&[0x60, 0]);
        }
        code.push(0x73);
        code.extend_from_slice(&target);
        code.extend_from_slice(&[
            0x61, 0x1a, 0xf4, opcode, 0x60, 32, 0x52, 0x60, 64, 0x60, 0, 0xf3,
        ]);
        let offset = u16::try_from(code.len()).unwrap().to_be_bytes();
        code[4..6].copy_from_slice(&offset);
        code.extend_from_slice(&input);
        let mut journal = journal_with_parent(code);
        let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
        let mut sequence = PeriodConsensusSequence::new(7_u64.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(PARENT, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(true),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("parent admitted")
        };
        assert_eq!(
            result.status,
            CodeExecutionStatus::Success,
            "opcode {opcode:x}"
        );
        assert_eq!(
            &result.output[..32],
            expected.as_slice(),
            "opcode {opcode:x}"
        );
        assert_eq!(
            &result.output[32..],
            &[vec![0; 31], vec![1]].concat(),
            "opcode {opcode:x}"
        );
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
    }
}

#[test]
fn stateless_top_level_calls_match_primitive_fixtures_without_consensus_facts() {
    let original: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/stateless_public.json"
    ))
    .unwrap();
    let modexp: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/modexp_public.json"
    ))
    .unwrap();
    for (number, row) in original["stateless"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["address"].as_u64().unwrap() as u8, row))
        .chain(
            modexp["modexp"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| (5, row)),
        )
    {
        let required = row["required_gas"].as_u64().unwrap();
        if required > 50_000 {
            continue;
        }
        let target = primitive_address(number);
        let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
        tx.input = hex::decode(row["input"].as_str().unwrap()).unwrap();
        let intrinsic = rustaxa_types::transaction::intrinsic_gas(&tx.input, false).unwrap();
        tx.gas_limit = (intrinsic + required + 100).into();
        let mut journal = journal_without_parent();
        let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
        let mut sequence = PeriodConsensusSequence::new(7_u64.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &block(),
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("must execute")
        };
        assert_eq!(result.status, CodeExecutionStatus::Success);
        assert_eq!(
            result.output,
            hex::decode(row["output"].as_str().unwrap()).unwrap()
        );
        assert_eq!(result.gas_used.as_u64(), intrinsic + required);
        assert_eq!(
            journal.account(target).unwrap().balance.value(),
            &num_bigint::BigInt::from(7)
        );
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
        assert!(
            journal
                .settle_transaction()
                .unwrap()
                .native_invocations
                .is_empty()
        );
    }
}

#[test]
fn stateless_quote_underfunding_reverts_value_and_retains_action_gas() {
    let target = primitive_address(1);
    let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
    tx.gas_limit = 23_999_u64.into();
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(7_u64.into());
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![target]),
        &AddressSet(Vec::new()),
        &mut port,
        &mut sequence,
        &block(),
        &tx,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(
        result.status,
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
    );
    assert_eq!(result.gas_used.as_u64(), 21_000);
    assert!(!journal.account(target).unwrap().exists);
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(sequence.next_sequence(), 0);
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .native_invocations
            .is_empty()
    );
}

fn append_primitive_call(code: &mut Vec<u8>, target: [u8; 20], input_length: u8, gas: u16) {
    code.extend_from_slice(&[0x60, 0, 0x60, 0, 0x60, input_length, 0x60, 0, 0x60, 0, 0x73]);
    code.extend_from_slice(&target);
    code.push(0x61);
    code.extend_from_slice(&gas.to_be_bytes());
    code.push(0xf1);
}

#[test]
fn interleaved_stateless_calls_leave_consensus_sequence_contiguous_after_revert() {
    let mut gas_used = Vec::new();
    for funded in [false, true] {
        let mut code = hex::decode("604060005260016020526001604052600360a053600d60a153").unwrap();
        for (index, (address, length, gas)) in [
            (primitive_address(2), 0, 1000),
            (NATIVE, 0, 1000),
            (primitive_address(5), 162, if funded { 204 } else { 203 }),
            (NATIVE, 0, 1000),
        ]
        .into_iter()
        .enumerate()
        {
            append_primitive_call(&mut code, address, length, gas);
            code.push(0x61);
            code.extend_from_slice(&(192_u16 + 32 * index as u16).to_be_bytes());
            code.push(0x52);
        }
        code.extend_from_slice(&[0x60, 128, 0x60, 192, 0xfd]);
        let mut journal = journal_with_parent(code);
        let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
        let mut sequence = PeriodConsensusSequence::new(7_u64.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![NATIVE, primitive_address(2), primitive_address(5)]),
            &AddressSet(vec![NATIVE]),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(PARENT, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("must execute")
        };
        assert_eq!(
            result.status,
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        );
        let mut expected = vec![0; 128];
        for index in [31, 63, 127] {
            expected[index] = 1;
        }
        expected[95] = u8::from(funded);
        assert_eq!(result.output, expected);
        gas_used.push(result.gas_used.as_u64());
        let calls = port.invocations.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|call| call.id.sequence)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert_eq!(sequence.next_sequence(), 2);
        let facts = journal.settle_transaction().unwrap().native_invocations;
        assert_eq!(facts.len(), 2);
        for (index, fact) in facts.iter().enumerate() {
            assert_eq!(fact.invocation, calls[index]);
            assert_eq!(
                fact.disposition,
                rustaxa_evm::contracts::ConsensusNativeDisposition::OuterFrameReverted
            );
        }
    }
    assert_eq!(gas_used[1] - gas_used[0], 204);
}

#[test]
fn stateless_overlap_is_rejected_and_default_entrypoint_stays_unavailable() {
    let target = primitive_address(2);
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(7_u64.into());
    assert_eq!(
        execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &AddressSet(vec![target]),
            &mut port,
            &mut sequence,
            &block(),
            &transaction(target, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false)
        ),
        Err(ExecutionDriverError::NativeClassifierOverlap { address: target })
    );
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(sequence.next_sequence(), 0);
    let mut journal = journal_without_parent();
    assert_eq!(
        rustaxa_evm::driver::execute_top_level_call(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &block(),
            &transaction(target, ExecutionValue::default()),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false)
        ),
        Err(ExecutionDriverError::NativeCallUnavailable { address: target })
    );
}

#[test]
fn stateless_calls_from_create_initcode_install_the_returned_code() {
    let target = primitive_address(4);
    let mut code = hex::decode("602a6000536001600160016000600073").unwrap();
    code.extend_from_slice(&target);
    code.extend_from_slice(&[0x61, 0x03, 0xe8, 0xf1, 0x50, 0x60, 1, 0x60, 1, 0xf3]);
    let mut tx = transaction(PARENT, ExecutionValue::default());
    tx.kind = ExecutionTransactionKind::Create;
    tx.receiver = None;
    tx.input = code;
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(7_u64.into());
    let result = execute_top_level_create_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![target]),
        &AddressSet(Vec::new()),
        &mut port,
        &mut sequence,
        &block(),
        &tx,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must create")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.output, vec![0x2a]);
    assert!(result.attempted_contract_address.is_some());
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(sequence.next_sequence(), 0);
    let settled = journal.settle_transaction().unwrap();
    assert!(settled.native_invocations.is_empty());
    assert_eq!(settled.writes.code.len(), 1);
    assert_eq!(settled.writes.code[0].code, vec![0x2a]);
}

#[test]
fn stateless_funds_rejection_does_not_touch_callee_or_consensus_sequence() {
    let target = primitive_address(2);
    let mut journal = journal_with_parent(parent_call_then_stop(target, 1, false));
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(7_u64.into());
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(vec![target]),
        &AddressSet(Vec::new()),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(PARENT, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute parent")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert!(!journal.account(target).unwrap().exists);
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(sequence.next_sequence(), 0);
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .native_invocations
            .is_empty()
    );
}

#[test]
fn curve_calls_preserve_funded_errors_gas_and_value_rollback_without_consensus_facts() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/curve_precompiles_reference_public.json"
    ))
    .unwrap();
    for row in corpus["curve_precompiles"].as_array().unwrap() {
        let target = primitive_address(row["address"].as_u64().unwrap() as u8);
        let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
        tx.input = hex::decode(row["input"].as_str().unwrap()).unwrap();
        let intrinsic = rustaxa_types::transaction::intrinsic_gas(&tx.input, false).unwrap();
        let required = row["required_gas"].as_u64().unwrap();
        tx.gas_limit = (intrinsic + required + 100).into();
        let mut journal = journal_without_parent();
        let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
        let mut sequence = PeriodConsensusSequence::new(7_u64.into());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![target]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &block(),
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("must execute")
        };
        let error = row["error"].as_str().unwrap();
        let expected_status = if error.is_empty() {
            CodeExecutionStatus::Success
        } else {
            CodeExecutionStatus::Failure(CodeExecutionError::Native(NativeContractFailure {
                error: error.into(),
            }))
        };
        assert_eq!(result.status, expected_status, "{}", row["name"]);
        assert_eq!(
            result.output,
            hex::decode(row["output"].as_str().unwrap()).unwrap()
        );
        assert_eq!(result.gas_used.as_u64(), intrinsic + required);
        assert_eq!(
            journal.account(target).unwrap().balance.value(),
            &num_bigint::BigInt::from(if error.is_empty() { 7 } else { 0 })
        );
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
        assert!(
            journal
                .settle_transaction()
                .unwrap()
                .native_invocations
                .is_empty()
        );
    }
}

#[test]
fn blake_activation_remains_owned_by_the_supplied_classifier() {
    let target = primitive_address(9);
    let mut journal = journal_without_parent();
    let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
    let mut sequence = PeriodConsensusSequence::new(7_u64.into());
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &AddressSet(Vec::new()),
        &AddressSet(Vec::new()),
        &mut port,
        &mut sequence,
        &block(),
        &transaction(target, ExecutionValue::default()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    // Without registry activation this is an empty ordinary account call.
    // Activated BLAKE2F instead rejects this empty input, as the curve corpus proves.
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert!(result.output.is_empty());
    assert_eq!(result.gas_used.as_u64(), 21_000);
    assert!(port.invocations.lock().unwrap().is_empty());
    assert_eq!(sequence.next_sequence(), 0);
}

#[test]
fn falcon_frame_dispatch_matches_reference_errors_gas_and_value() {
    use rustaxa_evm::falcon::FALCON_VERIFY_ADDRESS;
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/falcon/public.json"
    ))
    .unwrap();
    for row in fixture["falcon"].as_array().unwrap() {
        let target = FALCON_VERIFY_ADDRESS;
        let quote = row["required_gas"].as_u64().unwrap();
        for gas in [quote - 1, quote, quote + 1] {
            let mut tx = transaction(target, ExecutionValue::new(BigUint::from(7_u8)));
            tx.input = hex::decode(row["input"].as_str().unwrap()).unwrap();
            let intrinsic = rustaxa_types::transaction::intrinsic_gas(&tx.input, false).unwrap();
            tx.gas_limit = (intrinsic + gas).into();
            let mut journal = journal_without_parent();
            let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0xff]);
            let mut sequence = PeriodConsensusSequence::new(block().period);
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &AddressSet(vec![target]),
                &AddressSet(Vec::new()),
                &mut port,
                &mut sequence,
                &block(),
                &tx,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(true),
            );
            if gas >= quote && !row["panic"].as_str().unwrap().is_empty() {
                assert_eq!(
                    result,
                    Err(ExecutionDriverError::Stateless(
                        rustaxa_evm::contracts::NativePortError::Infrastructure(
                            "Falcon reference ABI would panic".into()
                        )
                    ))
                );
                assert!(port.invocations.lock().unwrap().is_empty());
                assert_eq!(sequence.next_sequence(), 0);
                // Infrastructure errors abort the containing execution session;
                // they must not become a normal receipt or a usable continuation.
                drop(journal);
                continue;
            }
            let TransactionExecutionResult::Executed(result) = result.unwrap() else {
                panic!("Falcon admission")
            };
            let error = row["error"].as_str().unwrap();
            if gas < quote {
                assert_eq!(
                    result.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                );
                assert_eq!(result.gas_used.as_u64(), intrinsic);
                assert!(result.output.is_empty());
            } else {
                assert_eq!(
                    result.status,
                    if error.is_empty() {
                        CodeExecutionStatus::Success
                    } else {
                        CodeExecutionStatus::Failure(CodeExecutionError::Native(
                            NativeContractFailure {
                                error: error.into(),
                            },
                        ))
                    },
                    "{}",
                    row["name"]
                );
                assert_eq!(result.gas_used.as_u64(), intrinsic + quote);
                assert_eq!(
                    result.output,
                    hex::decode(row["output"].as_str().unwrap()).unwrap()
                );
            }
            if gas >= quote && error.is_empty() {
                assert_eq!(
                    journal.account(target).unwrap().balance.value(),
                    &num_bigint::BigInt::from(7)
                );
            } else {
                assert!(!journal.account(target).unwrap().exists);
            }
            assert!(port.invocations.lock().unwrap().is_empty());
            assert_eq!(sequence.next_sequence(), 0);
        }
    }
}

/// Complete Go EVM executions pin return-copy ordering across all local phases.
#[test]
fn return_data_copy_matches_go_memory_bounds_gas_and_reference_panics() {
    use rustaxa_evm::profile::TaraxaPhase;
    let public: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/returndata_public.json"
    ))
    .unwrap();
    let local: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/returndata_local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    let rows = public["returndata"].as_array().unwrap();
    assert_eq!(rows.len(), 69);
    for row in rows {
        let phase = match row["phase"].as_str().unwrap() {
            "californicum" => TaraxaPhase::Californicum,
            "ficus" => TaraxaPhase::Ficus,
            "cacti" => TaraxaPhase::Cacti,
            _ => unreachable!(),
        };
        let code = hex::decode(row["code"].as_str().unwrap()).unwrap();
        let mut journal = journal_with_parent(code);
        let mut port = ScriptedPort::completed(NativeStatus::Success, Vec::new());
        let mut sequence = PeriodConsensusSequence::new(block().period);
        let mut request = transaction(PARENT, ExecutionValue::default());
        request.gas_limit = row["gas_limit"].as_u64().unwrap().into();
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &AddressSet(vec![primitive_address(4)]),
            &AddressSet(Vec::new()),
            &mut port,
            &mut sequence,
            &block(),
            &request,
            EnvelopeRules { cornus: true },
            TaraxaProfile::for_phase(phase),
        );
        if !row["panic"].as_str().unwrap().is_empty() {
            assert_eq!(
                result,
                Err(ExecutionDriverError::ReferenceInstructionPanic(0x3e)),
                "{}",
                row["name"]
            );
            continue;
        }
        let TransactionExecutionResult::Executed(result) = result.unwrap() else {
            panic!("reference admission")
        };
        let status = match row["execution_error"].as_str().unwrap() {
            "" => CodeExecutionStatus::Success,
            "return data out of bounds" => {
                CodeExecutionStatus::Failure(CodeExecutionError::ReturnDataOutOfBounds)
            }
            "out of gas" => CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            "gas uint64 overflow" => {
                CodeExecutionStatus::Failure(CodeExecutionError::GasUintOverflow)
            }
            "stack underflow (0 <=> 3)" => {
                CodeExecutionStatus::Failure(CodeExecutionError::StackUnderflow)
            }
            error => panic!("unhandled reference error {error}"),
        };
        assert_eq!(result.status, status, "{}", row["name"]);
        assert_eq!(
            result.gas_used.as_u64(),
            row["gas_used"].as_u64().unwrap(),
            "{}",
            row["name"]
        );
        assert_eq!(
            hex::encode(result.output),
            row["output"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        assert!(port.invocations.lock().unwrap().is_empty());
        assert_eq!(sequence.next_sequence(), 0);
    }
}
