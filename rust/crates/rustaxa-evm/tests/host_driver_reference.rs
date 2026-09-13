//! Journal-host and first complete top-level execution checks.

use std::{cell::Cell, collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::{
    context_interface::Host,
    primitives::{Address, B256, Log, U256, keccak256},
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    driver::{
        ExecutionDriverError, NativeAddressClassifier, execute_top_level_call,
        execute_top_level_create,
    },
    envelope::EnvelopeRules,
    host::{HostError, JournalHost},
    journal::{ExecutionJournal, JournalAccountOperation, JournalError},
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

const SENDER: [u8; 20] = [0xaa; 20];
const TARGET: [u8; 20] = [0xbb; 20];

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
    code_error: Option<ConcreteReadError>,
    code_reads: Cell<usize>,
}

impl ConcreteStateRead for MemoryReader {
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
        self.code_reads.set(self.code_reads.get() + 1);
        if let Some(error) = &self.code_error {
            return Err(error.clone());
        }
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .unwrap_or(ConcreteRead::Absent))
    }
}

struct BlockHashes;

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

impl BlockHashRead for BlockHashes {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        Ok([number.as_u64() as u8; 32])
    }
}

#[test]
fn executes_pinned_go_sstore_set_clear_through_envelope() {
    let fixture: Value =
        serde_json::from_str(&fs::read_to_string(fixture_path()).unwrap()).unwrap();
    let case = fixture["opcodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case"] == "sstore-set-clear")
        .unwrap();
    let code = hex::decode(case["code"].as_str().unwrap()).unwrap();
    let code_hash = keccak256(&code).0;
    let mut accounts = BTreeMap::new();
    accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    accounts.insert(
        TARGET,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash: Some(code_hash),
            code_size: code.len() as u64,
        },
    );
    let mut codes = BTreeMap::new();
    codes.insert(code_hash, ConcreteRead::Present(code));
    let mut journal = ExecutionJournal::new(MemoryReader {
        accounts,
        storage: BTreeMap::new(),
        codes,
        code_error: None,
        code_reads: Cell::new(0),
    });
    let transaction = ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(TARGET),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let result = execute_top_level_call(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &ExecutionBlockContext {
            period: FinalChainBlockNumber::new(1),
            author: [0_u8; 20],
            timestamp: 0,
            gas_limit: FinalChainGas::new(1_000_000),
            chain_id: 1,
            difficulty: BigUint::default(),
        },
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), case["gas_used"].as_u64().unwrap());
    assert_eq!(result.output, Vec::<u8>::new());
    assert_eq!(result.logs, Vec::new());
    assert_eq!(journal.refund(), case["refund"].as_u64().unwrap());
    assert_eq!(
        journal
            .ordinary_storage(TARGET, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::default()
    );
    let settled = journal.settle_transaction().unwrap();
    assert!(
        settled
            .writes
            .accounts
            .iter()
            .any(|write| write.address == SENDER
                && matches!(write.operation, JournalAccountOperation::Upsert { .. }))
    );
}

#[test]
fn create_executes_initcode_storage_and_installs_runtime() {
    let runtime = hex::decode("600160005500").unwrap();
    let initcode = hex::decode("602a6000556006601160003960066000f3600160005500").unwrap();
    let mut accounts = BTreeMap::new();
    accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::zero(),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    let mut journal = ExecutionJournal::new(MemoryReader {
        accounts,
        storage: BTreeMap::new(),
        codes: BTreeMap::new(),
        code_error: None,
        code_reads: Cell::new(0),
    });
    let transaction = ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x55; 32],
        sender: SENDER,
        receiver: None,
        nonce: FinalChainNonce::zero(),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: initcode,
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Create,
    };
    let result = execute_top_level_create(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &ExecutionBlockContext {
            period: FinalChainBlockNumber::new(1),
            author: [0_u8; 20],
            timestamp: 0,
            gas_limit: FinalChainGas::new(1_000_000),
            chain_id: 1,
            difficulty: BigUint::default(),
        },
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    let created = result.attempted_contract_address.unwrap();
    let metadata = journal.account_metadata(created).unwrap();
    assert_eq!(metadata.nonce, FinalChainNonce::from_u64(1));
    assert_eq!(metadata.code_size, runtime.len() as u64);
    assert_eq!(journal.account_code(created).unwrap(), runtime);
    assert_eq!(
        journal
            .ordinary_storage(created, ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::from(42_u8)
    );
}

#[test]
fn code_access_validates_loaded_and_staged_bytes() {
    let loaded = vec![0x60, 0x00];
    let hash = keccak256(&loaded).0;
    let mut reader = reader_with_account(TARGET, Some(hash), loaded.len() as u64);
    reader
        .codes
        .insert(hash, ConcreteRead::Present(loaded.clone()));
    let mut journal = ExecutionJournal::new(reader);
    assert_eq!(journal.account_code(TARGET).unwrap(), loaded);
    let staged = vec![0x60, 0x01, 0x00];
    journal.set_code(TARGET, staged.clone()).unwrap();
    assert_eq!(journal.account_code(TARGET).unwrap(), staged);
}

#[test]
fn code_access_rejects_missing_hash_row_size_and_hash_mismatch() {
    let missing_hash = ExecutionJournal::new(reader_with_account(TARGET, None, 1));
    assert!(matches!(
        missing_hash.account_code(TARGET),
        Err(JournalError::MissingCodeHash { .. })
    ));

    let bytes = vec![0x60, 0x00];
    let hash = keccak256(&bytes).0;
    let missing_row = ExecutionJournal::new(reader_with_account(TARGET, Some(hash), 2));
    assert!(matches!(
        missing_row.account_code(TARGET),
        Err(JournalError::ReferencedCodeMissing { .. })
    ));

    let mut size_reader = reader_with_account(TARGET, Some(hash), 3);
    size_reader
        .codes
        .insert(hash, ConcreteRead::Present(bytes.clone()));
    assert!(matches!(
        ExecutionJournal::new(size_reader).account_code(TARGET),
        Err(JournalError::CodeSizeMismatch { .. })
    ));

    let declared = [0x77; 32];
    let mut hash_reader = reader_with_account(TARGET, Some(declared), bytes.len() as u64);
    hash_reader
        .codes
        .insert(declared, ConcreteRead::Present(bytes));
    assert!(matches!(
        ExecutionJournal::new(hash_reader).account_code(TARGET),
        Err(JournalError::CodeHashMismatch { .. })
    ));

    let mut unavailable = reader_with_account(TARGET, Some(hash), 2);
    unavailable.code_error = Some(ConcreteReadError::Io("code read failed".into()));
    assert!(matches!(
        ExecutionJournal::new(unavailable).account_code(TARGET),
        Err(JournalError::State(ConcreteReadError::Io(_)))
    ));
}

#[test]
fn absent_and_existing_zero_code_do_not_read_code_rows() {
    let absent = ExecutionJournal::new(empty_reader());
    assert_eq!(absent.account_code(TARGET).unwrap(), Vec::<u8>::new());
    assert_eq!(absent.reader().code_reads.get(), 0);

    let existing = ExecutionJournal::new(reader_with_account(TARGET, Some([0x77; 32]), 0));
    assert_eq!(existing.account_code(TARGET).unwrap(), Vec::<u8>::new());
    assert_eq!(existing.reader().code_reads.get(), 0);
}

#[test]
fn host_preserves_transient_logs_code_hash_and_selfdestruct_failure() {
    let mut journal = ExecutionJournal::new(reader_with_account(TARGET, Some([0x77; 32]), 0));
    let transaction = ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0_u8; 32],
        sender: SENDER,
        receiver: Some(TARGET),
        nonce: FinalChainNonce::zero(),
        gas_price: ExecutionGasPrice::new(BigUint::default()),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let block = ExecutionBlockContext {
        period: FinalChainBlockNumber::new(300),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    };
    {
        let mut host = JournalHost::new(
            &mut journal,
            &BlockHashes,
            &block,
            &transaction,
            TaraxaProfile::new(false).gas_params(),
        );
        let key = U256::from(1_u8);
        host.tstore(Address::from(TARGET), key, U256::from(0x55_u8));
        assert_eq!(host.tload(Address::from(TARGET), key), U256::from(0x55_u8));
        host.log(Log::new_unchecked(
            Address::from(TARGET),
            vec![B256::from([0x22; 32])],
            vec![0x33].into(),
        ));
        assert_eq!(
            host.load_account_code_hash(Address::from(TARGET))
                .unwrap()
                .data,
            revm::primitives::KECCAK_EMPTY
        );
        assert_eq!(host.block_hash(300), Some(B256::ZERO));
        assert_eq!(host.block_hash(43), Some(B256::ZERO));
        assert_eq!(host.block_hash(44), Some(B256::from([44_u8; 32])));
        assert!(
            host.selfdestruct(Address::from(TARGET), Address::from(SENDER), false)
                .is_err()
        );
        assert_eq!(host.take_error(), Some(HostError::SelfDestructUnavailable));
    }
    assert_eq!(journal.logs()[0].topics, vec![[0x22; 32]]);
    assert_eq!(journal.logs()[0].data, vec![0x33]);
}

#[test]
fn extcodehash_uses_full_width_eip161_emptiness() {
    let subject = [0x99; 20];
    let mut code = vec![0x73];
    code.extend_from_slice(&subject);
    code.extend_from_slice(&[0x3f, 0x60, 0, 0x52, 0x60, 0x20, 0x60, 0, 0xf3]);
    let code_hash = keccak256(&code).0;
    let mut accounts = BTreeMap::new();
    accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    accounts.insert(
        TARGET,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash: Some(code_hash),
            code_size: code.len() as u64,
        },
    );
    accounts.insert(
        subject,
        ReaderAccount {
            nonce: FinalChainNonce::zero(),
            balance: ConcreteAccountBalance::new(BigUint::from(1_u8) << 256_usize),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    let mut codes = BTreeMap::new();
    codes.insert(code_hash, ConcreteRead::Present(code));
    let mut journal = ExecutionJournal::new(MemoryReader {
        accounts,
        storage: BTreeMap::new(),
        codes,
        code_error: None,
        code_reads: Cell::new(0),
    });
    let result = execute_top_level_call(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &test_block(),
        &call_transaction(TARGET),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(result.output, revm::primitives::KECCAK_EMPTY.to_vec());
}

#[test]
fn ef01_prefixes_remain_legacy_invalid_opcodes() {
    for code in [vec![0xef, 0x01], {
        let mut valid_7702_shape = vec![0xef, 0x01, 0x00];
        valid_7702_shape.extend_from_slice(&[0x44; 20]);
        valid_7702_shape
    }] {
        let code_hash = keccak256(&code).0;
        let mut accounts = BTreeMap::new();
        accounts.insert(
            SENDER,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        );
        accounts.insert(
            TARGET,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: Some(code_hash),
                code_size: code.len() as u64,
            },
        );
        let mut codes = BTreeMap::new();
        codes.insert(code_hash, ConcreteRead::Present(code));
        let mut journal = ExecutionJournal::new(MemoryReader {
            accounts,
            storage: BTreeMap::new(),
            codes,
            code_error: None,
            code_reads: Cell::new(0),
        });
        let result = execute_top_level_call(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &test_block(),
            &call_transaction(TARGET),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("must execute")
        };
        assert_eq!(
            result.status,
            CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(0xef))
        );
        assert_eq!(result.gas_used, FinalChainGas::new(100_000));
    }
}

#[test]
fn sstore_preserves_wide_gas_relations_and_writes_the_evm_word() {
    let mut reader = reader_with_account(TARGET, None, 0);
    reader.accounts.get_mut(&TARGET).unwrap().storage_root = Some([0x44; 32]);
    let key = ConcreteStorageKey([0_u8; 32]);
    let wide = BigUint::from(1_u8) << 256_usize;
    reader.storage.insert((TARGET, key), wide.to_bytes_be());
    let mut journal = ExecutionJournal::new(reader);
    let transaction = call_transaction(TARGET);
    let block = test_block();
    {
        let mut host = JournalHost::new(
            &mut journal,
            &BlockHashes,
            &block,
            &transaction,
            TaraxaProfile::new(false).gas_params(),
        );
        let result = host
            .sstore_skip_cold_load(Address::from(TARGET), U256::ZERO, U256::from(1_u8), false)
            .unwrap();
        assert!(result.data.is_original_eq_present());
        assert!(!result.data.is_original_eq_new());
        assert!(!result.data.is_original_zero());
        assert_eq!(host.take_error(), None);
    }
    assert_eq!(
        journal.ordinary_storage(TARGET, key).unwrap(),
        (wide, BigUint::from(1_u8))
    );
}

#[test]
fn zero_value_call_to_absent_ordinary_account_does_not_touch_target() {
    let mut reader = empty_reader();
    reader.accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    let mut journal = ExecutionJournal::new(reader);
    let result = execute_top_level_call(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &test_block(),
        &call_transaction(TARGET),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(result.gas_used, FinalChainGas::new(21_000));
    let settled = journal.settle_transaction().unwrap();
    assert!(
        !settled
            .writes
            .accounts
            .iter()
            .any(|write| write.address == TARGET)
    );
}

#[test]
fn call_preentry_funds_check_precedes_referenced_code_loading() {
    let unavailable_hash = [0x77; 32];
    let parent = call_program(TARGET, 1);
    let mut journal = journal_with_program_and_unavailable_target(parent, unavailable_hash);
    let result = execute_top_level_call(
        &mut journal,
        &BlockHashes,
        &NoNative,
        &test_block(),
        &call_transaction([0xcc; 20]),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("must execute")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used, FinalChainGas::new(28_421));
    assert_eq!(journal.reader().code_reads.get(), 1);
}

#[test]
fn call_value_new_account_gas_uses_full_width_eip161_emptiness() {
    let cases = [
        (
            "existing empty",
            ReaderAccount {
                nonce: FinalChainNonce::zero(),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            53_421,
        ),
        (
            "nonzero nonce",
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            28_421,
        ),
        (
            "wide nonzero balance with zero low word",
            ReaderAccount {
                nonce: FinalChainNonce::zero(),
                balance: ConcreteAccountBalance::new(BigUint::from(1_u8) << 256_usize),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
            28_421,
        ),
    ];

    for (name, target, expected_gas) in cases {
        let parent_address = [0xcc; 20];
        let parent = call_program(TARGET, 1);
        let parent_hash = keccak256(&parent).0;
        let mut reader = empty_reader();
        reader.accounts.insert(
            SENDER,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        );
        reader.accounts.insert(
            parent_address,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(10_u8)),
                storage_root: None,
                code_hash: Some(parent_hash),
                code_size: parent.len() as u64,
            },
        );
        reader.accounts.insert(TARGET, target);
        reader
            .codes
            .insert(parent_hash, ConcreteRead::Present(parent));
        let mut journal = ExecutionJournal::new(reader);
        let result = execute_top_level_call(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &test_block(),
            &call_transaction(parent_address),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("{name}: must execute")
        };
        assert_eq!(result.status, CodeExecutionStatus::Success, "{name}");
        assert_eq!(result.gas_used, FinalChainGas::new(expected_gas), "{name}");
        assert_eq!(journal.reader().code_reads.get(), 1, "{name}");
    }
}

#[test]
fn admitted_call_loads_and_rejects_unavailable_referenced_code() {
    let unavailable_hash = [0x77; 32];

    let mut nested =
        journal_with_program_and_unavailable_target(call_program(TARGET, 0), unavailable_hash);
    assert!(matches!(
        execute_top_level_call(
            &mut nested,
            &BlockHashes,
            &NoNative,
            &test_block(),
            &call_transaction([0xcc; 20]),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::Journal(
            JournalError::ReferencedCodeMissing { .. }
        ))
    ));
    assert_eq!(nested.reader().code_reads.get(), 2);

    let mut direct_reader = reader_with_account(TARGET, Some(unavailable_hash), 1);
    direct_reader.accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    let mut direct = ExecutionJournal::new(direct_reader);
    assert!(matches!(
        execute_top_level_call(
            &mut direct,
            &BlockHashes,
            &NoNative,
            &test_block(),
            &call_transaction(TARGET),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::Journal(
            JournalError::ReferencedCodeMissing { .. }
        ))
    ));
    assert_eq!(direct.reader().code_reads.get(), 1);
}

#[test]
fn call_preflight_scope_restores_before_strict_extcodesize_loading() {
    let unavailable_hash = [0x77; 32];
    let mut parent = call_program(TARGET, 1);
    assert_eq!(parent.pop(), Some(0x00));
    parent.extend_from_slice(&[0x50, 0x73]);
    parent.extend_from_slice(&TARGET);
    parent.extend_from_slice(&[0x3b, 0x00]);
    let mut journal = journal_with_program_and_unavailable_target(parent, unavailable_hash);
    assert!(matches!(
        execute_top_level_call(
            &mut journal,
            &BlockHashes,
            &NoNative,
            &test_block(),
            &call_transaction([0xcc; 20]),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        ),
        Err(ExecutionDriverError::Host(HostError::Journal(
            JournalError::ReferencedCodeMissing { .. }
        )))
    ));
    assert_eq!(journal.reader().code_reads.get(), 2);
}

#[test]
fn depth_preentry_does_not_load_the_rejected_target_code() {
    let addresses: Vec<[u8; 20]> = (0..1_025).map(chain_address).collect();
    let unavailable = [0xee; 20];
    let unavailable_hash = [0x77; 32];
    let mut reader = empty_reader();
    reader.accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    for (index, address) in addresses.iter().enumerate() {
        let callee = addresses.get(index + 1).copied().unwrap_or(unavailable);
        let mut code = hex::decode("60016000556000600060006000600073").unwrap();
        code.extend_from_slice(&callee);
        code.extend_from_slice(&[0x5a, 0xf1, 0x50, 0x00]);
        let hash = keccak256(&code).0;
        reader.accounts.insert(
            *address,
            ReaderAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                storage_root: None,
                code_hash: Some(hash),
                code_size: code.len() as u64,
            },
        );
        reader.codes.insert(hash, ConcreteRead::Present(code));
    }
    reader.accounts.insert(
        unavailable,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash: Some(unavailable_hash),
            code_size: 1,
        },
    );
    let mut journal = ExecutionJournal::new(reader);
    let mut transaction = call_transaction(addresses[0]);
    transaction.gas_limit = FinalChainGas::new(u64::MAX);
    transaction.gas_price = ExecutionGasPrice::new(BigUint::default());
    let mut block = test_block();
    block.gas_limit = FinalChainGas::new(u64::MAX);
    let result = execute_top_level_call(
        &mut journal,
        &BlockHashes,
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
    assert_eq!(journal.reader().code_reads.get(), 1_025);
    assert_eq!(
        journal
            .ordinary_storage(addresses[1_024], ConcreteStorageKey([0_u8; 32]))
            .unwrap()
            .1,
        BigUint::from(1_u8)
    );
}

fn reader_with_account(
    address: [u8; 20],
    code_hash: Option<[u8; 32]>,
    code_size: u64,
) -> MemoryReader {
    let mut reader = empty_reader();
    reader.accounts.insert(
        address,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash,
            code_size,
        },
    );
    reader
}

fn journal_with_program_and_unavailable_target(
    parent: Vec<u8>,
    unavailable_hash: [u8; 32],
) -> ExecutionJournal<MemoryReader> {
    let parent_address = [0xcc; 20];
    let parent_hash = keccak256(&parent).0;
    let mut reader = empty_reader();
    reader.accounts.insert(
        SENDER,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
            storage_root: None,
            code_hash: None,
            code_size: 0,
        },
    );
    reader.accounts.insert(
        parent_address,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash: Some(parent_hash),
            code_size: parent.len() as u64,
        },
    );
    reader.accounts.insert(
        TARGET,
        ReaderAccount {
            nonce: FinalChainNonce::from_u64(1),
            balance: ConcreteAccountBalance::default(),
            storage_root: None,
            code_hash: Some(unavailable_hash),
            code_size: 1,
        },
    );
    reader
        .codes
        .insert(parent_hash, ConcreteRead::Present(parent));
    ExecutionJournal::new(reader)
}

fn call_program(target: [u8; 20], value: u8) -> Vec<u8> {
    let mut code = hex::decode("6000600060006000").unwrap();
    code.extend_from_slice(&[0x60, value, 0x73]);
    code.extend_from_slice(&target);
    code.extend_from_slice(&[0x61, 0x03, 0xe8, 0xf1, 0x00]);
    code
}

fn chain_address(index: usize) -> [u8; 20] {
    let mut address = [0xdd; 20];
    address[12..].copy_from_slice(&(index as u64).to_be_bytes());
    address
}

fn empty_reader() -> MemoryReader {
    MemoryReader {
        accounts: BTreeMap::new(),
        storage: BTreeMap::new(),
        codes: BTreeMap::new(),
        code_error: None,
        code_reads: Cell::new(0),
    }
}

fn test_block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(1),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn call_transaction(receiver: [u8; 20]) -> ExecutionTransaction {
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(receiver),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/local.json")
}
