//! Pinned Go SSTORE matrix intake.
//!
//! The full executable host-backed comparison is intentionally left to the
//! journal owner: this additive test preserves the independently generated
//! matrix and its exact historical values without asserting a new Rust policy.

/// Ensures both pinned Go exports agree and retain every requested SSTORE case.
#[test]
fn pinned_go_sstore_matrix_is_complete_and_identical() {
    let local: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_local.json"
    ))
    .expect("local SSTORE fixture JSON");
    let public: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_public.json"
    ))
    .expect("public SSTORE fixture JSON");
    assert_eq!(local, public, "pinned Go references must agree");
    let cases = local["sstore"].as_array().expect("SSTORE cases");
    assert_eq!(cases.len(), 12);
    for expected in [
        "clean0-to0",
        "clean0-to1",
        "dirty0-to1-to0",
        "dirty0-to1-to2",
        "clean7-to7",
        "clean7-to0",
        "clean7-to8",
        "dirty7-to8-to0",
        "dirty7-to8-to7",
        "sentry-2300",
        "sentry-2301",
        "static-rejection",
    ] {
        assert!(
            cases.iter().any(|row| row["case"] == expected),
            "missing {expected}"
        );
    }
}
