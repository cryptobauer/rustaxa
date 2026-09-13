//! Exact structured-logger JSON comparisons against the pinned Go corpus.

use std::{fs, path::PathBuf};

use revm::interpreter::InstructionResult;
use rustaxa_evm::{
    structured_trace::{
        StructuredTraceResult, StructuredTraceSerializationError, serialize_structured_results,
    },
    trace::{AttemptedSstore, TraceEvent, TraceOpcode, TraceOpcodePhase},
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(&fs::read_to_string(fixture_path()).unwrap()).unwrap()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/trace_public.json")
}

fn opcode_byte(name: &str) -> u8 {
    match name {
        "STOP" => 0x00,
        "ADD" => 0x01,
        "ADDRESS" => 0x30,
        "BALANCE" => 0x31,
        "ORIGIN" => 0x32,
        "RETURNDATACOPY" => 0x3e,
        "MSTORE" => 0x52,
        "SLOAD" => 0x54,
        "SSTORE" => 0x55,
        "PUSH1" => 0x60,
        "PUSH32" => 0x7f,
        "DUP1" => 0x80,
        "RETURN" => 0xf3,
        "REVERT" => 0xfd,
        other => panic!("unmapped fixture opcode {other}"),
    }
}

fn word(value: &Value) -> [u8; 32] {
    hex::decode(value.as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap()
}

fn fixture_events(rows: &[Value], state_address: [u8; 20]) -> Vec<TraceEvent> {
    rows.iter()
        .map(|row| {
            let opcode = opcode_byte(row["op"].as_str().unwrap());
            let stack = row["stack"]
                .as_array()
                .unwrap()
                .iter()
                .map(word)
                .collect::<Vec<_>>();
            let attempted_sstore = (opcode == 0x55 && stack.len() >= 2).then(|| AttemptedSstore {
                key: stack[stack.len() - 1],
                value: stack[stack.len() - 2],
            });
            let memory = row["memory"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|item| hex::decode(item.as_str().unwrap()).unwrap())
                .collect();
            TraceEvent::Opcode(TraceOpcode {
                pc: row["pc"].as_u64().unwrap(),
                opcode,
                gas: row["gas"].as_u64().unwrap(),
                gas_cost: row["gasCost"].as_u64().unwrap(),
                depth: row["depth"].as_u64().unwrap() as u16,
                state_address,
                stack,
                memory,
                refund: 0,
                phase: if row.get("error").is_some() {
                    TraceOpcodePhase::Fault(if opcode == 0x3e {
                        InstructionResult::OutOfOffset
                    } else {
                        InstructionResult::Revert
                    })
                } else {
                    TraceOpcodePhase::BeforeExecution
                },
                attempted_sstore,
            })
        })
        .collect()
}

struct OwnedResult {
    gas_used: u64,
    failed: bool,
    return_value: Vec<u8>,
    events: Vec<TraceEvent>,
}

#[test]
fn all_actual_structured_results_match_exact_json_values() {
    let fixture = fixture();
    for case in fixture["cases"].as_array().unwrap() {
        let expected = &case["outputs"]["struct"]["result"];
        let owned = expected
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(index, result)| OwnedResult {
                gas_used: result["gas"].as_u64().unwrap(),
                failed: result["failed"].as_bool().unwrap(),
                return_value: hex::decode(result["returnValue"].as_str().unwrap()).unwrap(),
                events: fixture_events(result["structLogs"].as_array().unwrap(), {
                    let mut address = [0_u8; 20];
                    address[19] = index as u8 + 1;
                    address
                }),
            })
            .collect::<Vec<_>>();
        let supplied = owned
            .iter()
            .map(|result| StructuredTraceResult {
                gas_used: result.gas_used,
                failed: result.failed,
                return_value: &result.return_value,
                events: &result.events,
            })
            .collect::<Vec<_>>();

        let actual: Value =
            serde_json::from_slice(&serialize_structured_results(&supplied).unwrap()).unwrap();
        assert_eq!(actual, *expected, "case {}", case["name"]);
    }
}

fn synthetic_opcode(
    address: [u8; 20],
    opcode: u8,
    stack: Vec<[u8; 32]>,
    attempted_sstore: Option<AttemptedSstore>,
) -> TraceEvent {
    TraceEvent::Opcode(TraceOpcode {
        pc: u64::from(opcode),
        opcode,
        gas: 9,
        gas_cost: 3,
        depth: 1,
        state_address: address,
        stack,
        memory: Vec::new(),
        refund: 7,
        phase: TraceOpcodePhase::BeforeExecution,
        attempted_sstore,
    })
}

#[test]
fn storage_is_address_local_and_resets_for_each_target() {
    let key = [0x11; 32];
    let value = [0x22; 32];
    let first_events = vec![
        synthetic_opcode(
            [0xaa; 20],
            0x55,
            vec![value, key],
            Some(AttemptedSstore { key, value }),
        ),
        synthetic_opcode([0xbb; 20], 0x00, Vec::new(), None),
    ];
    let second_events = vec![synthetic_opcode([0xaa; 20], 0x00, Vec::new(), None)];
    let bytes = serialize_structured_results(&[
        StructuredTraceResult {
            gas_used: 1,
            failed: false,
            return_value: &[],
            events: &first_events,
        },
        StructuredTraceResult {
            gas_used: 2,
            failed: true,
            return_value: &[0xab],
            events: &second_events,
        },
    ])
    .unwrap();
    let key_hex = "11".repeat(32);
    let value_hex = "22".repeat(32);
    let expected = format!(
        "[{{\"gas\":1,\"failed\":false,\"returnValue\":\"\",\"structLogs\":[{{\"pc\":85,\"op\":\"SSTORE\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[\"{value_hex}\",\"{key_hex}\"],\"memory\":[],\"storage\":{{\"{key_hex}\":\"{value_hex}\"}}}},{{\"pc\":0,\"op\":\"STOP\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[],\"memory\":[],\"storage\":{{}}}}]}},{{\"gas\":2,\"failed\":true,\"returnValue\":\"ab\",\"structLogs\":[{{\"pc\":0,\"op\":\"STOP\",\"gas\":9,\"gasCost\":3,\"depth\":1,\"stack\":[],\"memory\":[],\"storage\":{{}}}}]}}]"
    );
    assert_eq!(String::from_utf8(bytes).unwrap(), expected);
}

#[test]
fn fault_shape_and_unrepresentable_rows_are_explicit() {
    let mut fault = match synthetic_opcode([0; 20], 0xfd, Vec::new(), None) {
        TraceEvent::Opcode(opcode) => opcode,
        _ => unreachable!(),
    };
    fault.phase = TraceOpcodePhase::Fault(InstructionResult::Revert);
    let events = [TraceEvent::Opcode(fault.clone())];
    let bytes = serialize_structured_results(&[StructuredTraceResult {
        gas_used: 3,
        failed: true,
        return_value: &[],
        events: &events,
    }])
    .unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("\"depth\":1,\"error\":{},\"stack\":[],\"memory\":[],\"storage\":{}")
    );

    fault.memory.push(0);
    let events = [TraceEvent::Opcode(fault.clone())];
    assert_eq!(
        serialize_structured_results(&[StructuredTraceResult {
            gas_used: 3,
            failed: true,
            return_value: &[],
            events: &events,
        }]),
        Err(StructuredTraceSerializationError::UnalignedMemory {
            result_index: 0,
            opcode_index: 0,
            length: 1,
        })
    );

    fault.memory.clear();
    fault.phase = TraceOpcodePhase::Fault(InstructionResult::OutOfGas);
    let events = [TraceEvent::Opcode(fault)];
    assert_eq!(
        serialize_structured_results(&[StructuredTraceResult {
            gas_used: 3,
            failed: true,
            return_value: &[],
            events: &events,
        }]),
        Err(StructuredTraceSerializationError::UnsupportedFault {
            result_index: 0,
            opcode_index: 0,
            result: InstructionResult::OutOfGas,
        })
    );
}
