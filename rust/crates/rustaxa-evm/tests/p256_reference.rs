//! P-256 address selection checks.
//!
//! Registry activation and frame settlement use separate integration tests.

use num_bigint::BigUint;
use rustaxa_evm::contracts::{
    ExecutionValue, NativeCallKind, StatelessInvocation, StatelessInvocationId,
};
use rustaxa_evm::p256::{P256_VERIFY_ADDRESS, PreparedP256Call};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!("fixtures/p256/public.json")).unwrap();
    public
}

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 17_u32.into(),
            ordinal: 19,
        },
        period: 25_706_949_u64.into(),
        depth: 7,
        kind,
        is_static: true,
        caller: [0x11; 20],
        contract: P256_VERIFY_ADDRESS,
        state_address: [0x22; 20],
        value: ExecutionValue::new((BigUint::from(1_u8) << 260_usize) + BigUint::from(9_u8)),
        input: hex::decode(row["input"].as_str().unwrap()).unwrap(),
        supplied_gas: gas.into(),
    }
}

#[test]
fn p256_selection_requires_exact_0x0100_address() {
    let corpus = corpus();
    let row = &corpus["p256"].as_array().unwrap()[0];
    let mut request = invocation(row, 6_900, NativeCallKind::Call);
    assert!(PreparedP256Call::prepare(request.clone()).is_ok());

    for address in [
        [0_u8; 20],
        {
            let mut value = P256_VERIFY_ADDRESS;
            value[19] = 1;
            value
        },
        {
            let mut value = P256_VERIFY_ADDRESS;
            value[0] = 1;
            value
        },
    ] {
        request.contract = address;
        assert!(PreparedP256Call::prepare(request.clone()).is_err());
    }
}
