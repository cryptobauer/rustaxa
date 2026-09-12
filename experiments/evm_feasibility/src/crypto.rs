//! Historical FN-DSA compatibility probes. Public deterministic Go vectors are
//! checked against two pinned Rust verifier versions. ABI acceptance is separate
//! from cryptographic validity; notably the reference rejects empty messages.
#[test]
fn historical_falcon_verifier_comparison() {
    use fn_dsa_vrfy::VerifyingKey as _;
    use fn_dsa_vrfy_legacy::VerifyingKey as _;
    let f: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/local.json")).unwrap();
    let mut legacy_mismatches = 0;
    let mut current_mismatches = 0;
    for row in f["falcon"].as_array().unwrap() {
        let decode = |key: &str| hex::decode(row[key].as_str().unwrap()).unwrap();
        let key = decode("key");
        let sig = decode("signature");
        let msg = decode("message");
        let expected = row["valid"].as_bool().unwrap();
        let legacy = fn_dsa_vrfy_legacy::VerifyingKeyStandard::decode(&key).is_some_and(|k| {
            k.verify(
                &sig,
                &fn_dsa_vrfy_legacy::DOMAIN_NONE,
                &fn_dsa_vrfy_legacy::HASH_ID_RAW,
                &msg,
            )
        });
        let current = fn_dsa_vrfy::VerifyingKeyStandard::decode(&key).is_some_and(|k| {
            k.verify(
                &sig,
                &fn_dsa_vrfy::DOMAIN_NONE,
                &fn_dsa_vrfy::HASH_ID_RAW,
                &msg,
            )
        });
        legacy_mismatches += usize::from(legacy != expected);
        current_mismatches += usize::from(current != expected);
        println!(
            "{} Go={expected} Rust0.3={legacy} Rust0.4={current}",
            row["case"]
        );
        let mut word = [0u8; 32];
        word[31] = u8::from(!expected || msg.is_empty());
        assert_eq!(hex::encode(word), row["return"].as_str().unwrap());
        let abi = decode("abi");
        let intrinsic = 21000
            + abi
                .iter()
                .map(|b| if *b == 0 { 4 } else { 68 })
                .sum::<u64>();
        assert_eq!(
            intrinsic + 1465 + 6 * (abi.len() as u64).div_ceil(32),
            row["gas_used"].as_u64().unwrap()
        );
        assert_eq!(row["error"], "");
    }
    println!("mismatches legacy={legacy_mismatches} current={current_mismatches}");
    assert_eq!(
        current_mismatches, 3,
        "current verifier rejects historical valid signatures"
    );
    assert_eq!(
        legacy_mismatches, 0,
        "historical verifier must match reference"
    );
}
