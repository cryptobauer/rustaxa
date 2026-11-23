//! Falcon address selection checks.
//!
//! The shared crate export is the same primitive used by the frame driver.
//! Registry activation and frame settlement remain separate integration gates.

use num_bigint::BigUint;
use rustaxa_evm::contracts::{
    ExecutionValue, NativeCallKind, StatelessInvocation, StatelessInvocationId,
};
use rustaxa_evm::falcon::{FALCON_BASE_GAS, FALCON_VERIFY_ADDRESS, PreparedFalconCall};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!("fixtures/falcon/public.json"))
        .expect("pinned public Falcon fixture must be valid JSON");
    let local: Value = serde_json::from_str(include_str!("fixtures/falcon/local.json"))
        .expect("pinned local Falcon fixture must be valid JSON");
    assert_eq!(
        public, local,
        "both Go pins must emit identical Falcon rows"
    );
    public
}

fn invocation(row: &Value, gas: u64, kind: NativeCallKind) -> StatelessInvocation {
    StatelessInvocation {
        id: StatelessInvocationId {
            transaction: 23_u32.into(),
            ordinal: 29,
        },
        period: 25_706_949_u64.into(),
        depth: 11,
        kind,
        is_static: true,
        caller: [0x11; 20],
        contract: FALCON_VERIFY_ADDRESS,
        state_address: [0x22; 20],
        value: ExecutionValue::new((BigUint::from(1_u8) << 300_usize) + BigUint::from(7_u8)),
        input: hex::decode(row["input"].as_str().expect("hex input"))
            .expect("fixture input must be valid hex"),
        supplied_gas: gas.into(),
    }
}

/// Rejects every lookalike except the exact Cacti address `0xfa1c`.
#[test]
fn falcon_selection_requires_exact_fa1c_address() {
    let corpus = corpus();
    let row = &corpus["falcon"].as_array().unwrap()[0];
    let mut request = invocation(row, FALCON_BASE_GAS, NativeCallKind::Call);
    assert!(PreparedFalconCall::prepare(request.clone()).is_ok());

    for address in [
        [0_u8; 20],
        {
            let mut value = FALCON_VERIFY_ADDRESS;
            value[19] = 0x1d;
            value
        },
        {
            let mut value = FALCON_VERIFY_ADDRESS;
            value[0] = 1;
            value
        },
    ] {
        request.contract = address;
        assert!(PreparedFalconCall::prepare(request.clone()).is_err());
    }
}
