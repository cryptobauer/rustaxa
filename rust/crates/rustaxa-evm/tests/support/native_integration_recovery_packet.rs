use super::*;

pub(super) struct RecoveryPacket {
    pub primary: Value,
    pub retry: Value,
    pub primary_sha: String,
    pub retry_sha: String,
    pub primary_manifest_sha: String,
    pub retry_manifest_sha: String,
}

fn stable_inputs(expected: &Value, actual: &Value) -> Result<()> {
    for field in ["configuration", "genesis", "registry", "inputs"] {
        ensure!(
            !expected[field].is_null() && expected[field] == actual[field],
            "recovery stable field {field}"
        );
    }
    let periods = expected["periods"].as_array().context("ordinary periods")?;
    let actual_periods = actual["periods"].as_array().context("phase periods")?;
    ensure!(
        periods.len() == 12 && actual_periods.len() == 12,
        "recovery period count"
    );
    ensure!(
        expected["genesis"]["root"].is_string(),
        "recovery genesis root"
    );
    for (index, (before, after)) in periods.iter().zip(actual_periods).enumerate() {
        ensure!(
            before["number"] == (index + 1) as u64
                && after["number"] == before["number"]
                && before["transactions"].is_array()
                && before["transactions"] == after["transactions"]
                && before["final"]["root"].is_string()
                && before["final"]["root"] == after["final"]["root"],
            "recovery transactions and root at period {}",
            index + 1
        );
    }
    Ok(())
}

pub(super) fn validate_pair(primary: &Value, retry: &Value) -> Result<()> {
    ensure!(
        primary["reopen"]["close_reopen_after_periods"] == serde_json::json!([5])
            && retry["reopen"]["close_reopen_after_periods"] == serde_json::json!([4, 5]),
        "recovery phase schedules"
    );
    stable_inputs(primary, retry)
}

fn load_phase(
    descriptor: &Value,
    case_name: &str,
    phase: &str,
    fixture_sha: &str,
    ordinary: &Value,
    schedule: &[u64],
) -> Result<(Value, String, String)> {
    let manifest_pin = &descriptor["manifest"];
    let manifest: Value = serde_json::from_slice(&pinned_file(manifest_pin)?)?;
    let review: Value = serde_json::from_slice(&pinned_file(&descriptor["review"])?)?;
    ensure!(
        manifest["status"] == "ACCEPTED"
            && manifest["case"] == case_name
            && manifest["phase"] == phase
            && manifest["reference_revision"] == "bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418"
            && manifest["schedule"] == serde_json::json!(schedule),
        "recovery phase manifest identity"
    );
    for field in ["source_epoch", "build_identity"] {
        ensure!(
            manifest[field].as_str().is_some_and(|v| !v.is_empty()),
            "phase {field}"
        );
    }
    ensure!(
        review["status"] == "ACCEPTED"
            && review["case"] == case_name
            && review["phase"] == phase
            && review["ordinary_fixture_sha256"] == fixture_sha
            && review["manifest_sha256"] == manifest_pin["sha256"]
            && review["source_epoch"] == manifest["source_epoch"]
            && review["build_identity"] == manifest["build_identity"],
        "recovery phase provenance review"
    );
    let evidence = review["producer_evidence"]
        .as_array()
        .context("phase producer evidence")?;
    ensure!(!evidence.is_empty(), "phase evidence required");
    for pin in evidence {
        pinned_file(pin)?;
    }
    let events = schedule
        .iter()
        .flat_map(|period| {
            ["closed", "reopened"].map(|event| {
                format!("RECOVERY_LIFECYCLE phase={phase} period={period} event={event}")
            })
        })
        .collect::<Vec<_>>();
    ensure!(
        manifest["lifecycle_events"] == serde_json::json!(events),
        "phase lifecycle events"
    );
    let path = Path::new(
        manifest_pin["path"]
            .as_str()
            .context("phase manifest path")?,
    );
    ensure!(
        manifest["body"]["encoding"] == "gzip",
        "bounded gzip phase body required"
    );
    let (fixture, sha) = read_stream(
        path.parent().context("phase packet parent")?,
        &manifest["body"],
    )?;
    ensure!(
        fixture["reopen"]["close_reopen_after_periods"] == serde_json::json!(schedule),
        "phase body schedule"
    );
    stable_inputs(ordinary, &fixture)?;
    Ok((
        fixture,
        sha,
        manifest_pin["sha256"]
            .as_str()
            .context("phase manifest hash")?
            .to_owned(),
    ))
}

pub(super) fn load(
    descriptor: &Value,
    case_name: &str,
    fixture_sha: &str,
    ordinary: &Value,
) -> Result<RecoveryPacket> {
    ensure!(
        ordinary["reopen"]["close_reopen_after_periods"] == serde_json::json!([9]),
        "ordinary case schedule"
    );
    let (primary, primary_sha, primary_manifest_sha) = load_phase(
        &descriptor["primary"],
        case_name,
        "primary",
        fixture_sha,
        ordinary,
        &[5],
    )?;
    let (retry, retry_sha, retry_manifest_sha) = load_phase(
        &descriptor["retry"],
        case_name,
        "retry",
        fixture_sha,
        ordinary,
        &[4, 5],
    )?;
    validate_pair(&primary, &retry)?;
    ensure!(primary_sha != retry_sha, "independent phase body hashes");
    Ok(RecoveryPacket {
        primary,
        retry,
        primary_sha,
        retry_sha,
        primary_manifest_sha,
        retry_manifest_sha,
    })
}

#[test]
fn recovery_phase_inputs_reject_wrong_schedule_and_case_data() -> Result<()> {
    // This fixture checks input identity and contains no execution results.
    let periods = (1..=12).map(|number| serde_json::json!({"number":number,"transactions":[{"test":"input identity"}],"final":{"root":format!("root{number}")}})).collect::<Vec<_>>();
    let primary = serde_json::json!({"configuration":{},"genesis":{"root":"genesis"},"registry":{},"inputs":{},"periods":periods,"reopen":{"close_reopen_after_periods":[5]}});
    let mut retry = primary.clone();
    retry["reopen"]["close_reopen_after_periods"] = serde_json::json!([4, 5]);
    validate_pair(&primary, &retry)?;
    ensure!(validate_pair(&retry, &primary).is_err());
    for field in ["configuration", "genesis", "registry", "inputs"] {
        let mut changed = retry.clone();
        changed[field] = Value::Null;
        ensure!(validate_pair(&primary, &changed).is_err());
        let mut absent = primary.clone();
        absent[field] = Value::Null;
        changed[field] = Value::Null;
        ensure!(validate_pair(&absent, &changed).is_err());
    }
    let mut numeric_primary = primary.clone();
    let mut numeric_retry = retry.clone();
    numeric_primary["genesis"]["root"] = serde_json::json!(1);
    numeric_retry["genesis"]["root"] = serde_json::json!(1);
    ensure!(validate_pair(&numeric_primary, &numeric_retry).is_err());
    let mut changed = retry.clone();
    changed["periods"][4]["transactions"][0]["test"] = serde_json::json!("other case");
    ensure!(validate_pair(&primary, &changed).is_err());
    changed = retry.clone();
    changed["periods"][4]["final"]["root"] = serde_json::json!("other root");
    ensure!(validate_pair(&primary, &changed).is_err());
    changed = retry;
    changed["periods"][4]["number"] = serde_json::json!(6);
    ensure!(validate_pair(&primary, &changed).is_err());
    Ok(())
}

#[test]
fn recovery_phase_packet_rejects_binding_and_body_tampering() -> Result<()> {
    use std::io::Write;
    // These documents check packet integrity without an execution oracle.
    let root = PathBuf::from(std::env::var("RUN_ARTIFACT_DIR")?).join(format!(
        "phase-packet-contract-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir(&root)?;
    let write_pin = |name: &str, bytes: &[u8]| -> Result<Value> {
        let path = root.join(name);
        std::fs::write(&path, bytes)?;
        Ok(serde_json::json!({"path":path,"bytes":bytes.len(),"sha256":sha256(bytes)?}))
    };
    let periods = (1..=12)
        .map(|number| {
            serde_json::json!({"number":number,
        "transactions":[{"test":"input identity"}],"final":{"root":format!("root{number}")}})
        })
        .collect::<Vec<_>>();
    let template = serde_json::json!({"configuration":{},"genesis":{"root":"genesis"},
        "registry":{},"inputs":{},"periods":periods,"reopen":{"close_reopen_after_periods":[9]}});
    let attempt = |kind: usize| -> Result<()> {
        let mut ordinary = template.clone();
        let mut phase = template.clone();
        phase["reopen"]["close_reopen_after_periods"] = serde_json::json!([5]);
        if kind == 8 {
            phase["periods"][4]["transactions"][0]["test"] = serde_json::json!("other input");
        }
        if kind == 9 {
            ordinary["genesis"]["root"] = serde_json::json!(1);
            phase["genesis"]["root"] = serde_json::json!(1);
        }
        let bytes = serde_json::to_vec(&phase)?;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes)?;
        let mut stored = encoder.finish()?;
        let mut body = serde_json::json!({"file":"witness.json.gz","encoding":"gzip",
            "bytes":bytes.len(),"sha256":sha256(&bytes)?,"stored_bytes":stored.len(),"stored_sha256":sha256(&stored)?});
        if kind == 5 {
            stored = bytes;
            body["encoding"] = Value::Null;
            body["stored_bytes"] = Value::Null;
            body["stored_sha256"] = Value::Null;
        }
        if kind == 7 {
            stored[0] ^= 1;
        }
        if kind == 13 {
            body["sha256"] = serde_json::json!("wrong body hash");
        }
        write_pin("witness.json.gz", &stored)?;
        let evidence = write_pin("producer.txt", b"metadata-only integrity test")?;
        let mut manifest = serde_json::json!({"status":"ACCEPTED","case":"metadata_only","phase":"primary",
            "reference_revision":"bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418","schedule":[5],
            "source_epoch":"test-source","build_identity":"test-build","body":body,
            "lifecycle_events":["RECOVERY_LIFECYCLE phase=primary period=5 event=closed",
                "RECOVERY_LIFECYCLE phase=primary period=5 event=reopened"]});
        if kind == 3 {
            manifest["case"] = serde_json::json!("other_case");
        }
        if kind == 6 {
            manifest["lifecycle_events"]
                .as_array_mut()
                .unwrap()
                .reverse();
        }
        let manifest_pin = write_pin("manifest.json", &serde_json::to_vec(&manifest)?)?;
        let mut review = serde_json::json!({"status":"ACCEPTED","case":"metadata_only","phase":"primary",
            "ordinary_fixture_sha256":"test-fixture","manifest_sha256":manifest_pin["sha256"],
            "source_epoch":"test-source","build_identity":"test-build","producer_evidence":[evidence]});
        match kind {
            1 => review["phase"] = serde_json::json!("retry"),
            2 => review["ordinary_fixture_sha256"] = serde_json::json!("other fixture"),
            4 => review["build_identity"] = serde_json::json!("other build"),
            10 => review["producer_evidence"] = serde_json::json!([]),
            11 => {
                std::fs::write(root.join("producer.txt"), b"changed evidence")?;
            }
            12 => review["manifest_sha256"] = serde_json::json!("wrong manifest hash"),
            _ => {}
        }
        let review_pin = write_pin("review.json", &serde_json::to_vec(&review)?)?;
        load_phase(
            &serde_json::json!({"manifest":manifest_pin,"review":review_pin}),
            "metadata_only",
            "primary",
            "test-fixture",
            &ordinary,
            &[5],
        )?;
        Ok(())
    };
    attempt(0)?;
    for kind in 1..=13 {
        ensure!(attempt(kind).is_err(), "reject phase tamper {kind}");
    }
    Ok(())
}
