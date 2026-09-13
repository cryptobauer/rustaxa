//! Differential primitive coverage against both independently executed Go pins.
//! Historical classification, frame settlement and production routing remain separate.

use num_bigint::BigUint;
use rustaxa_evm::{
    bls::{BlsPrecompile, BlsRegistry, PreparedBlsCall},
    contracts::{
        ExecutionValue, NativeCallKind, NativeContractFailure, NativeInvocationResult,
        NativeStatus, StatelessInvocation, StatelessInvocationId,
    },
};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/bls/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/bls/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public
}

fn registry(row: &Value) -> BlsRegistry {
    match row["registry"].as_str().unwrap() {
        "ficus" => BlsRegistry::Ficus,
        "cacti" => BlsRegistry::Cacti,
        other => panic!("unexpected registry {other}"),
    }
}

fn primitive(row: &Value) -> BlsPrecompile {
    match row["operation"].as_str().unwrap() {
        "g1_add" => BlsPrecompile::G1Add,
        "g1_mul" => BlsPrecompile::G1Mul,
        "g1_multiexp" => BlsPrecompile::G1MultiExp,
        "g2_add" => BlsPrecompile::G2Add,
        "g2_mul" => BlsPrecompile::G2Mul,
        "g2_multiexp" => BlsPrecompile::G2MultiExp,
        "pairing" => BlsPrecompile::Pairing,
        "map_g1" => BlsPrecompile::MapG1,
        "map_g2" => BlsPrecompile::MapG2,
        other => panic!("unexpected operation {other}"),
    }
}

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    let mut address = [0_u8; 20];
    address[19] = row["address"].as_u64().unwrap().try_into().unwrap();
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 17_u32.into(),
            ordinal: 23,
        },
        period: 25_706_949_u64.into(),
        depth: 4,
        kind,
        is_static: true,
        caller: [0x11; 20],
        contract: address,
        state_address: [0x22; 20],
        value: ExecutionValue::new((BigUint::from(1_u8) << 260_usize) + BigUint::from(19_u8)),
        input: fixture_input(row),
        supplied_gas: gas.into(),
    }
}

fn fixture_input(row: &Value) -> Vec<u8> {
    if let Some(repeat) = row.get("repeat") {
        let element = hex::decode(repeat["element"].as_str().unwrap()).unwrap();
        element.repeat(repeat["count"].as_u64().unwrap().try_into().unwrap())
    } else {
        hex::decode(row["input"].as_str().unwrap()).unwrap()
    }
}

fn row<'a>(rows: &'a [Value], registry: &str, name: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["registry"] == registry && row["name"] == name)
        .unwrap()
}

#[test]
fn bls_primitives_match_go_quotes_outputs_and_exact_errors() {
    let corpus = corpus();
    for row in corpus["bls"].as_array().unwrap() {
        let required = row["required_gas"].as_u64().unwrap();
        let request = invocation(row, required, NativeCallKind::Call);
        let prepared = PreparedBlsCall::prepare(registry(row), request.clone()).unwrap();
        assert_eq!(prepared.registry(), registry(row), "{}", row["name"]);
        assert_eq!(prepared.primitive(), primitive(row), "{}", row["name"]);
        assert_eq!(prepared.invocation(), &request, "{}", row["name"]);
        let quote = prepared.quote();
        assert_eq!(quote.invocation, request.id);
        assert_eq!(quote.required_gas.as_u64(), required, "{}", row["name"]);

        let result = prepared.invoke().unwrap();
        result.validate(&request, quote).unwrap();
        let NativeInvocationResult::Completed(outcome) = result else {
            panic!("funded BLS primitive did not execute: {}", row["name"])
        };
        let error = row["error"].as_str().unwrap();
        let expected_status = if error.is_empty() {
            NativeStatus::Success
        } else {
            NativeStatus::ContractFailure(NativeContractFailure {
                error: error.into(),
            })
        };
        assert_eq!(outcome.status, expected_status, "{}", row["name"]);
        assert_eq!(outcome.gas_used.as_u64(), required, "{}", row["name"]);
        assert_eq!(
            outcome.output,
            hex::decode(row["output"].as_str().unwrap()).unwrap(),
            "{}",
            row["name"]
        );
        assert!(outcome.account_mutations.is_empty());
        assert!(outcome.raw_mutations.is_empty());
        assert!(outcome.logs.is_empty());
        assert_eq!(outcome.diagnostic, None);

        if required != 0 {
            let underfunded = invocation(row, required - 1, NativeCallKind::Call);
            let result = PreparedBlsCall::prepare(registry(row), underfunded)
                .unwrap()
                .invoke()
                .unwrap();
            assert_eq!(
                result,
                NativeInvocationResult::InsufficientGas {
                    required_gas: required.into(),
                },
                "{}",
                row["name"]
            );
        }
    }
}

#[test]
fn registry_remap_is_exact_and_does_not_classify_history() {
    use BlsPrecompile::*;
    let ficus = [
        G1Add, G1Mul, G1MultiExp, G2Add, G2Mul, G2MultiExp, Pairing, MapG1, MapG2,
    ];
    for (offset, expected) in ficus.into_iter().enumerate() {
        let mut address = [0_u8; 20];
        address[19] = 0x0b + offset as u8;
        assert_eq!(
            BlsPrecompile::at_address(BlsRegistry::Ficus, address),
            Some(expected)
        );
    }

    let cacti = [G1Add, G1MultiExp, G2Add, G2MultiExp, Pairing, MapG1, MapG2];
    for (offset, expected) in cacti.into_iter().enumerate() {
        let mut address = [0_u8; 20];
        address[19] = 0x0b + offset as u8;
        assert_eq!(
            BlsPrecompile::at_address(BlsRegistry::Cacti, address),
            Some(expected)
        );
    }
    for number in [0x0a, 0x12, 0x13, 0xff] {
        let mut address = [0_u8; 20];
        address[19] = number;
        assert_eq!(BlsPrecompile::at_address(BlsRegistry::Cacti, address), None);
    }
    let mut lookalike = [0_u8; 20];
    lookalike[0] = 1;
    lookalike[19] = 0x0b;
    assert_eq!(
        BlsPrecompile::at_address(BlsRegistry::Ficus, lookalike),
        None
    );
    assert_eq!(
        BlsPrecompile::at_address(BlsRegistry::Cacti, lookalike),
        None
    );
}

#[test]
fn corpus_pins_discount_cap_subgroups_infinity_and_field_checks() {
    let corpus = corpus();
    let rows = corpus["bls"].as_array().unwrap();

    assert_eq!(
        row(rows, "ficus", "g1-multiexp-k128-discount")["required_gas"],
        267_264
    );
    assert_eq!(
        row(rows, "ficus", "g1-multiexp-k129-cap")["required_gas"],
        267_264
    );
    assert_eq!(
        row(rows, "cacti", "g2-multiexp-k128-discount")["required_gas"],
        1_224_960
    );
    assert_eq!(
        row(rows, "cacti", "g2-multiexp-k129-cap")["required_gas"],
        1_224_960
    );

    for registry in ["ficus", "cacti"] {
        assert_eq!(row(rows, registry, "g1-add-non-subgroup")["error"], "");
        assert_eq!(
            row(rows, registry, "g1-multiexp-non-subgroup-one")["error"],
            ""
        );
        assert_eq!(row(rows, registry, "g2-add-non-subgroup")["error"], "");
        assert_eq!(
            row(rows, registry, "g2-multiexp-non-subgroup-one")["error"],
            ""
        );
        assert_eq!(
            row(rows, registry, "pairing-g1-non-subgroup")["error"],
            "g1 point is not on correct subgroup"
        );
        assert_eq!(
            row(rows, registry, "pairing-g2-non-subgroup")["error"],
            "g2 point is not on correct subgroup"
        );
        assert_eq!(
            row(rows, registry, "pairing-infinity-true")["output"],
            format!("{}01", "00".repeat(31))
        );
        assert_eq!(
            row(rows, registry, "map-g1-bad-top")["error"],
            "invalid field element top bytes"
        );
        assert_eq!(
            row(rows, registry, "map-g1-noncanonical-field")["error"],
            "invalid fp.Element encoding"
        );
    }
}

#[test]
fn prepared_call_retains_call_family_context() {
    let corpus = corpus();
    let rows = corpus["bls"].as_array().unwrap();
    let row = row(rows, "cacti", "map-g2-one-two");
    let required = row["required_gas"].as_u64().unwrap();
    for kind in [
        NativeCallKind::Call,
        NativeCallKind::CallCode,
        NativeCallKind::DelegateCall,
        NativeCallKind::StaticCall,
    ] {
        let request = invocation(row, required + 1, kind);
        let prepared = PreparedBlsCall::prepare(BlsRegistry::Cacti, request.clone()).unwrap();
        assert_eq!(prepared.invocation(), &request);
        let quote = prepared.quote();
        let result = prepared.invoke().unwrap();
        result.validate(&request, quote).unwrap();
    }
}
