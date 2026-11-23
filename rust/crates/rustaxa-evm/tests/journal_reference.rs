//! Direct checks against the additive pinned-Go journal oracle.
//!
//! The fixture proves one transaction and one checkpoint. It deliberately does
//! not claim same-block cache, nested-frame, RocksDB or asynchronous sink parity.

use std::collections::BTreeMap;

use num_bigint::{BigInt, BigUint};
use rustaxa_evm::{
    contracts::{
        ExecutionBalance, NativeOrdinaryAccountMutation, NativeRawOperation, NativeRawValue,
    },
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

const ADDRESS: [u8; 20] = [0x11; 20];
const KEY: ConcreteStorageKey = ConcreteStorageKey({
    let mut key = [0_u8; 32];
    key[31] = 1;
    key
});

#[derive(Clone)]
struct MemoryState {
    address: [u8; 20],
    exists: bool,
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    storage_root: Option<[u8; 32]>,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
}

impl MemoryState {
    fn from_case(case: &Value) -> Self {
        let mut storage = BTreeMap::new();
        if case["exists"].as_bool().expect("exists boolean") {
            storage.insert((ADDRESS, KEY), vec![0x11]);
        }
        Self {
            address: ADDRESS,
            exists: case["exists"].as_bool().expect("exists boolean"),
            nonce: FinalChainNonce::from_u64(if case["exists"].as_bool().unwrap() {
                1
            } else {
                0
            }),
            balance: ConcreteAccountBalance::new(if case["exists"].as_bool().unwrap() {
                BigUint::from(100_u8)
            } else {
                BigUint::default()
            }),
            storage_root: case["exists"].as_bool().unwrap().then_some([0x33; 32]),
            code_hash: None,
            code_size: 0,
            codes: BTreeMap::new(),
            storage,
        }
    }

    fn apply(&mut self, plan: JournalWritePlan) {
        for write in plan.accounts {
            match write.operation {
                JournalAccountOperation::Upsert {
                    nonce,
                    balance,
                    code_hash,
                    code_size,
                } => {
                    self.exists = true;
                    self.nonce = nonce;
                    self.balance = balance;
                    self.code_hash = code_hash;
                    self.code_size = code_size;
                }
                JournalAccountOperation::Delete => {
                    self.exists = false;
                    self.nonce = FinalChainNonce::zero();
                    self.balance = ConcreteAccountBalance::default();
                    self.storage_root = None;
                    self.code_hash = None;
                    self.code_size = 0;
                    self.storage
                        .retain(|(address, _), _| address != &write.address);
                }
            }
        }
        for write in plan.code {
            self.codes.insert(write.code_hash, write.code);
        }
        for write in plan.ordinary_storage {
            if write.value == BigUint::default() {
                self.storage.remove(&(write.address, write.key));
            } else {
                self.storage
                    .insert((write.address, write.key), write.value.to_bytes_be());
            }
        }
        for write in plan.raw_storage {
            match write.operation {
                NativeRawOperation::Put(value) => {
                    self.storage
                        .insert((write.address, write.key), value.into_bytes());
                }
                NativeRawOperation::Delete => {
                    self.storage.remove(&(write.address, write.key));
                }
            }
        }
        if self.exists {
            self.storage_root = (!self.storage.is_empty()).then_some([0x44; 32]);
        }
    }
}

impl ConcreteStateRead for MemoryState {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(1),
            state_root: [0x22; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if address != self.address || !self.exists {
            return Ok(ConcreteRead::Absent);
        }
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: self.nonce.clone(),
                balance: self.balance.clone(),
                storage_root: self.storage_root,
                code_hash: self.code_hash,
                code_size: self.code_size,
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

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .codes
            .get(&code_hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}

#[test]
fn nested_commit_remains_revertible_while_raw_and_transient_survive() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/journal_local.json")).expect("parse fixture");
    let state = MemoryState::from_case(&fixture[0]);
    let mut journal = ExecutionJournal::new(state);
    let outer = journal.checkpoint();
    journal
        .set_ordinary_storage(ADDRESS, KEY, BigUint::from(2_u8))
        .unwrap();
    let inner = journal.checkpoint();
    journal
        .set_ordinary_storage(ADDRESS, KEY, BigUint::from(3_u8))
        .unwrap();
    journal
        .set_raw_storage(
            ADDRESS,
            KEY,
            NativeRawOperation::Put(NativeRawValue::new(vec![4]).unwrap()),
        )
        .unwrap();
    let mut transient = [0_u8; 32];
    transient[31] = 5;
    journal.set_transient_storage(ADDRESS, KEY, transient);
    journal.commit_checkpoint(inner).unwrap();
    journal.revert_checkpoint(outer).unwrap();

    assert_eq!(
        journal.ordinary_storage(ADDRESS, KEY).unwrap(),
        (BigUint::from(17_u8), BigUint::from(17_u8))
    );
    assert_eq!(
        journal.raw_storage(ADDRESS, KEY).unwrap(),
        ConcreteRead::Present(vec![4])
    );
    assert_eq!(journal.transient_storage(ADDRESS, KEY), transient);
}

#[test]
fn ordered_native_touch_establishes_account_before_nonce_effect() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/journal_local.json")).expect("parse fixture");
    let mut state = MemoryState::from_case(&fixture[2]);
    let mut journal = ExecutionJournal::new(state.clone());
    journal
        .apply_native_account_mutations(&[
            NativeOrdinaryAccountMutation::Touch {
                address: ADDRESS,
                expected_exists: false,
            },
            NativeOrdinaryAccountMutation::Balance {
                address: ADDRESS,
                expected_exists: true,
                expected: ExecutionBalance::default(),
                replacement: ExecutionBalance::new(BigInt::default()),
            },
            NativeOrdinaryAccountMutation::Nonce {
                address: ADDRESS,
                expected_exists: true,
                expected: FinalChainNonce::zero(),
                replacement: FinalChainNonce::from_u64(1),
            },
        ])
        .expect("apply ordered native effects");
    let settled = journal.settle_transaction().expect("settle native effects");
    state.apply(settled.writes);
    assert!(state.exists);
    assert_eq!(state.nonce, FinalChainNonce::from_u64(1));
}

#[test]
fn native_ensure_exists_preserves_zero_debit_lifecycle() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/journal_local.json")).unwrap();
    for exists in [false, true] {
        for reverted in [false, true] {
            let mut state = MemoryState::from_case(&fixture[0]);
            state.exists = exists;
            state.nonce = FinalChainNonce::zero();
            state.balance = ConcreteAccountBalance::default();
            state.storage_root = None;
            state.storage.clear();
            // RIPEMD makes an accidental zero-credit touch visible even on rollback.
            state.address = [0; 20];
            state.address[19] = 3;
            let address = state.address;
            let mut native = ExecutionJournal::new(state.clone());
            let checkpoint = native.checkpoint();
            native
                .apply_native_account_mutations(&[NativeOrdinaryAccountMutation::EnsureExists {
                    address,
                    expected_exists: exists,
                }])
                .unwrap();
            assert!(native.account_metadata(address).unwrap().exists);
            if reverted {
                native.revert_checkpoint(checkpoint).unwrap();
            } else {
                native.commit_checkpoint(checkpoint).unwrap();
            }
            let result = native.settle_transaction().unwrap();
            if exists || reverted {
                assert!(
                    result.writes.accounts.is_empty(),
                    "existing={exists}, reverted={reverted}"
                );
            } else {
                assert_eq!(result.writes.accounts.len(), 1);
                assert_eq!(
                    result.writes.accounts[0].operation,
                    JournalAccountOperation::Delete
                );
            }
            let mut debit = ExecutionJournal::new(state.clone());
            let checkpoint = debit.checkpoint();
            debit
                .subtract_balance(address, &BigUint::default())
                .unwrap();
            if reverted {
                debit.revert_checkpoint(checkpoint).unwrap();
            } else {
                debit.commit_checkpoint(checkpoint).unwrap();
            }
            assert_eq!(result, debit.settle_transaction().unwrap());
            let mut stale = ExecutionJournal::new(state);
            assert!(
                stale
                    .apply_native_account_mutations(&[
                        NativeOrdinaryAccountMutation::EnsureExists {
                            address,
                            expected_exists: !exists
                        }
                    ])
                    .is_err()
            );
            assert_eq!(stale.account_metadata(address).unwrap().exists, exists);
            assert!(
                stale
                    .settle_transaction()
                    .unwrap()
                    .writes
                    .accounts
                    .is_empty()
            );
        }
    }
}
