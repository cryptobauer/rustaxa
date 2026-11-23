//! Differential primitive coverage against both independently executed Go pins.
//! Frame-level native gas/rollback and the full historical registry are separate
//! tests; this corpus covers only prepared address-5 operations.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        ExecutionValue, NativeCallKind, NativeInvocationResult, StatelessInvocation,
        StatelessInvocationId,
    },
    modexp::PreparedModexpCall,
};
use serde_json::Value;

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    let mut address = [0; 20];
    address[19] = 5;
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
