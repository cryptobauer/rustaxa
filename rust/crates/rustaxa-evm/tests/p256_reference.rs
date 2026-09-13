//! Differential P-256 primitive coverage against both pinned Go revisions.
//!
//! The source module is included directly so this independently owned slice
//! can validate before the lead adds the shared crate export and frame route.
//! Registry activation and frame settlement remain separate integration gates.

mod contracts {
    pub use rustaxa_evm::contracts::*;
}

#[path = "../src/p256.rs"]
mod p256;

use num_bigint::BigUint;
use p256::{P256_VERIFY_ADDRESS, P256_VERIFY_GAS, PreparedP256Call};
use rustaxa_evm::contracts::{
    ExecutionValue, NativeCallKind, NativeInvocationResult, NativeStatus, StatelessInvocation,
    StatelessInvocationId,
};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/p256/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/p256/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
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
fn p256_matches_go_quotes_outputs_and_invalid_completion() {
    let corpus = corpus();
    for row in corpus["p256"].as_array().unwrap() {
        let output = hex::decode(row["output"].as_str().unwrap()).unwrap();
        assert_eq!(row["required_gas"], P256_VERIFY_GAS.as_u64());
        assert_eq!(row["error"], "");
        for kind in [
            NativeCallKind::Call,
            NativeCallKind::CallCode,
            NativeCallKind::DelegateCall,
            NativeCallKind::StaticCall,
        ] {
            for gas in [6_899, 6_900, 6_901] {
                let request = invocation(row, gas, kind);
                let prepared = PreparedP256Call::prepare(request.clone()).unwrap();
                assert_eq!(prepared.invocation(), &request, "{}", row["name"]);
                let quote = prepared.quote();
                assert_eq!(quote.invocation, request.id);
                assert_eq!(quote.required_gas, P256_VERIFY_GAS);
                let result = prepared.invoke().unwrap();
                result.validate(&request, quote).unwrap();
                if gas < P256_VERIFY_GAS.as_u64() {
                    assert_eq!(
                        result,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: P256_VERIFY_GAS,
                        },
                        "{}",
                        row["name"]
                    );
                    continue;
                }
                let NativeInvocationResult::Completed(outcome) = result else {
                    panic!("funded P-256 call did not run: {}", row["name"])
                };
                assert_eq!(outcome.status, NativeStatus::Success, "{}", row["name"]);
                assert_eq!(outcome.gas_used, P256_VERIFY_GAS, "{}", row["name"]);
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
fn p256_corpus_pins_exact_length_scalars_keys_and_high_s() {
    let corpus = corpus();
    let rows = corpus["p256"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|row| row["name"] == name).unwrap();
    let true_word = "00".repeat(31) + "01";

    assert_eq!(row("valid")["output"], true_word);
    assert_eq!(row("high-s-valid")["output"], true_word);
    for name in [
        "empty",
        "zero-159",
        "zero-160",
        "zero-161",
        "valid-truncated",
        "valid-trailing",
        "wrong-message",
        "r-zero",
        "r-order",
        "s-zero",
        "s-order",
        "public-x-zero",
        "public-y-zero",
        "public-x-field-prime",
        "public-y-field-prime",
    ] {
        assert_eq!(row(name)["output"], "", "{name}");
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
