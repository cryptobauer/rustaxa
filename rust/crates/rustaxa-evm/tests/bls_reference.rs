//! Differential primitive coverage against both independently executed Go pins.
//! Historical classification, frame settlement and production routing remain separate.

use num_bigint::BigUint;
use rustaxa_evm::{
    bls::{BlsPrecompile, BlsRegistry, PreparedBlsCall},
    contracts::{ExecutionValue, NativeCallKind, StatelessInvocation, StatelessInvocationId},
};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!("fixtures/bls/public.json")).unwrap();
    public
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
