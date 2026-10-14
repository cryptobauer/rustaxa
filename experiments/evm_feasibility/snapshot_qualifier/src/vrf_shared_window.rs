//! Sealed diagnostic VRF windows. Authenticate fixed raw authority and all71 prior
//! proofs before constructing47 requests. Each profile has fixed vote-row indices;
//! no caller can supply a voter, root, key or ceiling. Failures qualify no prefix.
//! Embedded22 source pins require the separately reviewed complete74-source freeze.
use crate::{
    artifact_admission as admission, interval_history, native_witness as native,
    parent_input_handoff, parent_reward_fields as fields, paths, vrf_witness as first,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteDiagnosticPlan, ConcreteDiagnosticTuple, ConcreteVrfPlan, ConcreteVrfResidualPlan,
    ConcreteVrfVoterPlan, ConcreteWitnessLimits, ConcreteWitnessLogical, ConcreteWitnessPhysical,
    capture_vrf_residual_run, capture_vrf_voter_run, replay_vrf_bundle, replay_vrf_residual_bundle,
    replay_vrf_voter_bundle,
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
    Eighth,
    Ninth,
    Tenth,
    ResidualFirst,
}
const RESIDUAL_VOTER: &str = "03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b";
const RESIDUAL_PROFILE: &str = "n4_residual_vrf_03bcf4079a8aaf266aaa8e07ca2a4c5a63f6881b_v1";
const RESIDUAL_KEY: &str = "89ef37281f8220316458526a5c28a4518ec2cd3eaf487baa7055e8831cb89c5b";
const RESIDUAL_INDICES: [usize; 46] = [
    62, 119, 179, 236, 344, 401, 461, 521, 578, 638, 695, 749, 806, 866, 920, 977, 1034, 1091,
    1151, 1211, 1271, 1328, 1382, 1430, 1487, 1547, 1604, 1661, 1715, 1772, 1832, 1889, 1943, 1997,
    2057, 2114, 2171, 2231, 2285, 2345, 2399, 2459, 2516, 2576, 2624, 2684,
];
const VOTERS: [&str; 10] = [
    "138c1b9baea7d93a981ba694f5318f735c997298",
    "264a30e880d3b53bb2cdeb234e34201ebf9a9965",
    "329d40b5f15cfe4de0e9953d0599d2a39a65fc04",
    "575910424ac437467726542781508b2220b71ce8",
    "6bf44c438f6328640b0bc5d7d7766310cf49610f",
    "a6cec53b4d7920709f05ca1f7ac0d66338d29cb9",
    "c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa",
    "d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef",
    "d6d73e4c65c92787a2bbc80d109c293d8d3477af",
    "f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d",
];
const PROFILES: [&str; 10] = [
    "n4_remaining_vrf_voter_138c1b9baea7d93a981ba694f5318f735c997298_v1",
    "n4_remaining_vrf_voter_264a30e880d3b53bb2cdeb234e34201ebf9a9965_v1",
    "n4_remaining_vrf_voter_329d40b5f15cfe4de0e9953d0599d2a39a65fc04_v1",
    "n4_remaining_vrf_voter_575910424ac437467726542781508b2220b71ce8_v1",
    "n4_remaining_vrf_voter_6bf44c438f6328640b0bc5d7d7766310cf49610f_v1",
    "n4_remaining_vrf_voter_a6cec53b4d7920709f05ca1f7ac0d66338d29cb9_v1",
    "n4_remaining_vrf_voter_c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa_v1",
    "n4_remaining_vrf_voter_d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef_v1",
    "n4_remaining_vrf_voter_d6d73e4c65c92787a2bbc80d109c293d8d3477af_v1",
    "n4_remaining_vrf_voter_f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d_v1",
];
const PRIOR_INDICES: [usize; 10] = [1, 2, 3, 6, 7, 10, 13, 14, 15, 18];
const INDICES: [[usize; 47]; 10] = [
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
    [
        101, 161, 221, 275, 326, 383, 443, 503, 563, 620, 677, 734, 788, 848, 905, 959, 1016, 1073,
        1133, 1193, 1253, 1313, 1364, 1415, 1469, 1529, 1586, 1643, 1700, 1757, 1814, 1871, 1925,
        1979, 2039, 2096, 2153, 2213, 2267, 2327, 2384, 2441, 2498, 2558, 2609, 2666, 2726,
    ],
    [
        104, 164, 224, 278, 329, 386, 446, 506, 566, 623, 680, 737, 791, 851, 908, 962, 1019, 1076,
        1136, 1196, 1256, 1316, 1367, 1418, 1472, 1532, 1589, 1646, 1703, 1760, 1817, 1874, 1928,
        1982, 2042, 2099, 2156, 2216, 2270, 2330, 2387, 2444, 2501, 2561, 2612, 2669, 2729,
    ],
    [
        113, 173, 230, 287, 338, 395, 455, 515, 572, 632, 689, 743, 800, 860, 914, 971, 1028, 1085,
        1145, 1205, 1265, 1322, 1376, 1424, 1481, 1541, 1598, 1655, 1709, 1769, 1826, 1883, 1937,
        1991, 2051, 2108, 2165, 2225, 2279, 2339, 2396, 2453, 2510, 2570, 2618, 2678, 2738,
    ],
];
impl Profile {
    fn voter(self) -> &'static str {
        if self == Self::ResidualFirst {
            RESIDUAL_VOTER
        } else {
            VOTERS[self as usize]
        }
    }
    fn name(self) -> &'static str {
        if self == Self::ResidualFirst {
            RESIDUAL_PROFILE
        } else {
            PROFILES[self as usize]
        }
    }
    fn prior_index(self) -> usize {
        if self == Self::ResidualFirst {
            0
        } else {
            PRIOR_INDICES[self as usize]
        }
    }
    fn payloads(self) -> usize {
        if self == Self::ResidualFirst { 46 } else { 47 }
    }
}
pub(crate) fn select(profile: Profile, rows: &Value) -> Result<Value> {
    if profile == Profile::ResidualFirst {
        return select_residual(rows);
    }
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
/// Select the fixed partial-period raw request set. The omitted period has no
/// payload; it remains in the authenticated constructor/retention context.
fn select_residual(rows: &Value) -> Result<Value> {
    let rows = rows.as_array().context("residual original rows")?;
    ensure!(rows.len() == 2742, "residual original count");
    let selected: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            r["voter_hex"] == RESIDUAL_VOTER
                && r["semantic_key"] == "fe_vrf_00_04_address"
                && r["delayed_period"] != FIRST
        })
        .collect();
    ensure!(selected.len() == 46, "fixed residual count");
    let mut out = Vec::with_capacity(46);
    for (i, (index, row)) in selected.into_iter().enumerate() {
        let delayed_index = if i < 4 { i + 1 } else { i + 2 };
        let d = FIRST + delayed_index as u64;
        ensure!(
            index == RESIDUAL_INDICES[i]
                && row["delayed_period"] == d
                && row["qualified"] == false
                && row["value"].is_null()
                && row["proof"].is_null(),
            "residual request/order drift"
        );
        let refs = row["source_references"]
            .as_array()
            .context("residual vote references")?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|r| r["period"] == d + 7),
            "residual source period"
        );
        out.push(json!({"parent_manifest_index":index,"source_row":row}));
    }
    Ok(json!(out))
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
    ensure!(index == profile.prior_index(), "prior selected index");
    let values = first::decode_values(&verified.logical)?;
    let value = accepted_prior_value(&verified.logical[index], &verified.physical[index])?;
    if profile == Profile::ResidualFirst {
        ensure!(
            hex::encode(value)
                == "6fabaf3eaa629b97b1dfaf77b9025072a1197299dd90cb19e46038794e91447b"
                && admission::sha(value)
                    == "1840cc6d06879636b32f20952e587930ba6c2cd12a32c48a3c72261574db4862",
            "fixed residual prior binding"
        );
    }
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
trait PlanView {
    fn context(&self) -> &ConcreteDiagnosticPlan;
    fn tuples(&self) -> &[ConcreteDiagnosticTuple];
}
impl PlanView for ConcreteVrfVoterPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        self.context()
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        self.tuples()
    }
}
impl PlanView for ConcreteVrfResidualPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        self.context()
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        self.tuples()
    }
}
enum WindowPlan {
    Complete(ConcreteVrfVoterPlan),
    Residual(ConcreteVrfResidualPlan),
}
impl PlanView for WindowPlan {
    fn context(&self) -> &ConcreteDiagnosticPlan {
        match self {
            Self::Complete(p) => p.context(),
            Self::Residual(p) => p.context(),
        }
    }
    fn tuples(&self) -> &[ConcreteDiagnosticTuple] {
        match self {
            Self::Complete(p) => p.tuples(),
            Self::Residual(p) => p.tuples(),
        }
    }
}
fn window_authority(profile: Profile) -> Result<(WindowPlan, Value, Value)> {
    if profile == Profile::ResidualFirst {
        let (p, s, a) =
            authority_with(profile, |first, _| Ok(ConcreteVrfResidualPlan::new(first)?))?;
        Ok((WindowPlan::Residual(p), s, a))
    } else {
        let (p, s, a) = authority(profile)?;
        Ok((WindowPlan::Complete(p), s, a))
    }
}
pub(crate) fn authority(profile: Profile) -> Result<(ConcreteVrfVoterPlan, Value, Value)> {
    ensure!(
        profile != Profile::ResidualFirst,
        "complete profile required"
    );
    authority_with(profile, |first, voter| {
        Ok(ConcreteVrfVoterPlan::new(first, voter)?)
    })
}
fn authority_with<T: PlanView>(
    profile: Profile,
    build: impl FnOnce(ConcreteVrfPlan, [u8; 20]) -> Result<T>,
) -> Result<(T, Value, Value)> {
    let (context, rows, six) = parent_input_handoff::load_authenticated_voter_authority()?;
    let (first_plan, first_selected) = first::selection(context, &rows)?;
    let mut selected = select(profile, &rows)?;
    let prior_index = profile.prior_index();
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
            let plan = build(first_plan, voter)?;
            ensure!(
                plan.tuples().len() == profile.payloads(),
                "fixed payload count"
            );
            for (i, row) in selected
                .as_array_mut()
                .context("window selection")?
                .iter_mut()
                .enumerate()
            {
                let tuple = plan.tuples()[i];
                let fixed_key = match profile {
                    Profile::Seventh => {
                        Some("fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f")
                    }
                    Profile::Eighth => {
                        Some("d801919ffd29925243c28612983151535ef289be8872b1db4b4b41885167ee42")
                    }
                    Profile::Ninth => {
                        Some("da94f95efae9d2677940e6f37edbad5613af9bdb4d920a54465aae06f871e07d")
                    }
                    Profile::Tenth => {
                        Some("afff6c972f0508d1a892f0fb547e3044bf50ee2df3293d56c78ba7ab8939aebe")
                    }
                    Profile::ResidualFirst => Some(RESIDUAL_KEY),
                    _ => None,
                };
                if let Some(key) = fixed_key {
                    ensure!(
                        hex::encode(tuple.key.0) == key,
                        "fixed successor key binding"
                    );
                }
                ensure!(
                    row["source_row"]["delayed_period"] == tuple.identity.period.as_u64(),
                    "authenticated tuple period"
                );
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
        Profile::Eighth => [
            pin!("bin/live_vrf_eighth_voter_window.rs"),
            pin!("bin/replay_vrf_eighth_voter_window.rs"),
        ],
        Profile::Ninth => [
            pin!("bin/live_vrf_ninth_voter_window.rs"),
            pin!("bin/replay_vrf_ninth_voter_window.rs"),
        ],
        Profile::Tenth => [
            pin!("bin/live_vrf_tenth_voter_window.rs"),
            pin!("bin/replay_vrf_tenth_voter_window.rs"),
        ],
        Profile::ResidualFirst => [
            pin!("bin/live_vrf_residual_first.rs"),
            pin!("bin/replay_vrf_residual_first.rs"),
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
    bounds["payload_tuples_exact"] = json!(profile.payloads());
    bounds["proofs_exact"] = json!(51 + profile.payloads());
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
    let (plan, selected, authority) = window_authority(profile)?;
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
    let (plan, selected, authority) = window_authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    interval_history::encode_bounded(&report, REPORT_CAP)?;
    let captured = match &plan {
        WindowPlan::Complete(p) => capture_vrf_voter_run(
            &paths.application,
            &paths.state,
            p,
            ConcreteWitnessLimits::default(),
        ),
        WindowPlan::Residual(p) => capture_vrf_residual_run(
            &paths.application,
            &paths.state,
            p,
            ConcreteWitnessLimits::default(),
        ),
    };
    match captured {
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
    let (plan, selected, authority) = window_authority(profile)?;
    let mut report = base(profile, selected, authority)?;
    let bytes = admission::read_capped(fs::File::open(&input)?, native::BUNDLE_CAP)?;
    let result = match &plan {
        WindowPlan::Complete(p) => {
            replay_vrf_voter_bundle(&bytes, p, ConcreteWitnessLimits::default())
                .map(|v| replay_values(v.logical, v.physical).map(|values| (values, v.usage)))
        }
        WindowPlan::Residual(p) => {
            replay_vrf_residual_bundle(&bytes, p, ConcreteWitnessLimits::default())
                .map(|v| replay_values(v.logical, v.physical).map(|values| (values, v.usage)))
        }
    };
    let (values, usage) = match result {
        Ok(v) => v?,
        Err(f) => {
            stopped(&mut report, &f);
            publish(&output, &report)?;
            anyhow::bail!("window replay stopped; no prefix")
        }
    };
    report["status"] = json!(if profile == Profile::ResidualFirst {
        "all_forty_six_residual_vrf_inputs_authenticated"
    } else {
        "all_forty_seven_vrf_inputs_authenticated"
    });
    report["actual"] = native::usage(usage);
    report["bundle"] = json!({"path":input,"bytes":bytes.len(),"sha256":admission::sha(&bytes)});
    report["values"] = values;
    report["qualification"]["vrf_family_qualified"] = json!(true);
    publish(&output, &report)
}
fn replay_values<const N: usize>(
    logical: [ConcreteWitnessLogical; N],
    physical: [ConcreteWitnessPhysical; N],
) -> Result<Value> {
    let mut values = first::decode_values(&logical)?;
    for (i, (value, physical)) in values
        .as_array_mut()
        .context("window values")?
        .iter_mut()
        .zip(physical)
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
    Ok(values)
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

/// Authenticate the fixed eighth profile offline; errors publish no partial authority.
pub fn eighth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Eighth, output)
}
/// Capture the fixed eighth profile through its independently reviewed one-use guard.
pub fn eighth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Eighth, bundle, report)
}
/// Replay all fixed eighth proofs offline; errors qualify no prefix.
pub fn eighth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Eighth, bundle, report)
}

/// Authenticate the fixed ninth profile offline; errors publish no partial authority.
pub fn ninth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Ninth, output)
}
/// Capture the fixed ninth profile through its independently reviewed one-use guard.
pub fn ninth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Ninth, bundle, report)
}
/// Replay all fixed ninth proofs offline; errors qualify no prefix.
pub fn ninth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Ninth, bundle, report)
}

/// Authenticate the fixed tenth profile offline; errors publish no partial authority.
pub fn tenth_preflight(output: &Path) -> Result<()> {
    preflight(Profile::Tenth, output)
}
/// Capture the fixed tenth profile through its independently reviewed one-use guard.
pub fn tenth_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::Tenth, bundle, report)
}
/// Replay all fixed tenth proofs offline; errors qualify no prefix.
pub fn tenth_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::Tenth, bundle, report)
}

/// Authenticate six raw files and all 71 prior proofs, then publish the fixed
/// 46-row partial-period manifest at an exclusive outside-DB path. No DB open
/// occurs. Invalid authority or output admission returns an error; no prefix
/// supplies authority. Omitted periods remain in the full retention context.
pub fn residual_first_preflight(output: &Path) -> Result<()> {
    preflight(Profile::ResidualFirst, output)
}
/// Capture the sealed 46-row profile from the fixed read-only copy after exact
/// independent guard READY. Exclusive bundle/report outputs must be distinct
/// and outside the DB. All 97 proofs are required. A diagnostic failure attempts
/// a stopped report and returns an error; publication errors may leave no report.
/// No prefix qualifies and no omitted period is assigned a value.
pub fn residual_first_capture(bundle: &Path, report: &Path) -> Result<()> {
    capture(Profile::ResidualFirst, bundle, report)
}
/// Replay a bounded RCV3 bundle without DB input and publish all 46 logical
/// values and physical provenance at an exclusive outside-DB path. Fresh raw
/// authentication, the 71-proof prior barrier, all 97 bundle proofs and exact EOF
/// are required. Errors qualify no prefix and attempt a stopped report when
/// possible. Fact acceptance separately requires all 46 positive members.
pub fn residual_first_replay(bundle: &Path, report: &Path) -> Result<()> {
    replay(Profile::ResidualFirst, bundle, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    // Only reviewed changed storage source pins can differ. Semantic fields
    // remain compared in full; this is an exact source-successor mapping.
    fn storage_successor(path: &str) -> Option<&'static [u8]> {
        match path {
            "../../../../rust/crates/rustaxa-storage/src/lib.rs" => Some(include_bytes!(
                "../../../../rust/crates/rustaxa-storage/src/lib.rs"
            )),
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs" => Some(
                include_bytes!("../../../../rust/crates/rustaxa-storage/src/concrete_state/mod.rs"),
            ),
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs"
                ))
            }
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs"
                ))
            }
            "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs" => {
                Some(include_bytes!(
                    "../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs"
                ))
            }
            _ => None,
        }
    }
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
            (
                Profile::Seventh,
                "/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/seventh-voter/vrf-seventh-voter-window-replay.json",
                59467,
                "f551c202f8b929616294395d0748f8e269b53352748a8411fd7671a9a37a8c5e",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Eighth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/eighth-voter/vrf-eighth-voter-window-replay.json",
                59470,
                "59280657393129d87ead097ecb1ce967f24e6ce5974ec8f3f926aed328f7631d",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Ninth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/ninth-voter/vrf-ninth-voter-window-replay.json",
                59464,
                "3833e18901c4c633e48962f607248a5850c2ce24e759eca94e37955f2c435d5d",
                "vrf_shared_window.rs",
            ),
            (
                Profile::Tenth,
                "/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013/tenth-voter/vrf-tenth-voter-window-replay.json",
                59464,
                "760aab8712c5047c4efd9a1ee3c7ab518d2dd1af468a4f975e65cfa3e2ff9d90",
                "vrf_shared_window.rs",
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
                    let expected = if path == "lib.rs" && profile != Profile::Seventh {
                        Some(("lib.rs", include_bytes!("lib.rs").as_slice()))
                    } else if path == adapter {
                        Some((
                            "vrf_shared_window.rs",
                            include_bytes!("vrf_shared_window.rs").as_slice(),
                        ))
                    } else {
                        storage_successor(path).map(|bytes| (path, bytes))
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
    fn residual_fixed_selection_prior_and_complete_guard() {
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let selected = select(Profile::ResidualFirst, &rows).unwrap();
        assert_eq!(selected.as_array().unwrap().len(), 46);
        for i in 0..46 {
            assert_eq!(selected[i]["parent_manifest_index"], RESIDUAL_INDICES[i]);
            assert_ne!(selected[i]["source_row"]["delayed_period"], FIRST + 5);
        }
        for (field, value) in [
            ("voter_hex", json!(VOTERS[0])),
            ("delayed_period", json!(FIRST + 5)),
            ("qualified", json!(true)),
            ("proof", json!({})),
            ("value", json!("00")),
        ] {
            let mut bad = rows.clone();
            bad[RESIDUAL_INDICES[0]][field] = value;
            assert!(select(Profile::ResidualFirst, &bad).is_err());
        }
        let mut duplicate = rows.clone();
        duplicate[RESIDUAL_INDICES[1]] = duplicate[RESIDUAL_INDICES[0]].clone();
        assert!(select(Profile::ResidualFirst, &duplicate).is_err());
        let mut reorder = rows.clone();
        reorder
            .as_array_mut()
            .unwrap()
            .swap(RESIDUAL_INDICES[0], RESIDUAL_INDICES[1]);
        assert!(select(Profile::ResidualFirst, &reorder).is_err());
        let mut missing = rows.clone();
        missing.as_array_mut().unwrap().remove(RESIDUAL_INDICES[0]);
        assert!(select(Profile::ResidualFirst, &missing).is_err());
        let mut extra = rows.clone();
        extra[65]["voter_hex"] = json!(RESIDUAL_VOTER);
        assert!(select(Profile::ResidualFirst, &extra).is_err());
        let mut refs = rows.clone();
        refs[RESIDUAL_INDICES[0]]["source_references"] = json!([]);
        assert!(select(Profile::ResidualFirst, &refs).is_err());
        assert!(authority(Profile::ResidualFirst).is_err());
        let mut partial_complete = rows.clone();
        partial_complete[INDICES[0][4]]["voter_hex"] = json!(RESIDUAL_VOTER);
        assert!(select(Profile::First, &partial_complete).is_err());
        let (plan, selected, summary) = window_authority(Profile::ResidualFirst).unwrap();
        assert_eq!(summary["prior_first_d"]["selected"]["proof_index"], 51);
        assert_eq!(
            summary["prior_first_d"]["actual"]["admitted_input_bytes"],
            PRIOR_BYTES
        );
        assert_eq!(plan.context().delayed.len(), 48);
        assert_eq!(plan.tuples().len(), 46);
        for (i, t) in plan.tuples().iter().enumerate() {
            assert_eq!(hex::encode(t.key.0), RESIDUAL_KEY);
            assert_eq!(
                selected[i]["tuple"]["identity"],
                native::identity_json(t.identity)
            );
        }
        let report = base(Profile::ResidualFirst, selected, summary).unwrap();
        assert_eq!(report["bounds"]["proofs_exact"], 97);
        assert_eq!(report["bounds"]["payload_tuples_exact"], 46);
        assert!(
            report["qualification"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == false)
        );
        let (context, _, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let (first, _) = first::selection(context, &rows).unwrap();
        let mut bad = canonical_prior().unwrap();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(
            after_prior(Profile::ResidualFirst, first, &bad, |_, _| -> Result<()> {
                panic!("bad prior reached residual constructor")
            })
            .is_err()
        );
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

    #[test]
    fn fixed_successors_authenticate_prior_and_reject_substitution() {
        let (_, rows, _) = parent_input_handoff::load_authenticated_voter_authority().unwrap();
        let prior_bytes = canonical_prior().unwrap();
        for (profile, key, prior_index) in [
            (
                Profile::Eighth,
                "d801919ffd29925243c28612983151535ef289be8872b1db4b4b41885167ee42",
                14,
            ),
            (
                Profile::Ninth,
                "da94f95efae9d2677940e6f37edbad5613af9bdb4d920a54465aae06f871e07d",
                15,
            ),
            (
                Profile::Tenth,
                "afff6c972f0508d1a892f0fb547e3044bf50ee2df3293d56c78ba7ab8939aebe",
                18,
            ),
        ] {
            let (plan, selected, summary) = authority(profile).unwrap();
            assert_eq!(hex::encode(plan.voter()), profile.voter());
            assert_eq!(
                summary["prior_first_d"]["selected"]["proof_index"],
                51 + prior_index
            );
            assert_eq!(selected.as_array().unwrap().len(), 47);
            for (i, tuple) in plan.tuples().iter().enumerate() {
                assert_eq!(hex::encode(tuple.key.0), key);
                assert_eq!(
                    selected[i]["tuple"]["identity"],
                    native::identity_json(tuple.identity)
                );
                assert_eq!(
                    selected[i]["parent_manifest_index"],
                    INDICES[profile as usize][i]
                );
            }
            assert_eq!(
                select(profile, &rows).unwrap(),
                select(profile, &rows).unwrap()
            );
            for index in [INDICES[profile as usize][0], INDICES[profile as usize][46]] {
                let mut wrong = rows.clone();
                wrong[index]["voter_hex"] = json!(VOTERS[6]);
                assert!(select(profile, &wrong).is_err());
                let mut wrong = rows.clone();
                wrong[index]["delayed_period"] = json!(FIRST);
                assert!(select(profile, &wrong).is_err());
            }
            let mut reordered = rows.clone();
            reordered
                .as_array_mut()
                .unwrap()
                .swap(INDICES[profile as usize][0], INDICES[profile as usize][1]);
            assert!(select(profile, &reordered).is_err());
            let mut duplicate = rows.clone();
            duplicate[INDICES[profile as usize][1]] =
                duplicate[INDICES[profile as usize][0]].clone();
            assert!(select(profile, &duplicate).is_err());
            let mut missing = rows.clone();
            missing
                .as_array_mut()
                .unwrap()
                .remove(INDICES[profile as usize][0]);
            assert!(select(profile, &missing).is_err());
            let (context, _, _) =
                parent_input_handoff::load_authenticated_voter_authority().unwrap();
            let (first_plan, _) = first::selection(context, &rows).unwrap();
            let mut bad_prior = prior_bytes.clone();
            bad_prior[0] ^= 1;
            assert!(
                after_prior(profile, first_plan, &bad_prior, |_, _| -> Result<()> {
                    panic!("prior failure reached window construction")
                })
                .is_err()
            );
            let old_path = Path::new(
                "/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013/seventh-voter/vrf-seventh-voter-window.rcv2",
            );
            let old = admission::evidence(
                old_path,
                old_path,
                "a85b207821918e85e687180f94eca13a7e1dab6fc5659cbd4a80d917d476bf9f",
                680089,
                680089,
            )
            .unwrap();
            let failure = replay_vrf_voter_bundle(&old, &plan, ConcreteWitnessLimits::default())
                .err()
                .unwrap();
            assert_eq!(
                failure.error,
                rustaxa_storage::ConcreteWitnessError::Identity
            );
            assert_eq!(failure.index, 0);
            assert_eq!(
                failure
                    .usage
                    .used(rustaxa_storage::ConcreteWitnessResource::PayloadTuples),
                0
            );
        }
    }
}
