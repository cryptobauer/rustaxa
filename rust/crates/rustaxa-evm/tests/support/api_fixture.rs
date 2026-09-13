//! Shared test-only materialization of the actual Go API seed rows.

use std::{cell::Cell, collections::BTreeMap};

use num_bigint::BigUint;
use rustaxa_evm::contracts::{
    ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
    ExecutionValue,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

#[derive(Clone)]
pub(crate) struct FixtureReader {
    pub(crate) identity: ConcreteStateIdentity,
    pub(crate) accounts: BTreeMap<[u8; 20], ConcreteAccountRecord>,
    pub(crate) storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    pub(crate) codes: BTreeMap<[u8; 32], Vec<u8>>,
    pub(crate) account_reads: Cell<usize>,
    pub(crate) account_error: Option<ConcreteReadError>,
}

impl FixtureReader {
    pub(crate) fn from_fixture(fixture: &Value) -> Self {
        let state = &fixture["state_before"];
        let mut accounts = BTreeMap::new();
        let mut codes = BTreeMap::new();
        for row in state["accounts"].as_array().expect("fixture accounts") {
            if !row["exists"].as_bool().expect("account existence") {
                continue;
            }
            let code_hash = optional_hash(&row["code_hash"]);
            if let Some(hash) = code_hash {
                codes.insert(hash, bytes(&row["code"]));
            }
            accounts.insert(
                address(&row["address"]),
                ConcreteAccountRecord {
                    account: ConcreteAccount {
                        nonce: nonce(&row["nonce"]),
                        balance: ConcreteAccountBalance::new(number(&row["balance"])),
                        storage_root: optional_hash(&row["storage_root"]),
                        code_hash,
                        code_size: row["code_size"].as_u64().unwrap_or_default(),
                    },
                    physical_rlp: bytes(&row["encoded"]),
                },
            );
        }
        let slot = &state["slot"];
        let storage = BTreeMap::from([(
            (
                address(&slot["address"]),
                ConcreteStorageKey(hash(&slot["key"])),
            ),
            bytes(&slot["value"]),
        )]);
        Self {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(state["period"].as_u64().expect("period")),
                state_root: hash(&state["root"]),
            },
            accounts,
            storage,
            codes,
            account_reads: Cell::new(0),
            account_error: None,
        }
    }
}

impl ConcreteStateRead for FixtureReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.account_reads.set(self.account_reads.get() + 1);
        if let Some(error) = &self.account_error {
            return Err(error.clone());
        }
        Ok(self
            .accounts
            .get(&address)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
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

pub(crate) fn fixture(reference: &str) -> Value {
    let source = match reference {
        "public" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/api_public.json"
        )),
        "local" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/api_local.json"
        )),
        _ => unreachable!(),
    };
    serde_json::from_str(source).expect("API fixture JSON")
}

pub(crate) fn block(fixture: &Value) -> ExecutionBlockContext {
    let config = &fixture["reference_config"];
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(config["period"].as_u64().unwrap()),
        author: [0_u8; 20],
        timestamp: config["timestamp"].as_u64().unwrap(),
        gas_limit: FinalChainGas::new(config["block_gas_limit"].as_u64().unwrap()),
        chain_id: config["chain_id"].as_u64().unwrap(),
        difficulty: number(&config["difficulty"]),
    }
}

pub(crate) fn transaction(case: &Value) -> ExecutionTransaction {
    let input = &case["input"];
    let receiver = input
        .get("to")
        .filter(|value| !value.is_null())
        .map(address);
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0_u8; 32],
        sender: address(&input["from"]),
        receiver,
        nonce: nonce(&input["supplied_nonce"]),
        gas_price: ExecutionGasPrice::new(number(&input["gas_price"])),
        gas_limit: FinalChainGas::new(input["gas"].as_u64().unwrap()),
        value: ExecutionValue::new(number(&input["value"])),
        input: bytes(&input["input"]),
        canonical_rlp: None,
        kind: if receiver.is_some() {
            ExecutionTransactionKind::Call
        } else {
            ExecutionTransactionKind::Create
        },
    }
}

fn optional_hash(value: &Value) -> Option<[u8; 32]> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(|_| hash(value))
}

pub(crate) fn address(value: &Value) -> [u8; 20] {
    hex::decode(value.as_str().expect("hex address"))
        .expect("address hex")
        .try_into()
        .expect("20-byte address")
}

pub(crate) fn hash(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().expect("hex hash"))
        .expect("hash hex")
        .try_into()
        .expect("32-byte hash")
}

pub(crate) fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex bytes")).expect("valid hex")
}

pub(crate) fn number(value: &Value) -> BigUint {
    number_str(value.as_str().expect("decimal number"))
}

pub(crate) fn number_str(value: &str) -> BigUint {
    BigUint::parse_bytes(value.as_bytes(), 10).expect("decimal integer")
}

pub(crate) fn nonce(value: &Value) -> FinalChainNonce {
    let number = number(value);
    let bytes = if number == BigUint::default() {
        Vec::new()
    } else {
        number.to_bytes_be()
    };
    FinalChainNonce::from_bytes(&bytes).expect("canonical nonce")
}

/// A disposable persisted fixture built from actual Go TrieSink rows.
pub(crate) struct PersistedApiFixture(pub(crate) std::path::PathBuf);

impl PersistedApiFixture {
    pub(crate) fn materialize(fixture: &Value) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustaxa-api-reopen-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        assert!(
            !path.exists(),
            "unique fixture path must not replace existing data"
        );
        let mut options = rocksdb::Options::default();
        options.create_if_missing(true);
        options.create_missing_column_families(true);
        let db = rocksdb::DB::open_cf(&options, &path, ["1", "2", "3", "4", "5", "6", "7", "8"])
            .unwrap();
        let period =
            FinalChainBlockNumber::new(fixture["state_before"]["period"].as_u64().unwrap());
        for row in fixture["state_before"]["seed_rows"].as_array().unwrap() {
            let column = match row["column"].as_u64().unwrap() {
                0 => "1",
                1 => "2",
                2 => "3",
                3 => "4",
                4 => "5",
                _ => panic!("unknown Go column"),
            };
            let key = if matches!(column, "3" | "5") {
                rustaxa_storage::versioned_key(hash(&row["key"]), period).to_vec()
            } else {
                bytes(&row["key"])
            };
            db.put_cf(&db.cf_handle(column).unwrap(), key, bytes(&row["value"]))
                .unwrap();
        }
        let mut descriptor = rlp::RlpStream::new_list(2);
        descriptor.append(&period.as_u64());
        descriptor.append(&bytes(&fixture["state_before"]["root"]).as_slice());
        db.put(b"last_committed_descriptor", descriptor.out())
            .unwrap();
        db.flush().unwrap();
        drop(db);
        Self(path)
    }

    pub(crate) fn append_newer_sender(&self, fixture: &Value) -> ConcreteStateIdentity {
        let original = FixtureReader::from_fixture(fixture);
        let sender = transaction(&fixture["cases"][0]).sender;
        let prior = &original.accounts[&sender];
        let original_rlp = rlp::Rlp::new(&prior.physical_rlp);
        let mut updated = rlp::RlpStream::new_list(5);
        for index in 0..5 {
            if index == 1 {
                updated.append(&0_u8);
            } else {
                updated.append_raw(original_rlp.at(index).unwrap().as_raw(), 1);
            }
        }
        let account = rustaxa_storage::decode_physical_account(&updated.out()).unwrap();
        let writer =
            rustaxa_storage::ConcreteStateWriter::open(&self.0, original.identity).unwrap();
        let prepared = writer
            .prepare(
                FinalChainBlockNumber::new(8),
                rustaxa_storage::ConcreteStateMutationBatch {
                    accounts: vec![rustaxa_storage::ConcreteAccountMutation::Upsert {
                        address: sender,
                        record: account,
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        let next = prepared.next_identity();
        assert_ne!(next.state_root, original.identity.state_root);
        writer.persist_contents(&prepared).unwrap();
        drop(writer);
        let db = rocksdb::DB::open_cf(
            &rocksdb::Options::default(),
            &self.0,
            ["1", "2", "3", "4", "5", "6", "7", "8"],
        )
        .unwrap();
        let mut descriptor = rlp::RlpStream::new_list(2);
        descriptor.append(&next.period.as_u64());
        descriptor.append(&next.state_root.as_slice());
        db.put(b"last_committed_descriptor", descriptor.out())
            .unwrap();
        db.flush().unwrap();
        next
    }

    pub(crate) fn rows(&self) -> Vec<(String, Vec<u8>, Vec<u8>)> {
        let options = rocksdb::Options::default();
        let columns = ["default", "1", "2", "3", "4", "5", "6", "7", "8"];
        let descriptors = columns
            .iter()
            .map(|name| rocksdb::ColumnFamilyDescriptor::new(*name, rocksdb::Options::default()));
        let db = rocksdb::DB::open_cf_descriptors_read_only(&options, &self.0, descriptors, false)
            .unwrap();
        let mut rows = Vec::new();
        for column in columns {
            for row in db.iterator_cf(&db.cf_handle(column).unwrap(), rocksdb::IteratorMode::Start)
            {
                let (key, value) = row.unwrap();
                rows.push((column.into(), key.to_vec(), value.to_vec()));
            }
        }
        rows
    }
}

impl Drop for PersistedApiFixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
