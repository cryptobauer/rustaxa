//! Direct checks against the additive pinned-Go journal oracle.
//!
//! The fixture proves one transaction and one checkpoint. It deliberately does
//! not claim same-block cache, nested-frame, RocksDB or asynchronous sink parity.

use std::{collections::BTreeMap, fs, path::PathBuf};

use num_bigint::{BigInt, BigUint};
use rustaxa_evm::{
    contracts::{
        ExecutionBalance, ExecutionLog, NativeOrdinaryAccountMutation, NativeRawOperation,
        NativeRawValue,
    },
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan, SettledTransaction},
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
    exists: bool,
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
}

impl MemoryState {
    fn from_case(case: &Value) -> Self {
        let mut storage = BTreeMap::new();
        if case["exists"].as_bool().expect("exists boolean") {
            storage.insert((ADDRESS, KEY), vec![0x11]);
        }
        Self {
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
            storage,
        }
    }

    fn apply(&mut self, plan: JournalWritePlan) {
        for write in plan.accounts {
            match write.operation {
                JournalAccountOperation::Upsert { nonce, balance, .. } => {
                    self.exists = true;
                    self.nonce = nonce;
                    self.balance = balance;
                }
                JournalAccountOperation::Delete => {
                    self.exists = false;
                    self.nonce = FinalChainNonce::zero();
                    self.balance = ConcreteAccountBalance::default();
                    self.storage
                        .retain(|(address, _), _| address != &write.address);
                }
            }
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
        if address != ADDRESS || !self.exists {
            return Ok(ConcreteRead::Absent);
        }
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: self.nonce.clone(),
                balance: self.balance.clone(),
                storage_root: None,
                code_hash: None,
                code_size: 0,
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

    fn code(&self, _code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
}

#[test]
fn journal_matches_single_checkpoint_go_oracle() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/journal_local.json");
    let fixture: Value = serde_json::from_slice(&fs::read(fixture_path).expect("read fixture"))
        .expect("parse fixture");
    let cases = fixture.as_array().expect("fixture array");
    assert_eq!(cases.len(), 10);

    for case in cases {
        let mut state = MemoryState::from_case(case);
        let mut journal = ExecutionJournal::new(state.clone());
        assert_observation(&journal, &case["before"]);

        let checkpoint = journal.checkpoint();
        journal.touch_account(ADDRESS).expect("touch account");
        if case["keep_new"].as_bool().expect("keep_new boolean") {
            journal
                .set_nonce(ADDRESS, FinalChainNonce::from_u64(1))
                .expect("set nonce");
        }
        journal
            .set_ordinary_storage(
                ADDRESS,
                KEY,
                parse_biguint(&case["ordinary_write"], "ordinary_write"),
            )
            .expect("set ordinary storage");
        let raw = parse_hex(case["raw_write"].as_str().expect("raw_write string"));
        let raw_operation = if raw.is_empty() {
            NativeRawOperation::Delete
        } else {
            NativeRawOperation::Put(NativeRawValue::new(raw).expect("nonempty raw put"))
        };
        journal
            .set_raw_storage(ADDRESS, KEY, raw_operation)
            .expect("set raw storage");
        let mut transient = [0_u8; 32];
        transient[31] = 0x55;
        journal.set_transient_storage(ADDRESS, KEY, transient);
        journal.push_log(ExecutionLog {
            address: ADDRESS,
            topics: Vec::new(),
            data: vec![0xaa],
        });
        journal.add_refund(123).expect("add refund");

        assert_observation(&journal, &case["after_writes"]);
        if case["revert"].as_bool().expect("revert boolean") {
            journal
                .revert_checkpoint(checkpoint)
                .expect("revert checkpoint");
        } else {
            journal
                .commit_checkpoint(checkpoint)
                .expect("commit checkpoint");
        }
        assert_observation(&journal, &case["after_frame"]);

        let SettledTransaction { writes, .. } =
            journal.settle_transaction().expect("settle transaction");
        assert_transaction_reset(&journal, &case["after_transaction"]);
        state.apply(writes);
        let reopened = ExecutionJournal::new(state);
        assert_observation(&reopened, &case["reopened"]);
    }
}

#[test]
fn nested_commit_remains_revertible_while_raw_and_transient_survive() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/journal_local.json"
    ))
    .expect("parse fixture");
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
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/journal_local.json"
    ))
    .expect("parse fixture");
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

fn assert_observation(journal: &ExecutionJournal<MemoryState>, expected: &Value) {
    let account = journal.account(ADDRESS).expect("read account");
    assert_eq!(account.exists, expected["exists"].as_bool().unwrap());
    assert_eq!(
        BigUint::from_bytes_be(&account.nonce.to_bytes()),
        parse_biguint(&expected["nonce"], "nonce")
    );
    assert_eq!(
        account.balance.value(),
        &num_bigint::BigInt::from(parse_biguint(&expected["balance"], "balance"))
    );
    let (original, current) = journal
        .ordinary_storage(ADDRESS, KEY)
        .expect("read ordinary storage");
    assert_eq!(original, parse_biguint(&expected["original"], "original"));
    assert_eq!(current, parse_biguint(&expected["ordinary"], "ordinary"));

    let raw = journal.raw_storage(ADDRESS, KEY).expect("read raw storage");
    let raw_expected = &expected["raw"];
    if raw_expected["present"].as_bool().unwrap() {
        assert_eq!(
            raw,
            ConcreteRead::Present(parse_hex(raw_expected["bytes"].as_str().unwrap()))
        );
    } else {
        assert!(matches!(
            raw,
            ConcreteRead::Absent | ConcreteRead::Tombstone
        ));
    }
    assert_eq!(
        journal.transient_storage(ADDRESS, KEY),
        parse_word(expected["transient"].as_str().unwrap())
    );
    assert_eq!(
        journal.logs().len() as u64,
        expected["logs"].as_u64().unwrap()
    );
    assert_eq!(journal.refund(), expected["refund"].as_u64().unwrap());
}

fn assert_transaction_reset(journal: &ExecutionJournal<MemoryState>, expected: &Value) {
    assert_eq!(
        journal.transient_storage(ADDRESS, KEY),
        parse_word(expected["transient"].as_str().unwrap())
    );
    assert_eq!(
        journal.logs().len() as u64,
        expected["logs"].as_u64().unwrap()
    );
    assert_eq!(journal.refund(), expected["refund"].as_u64().unwrap());
}

fn parse_biguint(value: &Value, field: &str) -> BigUint {
    BigUint::parse_bytes(value.as_str().expect(field).as_bytes(), 10).expect(field)
}

fn parse_word(value: &str) -> [u8; 32] {
    parse_hex(value).try_into().expect("32-byte word")
}

fn parse_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex utf8");
            u8::from_str_radix(text, 16).expect("hex byte")
        })
        .collect()
}
