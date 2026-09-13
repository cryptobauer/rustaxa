//! Public DryRunner parity for disposable simulations over committed state.
//!
//! The checked-in fixture comes from the real pinned Go
//! `state_dry_runner.DryRunner.Apply` entrypoint. These tests rebuild its exact
//! account/code/slot seed as an immutable Rust reader, compare ordinary CALL
//! and CREATE results, prove supplied nonces are ignored, and ensure identity
//! mismatches fail before any state read. Fresh estimator probes are composed here.
//! RPC defaults, full RPC estimate behavior, traces,
//! native calls, persistence, and production routing are outside this test.

use std::{cell::Cell, collections::BTreeMap};

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, TransactionExecutionResult,
    },
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    profile::TaraxaProfile,
    simulation::{SimulationError, simulate_ordinary},
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
struct FixtureReader {
    identity: ConcreteStateIdentity,
    accounts: BTreeMap<[u8; 20], ConcreteAccountRecord>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    codes: BTreeMap<[u8; 32], Vec<u8>>,
    account_reads: Cell<usize>,
    account_error: Option<ConcreteReadError>,
}

impl FixtureReader {
    fn from_fixture(fixture: &Value) -> Self {
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

struct NoHistory;

impl BlockHashRead for NoHistory {
    fn block_hash(&self, _number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("bounded bytecode does not execute BLOCKHASH")
    }
}

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

#[test]
fn ordinary_simulation_matches_both_pinned_go_dry_runners() {
    let public = fixture("public");
    let local = fixture("local");
    assert_eq!(public, local, "public/local DryRunner fixtures diverged");
    assert_eq!(public["committed_state_unchanged"], true);
    assert_eq!(public["state_before"], public["state_after"]);
    let reader = FixtureReader::from_fixture(&public);
    let block = block(&public);
    let supplied_nonces: Vec<_> = public["cases"]
        .as_array()
        .expect("cases")
        .iter()
        .map(|case| case["input"]["supplied_nonce"].as_str().unwrap())
        .collect();
    assert!(supplied_nonces.contains(&"0"));
    assert!(
        supplied_nonces
            .iter()
            .any(|value| number_str(value).bits() > 256)
    );

    let mut outputs = BTreeMap::new();
    for case in public["cases"].as_array().expect("cases") {
        let name = case["name"].as_str().expect("case name");
        let result = simulate_ordinary(
            &reader,
            &NoHistory,
            &NoNative,
            &block,
            &transaction(case),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert_eq!(result.state, reader.identity, "{name}: state identity");
        compare_execution(name, &result.execution, &case["output"]);
        outputs.insert(name, result.execution);
    }
    assert_eq!(
        outputs["call_stale_nonce"], outputs["call_large_nonce"],
        "DryRunner must ignore both stale and >256-bit supplied nonces"
    );
}

#[test]
fn repeated_simulation_discards_writes_and_reuses_the_same_committed_seed() {
    let fixture = fixture("public");
    let reader = FixtureReader::from_fixture(&fixture);
    let block = block(&fixture);
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "call_stale_nonce")
        .unwrap();
    let run = || {
        simulate_ordinary(
            &reader,
            &NoHistory,
            &NoNative,
            &block,
            &transaction(case),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap()
    };
    let first = run();
    let second = run();
    assert_eq!(first, second);
    compare_execution(
        "repeat first",
        &first.execution,
        &fixture["repeated"]["first"],
    );
    compare_execution(
        "repeat second",
        &second.execution,
        &fixture["repeated"]["second"],
    );
    assert_eq!(fixture["repeated"]["identical"], true);
    assert_eq!(reader.storage.values().next().unwrap(), &vec![7]);
    assert_eq!(
        reader.accounts[&address(&case["input"]["from"])]
            .account
            .nonce,
        nonce(&fixture["state_before"]["accounts"][0]["nonce"])
    );
}

#[test]
fn period_mismatch_fails_before_reading_the_sender() {
    let fixture = fixture("public");
    let reader = FixtureReader::from_fixture(&fixture);
    let mut block = block(&fixture);
    block.period = FinalChainBlockNumber::new(block.period.as_u64() + 1);
    let case = &fixture["cases"][0];
    let error = simulate_ordinary(
        &reader,
        &NoHistory,
        &NoNative,
        &block,
        &transaction(case),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap_err();
    assert!(matches!(error, SimulationError::StatePeriodMismatch { .. }));
    assert_eq!(reader.account_reads.get(), 0);
}

#[test]
fn gas_search_uses_a_fresh_simulation_for_each_probe() {
    use rustaxa_evm::estimate::{EstimateProbe, estimate_gas};
    let fixture = fixture("public");
    let reader = FixtureReader::from_fixture(&fixture);
    let original_accounts = reader.accounts.clone();
    let original_storage = reader.storage.clone();
    let original_code = reader.codes.clone();
    let case = &fixture["cases"][0];
    let mut tx = transaction(case);
    let mut probes = Vec::new();
    let estimate = estimate_gas(tx.gas_limit.as_u64(), |gas| {
        probes.push(gas);
        tx.gas_limit = gas.into();
        let result = simulate_ordinary(
            &reader,
            &NoHistory,
            &NoNative,
            &block(&fixture),
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )?;
        let TransactionExecutionResult::Executed(executed) = result.execution else {
            panic!("funded fixture must pass admission")
        };
        assert_eq!(executed.status, CodeExecutionStatus::Success);
        // The contract increments the committed slot from seven to eight. A
        // leaked journal would expose nine (and different SSTORE gas) next time.
        assert_eq!(
            &executed.output[..32],
            &bytes(&case["output"]["return"])[..32]
        );
        assert_eq!(result.state, reader.identity);
        Ok::<_, SimulationError>(EstimateProbe::Success {
            gas_used: executed.gas_used.as_u64(),
        })
    })
    .unwrap();
    assert_eq!(estimate, 29_373);
    assert_eq!(
        probes,
        [100_000, 64_126, 46_189, 37_220, 32_736, 30_494, 29_373]
    );
    assert_eq!(reader.accounts, original_accounts);
    assert_eq!(reader.storage, original_storage);
    assert_eq!(reader.codes, original_code);
}

#[test]
fn unavailable_or_corrupt_sender_is_not_simulated_as_an_empty_account() {
    let fixture = fixture("public");
    let mut reader = FixtureReader::from_fixture(&fixture);
    for error in [
        ConcreteReadError::HistoryUnavailable(reader.identity),
        ConcreteReadError::Corrupt("sender encoding".into()),
    ] {
        reader.account_error = Some(error.clone());
        assert_eq!(
            simulate_ordinary(
                &reader,
                &NoHistory,
                &NoNative,
                &block(&fixture),
                &transaction(&fixture["cases"][0]),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            ),
            Err(SimulationError::Sender(
                rustaxa_evm::journal::JournalError::State(error)
            ))
        );
    }
}

fn compare_execution(name: &str, actual: &TransactionExecutionResult, expected: &Value) {
    let TransactionExecutionResult::Executed(actual) = actual else {
        panic!("{name}: Go fixture executed but Rust returned consensus failure")
    };
    let error = expected["execution_error"].as_str().unwrap();
    assert_eq!(
        actual.status,
        if error.is_empty() {
            CodeExecutionStatus::Success
        } else {
            assert!(error.starts_with("execution reverted"), "{name}: {error}");
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        },
        "{name}: status"
    );
    assert_eq!(
        actual.gas_used.as_u64(),
        expected["gas_used"].as_u64().unwrap(),
        "{name}: gas"
    );
    assert_eq!(actual.output, bytes(&expected["return"]), "{name}: output");
    assert!(actual.logs.is_empty(), "{name}: logs");
    let created = address(&expected["created"]);
    assert_eq!(
        actual.attempted_contract_address,
        (created != [0_u8; 20]).then_some(created),
        "{name}: attempted CREATE address"
    );
    assert_eq!(
        expected["consensus_error"], "",
        "{name}: Go consensus result"
    );
}

fn fixture(reference: &str) -> Value {
    let source = match reference {
        "public" => {
            include_str!("../../../../experiments/evm_feasibility/fixtures/api_public.json")
        }
        "local" => include_str!("../../../../experiments/evm_feasibility/fixtures/api_local.json"),
        _ => unreachable!(),
    };
    serde_json::from_str(source).expect("API fixture JSON")
}

fn block(fixture: &Value) -> ExecutionBlockContext {
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

fn transaction(case: &Value) -> ExecutionTransaction {
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

fn address(value: &Value) -> [u8; 20] {
    hex::decode(value.as_str().expect("hex address"))
        .expect("address hex")
        .try_into()
        .expect("20-byte address")
}

fn hash(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().expect("hex hash"))
        .expect("hash hex")
        .try_into()
        .expect("32-byte hash")
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("hex bytes")).expect("valid hex")
}

fn number(value: &Value) -> BigUint {
    number_str(value.as_str().expect("decimal number"))
}

fn number_str(value: &str) -> BigUint {
    BigUint::parse_bytes(value.as_bytes(), 10).expect("decimal integer")
}

fn nonce(value: &Value) -> FinalChainNonce {
    let number = number(value);
    let bytes = if number == BigUint::default() {
        Vec::new()
    } else {
        number.to_bytes_be()
    };
    FinalChainNonce::from_bytes(&bytes).expect("canonical nonce")
}
