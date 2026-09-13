//! Public DryRunner parity for disposable simulations over committed state.
//!
//! The checked-in fixture comes from the real pinned Go
//! `state_dry_runner.DryRunner.Apply` entrypoint. These tests rebuild its exact
//! account/code/slot seed as an immutable Rust reader, compare ordinary CALL
//! and CREATE results, prove supplied nonces are ignored, and ensure identity
//! mismatches fail before any state read. Fresh estimator probes are composed here.
//! Persisted fixtures additionally prove historical selection, reopen and missing-
//! dependency behavior without committed writes. RPC defaults, full RPC estimate
//! behavior, traces, native calls and production routing are outside this test.

use std::collections::BTreeMap;

#[path = "support/api_fixture.rs"]
mod api_fixture;

use api_fixture::{
    FixtureReader, PersistedApiFixture, address, block, bytes, fixture, nonce, number_str,
    transaction,
};

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        TransactionExecutionResult,
    },
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    profile::TaraxaProfile,
    simulation::{SimulationError, simulate_ordinary},
};
use rustaxa_types::{
    FinalChainBlockNumber,
    concrete_state::{
        ConcreteAccountBalance, ConcreteRead, ConcreteReadError, ConcreteStateRead,
        ConcreteStorageKey,
    },
};
use serde_json::Value;

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
    if actual.status == CodeExecutionStatus::Failure(CodeExecutionError::Revert) {
        assert_eq!(
            rustaxa_evm::revert::dry_run_revert_diagnostic(&actual.output),
            error.as_bytes(),
            "{name}: full dry-run revert diagnostic",
        );
    }
    assert_eq!(
        actual.status,
        match error {
            "" => CodeExecutionStatus::Success,
            "return data out of bounds" =>
                CodeExecutionStatus::Failure(CodeExecutionError::ReturnDataOutOfBounds),
            "execution reverted: oracle boom" =>
                CodeExecutionStatus::Failure(CodeExecutionError::Revert),
            error => panic!("unexpected reference error {name}: {error}"),
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

#[test]
fn persisted_go_state_reopens_for_queries_and_disposable_simulations() {
    use rustaxa_storage::ConcreteStateReader;
    let fixture = fixture("public");
    let expected = FixtureReader::from_fixture(&fixture);
    let db = PersistedApiFixture::materialize(&fixture);
    let committed = db.append_newer_sender(&fixture);
    let latest = ConcreteStateReader::open_read_only(&db.0, committed).unwrap();
    let ConcreteRead::Present(sender) = latest
        .account(transaction(&fixture["cases"][0]).sender)
        .unwrap()
    else {
        panic!("latest sender")
    };
    assert_eq!(
        sender.account.balance,
        ConcreteAccountBalance::new(BigUint::default())
    );
    drop(latest);
    let before = db.rows();
    for _ in 0..2 {
        let reader =
            ConcreteStateReader::open_historical_read_only(&db.0, committed, expected.identity)
                .unwrap();
        for (address, account) in &expected.accounts {
            assert_eq!(
                reader.account(*address).unwrap(),
                ConcreteRead::Present(account.clone())
            );
        }
        for (hash, code) in &expected.codes {
            assert_eq!(
                reader.code(*hash).unwrap(),
                ConcreteRead::Present(code.clone())
            );
        }
        for ((address, key), value) in &expected.storage {
            assert_eq!(
                reader.storage(*address, *key).unwrap(),
                ConcreteRead::Present(value.clone())
            );
            assert_eq!(
                reader.verify_storage_path(*address, *key).unwrap(),
                rustaxa_storage::ConcreteStoragePath::Member(value.clone())
            );
        }
        for case in fixture["cases"].as_array().unwrap() {
            for _ in 0..2 {
                let result = simulate_ordinary(
                    &reader,
                    &NoHistory,
                    &NoNative,
                    &block(&fixture),
                    &transaction(case),
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )
                .unwrap();
                assert_eq!(result.state, expected.identity);
                compare_execution(
                    case["name"].as_str().unwrap(),
                    &result.execution,
                    &case["output"],
                );
            }
        }
        let mut request = transaction(&fixture["cases"][0]);
        let mut probes = Vec::new();
        let estimate = rustaxa_evm::estimate::estimate_gas(request.gas_limit.as_u64(), |gas| {
            probes.push(gas);
            request.gas_limit = gas.into();
            let simulation = simulate_ordinary(
                &reader,
                &NoHistory,
                &NoNative,
                &block(&fixture),
                &request,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )?;
            let TransactionExecutionResult::Executed(executed) = simulation.execution else {
                panic!("historical probe admission")
            };
            assert_eq!(executed.status, CodeExecutionStatus::Success);
            assert_eq!(
                BigUint::from_bytes_be(&executed.output[..32]),
                BigUint::from(8_u8)
            );
            Ok::<_, SimulationError>(rustaxa_evm::estimate::EstimateProbe::Success {
                gas_used: executed.gas_used.as_u64(),
            })
        })
        .unwrap();
        assert_eq!(estimate, 29_373);
        assert_eq!(
            probes,
            vec![100_000, 64_126, 46_189, 37_220, 32_736, 30_494, 29_373]
        );
        drop(reader);
        assert_eq!(
            db.rows(),
            before,
            "all persisted rows and descriptor must remain identical"
        );
    }
}

#[test]
fn persisted_missing_sender_version_stays_unavailable_after_reopen() {
    use rustaxa_evm::journal::JournalError;
    use rustaxa_storage::ConcreteStateReader;
    let fixture = fixture("public");
    let identity = FixtureReader::from_fixture(&fixture).identity;
    let db = PersistedApiFixture::materialize(&fixture);
    let case = &fixture["cases"][0];
    let key = rustaxa_storage::versioned_key(
        rustaxa_storage::account_version_prefix(transaction(case).sender),
        identity.period,
    );
    {
        let writable = rocksdb::DB::open_cf(
            &rocksdb::Options::default(),
            &db.0,
            ["1", "2", "3", "4", "5", "6", "7", "8"],
        )
        .unwrap();
        writable
            .delete_cf(&writable.cf_handle("3").unwrap(), key)
            .unwrap();
        writable.flush().unwrap();
    }
    let before = db.rows();
    let reader = ConcreteStateReader::open_historical_read_only(&db.0, identity, identity).unwrap();
    let error = simulate_ordinary(
        &reader,
        &NoHistory,
        &NoNative,
        &block(&fixture),
        &transaction(case),
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap_err();
    assert!(
        matches!(
            error,
            SimulationError::Sender(JournalError::State(
                ConcreteReadError::HistoryUnavailable { .. }
            ))
        ),
        "{error:?}"
    );
    drop(reader);
    assert_eq!(db.rows(), before);
}

#[test]
fn persisted_missing_code_or_slot_aborts_simulation_without_state_changes() {
    use rustaxa_evm::{driver::ExecutionDriverError, host::HostError, journal::JournalError};
    let fixture = fixture("public");
    let expected = FixtureReader::from_fixture(&fixture);
    let case = &fixture["cases"][0];
    let target = transaction(case).receiver.unwrap();
    let code_key = expected.accounts[&target]
        .account
        .code_hash
        .unwrap()
        .to_vec();
    let slot_key = rustaxa_storage::versioned_key(
        rustaxa_storage::storage_version_prefix(target, ConcreteStorageKey([0; 32])),
        expected.identity.period,
    )
    .to_vec();
    for (column, key) in [("1", code_key), ("5", slot_key)] {
        let db = PersistedApiFixture::materialize(&fixture);
        {
            let writable = rocksdb::DB::open_cf(
                &rocksdb::Options::default(),
                &db.0,
                ["1", "2", "3", "4", "5", "6", "7", "8"],
            )
            .unwrap();
            writable
                .delete_cf(&writable.cf_handle(column).unwrap(), key)
                .unwrap();
            writable.flush().unwrap();
        }
        let before = db.rows();
        let reader =
            rustaxa_storage::ConcreteStateReader::open_read_only(&db.0, expected.identity).unwrap();
        let error = simulate_ordinary(
            &reader,
            &NoHistory,
            &NoNative,
            &block(&fixture),
            &transaction(case),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap_err();
        let unavailable =
            JournalError::State(ConcreteReadError::HistoryUnavailable(expected.identity));
        let expected_error = if column == "1" {
            SimulationError::Execution(ExecutionDriverError::Journal(unavailable))
        } else {
            SimulationError::Execution(ExecutionDriverError::Host(HostError::Journal(unavailable)))
        };
        assert_eq!(error, expected_error);
        drop(reader);
        assert_eq!(db.rows(), before);
    }
}
