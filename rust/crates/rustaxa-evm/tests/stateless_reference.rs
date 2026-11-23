//! Differential primitive coverage against both independently executed Go pins.
//! Frame-level native gas/rollback and the full historical registry are separate
//! tests; this corpus covers only prepared original stateless operations.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{ExecutionValue, NativeCallKind, StatelessInvocation, StatelessInvocationId},
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
