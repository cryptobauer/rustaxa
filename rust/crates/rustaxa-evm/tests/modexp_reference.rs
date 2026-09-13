//! Differential primitive coverage against both independently executed Go pins.
//! Frame-level native gas/rollback and the full historical registry are separate
//! tests; this corpus covers only prepared address-5 operations.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionValue, NativeCallKind, NativeInvocation, NativeInvocationId,
        NativeInvocationResult, NativeStatus,
    },
    modexp::PreparedModexpCall,
};
use serde_json::Value;

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> NativeInvocation {
    let mut address = [0; 20];
    address[19] = 5;
    NativeInvocation {
        id: NativeInvocationId {
            transaction: 3_u32.into(),
            sequence: 7,
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
fn modexp_matches_go_full_width_quotes_and_bounded_outputs() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/modexp_public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/modexp_local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    for row in public["modexp"].as_array().unwrap() {
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
            for gas in [
                Some(required),
                required.checked_sub(1),
                required.checked_add(1),
            ]
            .into_iter()
            .flatten()
            {
                let request = invocation(row, gas, kind);
                let prepared = PreparedModexpCall::prepare(request.clone()).unwrap();
                assert_eq!(
                    prepared.invocation(),
                    &request,
                    "all request facts stay bound"
                );
                let quote = prepared.quote();
                assert_eq!(quote.invocation, request.id);
                assert_eq!(quote.required_gas.as_u64(), required, "{}", row["case"]);
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
                        row["case"]
                    );
                    assert!(outcome.account_mutations.is_empty());
                    assert!(outcome.raw_mutations.is_empty());
                    assert!(outcome.logs.is_empty());
                }
            }
        }
    }
}

#[test]
fn huge_unfunded_operands_are_not_allocated() {
    let row = serde_json::json!({"input": "ff".repeat(96)});
    let request = invocation(&row, u64::MAX - 1, NativeCallKind::Call);
    let prepared = PreparedModexpCall::prepare(request).unwrap();
    assert_eq!(prepared.quote().required_gas.as_u64(), u64::MAX);
    assert_eq!(
        prepared.invoke().unwrap(),
        NativeInvocationResult::InsufficientGas {
            required_gas: u64::MAX.into(),
        }
    );
    let mut request = invocation(&row, 0, NativeCallKind::Call);
    request.contract[19] = 4;
    assert!(PreparedModexpCall::prepare(request.clone()).is_err());
    request.contract[19] = 5;
    request.contract[0] = 1;
    assert!(PreparedModexpCall::prepare(request).is_err());
}
