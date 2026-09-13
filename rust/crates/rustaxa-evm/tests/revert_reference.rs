//! Exact pinned ABI reason bytes, separate from RPC JSON string normalization.

use rustaxa_evm::revert::{dry_run_revert_diagnostic, revert_reason_bytes};
use serde_json::Value;

#[test]
fn revert_diagnostics_match_both_go_abi_decoders() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/revert/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/revert/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    let rows = public.as_array().unwrap();
    assert_eq!(rows.len(), 17);
    for row in rows {
        let input = hex::decode(row["input"].as_str().unwrap()).unwrap();
        let reason = hex::decode(row["reason"].as_str().unwrap()).unwrap();
        let diagnostic = hex::decode(row["diagnostic"].as_str().unwrap()).unwrap();
        let expected = row["valid"].as_bool().unwrap().then_some(reason.as_slice());
        assert_eq!(revert_reason_bytes(&input), expected, "{}", row["name"]);
        assert_eq!(
            dry_run_revert_diagnostic(&input),
            diagnostic,
            "{}",
            row["name"]
        );
    }
    let invalid_utf8 = rows.iter().find(|row| row["name"] == "non-utf8").unwrap();
    assert_eq!(invalid_utf8["reason"], "f09f00ff");
    assert_eq!(invalid_utf8["valid"], true);
}
