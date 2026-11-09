use super::*;

pub(super) struct CasePacket {
    pub api: Value,
    pub cpp: Value,
    pub roster: Value,
    pub rules: Value,
    pub manifest_sha: String,
    pub api_sha: String,
    pub cpp_sha: String,
    pub roster_sha: String,
    pub rules_sha: String,
    pub source_epoch: String,
    pub build_identity: String,
}

fn sha256(raw: &[u8]) -> Result<String> {
    let hash = revm::precompile::hash::sha256_run(raw, u64::MAX)
        .map_err(|error| anyhow::anyhow!("packet SHA-256: {error:?}"))?;
    Ok(hex::encode(hash.bytes))
}

fn load(case_name: &str, fixture_sha: &str) -> Result<CasePacket> {
    let manifest_path = PathBuf::from(std::env::var("RUSTAXA_CRYPTO_PACKET_MANIFEST")?);
    let manifest_sha = std::env::var("RUSTAXA_CRYPTO_PACKET_MANIFEST_SHA256")?;
    let source_epoch = std::env::var("RUSTAXA_CRYPTO_SOURCE_EPOCH")?;
    let build_identity = std::env::var("RUSTAXA_CRYPTO_BUILD_IDENTITY")?;
    let raw = std::fs::read(&manifest_path)?;
    ensure!(sha256(&raw)? == manifest_sha, "packet manifest hash");
    let manifest: Value = serde_json::from_slice(&raw)?;
    ensure!(
        manifest["status"] == "ACCEPTED"
            && manifest["case"] == case_name
            && manifest["fixture_sha256"] == fixture_sha
            && manifest["source_epoch"] == source_epoch
            && manifest["build_identity"] == build_identity,
        "reviewed packet manifest identity"
    );
    let parent = manifest_path.parent().context("packet parent")?;
    let read = |name: &str| -> Result<(Value, String)> {
        let pin = &manifest["streams"][name];
        let file = pin["file"].as_str().context("packet filename")?;
        let mut components = Path::new(file).components();
        ensure!(
            matches!(components.next(), Some(std::path::Component::Normal(_)))
                && components.next().is_none(),
            "local packet filename"
        );
        let path = parent.join(file);
        ensure!(!path.is_symlink(), "packet symlink");
        let bytes = std::fs::read(path)?;
        let hash = sha256(&bytes)?;
        ensure!(
            pin["sha256"] == hash && pin["bytes"] == bytes.len() as u64,
            "packet body pin"
        );
        Ok((serde_json::from_slice(&bytes)?, hash))
    };
    let (api, api_sha) = read("api")?;
    let (cpp, cpp_sha) = read("cpp")?;
    let (roster, roster_sha) = read("roster")?;
    let (rules, rules_sha) = read("rules")?;
    ensure!(
        roster["fixture_sha256"] == fixture_sha && roster["rules_sha256"] == rules_sha,
        "roster fixture and rules identity"
    );
    Ok(CasePacket {
        api,
        cpp,
        roster,
        rules,
        manifest_sha,
        api_sha,
        cpp_sha,
        roster_sha,
        rules_sha,
        source_epoch,
        build_identity,
    })
}

fn qualify(case_name: &str, first_error: &str, second_error: &str) -> Result<()> {
    let (fixture, fixture_sha) = wellformed_proof_fixture(case_name, first_error, second_error)?;
    let packet = load(case_name, &fixture_sha)?;
    api::validate_case_documents(
        &fixture,
        case_name,
        &packet.api,
        &packet.cpp,
        &packet.roster,
        &packet.rules,
    )?;
    run_workload(&fixture, false, false, Some((case_name, &packet)))?;
    let stream = format!("local.observer.{case_name}");
    run_recovery_with_retry(
        &fixture,
        &fixture,
        5,
        Some((&fixture_sha, &fixture_sha)),
        &stream,
        true,
    )?;
    let rows = packet.roster["cases"].as_array().context("roster cases")?;
    let estimates = rows
        .iter()
        .flat_map(|row| row["estimates"].as_array().unwrap())
        .collect::<Vec<_>>();
    let probes: usize = estimates
        .iter()
        .map(|row| row["probe_gases"].as_array().unwrap().len())
        .sum();
    eprintln!(
        "CRYPTO_QUALIFICATION {}",
        serde_json::json!({
            "case": case_name, "source_epoch": packet.source_epoch, "build_identity": packet.build_identity,
            "fixture_sha256": fixture_sha, "manifest_sha256": packet.manifest_sha,
            "api_sha256": packet.api_sha, "cpp_sha256": packet.cpp_sha, "roster_sha256": packet.roster_sha, "rules_sha256": packet.rules_sha,
            "seed_roots": packet.api["seed_roots"], "heads": 13, "requests": rows.len(),
            "estimates": estimates.len(), "probes": probes, "traces": rows.len() * 2,
            "interacting_traces": packet.roster["interacting_traces"].as_array().context("trace roster")?.len(),
            "reopen_after": 9, "split": 5, "faults": ["BeforeCommit", "AfterCommit"],
            "recovery_and_reconciliation": "complete", "final_period": 12
        })
    );
    Ok(())
}

macro_rules! qualification_test {
    ($name:ident, $case:literal, $first:literal, $second:literal) => {
        #[test]
        fn $name() -> Result<()> {
            qualify($case, $first, $second)
        }
    };
}

qualification_test!(
    native_integration_crypto_qualification_default,
    "default",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_high_s_a,
    "high_s_a",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_different_signers,
    "different_signers",
    "Votes validators differ",
    "Votes validators differ"
);

qualification_test!(
    native_integration_crypto_qualification_invalid_signature_a,
    "invalid_signature_a",
    "Invalid vote signature",
    "Invalid vote signature"
);

qualification_test!(
    native_integration_crypto_qualification_invalid_signature_b,
    "invalid_signature_b",
    "Invalid vote signature",
    "Invalid vote signature"
);

qualification_test!(
    native_integration_crypto_qualification_identical_unsigned,
    "identical_unsigned",
    "Votes are identical",
    "Votes are identical"
);

qualification_test!(
    native_integration_crypto_qualification_same_block_distinct_sortition,
    "same_block_distinct_sortition",
    "Invalid votes block hash",
    "Invalid votes block hash"
);

qualification_test!(
    native_integration_crypto_qualification_period_mismatch,
    "period_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);

qualification_test!(
    native_integration_crypto_qualification_round_mismatch,
    "round_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);

qualification_test!(
    native_integration_crypto_qualification_step_mismatch,
    "step_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);

qualification_test!(
    native_integration_crypto_qualification_odd_step_mixed_zero_forward,
    "odd_step_mixed_zero_forward",
    "Invalid votes block hash",
    "Invalid votes block hash"
);

qualification_test!(
    native_integration_crypto_qualification_odd_step_mixed_zero_reversed,
    "odd_step_mixed_zero_reversed",
    "Invalid votes block hash",
    "Invalid votes block hash"
);

qualification_test!(
    native_integration_crypto_qualification_even_step_mixed_zero,
    "even_step_mixed_zero",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_reversed_duplicate,
    "reversed_duplicate",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_duplicate_before_signature,
    "duplicate_before_signature",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_fresh_reverse,
    "fresh_reverse",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_compressed_recovery_4,
    "compressed_recovery_4",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_compressed_recovery_5,
    "compressed_recovery_5",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_current_member_delayed_absent,
    "current_member_delayed_absent",
    "",
    "Existing double voting proof"
);

qualification_test!(
    native_integration_crypto_qualification_current_nonmember_delayed_member,
    "current_nonmember_delayed_member",
    "",
    "Existing double voting proof"
);
