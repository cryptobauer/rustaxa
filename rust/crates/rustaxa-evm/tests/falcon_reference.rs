//! Differential Falcon-512 coverage against both pinned Taraxa Go revisions.
//!
//! The source module is included directly so this independently owned slice
//! can validate before the lead adds the shared crate export and Cacti route.
//! Registry activation and frame settlement remain separate integration gates.

mod contracts {
    pub use rustaxa_evm::contracts::*;
}

#[path = "../src/falcon.rs"]
mod falcon;

use falcon::{FALCON_BASE_GAS, FALCON_PER_WORD_GAS, FALCON_VERIFY_ADDRESS, PreparedFalconCall};
use num_bigint::BigUint;
use rustaxa_evm::contracts::{
    ExecutionValue, NativeCallKind, NativeInvocationResult, NativePortError, NativeStatus,
    StatelessInvocation, StatelessInvocationId,
};
use serde_json::Value;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/falcon/public.json"
    ))
    .expect("pinned public Falcon fixture must be valid JSON");
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/falcon/local.json"
    ))
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

/// Compares the quote and funded result of every direct Go invocation.
#[test]
fn falcon_matches_go_quotes_abi_crypto_and_failures() {
    let corpus = corpus();
    assert_eq!(corpus["address"], hex::encode(FALCON_VERIFY_ADDRESS));
    assert_eq!(corpus["signature_size"], 666);
    assert_eq!(corpus["verifying_key_size"], 897);
    assert_eq!(corpus["method_selector"], "de8f50a1");
    let rows = corpus["falcon"].as_array().expect("Falcon row array");
    assert_eq!(rows.len(), 33);

    for row in rows {
        let required = row["required_gas"].as_u64().expect("required gas");
        let name = row["name"].as_str().expect("case name");
        let input_length = row["input"].as_str().expect("hex input").len() / 2;
        assert_eq!(
            required,
            FALCON_BASE_GAS
                + FALCON_PER_WORD_GAS * u64::try_from(input_length.div_ceil(32)).unwrap(),
            "{name}: gas schedule"
        );
        for kind in [
            NativeCallKind::Call,
            NativeCallKind::CallCode,
            NativeCallKind::DelegateCall,
            NativeCallKind::StaticCall,
        ] {
            for supplied in [required - 1, required, required + 1] {
                let request = invocation(row, supplied, kind);
                let prepared = PreparedFalconCall::prepare(request.clone()).unwrap();
                assert_eq!(prepared.invocation(), &request, "{name}");
                let quote = prepared.quote();
                assert_eq!(quote.invocation, request.id, "{name}");
                assert_eq!(quote.required_gas.as_u64(), required, "{name}");
                let result = prepared.invoke();
                if supplied < required {
                    let result = result.unwrap();
                    result.validate(&request, quote).unwrap();
                    assert_eq!(
                        result,
                        NativeInvocationResult::InsufficientGas {
                            required_gas: required.into(),
                        },
                        "{name}"
                    );
                    continue;
                }
                if !row["panic"].as_str().expect("panic text").is_empty() {
                    assert_eq!(
                        result.unwrap_err(),
                        NativePortError::Infrastructure("Falcon reference ABI would panic".into()),
                        "{name}"
                    );
                    continue;
                }
                let result = result.unwrap();
                result.validate(&request, quote).unwrap();
                let NativeInvocationResult::Completed(outcome) = result else {
                    panic!("{name}: funded Falcon invocation did not complete")
                };
                assert_eq!(outcome.gas_used.as_u64(), required, "{name}");
                assert_eq!(
                    outcome.output,
                    hex::decode(row["output"].as_str().expect("hex output")).unwrap(),
                    "{name}"
                );
                match row["error"].as_str().expect("error text") {
                    "" => assert_eq!(outcome.status, NativeStatus::Success, "{name}"),
                    error => {
                        let NativeStatus::ContractFailure(failure) = outcome.status else {
                            panic!("{name}: Go contract error became Rust success")
                        };
                        assert_eq!(failure.error, error, "{name}");
                    }
                }
                assert!(outcome.account_mutations.is_empty(), "{name}");
                assert!(outcome.raw_mutations.is_empty(), "{name}");
                assert!(outcome.logs.is_empty(), "{name}");
                assert_eq!(outcome.diagnostic, None, "{name}");
            }
        }
    }
}

/// Pins the compatibility distinctions that are easy to lose in a generic ABI decoder.
#[test]
fn falcon_corpus_pins_noncanonical_abi_and_empty_message() {
    let corpus = corpus();
    let rows = corpus["falcon"].as_array().unwrap();
    let row = |name: &str| rows.iter().find(|row| row["name"] == name).unwrap();
    let valid_word = "00".repeat(32);
    let invalid_word = "00".repeat(31) + "01";

    assert_eq!(row("empty-input")["output"], "");
    assert_eq!(row("empty-input")["error"], "invalid input format");
    assert_eq!(row("wrong-selector")["output"], "");
    assert_eq!(row("wrong-selector")["error"], "invalid method signature");
    assert_eq!(row("historical-empty-message")["cryptographic_valid"], true);
    assert_eq!(row("historical-empty-message")["output"], invalid_word);
    for name in [
        "historical-valid",
        "historical-valid-long-message",
        "go-right-padded-message",
        "signed-message-length-tail",
    ] {
        assert_eq!(row(name)["output"], valid_word, "{name}");
    }
    assert_eq!(row("beyond-go-right-padding")["output"], invalid_word);
    assert_ne!(row("wrapped-message-length-panic")["panic"], "");
    for name in [
        "reordered-fields",
        "unaligned-fields",
        "high-bits-offsets",
        "high-bits-lengths",
        "trailing-bytes",
    ] {
        assert_eq!(row(name)["output"], valid_word, "{name}");
    }
    for name in [
        "selector-only",
        "zero-signature-offset",
        "zero-key-offset",
        "zero-message-offset",
        "wrong-signature-length",
        "wrong-key-length",
        "invalid-signature",
        "invalid-message",
    ] {
        assert_eq!(row(name)["output"], invalid_word, "{name}");
        assert_eq!(row(name)["error"], "", "{name}");
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
