use super::*;

#[path = "native_integration_recovery_packet.rs"]
mod recovery;

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
    recovery: Value,
}

fn sha256(raw: &[u8]) -> Result<String> {
    let hash = revm::precompile::hash::sha256_run(raw, u64::MAX)
        .map_err(|error| anyhow::anyhow!("packet SHA-256: {error:?}"))?;
    Ok(hex::encode(hash.bytes))
}

fn pinned_file(pin: &Value) -> Result<Vec<u8>> {
    let path = Path::new(pin["path"].as_str().context("packet pin path")?);
    ensure!(
        path.is_absolute() && !path.is_symlink(),
        "absolute packet pin without symlink"
    );
    let raw = std::fs::read(path)?;
    ensure!(
        pin["bytes"] == raw.len() as u64 && pin["sha256"] == sha256(&raw)?,
        "packet file pin"
    );
    Ok(raw)
}

fn indexed_manifest(
    index: &Value,
    case_name: &str,
    fixture_sha: &str,
) -> Result<(PathBuf, String, String, String)> {
    ensure!(
        index["schema"] == 1 && index["status"] == "ACCEPTED",
        "reviewed packet index"
    );
    let entry = index["cases"]
        .get(case_name)
        .context("missing case packet index entry")?;
    ensure!(
        entry["fixture_sha256"] == fixture_sha,
        "indexed fixture identity"
    );
    let source = entry["source_epoch"]
        .as_str()
        .filter(|v| !v.is_empty())
        .context("indexed source epoch")?;
    let build = entry["build_identity"]
        .as_str()
        .filter(|v| !v.is_empty())
        .context("indexed build identity")?;
    let manifest = &entry["manifest"];
    let review: Value = serde_json::from_slice(&pinned_file(&entry["review"])?)?;
    ensure!(
        review["status"] == "ACCEPTED"
            && review["case"] == case_name
            && review["fixture_sha256"] == fixture_sha
            && review["source_epoch"] == source
            && review["build_identity"] == build
            && review["manifest_sha256"] == manifest["sha256"],
        "case packet provenance review"
    );
    let evidence = review["producer_evidence"]
        .as_array()
        .context("reviewed producer evidence")?;
    ensure!(!evidence.is_empty(), "producer evidence required");
    for pin in evidence {
        pinned_file(pin)?;
    }
    pinned_file(manifest)?;
    Ok((
        PathBuf::from(manifest["path"].as_str().context("indexed manifest path")?),
        manifest["sha256"]
            .as_str()
            .context("indexed manifest hash")?
            .to_owned(),
        source.to_owned(),
        build.to_owned(),
    ))
}

fn packet_body(stored: Vec<u8>, pin: &Value) -> Result<(Vec<u8>, String)> {
    let bytes = match &pin["encoding"] {
        Value::Null => stored,
        Value::String(encoding) if encoding == "gzip" => {
            use std::io::Read;
            ensure!(
                pin["stored_bytes"] == stored.len() as u64
                    && pin["stored_sha256"] == sha256(&stored)?,
                "stored packet body pin"
            );
            let expected = pin["bytes"].as_u64().context("decoded packet size")?;
            ensure!(expected <= 256 * 1024 * 1024, "decoded packet size bound");
            let mut decoded = Vec::new();
            flate2::read::GzDecoder::new(stored.as_slice())
                .take(expected + 1)
                .read_to_end(&mut decoded)?;
            decoded
        }
        _ => anyhow::bail!("unsupported packet encoding"),
    };
    let hash = sha256(&bytes)?;
    ensure!(
        pin["bytes"] == bytes.len() as u64 && pin["sha256"] == hash,
        "packet body pin"
    );
    Ok((bytes, hash))
}

fn read_stream(parent: &Path, pin: &Value) -> Result<(Value, String)> {
    let file = pin["file"].as_str().context("packet filename")?;
    let mut components = Path::new(file).components();
    ensure!(
        matches!(components.next(), Some(std::path::Component::Normal(_)))
            && components.next().is_none(),
        "local packet filename"
    );
    let path = parent.join(file);
    ensure!(!path.is_symlink(), "packet symlink");
    if pin["encoding"] == "gzip" {
        let stored_len = std::fs::metadata(&path)?.len();
        ensure!(
            stored_len <= 256 * 1024 * 1024 && pin["stored_bytes"] == stored_len,
            "stored packet size bound"
        );
    }
    let (bytes, hash) = packet_body(std::fs::read(path)?, pin)?;
    Ok((serde_json::from_slice(&bytes)?, hash))
}

fn load(case_name: &str, fixture_sha: &str) -> Result<CasePacket> {
    let mut recovery = Value::Null;
    let (manifest_path, manifest_sha, source_epoch, build_identity) =
        if let Some(path) = std::env::var_os("RUSTAXA_CRYPTO_PACKET_INDEX") {
            ensure!(
                std::env::var_os("RUSTAXA_CRYPTO_PACKET_MANIFEST").is_none(),
                "select one packet provisioning route"
            );
            let raw = std::fs::read(path)?;
            ensure!(
                sha256(&raw)? == std::env::var("RUSTAXA_CRYPTO_PACKET_INDEX_SHA256")?,
                "packet index hash"
            );
            let index: Value = serde_json::from_slice(&raw)?;
            recovery = index["cases"][case_name]["recovery"].clone();
            indexed_manifest(&index, case_name, fixture_sha)?
        } else {
            (
                PathBuf::from(std::env::var("RUSTAXA_CRYPTO_PACKET_MANIFEST")?),
                std::env::var("RUSTAXA_CRYPTO_PACKET_MANIFEST_SHA256")?,
                std::env::var("RUSTAXA_CRYPTO_SOURCE_EPOCH")?,
                std::env::var("RUSTAXA_CRYPTO_BUILD_IDENTITY")?,
            )
        };
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
    if recovery.is_null() {
        recovery = manifest["recovery"].clone();
    }
    let parent = manifest_path.parent().context("packet parent")?;
    let read = |name: &str| read_stream(parent, &manifest["streams"][name]);
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
        recovery,
    })
}

#[test]
fn packet_gzip_checks_stored_and_decoded_pins() -> Result<()> {
    use std::io::Write;
    let body = br#"{"test":"packet encoding"}"#;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(body)?;
    let stored = encoder.finish()?;
    let pin = serde_json::json!({"encoding":"gzip","bytes":body.len(),"sha256":sha256(body)?,
        "stored_bytes":stored.len(),"stored_sha256":sha256(&stored)?});
    ensure!(packet_body(stored.clone(), &pin)?.0 == body);
    for field in [
        "sha256",
        "stored_sha256",
        "bytes",
        "stored_bytes",
        "encoding",
    ] {
        let mut bad = pin.clone();
        bad[field] = serde_json::json!("changed");
        ensure!(
            packet_body(stored.clone(), &bad).is_err(),
            "reject changed {field}"
        );
    }
    let mut short = pin.clone();
    short["bytes"] = serde_json::json!(body.len() - 1);
    ensure!(packet_body(stored.clone(), &short).is_err());
    let mut large = pin.clone();
    large["bytes"] = serde_json::json!(256 * 1024 * 1024 + 1u64);
    ensure!(packet_body(stored.clone(), &large).is_err());
    let mut crc_changed = stored.clone();
    let crc_index = crc_changed.len() - 8;
    crc_changed[crc_index] ^= 1;
    for broken in [stored[..stored.len() - 4].to_vec(), crc_changed] {
        let mut broken_pin = pin.clone();
        broken_pin["stored_bytes"] = serde_json::json!(broken.len());
        broken_pin["stored_sha256"] = serde_json::json!(sha256(&broken)?);
        ensure!(packet_body(broken, &broken_pin).is_err());
    }
    let mut changed = stored;
    changed[0] ^= 1;
    ensure!(packet_body(changed, &pin).is_err());
    Ok(())
}

#[test]
fn packet_index_binds_each_case_and_review() -> Result<()> {
    let root = PathBuf::from(std::env::var("RUN_ARTIFACT_DIR")?)
        .join(format!("packet-index-contract-{}", std::process::id()));
    std::fs::create_dir(&root)?;
    let write_pin = |name: &str, body: &[u8]| -> Result<Value> {
        let path = root.join(name);
        std::fs::write(&path, body)?;
        Ok(serde_json::json!({"path": path, "bytes": body.len(), "sha256": sha256(body)?}))
    };
    // These documents test routing and pins only. They contain no reference results.
    let evidence = write_pin("producer.json", br#"{"test":"provenance pin"}"#)?;
    let mut index = serde_json::json!({"schema":1,"status":"ACCEPTED","cases":{}});
    for case in ["first", "second"] {
        let manifest = write_pin(&format!("{case}.json"), case.as_bytes())?;
        let review = serde_json::json!({"status":"ACCEPTED","case":case,"fixture_sha256":case,
            "source_epoch":"test-source","build_identity":"test-build","manifest_sha256":manifest["sha256"],
            "producer_evidence":[evidence]});
        let review = write_pin(
            &format!("{case}-review.json"),
            &serde_json::to_vec(&review)?,
        )?;
        index["cases"][case] = serde_json::json!({"fixture_sha256":case,"source_epoch":"test-source",
            "build_identity":"test-build","manifest":manifest,"review":review});
    }
    let first = indexed_manifest(&index, "first", "first")?;
    let second = indexed_manifest(&index, "second", "second")?;
    ensure!(first.0 != second.0 && first.1 != second.1);
    ensure!(indexed_manifest(&index, "absent", "first").is_err());
    ensure!(indexed_manifest(&index, "first", "second").is_err());
    for field in ["source_epoch", "build_identity", "manifest", "review"] {
        let mut bad = index.clone();
        bad["cases"]["first"][field] = index["cases"]["second"][field].clone();
        if field == "source_epoch" || field == "build_identity" {
            bad["cases"]["first"][field] = Value::String("other".into());
        }
        ensure!(
            indexed_manifest(&bad, "first", "first").is_err(),
            "reject changed {field}"
        );
    }
    let mut bad = index.clone();
    bad["status"] = Value::String("REQUIRES_INDEPENDENT_REVIEW".into());
    ensure!(indexed_manifest(&bad, "first", "first").is_err());
    std::fs::write(root.join("producer.json"), b"changed provenance")?;
    ensure!(indexed_manifest(&index, "first", "first").is_err());
    Ok(())
}

pub(super) fn validate_recovery_pair(primary: &Value, retry: &Value) -> Result<()> {
    recovery::validate_pair(primary, retry)
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
    let phases = recovery::load(&packet.recovery, case_name, &fixture_sha, &fixture)?;
    let stream = format!("local.observer.{case_name}");
    run_recovery_with_retry(
        &phases.primary,
        &phases.retry,
        5,
        Some((&phases.primary_sha, &phases.retry_sha)),
        &stream,
        true,
    )?;
    eprintln!(
        "CRYPTO_CASE_RECOVERY {}",
        serde_json::json!({
            "scope": "recovery_only", "case": case_name,
            "fixture_sha256": fixture_sha, "manifest_sha256": packet.manifest_sha,
            "primary_sha256": phases.primary_sha, "retry_sha256": phases.retry_sha,
            "primary_manifest_sha256": phases.primary_manifest_sha,
            "retry_manifest_sha256": phases.retry_manifest_sha,
            "ordinary_source_epoch": packet.source_epoch, "ordinary_build_identity": packet.build_identity,
            "split": 5, "final_period": 12,
            "faults": ["BeforeCommit", "AfterCommit"], "recovery_and_reconciliation": "complete"
        })
    );
    run_workload(&fixture, false, false, Some((case_name, &packet)))?;
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
            "primary_manifest_sha256": phases.primary_manifest_sha,
            "retry_manifest_sha256": phases.retry_manifest_sha,
            "primary_sha256": phases.primary_sha, "retry_sha256": phases.retry_sha,
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

#[test]
fn native_integration_crypto_default_trace_observations() -> Result<()> {
    let (fixture, fixture_sha) =
        wellformed_proof_fixture("default", "", "Existing double voting proof")?;
    let packet = load("default", &fixture_sha)?;
    api::validate_case_documents(
        &fixture,
        "default",
        &packet.api,
        &packet.cpp,
        &packet.roster,
        &packet.rules,
    )?;
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .context("task scratch")?;
    let root = base.join(format!(
        "default-trace-observations-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    let application = root.join("application");
    let concrete = root.join("concrete");
    let periods = fixture["periods"].as_array().context("periods")?;
    run_segment(&fixture, &periods[..9], &application, &concrete, true)?;
    run_segment(&fixture, &periods[9..], &application, &concrete, false)?;
    let count =
        api::verify_default_trace_observations(&fixture, (&application, &concrete), &packet.api)?;
    eprintln!(
        "CRYPTO_TRACE_OBSERVATIONS {}",
        serde_json::json!({"scope":"focused_trace_observations_only","case":"default","traces":count,"fixture_sha256":fixture_sha,"manifest_sha256":packet.manifest_sha})
    );
    era_retention::finish(
        scratch,
        era_retention::evidence(
            &fixture,
            "44 authenticated default trace observations; persistence and lifecycle unchanged",
        )?,
        true,
    )?;
    Ok(())
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

#[test]
fn native_integration_crypto_historical_slashing_account_views() -> Result<()> {
    for (case_name, first_error, second_error) in [
        (
            "different_signers",
            "Votes validators differ",
            "Votes validators differ",
        ),
        (
            "identical_unsigned",
            "Votes are identical",
            "Votes are identical",
        ),
        ("default", "", "Existing double voting proof"),
    ] {
        let (fixture, fixture_sha) =
            wellformed_proof_fixture(case_name, first_error, second_error)?;
        let packet = load(case_name, &fixture_sha)?;
        api::validate_case_documents(
            &fixture,
            case_name,
            &packet.api,
            &packet.cpp,
            &packet.roster,
            &packet.rules,
        )?;
        let selected = packet.api["cases"]
            .as_array()
            .context("API cases")?
            .iter()
            .filter(|row| {
                let name = row["name"].as_str().unwrap_or_default();
                name.ends_with("/delayed_jail")
                    || name.ends_with("/delayed_jailed")
                    || name.ends_with("/slash_reversed_duplicate")
            })
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            selected.len() == 39,
            "three slashing requests at each of13 heads"
        );
        for head in 0..=12 {
            ensure!(
                selected.iter().filter(|row| row["head"] == head).count() == 3,
                "complete slashing request head {head}"
            );
        }
        let estimates = selected
            .iter()
            .flat_map(|row| row["estimates"].as_array().unwrap())
            .collect::<Vec<_>>();
        ensure!(
            estimates.len() == selected.len() * 3,
            "three estimates per selected request"
        );
        let probes = estimates
            .iter()
            .map(|row| row["probes"].as_array().unwrap().len())
            .sum::<usize>();
        ensure!(probes > 0, "selected estimator probes");
        let names = estimates
            .iter()
            .map(|row| row["name"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let cpp_rows = packet.cpp["cases"]
            .as_array()
            .context("C++ cases")?
            .iter()
            .filter(|row| names.contains(row["name"].as_str().unwrap_or_default()))
            .cloned()
            .collect::<Vec<_>>();
        ensure!(
            cpp_rows.len() == estimates.len(),
            "complete selected C++ transcript"
        );
        let mut focused_api = packet.api.clone();
        let mut focused_cpp = packet.cpp.clone();
        focused_api["cases"] = Value::Array(selected.clone());
        focused_cpp["cases"] = Value::Array(cpp_rows);

        let base = std::env::var_os("RUN_ARTIFACT_DIR")
            .map(PathBuf::from)
            .context("task scratch")?;
        let root = base.join(format!(
            "historical-slashing-{}-{}-{}",
            case_name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let mut scratch = era_retention::register(&root, &fixture)?;
        std::fs::create_dir(&root)?;
        era_retention::created(&mut scratch)?;
        let application = root.join("application");
        let concrete = root.join("concrete");
        let periods = fixture["periods"].as_array().context("periods")?;
        run_segment(&fixture, &periods[..9], &application, &concrete, true)?;
        run_segment(&fixture, &periods[9..], &application, &concrete, false)?;
        api::verify_documents(
            &fixture,
            (&application, &concrete),
            &focused_api,
            &focused_cpp,
            (selected.len() * 2, estimates.len()),
            true,
            Some(probes),
        )?;
        eprintln!(
            "CRYPTO_SLASHING_ACCOUNT_VIEWS {}",
            serde_json::json!({
                "scope":"focused_historical_slashing_account_views", "case":case_name,
                "fixture_sha256":fixture_sha, "manifest_sha256":packet.manifest_sha,
                "api_sha256":packet.api_sha, "cpp_sha256":packet.cpp_sha,
                "heads":13, "selected_requests":selected.iter().map(|row| &row["name"]).collect::<Vec<_>>(),
                "estimates":estimates.len(), "probes":probes, "traces":selected.len()*2,
                "interacting_traces":packet.api["interacting_traces"].as_array().context("interacting traces")?.len()
            })
        );
        era_retention::finish(
            scratch,
            era_retention::evidence(
                &fixture,
                "authenticated selected slashing requests and all interacting traces; persistence and lifecycle unchanged",
            )?,
            true,
        )?;
    }
    Ok(())
}
