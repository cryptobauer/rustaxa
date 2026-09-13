//! Iterative nested-frame checks against the pinned Go fixture corpus.
//!
//! These tests execute actual REVM CALL/CREATE actions over the Rust journal.
//! They compare logical execution facts only; persisted trie roots, native
//! kernels and production routing remain separate boundaries.

use std::{cell::Cell, collections::BTreeMap};

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call},
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
use serde_json::Value;

const SENDER: [u8; 20] = address_with_last(0xaa);
const PARENT: [u8; 20] = address_with_last(0xbb);
const CHILD: [u8; 20] = address_with_last(0xcc);

#[derive(Clone)]
struct SeedAccount {
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    storage_root: Option<[u8; 32]>,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
}

struct FixtureReader {
    accounts: BTreeMap<[u8; 20], SeedAccount>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    reads: Cell<usize>,
}

impl ConcreteStateRead for FixtureReader {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: [0x44; 32],
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
        self.reads.set(self.reads.get() + 1);
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
        unreachable!("fixtures do not execute BLOCKHASH")
    }
}

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

#[test]
fn iterative_creation_matches_all_pinned_go_frame_rows() {
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .unwrap();
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/public.json"
    ))
    .unwrap();
    assert_eq!(local["creation_frames"], public["creation_frames"]);

    for row in local["creation_frames"].as_array().unwrap() {
        let name = row["case"].as_str().unwrap();
        let parent_code = bytes(&row["parent_code"]);
        let parent_hash = keccak256(&parent_code).0;
        let mut accounts = BTreeMap::from([
            (
                SENDER,
                SeedAccount {
                    nonce: FinalChainNonce::from_u64(1),
                    balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                    storage_root: None,
                    code_hash: None,
                    code_size: 0,
                },
            ),
            (
                PARENT,
                SeedAccount {
                    nonce: nonce_hex(row["parent_nonce"].as_str().unwrap()),
                    balance: ConcreteAccountBalance::default(),
                    storage_root: None,
                    code_hash: Some(parent_hash),
                    code_size: parent_code.len() as u64,
                },
            ),
        ]);
        if row["collision"] == true {
            accounts.insert(
                address(&row["child"]),
                SeedAccount {
                    nonce: FinalChainNonce::from_u64(1),
                    balance: ConcreteAccountBalance::default(),
                    storage_root: None,
                    code_hash: None,
                    code_size: 0,
                },
            );
        }
        let mut journal = ExecutionJournal::new(FixtureReader {
            accounts,
            codes: BTreeMap::from([(parent_hash, parent_code)]),
            storage: BTreeMap::new(),
            reads: Cell::new(0),
        });
        let gas_cap = row["gas_cap"].as_u64().unwrap();
        let result = execute_top_level_call(
            &mut journal,
            &NoHistory,
            &NoNative,
            &block(false),
            &transaction(PARENT, gas_cap),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("{name}: expected execution")
        };
        let expected_status = if row["execution_error"].as_str().unwrap().is_empty() {
            CodeExecutionStatus::Success
        } else {
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        };
        assert_eq!(result.status, expected_status, "{name}: status");
        assert_eq!(result.gas_used.as_u64(), row["gas_used"], "{name}: gas");
        assert_eq!(hex::encode(result.output), row["return"], "{name}: output");

        for (key, expected) in row["accounts"].as_object().unwrap() {
            let address = address_text(key);
            let metadata = journal.account_metadata(address).unwrap();
            if expected.is_null() {
                assert!(!metadata.exists, "{name}: unexpected account {key}");
                continue;
            }
            assert!(metadata.exists, "{name}: missing account {key}");
            assert_eq!(
                metadata.nonce,
                nonce_decimal(expected["nonce"].as_str().unwrap()),
                "{name}: nonce {key}"
            );
            assert_eq!(
                metadata.balance.value().to_string(),
                expected["balance"].as_str().unwrap(),
                "{name}: balance {key}"
            );
            assert_eq!(
                hex::encode(journal.account_code(address).unwrap()),
                expected["code"].as_str().unwrap(),
                "{name}: code {key}"
            );
        }
    }
}

#[test]
fn nested_transient_revert_matches_pinned_go_call_result() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .unwrap();
    let row = fixture["opcodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["case"] == "nested-transient-revert")
        .unwrap();
    let parent = bytes(&row["code"]);
    let child = bytes(&row["child_code"]);
    let mut journal = journal_with_code_accounts(&[(PARENT, parent), (CHILD, child)]);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(true),
        &transaction(PARENT, 100_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(true),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("expected execution")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), row["gas_used"]);
    assert_eq!(hex::encode(result.output), row["return"]);
    let mut key = [0_u8; 32];
    key[31] = 1;
    assert_eq!(
        hex::encode(journal.transient_storage(CHILD, ConcreteStorageKey(key))),
        row["transient"]
    );
}

#[test]
fn nested_static_sstore_rejection_matches_pinned_go_result() {
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_local.json"
    ))
    .unwrap();
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_public.json"
    ))
    .unwrap();
    assert_eq!(local, public);
    let row = local["sstore"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["case"] == "static-rejection")
        .unwrap();
    let parent = bytes(&row["code"]);
    let child = bytes(&row["child_code"]);
    let mut journal = journal_with_code_accounts(&[(PARENT, parent), (CHILD, child)]);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, row["gas_cap"].as_u64().unwrap()),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("expected execution")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), row["gas_used"]);
    assert_eq!(hex::encode(result.output), row["return"]);
    assert_eq!(journal.refund(), row["refund"].as_u64().unwrap());
    assert_eq!(
        journal
            .ordinary_storage(CHILD, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::default()
    );
}

#[test]
fn callcode_and_delegatecall_preserve_taraxa_frame_context() {
    let child = hex::decode("30600055336001553460025500").unwrap();

    let mut callcode_parent = vec![
        0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x07, 0x73,
    ];
    callcode_parent.extend_from_slice(&CHILD);
    callcode_parent.extend_from_slice(&[0x61, 0xff, 0xff, 0xf2, 0x50, 0x00]);
    let mut journal = journal_with_code_accounts_and_balances(
        &[(PARENT, callcode_parent), (CHILD, child.clone())],
        &[(PARENT, 100)],
    );
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, 200_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    assert_slots(
        &journal,
        PARENT,
        [
            BigUint::from_bytes_be(&PARENT),
            BigUint::from_bytes_be(&PARENT),
            BigUint::from(7_u8),
        ],
    );
    assert_slots(&journal, CHILD, std::array::from_fn(|_| BigUint::default()));
    assert_eq!(
        journal.account(PARENT).unwrap().balance.value(),
        &num_bigint::BigInt::from(100_u8)
    );

    let mut delegate_parent = vec![0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x73];
    delegate_parent.extend_from_slice(&CHILD);
    delegate_parent.extend_from_slice(&[0x61, 0xff, 0xff, 0xf4, 0x50, 0x00]);
    let mut journal = journal_with_code_accounts(&[(PARENT, delegate_parent), (CHILD, child)]);
    let mut transaction = transaction(PARENT, 200_000);
    transaction.value = ExecutionValue::new(BigUint::from(9_u8));
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
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
    assert_slots(
        &journal,
        PARENT,
        [
            BigUint::from_bytes_be(&PARENT),
            BigUint::from_bytes_be(&SENDER),
            BigUint::from(9_u8),
        ],
    );
    assert_slots(&journal, CHILD, std::array::from_fn(|_| BigUint::default()));
}

#[test]
fn child_negative_refund_is_discarded_with_enclosing_revert() {
    let child = hex::decode("600760005500").unwrap();
    let mut parent = hex::decode("60006000556000600060006000").unwrap();
    parent.push(0x73);
    parent.extend_from_slice(&CHILD);
    parent.extend_from_slice(&[0x61, 0xff, 0xff, 0xf4, 0x50]);

    let mut success = parent.clone();
    success.push(0x00);
    let mut journal = journal_with_original_slot(&[(PARENT, success), (CHILD, child.clone())], 7);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, 200_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Success
    ));
    assert_eq!(journal.refund(), 4_800);
    assert_eq!(
        journal
            .ordinary_storage(PARENT, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::from(7_u8)
    );

    parent.extend_from_slice(&[0x60, 0x00, 0x60, 0x00, 0xfd]);
    let mut journal = journal_with_original_slot(&[(PARENT, parent), (CHILD, child)], 7);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, 200_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert!(matches!(
        result,
        TransactionExecutionResult::Executed(ref result)
            if result.status == CodeExecutionStatus::Failure(CodeExecutionError::Revert)
    ));
    assert_eq!(journal.refund(), 0);
    assert_eq!(
        journal
            .ordinary_storage(PARENT, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::from(7_u8)
    );
}

#[test]
fn depth_1025_call_is_rejected_at_the_pre_entry_boundary() {
    let mut parent = hex::decode("6000546001016000556000600060006000600073").unwrap();
    parent.extend_from_slice(&PARENT);
    parent.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
    let mut journal = journal_with_code_accounts(&[(PARENT, parent)]);
    let mut transaction = transaction(PARENT, u64::MAX);
    transaction.gas_price = ExecutionGasPrice::new(BigUint::default());
    let mut block = block(false);
    block.gas_limit = FinalChainGas::new(u64::MAX);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
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
    assert_eq!(
        journal
            .ordinary_storage(PARENT, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::from(1_025_u16)
    );
}

#[test]
fn out_of_funds_returns_call_value_stipend_to_parent() {
    let mut parent = hex::decode("6000600060006000600173").unwrap();
    parent.extend_from_slice(&CHILD);
    parent.extend_from_slice(&[0x61, 0x03, 0xe8, 0xf1, 0x00]);
    let mut journal = journal_with_code_accounts(&[(PARENT, parent), (CHILD, Vec::new())]);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, 100_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("expected execution")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), 28_421);
    assert_eq!(
        journal.account(PARENT).unwrap().balance.value(),
        &num_bigint::BigInt::default()
    );
}

#[test]
fn call_copies_only_the_requested_return_memory_prefix() {
    let child = hex::decode("63aabbccdd6000526004601cf3").unwrap();
    let mut parent =
        hex::decode("60ee60005360ee60015360ee60025360ee60035360026000600060006000").unwrap();
    parent.push(0x73);
    parent.extend_from_slice(&CHILD);
    parent.extend_from_slice(&[0x61, 0xff, 0xff, 0xf1, 0x50, 0x60, 0x04, 0x60, 0x00, 0xf3]);
    let mut journal = journal_with_code_accounts(&[(PARENT, parent), (CHILD, child)]);
    let result = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(false),
        &transaction(PARENT, 100_000),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("expected execution")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.output, hex::decode("aabbeeee").unwrap());
}

fn journal_with_code_accounts(
    code_accounts: &[([u8; 20], Vec<u8>)],
) -> ExecutionJournal<FixtureReader> {
    journal_with_code_accounts_and_balances(code_accounts, &[])
}

fn journal_with_code_accounts_and_balances(
    code_accounts: &[([u8; 20], Vec<u8>)],
    balances: &[([u8; 20], u64)],
) -> ExecutionJournal<FixtureReader> {
    let mut accounts = BTreeMap::from([(
        SENDER,
        SeedAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    )]);
    let mut codes = BTreeMap::new();
    for (address, code) in code_accounts {
        let hash = keccak256(code).0;
        accounts.insert(
            *address,
            SeedAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: balances
                    .iter()
                    .find_map(|(owner, balance)| (owner == address).then_some(*balance))
                    .map(BigUint::from)
                    .map(ConcreteAccountBalance::new)
                    .unwrap_or_default(),
                storage_root: None,
                code_hash: Some(hash),
                code_size: code.len() as u64,
            },
        );
        codes.insert(hash, code.clone());
    }
    ExecutionJournal::new(FixtureReader {
        accounts,
        codes,
        storage: BTreeMap::new(),
        reads: Cell::new(0),
    })
}

fn journal_with_original_slot(
    code_accounts: &[([u8; 20], Vec<u8>)],
    original: u64,
) -> ExecutionJournal<FixtureReader> {
    let mut accounts = BTreeMap::from([(
        SENDER,
        SeedAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    )]);
    let mut codes = BTreeMap::new();
    for (address, code) in code_accounts {
        let hash = keccak256(code).0;
        accounts.insert(
            *address,
            SeedAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: (*address == PARENT).then_some([0x55; 32]),
                code_hash: Some(hash),
                code_size: code.len() as u64,
            },
        );
        codes.insert(hash, code.clone());
    }
    ExecutionJournal::new(FixtureReader {
        accounts,
        codes,
        storage: BTreeMap::from([(
            (PARENT, ConcreteStorageKey([0_u8; 32])),
            BigUint::from(original).to_bytes_be(),
        )]),
        reads: Cell::new(0),
    })
}

fn assert_slots(
    journal: &ExecutionJournal<FixtureReader>,
    address: [u8; 20],
    expected: [BigUint; 3],
) {
    for (index, expected) in expected.into_iter().enumerate() {
        let mut key = [0_u8; 32];
        key[31] = index as u8;
        assert_eq!(
            journal
                .ordinary_storage(address, ConcreteStorageKey(key))
                .unwrap()
                .1,
            expected,
            "slot {index}"
        );
    }
}

fn transaction(receiver: [u8; 20], gas_limit: u64) -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(receiver),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(gas_limit),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn block(_cacti: bool) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(1),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn address(value: &Value) -> [u8; 20] {
    address_text(value.as_str().unwrap())
}

fn address_text(value: &str) -> [u8; 20] {
    hex::decode(value).unwrap().try_into().unwrap()
}

fn nonce_hex(value: &str) -> FinalChainNonce {
    let value = BigUint::parse_bytes(value.trim_start_matches("0x").as_bytes(), 16).unwrap();
    nonce(value)
}

fn nonce_decimal(value: &str) -> FinalChainNonce {
    nonce(BigUint::parse_bytes(value.as_bytes(), 10).unwrap())
}

fn nonce(value: BigUint) -> FinalChainNonce {
    if value == BigUint::default() {
        FinalChainNonce::zero()
    } else {
        FinalChainNonce::from_bytes(&value.to_bytes_be()).unwrap()
    }
}

const fn address_with_last(last: u8) -> [u8; 20] {
    let mut address = [0_u8; 20];
    address[19] = last;
    address
}
