//! Sealed diagnostic VRF windows. Authenticate fixed raw authority and all71 prior
//! proofs before constructing47 requests. Each profile has fixed vote-row indices;
//! no caller can supply a voter, root, key or ceiling. Failures qualify no prefix.
//! Embedded22 source pins require the separately reviewed complete68-source freeze.
use crate::{
    artifact_admission as admission, interval_history, native_witness as native,
    parent_input_handoff, parent_reward_fields as fields, paths, vrf_witness as first,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteVrfPlan, ConcreteVrfVoterPlan, ConcreteWitnessLimits, ConcreteWitnessLogical,
    ConcreteWitnessPhysical, capture_vrf_voter_run, replay_vrf_bundle, replay_vrf_voter_bundle,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
pub const REPORT_CAP: usize = 131072;
const FIRST: u64 = 25706894;
const PRIOR_PATH: &str =
    "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-first-d.rcv1";
const PRIOR_BYTES: usize = 417594;
const PRIOR_SHA: &str = "6382e8f25cc9ec2225e563111e37b9ad8c8beae4167543cb4d9e915a7d5cbb61";
/// Only crate-owned fixed entrypoints can select a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Profile {
    First,
    Second,
    Third,
    Fourth,
    Fifth,
    Sixth,
    Seventh,
}
const VOTERS: [&str; 7] = [
    "138c1b9baea7d93a981ba694f5318f735c997298",
    "264a30e880d3b53bb2cdeb234e34201ebf9a9965",
    "329d40b5f15cfe4de0e9953d0599d2a39a65fc04",
    "575910424ac437467726542781508b2220b71ce8",
    "6bf44c438f6328640b0bc5d7d7766310cf49610f",
    "a6cec53b4d7920709f05ca1f7ac0d66338d29cb9",
    "c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa",
];
const PROFILES: [&str; 7] = [
    "n4_remaining_vrf_voter_138c1b9baea7d93a981ba694f5318f735c997298_v1",
    "n4_remaining_vrf_voter_264a30e880d3b53bb2cdeb234e34201ebf9a9965_v1",
    "n4_remaining_vrf_voter_329d40b5f15cfe4de0e9953d0599d2a39a65fc04_v1",
    "n4_remaining_vrf_voter_575910424ac437467726542781508b2220b71ce8_v1",
    "n4_remaining_vrf_voter_6bf44c438f6328640b0bc5d7d7766310cf49610f_v1",
    "n4_remaining_vrf_voter_a6cec53b4d7920709f05ca1f7ac0d66338d29cb9_v1",
    "n4_remaining_vrf_voter_c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa_v1",
];
const PRIOR_INDICES: [usize; 7] = [1, 2, 3, 6, 7, 10, 13];
const INDICES: [[usize; 47]; 7] = [
    [
        65, 122, 182, 239, 293, 347, 404, 464, 524, 581, 641, 698, 752, 809, 869, 923, 980, 1037,
        1094, 1154, 1214, 1274, 1331, 1385, 1433, 1490, 1550, 1607, 1664, 1718, 1775, 1835, 1892,
        1946, 2000, 2060, 2117, 2174, 2234, 2288, 2348, 2402, 2462, 2519, 2579, 2627, 2687,
    ],
    [
        68, 125, 185, 242, 296, 350, 407, 467, 527, 584, 644, 701, 755, 812, 872, 926, 983, 1040,
        1097, 1157, 1217, 1277, 1334, 1388, 1436, 1493, 1553, 1610, 1667, 1721, 1778, 1838, 1895,
        1949, 2003, 2063, 2120, 2177, 2237, 2291, 2351, 2405, 2465, 2522, 2582, 2630, 2690,
    ],
    [
        71, 128, 188, 245, 299, 353, 410, 470, 530, 587, 647, 704, 758, 815, 875, 929, 986, 1043,
        1100, 1160, 1220, 1280, 1337, 1391, 1439, 1496, 1556, 1613, 1670, 1724, 1781, 1841, 1898,
        1952, 2006, 2066, 2123, 2180, 2240, 2294, 2354, 2408, 2468, 2525, 2585, 2633, 2693,
    ],
    [
        80, 137, 197, 251, 308, 359, 419, 479, 539, 596, 656, 710, 764, 824, 881, 938, 992, 1049,
        1109, 1169, 1229, 1289, 1346, 1397, 1445, 1505, 1565, 1619, 1679, 1733, 1790, 1847, 1904,
        1958, 2015, 2075, 2132, 2189, 2246, 2303, 2360, 2417, 2474, 2534, 2591, 2642, 2702,
    ],
    [
        83, 140, 200, 254, 311, 362, 422, 482, 542, 599, 659, 713, 767, 827, 884, 941, 995, 1052,
        1112, 1172, 1232, 1292, 1349, 1400, 1448, 1508, 1568, 1622, 1682, 1736, 1793, 1850, 1907,
        1961, 2018, 2078, 2135, 2192, 2249, 2306, 2363, 2420, 2477, 2537, 2594, 2645, 2705,
    ],
    [
        89, 149, 209, 263, 314, 371, 431, 491, 551, 608, 665, 722, 776, 836, 893, 947, 1004, 1061,
        1121, 1181, 1241, 1301, 1352, 1406, 1457, 1517, 1574, 1631, 1688, 1745, 1802, 1859, 1913,
        1967, 2027, 2087, 2141, 2201, 2255, 2315, 2372, 2429, 2486, 2546, 2600, 2654, 2714,
    ],
    [
        98, 158, 218, 272, 323, 380, 440, 500, 560, 617, 674, 731, 785, 845, 902, 956, 1013, 1070,
        1130, 1190, 1250, 1310, 1361, 1412, 1466, 1526, 1583, 1640, 1697, 1754, 1811, 1868, 1922,
        1976, 2036, 2093, 2150, 2210, 2264, 2324, 2381, 2438, 2495, 2555, 2606, 2663, 2723,
    ],
];
impl Profile {
    fn voter(self) -> &'static str {
        VOTERS[self as usize]
    }
    fn name(self) -> &'static str {
        PROFILES[self as usize]
    }
}
pub(crate) fn select(profile: Profile, rows: &Value) -> Result<Value> {
    let rows = rows.as_array().context("complete voter rows")?;
    ensure!(rows.len() == 2742, "window original count");
    let mut periods = BTreeMap::<&str, BTreeSet<u64>>::new();
    for row in rows {
        if row["semantic_key"] == "fe_vrf_00_04_address" {
            periods
                .entry(row["voter_hex"].as_str().context("window voter")?)
                .or_default()
                .insert(row["delayed_period"].as_u64().context("window period")?);
        }
    }
    let all: BTreeSet<_> = (FIRST..FIRST + 48).collect();
    let candidates: Vec<_> = periods
        .iter()
        .filter(|(_, p)| **p == all)
        .map(|(v, _)| *v)
        .collect();
    ensure!(
        candidates.len() == 10 && candidates[..=profile as usize] == VOTERS[..=profile as usize],
        "fixed complete voter selector"
    );
    let selected: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["voter_hex"] == profile.voter()
                && r["semantic_key"] == "fe_vrf_00_04_address"
                && r["delayed_period"] != FIRST
        })
        .collect();
    ensure!(selected.len() == 47, "complete remaining voter family");
    let mut output = Vec::with_capacity(47);
    for (i, (index, row)) in selected.into_iter().enumerate() {
        let d = FIRST + 1 + i as u64;
        ensure!(
            index == INDICES[profile as usize][i]
                && row["delayed_period"] == d
                && row["qualified"] == false
                && row["value"].is_null()
                && row["proof"].is_null(),
            "window request/order drift"
        );
        let refs = row["source_references"]
            .as_array()
            .context("window vote references")?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|r| r["period"] == d + 7),
            "window source period"
        );
        output.push(json!({"parent_manifest_index":index,"source_row":row}));
    }
    Ok(json!(output))
}
pub(crate) fn prior(profile: Profile, bytes: &[u8], plan: &ConcreteVrfPlan) -> Result<Value> {
    ensure!(
        bytes.len() == PRIOR_BYTES && admission::sha(bytes) == PRIOR_SHA,
        "prior VRF pin drift"
    );
    let verified = replay_vrf_bundle(bytes, plan, ConcreteWitnessLimits::default())
        .map_err(|e| anyhow::anyhow!("prior VRF proof: {e:?}"))?;
    let index = plan
        .voters()
        .iter()
        .position(|v| hex::encode(v) == profile.voter())
        .context("prior selected voter")?;
    ensure!(
        index == PRIOR_INDICES[profile as usize],
        "prior selected index"
    );
    let values = first::decode_values(&verified.logical)?;
    let value = accepted_prior_value(&verified.logical[index], &verified.physical[index])?;
    Ok(
        json!({"input":{"path":PRIOR_PATH,"bytes":PRIOR_BYTES,"sha256":PRIOR_SHA},
        "actual":native::usage(verified.usage),"semantic_sha256":fields::normalized_digest(&values)?,
        "selected":{"proof_index":51 + index,"identity":native::identity_json(plan.context().delayed[0]),
            "voter_hex":profile.voter(),"value_hex":hex::encode(value),"usable_vrf_key":true}}),
    )
}
pub(crate) fn accepted_prior_value<'a>(
    logical: &'a ConcreteWitnessLogical,
    physical: &ConcreteWitnessPhysical,
) -> Result<&'a [u8]> {
    let value = match logical {
        ConcreteWitnessLogical::Member(v) if v.len() == 32 && v.iter().any(|b| *b != 0) => v,
        _ => anyhow::bail!("required accepted firstD member unavailable"),
    };
    ensure!(
        matches!(physical,ConcreteWitnessPhysical::Present(v) if v==value),
        "prior physical member mismatch"
    );
    Ok(value)
}
pub(crate) fn canonical_prior() -> Result<Vec<u8>> {
    let p = Path::new(PRIOR_PATH);
    let mut current = std::path::PathBuf::new();
    for part in p.components() {
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "prior symlink"
        );
    }
    ensure!(
        fs::canonicalize(p)? == p && fs::metadata(p)?.is_file(),
        "prior regular file"
    );
    admission::evidence(p, p, PRIOR_SHA, PRIOR_BYTES, PRIOR_BYTES)
}
pub(crate) fn authority(profile: Profile) -> Result<(ConcreteVrfVoterPlan, Value, Value)> {
    let (context, rows, six) = parent_input_handoff::load_authenticated_voter_authority()?;
    let (first_plan, first_selected) = first::selection(context, &rows)?;
    let mut selected = select(profile, &rows)?;
    let prior_index = PRIOR_INDICES[profile as usize];
    ensure!(
        first_selected[prior_index]["parent_manifest_index"] == prior_index * 3 + 2,
        "prior parent index"
    );
    ensure!(
        first_selected[prior_index]["source_row"]["voter_hex"] == profile.voter(),
        "prior voter binding"
    );
    after_prior(
        profile,
        first_plan,
        &canonical_prior()?,
        |first_plan, old_proof| {
            let voter: [u8; 20] = hex::decode(profile.voter())?
                .try_into()
                .map_err(|_| anyhow::anyhow!("fixed voter width"))?;
            let plan = ConcreteVrfVoterPlan::new(first_plan, voter)?;
            for (i, row) in selected
                .as_array_mut()
                .context("window selection")?
                .iter_mut()
                .enumerate()
            {
                let tuple = plan.tuples()[i];
                if profile == Profile::Seventh {
                    ensure!(
                        hex::encode(tuple.key.0)
                            == "fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f",
                        "seventh key binding"
                    );
                }
                row["tuple"] = json!({"identity":native::identity_json(tuple.identity),"address_hex":hex::encode(tuple.address),
            "logical_key_hex":hex::encode(tuple.key.0),"key_preimage_hex":format!("0004{}", profile.voter())});
            }
            Ok((
                plan,
                selected,
                json!({"six_input_owner":six,"prior_first_d":old_proof,
        "raw_artifact_reads":7,"raw_artifact_bytes":6785636}),
            ))
        },
    )
}
/// The continuation reaches the window/open stage only after fresh prior proof.
pub(crate) fn after_prior<T>(
    profile: Profile,
    plan: ConcreteVrfPlan,
    bytes: &[u8],
    next: impl FnOnce(ConcreteVrfPlan, Value) -> Result<T>,
) -> Result<T> {
    let proof = prior(profile, bytes, &plan)?;
    next(plan, proof)
}

pub(crate) fn base(profile: Profile, selected: Value, authority: Value) -> Result<Value> {
    macro_rules! pin {($p:literal)=>{{let b=include_bytes!($p);
        ensure!(b.len()<=1048576,"window source file cap");
        json!({"path":$p,"bytes":b.len(),"sha256":admission::sha(b)})}};}
    let mut sources = first::sources()?
        .as_array()
        .context("shared VRF sources")?
        .clone();
    sources.push(pin!("vrf_shared_window.rs"));
    sources.extend(match profile {
        Profile::First => [
            pin!("bin/live_vrf_voter_window.rs"),
            pin!("bin/replay_vrf_voter_window.rs"),
        ],
        Profile::Second => [
            pin!("bin/live_vrf_next_voter_window.rs"),
            pin!("bin/replay_vrf_next_voter_window.rs"),
        ],
        Profile::Third => [
            pin!("bin/live_vrf_third_voter_window.rs"),
            pin!("bin/replay_vrf_third_voter_window.rs"),
        ],
        Profile::Fourth => [
            pin!("bin/live_vrf_fourth_voter_window.rs"),
            pin!("bin/replay_vrf_fourth_voter_window.rs"),
        ],
        Profile::Fifth => [
            pin!("bin/live_vrf_fifth_voter_window.rs"),
            pin!("bin/replay_vrf_fifth_voter_window.rs"),
        ],
        Profile::Sixth => [
            pin!("bin/live_vrf_sixth_voter_window.rs"),
            pin!("bin/replay_vrf_sixth_voter_window.rs"),
        ],
        Profile::Seventh => [
            pin!("bin/live_vrf_seventh_voter_window.rs"),
            pin!("bin/replay_vrf_seventh_voter_window.rs"),
        ],
    });
    ensure!(sources.len() == 22, "window source count");
    let bytes = authority["six_input_owner"]["sources"]
        .as_array()
        .context("owner sources")?
        .iter()
        .chain(&sources)
        .try_fold(0u64, |n, p| {
            n.checked_add(p["bytes"].as_u64().context("source bytes")?)
                .context("source total")
        })?;
    ensure!(bytes <= 4194304, "combined window source cap");
    let mut bounds = native::policy();
    bounds["report_bytes"] = json!(REPORT_CAP);
    bounds["authority_input_bytes"] = json!(6785636);
    bounds["raw_artifacts_exact"] = json!(7);
    bounds["payload_tuples_exact"] = json!(47);
    bounds["proofs_exact"] = json!(98);
    let digest = fields::normalized_digest(&selected)?;
    Ok(
        json!({"schema":1,"profile":profile.name(),"selection":selected,"selection_sha256":digest,
        "authority":authority,"vrf_sources":sources,"bounds":bounds,
        "qualification":{"vrf_family_qualified":false,"voter_weights_qualified":false,
        "reward_cache_equal":false,"owner_ready":false,"producer_verified":false,"production_routing_authorized":false}}),
    )
}
fn publish(output: &Path, report: &Value) -> Result<()> {
    admission::publish_exclusive(
        output,
        &interval_history::encode_bounded(report, REPORT_CAP)?,
    )
}
fn stopped(report: &mut Value, failure: &rustaxa_storage::ConcreteDiagnosticFailure) {
    let f = native::failure_report(failure);
    report["status"] = json!("stopped");
    report["failure"] = f["failure"].clone();
    report["actual"] = f["actual"].clone();
    report["values"] = Value::Null;
}
/// Preflight repeats raw-seven authentication and prior proof without DB path
/// inspection. It publishes the complete fixed request/source/bound profile.
pub(crate) fn preflight(profile: Profile, output: &Path) -> Result<()> {
    let output = native::output_path(output)?;
    let (plan, selected, authority) = authority(profile)?;
    let mut report = base(profile, selected, authority)?;
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
    publish(&output, &report)
}
/// Capture exactly forty-seven remaining requests after complete prior-proof
/// authority. One failure stops later reads and publishes no accepted prefix.
pub(crate) fn capture(profile: Profile, bundle_output: &Path, report_output: &Path) -> Result<()> {
    let paths = paths::validate(Path::new(native::COPY), bundle_output)?;
    let reports = paths::validate(Path::new(native::COPY), report_output)?;
    ensure!(paths.output != reports.output, "window outputs overlap");
    let (plan, selected, authority) = authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    interval_history::encode_bounded(&report, REPORT_CAP)?;
    match capture_vrf_voter_run(
        &paths.application,
        &paths.state,
        &plan,
        ConcreteWitnessLimits::default(),
    ) {
        Err(f) => {
            stopped(&mut report, &f);
            publish(&reports.output, &report)?;
            anyhow::bail!("window capture stopped; no prefix")
        }
        Ok(c) => {
            ensure!(c.bundle.len() <= native::BUNDLE_CAP, "window bundle cap");
            report["status"] = json!("captured_requires_offline_replay");
            report["actual"] = native::usage(c.usage);
            report["bundle"] = json!({"path":paths.output,"bytes":c.bundle.len(),"sha256":admission::sha(&c.bundle)});
            let encoded = interval_history::encode_bounded(&report, REPORT_CAP)?;
            admission::publish_exclusive(&paths.output, &c.bundle)?;
            admission::publish_exclusive(&reports.output, &encoded)
        }
    }
}
/// Offline replay repeats raw-seven authority, consumes every proof and returns
/// all forty-seven canonical values/provenance. No DB input is available.
pub(crate) fn replay(profile: Profile, bundle_input: &Path, report_output: &Path) -> Result<()> {
    let output = native::output_path(report_output)?;
    let input = fs::canonicalize(bundle_input)?;
    admission::outside_db(&input)?;
    ensure!(
        fs::metadata(&input)?.is_file() && input != output,
        "window bundle input"
    );
    let (plan, selected, authority) = authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    let bytes = admission::read_capped(fs::File::open(&input)?, native::BUNDLE_CAP)?;
    let verified = match replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default()) {
        Ok(v) => v,
        Err(f) => {
            stopped(&mut report, &f);
            publish(&output, &report)?;
            anyhow::bail!("window replay stopped; no prefix")
        }
    };
    let mut values = first::decode_values(&verified.logical)?;
    for (i, (value, physical)) in values
        .as_array_mut()
        .context("window values")?
        .iter_mut()
        .zip(verified.physical)
        .enumerate()
    {
        value["proof_index"] = json!(51 + i);
        value["physical"] = match physical {
            ConcreteWitnessPhysical::Present(v) => {
                json!({"kind":"present","bytes":v.len(),"sha256":admission::sha(&v)})
            }
            ConcreteWitnessPhysical::Tombstone => json!({"kind":"tombstone"}),
            ConcreteWitnessPhysical::HistoryUnavailable(id) => {
                json!({"kind":"successful_seek_without_matching_version","identity":native::identity_json(id)})
            }
        };
    }
    report["status"] = json!("all_forty_seven_vrf_inputs_authenticated");
    report["actual"] = native::usage(verified.usage);
    report["bundle"] = json!({"path":input,"bytes":bytes.len(),"sha256":admission::sha(&bytes)});
    report["values"] = values;
    report["qualification"]["vrf_family_qualified"] = json!(true);
    publish(&output, &report)
}

/// Authenticate the fixed seventh profile offline and publish its bounded manifest.
pub fn seventh_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Seventh, output)
}
/// Capture the fixed seventh profile once through an independently reviewed guard.
pub fn seventh_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Seventh, bundle, report)
}
/// Consume every seventh proof offline; no database input is available.
pub fn seventh_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Seventh, bundle, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    fn verify_pin(pin: &Value) {
        use sha2::{Digest, Sha256};
        let p = Path::new(pin["path"].as_str().unwrap());
        admission::outside_db(p).unwrap();
        let size = pin["bytes"].as_u64().unwrap();
        assert!(size <= 268435456);
        assert_eq!(fs::metadata(p).unwrap().len(), size);
        let mut file = fs::File::open(p).unwrap();
        let mut hash = Sha256::new();
        let mut buf = [0; 8192];
        let mut count = 0;
        loop {
            let n = file.read(&mut buf).unwrap();
            if n == 0 {
                break;
            };
            count += n as u64;
            assert!(count <= size);
            hash.update(&buf[..n]);
        }
        assert_eq!(count, size);
        assert_eq!(
            hex::encode(hash.finalize()),
            pin["sha256"].as_str().unwrap()
        );
    }
    #[test]
    fn six_saved_windows_have_complete_equivalent_reports() {
        let cases = [
            (
                Profile::First,
                "/home/fry/artifacts/n4-run-01a12d5e-f42f-74f0-86fb-26bc2c433942-20261011/vrf-voter-window-replay.json",
                59391,
                "b6278f844e627f5830d56803a5555668aa982b56ce2c81fdc9b0cb677b9bc3e6",
                "vrf_voter_window.rs",
            ),
            (
                Profile::Second,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/vrf-next-voter-window-replay.json",
                59374,
                "0e3b5e18f00209a7799da50565d90bc273ef226c17cc33c5685578255495db42",
                "vrf_next_voter_window.rs",
            ),
            (
                Profile::Third,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/third-voter/vrf-third-voter-window-replay.json",
                59403,
                "a7f94b240fcdf7293475ae0ec7972fb0d1fb72609703db7080b339066de6c59a",
                "vrf_third_voter_window.rs",
            ),
            (
                Profile::Fourth,
                "/home/fry/artifacts/n4-vrf-round2-01a12d5e-20261012/fourth-voter/vrf-fourth-voter-window-replay.json",
                59410,
                "bd099e106c56454bcf94cb6c2fbadbb5d4f14d0acad9b74786d569af75525815",
                "vrf_fourth_voter_window.rs",
            ),
            (
                Profile::Fifth,
                "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/fifth-voter/vrf-fifth-voter-window-replay.json",
                59403,
                "9f3124ada4f7c88dd2c8037ec0ac7c7e87907bcea61d683df8746054d9f39fce",
                "vrf_fifth_voter_window.rs",
            ),
            (
                Profile::Sixth,
                "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/sixth-voter/vrf-sixth-voter-window-replay.json",
                59404,
                "35c05787ece4bb2b38a3cbe2fb3137019cbb34d79fd24c9406b857821d5fe187",
                "vrf_sixth_voter_window.rs",
            ),
        ];
        let scratch = Path::new("/home/fry/artifacts").join(format!(
            "shared-vrf-equivalence-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&scratch).unwrap();
        for (profile, path, size, sha, adapter) in cases {
            let path = Path::new(path);
            let mut old = admission::parse_unique(
                &admission::evidence(path, path, sha, size, REPORT_CAP).unwrap(),
            )
            .unwrap();
            verify_pin(&old["bundle"]);
            verify_pin(&old["authority"]["six_input_owner"]["executable"]);
            let input = Path::new(old["bundle"]["path"].as_str().unwrap());
            let output = scratch.join(format!("{}.json", profile as usize));
            replay(profile, input, &output).unwrap();
            let mut new = admission::parse_unique(
                &admission::read_capped(fs::File::open(&output).unwrap(), REPORT_CAP).unwrap(),
            )
            .unwrap();
            let exe = &new["authority"]["six_input_owner"]["executable"];
            assert_eq!(
                exe["path"],
                json!(fs::canonicalize(std::env::current_exe().unwrap()).unwrap())
            );
            verify_pin(exe);
            old["authority"]["six_input_owner"]["executable"] = exe.clone();
            for (parent, key) in [("", "vrf_sources"), ("authority", "sources")] {
                let (a, b) = if parent.is_empty() {
                    (&mut old[key], &mut new[key])
                } else {
                    (
                        &mut old[parent]["six_input_owner"][key],
                        &mut new[parent]["six_input_owner"][key],
                    )
                };
                let a = a.as_array_mut().unwrap();
                let b = b.as_array_mut().unwrap();
                assert_eq!(a.len(), b.len());
                for (before, after) in a.iter_mut().zip(b.iter()) {
                    let path = before["path"].as_str().unwrap();
                    let expected = if path == "lib.rs" {
                        Some(("lib.rs", include_bytes!("lib.rs").as_slice()))
                    } else if path == adapter {
                        Some((
                            "vrf_shared_window.rs",
                            include_bytes!("vrf_shared_window.rs").as_slice(),
                        ))
                    } else {
                        None
                    };
                    if let Some((name, bytes)) = expected {
                        assert_eq!(
                            *after,
                            json!({"path":name,"bytes":bytes.len(),"sha256":admission::sha(bytes)})
                        );
                        *before = after.clone();
                    } else {
                        assert_eq!(before, after);
                    }
                }
            }
            assert_eq!(old, new, "complete report for {profile:?}");
            assert_eq!(new["values"].as_array().unwrap().len(), 47);
            assert_eq!(new["qualification"]["vrf_family_qualified"], true);
        }
        fs::remove_dir_all(scratch).unwrap();
    }
    #[test]
    fn seventh_is_fixed_derived_and_reverse_rejected() {
        let (plan, selected, summary) = authority(Profile::Seventh).unwrap();
        assert_eq!(hex::encode(plan.voter()), VOTERS[6]);
        assert_eq!(selected.as_array().unwrap().len(), 47);
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 64);
        assert_eq!(
            summary["prior_first_d"]["selected"]["value_hex"],
            "d574a33dae0aa6a0f41a2f7e3dd6d73d1048292bec55f03c36f27c56067c25fd"
        );
        for tuple in plan.tuples() {
            assert_eq!(
                hex::encode(tuple.key.0),
                "fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f"
            );
        }
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        assert_eq!(
            select(Profile::Seventh, &rows).unwrap(),
            select(Profile::Seventh, &rows).unwrap()
        );
        for index in [INDICES[6][0], INDICES[6][46]] {
            let mut wrong = rows.clone();
            wrong[index]["voter_hex"] = json!(VOTERS[5]);
            assert!(select(Profile::Seventh, &wrong).is_err());
            let mut wrong = rows.clone();
            wrong[index]["delayed_period"] = json!(FIRST);
            assert!(select(Profile::Seventh, &wrong).is_err());
        }
        let mut duplicate = rows.clone();
        duplicate[INDICES[6][1]] = duplicate[INDICES[6][0]].clone();
        assert!(select(Profile::Seventh, &duplicate).is_err());
        let p = Path::new(
            "/home/fry/artifacts/n4-vrf-round3-01a12d5e-20261013/sixth-voter/vrf-sixth-voter-window.rcv2",
        );
        let bytes = admission::evidence(
            p,
            p,
            "7b270eb9a7bd023d66a74e7f3cf775e07321ad5361db08ed083756f54cf04d5b",
            678585,
            678585,
        )
        .unwrap();
        let f = replay_vrf_voter_bundle(&bytes, &plan, ConcreteWitnessLimits::default())
            .err()
            .unwrap();
        assert_eq!(f.error, rustaxa_storage::ConcreteWitnessError::Identity);
        assert_eq!(f.index, 0);
        assert_eq!(
            f.usage
                .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
            0
        );
    }
}
