//! Authenticated current-profile corpus consumed by the existing mixed owner.
use super::*;
use std::io::Read;

pub(super) fn compare_reward_map(
    context: &FinalChainNativeRewardsContext,
    rewards: &Value,
    end_block: &Value,
    input: &Value,
) -> Result<()> {
    let expected = rewards.as_array().context("reward writes")?;
    let end = end_block.as_array().context("end-block writes")?;
    let validators = input["validators"]
        .as_array()
        .context("one reward distribution")?;
    ensure!(
        expected.len() == validators.len() + 2 || expected.len() == validators.len() + 3,
        "single-distribution reward shape"
    );
    ensure!(expected.len() >= 3, "nonempty Aspen reward map");
    ensure!(
        context.raw_mutations.len() == expected.len() + end.len(),
        "complete reward occurrence count"
    );
    let (actual, actual_end) = context.raw_mutations.split_at(expected.len());
    ensure!(
        fixed::<32>(&expected[0]["key"]) == keccak256([8]).0
            && fixed::<32>(&expected.last().unwrap()["key"]) == keccak256([7]).0,
        "yield/supply barrier identities"
    );
    compare_raw_mutation(&actual[0], &expected[0])?;
    compare_raw_mutation(actual.last().unwrap(), expected.last().unwrap())?;
    let mut first = 1;
    // A positive author bonus precedes the map and writes its row twice.
    if expected.len() == validators.len() + 3 {
        let mut key = vec![0, 2];
        key.extend_from_slice(&fixed::<20>(&input["block_author"]));
        ensure!(
            fixed::<32>(&expected[1]["key"]) == keccak256(key).0,
            "author bonus identity"
        );
        compare_raw_mutation(&actual[1], &expected[1])?;
        first = 2;
    }
    let mut remaining = expected[first..expected.len() - 1]
        .iter()
        .map(|row| {
            (
                (fixed::<20>(&row["address"]), fixed::<32>(&row["key"])),
                row,
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    ensure!(
        remaining.len() == expected.len() - 1 - first,
        "one write per disjoint validator map block"
    );
    let expected_map = validators
        .iter()
        .map(|validator| {
            let mut key = vec![0, 2];
            key.extend_from_slice(&fixed::<20>(&validator["validator"]));
            (
                [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
                ],
                keccak256(key).0,
            )
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        remaining.keys().copied().collect::<BTreeSet<_>>() == expected_map,
        "reward map matches distribution validators"
    );
    for mutation in &actual[first..actual.len() - 1] {
        let row = remaining
            .remove(&(mutation.address, mutation.key.0))
            .context("validator reward-map occurrence")?;
        compare_raw_mutation(mutation, row)?;
    }
    ensure!(remaining.is_empty(), "complete validator reward map");
    for (mutation, expected) in actual_end.iter().zip(end) {
        compare_raw_mutation(mutation, expected)?;
    }
    Ok(())
}

pub(super) fn fixture_stream(stream: &str) -> Result<Value> {
    fixture_stream_from("native_integration", stream)
}

fn fixture_stream_from(directory: &str, stream: &str) -> Result<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures")
        .join(directory);
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    ensure!(manifest["registry"].as_array().context("registry")?.len() == 29);
    let pin = &manifest["streams"][stream];
    let name = string(&pin["file"]);
    ensure!(
        Path::new(name).components().count() == 1,
        "local corpus filename"
    );
    let stored = std::fs::read(root.join(name))?;
    let body = if pin["encoding"] == "gzip" {
        let mut body = Vec::new();
        flate2::read::GzDecoder::new(stored.as_slice()).read_to_end(&mut body)?;
        body
    } else {
        stored
    };
    let hash = revm::precompile::hash::sha256_run(&body, u64::MAX)
        .map_err(|error| anyhow::anyhow!("corpus SHA-256: {error:?}"))?;
    ensure!(
        hex::encode(hash.bytes) == string(&pin["sha256"]),
        "authenticated corpus stream"
    );
    ensure!(
        body.len() as u64 == number(&pin["bytes"]),
        "corpus byte length"
    );
    let fixture: Value = serde_json::from_slice(&body)?;
    if stream != "cpp.api" {
        ensure!(fixture["profile"] == "synthetic_current");
        ensure!(fixture["registry"] == manifest["registry"]);
    }
    Ok(fixture)
}

pub(super) fn verify_retained(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
) -> Result<()> {
    for period in fixture["periods"].as_array().context("periods")? {
        let number = number(&period["number"]);
        if number > committed.period.as_u64() {
            break;
        }
        if number == 4 {
            eprintln!(
                "CRYPTO_HISTORY_DIAGNOSTIC {}",
                serde_json::json!({"requested_period":number,"expected_root":period["final"]["root"],"committed_period":committed.period.as_u64(),"committed_root":hex::encode(committed.state_root),"stage":"before_historical_open"})
            );
        }
        let reader = historical_reader(path, committed, fixture, number as usize).with_context(|| format!("retained historical open period={} expected_root={} committed={:?} fixture=periods[{}].final", number, period["final"]["root"], committed, number - 1))?;
        verify_final_reader(&reader, &period["final"]).with_context(|| format!("retained historical reads period={} expected_root={} committed={:?} fixture=periods[{}].final", number, period["final"]["root"], committed, number - 1))?;
    }
    Ok(())
}

#[test]
fn native_integration_signed_periods_publish_reopen_and_retain() -> Result<()> {
    workload("local.observer.traced", true)
}
#[test]
fn native_integration_cacti_bracket_publish_reopen_and_retain() -> Result<()> {
    workload("local.observer.traced.bracket", false)
}
fn workload(stream: &str, apis: bool) -> Result<()> {
    let fixture = fixture_stream(stream)?;
    run_workload(&fixture, apis, false, None)
}

#[cfg(feature = "synthetic-fixtures")]
thread_local! { static CLEANUP_REQUESTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
#[cfg(feature = "synthetic-fixtures")]
pub(super) fn cleanup_requested() -> bool {
    CLEANUP_REQUESTED.with(std::cell::Cell::get)
}
#[cfg(feature = "synthetic-fixtures")]
struct CleanupRequest;
#[cfg(feature = "synthetic-fixtures")]
impl Drop for CleanupRequest {
    fn drop(&mut self) {
        CLEANUP_REQUESTED.with(|flag| flag.set(false));
    }
}

fn wellformed_proof_fixture(
    case: &str,
    first_error: &str,
    second_error: &str,
) -> Result<(Value, String)> {
    let stream = format!("local.observer.{case}");
    let fixture = fixture_stream_from("native_crypto_proofs", &stream)?;
    ensure!(fixture["reopen"]["close_reopen_after_periods"] == serde_json::json!([9]));
    let periods = fixture["periods"]
        .as_array()
        .context("well-formed periods")?;
    ensure!(periods.len() == 12 && number(&periods[3]["number"]) == 4);
    let rows = periods[3]["transactions"]
        .as_array()
        .context("period four transactions")?;
    let proof_rows = rows
        .iter()
        .filter(|row| matches!(row["name"].as_str(), Some("slash_a" | "slash_duplicate")))
        .collect::<Vec<_>>();
    ensure!(
        proof_rows.len() == 2
            && proof_rows[0]["name"] == "slash_a"
            && proof_rows[1]["name"] == "slash_duplicate"
    );
    for (row, expected_error) in proof_rows.iter().zip([first_error, second_error]) {
        ensure!(
            row["execution_error"] == expected_error
                && row["status"] == u64::from(expected_error.is_empty())
        );
        ensure!(row["output"] == "");
    }
    if case == "current_member_delayed_absent" || case == "current_nonmember_delayed_member" {
        let names = [
            "membership_live_validator",
            "membership_delayed_eligible",
            "membership_delayed_votes",
            "membership_proof_v4",
        ];
        let membership = rows
            .iter()
            .filter(|row| {
                row["name"]
                    .as_str()
                    .is_some_and(|name| names.contains(&name))
            })
            .collect::<Vec<_>>();
        ensure!(membership.len() == names.len());
        for (row, expected_name) in membership.iter().zip(names) {
            ensure!(row["name"] == expected_name);
        }
        let proof = membership[3];
        if case == "current_member_delayed_absent" {
            ensure!(
                proof["status"] == 0
                    && proof["execution_error"] == "Votes validators is not a validator"
            );
            ensure!(
                proof["ordered_raw_writes"]
                    .as_array()
                    .is_some_and(Vec::is_empty)
            );
            ensure!(proof["logs"].as_array().is_some_and(Vec::is_empty));
        } else {
            ensure!(proof["status"] == 1 && proof["execution_error"] == "");
        }
    }
    if case == "default" {
        let writes = proof_rows[0]["ordered_raw_writes"]
            .as_array()
            .context("default proof writes")?;
        ensure!(
            writes.iter().any(
                |row| row["address"] == "00000000000000000000000000000000000000ee"
                    && row["key"]
                        == "dc35efc475e49bffe215eea886063ea790bde40e358685c728d52129f16ae9db"
                    && row["value"] == "01"
            ),
            "default positive proof marker fixture"
        );
    }
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_crypto_proofs/manifest.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    let body_sha = string(&manifest["streams"][stream.as_str()]["sha256"]);
    Ok((fixture, body_sha.to_string()))
}

fn wellformed_proof_case(case: &str, first_error: &str, second_error: &str) -> Result<()> {
    let (fixture, body_sha) = wellformed_proof_fixture(case, first_error, second_error)?;
    #[cfg(feature = "synthetic-fixtures")]
    let _cleanup_request = if case == "invalid_signature_a" {
        eprintln!(
            "CLEANUP_DIAGNOSTIC_SCOPE case={} body_sha256={}",
            case, body_sha
        );
        CLEANUP_REQUESTED.with(|flag| {
            assert!(!flag.replace(true), "nested cleanup request");
        });
        Some(CleanupRequest)
    } else {
        None
    };
    let result = run_workload(&fixture, false, false, None)
        .with_context(|| format!("crypto case={} body_sha256={}", case, body_sha));
    if result.is_ok() {
        eprintln!(
            "CRYPTO_REPLAY_SCOPE {}",
            serde_json::json!({"scope":"DIRECT_LOCAL_OBSERVER_RUST","case":case,"body_sha256":body_sha,"status":"PASS"})
        );
    }
    result
}

macro_rules! wellformed_proof_test {
    ($name:ident, $case:literal, $first:literal, $second:literal) => {
        #[test]
        fn $name() -> Result<()> {
            wellformed_proof_case($case, $first, $second)
        }
    };
}

wellformed_proof_test!(
    native_integration_wellformed_default,
    "default",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_high_s_a,
    "high_s_a",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_different_signers,
    "different_signers",
    "Votes validators differ",
    "Votes validators differ"
);
wellformed_proof_test!(
    native_integration_wellformed_invalid_signature_a,
    "invalid_signature_a",
    "Invalid vote signature",
    "Invalid vote signature"
);
wellformed_proof_test!(
    native_integration_wellformed_invalid_signature_b,
    "invalid_signature_b",
    "Invalid vote signature",
    "Invalid vote signature"
);
wellformed_proof_test!(
    native_integration_wellformed_identical_unsigned,
    "identical_unsigned",
    "Votes are identical",
    "Votes are identical"
);
wellformed_proof_test!(
    native_integration_wellformed_same_block_distinct_sortition,
    "same_block_distinct_sortition",
    "Invalid votes block hash",
    "Invalid votes block hash"
);
wellformed_proof_test!(
    native_integration_wellformed_period_mismatch,
    "period_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);
wellformed_proof_test!(
    native_integration_wellformed_round_mismatch,
    "round_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);
wellformed_proof_test!(
    native_integration_wellformed_step_mismatch,
    "step_mismatch",
    "Votes period/round/step differ",
    "Votes period/round/step differ"
);
wellformed_proof_test!(
    native_integration_wellformed_odd_step_mixed_zero_forward,
    "odd_step_mixed_zero_forward",
    "Invalid votes block hash",
    "Invalid votes block hash"
);
wellformed_proof_test!(
    native_integration_wellformed_odd_step_mixed_zero_reversed,
    "odd_step_mixed_zero_reversed",
    "Invalid votes block hash",
    "Invalid votes block hash"
);
wellformed_proof_test!(
    native_integration_wellformed_even_step_mixed_zero,
    "even_step_mixed_zero",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_reversed_duplicate,
    "reversed_duplicate",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_duplicate_before_signature,
    "duplicate_before_signature",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_fresh_reverse,
    "fresh_reverse",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_compressed_recovery_4,
    "compressed_recovery_4",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_compressed_recovery_5,
    "compressed_recovery_5",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_current_member_delayed_absent,
    "current_member_delayed_absent",
    "",
    "Existing double voting proof"
);
wellformed_proof_test!(
    native_integration_wellformed_current_nonmember_delayed_member,
    "current_nonmember_delayed_member",
    "",
    "Existing double voting proof"
);

#[test]
fn native_integration_magnolia_cancel_boundary_publish_reopen_and_retain() -> Result<()> {
    let fixture = fixture_stream_from("native_cancel_boundary", "local.observer.traced")?;
    let rows = fixture["pending_queue"]
        .as_array()
        .context("pending queue")?;
    ensure!(
        rows.len() == 3
            && rows
                .iter()
                .all(|row| number(&row["block"]) == 6 && !string(&row["raw"]).is_empty()),
        "committed queue maturity"
    );
    run_workload(&fixture, false, true, None)
}

fn run_workload(
    fixture: &Value,
    apis: bool,
    boundary_apis: bool,
    case_packet: Option<(&str, &qualification::CasePacket)>,
) -> Result<()> {
    let periods = fixture["periods"].as_array().context("periods")?;
    let bound = &fixture["period_bound"];
    ensure!(
        periods.len() as u64
            == number(&bound["last_custody"])
                + number(&bound["locking"])
                + number(&bound["delay"])
                + number(&bound["continuation"])
    );
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!(
        "mixed-native-integration-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&root, fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    let application = root.join("application");
    let concrete = root.join("concrete");
    run_segment(fixture, &periods[..9], &application, &concrete, true)?;
    run_segment(fixture, &periods[9..], &application, &concrete, false)?;
    if apis {
        api::verify(fixture, &application, &concrete)?;
    }
    if let Some((case_name, packet)) = case_packet {
        api::verify_case_documents(
            fixture,
            case_name,
            (&application, &concrete),
            &packet.api,
            &packet.cpp,
            &packet.roster,
            &packet.rules,
        )?;
    }
    if boundary_apis {
        let api = fixture_stream_from("native_cancel_boundary", "local.observer.traced.api")?;
        let cpp = fixture_stream_from("native_cancel_boundary", "cpp.api")?;
        api::verify_documents(
            fixture,
            (&application, &concrete),
            &api,
            &cpp,
            (30, 45),
            false,
            None,
        )?;
    }
    era_retention::finish(
        scratch,
        era_retention::evidence(
            fixture,
            "complete native integration periods,receipts/native/history,physicalreopen; API branch recorded by test/case ID; all owners closed",
        )?,
        true,
    )?;
    Ok(())
}

#[path = "native_integration_qualification.rs"]
mod qualification;

#[path = "native_integration_api.rs"]
pub(crate) mod api;

#[test]
fn native_integration_reward_map_permutation_rejects_barrier_and_occurrence_changes() -> Result<()>
{
    use rustaxa_consensus::native_session::{
        FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRawValue,
    };
    let fixture = fixture_stream("local.observer.traced")?;
    let period = &fixture["periods"][1];
    let rewards = &period["ordered_raw_writes"]["rewards"];
    let end = &period["ordered_raw_writes"]["end_block"];
    let input = &period["reward_input"];
    let mut context = FinalChainNativeRewardsContext::default();
    for row in rewards
        .as_array()
        .unwrap()
        .iter()
        .chain(end.as_array().unwrap())
    {
        let value = bytes(&row["value"]);
        context.raw_mutations.push(FinalChainNativeRawMutation {
            address: fixed(&row["address"]),
            key: ConcreteStorageKey(fixed(&row["key"])),
            expected: ConcreteRead::Absent,
            operation: if value.is_empty() {
                FinalChainNativeRawOperation::Delete
            } else {
                FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(value).unwrap())
            },
        });
    }
    compare_reward_map(&context, rewards, end, input)?;
    let first =
        if rewards.as_array().unwrap().len() == input["validators"].as_array().unwrap().len() + 3 {
            2
        } else {
            1
        };
    let mut permutation = context.clone();
    permutation.raw_mutations.swap(first, first + 1);
    compare_reward_map(&permutation, rewards, end, input)?;
    for kind in 0..7 {
        let mut bad = context.clone();
        match kind {
            0 => bad.raw_mutations.swap(0, first),
            1 => {
                bad.raw_mutations[first] = bad.raw_mutations[first + 1].clone();
            }
            2 => {
                bad.raw_mutations.remove(first);
            }
            3 => bad.raw_mutations.push(bad.raw_mutations[first].clone()),
            4 => {
                bad.raw_mutations[first].operation = FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(vec![0xff]).unwrap(),
                )
            }
            5 => bad.raw_mutations[first].address = [9; 20],
            _ => {
                let last = rewards.as_array().unwrap().len() - 1;
                bad.raw_mutations.swap(first, last);
            }
        }
        ensure!(
            compare_reward_map(&bad, rewards, end, input).is_err(),
            "invalid reward permutation {kind}"
        );
    }
    ensure!(
        compare_reward_map(
            &context,
            &serde_json::json!([{}, {}]),
            end,
            &serde_json::json!({"validators":[]})
        )
        .is_err(),
        "short invalid shape fails without indexing"
    );
    Ok(())
}

#[test]
fn native_integration_bounded_recovery_repeats_and_continues() -> Result<()> {
    let fixture = fixture_stream("local.observer.traced")?;
    run_recovery(&fixture, 9)
}

#[test]
fn native_integration_first_proof_recovery() -> Result<()> {
    let fixture = fixture_stream_from("native_jail_reopen", "local.observer.traced")?;
    run_recovery(&fixture, 4)
}

#[test]
fn native_integration_parent_revert_proof_recovery() -> Result<()> {
    let primary = fixture_stream_from("native_jail_reopen", "local.observer.reopen5")?;
    let retry = fixture_stream_from("native_jail_reopen", "local.observer.reopen4_5")?;
    ensure!(primary["reopen"]["close_reopen_after_periods"] == serde_json::json!([5]));
    ensure!(retry["reopen"]["close_reopen_after_periods"] == serde_json::json!([4, 5]));
    for key in ["configuration", "genesis", "registry", "inputs"] {
        ensure!(primary[key] == retry[key], "phase fixture mismatch: {key}");
    }
    let left = primary["periods"].as_array().context("primary periods")?;
    let right = retry["periods"].as_array().context("retry periods")?;
    ensure!(left.len() == 12 && right.len() == 12);
    for (a, b) in left.iter().zip(right) {
        ensure!(a["number"] == b["number"] && a["transactions"] == b["transactions"]);
        ensure!(a["final"]["root"] == b["final"]["root"]);
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_jail_reopen/manifest.json");
    let manifest: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let pins = (
        string(&manifest["streams"]["local.observer.reopen5"]["sha256"]),
        string(&manifest["streams"]["local.observer.reopen4_5"]["sha256"]),
    );
    run_recovery_with_retry(
        &primary,
        &retry,
        5,
        Some(pins),
        "local.observer.reopen5",
        false,
    )
}

#[test]
fn native_integration_jail_expiry_recovery() -> Result<()> {
    let fixture = fixture_stream("local.observer.traced")?;
    run_recovery(&fixture, 7)
}

#[test]
fn native_integration_magnolia_cancel_boundary_recovery() -> Result<()> {
    let fixture = fixture_stream_from("native_cancel_boundary", "local.observer.traced")?;
    run_recovery(&fixture, 7)
}

fn recovery_diagnostic_phase(
    split: usize,
    phase: &str,
    root: &Path,
    periods: &[Value],
    mode: (bool, &str),
    action: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let (fresh, fault) = mode;
    let first = periods.first().map(|period| number(&period["number"]));
    let last = periods.last().map(|period| number(&period["number"]));
    let record = |state: &str, error: Option<String>| {
        eprintln!(
            "RECOVERY_DIAGNOSTIC {}",
            serde_json::json!({
                "test": std::thread::current().name(), "split": split, "phase": phase,
                "root": root.display().to_string(), "first_period": first, "last_period": last,
                "fresh": fresh, "fault": fault, "state": state, "error": error,
            })
        );
    };
    record("enter", None);
    let result = action();
    match &result {
        Ok(()) => record("ok", None),
        Err(error) => record("error", Some(format!("{error:#}"))),
    }
    result
}

fn run_recovery(fixture: &Value, split: usize) -> Result<()> {
    run_recovery_with_retry(
        fixture,
        fixture,
        split,
        None,
        "local.observer.traced",
        false,
    )
}

fn run_recovery_with_retry(
    fixture: &Value,
    retry_fixture: &Value,
    split: usize,
    pins: Option<(&str, &str)>,
    fixture_label: &str,
    qualification: bool,
) -> Result<()> {
    let periods = fixture["periods"].as_array().unwrap();
    let retry_periods = retry_fixture["periods"].as_array().unwrap();
    if qualification {
        ensure!(
            split == 5 && periods.len() == 12 && retry_periods.len() == 12,
            "crypto recovery extent"
        );
        ensure!(
            fixture == retry_fixture
                && number(&periods[split - 1]["number"]) == 5
                && number(&periods[11]["number"]) == 12,
            "same case period five recovery"
        );
        ensure!(
            pins.is_some_and(|(primary, retry)| primary == retry),
            "same case body pins"
        );
    }
    let authority = match pins {
        Some((primary_sha, retry_sha)) => Some(serde_json::json!({
            "configuration": fixture["configuration"],
            "primary": {"sha256": primary_sha, "keccak256": hex::encode(keccak256(serde_json::to_vec(fixture)?)), "schedule": fixture["reopen"]["close_reopen_after_periods"]},
            "retry": {"sha256": retry_sha, "keccak256": hex::encode(keccak256(serde_json::to_vec(retry_fixture)?)), "schedule": retry_fixture["reopen"]["close_reopen_after_periods"]},
        })),
        None => None,
    };
    let phase_authority = |phase: &str, retry: bool, root: &Path, fault: &str| {
        if let Some(value) = &authority {
            eprintln!(
                "RECOVERY_PHASE_AUTHORITY {}",
                serde_json::json!({"phase":phase,"root":root.display().to_string(),"fault":fault,"split":split,"oracle":if retry { &value["retry"] } else { &value["primary"] }})
            );
        }
    };
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let parent = base.join(format!(
        "native-recovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let mut scratch = era_retention::register(&parent, authority.as_ref().unwrap_or(fixture))?;
    std::fs::create_dir(&parent)?;
    era_retention::created(&mut scratch)?;
    let baseline = parent.join("baseline");
    std::fs::create_dir(&baseline)?;
    eprintln!(
        "RECOVERY_CONFIG {}",
        serde_json::json!({
            "split": split, "configuration": fixture["configuration"],
            "fixture": fixture_label, "timer": "UNKNOWN",
        })
    );
    phase_authority("baseline-prefix", false, &baseline, "None");
    recovery_diagnostic_phase(
        split,
        "baseline-prefix",
        &baseline,
        &periods[..split],
        (true, "None"),
        || {
            run_segment(
                fixture,
                &periods[..split],
                &baseline.join("application"),
                &baseline.join("state_db"),
                true,
            )
        },
    )?;
    phase_authority("baseline-continuation", false, &baseline, "None");
    recovery_diagnostic_phase(
        split,
        "baseline-continuation",
        &baseline,
        &periods[split..],
        (false, "None"),
        || {
            run_segment(
                fixture,
                &periods[split..],
                &baseline.join("application"),
                &baseline.join("state_db"),
                false,
            )
        },
    )?;
    let expected = mixed_recovery::final_facts(&baseline, fixture)?;
    for fault in [
        mixed_recovery::Fault::BeforeCommit,
        mixed_recovery::Fault::AfterCommit,
    ] {
        let root = parent.join(format!("{fault:?}"));
        std::fs::create_dir(&root)?;
        let fault_name = format!("{fault:?}");
        phase_authority("fault-prefix", false, &root, &fault_name);
        recovery_diagnostic_phase(
            split,
            "fault-prefix",
            &root,
            &periods[..split],
            (true, &fault_name),
            || {
                run_segment_fault(
                    fixture,
                    &periods[..split],
                    &root.join("application"),
                    &root.join("state_db"),
                    true,
                    fault,
                )
            },
        )?;
        if qualification {
            let (application, chain) = open_chain(&root.join("application"), fixture)?;
            let state_path = root.join("state_db");
            let observed = ConcreteStateLifecycle::inspect_existing(
                &state_path,
                chain.concrete_chain_identity()?,
            )?;
            let committed_index = if fault == mixed_recovery::Fault::BeforeCommit {
                split - 2
            } else {
                split - 1
            };
            let expected_state = &periods[committed_index]["final"];
            ensure!(
                observed.committed.period.as_u64() == (committed_index + 1) as u64
                    && observed.committed.state_root == fixed(&expected_state["root"]),
                "fault committed period and root"
            );
            ensure!(
                application
                    .final_chain()
                    .external_evm_pending_publication_raw()?
                    .is_some(),
                "fault pending publication"
            );
            ensure!(
                application.final_chain().block_header_raw(5)?.is_none(),
                "fault period not published"
            );
            for tx in periods[split - 1]["transactions"]
                .as_array()
                .context("fault transactions")?
            {
                ensure!(
                    application
                        .final_chain()
                        .receipt_by_trx_hash(H256(fixed(&tx["hash"])))?
                        .is_none(),
                    "fault receipt not published"
                );
            }
            drop(chain);
            drop(application);
            verify_physical_rows(&state_path, &expected_state["rows"])?;
        }
        phase_authority("recovery-and-repeat", false, &root, &fault_name);
        recovery_diagnostic_phase(
            split,
            "recovery-and-repeat",
            &root,
            &periods[split - 1..split],
            (false, &fault_name),
            || {
                mixed_recovery::recover_period(
                    &root,
                    fixture,
                    split - 1,
                    fault == mixed_recovery::Fault::AfterCommit,
                )
            },
        )?;
        if fault == mixed_recovery::Fault::BeforeCommit {
            phase_authority("retry", true, &root, &fault_name);
            recovery_diagnostic_phase(
                split,
                "retry",
                &root,
                &retry_periods[split - 1..split],
                (false, &fault_name),
                || {
                    run_segment(
                        retry_fixture,
                        &retry_periods[split - 1..split],
                        &root.join("application"),
                        &root.join("state_db"),
                        false,
                    )
                },
            )?;
        }
        let use_retry = fault == mixed_recovery::Fault::BeforeCommit;
        let continuation_fixture = if use_retry { retry_fixture } else { fixture };
        let continuation_periods = if use_retry { retry_periods } else { periods };
        phase_authority("continuation", use_retry, &root, &fault_name);
        recovery_diagnostic_phase(
            split,
            "continuation",
            &root,
            &continuation_periods[split..],
            (false, &fault_name),
            || {
                run_segment(
                    continuation_fixture,
                    &continuation_periods[split..],
                    &root.join("application"),
                    &root.join("state_db"),
                    false,
                )
            },
        )?;
        ensure!(
            mixed_recovery::final_facts(&root, fixture)? == expected,
            "recovered complete workload facts"
        );
    }
    let mut evidence = era_retention::evidence(
        fixture,
        "uninterrupted baseline plus BeforeCommit/AfterCommit repeatedrecovery/continuation exactfinalfacts; all owners closed",
    )?;
    if let Some(value) = authority {
        evidence["phase_oracles"] = value;
        evidence["retry_fixture_evidence"] = era_retention::evidence(
            retry_fixture,
            "BeforeCommit retry and continuation phase assertions",
        )?;
    }
    era_retention::finish(scratch, evidence, qualification)?;
    Ok(())
}

pub(super) fn run_segment(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
) -> Result<()> {
    run_segment_fault(
        fixture,
        periods,
        application_path,
        concrete_path,
        fresh,
        mixed_recovery::Fault::None,
    )
}
pub(super) fn run_segment_fault(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    fault: mixed_recovery::Fault,
) -> Result<()> {
    run_segment_observing(
        fixture,
        periods,
        application_path,
        concrete_path,
        fresh,
        fault,
        None,
    )
}
pub(super) fn run_segment_observing(
    fixture: &Value,
    periods: &[Value],
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    fault: mixed_recovery::Fault,
    ordinary: Option<OrdinaryObserver>,
) -> Result<()> {
    let (application, mut chain) = open_chain(application_path, fixture)?;
    let identity = chain.concrete_chain_identity()?;
    let mut concrete = if fresh {
        let concrete = fresh_concrete(concrete_path, &chain, fixture)?;
        chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
        concrete
    } else {
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
        let genesis = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: fixed(&fixture["genesis"]["root"]),
        };
        if fixture["profile"] != "synthetic_mainnet_era" {
            let reader = ConcreteStateReader::open_historical_read_only(
                concrete_path,
                observed.committed,
                genesis,
            )?;
            chain.hydrate_concrete_genesis_accounts(&reader)?;
            drop(reader);
        }
        ConcreteStateLifecycle::open(concrete_path, identity, observed.committed)?
    };
    let mut epoch = StateApiEpoch::new();
    for (index, period) in periods.iter().enumerate() {
        if let Some(observer) = &ordinary {
            let observed = concrete.observation()?;
            observer.borrow_mut().begin(
                concrete_path,
                observed.identity,
                observed.committed,
                number(&period["number"]),
                observed.generation,
            )?;
        }
        let inner = Adapter {
            chain: &chain,
            application: &application,
            concrete: RefCell::new(Some(concrete)),
            staged: RefCell::new(None),
            native: RefCell::new(None),
            native_context: RefCell::new(None),
            physical_receipt: None,
            state_api_epoch: epoch,
            ordinary_observer: ordinary.clone(),
            fixture: period,
            account_node_publication: if fixture["oracle_lifecycle"] == "public_batched" {
                AccountNodePublication::FinalPeriod {
                    limits: rustaxa_storage::ConcreteStorageInventoryLimits {
                        max_nodes: 1_000_000,
                        max_leaves: 100_000,
                        max_value_bytes: 32_000_000,
                    },
                }
            } else {
                AccountNodePublication::ObserverHistory
            },
        };
        let adapter = mixed_recovery::FaultAdapter {
            inner,
            fault: if index + 1 == periods.len() {
                fault
            } else {
                mixed_recovery::Fault::None
            },
            executions: std::cell::Cell::new(0),
            rewards: std::cell::Cell::new(0),
            commits: std::cell::Cell::new(0),
            original_projection_hash: std::cell::Cell::new(None),
            recovery_capture: None,
            recovery_journal_cap: 0,
        };
        let execute = || -> Result<()> {
            if index == 0 {
                initialize_and_recover_final_chain_application_state_at_joint_startup(
                    &chain,
                    &adapter.inner,
                )?;
            }
            mainnet_era::bind_legacy_scheduler_order(&chain, fixture, period)?;
            let request = request(fixture, period)?;
            chain.ensure_period_data(
                FinalChainBlockNumber::new(number(&period["number"])),
                &period_data(&request),
            )?;
            #[cfg(feature = "synthetic-fixtures")]
            let diagnostic_before = if std::env::var_os("RUST_LIB_BACKTRACE").as_deref()
                == Some(std::ffi::OsStr::new("1"))
            {
                Some((
                    adapter
                        .inner
                        .concrete
                        .borrow()
                        .as_ref()
                        .and_then(|owner| owner.observation().ok()),
                    adapter.inner.state_api_epoch.current(),
                    adapter.inner.concrete.borrow().is_some(),
                ))
            } else {
                None
            };
            let outcome = execute_final_chain_application_task(
                &chain,
                request,
                FinalChainProposalPeriodDagLevelUpdate::default(),
                false,
                fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
                &adapter,
            );
            #[cfg(feature = "synthetic-fixtures")]
            if let (Err(error), Some((before, epoch_before, writer_before))) =
                (&outcome, diagnostic_before)
            {
                let request_after = adapter
                    .inner
                    .native_context
                    .borrow()
                    .as_ref()
                    .map(|context| format!("{:?}", context.request_id))
                    .unwrap_or_else(|| "UNAVAILABLE".to_owned());
                let writer_after = adapter.inner.concrete.borrow().is_some();
                let view_after = adapter.inner.native_context.borrow().is_some();
                eprintln!(
                    "sst_diagnostic task_error period={} request_after_return={request_after:?} committed_before_task={:?} generation_before_task={:?} epoch_before_task={epoch_before} writer_handle_before_task={writer_before} writer_handle_after_return={writer_after} native_context_after_return={view_after} selected_at_failure=UNAVAILABLE committed_at_failure=UNAVAILABLE generation_at_failure=UNAVAILABLE epoch_at_failure=UNAVAILABLE writer_live_at_failure=UNAVAILABLE view_live_at_failure=UNAVAILABLE error={error:#}",
                    number(&period["number"]),
                    before
                        .as_ref()
                        .map(|value| format!("{:?}", value.committed))
                        .unwrap_or_else(|| "UNAVAILABLE".to_owned()),
                    before
                        .as_ref()
                        .map(|value| format!("{:?}", value.generation))
                        .unwrap_or_else(|| "UNAVAILABLE".to_owned())
                );
            }
            if adapter.fault != mixed_recovery::Fault::None {
                let error = outcome.expect_err("bounded native integration interruption");
                let expected_fault = match adapter.fault {
                    mixed_recovery::Fault::BeforeCommit => {
                        "mixed fixture interrupted before concrete commit"
                    }
                    mixed_recovery::Fault::AfterCommit => {
                        "mixed fixture interrupted after concrete commit"
                    }
                    _ => "mixed fixture interrupted",
                };
                ensure!(format!("{error:#}").contains(expected_fault), "{error:#}");
                ensure!(
                    adapter.executions.get() == 1
                        && adapter.rewards.get() == 1
                        && adapter.commits.get() == 1
                );
                ensure!(chain.last_block_number_typed()?.as_u64() + 1 == number(&period["number"]));
                ensure!(
                    application
                        .final_chain()
                        .external_evm_pending_publication_raw()?
                        .is_some()
                );
                ensure!(
                    application
                        .final_chain()
                        .block_header_raw(number(&period["number"]))?
                        .is_none()
                );
                return Ok(());
            }
            if number(&period["number"]) == 4 {
                eprintln!(
                    "CRYPTO_PUBLICATION_DIAGNOSTIC {}",
                    serde_json::json!({"period":4,"stage":"task_returned","outcome":if outcome.is_ok(){"OK"}else{"ERROR"},"expected_final_root":period["final"]["root"],"actual_published_root":"NOT_OBSERVED","published_receipts":"NOT_OBSERVED"})
                );
            }
            let report = outcome?;
            ensure!(report.period.as_u64() == number(&period["number"]));
            verify_receipts(&application, period)?;
            ensure!(
                adapter.inner.concrete.borrow().is_none(),
                "concrete commit consumes its period handle"
            );
            Ok(())
        };
        execute()
            .with_context(|| format!("native integration period {}", number(&period["number"])))?;
        let had_fault = adapter.fault != mixed_recovery::Fault::None;
        // Drop the remaining adapter owners before opening the database again.
        epoch = {
            let owned_adapter = adapter;
            owned_adapter.inner.state_api_epoch
        };
        if had_fault {
            return Ok(());
        }
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
        ensure!(
            observed.committed.state_root == fixed(&period["final"]["root"]),
            "published canonical root"
        );
        ensure!(chain.last_block_number_typed()? == observed.committed.period);
        ensure!(
            application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_none()
        );
        if let Some(observer) = &ordinary {
            observer.borrow_mut().promote(
                concrete_path,
                observed.identity,
                observed.committed,
                observed.generation,
            )?;
        }
        let reader = ConcreteStateReader::open_read_only(concrete_path, observed.committed)?;
        verify_final_reader(&reader, &period["final"])?;
        if let Some(probe) = period.get("proof_marker_probe") {
            ensure!(
                probe["callback_called"] == false
                    && probe["nonempty"] == false
                    && probe["value"] == ""
            );
            ensure!(
                probe["address"] == "00000000000000000000000000000000000000ee"
                    && probe["key"]
                        == "dc35efc475e49bffe215eea886063ea790bde40e358685c728d52129f16ae9db"
            );
            let state = reader
                .verify_storage_path(
                    fixed(&probe["address"]),
                    ConcreteStorageKey(fixed(&probe["key"])),
                )
                .with_context(|| {
                    format!(
                        "invalid_signature_a proof_marker_probe period={} address={} key={}",
                        number(&period["number"]),
                        probe["address"],
                        probe["key"]
                    )
                })?;
            ensure!(
                state == rustaxa_storage::ConcreteStoragePath::NonMember,
                "proof marker is a member"
            );
        }
        if number(&period["number"]) == 4 {
            let key = "dc35efc475e49bffe215eea886063ea790bde40e358685c728d52129f16ae9db";
            let address = "00000000000000000000000000000000000000ee";
            let rows = period["transactions"]
                .as_array()
                .context("period transactions")?;
            if let Some(slash) = rows.iter().find(|row| row["name"] == "slash_a")
                && slash["ordered_raw_writes"]
                    .as_array()
                    .context("slash writes")?
                    .iter()
                    .any(|row| {
                        row["address"] == address && row["key"] == key && row["value"] == "01"
                    })
            {
                let path = reader
                    .verify_storage_path(
                        fixed(&serde_json::json!(address)),
                        ConcreteStorageKey(fixed(&serde_json::json!(key))),
                    )
                    .with_context(|| {
                        format!(
                            "positive proof marker period=4 address={} key={}",
                            address, key
                        )
                    })?;
                ensure!(
                    path == rustaxa_storage::ConcreteStoragePath::Member(vec![1]),
                    "positive proof marker path"
                );
            }
        }
        drop(reader);
        // The long corpus verifies the current reader at every publication
        // above, and sweeps every retained reader at both distributions and
        // at the concrete close/reopen boundary. Avoid opening the entire
        // same history after each empty non-distribution period.
        let sweep = fixture["profile"] != "synthetic_mainnet_era"
            || observed.committed.period.as_u64().is_multiple_of(100)
            || observed.committed.period.as_u64() == 25_706_905
            || index + 1 == periods.len();
        if sweep {
            verify_retained(concrete_path, observed.committed, fixture)?;
        }
        // Reopen the consumed storage handle without replacing the live
        // FinalChain scheduler or the process-local StateAPI epoch.
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, identity)?;
        concrete = ConcreteStateLifecycle::open(concrete_path, identity, observed.committed)?;
    }
    drop(concrete);
    drop(chain);
    drop(application);
    Ok(())
}
