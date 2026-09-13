//! Differential primitive coverage against both independently executed Go pins.
//! Registry activation, frame settlement and production routing remain separate.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionValue, NativeCallKind, NativeContractFailure, NativeInvocationResult,
        NativeStatus, StatelessInvocation, StatelessInvocationId,
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
        "../../../../experiments/evm_feasibility/fixtures/curve_precompiles_reference_public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/curve_precompiles_reference_local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public
}

#[test]
fn curve_precompiles_match_go_quotes_outputs_and_errors() {
    let corpus = corpus();
    let rows = corpus["curve_precompiles"].as_array().unwrap();
    for row in rows {
        let required = row["required_gas"].as_u64().unwrap();
        let error = row["error"].as_str().unwrap();
        let output = hex::decode(row["output"].as_str().unwrap()).unwrap();
        let mut gas_limits = vec![required, required.saturating_add(1)];
        if let Some(insufficient) = required.checked_sub(1) {
            gas_limits.push(insufficient);
        }
        gas_limits.sort_unstable();
        gas_limits.dedup();

        for kind in [
            NativeCallKind::Call,
            NativeCallKind::CallCode,
            NativeCallKind::DelegateCall,
            NativeCallKind::StaticCall,
        ] {
            for gas in &gas_limits {
                let request = invocation(row, *gas, kind);
                let prepared = PreparedCurvePrecompileCall::prepare(request.clone()).unwrap();
                assert_eq!(prepared.invocation(), &request, "{}", row["name"]);
                let quote = prepared.quote();
                assert_eq!(quote.invocation, request.id);
                assert_eq!(quote.required_gas.as_u64(), required, "{}", row["name"]);
                let result = prepared.invoke().unwrap();
                result.validate(&request, quote).unwrap();

                if *gas < required {
                    assert_eq!(
                        result,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: required.into(),
                        },
                        "{}",
                        row["name"]
                    );
                    continue;
                }

                let NativeInvocationResult::Completed(outcome) = result else {
                    panic!("funded primitive did not run: {}", row["name"])
                };
                let expected_status = if error.is_empty() {
                    NativeStatus::Success
                } else {
                    NativeStatus::ContractFailure(NativeContractFailure {
                        error: error.to_owned(),
                    })
                };
                assert_eq!(outcome.status, expected_status, "{}", row["name"]);
                assert_eq!(outcome.gas_used.as_u64(), required, "{}", row["name"]);
                assert_eq!(outcome.output, output, "{}", row["name"]);
                assert!(outcome.account_mutations.is_empty());
                assert!(outcome.raw_mutations.is_empty());
                assert!(outcome.logs.is_empty());
                assert_eq!(outcome.diagnostic, None);
            }
        }
    }
}

#[test]
fn corpus_pins_padding_pairing_and_blake_boundaries() {
    let corpus = corpus();
    let rows = corpus["curve_precompiles"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|row| row["name"] == name).unwrap();

    assert_eq!(
        row("add-generator-infinity")["output"],
        row("add-trailing-ignored")["output"]
    );
    assert_eq!(
        row("mul-generator-one")["output"],
        row("mul-trailing-ignored")["output"]
    );
    assert_ne!(
        row("mul-truncated-scalar")["output"],
        row("mul-generator-one")["output"]
    );
    assert_eq!(
        row("pairing-empty-true")["output"],
        row("pairing-negated-product-true")["output"]
    );
    assert_ne!(
        row("pairing-empty-true")["output"],
        row("pairing-one-false")["output"]
    );
    assert_eq!(row("pairing-bad-length-191")["required_gas"], 100_000);
    assert_eq!(row("pairing-bad-length-193")["required_gas"], 180_000);
    assert_eq!(row("blake-short-212")["required_gas"], 0);
    assert_eq!(row("blake-invalid-final")["required_gas"], 2);
    assert_eq!(
        row("blake-known-abc-final")["output"],
        "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923"
    );
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
