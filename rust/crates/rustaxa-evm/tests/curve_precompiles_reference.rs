//! Differential primitive coverage against both independently executed Go pins.
//! Registry activation, frame settlement and production routing remain separate.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionValue, NativeCallKind, NativeInvocationResult, StatelessInvocation,
        StatelessInvocationId,
    },
    curve_precompiles::{OriginalCurvePrecompile, PreparedCurvePrecompileCall},
};
use serde_json::Value;

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    let mut address = [0_u8; 20];
    address[19] = row["address"].as_u64().unwrap().try_into().unwrap();
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 11_u32.into(),
            ordinal: 13,
        },
        period: 25_706_949_u64.into(),
        depth: 3,
        kind,
        is_static: true,
        caller: [0x11; 20],
        contract: address,
        state_address: [0x22; 20],
        value: ExecutionValue::new((BigUint::from(1_u8) << 260_usize) + BigUint::from(9_u8)),
        input: hex::decode(row["input"].as_str().unwrap()).unwrap(),
        supplied_gas: gas.into(),
    }
}

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "fixtures/curve_precompiles_reference_public.json"
    ))
    .unwrap();
    public
}

#[test]
fn insufficient_gas_precedes_primitive_validation() {
    for name in [
        "add-off-curve-first",
        "pairing-invalid-g2",
        "blake-invalid-final",
    ] {
        let corpus = corpus();
        let row = corpus["curve_precompiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == name)
            .unwrap();
        let required = row["required_gas"].as_u64().unwrap();
        let request = invocation(row, required - 1, NativeCallKind::Call);
        let result = PreparedCurvePrecompileCall::prepare(request)
            .unwrap()
            .invoke()
            .unwrap();
        assert_eq!(
            result,
            NativeInvocationResult::InsufficientGas {
                required_gas: required.into(),
            }
        );
    }
}

#[test]
fn primitive_selection_does_not_activate_a_registry() {
    for number in [6, 7, 8, 9] {
        let mut address = [0_u8; 20];
        address[19] = number;
        assert!(OriginalCurvePrecompile::at_address(address).is_some());
    }
    for number in [0, 5, 10, 0xfe] {
        let mut address = [0_u8; 20];
        address[19] = number;
        assert_eq!(OriginalCurvePrecompile::at_address(address), None);
    }
    let mut lookalike = [0_u8; 20];
    lookalike[0] = 1;
    lookalike[19] = 9;
    assert_eq!(OriginalCurvePrecompile::at_address(lookalike), None);

    let row = serde_json::json!({"address": 10, "input": ""});
    let mut request = invocation(&row, 0, NativeCallKind::Call);
    assert!(PreparedCurvePrecompileCall::prepare(request.clone()).is_err());
    request.contract = lookalike;
    assert!(PreparedCurvePrecompileCall::prepare(request).is_err());
}
