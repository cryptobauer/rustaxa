//! End-to-end default TraceRunner comparisons against the actual pinned Go corpus.

use std::collections::BTreeMap;

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

fn fixture(reference: &str) -> Value {
    let source = match reference {
        "public" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/trace_public.json"
        )),
        "local" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/trace_local.json"
        )),
        _ => unreachable!(),
    };
    serde_json::from_str(source).unwrap()
}

fn refund_fixture(reference: &str) -> Value {
    let source = match reference {
        "public" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/trace_refund/public.json"
        )),
        "local" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../experiments/evm_feasibility/fixtures/trace_refund/local.json"
        )),
        _ => unreachable!(),
    };
    serde_json::from_str(source).unwrap()
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

fn sequence_transactions(inputs: &Value, nonces: &Value) -> Vec<ExecutionTransaction> {
    inputs
        .as_array()
        .unwrap()
        .iter()
        .zip(nonces.as_array().unwrap())
        .enumerate()
        .map(|(index, (input, nonce))| {
            let nonce = BigUint::parse_bytes(nonce.as_str().unwrap().as_bytes(), 10).unwrap();
            ExecutionTransaction {
                position: FinalChainTransactionPosition::from(index as u32),
                hash: [index as u8; 32],
                sender: {
                    let mut sender = [0; 20];
                    sender[19] = 0xaa;
                    sender
                },
                receiver: Some({
                    let mut target = [0; 20];
                    target[19] = 0xbb;
                    target
                }),
                nonce: FinalChainNonce::from_bytes(&nonce.to_bytes_be()).unwrap(),
                gas_price: ExecutionGasPrice::new(BigUint::from(2_u8)),
                gas_limit: FinalChainGas::new(100_000),
                value: ExecutionValue::new(BigUint::from(3_u8)),
                input: hex::decode(input.as_str().unwrap()).unwrap(),
                canonical_rlp: None,
                kind: ExecutionTransactionKind::Call,
            }
        })
        .collect()
}

fn refund_block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: FinalChainBlockNumber::new(8),
        author: [0; 20],
        timestamp: 0,
        gas_limit: FinalChainGas::new(1_000_000),
        chain_id: 1,
        difficulty: BigUint::default(),
    }
}

fn raw_opcode_name(opcode: u64) -> &'static str {
    match opcode {
        0x00 => "STOP",
        0x15 => "ISZERO",
        0x35 => "CALLDATALOAD",
        0x36 => "CALLDATASIZE",
        0x52 => "MSTORE",
        0x55 => "SSTORE",
        0x57 => "JUMPI",
        0x5b => "JUMPDEST",
        0x60 => "PUSH1",
        0xb3 => "TLOAD",
        0xf3 => "RETURN",
        other => panic!("unmapped sequence opcode {other:#x}"),
    }
}

fn raw_word(value: &Value) -> String {
    let value = BigUint::parse_bytes(value.as_str().unwrap().as_bytes(), 10).unwrap();
    format!("{value:064x}")
}

fn decode_base64(value: &str) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    assert_eq!(value.len() % 4, 0);
    let mut decoded = Vec::with_capacity(value.len() / 4 * 3);
    for chunk in value.as_bytes().as_chunks::<4>().0 {
        let mut packed = 0_u32;
        let mut retained = 3_usize;
        for (index, byte) in chunk.iter().copied().enumerate() {
            let digit = if byte == b'=' {
                retained -= 1;
                0
            } else {
                ALPHABET
                    .iter()
                    .position(|candidate| *candidate == byte)
                    .unwrap() as u32
            };
            packed |= digit << (18 - index * 6);
        }
        decoded.extend_from_slice(&packed.to_be_bytes()[1..=retained]);
    }
    decoded
}

/// Converts raw `StructLogger` facts captured by the sequence oracle into the
/// `FormatLogs` shape returned by `TraceRunner`. The independent serializer
/// corpus proves this representation boundary; these rows exercise facade
/// composition with one retained journal and fresh target loggers.
fn formatted_sequence_result(case: &Value) -> Value {
    Value::Array(
        case["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|result| {
                let mut storage = BTreeMap::<String, String>::new();
                let logs = result["structLogs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|row| {
                        let opcode = row["op"].as_u64().unwrap();
                        let stack = row["stack"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(raw_word)
                            .collect::<Vec<_>>();
                        if opcode == 0x55 {
                            storage.insert(
                                stack[stack.len() - 1].clone(),
                                stack[stack.len() - 2].clone(),
                            );
                        }
                        let memory = decode_base64(row["memory"].as_str().unwrap());
                        assert_eq!(memory.len() as u64, row["memSize"].as_u64().unwrap());
                        let memory = memory
                            .as_chunks::<32>()
                            .0
                            .iter()
                            .map(hex::encode)
                            .collect::<Vec<_>>();
                        serde_json::json!({
                            "pc": row["pc"],
                            "op": raw_opcode_name(opcode),
                            "gas": row["gas"],
                            "gasCost": row["gasCost"],
                            "depth": row["depth"],
                            "stack": stack,
                            "memory": memory,
                            "storage": storage,
                        })
                    })
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "gas": result["gas"],
                    "failed": result["failed"],
                    "returnValue": result["returnValue"],
                    "structLogs": logs,
                })
            })
            .collect(),
    )
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
fn all_actual_default_traces_match_both_pins() {
    let public = fixture("public");
    let local = fixture("local");
    assert_eq!(public, local, "public/local TraceRunner fixtures diverged");
    assert_eq!(public["committed_state_unchanged"], true);
    assert_eq!(public["state_before"], public["state_after"]);
    let reader = FixtureReader::from_fixture(&public);
    let expected_state = reader.identity();

    for case in public["cases"].as_array().unwrap() {
        let prefix = transactions(&case["prefix"], 0);
        let targets = transactions(&case["targets"], prefix.len());
        let run = run_structured_trace(
            &reader,
            &NoHistory,
            &NoNative,
            &block(&public),
            &prefix,
            &targets,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        assert_eq!(run.state, expected_state);
        assert_eq!(
            serde_json::from_slice::<Value>(&run.output).unwrap(),
            case["outputs"]["struct"]["result"],
            "case {}",
            case["name"]
        );
    }
}

#[test]
fn retained_sequence_state_matches_actual_go_raw_corpus() {
    let public = refund_fixture("public");
    let local = refund_fixture("local");
    assert_eq!(public, local, "public/local sequence fixtures diverged");
    let cases = public
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("targets").is_some())
        .collect::<Vec<_>>();
    assert_eq!(cases.len(), 3);

    for case in cases {
        assert_eq!(case["state_before"], case["state_after"]);
        let reader = FixtureReader::from_fixture(case);
        let prefix = sequence_transactions(&case["prefix_inputs"], &case["prefix_nonces"]);
        let targets = sequence_transactions(&case["target_inputs"], &case["target_nonces"]);
        let first = run_structured_trace(
            &reader,
            &NoHistory,
            &NoNative,
            &refund_block(),
            &prefix,
            &targets,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let second = run_structured_trace(
            &reader,
            &NoHistory,
            &NoNative,
            &refund_block(),
            &prefix,
            &targets,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        assert_eq!(first, second, "case {} was not disposable", case["name"]);
        assert_eq!(
            serde_json::from_slice::<Value>(&first.output).unwrap(),
            formatted_sequence_result(case),
            "case {}",
            case["name"]
        );
    }
}

#[test]
fn prefix_sequence_and_whole_api_call_are_disposable() {
    let fixture = fixture("public");
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
        serde_json::from_slice::<Value>(&first).unwrap(),
        case["outputs"]["struct"]["result"]
    );
}

#[test]
fn preceding_period_mismatch_fails_before_state_reads() {
    let fixture = fixture("public");
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
    let fixture = fixture("public");
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
    let fixture = fixture("public");
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
    let fixture = fixture("public");
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
    let fixture = fixture("public");
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
