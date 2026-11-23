//! Historical tracing state selection, isolation, and restart checks.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, ExecutionBlockContext, ExecutionGasPrice,
        ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
    },
    driver::{ExecutionDriverError, NativeAddressClassifier},
    envelope::{EnvelopeError, EnvelopeRules},
    host::HostError,
    journal::JournalError,
    profile::TaraxaProfile,
    trace_runner::{StructuredTraceRunnerError, TraceSequenceStage, run_structured_trace},
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{ConcreteReadError, ConcreteStateRead},
};
use serde_json::Value;

#[path = "support/api_fixture.rs"]
#[allow(dead_code)]
mod api_fixture;

use api_fixture::{FixtureReader, PersistedApiFixture};

struct NoHistory;

impl BlockHashRead for NoHistory {
    fn block_hash(&self, _number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        unreachable!("bounded corpus bytecode does not execute BLOCKHASH")
    }
}

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/trace_public.json")).unwrap()
}

fn number(value: &Value) -> BigUint {
    value
        .as_u64()
        .map(BigUint::from)
        .or_else(|| {
            value
                .as_str()
                .and_then(|value| BigUint::parse_bytes(value.as_bytes(), 10))
        })
        .unwrap()
}

fn nonce(value: &Value, offset: usize) -> FinalChainNonce {
    let number = if value.as_u64() == Some(0) {
        BigUint::default()
    } else {
        (BigUint::from(1_u8) << 264) + BigUint::from(6_u64 + offset as u64)
    };
    let bytes = if number == BigUint::default() {
        Vec::new()
    } else {
        number.to_bytes_be()
    };
    FinalChainNonce::from_bytes(&bytes).unwrap()
}

fn address(value: &Value) -> [u8; 20] {
    hex::decode(value.as_str().unwrap().trim_start_matches("0x"))
        .unwrap()
        .try_into()
        .unwrap()
}

fn transaction(value: &Value, position: usize, nonce_offset: usize) -> ExecutionTransaction {
    let receiver = value
        .get("To")
        .filter(|value| !value.is_null())
        .map(address);
    let input = match value.get("Input").and_then(Value::as_str) {
        None => Vec::new(),
        Some("f2AqYABSYCBgAPMAAAAAAAAAAAAAAAAAAAAAAAAAAABgAFJgCmAA8w==") => hex::decode(
            "7f602a60005260206000f3000000000000000000000000000000000000000000600052600a6000f3",
        )
        .unwrap(),
        Some(other) => panic!("unmapped fixture base64 input {other}"),
    };
    ExecutionTransaction {
        position: FinalChainTransactionPosition::from(position as u32),
        hash: [position as u8; 32],
        sender: address(&value["From"]),
        receiver,
        nonce: nonce(&value["Nonce"], nonce_offset),
        gas_price: ExecutionGasPrice::new(number(&value["GasPrice"])),
        gas_limit: FinalChainGas::new(value["Gas"].as_u64().unwrap()),
        value: ExecutionValue::new(number(&value["Value"])),
        input,
        canonical_rlp: None,
        kind: if receiver.is_some() {
            ExecutionTransactionKind::Call
        } else {
            ExecutionTransactionKind::Create
        },
    }
}

fn transactions(values: &Value, nonce_offset: usize) -> Vec<ExecutionTransaction> {
    values
        .as_array()
        .map(|values| {
            values
                .iter()
                .enumerate()
                .map(|(index, value)| transaction(value, index, nonce_offset + index))
                .collect()
        })
        .unwrap_or_default()
}

fn block(fixture: &Value) -> ExecutionBlockContext {
    let value = &fixture["block"];
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(value["Number"].as_u64().unwrap()),
        author: address(&Value::String(value["Author"].as_str().unwrap().to_owned())),
        timestamp: value["Time"].as_u64().unwrap(),
        gas_limit: FinalChainGas::new(value["GasLimit"].as_u64().unwrap()),
        chain_id: 1,
        difficulty: number(&value["Difficulty"]),
    }
}

fn run_case(reader: &impl ConcreteStateRead, fixture: &Value, case: &Value) -> Vec<u8> {
    let prefix = transactions(&case["prefix"], 0);
    let targets = transactions(&case["targets"], prefix.len());
    run_structured_trace(
        reader,
        &NoHistory,
        &NoNative,
        &block(fixture),
        &prefix,
        &targets,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap()
    .output
}

#[test]
fn prefix_sequence_and_whole_api_call_are_disposable() {
    let fixture = fixture();
    let reader = FixtureReader::from_fixture(&fixture);
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "prefix_then_two_calls")
        .unwrap();
    let first = run_case(&reader, &fixture, case);
    let second = run_case(&reader, &fixture, case);
    assert_eq!(first, second);
    assert_eq!(
        serde_json::from_slice::<Value>(&first)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn preceding_period_mismatch_fails_before_state_reads() {
    let fixture = fixture();
    let mut reader = FixtureReader::from_fixture(&fixture);
    reader.identity.period = FinalChainBlockNumber::new(6);
    let case = &fixture["cases"][0];
    let targets = transactions(&case["targets"], 0);
    let error = run_structured_trace(
        &reader,
        &NoHistory,
        &NoNative,
        &block(&fixture),
        &[],
        &targets,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap_err();
    assert_eq!(reader.account_reads.get(), 0);
    assert_eq!(
        error,
        StructuredTraceRunnerError::StatePeriodMismatch {
            state: reader.identity,
            expected: FinalChainBlockNumber::new(7),
            execution: FinalChainBlockNumber::new(8),
        }
    );
}

#[test]
fn execution_period_zero_selects_state_period_zero() {
    let fixture = fixture();
    let mut reader = FixtureReader::from_fixture(&fixture);
    reader.identity.period = FinalChainBlockNumber::new(0);
    let mut execution_block = block(&fixture);
    execution_block.period = FinalChainBlockNumber::new(0);

    let run = run_structured_trace(
        &reader,
        &NoHistory,
        &NoNative,
        &execution_block,
        &[],
        &[],
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    assert_eq!(run.state, reader.identity);
    assert_eq!(run.output, b"[]");
    assert_eq!(reader.account_reads.get(), 0);
}

#[test]
fn unavailable_sender_history_retains_target_index_and_error() {
    let fixture = fixture();
    let mut reader = FixtureReader::from_fixture(&fixture);
    reader.account_error = Some(ConcreteReadError::HistoryUnavailable(reader.identity));
    let case = &fixture["cases"][0];
    let targets = transactions(&case["targets"], 0);
    let error = run_structured_trace(
        &reader,
        &NoHistory,
        &NoNative,
        &block(&fixture),
        &[],
        &targets,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap_err();
    assert_eq!(
        error,
        StructuredTraceRunnerError::Execution {
            stage: TraceSequenceStage::Target,
            index: 0,
            error: ExecutionDriverError::Envelope(EnvelopeError::Journal(JournalError::State(
                ConcreteReadError::HistoryUnavailable(reader.identity)
            ))),
        }
    );
}

#[test]
fn persisted_preceding_state_reopens_without_writes() {
    let fixture = fixture();
    let expected = FixtureReader::from_fixture(&fixture).identity;
    let database = PersistedApiFixture::materialize(&fixture);
    let before = database.rows();
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "prefix_then_two_calls")
        .unwrap();

    for _ in 0..2 {
        let reader =
            rustaxa_storage::ConcreteStateReader::open_read_only(&database.0, expected).unwrap();
        assert_eq!(
            run_case(&reader, &fixture, case),
            run_case(&reader, &fixture, case)
        );
        drop(reader);
        assert_eq!(database.rows(), before);
    }
}

#[test]
fn persisted_missing_sequence_dependencies_remain_unavailable() {
    let fixture = fixture();
    let expected = FixtureReader::from_fixture(&fixture);
    let case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "storage_call")
        .unwrap();
    let target = transactions(&case["targets"], 0)[0].receiver.unwrap();
    let sender = transactions(&case["targets"], 0)[0].sender;
    let identity = expected.identity;
    let unavailable = JournalError::State(ConcreteReadError::HistoryUnavailable(identity));
    let cases = [
        (
            "3",
            rustaxa_storage::versioned_key(
                rustaxa_storage::account_version_prefix(sender),
                identity.period,
            )
            .to_vec(),
            ExecutionDriverError::Envelope(EnvelopeError::Journal(unavailable.clone())),
        ),
        (
            "1",
            expected.accounts[&target]
                .account
                .code_hash
                .unwrap()
                .to_vec(),
            ExecutionDriverError::Journal(unavailable.clone()),
        ),
        (
            "5",
            rustaxa_storage::versioned_key(
                rustaxa_storage::storage_version_prefix(
                    target,
                    rustaxa_types::concrete_state::ConcreteStorageKey([0; 32]),
                ),
                identity.period,
            )
            .to_vec(),
            ExecutionDriverError::Host(HostError::Journal(unavailable)),
        ),
    ];

    for (column, key, expected_error) in cases {
        let database = PersistedApiFixture::materialize(&fixture);
        {
            let writable = rocksdb::DB::open_cf(
                &rocksdb::Options::default(),
                &database.0,
                ["1", "2", "3", "4", "5", "6", "7", "8"],
            )
            .unwrap();
            writable
                .delete_cf(&writable.cf_handle(column).unwrap(), key)
                .unwrap();
            writable.flush().unwrap();
        }
        let before = database.rows();
        let reader =
            rustaxa_storage::ConcreteStateReader::open_read_only(&database.0, identity).unwrap();
        let error = run_structured_trace(
            &reader,
            &NoHistory,
            &NoNative,
            &block(&fixture),
            &[],
            &transactions(&case["targets"], 0),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap_err();
        assert_eq!(
            error,
            StructuredTraceRunnerError::Execution {
                stage: TraceSequenceStage::Target,
                index: 0,
                error: expected_error,
            }
        );
        drop(reader);
        assert_eq!(database.rows(), before);
    }
}

#[path = "../../rustaxa-storage/src/test_paths.rs"]
mod test_paths;
