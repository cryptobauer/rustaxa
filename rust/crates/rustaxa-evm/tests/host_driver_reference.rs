//! Journal-host and first complete top-level execution checks.

use std::{cell::Cell, collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::{
    context_interface::Host,
    primitives::{Address, B256, Log, U256, keccak256},
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
        TransactionExecutionResult,
    },
    driver::execute_top_level_call,
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

fn empty_reader() -> MemoryReader {
    MemoryReader {
        accounts: BTreeMap::new(),
        storage: BTreeMap::new(),
        codes: BTreeMap::new(),
        code_error: None,
        code_reads: Cell::new(0),
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/local.json")
}
