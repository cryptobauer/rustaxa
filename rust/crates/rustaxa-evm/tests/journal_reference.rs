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

#[test]
fn journal_matches_mutator_edge_oracle() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/journal_mutators_local.json"
    ))
    .expect("parse mutator fixture");
    let cases = fixture.as_array().expect("fixture array");
    assert_eq!(cases.len(), 9);

    for case in cases {
        let address: [u8; 20] = parse_hex(case["address"].as_str().unwrap())
            .try_into()
            .expect("20-byte address");
        let before = &case["before"];
        let raw_before = &before["raw"];
        let mut storage = BTreeMap::new();
        if raw_before["present"].as_bool().unwrap() {
            storage.insert(
                (address, KEY),
                parse_hex(raw_before["bytes"].as_str().unwrap()),
            );
        }
        let mut state = MemoryState {
            address,
            exists: before["exists"].as_bool().unwrap(),
            nonce: parse_nonce(&before["nonce"]),
            balance: ConcreteAccountBalance::new(parse_biguint(&before["balance"], "balance")),
            storage_root: before["exists"].as_bool().unwrap().then_some([0x66; 32]),
            code_hash: None,
            code_size: 0,
            codes: BTreeMap::new(),
            storage,
        };
        let code_rows = case["prior_rows"][0].as_object().unwrap();
        assert!(
            code_rows.len() <= 1,
            "mutator fixture needs explicit account code selection"
        );
        for (hash, code) in code_rows {
            let hash: [u8; 32] = parse_hex(hash).try_into().unwrap();
            let code = parse_hex(code.as_str().unwrap());
            state.code_hash = Some(hash);
            state.code_size = code.len() as u64;
            state.codes.insert(hash, code);
        }
        let mut journal = ExecutionJournal::new(state.clone());
        match case["case"].as_str().unwrap() {
            "storage-noop-leading-zero" => journal
                .set_ordinary_storage(address, KEY, BigUint::from(17_u8))
                .unwrap(),
            "empty-by-balance" => journal
                .set_balance(address, ExecutionBalance::default())
                .unwrap(),
            "existing-empty-touch" => journal.touch_account(address).unwrap(),
            "existing-empty-raw" => journal
                .set_raw_storage(
                    address,
                    KEY,
                    NativeRawOperation::Put(NativeRawValue::new(vec![0x44]).unwrap()),
                )
                .unwrap(),
            "empty-code-noop" => journal.set_code(address, Vec::new()).unwrap(),
            "reverted-new" => {
                let checkpoint = journal.checkpoint();
                journal
                    .set_nonce(address, FinalChainNonce::from_u64(1))
                    .unwrap();
                journal.revert_checkpoint(checkpoint).unwrap();
            }
            "nonce-decrease" => assert_eq!(
                journal.set_nonce(address, FinalChainNonce::zero()),
                Err(rustaxa_evm::journal::JournalError::NonceDecrease)
            ),
            "ripemd-touch-revert" => {
                let checkpoint = journal.checkpoint();
                journal.touch_account(address).unwrap();
                journal.revert_checkpoint(checkpoint).unwrap();
            }
            "empty-nonce-raw-revert" => {
                let checkpoint = journal.checkpoint();
                journal
                    .set_nonce(address, FinalChainNonce::from_u64(1))
                    .unwrap();
                journal
                    .set_raw_storage(
                        address,
                        KEY,
                        NativeRawOperation::Put(NativeRawValue::new(vec![0x44]).unwrap()),
                    )
                    .unwrap();
                journal.revert_checkpoint(checkpoint).unwrap();
            }
            other => panic!("unknown mutator case {other}"),
        }

        let settled = journal.settle_transaction().expect("settle mutator");
        if matches!(
            case["case"].as_str().unwrap(),
            "storage-noop-leading-zero" | "empty-code-noop" | "reverted-new" | "nonce-decrease"
        ) {
            assert_eq!(settled.writes, JournalWritePlan::default());
        }
        state.apply(settled.writes);
        let reached_code = state
            .code_hash
            .and_then(|hash| state.codes.get(&hash))
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            reached_code,
            parse_hex(case["reopened"]["code"].as_str().unwrap())
        );

        let reopened = ExecutionJournal::new(state);
        let expected = &case["reopened"];
        let account = reopened.account(address).unwrap();
        assert_eq!(account.exists, expected["exists"].as_bool().unwrap());
        assert_eq!(account.nonce, parse_nonce(&expected["nonce"]));
        assert_eq!(
            account.balance.value(),
            &BigInt::from(parse_biguint(&expected["balance"], "balance"))
        );
        let raw = reopened.raw_storage(address, KEY).unwrap();
        if expected["raw"]["present"].as_bool().unwrap() {
            assert_eq!(
                raw,
                ConcreteRead::Present(parse_hex(expected["raw"]["bytes"].as_str().unwrap()))
            );
        } else {
            assert!(matches!(
                raw,
                ConcreteRead::Absent | ConcreteRead::Tombstone
            ));
        }
    }
}

#[test]
fn journal_matches_reverse_nested_and_nil_root_oracle() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/journal_extended_local.json"
    ))
    .expect("parse extended fixture");
    let cases = fixture.as_array().expect("fixture array");
    assert_eq!(cases.len(), 12);

    for case in cases {
        let before = &case["before"];
        let mut storage = BTreeMap::new();
        if before["raw"]["present"].as_bool().unwrap() {
            storage.insert(
                (ADDRESS, KEY),
                parse_hex(before["raw"]["bytes"].as_str().unwrap()),
            );
        }
        let mut state = MemoryState {
            address: ADDRESS,
            exists: before["exists"].as_bool().unwrap(),
            nonce: parse_nonce(&before["nonce"]),
            balance: ConcreteAccountBalance::new(parse_biguint(&before["balance"], "balance")),
            storage_root: (before["exists"].as_bool().unwrap()
                && !case["nil_root"].as_bool().unwrap())
            .then_some([0x88; 32]),
            code_hash: None,
            code_size: 0,
            codes: BTreeMap::new(),
            storage,
        };
        let mut journal = ExecutionJournal::new(state.clone());
        assert_observation(&journal, before);
        let mut checkpoints = Vec::new();
        for step in case["steps"].as_array().unwrap() {
            match step["op"].as_str().unwrap() {
                "snapshot" => checkpoints.push(journal.checkpoint()),
                "revert" => journal
                    .revert_checkpoint(checkpoints.pop().expect("checkpoint for revert"))
                    .unwrap(),
                "nonce" => journal
                    .set_nonce(ADDRESS, parse_nonce(&step["value"]))
                    .unwrap(),
                "ordinary" => journal
                    .set_ordinary_storage(ADDRESS, KEY, parse_biguint(&step["value"], "ordinary"))
                    .unwrap(),
                "raw" => {
                    let value = parse_hex(step["value"].as_str().unwrap());
                    let operation = if value.is_empty() {
                        NativeRawOperation::Delete
                    } else {
                        NativeRawOperation::Put(NativeRawValue::new(value).unwrap())
                    };
                    journal.set_raw_storage(ADDRESS, KEY, operation).unwrap();
                }
                "transient" => {
                    let mut value = [0_u8; 32];
                    value[31] = u8::from_str_radix(step["value"].as_str().unwrap(), 16).unwrap();
                    journal.set_transient_storage(ADDRESS, KEY, value);
                }
                "refund" => journal.add_refund(step["value"].as_u64().unwrap()).unwrap(),
                "log" => journal.push_log(ExecutionLog {
                    address: ADDRESS,
                    topics: Vec::new(),
                    data: parse_hex(step["value"].as_str().unwrap()),
                }),
                other => panic!("unknown extended operation {other}"),
            }
            assert_observation(&journal, &step["view"]);
        }
        while let Some(checkpoint) = checkpoints.pop() {
            journal.commit_checkpoint(checkpoint).unwrap();
        }
        let settled = journal.settle_transaction().unwrap();
        assert_transaction_reset(&journal, &case["after_transaction"]);
        state.apply(settled.writes);
        assert_observation(&ExecutionJournal::new(state), &case["reopened"]);
    }
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

fn parse_nonce(value: &Value) -> FinalChainNonce {
    let value = parse_biguint(value, "nonce");
    if value == BigUint::default() {
        FinalChainNonce::zero()
    } else {
        FinalChainNonce::from_bytes(&value.to_bytes_be()).expect("canonical nonce")
    }
}

#[test]
fn installed_code_hash_matches_reference_code_row() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/journal_mutators_local.json"
    ))
    .unwrap();
    let case = fixtures
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["case"] == "empty-code-noop")
        .unwrap();
    let (hash, code) = case["prior_rows"][0]
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let expected_hash: [u8; 32] = parse_hex(hash).try_into().unwrap();
    let code = parse_hex(code.as_str().unwrap());
    let mut state = MemoryState::from_case(&serde_json::json!({"exists": false}));
    let mut journal = ExecutionJournal::new(state.clone());
    journal.set_code(ADDRESS, code.clone()).unwrap();
    let settled = journal.settle_transaction().unwrap();
    assert_eq!(settled.writes.code.len(), 1);
    assert_eq!(settled.writes.code[0].code_hash, expected_hash);
    assert_eq!(settled.writes.code[0].code, code);
    state.apply(settled.writes);
    assert!(state.exists);
    assert_eq!(state.code_hash, Some(expected_hash));
    assert_eq!(state.code_size, code.len() as u64);
    assert_eq!(
        state.code(expected_hash).unwrap(),
        ConcreteRead::Present(code)
    );
}
