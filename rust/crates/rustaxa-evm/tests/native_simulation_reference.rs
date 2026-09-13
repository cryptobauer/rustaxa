//! Actual Go DryRunner parity over persisted seed rows and Rust native snapshots.
//!
//! The synthetic native history is established through public Rust finalization,
//! independently of the Go concrete seed. Only DPoS semantic history is shared
//! with simulation; authoritative ordinary accounts/code/raw reads come from the
//! exact Go trie. This fixture does not adopt an existing-network checkpoint.

#[allow(dead_code)]
#[path = "support/mixed_native.rs"]
mod mixed_native;

use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

use ethereum_types::{H256, U256};
use k256::ecdsa::SigningKey;
use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::FinalChain;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    profile::{TaraxaPhase, TaraxaProfile},
    simulation::simulate_with_native,
};
use rustaxa_storage::{Column, ConcreteStateReader, Config, Storage};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce, FinalChainRewardsConfig, FinalizationTransaction,
    GenesisAccount, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
    concrete_state::{
        ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
        ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::{Value, json};

fn address(last: u8) -> [u8; 20] {
    let mut result = [0; 20];
    result[19] = last;
    result
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn number(value: &Value) -> BigUint {
    BigUint::parse_bytes(value.as_str().unwrap().as_bytes(), 10).unwrap()
}

struct FixturePath(PathBuf);

impl FixturePath {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustaxa-native-dry-run-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        assert!(!path.exists());
        Self(path)
    }
}

impl Drop for FixturePath {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn materialize(fixture: &Value, path: &FixturePath) -> ConcreteStateIdentity {
    let mut options = rocksdb::Options::default();
    options.create_if_missing(true);
    options.create_missing_column_families(true);
    let db =
        rocksdb::DB::open_cf(&options, &path.0, ["1", "2", "3", "4", "5", "6", "7", "8"]).unwrap();
    for row in fixture["state_before"]["seed_rows"].as_array().unwrap() {
        let column = (row["column"].as_u64().unwrap() + 1).to_string();
        // The Go exporter already includes exact big-endian period suffixes.
        db.put_cf(
            &db.cf_handle(&column).unwrap(),
            bytes(&row["key"]),
            bytes(&row["value"]),
        )
        .unwrap();
    }
    let identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(fixture["state_before"]["period"].as_u64().unwrap()),
        state_root: bytes(&fixture["state_before"]["root"]).try_into().unwrap(),
    };
    let mut descriptor = RlpStream::new_list(2);
    descriptor.append(&identity.period.as_u64());
    descriptor.append(&identity.state_root.as_slice());
    // Synthetic fixture setup only; this is not checkpoint adoption authority.
    db.put(b"last_committed_descriptor", descriptor.out())
        .unwrap();
    db.flush().unwrap();
    identity
}

fn concrete_rows(path: &FixturePath) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let db = rocksdb::DB::open_cf_for_read_only(
        &rocksdb::Options::default(),
        &path.0,
        ["1", "2", "3", "4", "5", "6", "7", "8"],
        false,
    )
    .unwrap();
    let mut result = Vec::new();
    for column in ["default", "1", "2", "3", "4", "5", "6", "7", "8"] {
        for row in db.iterator_cf(&db.cf_handle(column).unwrap(), rocksdb::IteratorMode::Start) {
            let (key, value) = row.unwrap();
            result.push((column.into(), key.to_vec(), value.to_vec()));
        }
    }
    result
}

/// Absence authority exists only for this complete, unpruned Go-generated seed.
/// A listed physical row that disappears still fails, and unlisted logical
/// membership must be disproved against the authenticated root before absence.
/// This capability cannot be obtained from the light-node checkpoint inventory.
struct CompleteSeedReader {
    inner: ConcreteStateReader,
    storage_prefixes: BTreeSet<[u8; 32]>,
}

impl CompleteSeedReader {
    fn open(path: &FixturePath, fixture: &Value, identity: ConcreteStateIdentity) -> Self {
        Self {
            inner: ConcreteStateReader::open_read_only(&path.0, identity).unwrap(),
            storage_prefixes: fixture["state_before"]["seed_rows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["column"] == 4)
                .map(|row| bytes(&row["key"])[..32].try_into().unwrap())
                .collect(),
        }
    }
}

impl ConcreteStateRead for CompleteSeedReader {
    fn identity(&self) -> ConcreteStateIdentity {
        self.inner.identity()
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.inner.account(address)
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        match self.inner.storage(address, key) {
            Err(ConcreteReadError::HistoryUnavailable(_))
                if !self
                    .storage_prefixes
                    .contains(&rustaxa_storage::storage_version_prefix(address, key)) =>
            {
                match self.inner.verify_storage_path(address, key)? {
                    rustaxa_storage::ConcreteStoragePath::NonMember => Ok(ConcreteRead::Absent),
                    rustaxa_storage::ConcreteStoragePath::Member(_) => {
                        Err(ConcreteReadError::HistoryUnavailable(self.identity()))
                    }
                }
            }
            result => result,
        }
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.inner.code(hash)
    }
}

fn native_history(path: &FixturePath) -> FinalChain {
    let storage = Arc::new(Storage::new(Config::new(path.0.clone())).unwrap());
    let final_chain = FinalChain::new_with_rewards_config(
        storage.clone(),
        500_000.into(),
        0,
        vec![GenesisAccount {
            address: address(0xbb),
            balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(1_000_000)),
        }],
        vec![GenesisValidator {
            address: address(0x31),
            vrf_key: [0x44; 32],
            total_stake: U256::from(90).to_big_endian().to_vec(),
            delegations: vec![(address(0xbb), U256::from(90).to_big_endian().to_vec())],
            metadata: GenesisValidatorMetadata {
                owner: address(0xaa),
                commission: 100,
                ..Default::default()
            },
        }],
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::one().into(),
            delegation_delay: 1,
            commission_change_delta: 0,
            commission_change_frequency: 0,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            fix_claim_all_block_num: 0.into(),
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            dpos_blocks_per_year: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let mut data = hex::decode("5c19a95c").unwrap();
    data.extend_from_slice(&[0; 12]);
    data.extend_from_slice(&address(0x31));
    // Recovered transaction metadata is the existing FinalChain fixture boundary.
    let tx = FinalizationTransaction {
        hash: [0x51; 32],
        sender: address(0xbb),
        receiver: Some(address(0xfe)),
        nonce: FinalChainNonce::zero(),
        value: U256::from(20).into(),
        gas_price: U256::zero().into(),
        gas_limit: 200_000.into(),
        data,
        rlp: vec![0xc0],
    };
    fn fields(stream: &mut RlpStream) {
        for value in 10..14 {
            stream.append(&H256::from_low_u64_be(value));
        }
        stream
            .append(&1_u64)
            .append(&1_700_000_001_u64)
            .begin_list(0);
    }
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned);
    let (signature, recovery) = SigningKey::from_slice(&[9; 32])
        .unwrap()
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .unwrap();
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut pbft = RlpStream::new_list(8);
    fields(&mut pbft);
    pbft.append(&signature);
    let pbft = pbft.out().to_vec();
    let mut period = RlpStream::new_list(4);
    period
        .append_raw(&pbft, 1)
        .begin_list(0)
        .begin_list(0)
        .begin_list(1)
        .append_raw(&tx.rlp, 1);
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(
            &mut batch,
            Column::PeriodData,
            &1_u64.to_le_bytes(),
            &period.out(),
        )
        .unwrap();
    storage.commit_write_batch_with_sync(batch, false).unwrap();
    let (_, receipts) = final_chain.finalize_block(pbft, vec![tx], vec![]).unwrap();
    assert_eq!(rlp::Rlp::new(&receipts[0]).val_at::<u8>(0).unwrap(), 1);
    assert_eq!(
        final_chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![110]
    );
    final_chain
}

struct Dpos;
impl NativeAddressClassifier for Dpos {
    fn is_native_address(&self, _: FinalChainBlockNumber, target: [u8; 20]) -> bool {
        target == address(0xfe)
    }
}
struct NoHistory;
impl BlockHashRead for NoHistory {
    fn block_hash(&self, _: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("fixture has no BLOCKHASH")
    }
}

#[test]
fn persisted_native_simulations_match_actual_go_dry_runner_and_reopen() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/native_simulation/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/native_simulation/local.json"
    ))
    .unwrap();
    assert_eq!(fixture, local);
    assert_eq!(fixture["state_before"], fixture["state_after"]);
    let concrete = FixturePath::new("concrete");
    let app = FixturePath::new("app");
    let identity = materialize(&fixture, &concrete);
    let final_chain = native_history(&app);
    let before = concrete_rows(&concrete);
    for _ in 0..2 {
        let reader = CompleteSeedReader::open(&concrete, &fixture, identity);
        for case in fixture["cases"].as_array().unwrap() {
            assert_eq!(case["output"]["consensus_error"], "");
            let ConcreteRead::Present(sender) = reader.account(address(0xaa)).unwrap() else {
                panic!("fixture sender must exist");
            };
            assert_eq!(
                BigUint::from_bytes_be(&sender.account.nonce.next().to_bytes()),
                number(&case["output"]["effective_nonce"]),
            );
            let transaction = ExecutionTransaction {
                position: 0.into(),
                hash: [0; 32],
                sender: address(0xaa),
                receiver: Some(bytes(&case["to"]).try_into().unwrap()),
                nonce: if number(&case["supplied_nonce"]) == BigUint::default() {
                    FinalChainNonce::zero()
                } else {
                    FinalChainNonce::from_bytes(&number(&case["supplied_nonce"]).to_bytes_be())
                        .unwrap()
                },
                gas_price: ExecutionGasPrice::new(number(&case["gas_price"])),
                gas_limit: case["gas"].as_u64().unwrap().into(),
                value: ExecutionValue::new(number(&case["value"])),
                input: bytes(&case["input"]),
                canonical_rlp: None,
                kind: ExecutionTransactionKind::Call,
            };
            let result = simulate_with_native(
                &reader,
                &NoHistory,
                &Dpos,
                &Dpos,
                |selected| {
                    assert_eq!(selected, identity);
                    Ok(mixed_native::SimulationNativeExecutionPort::new(
                        final_chain
                            .begin_native_simulation(selected.period)
                            .unwrap(),
                    ))
                },
                &ExecutionBlockContext {
                    period: identity.period,
                    author: address(0x31),
                    timestamp: 1_700_000_001,
                    gas_limit: 500_000.into(),
                    chain_id: 666,
                    difficulty: BigUint::default(),
                },
                &transaction,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            )
            .unwrap_or_else(|error| panic!("{}: {error:?}", case["name"]));
            let TransactionExecutionResult::Executed(result) = result.execution else {
                panic!("{}: unexpected admission rejection", case["name"]);
            };
            let error = match &result.status {
                CodeExecutionStatus::Success => "",
                CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) => {
                    error.error.as_str()
                }
                status => panic!("{}: unexpected status {status:?}", case["name"]),
            };
            assert_eq!(
                error,
                case["output"]["execution_error"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                result.gas_used.as_u64(),
                case["output"]["gas_used"].as_u64().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                result.output,
                bytes(&case["output"]["return"]),
                "{}",
                case["name"]
            );
            let logs = result
                .logs
                .iter()
                .map(|log| {
                    json!({
                        "address": hex::encode(log.address),
                        "topics": log.topics.iter().map(hex::encode).collect::<Vec<_>>(),
                        "data": hex::encode(&log.data),
                    })
                })
                .collect::<Vec<_>>();
            let expected = case["output"]["logs"].as_array().unwrap().to_vec();
            assert_eq!(logs, expected, "{}", case["name"]);
        }
    }
    assert_eq!(concrete_rows(&concrete), before);
    assert_eq!(
        final_chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![110]
    );
}

/// Complete fixture provenance must never hide loss of an expected physical row.
#[test]
fn complete_native_seed_keeps_missing_known_rows_unavailable() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/native_simulation/public.json"
    ))
    .unwrap();
    let path = FixturePath::new("missing");
    let identity = materialize(&fixture, &path);
    let key = ConcreteStorageKey(keccak256([4_u8]).0);
    let prefix = rustaxa_storage::storage_version_prefix(address(0xfe), key);
    let reader = CompleteSeedReader::open(&path, &fixture, identity);
    assert!(reader.storage_prefixes.contains(&prefix));
    assert!(matches!(
        reader.storage(address(0xfe), key).unwrap(),
        ConcreteRead::Present(_)
    ));
    drop(reader);
    let db = rocksdb::DB::open_cf(
        &rocksdb::Options::default(),
        &path.0,
        ["1", "2", "3", "4", "5", "6", "7", "8"],
    )
    .unwrap();
    for row in fixture["state_before"]["seed_rows"].as_array().unwrap() {
        if row["column"] == 4 && bytes(&row["key"]).starts_with(&prefix) {
            db.delete_cf(&db.cf_handle("5").unwrap(), bytes(&row["key"]))
                .unwrap();
        }
    }
    db.flush().unwrap();
    drop(db);
    let reader = CompleteSeedReader::open(&path, &fixture, identity);
    assert_eq!(
        reader.storage(address(0xfe), key),
        Err(ConcreteReadError::HistoryUnavailable(identity))
    );
}
