//! Pinned Go SELFDESTRUCT frame, refund, visibility and logical settlement parity.

use std::{collections::BTreeMap, fs, path::PathBuf};

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, NativeRawOperation, NativeRawValue, TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call, execute_top_level_create},
    envelope::EnvelopeRules,
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
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

const SENDER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
];
const TARGET: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbb,
];
const CHILD: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xcc,
];
const KEY: ConcreteStorageKey = ConcreteStorageKey([0_u8; 32]);

struct Reader {
    accounts: BTreeMap<[u8; 20], ConcreteAccount>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
}

impl ConcreteStateRead for Reader {
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
        Ok(self
            .codes
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}

struct NoHistory;

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

impl BlockHashRead for NoHistory {
    fn block_hash(&self, _number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("predicate bytecode does not execute BLOCKHASH")
    }
}

#[test]
fn frames_and_settlement_match_both_pinned_go_references() {
    let public: Value = serde_json::from_slice(&fs::read(fixture_path("public")).unwrap()).unwrap();
    let local: Value = serde_json::from_slice(&fs::read(fixture_path("local")).unwrap()).unwrap();
    assert_eq!(public, local);
    let cases = public["selfdestruct"].as_array().unwrap();
    assert_eq!(cases.len(), 21);
    for case in cases {
        let name = case["case"].as_str().unwrap();
        let dest: [u8; 20] = hex::decode(case["beneficiary"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let parent = hex::decode(case["code"].as_str().unwrap()).unwrap();
        let child = hex::decode(case["child_code"].as_str().unwrap()).unwrap();
        let source = if child.is_empty() { TARGET } else { CHILD };
        let mut accounts = BTreeMap::from([(SENDER, account(1, BigUint::from(1_000_000_u64)))]);
        let mut codes = BTreeMap::new();
        for (address, code) in [(TARGET, parent.clone()), (CHILD, child)] {
            if code.is_empty() {
                continue;
            }
            let hash = keccak256(&code).0;
            let mut value = account(
                1,
                if address == source {
                    decimal(&case["balance"])
                } else {
                    BigUint::default()
                },
            );
            value.code_hash = Some(hash);
            value.code_size = code.len() as u64;
            value.storage_root = Some([0x55; 32]);
            accounts.insert(address, value);
            codes.insert(hash, code);
        }
        let kind = case["beneficiary_kind"].as_str().unwrap();
        if kind != "absent" && dest != source {
            accounts.insert(
                dest,
                account(u64::from(kind == "nonempty"), BigUint::default()),
            );
        }
        let mut journal = ExecutionJournal::new(Reader {
            accounts,
            codes,
            storage: BTreeMap::from([((TARGET, KEY), vec![7])]),
        });
        let mut tx = transaction();
        tx.gas_limit = FinalChainGas::new(case["gas_limit"].as_u64().unwrap());
        if name == "create-suicide" {
            tx.receiver = None;
            tx.kind = ExecutionTransactionKind::Create;
            tx.input = parent;
            tx.value = ExecutionValue::new(BigUint::from(7_u8));
        }
        let execute = if name == "create-suicide" {
            execute_top_level_create
        } else {
            execute_top_level_call
        };
        let result = execute(
            &mut journal,
            &NoHistory,
            &NoNative,
            &block(),
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(result) = result else {
            panic!("{name}: rejected")
        };
        if let Some(address) = case.get("created_address") {
            assert_eq!(
                result
                    .attempted_contract_address
                    .map(hex::encode)
                    .as_deref(),
                address.as_str(),
                "{name}: created address"
            );
        }
        assert_eq!(case["consensus_error"], "", "{name}");
        assert_eq!(case["error"], case["execution_error"], "{name}");
        let expected = match case["execution_error"].as_str().unwrap() {
            "" => CodeExecutionStatus::Success,
            "stack underflow (0 <=> 1)" => {
                CodeExecutionStatus::Failure(CodeExecutionError::StackUnderflow)
            }
            "out of gas" => CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas),
            "execution reverted" => CodeExecutionStatus::Failure(CodeExecutionError::Revert),
            other => panic!("{name}: {other}"),
        };
        assert_eq!(result.status, expected, "{name}: status");
        assert_eq!(
            result.gas_used.as_u64(),
            case["gas_used"].as_u64().unwrap(),
            "{name}: gas"
        );
        assert_eq!(
            result.output,
            hex::decode(case["output"].as_str().unwrap()).unwrap(),
            "{name}: output"
        );
        assert!(result.logs.is_empty(), "{name}: logs");
        assert_eq!(
            journal.refund(),
            case["refund"].as_u64().unwrap(),
            "{name}: refund"
        );
        for (address, expected) in case["visible"].as_object().unwrap() {
            let address: [u8; 20] = hex::decode(address).unwrap().try_into().unwrap();
            let value = journal.account_metadata(address).unwrap();
            assert_eq!(
                serde_json::json!({"exists":value.exists,"nonce":BigUint::from_bytes_be(&value.nonce.to_bytes()).to_string(),"balance":value.balance.value().to_string(),"code_size":value.code_size}),
                *expected,
                "{name}: visible {}",
                hex::encode(address)
            );
        }
        let settled = journal.settle_transaction().unwrap();
        assert_eq!(
            write_json(&settled.writes),
            case["writes"],
            "{name}: settlement"
        );
        assert!(
            settled.writes.code.is_empty(),
            "{name}: no deleted code publication"
        );
        assert!(settled.writes.ordinary_storage.is_empty(), "{name}");
        assert!(settled.writes.raw_storage.is_empty(), "{name}");
        assert!(settled.native_invocations.is_empty(), "{name}");
    }
}

#[test]
fn suicide_lifetimes_match_go_transaction_flush() {
    let public: Value = serde_json::from_slice(&fs::read(fixture_path("public")).unwrap()).unwrap();
    for case in public["lanes"].as_array().unwrap() {
        let name = case["case"].as_str().unwrap();
        let exists = case["exists"].as_bool().unwrap();
        let mut source = account(1, BigUint::from(7_u8));
        source.storage_root = Some([0x55; 32]);
        let mut journal = ExecutionJournal::new(Reader {
            accounts: if exists {
                BTreeMap::from([(TARGET, source)])
            } else {
                BTreeMap::new()
            },
            codes: BTreeMap::new(),
            storage: BTreeMap::from([((TARGET, KEY), vec![7])]),
        });
        let beneficiary: [u8; 20] = hex::decode(case["beneficiary"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let mut raw_key = [0; 32];
        raw_key[31] = 1;
        let raw_key = ConcreteStorageKey(raw_key);
        let mut transient = [0; 32];
        transient[31] = 9;
        let checkpoint = journal.checkpoint();
        if exists || name == "new-account-revert" || name == "new-account-recreate" {
            journal
                .set_ordinary_storage(TARGET, KEY, BigUint::from(8_u8))
                .unwrap();
            if name != "ordinary-only-revert" && name != "revert-then-nonce" {
                journal
                    .set_raw_storage(
                        TARGET,
                        raw_key,
                        NativeRawOperation::Put(NativeRawValue::new(vec![0xaa, 0xbb]).unwrap()),
                    )
                    .unwrap();
            }
            journal.set_transient_storage(TARGET, KEY, transient);
        }
        journal.selfdestruct(TARGET, beneficiary).unwrap();
        if case["revert"].as_bool().unwrap() {
            journal.revert_checkpoint(checkpoint).unwrap();
        } else {
            journal.commit_checkpoint(checkpoint).unwrap();
        }
        if name == "revert-then-nonce" || name == "new-account-recreate" {
            journal
                .set_nonce(TARGET, FinalChainNonce::from_u64(2))
                .unwrap();
        }
        let source = journal.account_metadata(TARGET).unwrap();
        let dest = journal.account_metadata(beneficiary).unwrap();
        assert_eq!(
            source.exists,
            case["source_exists"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(
            source.balance.value().to_string(),
            case["source_balance"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            dest.exists,
            case["beneficiary_exists"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(
            dest.balance.value().to_string(),
            case["beneficiary_balance"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            journal.ordinary_storage(TARGET, KEY).unwrap().1,
            decimal(&case["storage"]),
            "{name}"
        );
        let raw = match journal.raw_storage(TARGET, raw_key).unwrap() {
            ConcreteRead::Present(v) => v,
            ConcreteRead::Absent | ConcreteRead::Tombstone => vec![],
        };
        assert_eq!(hex::encode(raw), case["raw"].as_str().unwrap(), "{name}");
        assert_eq!(
            format!("0x{}", hex::encode(journal.transient_storage(TARGET, KEY))),
            case["transient"].as_str().unwrap(),
            "{name}"
        );
        let settled = journal.settle_transaction().unwrap();
        assert_eq!(
            write_json(&settled.writes),
            case["writes"],
            "{name}: writes"
        );
        assert_eq!(
            journal.transient_storage(TARGET, KEY),
            [0; 32],
            "{name}: transient reset"
        );
    }
}

#[test]
fn beneficiary_quote_needs_only_emptiness_metadata() {
    let mut destination = [0; 20];
    destination[19] = 0xdd;
    let mut code = vec![0x73];
    code.extend_from_slice(&destination);
    code.push(0xff);
    let hash = keccak256(&code).0;
    let mut source = account(1, BigUint::default());
    source.code_hash = Some(hash);
    source.code_size = code.len() as u64;
    // The beneficiary's code reference is deliberately incomplete. SELFDESTRUCT
    // only needs its semantic nonemptiness; EXTCODE operations stay strict.
    let mut beneficiary = account(0, BigUint::default());
    beneficiary.code_size = 1;
    let mut journal = ExecutionJournal::new(Reader {
        accounts: BTreeMap::from([
            (SENDER, account(1, BigUint::from(1_000_000_u64))),
            (TARGET, source),
            (destination, beneficiary),
        ]),
        codes: BTreeMap::from([(hash, code)]),
        storage: BTreeMap::new(),
    });
    let TransactionExecutionResult::Executed(result) = execute_top_level_call(
        &mut journal,
        &NoHistory,
        &NoNative,
        &block(),
        &transaction(),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap() else {
        panic!("must execute")
    };
    assert_eq!(result.status, CodeExecutionStatus::Success);
    assert_eq!(result.gas_used.as_u64(), 13_002);
    let settled = journal.settle_transaction().unwrap();
    assert!(
        !settled
            .writes
            .accounts
            .iter()
            .any(|write| write.address == destination)
    );
}

#[test]
fn signed_suicide_transfer_and_self_transfer_restore_on_revert() {
    use num_bigint::BigInt;
    use rustaxa_evm::contracts::ExecutionBalance;
    let mut destination = [0; 20];
    destination[19] = 0xdd;
    for beneficiary in [destination, TARGET] {
        let mut journal = ExecutionJournal::new(Reader {
            accounts: BTreeMap::from([
                (TARGET, account(1, BigUint::from(7_u8))),
                (destination, account(1, BigUint::from(10_u8))),
            ]),
            codes: BTreeMap::new(),
            storage: BTreeMap::new(),
        });
        journal
            .set_balance(TARGET, ExecutionBalance::new(BigInt::from(-7)))
            .unwrap();
        let checkpoint = journal.checkpoint();
        journal.selfdestruct(TARGET, beneficiary).unwrap();
        assert_eq!(
            journal.account_metadata(TARGET).unwrap().balance.value(),
            &BigInt::from(0)
        );
        if beneficiary == destination {
            assert_eq!(
                journal
                    .account_metadata(destination)
                    .unwrap()
                    .balance
                    .value(),
                &BigInt::from(3)
            );
        }
        journal.revert_checkpoint(checkpoint).unwrap();
        assert_eq!(
            journal.account_metadata(TARGET).unwrap().balance.value(),
            &BigInt::from(-7)
        );
        assert_eq!(
            journal
                .account_metadata(destination)
                .unwrap()
                .balance
                .value(),
            &BigInt::from(10)
        );
    }
}

fn write_json(plan: &JournalWritePlan) -> Value {
    let mut writes = serde_json::Map::new();
    for write in &plan.accounts {
        let value = match &write.operation {
            JournalAccountOperation::Delete => serde_json::json!({"kind":"delete"}),
            JournalAccountOperation::Upsert {
                nonce,
                balance,
                code_size,
                ..
            } => {
                serde_json::json!({"kind":"upsert","nonce":BigUint::from_bytes_be(&nonce.to_bytes()).to_string(),"balance":balance.value().to_string(),"code_size":code_size,"storage":{},"raw":{}})
            }
        };
        writes.insert(hex::encode(write.address), value);
    }
    for write in &plan.ordinary_storage {
        let key = write.key.0;
        let start = key.iter().position(|b| *b != 0).unwrap_or(key.len());
        writes.get_mut(&hex::encode(write.address)).unwrap()["storage"]
            [hex::encode(&key[start..])] = Value::String(write.value.to_string());
    }
    for write in &plan.raw_storage {
        let bytes = match &write.operation {
            NativeRawOperation::Put(value) => value.as_bytes(),
            NativeRawOperation::Delete => &[],
        };
        writes.get_mut(&hex::encode(write.address)).unwrap()["raw"][hex::encode(write.key.0)] =
            Value::String(hex::encode(bytes));
    }
    Value::Object(writes)
}

fn account(nonce: u64, balance: BigUint) -> ConcreteAccount {
    ConcreteAccount {
        nonce: FinalChainNonce::from_u64(nonce),
        balance: ConcreteAccountBalance::new(balance),
        storage_root: None,
        code_hash: None,
        code_size: 0,
    }
}

fn decimal(value: &Value) -> BigUint {
    BigUint::parse_bytes(value.as_str().unwrap().as_bytes(), 10).unwrap()
}

fn fixture_path(reference: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../../experiments/evm_feasibility/fixtures/selfdestruct_{reference}.json"
    ))
}

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(1),
        author: [0_u8; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn transaction() -> ExecutionTransaction {
    ExecutionTransaction {
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
    }
}
