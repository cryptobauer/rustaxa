//! Execution trace event and rollback checks.

use std::{cell::Cell, collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::{interpreter::InstructionResult, primitives::keccak256};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
        TransactionExecutionResult,
    },
    driver::{ExecutionDriverError, NativeAddressClassifier, execute_top_level_call_with_trace},
    envelope::EnvelopeRules,
    host::HostError,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
    trace::{TraceCollector, TraceEvent, TraceOpcode},
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trace_public.json")
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
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("empty call must be admitted");
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used, 21_000.into());
    assert!(result.output.is_empty());
}
