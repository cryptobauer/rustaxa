//! Focused comparisons between the typed driver observer and pinned Go rows.

use std::{cell::Cell, collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::{interpreter::InstructionResult, primitives::keccak256};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
        TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, execute_top_level_call_with_trace,
        execute_top_level_create_with_trace,
    },
    envelope::EnvelopeRules,
    host::HostError,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
    structured_trace::{StructuredTraceResult, serialize_structured_results},
    trace::{TraceCollector, TraceEvent, TraceOpcode, TraceOpcodePhase},
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

const SENDER: [u8; 20] = address(0xaa);

const fn address(suffix: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = suffix;
    address
}

#[derive(Clone)]
struct ReaderAccount {
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    storage_root: Option<[u8; 32]>,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
}

struct MemoryReader {
    accounts: BTreeMap<[u8; 20], ReaderAccount>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    codes: BTreeMap<[u8; 32], ConcreteRead<Vec<u8>>>,
    code_reads: Cell<usize>,
}

impl ConcreteStateRead for MemoryReader {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(7),
            state_root: [0x77; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        let Some(account) = self.accounts.get(&address) else {
            return Ok(ConcreteRead::Absent);
        };
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: account.nonce.clone(),
                balance: account.balance.clone(),
                storage_root: account.storage_root,
                code_hash: account.code_hash,
                code_size: account.code_size,
            },
            physical_rlp: vec![0xc0],
        }))
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .storage
            .get(&(address, key))
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.code_reads.set(self.code_reads.get() + 1);
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .unwrap_or(ConcreteRead::Absent))
    }
}

struct BlockHashes;

impl BlockHashRead for BlockHashes {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        Ok([number.as_u64() as u8; 32])
    }
}

struct MissingBlockHashes;

impl BlockHashRead for MissingBlockHashes {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        Err(BlockHashReadError::HistoryUnavailable(number))
    }
}

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

fn fixture() -> Value {
    serde_json::from_str(&fs::read_to_string(fixture_path()).unwrap()).unwrap()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/trace_public.json")
}

fn scenario<'a>(fixture: &'a Value, name: &str) -> &'a Value {
    fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()
}

fn account_code(fixture: &Value, suffix: &str) -> Vec<u8> {
    fixture["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["address"].as_str().unwrap().ends_with(suffix))
        .and_then(|account| account["code"].as_str())
        .map(|code| hex::decode(code).unwrap())
        .unwrap()
}

fn decimal(value: &str) -> BigUint {
    BigUint::parse_bytes(value.as_bytes(), 10).unwrap()
}

fn reader(fixture: &Value, target: [u8; 20], code: Vec<u8>, slot: Option<u8>) -> MemoryReader {
    let sender_row = fixture["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["address"] == "00000000000000000000000000000000000000aa")
        .unwrap();
    let sender_nonce = decimal(sender_row["nonce"].as_str().unwrap());
    let code_hash = keccak256(&code).0;
    let accounts = BTreeMap::from([
        (
            SENDER,
            ReaderAccount {
                nonce: FinalChainNonce::from_bytes(&sender_nonce.to_bytes_be()).unwrap(),
                balance: ConcreteAccountBalance::new(decimal(
                    sender_row["balance"].as_str().unwrap(),
                )),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        ),
        (
            target,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(if target == address(0xbb) {
                    50_u8
                } else if target == address(0xcc) {
                    11_u8
                } else {
                    0_u8
                })),
                storage_root: slot.map(|_| [0x44; 32]),
                code_hash: Some(code_hash),
                code_size: code.len() as u64,
            },
        ),
    ]);
    let storage = slot.map_or_else(BTreeMap::new, |value| {
        BTreeMap::from([((target, ConcreteStorageKey([0; 32])), vec![value])])
    });
    MemoryReader {
        accounts,
        storage,
        codes: BTreeMap::from([(code_hash, ConcreteRead::Present(code))]),
        code_reads: Cell::new(0),
    }
}

fn transaction(fixture: &Value, target: Option<[u8; 20]>, input: Vec<u8>) -> ExecutionTransaction {
    let stored = fixture["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["address"] == "00000000000000000000000000000000000000aa")
        .unwrap();
    let nonce = decimal(stored["nonce"].as_str().unwrap()) + BigUint::from(1_u8);
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: target,
        nonce: FinalChainNonce::from_bytes(&nonce.to_bytes_be()).unwrap(),
        gas_price: ExecutionGasPrice::new(BigUint::from(2_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::new(BigUint::from(3_u8)),
        input,
        canonical_rlp: None,
        kind: if target.is_some() {
            ExecutionTransactionKind::Call
        } else {
            ExecutionTransactionKind::Create
        },
    }
}

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(8),
        author: [0; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn opcode_rows(collector: &TraceCollector) -> Vec<&TraceOpcode> {
    collector
        .events()
        .iter()
        .filter_map(|event| match event {
            TraceEvent::Opcode(opcode) => Some(opcode),
            TraceEvent::FrameEnter(_) | TraceEvent::FrameExit(_) => None,
        })
        .collect()
}

fn expected_word(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

fn expected_opcode(name: &str) -> u8 {
    match name {
        "STOP" => 0x00,
        "ADD" => 0x01,
        "ADDRESS" => 0x30,
        "BALANCE" => 0x31,
        "ORIGIN" => 0x32,
        "RETURNDATACOPY" => 0x3e,
        "MSTORE" => 0x52,
        "SLOAD" => 0x54,
        "SSTORE" => 0x55,
        "PUSH1" => 0x60,
        "PUSH32" => 0x7f,
        "DUP1" => 0x80,
        "RETURN" => 0xf3,
        "REVERT" => 0xfd,
        other => panic!("unmapped fixture opcode {other}"),
    }
}

fn compare_struct_rows(actual: &[&TraceOpcode], expected: &Value, fault: InstructionResult) {
    let expected = expected["outputs"]["struct"]["result"][0]["structLogs"]
        .as_array()
        .unwrap();
    assert_eq!(actual.len(), expected.len());
    for (actual, expected) in actual.iter().zip(expected) {
        assert_eq!(actual.pc, expected["pc"].as_u64().unwrap());
        assert_eq!(
            actual.opcode,
            expected_opcode(expected["op"].as_str().unwrap())
        );
        assert_eq!(actual.gas, expected["gas"].as_u64().unwrap());
        assert_eq!(actual.gas_cost, expected["gasCost"].as_u64().unwrap());
        assert_eq!(actual.depth, expected["depth"].as_u64().unwrap() as u16);
        assert_eq!(actual.refund, 0);
        assert_eq!(
            actual.stack,
            expected["stack"]
                .as_array()
                .unwrap()
                .iter()
                .map(expected_word)
                .collect::<Vec<_>>()
        );
        let memory = expected["memory"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|word| hex::decode(word.as_str().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual.memory, memory);
        assert_eq!(
            actual.phase,
            if expected.get("error").is_some() {
                TraceOpcodePhase::Fault(fault)
            } else {
                TraceOpcodePhase::BeforeExecution
            }
        );
    }
}

#[test]
fn unsupported_gas_failure_emits_no_guessed_fault_row() {
    let fixture = fixture();
    let target = address(0xdd);
    let code = hex::decode("6000600055").unwrap();
    let mut journal = ExecutionJournal::new(reader(&fixture, target, code, None));
    let mut collector = TraceCollector::default();
    let mut transaction = transaction(&fixture, Some(target), vec![]);
    transaction.gas_limit = FinalChainGas::new(21_008);

    let error = execute_top_level_call_with_trace(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &block(),
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap_err();

    assert_eq!(
        error,
        ExecutionDriverError::TraceFactUnavailable {
            opcode: 0x55,
            result: InstructionResult::OutOfGas,
        }
    );
    let rows = opcode_rows(&collector);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.opcode == 0x60));
}

#[test]
fn unsupported_nested_call_emits_no_guessed_call_row() {
    let fixture = fixture();
    let target = address(0xee);
    let code = hex::decode("6000600060006000600060006000f100").unwrap();
    let mut journal = ExecutionJournal::new(reader(&fixture, target, code, None));
    let mut collector = TraceCollector::default();

    let error = execute_top_level_call_with_trace(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &block(),
        &transaction(&fixture, Some(target), vec![]),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap_err();

    assert_eq!(
        error,
        ExecutionDriverError::TraceFactUnavailable {
            opcode: 0xf1,
            result: InstructionResult::Suspend,
        }
    );
    let rows = opcode_rows(&collector);
    assert_eq!(rows.len(), 7);
    assert!(rows.iter().all(|row| row.opcode == 0x60));
}

#[test]
fn host_failure_keeps_exact_error_and_emits_no_opcode_row() {
    let fixture = fixture();
    let target = address(0xef);
    let code = hex::decode("60004000").unwrap();
    let mut journal = ExecutionJournal::new(reader(&fixture, target, code, None));
    let mut collector = TraceCollector::default();

    let error = execute_top_level_call_with_trace(
        &mut journal,
        &MissingBlockHashes,
        &NoNative,
        &block(),
        &transaction(&fixture, Some(target), vec![]),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap_err();

    assert_eq!(
        error,
        ExecutionDriverError::Host(HostError::BlockHash(
            BlockHashReadError::HistoryUnavailable(FinalChainBlockNumber::new(0))
        ))
    );
    let rows = opcode_rows(&collector);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].opcode, 0x60);
}

fn compare_execution_json(
    result: &TransactionExecutionResult,
    collector: &TraceCollector,
    expected: &Value,
) {
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("witness must pass admission");
    };
    let encoded = serialize_structured_results(&[StructuredTraceResult {
        gas_used: result.gas_used.as_u64(),
        failed: result.status != CodeExecutionStatus::Success,
        return_value: &result.output,
        events: collector.events(),
    }])
    .unwrap();
    let actual: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(actual, expected["outputs"]["struct"]["result"]);
}

#[test]
fn storage_call_matches_pinned_go_opcode_facts() {
    let fixture = fixture();
    let expected = scenario(&fixture, "storage_call");
    let target = address(0xbb);
    let code = account_code(&fixture, "bb");
    let mut journal = ExecutionJournal::new(reader(&fixture, target, code, Some(7)));
    let mut collector = TraceCollector::default();

    let result = execute_top_level_call_with_trace(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &block(),
        &transaction(&fixture, Some(target), vec![]),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap();

    compare_execution_json(&result, &collector, expected);
    let rows = opcode_rows(&collector);
    compare_struct_rows(&rows, expected, InstructionResult::Revert);
    assert_eq!(rows[6].opcode, 0x55);
    assert_eq!(rows[6].attempted_sstore.unwrap().value[31], 8);
    assert_eq!(collector.attempted_storage()[&target][&[0; 32]][31], 8);
}

#[test]
fn revert_and_return_bounds_match_pinned_go_duplicate_fault_rows() {
    let fixture = fixture();
    for (name, target, fault) in [
        ("revert", address(0xcc), InstructionResult::Revert),
        (
            "return_bounds",
            {
                let mut target = [0; 20];
                target[18] = 0xab;
                target[19] = 0xcd;
                target
            },
            InstructionResult::OutOfOffset,
        ),
    ] {
        let code = account_code(&fixture, if name == "revert" { "cc" } else { "abcd" });
        let mut journal = ExecutionJournal::new(reader(&fixture, target, code, None));
        let mut collector = TraceCollector::default();

        let result = execute_top_level_call_with_trace(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &block(),
            &transaction(&fixture, Some(target), vec![]),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
            &mut collector,
        )
        .unwrap();

        compare_execution_json(&result, &collector, scenario(&fixture, name));
        compare_struct_rows(&opcode_rows(&collector), scenario(&fixture, name), fault);
    }
}

#[test]
fn create_initcode_matches_pinned_go_opcode_facts() {
    let fixture = fixture();
    let initcode = hex::decode(
        "7f602a60005260206000f3000000000000000000000000000000000000000000600052600a6000f3",
    )
    .unwrap();
    let mut accounts = BTreeMap::new();
    let sender_row = fixture["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["address"] == "00000000000000000000000000000000000000aa")
        .unwrap();
    let nonce = decimal(sender_row["nonce"].as_str().unwrap());
    accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_bytes(&nonce.to_bytes_be()).unwrap(),
            balance: ConcreteAccountBalance::new(decimal(sender_row["balance"].as_str().unwrap())),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    let mut journal = ExecutionJournal::new(MemoryReader {
        accounts,
        storage: BTreeMap::new(),
        codes: BTreeMap::new(),
        code_reads: Cell::new(0),
    });
    let mut collector = TraceCollector::default();

    let result = execute_top_level_create_with_trace(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &block(),
        &transaction(&fixture, None, initcode),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap();

    compare_execution_json(&result, &collector, scenario(&fixture, "create"));
    compare_struct_rows(
        &opcode_rows(&collector),
        scenario(&fixture, "create"),
        InstructionResult::Revert,
    );
}

/// Raw Go logger observations prove refund changes at the same opcode boundary,
/// including the duplicate REVERT row before outer rollback clears the refund.
#[test]
fn nonzero_refund_trace_facts_match_actual_go_logger() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/trace_refund/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/trace_refund/local.json"
    ))
    .unwrap();
    assert_eq!(cases, local);
    let singles = cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("targets").is_none())
        .collect::<Vec<_>>();
    assert_eq!(singles.len(), 4);
    for case in singles {
        assert_eq!(case["state_before"], case["state_after"]);
        let target = address(0xbb);
        let mut journal = ExecutionJournal::new(reader(
            case,
            target,
            hex::decode(case["code"].as_str().unwrap()).unwrap(),
            Some(7),
        ));
        let mut collector = TraceCollector::default();
        let result = execute_top_level_call_with_trace(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &block(),
            &transaction(case, Some(target), vec![]),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
            &mut collector,
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("refund witness must pass admission");
        };
        let expected = &case["result"][0];
        assert_eq!(result.gas_used.as_u64(), expected["gas"].as_u64().unwrap());
        assert_eq!(
            result.status != CodeExecutionStatus::Success,
            expected["failed"].as_bool().unwrap()
        );
        assert_eq!(hex::encode(&result.output), expected["returnValue"]);
        let actual = opcode_rows(&collector);
        let expected_rows = expected["structLogs"].as_array().unwrap();
        assert_eq!(actual.len(), expected_rows.len());
        assert!(
            expected_rows
                .iter()
                .any(|row| row["refund"].as_u64().unwrap() > 0)
        );
        for (index, (actual, expected)) in actual.iter().zip(expected_rows).enumerate() {
            assert_eq!(actual.pc, expected["pc"].as_u64().unwrap());
            assert_eq!(actual.opcode as u64, expected["op"].as_u64().unwrap());
            assert_eq!(actual.gas, expected["gas"].as_u64().unwrap());
            assert_eq!(actual.gas_cost, expected["gasCost"].as_u64().unwrap());
            assert_eq!(actual.depth as u64, expected["depth"].as_u64().unwrap());
            assert_eq!(actual.refund, expected["refund"].as_u64().unwrap());
            assert_eq!(actual.state_address, target);
            // All four input programs leave memory empty; raw Go []byte JSON is base64.
            assert_eq!(expected["memory"], "");
            assert_eq!(expected["memSize"], 0);
            assert!(actual.memory.is_empty());
            let stack = expected["stack"]
                .as_array()
                .unwrap()
                .iter()
                .map(|word| {
                    let bytes = decimal(word.as_str().unwrap()).to_bytes_be();
                    let mut value = [0; 32];
                    value[32 - bytes.len()..].copy_from_slice(&bytes);
                    value
                })
                .collect::<Vec<_>>();
            assert_eq!(actual.stack, stack);
            assert_eq!(
                actual.phase,
                if case["name"] == "clear_revert" && index + 1 == expected_rows.len() {
                    TraceOpcodePhase::Fault(InstructionResult::Revert)
                } else {
                    TraceOpcodePhase::BeforeExecution
                }
            );
        }
    }
}

/// Go retains storage originals, cumulative refunds and transient cells across
/// prefix and target Main calls; only each target's logger is freshly allocated.
#[test]
fn sequential_trace_refunds_and_transient_state_match_go() {
    let cases: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/trace_refund/public.json"
    ))
    .unwrap();
    let sequences = cases
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("targets").is_some())
        .collect::<Vec<_>>();
    assert_eq!(sequences.len(), 3);
    for case in sequences {
        let target = address(0xbb);
        let mut journal = ExecutionJournal::new(reader(
            case,
            target,
            hex::decode(case["code"].as_str().unwrap()).unwrap(),
            Some(7),
        ));
        assert_eq!(
            case["prefix_inputs"].as_array().unwrap().len(),
            case["prefix_nonces"].as_array().unwrap().len()
        );
        assert_eq!(
            case["target_inputs"].as_array().unwrap().len(),
            case["target_nonces"].as_array().unwrap().len()
        );
        assert_eq!(
            case["target_inputs"].as_array().unwrap().len(),
            case["result"].as_array().unwrap().len()
        );
        let make_transaction = |input: &Value, nonce: &Value| {
            let mut tx = transaction(
                case,
                Some(target),
                hex::decode(input.as_str().unwrap()).unwrap(),
            );
            tx.nonce = FinalChainNonce::from_bytes(&decimal(nonce.as_str().unwrap()).to_bytes_be())
                .unwrap();
            tx
        };
        for (input, nonce) in case["prefix_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["prefix_nonces"].as_array().unwrap())
        {
            rustaxa_evm::driver::execute_top_level_call(
                &mut journal,
                &BlockHashes,
                &NoNative,
                &block(),
                &make_transaction(input, nonce),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
        }
        for ((input, nonce), expected) in case["target_inputs"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["target_nonces"].as_array().unwrap())
            .zip(case["result"].as_array().unwrap())
        {
            let mut collector = TraceCollector::default();
            let result = execute_top_level_call_with_trace(
                &mut journal,
                &BlockHashes,
                &NoNative,
                &block(),
                &make_transaction(input, nonce),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
                &mut collector,
            )
            .unwrap();
            let TransactionExecutionResult::Executed(result) = result else {
                panic!("sequence admission failed");
            };
            assert_eq!(
                result.gas_used.as_u64(),
                expected["gas"].as_u64().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                result.status != CodeExecutionStatus::Success,
                expected["failed"].as_bool().unwrap()
            );
            assert_eq!(hex::encode(&result.output), expected["returnValue"]);
            let actual = opcode_rows(&collector);
            let expected_rows = expected["structLogs"].as_array().unwrap();
            assert_eq!(actual.len(), expected_rows.len());
            for (actual, expected) in actual.iter().zip(expected_rows) {
                assert_eq!(actual.pc, expected["pc"].as_u64().unwrap());
                assert_eq!(actual.opcode as u64, expected["op"].as_u64().unwrap());
                assert_eq!(actual.gas, expected["gas"].as_u64().unwrap());
                assert_eq!(actual.gas_cost, expected["gasCost"].as_u64().unwrap());
                assert_eq!(actual.refund, expected["refund"].as_u64().unwrap());
                assert_eq!(actual.depth as u64, expected["depth"].as_u64().unwrap());
                assert_eq!(
                    actual.memory.len() as u64,
                    expected["memSize"].as_u64().unwrap()
                );
                assert_eq!(base64(&actual.memory), expected["memory"]);
                assert_eq!(actual.phase, TraceOpcodePhase::BeforeExecution);
                let expected_stack = expected["stack"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| decimal(value.as_str().unwrap()))
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual
                        .stack
                        .iter()
                        .map(|word| BigUint::from_bytes_be(word))
                        .collect::<Vec<_>>(),
                    expected_stack
                );
            }
            assert_eq!(
                journal.refund(),
                expected_rows.last().unwrap()["refund"].as_u64().unwrap()
            );
        }
    }
}

/// Malformed caller-provided refund state must fail without committing the
/// successful interpreter's storage effects or changing the prior counter.
#[test]
fn invalid_cumulative_refund_reverts_root_checkpoint() {
    for (base, current, code, expected) in [
        (
            u64::MAX,
            7_u8,
            "600060005500",
            ExecutionDriverError::Journal(rustaxa_evm::journal::JournalError::RefundOverflow),
        ),
        (
            0,
            0,
            "600760005500",
            ExecutionDriverError::NegativeRootRefund(-10_200),
        ),
    ] {
        let fixture = fixture();
        let target = address(0xbb);
        let mut journal = ExecutionJournal::new(reader(
            &fixture,
            target,
            hex::decode(code).unwrap(),
            Some(7),
        ));
        let key = ConcreteStorageKey([0; 32]);
        journal
            .set_ordinary_storage(target, key, BigUint::from(current))
            .unwrap();
        journal.add_refund(base).unwrap();
        let error = rustaxa_evm::driver::execute_top_level_call(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &block(),
            &transaction(&fixture, Some(target), vec![]),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap_err();
        assert_eq!(error, expected);
        assert_eq!(journal.refund(), base);
        assert_eq!(
            journal.ordinary_storage(target, key).unwrap(),
            (BigUint::from(7_u8), BigUint::from(current))
        );
    }
}

// Raw Go []byte JSON uses standard padded base64, unlike FormatLogs hex words.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let word = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for index in 0..4 {
            result.push(if index > chunk.len() {
                '='
            } else {
                ALPHABET[((word >> (18 - 6 * index)) & 63) as usize] as char
            });
        }
    }
    result
}

#[test]
fn empty_code_has_no_synthetic_stop_trace_row() {
    let fixture = fixture();
    let target = address(0xdd);
    let mut source = reader(&fixture, target, vec![], None);
    source.accounts.remove(&target);
    let mut journal = ExecutionJournal::new(source);
    let mut collector = TraceCollector::default();
    let result = execute_top_level_call_with_trace(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &block(),
        &transaction(&fixture, Some(target), vec![]),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
        &mut collector,
    )
    .unwrap();
    assert!(collector.events().is_empty());
    compare_execution_json(&result, &collector, scenario(&fixture, "empty_code"));
}
