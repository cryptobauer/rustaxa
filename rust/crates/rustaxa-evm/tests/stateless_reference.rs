//! Differential primitive coverage against both independently executed Go pins.
//! Frame-level native gas/rollback and the full historical registry are separate
//! tests; this corpus covers only prepared original stateless operations.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionValue, NativeCallKind, NativeInvocationResult, NativeStatus, StatelessInvocation,
        StatelessInvocationId,
    },
    stateless::{OriginalStatelessPrecompile, PreparedStatelessCall},
};
use serde_json::Value;

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    let mut address = [0; 20];
    address[19] = row["address"].as_u64().unwrap().try_into().unwrap();
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 3_u32.into(),
            ordinal: 7,
        },
        period: 25_706_949_u64.into(),
        depth: 2,
        kind,
        is_static: true,
        caller: [0x11; 20],
        contract: address,
        state_address: [0x22; 20],
        value: ExecutionValue::new((BigUint::from(1_u8) << 260_usize) + BigUint::from(7_u8)),
        input: hex::decode(row["input"].as_str().unwrap()).unwrap(),
        supplied_gas: gas.into(),
    }
}

#[test]
fn original_precompiles_match_go_bytes_and_quotes() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/stateless_public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/stateless_local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    for row in public["stateless"].as_array().unwrap() {
        let required = row["required_gas"].as_u64().unwrap();
        assert_eq!(
            row["error"], "",
            "primitive corpus includes normal empty-output failures"
        );
        for kind in [
            NativeCallKind::Call,
            NativeCallKind::CallCode,
            NativeCallKind::DelegateCall,
            NativeCallKind::StaticCall,
        ] {
            for gas in [required - 1, required, required + 1] {
                let request = invocation(row, gas, kind);
                let prepared = PreparedStatelessCall::prepare(request.clone()).unwrap();
                assert_eq!(
                    prepared.invocation(),
                    &request,
                    "all request facts stay bound"
                );
                let quote = prepared.quote();
                assert_eq!(quote.invocation, request.id);
                assert_eq!(quote.required_gas.as_u64(), required, "{}", row["name"]);
                let result = prepared.invoke().unwrap();
                result.validate(&request, quote).unwrap();
                if gas < required {
                    assert_eq!(
                        result,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: required.into()
                        }
                    );
                } else {
                    let NativeInvocationResult::Completed(outcome) = result else {
                        panic!("funded primitive did not run")
                    };
                    assert_eq!(outcome.status, NativeStatus::Success);
                    assert_eq!(outcome.gas_used.as_u64(), required);
                    assert_eq!(
                        outcome.output,
                        hex::decode(row["output"].as_str().unwrap()).unwrap(),
                        "{}",
                        row["name"]
                    );
                    assert!(outcome.account_mutations.is_empty());
                    assert!(outcome.raw_mutations.is_empty());
                    assert!(outcome.logs.is_empty());
                }
            }
        }
    }
    let rows = public["stateless"].as_array().unwrap();
    let output = |name| &rows.iter().find(|row| row["name"] == name).unwrap()["output"];
    assert_ne!(output("valid-low-s"), "");
    assert_eq!(output("valid-low-s"), output("valid-high-s"));
    assert_eq!(output("valid-low-s"), output("trailing-byte"));
    for name in [
        "r-zero",
        "s-zero",
        "r-order",
        "s-order",
        "invalid-v",
        "nonzero-v-padding",
    ] {
        assert_eq!(output(name), "");
    }
}

#[test]
fn primitive_selection_does_not_infer_the_full_native_registry() {
    for number in [0, 5, 9, 0xfe] {
        let mut address = [0; 20];
        address[19] = number;
        assert_eq!(OriginalStatelessPrecompile::at_address(address), None);
    }
    let mut lookalike = [0; 20];
    lookalike[0] = 1;
    lookalike[19] = 1;
    assert_eq!(OriginalStatelessPrecompile::at_address(lookalike), None);
    let mut request = invocation(
        &serde_json::json!({"address": 5, "input": ""}),
        10_000,
        NativeCallKind::Call,
    );
    assert!(PreparedStatelessCall::prepare(request.clone()).is_err());
    request.contract = lookalike;
    assert!(PreparedStatelessCall::prepare(request).is_err());
}
