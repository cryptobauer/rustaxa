//! Complete first delayed-period VRF evidence. Raw signed votes and roots are
//! reauthenticated through the unchanged six-input owner before any DB open.
//! No stake, jail, full voter weight, reward cache or production authority follows.
use crate::{
    artifact_admission as admission, interval_history, native_witness as native,
    parent_input_handoff, parent_reward_fields as fields, paths,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteVrfPlan, ConcreteWitnessLimits, ConcreteWitnessLogical, ConcreteWitnessPhysical,
    capture_vrf_run, replay_vrf_bundle,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub const PROFILE: &str = "n4_vrf_first_delayed_25706894_v1";
const FIRST: u64 = 25_706_894;
const VOTER_SHA: &str = "df11561223b5222e9bb311cf2dc51ee92510a149e164db579d8a7dc767de905a";
const ROOT: &str = "d0e50649639d5d5f4d1dd0be22f017d70787a28d12eead469e4e588c4bc40c49";

fn selection(
    context: rustaxa_storage::ConcreteDiagnosticPlan,
    rows: &Value,
) -> Result<(ConcreteVrfPlan, Value)> {
    ensure!(
        fields::normalized_digest(rows)? == VOTER_SHA,
        "full voter manifest drift"
    );
    ensure!(
        hex::encode(context.delayed[0].state_root) == ROOT,
        "first VRF root drift"
    );
    let all = rows.as_array().context("voter rows")?;
    ensure!(all.len() == 2742, "complete voter row count");
    let mut selected = Vec::new();
    let mut voters = Vec::new();
    for (index, row) in all.iter().enumerate() {
        if row["delayed_period"] == FIRST && row["semantic_key"] == "fe_vrf_00_04_address" {
            ensure!(
                index == selected.len() * 3 + 2
                    && row["qualified"] == false
                    && row["value"].is_null()
                    && row["proof"].is_null(),
                "VRF selection shape"
            );
            let refs = row["source_references"]
                .as_array()
                .context("VRF vote references")?;
            ensure!(
                !refs.is_empty() && refs.iter().all(|r| r["period"] == FIRST + 7),
                "VRF source period"
            );
            let voter: [u8; 20] = hex::decode(row["voter_hex"].as_str().context("VRF voter")?)?
                .try_into()
                .map_err(|_| anyhow::anyhow!("VRF voter width"))?;
            voters.push(voter);
            selected.push(json!({"parent_manifest_index":index,"source_row":row}));
        }
    }
    let voters = voters
        .try_into()
        .map_err(|_| anyhow::anyhow!("complete twenty VRF voters required"))?;
    let plan = ConcreteVrfPlan::new(context, voters)?;
    for (i, row) in selected.iter_mut().enumerate() {
        row["tuple"] = json!({"identity":native::identity_json(plan.tuples()[i].identity),
            "address_hex":hex::encode(plan.tuples()[i].address),
            "logical_key_hex":hex::encode(plan.tuples()[i].key.0),
            "key_preimage_hex":format!("0004{}", hex::encode(plan.voters()[i]))});
    }
    Ok((plan, json!(selected)))
}
fn authority() -> Result<(ConcreteVrfPlan, Value, Value)> {
    let (context, rows, summary) = parent_input_handoff::load_authenticated_voter_authority()?;
    let (plan, selected) = selection(context, &rows)?;
    Ok((plan, selected, summary))
}
fn sources() -> Result<Value> {
    macro_rules! pin { ($p:literal) => {{let b=include_bytes!($p);
        ensure!(b.len()<=1048576,"VRF source file cap");
        json!({"path":$p,"bytes":b.len(),"sha256":admission::sha(b)})}}; }
    let pins = vec![
        pin!("vrf_witness.rs"),
        pin!("bin/live_vrf_witness.rs"),
        pin!("bin/replay_vrf_witness.rs"),
        pin!("lib.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/lib.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/db.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/config.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/final_chain.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/metadata.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_rlp.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/db.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/db_bounded.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/bounded_read.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/db_iterator.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/db_options.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/db_pinnable_slice.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/column_family.rs"),
        pin!("../../../../rust/vendor/rocksdb/src/ffi_util.rs"),
    ];
    ensure!(
        pins.iter()
            .map(|p| p["bytes"].as_u64().unwrap())
            .sum::<u64>()
            <= 4194304,
        "VRF source aggregate cap"
    );
    Ok(json!(pins))
}
fn report_base(selected: Value, authority: Value) -> Result<Value> {
    let source_pins = sources()?;
    ensure!(
        source_pins.as_array().context("VRF source count")?.len() == 19,
        "VRF source count drift"
    );
    let total = authority["sources"]
        .as_array()
        .context("authority source profile")?
        .iter()
        .chain(source_pins.as_array().context("VRF source profile")?)
        .try_fold(0u64, |n, p| {
            n.checked_add(p["bytes"].as_u64().context("source bytes")?)
                .context("source total overflow")
        })?;
    ensure!(total <= 4194304, "combined source aggregate cap");
    let selection_digest = fields::normalized_digest(&selected)?;
    let mut bounds = native::policy();
    bounds["authority_input_bytes"] = json!(6368042);
    bounds["authority_artifact_cap"] = json!(16777216);
    bounds["authority_max_reads"] = json!(11);
    bounds["payload_tuples_exact"] = json!(20);
    bounds["proofs_exact"] = json!(71);
    Ok(json!({"schema":1,"profile":PROFILE,"selection":selected,
        "selection_sha256":selection_digest,"authority":authority,"vrf_sources":source_pins,
        "bounds":bounds,"qualification":{"vrf_family_qualified":false,
        "voter_weights_qualified":false,"reward_cache_equal":false,"owner_ready":false,
        "producer_verified":false,"production_routing_authorized":false}}))
}
/// Authority-only preflight has no DB path inspection or open. Exact command
/// READY remains external to this diagnostic; publication is exclusive/capped.
pub fn preflight(output: &Path) -> Result<()> {
    let output = native::output_path(output)?;
    let (plan, selected, authority) = authority()?;
    let mut report = report_base(selected, authority)?;
    report["status"] = json!("authority_inputs_authenticated_no_database_open");
    report["head"] = native::identity_json(plan.context().head);
    report["parent"] = native::identity_json(plan.context().parent);
    report["delayed"] = json!(
        plan.context()
            .delayed
            .iter()
            .copied()
            .map(native::identity_json)
            .collect::<Vec<_>>()
    );
    report["genesis_hash_hex"] = json!(hex::encode(plan.context().genesis_hash));
    report["head_header_sha256"] = json!(hex::encode(plan.context().head_header_sha256));
    native::publish_report(&output, &report)
}
/// Fixed copy capture of all twenty tuples. One sticky storage failure publishes
/// only first-failure evidence and prevents any subsequent read or accepted prefix.
pub fn capture(bundle_output: &Path, report_output: &Path) -> Result<()> {
    let bundle_paths = paths::validate(Path::new(native::COPY), bundle_output)?;
    let report_paths = paths::validate(Path::new(native::COPY), report_output)?;
    ensure!(
        bundle_paths.output != report_paths.output,
        "VRF outputs overlap"
    );
    let (plan, selected, authority) = authority()?;
    let mut report = report_base(selected, authority)?;
    // Admit the report profile before physical work as well as at publication.
    interval_history::encode_bounded(&report, native::REPORT_CAP)?;
    match capture_vrf_run(
        &bundle_paths.application,
        &bundle_paths.state,
        &plan,
        ConcreteWitnessLimits::default(),
    ) {
        Err(failure) => {
            let failed = native::failure_report(&failure);
            report["status"] = json!("stopped");
            report["failure"] = failed["failure"].clone();
            report["actual"] = failed["actual"].clone();
            report["values"] = Value::Null;
            native::publish_report(&report_paths.output, &report)?;
            anyhow::bail!("VRF capture stopped; no accepted prefix")
        }
        Ok(captured) => {
            ensure!(
                captured.bundle.len() <= native::BUNDLE_CAP,
                "VRF bundle cap"
            );
            report["status"] = json!("captured_requires_offline_replay");
            report["actual"] = native::usage(captured.usage);
            report["bundle"] = json!({"path":bundle_paths.output,"bytes":captured.bundle.len(),
                "sha256":admission::sha(&captured.bundle)});
            let encoded = interval_history::encode_bounded(&report, native::REPORT_CAP)?;
            admission::publish_exclusive(&bundle_paths.output, &captured.bundle)?;
            admission::publish_exclusive(&report_paths.output, &encoded)
        }
    }
}
/// Canonical VRF semantics after complete proof replay. Present bytes remain
/// exactly32 bytes, including a zero key. None comes only from logical proof.
fn decode(logical: &[ConcreteWitnessLogical; 20]) -> Result<Value> {
    let mut values = Vec::with_capacity(20);
    for item in logical {
        values.push(match item {
            ConcreteWitnessLogical::Member(bytes) => {
                ensure!(bytes.len() == 32, "VRF present width");
                json!({"logical":"member","value_hex":hex::encode(bytes),
                    "usable_vrf_key":bytes.iter().any(|b|*b!=0)})
            }
            ConcreteWitnessLogical::NonMember => json!({"logical":"nonmember",
                "value_hex":null,"usable_vrf_key":false}),
        });
    }
    Ok(json!(values))
}
/// Independent offline replay repeats raw vote/root authentication. Only a
/// fully consumed, canonical twenty-request family can publish qualified values.
pub fn replay(bundle_input: &Path, report_output: &Path) -> Result<()> {
    let output = native::output_path(report_output)?;
    let input = fs::canonicalize(bundle_input)?;
    admission::outside_db(&input)?;
    ensure!(
        fs::metadata(&input)?.is_file() && input != output,
        "VRF bundle input"
    );
    let (plan, selected, authority) = authority()?;
    let mut report = report_base(selected, authority)?;
    let bytes = admission::read_capped(fs::File::open(&input)?, native::BUNDLE_CAP)?;
    let verified = match replay_vrf_bundle(&bytes, &plan, ConcreteWitnessLimits::default()) {
        Ok(v) => v,
        Err(failure) => {
            let failed = native::failure_report(&failure);
            report["status"] = json!("stopped");
            report["failure"] = failed["failure"].clone();
            report["actual"] = failed["actual"].clone();
            report["values"] = Value::Null;
            native::publish_report(&output, &report)?;
            anyhow::bail!("VRF replay stopped; no accepted prefix")
        }
    };
    let mut values = match decode(&verified.logical) {
        Ok(v) => v,
        Err(error) => {
            report["status"] = json!("vrf_semantic_rejected");
            report["actual"] = native::usage(verified.usage);
            report["failure"] = json!({"phase":"VrfDecode","reason":error.to_string()});
            report["values"] = Value::Null;
            native::publish_report(&output, &report)?;
            return Err(error);
        }
    };
    for (i, (value, physical)) in values
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(verified.physical)
        .enumerate()
    {
        value["proof_index"] = json!(51 + i);
        value["physical"] = match physical {
            ConcreteWitnessPhysical::Present(raw) => json!({"kind":"present",
                "bytes":raw.len(),"sha256":admission::sha(&raw)}),
            ConcreteWitnessPhysical::Tombstone => json!({"kind":"tombstone"}),
            ConcreteWitnessPhysical::HistoryUnavailable(identity) => json!({
                "kind":"successful_seek_without_matching_version",
                "identity":native::identity_json(identity)}),
        };
    }
    report["status"] = json!("all_twenty_vrf_inputs_authenticated");
    report["actual"] = native::usage(verified.usage);
    report["bundle"] = json!({"path":input,"bytes":bytes.len(),"sha256":admission::sha(&bytes)});
    report["values"] = values;
    report["qualification"]["vrf_family_qualified"] = json!(true);
    native::publish_report(&output, &report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_vrf_width_zero_nonzero_and_nonmembership() {
        let mut logical = std::array::from_fn(|_| ConcreteWitnessLogical::NonMember);
        logical[0] = ConcreteWitnessLogical::Member(vec![0; 32]);
        logical[1] = ConcreteWitnessLogical::Member(vec![1; 32]);
        let v = decode(&logical).unwrap();
        assert_eq!(v[0]["usable_vrf_key"], false);
        assert_eq!(v[1]["usable_vrf_key"], true);
        assert!(v[2]["value_hex"].is_null());
        for n in [0, 1, 31, 33, 64] {
            logical[19] = ConcreteWitnessLogical::Member(vec![1; n]);
            assert!(decode(&logical).is_err());
        }
    }
    #[test]
    fn full_raw_authority_rederives_twenty_voters_and_rejects_drift() {
        let (context, rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let (plan, selected) = selection(context, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 20);
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(selected[i]["parent_manifest_index"], i * 3 + 2);
            assert_eq!(t.identity.period.as_u64(), FIRST);
            assert_eq!(t.address[19], 0xfe);
        }
        let (context, mut rows, _) =
            parent_input_handoff::load_authenticated_voter_authority().unwrap();
        rows[2]["voter_hex"] = json!("0000000000000000000000000000000000000000");
        assert!(selection(context, &rows).is_err());
    }
}
