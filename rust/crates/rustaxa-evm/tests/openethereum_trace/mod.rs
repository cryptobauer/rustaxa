//! Compare the Rust historical replay with the pinned Go TraceRunner.

use std::collections::BTreeMap;

use super::{FixtureReader, NoHistory};

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionBlockContext, ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind,
        ExecutionValue, NativeExecutionPort, NativeGasQuote, NativeInvocation,
        NativeInvocationResult, NativeJournalRead, NativePortError,
    },
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    profile::TaraxaProfile,
    trace_runner::{HistoricalTraceFormat, run_trace_with_native},
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteStateIdentity,
    },
};
use serde_json::{Value, json};

struct NoConsensusNative;
impl NativeExecutionPort for NoConsensusNative {
    fn prepare(
        &mut self,
        _: &NativeInvocation,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        unreachable!("fixture uses stateless precompiles");
    }

    fn invoke(
        &mut self,
        _: &NativeInvocation,
        _: NativeGasQuote,
        _: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        unreachable!("fixture uses stateless precompiles");
    }
}

struct NativeAddresses(bool);

impl NativeAddressClassifier for NativeAddresses {
    fn is_native_address(&self, _: FinalChainBlockNumber, candidate: [u8; 20]) -> bool {
        self.0 && candidate == address(4)
    }
}

fn address(last: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = last;
    address
}

fn transaction(
    to: Option<u8>,
    input: &str,
    nonce: u64,
    value: u64,
    gas: u64,
) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0; 32],
        sender: address(0x11),
        receiver: to.map(address),
        nonce: FinalChainNonce::from_u64(nonce),
        gas_price: ExecutionGasPrice::new(BigUint::default()),
        gas_limit: gas.into(),
        value: ExecutionValue::new(value.into()),
        input: hex::decode(input).unwrap(),
        canonical_rlp: None,
        kind: to.map_or(ExecutionTransactionKind::Create, |_| {
            ExecutionTransactionKind::Call
        }),
    }
}

#[test]
fn openethereum_modes_match_pinned_trace_runner() {
    let mut reader = FixtureReader {
        identity: ConcreteStateIdentity {
            period: 0.into(),
            state_root: [0; 32],
        },
        accounts: BTreeMap::new(),
        storage: BTreeMap::new(),
        codes: BTreeMap::new(),
        account_reads: 0.into(),
        account_error: None,
    };
    for (last, balance, code) in [
        (0x11, 1_000_000_u64, ""),
        (0xaa, 0, "602a60005260206000f3"),
        (0xbb, 0, "6020600060006000600060aa61fffff15060206000f3"),
        (0xcc, 0, "602a60005260206000fd"),
        (0xdd, 0, "fe"),
        (0xa1, 0, "6000548060010160005560005260206000f3"),
        (
            0xa2,
            0,
            "600160005360016000600160006000600461fffff15060016000f3",
        ),
        (0xe1, 0, "600156"),
        (0xe2, 0, "600001"),
        (0xe3, 0, "602a61ffff52"),
        (0xe4, 0, "69600060005360016000f3600052600a60166000f05000"),
        (0xe5, 0, "6020600060006000600060aa61fffff25060206000f3"),
        (0xe6, 0, "602060006000600060aa61fffff45060206000f3"),
        (0xe7, 0, "602060006000600060a161fffffa5060206000f3"),
        (0xe8, 3, "6033ff"),
        (0xe9, 0, "60206000600060006000603361fffff15060206000f3"),
        (0xea, 0, "600101"),
        (0xeb, 0, "6000600060006000600060ec61fffff100"),
        (0xec, 0, "00"),
        (0xc1, 0, "60ff60020a"),
        (0xc2, 0, "6001600055"),
        (0xc3, 0, "6000600060006000600060aa61fffff100"),
        (0xc4, 0, "61ffff6000600037"),
        (0xc5, 0, "6101006000a0"),
        (0xc6, 0, "600001"),
        (0xc7, 0, "602060002000"),
        (0xc8, 0, "6020600020"),
        (0xc9, 0, "6020600060003e"),
        (0xca, 0, "600160006000f0"),
        (0xcb, 0, "6000600160006000f5"),
        (0xd0, 0, "600060006000600060d2600afa00"),
        (0xd1, 0, "600060006000600060d260fffa00"),
        (0xd3, 0, "b3"),
        (0xd4, 0, "b4"),
        (0xd2, 0, "60016000b400"),
        (0xd5, 0, "600060006000600060d660fffa00"),
        (0xd6, 0, "b400"),
        (0xd7, 0, "48"),
        (0xd8, 0, "5d"),
    ] {
        let code = hex::decode(code).unwrap();
        let size = code.len() as u64;
        let code_hash = (!code.is_empty()).then(|| {
            let hash = revm::primitives::keccak256(&code).0;
            reader.codes.insert(hash, code);
            hash
        });
        reader.accounts.insert(
            address(last),
            ConcreteAccountRecord {
                account: ConcreteAccount {
                    nonce: FinalChainNonce::from_u64(0),
                    balance: ConcreteAccountBalance::new(balance.into()),
                    storage_root: None,
                    code_hash,
                    code_size: size,
                },
                physical_rlp: vec![],
            },
        );
    }
    let call = |to| transaction(Some(to), "", 0, 0, 100_000);
    let increment = call(0xa1);
    let mut increment_again = transaction(Some(0xa1), "", 1, 0, 100_000);
    increment_again.position = 1.into();
    let cases = [
        ("call_memory_return", vec![], vec![call(0xaa)]),
        ("nested_call", vec![], vec![call(0xbb)]),
        (
            "create",
            vec![],
            vec![transaction(None, "600060005360016000f3", 0, 0, 100_000)],
        ),
        (
            "create_revert",
            vec![],
            vec![transaction(None, "602a60005260206000fd", 0, 0, 100_000)],
        ),
        ("call_revert", vec![], vec![call(0xcc)]),
        (
            "native_identity",
            vec![],
            vec![transaction(Some(4), "1234", 0, 0, 100_000)],
        ),
        ("nested_native_identity", vec![], vec![call(0xa2)]),
        ("invalid_jump", vec![], vec![call(0xe1)]),
        ("stack_underflow", vec![], vec![call(0xe2)]),
        (
            "memory_out_of_gas",
            vec![],
            vec![transaction(Some(0xe3), "", 0, 0, 22_000)],
        ),
        ("nested_create", vec![], vec![call(0xe4)]),
        ("nested_callcode", vec![], vec![call(0xe5)]),
        (
            "nested_delegate",
            vec![],
            vec![transaction(Some(0xe6), "", 0, 2, 100_000)],
        ),
        ("nested_static_error", vec![], vec![call(0xe7)]),
        ("selfdestruct", vec![], vec![call(0xe8)]),
        ("nested_no_code", vec![], vec![call(0xe9)]),
        (
            "insufficient_intrinsic",
            vec![],
            vec![transaction(Some(0xea), "abcdef", 0, 0, 21_000)],
        ),
        (
            "native_identity_in_initcode",
            vec![],
            vec![transaction(
                None,
                "600160005360016000600160006000600461fffff15060016000f3",
                0,
                0,
                100_000,
            )],
        ),
        ("nested_stop", vec![], vec![call(0xeb)]),
        (
            "exp_out_of_gas",
            vec![],
            vec![transaction(Some(0xc1), "", 0, 0, 21020)],
        ),
        (
            "sstore_out_of_gas",
            vec![],
            vec![transaction(Some(0xc2), "", 0, 0, 21012)],
        ),
        (
            "call_out_of_gas",
            vec![],
            vec![transaction(Some(0xc3), "", 0, 0, 21300)],
        ),
        (
            "copy_out_of_gas",
            vec![],
            vec![transaction(Some(0xc4), "", 0, 0, 22000)],
        ),
        (
            "log_out_of_gas",
            vec![],
            vec![transaction(Some(0xc5), "", 0, 0, 22000)],
        ),
        (
            "low_gas_underflow",
            vec![],
            vec![transaction(Some(0xc6), "", 0, 0, 21003)],
        ),
        (
            "keccak",
            vec![],
            vec![transaction(Some(0xc7), "", 0, 0, 100000)],
        ),
        (
            "keccak_out_of_gas",
            vec![],
            vec![transaction(Some(0xc8), "", 0, 0, 21012)],
        ),
        (
            "returndatacopy_out_of_gas",
            vec![],
            vec![transaction(Some(0xc9), "", 0, 0, 21012)],
        ),
        (
            "create_out_of_gas",
            vec![],
            vec![transaction(Some(0xca), "", 0, 0, 21021)],
        ),
        (
            "create2_out_of_gas",
            vec![],
            vec![transaction(Some(0xcb), "", 0, 0, 21021)],
        ),
        ("static_tstore_low", vec![], vec![call(0xd0)]),
        ("static_tstore_affordable", vec![], vec![call(0xd1)]),
        ("tload_underflow", vec![], vec![call(0xd3)]),
        ("tstore_underflow", vec![], vec![call(0xd4)]),
        ("static_tstore_missing_operands", vec![], vec![call(0xd5)]),
        (
            "unsupported_basefee_low_gas",
            vec![],
            vec![transaction(Some(0xd7), "", 0, 0, 21000)],
        ),
        (
            "inactive_tstore_low_gas",
            vec![],
            vec![transaction(Some(0xd8), "", 0, 0, 21000)],
        ),
        ("invalid_opcode", vec![], vec![call(0xdd)]),
        (
            "pure_transfer",
            vec![],
            vec![transaction(Some(0x33), "", 0, 2, 21_000)],
        ),
        (
            "replay_with_prefix",
            vec![increment.clone()],
            vec![increment_again.clone()],
        ),
        ("sequential_block", vec![], vec![increment, increment_again]),
    ];
    let block = ExecutionBlockContext {
        period: 1.into(),
        author: address(0x22),
        timestamp: 10,
        gas_limit: 100_000_000.into(),
        chain_id: 1,
        difficulty: Default::default(),
    };
    let reference: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/openethereum_trace/expected.json"
    )))
    .unwrap();
    let mut mismatches = Vec::new();
    for (name, prefix, targets) in cases {
        for (trace, vm_trace) in [(true, false), (false, true), (true, true)] {
            let result = run_trace_with_native(
                &reader,
                &NoHistory,
                &NativeAddresses(true),
                &NativeAddresses(false),
                |_, _| Ok(NoConsensusNative),
                &block,
                &prefix,
                &targets,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
                HistoricalTraceFormat::OpenEthereum { trace, vm_trace },
            )
            .unwrap();
            let mut expected = reference[name].clone();
            if !vm_trace {
                for row in expected.as_array_mut().unwrap() {
                    row["vmTrace"] = Value::Null;
                }
            }
            let actual: Value = serde_json::from_slice(&result.output).unwrap();
            if actual != expected {
                mismatches.push(json!({ "name": name, "vmTrace": vm_trace, "trace": trace, "actual": actual, "expected": expected }));
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{}",
        serde_json::to_string(&mismatches).unwrap()
    );
}
