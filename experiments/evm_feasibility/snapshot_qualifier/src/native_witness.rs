//! Fixed N4 native total/jail witness commands. Header admission is a separate
//! finite authority step outside the storage witness budget. No writable owner,
//! full reward cache, voter weight or production routing is constructed here.
use crate::{
    artifact_admission as admission, delayed_headers, interval_history, parent_reward_fields, paths,
};
use anyhow::{Context, Result, ensure};
use rustaxa_storage::{
    ConcreteDiagnosticFailure, ConcreteDiagnosticPlan, ConcreteDiagnosticTuple,
    ConcreteWitnessLimits, ConcreteWitnessLogical, ConcreteWitnessResource as Resource,
    ConcreteWitnessUsage, capture_diagnostic_run, replay_diagnostic_bundle,
};
use rustaxa_types::{
    FinalChainBlockNumber, StoredFinalChainBlockHeader,
    codec::rlp::final_chain::StoredBlockHeaderRlp,
    concrete_state::{ConcreteStateIdentity, ConcreteStorageKey},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use tiny_keccak::{Hasher, Keccak};

pub const COPY: &str = "/workspaces/rustaxa-evm/local/evm-state-db/snapshot-litenode-copy";
pub const AUTHORITY_INPUT_CAP: usize = 5_281_184;
pub const BUNDLE_CAP: usize = 134_217_728;
pub const REPORT_CAP: usize = 65_536;
const FIRST: u64 = 25_706_894;
const HEAD: u64 = 25_706_949;
const PARENT_PATH: &str = "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/parent-cache-fields-cap-tests.json";
const PARENT_SIZE: usize = 2_257_600;
const PARENT_SHA: &str = "b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b";
const DELAYED_PATH: &str = "/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/delayed-header-capture.json";
const DELAYED_SIZE: usize = 256_840;
const DELAYED_SHA: &str = "7d57845a66978c6107b2d3f733f7e9e743cde0761926d07060517bc11e7c867f";
const RESOURCES: [Resource; 13] = [
    Resource::KeyBytes,
    Resource::RowBytes,
    Resource::RawCopyBytes,
    Resource::CanonicalBytes,
    Resource::EncodedBytes,
    Resource::Operations,
    Resource::Visits,
    Resource::Records,
    Resource::FrameInspections,
    Resource::ConstructorIdentities,
    Resource::Registrations,
    Resource::DistinctIdentities,
    Resource::PayloadTuples,
];
fn keccak(bytes: &[u8]) -> [u8; 32] {
    let mut state = Keccak::v256();
    state.update(bytes);
    let mut out = [0; 32];
    state.finalize(&mut out);
    out
}
fn identity(period: u64, header: &[u8]) -> Result<ConcreteStateIdentity> {
    let decoded = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(header))?;
    Ok(ConcreteStateIdentity {
        period: FinalChainBlockNumber::from(period),
        state_root: decoded.state_root.0,
    })
}
fn manifest(delayed: [ConcreteStateIdentity; 48]) -> [ConcreteDiagnosticTuple; 96] {
    let mut total = [0; 20];
    total[19] = 0xfe;
    let mut jail = [0; 20];
    jail[19] = 0xee;
    let mut jail_key = [0; 32];
    jail_key[31] = 2;
    std::array::from_fn(|index| ConcreteDiagnosticTuple {
        identity: delayed[index % 48],
        address: if index < 48 { total } else { jail },
        key: ConcreteStorageKey(if index < 48 { keccak(&[4]) } else { jail_key }),
    })
}
/// Load only three exact independently accepted artifacts. Their aggregate raw
/// byte admission is separate from the one application storage witness budget.
/// Re-authenticate all chain links, derive roots from raw headers and compare the
/// exact missing-input manifest; retained report totals never supply authority.
pub fn load_fixed_plan() -> Result<ConcreteDiagnosticPlan> {
    ensure!(
        delayed_headers::INPUT_SIZE
            .checked_add(PARENT_SIZE)
            .and_then(|n| n.checked_add(DELAYED_SIZE))
            == Some(AUTHORITY_INPUT_CAP),
        "authority cap drift"
    );
    let interval_bytes = admission::evidence(
        Path::new(delayed_headers::INPUT_PATH),
        Path::new(delayed_headers::INPUT_PATH),
        delayed_headers::INPUT_SHA,
        delayed_headers::INPUT_SIZE,
        delayed_headers::INPUT_SIZE,
    )?;
    let value = admission::parse_unique(&interval_bytes)?;
    let rows: interval_history::IntervalHistoryRows =
        serde_json::from_value(value["authentication"]["rows"].clone())?;
    interval_history::authenticate_interval(&rows)?;
    let parent_bytes = admission::evidence(
        Path::new(PARENT_PATH),
        Path::new(PARENT_PATH),
        PARENT_SHA,
        PARENT_SIZE,
        PARENT_SIZE,
    )?;
    let parent = admission::parse_unique(&parent_bytes)?;
    parent_reward_fields::validate_field_report(&parent)?;
    let derived = parent_reward_fields::derive_available_fields(&rows)?;
    ensure!(
        parent["missing_inputs"]["delayed_roots_and_totals"]
            == derived["missing_inputs"]["delayed_roots_and_totals"],
        "native manifest drift"
    );
    let delayed_bytes = admission::evidence(
        Path::new(DELAYED_PATH),
        Path::new(DELAYED_PATH),
        DELAYED_SHA,
        DELAYED_SIZE,
        DELAYED_SIZE,
    )?;
    let evidence = delayed_headers::Evidence::admit_fixed(&interval_bytes)?;
    let replay = delayed_headers::replay_fixed_historical(&evidence, &delayed_bytes)?;
    ensure!(
        replay["authentication"]["status"] == "seven_delayed_headers_authenticated_to_local_h",
        "delayed headers unavailable"
    );
    let earliest = replay["authentication"]["rows"]
        .as_array()
        .context("delayed roots")?;
    ensure!(earliest.len() == 7, "delayed root count");
    let head = identity(HEAD, &rows.head.header)?;
    let parent = identity(HEAD - 1, &rows.predecessors[0].header)?;
    let mut delayed = [head; 48];
    for (index, target) in delayed.iter_mut().enumerate() {
        let period = FIRST + index as u64;
        *target = if index < 7 {
            let row = earliest
                .iter()
                .find(|row| row["period"] == period)
                .context("missing earliest header")?;
            let raw = row["header_hex"].as_str().context("raw earliest header")?;
            ensure!(
                raw.len() <= 2 * delayed_headers::ROW_CAP,
                "earliest header cap"
            );
            identity(period, &hex::decode(raw)?)?
        } else {
            let offset = (HEAD - 1 - period) as usize;
            identity(
                period,
                &rows
                    .predecessors
                    .get(offset)
                    .context("missing interval header")?
                    .header,
            )?
        };
    }
    Ok(ConcreteDiagnosticPlan {
        genesis_hash: rows
            .head
            .genesis
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("genesis width"))?,
        head_header_sha256: Sha256::digest(&rows.head.header).into(),
        head,
        parent,
        tuples: manifest(delayed),
        delayed,
    })
}
/// Hash the compiled source profile and this executable. Executable admission
/// uses a fixed8KiB buffer outside the storage witness budget.
fn source_profile() -> Result<Value> {
    macro_rules! pin { ($path:literal) => {{ let bytes=include_bytes!($path);
        json!({"path":$path,"bytes":bytes.len(),"sha256":admission::sha(bytes)}) }}; }
    let executable = std::env::current_exe()?;
    let mut file = fs::File::open(&executable)?;
    let mut buffer = [0u8; 8192];
    let mut length = 0usize;
    let mut hash = Sha256::new();
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(count)
            .context("executable length overflow")?;
        ensure!(length <= 256 * 1024 * 1024, "executable byte cap");
        hash.update(&buffer[..count]);
    }
    let executable_sha = hex::encode(hash.finalize());
    Ok(
        json!({"compiled":[pin!("native_witness.rs"),pin!("delayed_headers.rs"),
        pin!("bin/live_native_witness.rs"),pin!("bin/replay_native_witness.rs"),
        pin!("bin/parent_cache_fields/admission.rs"),pin!("paths.rs"),pin!("interval_history.rs"),
        pin!("parent_reward_fields.rs"),pin!("../Cargo.toml"),pin!("../Cargo.lock"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_live.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_bundle.rs"),
        pin!("../../../../rust/crates/rustaxa-storage/src/concrete_state/witness_binary.rs"),
        pin!("../../../../rust/crates/rustaxa-consensus/src/final_chain/native_session/vote_inputs.rs")],
        "executable":{"path":executable,"bytes":length,"sha256":executable_sha}}),
    )
}
/// Authenticate the fixed authority inputs and publish the exact manifest and
/// executable profile without inspecting or opening any database path.
pub fn preflight(report_output: &Path) -> Result<()> {
    let output = output_path(report_output)?;
    let plan = load_fixed_plan()?;
    publish_report(
        &output,
        &json!({"schema":1,"status":"authority_inputs_authenticated_no_database_open",
        "bounds":policy(),"source_profile":source_profile()?,
        "genesis_hash_hex":hex::encode(plan.genesis_hash),
        "head_header_sha256":hex::encode(plan.head_header_sha256),
        "head":identity_json(plan.head),"parent":identity_json(plan.parent),
        "delayed":plan.delayed.iter().copied().map(identity_json).collect::<Vec<_>>(),
        "tuples":plan.tuples.iter().map(|t|json!({"identity":identity_json(t.identity),
            "address_hex":hex::encode(t.address),"logical_key_hex":hex::encode(t.key.0)})).collect::<Vec<_>>(),
        "qualification":{"historical_totals_qualified":false,"production_routing_authorized":false}}),
    )
}
fn policy() -> Value {
    let limits = ConcreteWitnessLimits::default();
    json!({"authority_input_bytes":AUTHORITY_INPUT_CAP,"bundle_bytes":BUNDLE_CAP,
        "report_bytes":REPORT_CAP,"executable_bytes":268435456,"executable_hash_buffer_bytes":8192,"storage_resources":RESOURCES.iter().map(|r|
            json!({"resource":format!("{r:?}"),"limit":limits.limit(*r)})).collect::<Vec<_>>(),
        "exclusions":["authority admission allocations","engine memory","allocator overhead","OS I/O","RSS"]})
}
fn usage(usage: ConcreteWitnessUsage) -> Value {
    json!({"admitted_input_bytes":usage.admitted_input_bytes,
        "resources":RESOURCES.iter().map(|r|json!({"resource":format!("{r:?}"),
            "used":usage.used(*r)})).collect::<Vec<_>>()})
}
fn identity_json(identity: ConcreteStateIdentity) -> Value {
    json!({"period":identity.period.as_u64(),"state_root_hex":hex::encode(identity.state_root)})
}
/// Finite typed first-failure report. It retains exact tuple and last request,
/// but no engine error string, accepted prefix or inferred numeric zero.
pub fn failure_report(failure: &ConcreteDiagnosticFailure) -> Value {
    json!({"schema":1,"profile":"fixed_n4_native_total_jail","status":"stopped",
        "bounds":policy(),"actual":usage(failure.usage),"failure":{
            "error":format!("{:?}",failure.error),"phase":format!("{:?}",failure.phase),
            "index":failure.index,"identity":failure.identity.map(identity_json),
            "tuple":failure.tuple.map(|t|json!({"identity":identity_json(t.identity),
                "address_hex":hex::encode(t.address),"logical_key_hex":hex::encode(t.key.0)})),
            "last_read":failure.last_read.map(|r|json!({"operation":r.operation,
                "column":r.column,"request_hex":hex::encode(r.request),
                "outcome":format!("{:?}",r.outcome)}))},"totals":null,
        "qualification":{"all_96_authenticated":false,"historical_totals_qualified":false,
            "owner_ready":false,"production_routing_authorized":false}})
}
/// Canonical total bytes require membership,1..8 bytes and a nonzero first
/// byte. Jail may be authenticated nonmembership or the exact empty RLP list.
/// Decode all96 only after storage replay succeeds; return no partial totals.
pub fn decode_totals(logical: &[ConcreteWitnessLogical; 96]) -> Result<[u64; 48]> {
    let mut totals = [0; 48];
    for index in 0..48 {
        let total = match &logical[index] {
            ConcreteWitnessLogical::Member(bytes) => bytes.as_slice(),
            ConcreteWitnessLogical::NonMember => {
                anyhow::bail!("required total nonmembership at index {index}")
            }
        };
        ensure!(
            (1..=8).contains(&total.len()) && total[0] != 0,
            "noncanonical total at index {index}"
        );
        let jail = match &logical[index + 48] {
            ConcreteWitnessLogical::NonMember => None,
            ConcreteWitnessLogical::Member(bytes) if bytes.as_slice() == [0xc0] => {
                Some(bytes.as_slice())
            }
            _ => anyhow::bail!("nonempty or malformed jail at index {index}"),
        };
        totals[index] = rustaxa_consensus::native_session::vote_inputs::historical_total_votes(
            Some(total),
            jail,
        )?;
    }
    Ok(totals)
}
fn output_path(output: &Path) -> Result<PathBuf> {
    let parent = fs::canonicalize(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    admission::outside_db(&parent)?;
    let output = parent.join(output.file_name().context("output filename")?);
    ensure!(
        fs::symlink_metadata(&output).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "output exists or cannot be checked"
    );
    Ok(output)
}
fn publish_report(path: &Path, report: &Value) -> Result<()> {
    admission::publish_exclusive(path, &interval_history::encode_bounded(report, REPORT_CAP)?)
}
/// Fixed copy capture. Both output paths and all historical authority inputs
/// are checked before opening any DB. A typed storage failure publishes exactly
/// one bounded stop report and returns an error; it never retries or broadens.
pub fn capture(bundle_output: &Path, report_output: &Path) -> Result<()> {
    let paths = paths::validate(Path::new(COPY), bundle_output)?;
    let report_paths = paths::validate(Path::new(COPY), report_output)?;
    ensure!(paths.output != report_paths.output, "outputs overlap");
    let plan = load_fixed_plan()?;
    let source_profile = source_profile()?;
    match capture_diagnostic_run(
        &paths.application,
        &paths.state,
        &plan,
        ConcreteWitnessLimits::default(),
    ) {
        Err(failure) => {
            let mut report = failure_report(&failure);
            report["source_profile"] = source_profile;
            publish_report(&report_paths.output, &report)?;
            anyhow::bail!("fixed native capture stopped; see failure report")
        }
        Ok(captured) => {
            ensure!(captured.bundle.len() <= BUNDLE_CAP, "capture envelope cap");
            let report = json!({"schema":1,"profile":"fixed_n4_native_total_jail","status":"captured_requires_offline_replay","source_profile":source_profile,
                "bounds":policy(),"actual":usage(captured.usage),"bundle":{"path":paths.output,
                    "bytes":captured.bundle.len(),"sha256":admission::sha(&captured.bundle)},
                "qualification":{"all_96_authenticated":false,"historical_totals_qualified":false,
                    "owner_ready":false,"production_routing_authorized":false}});
            let report_bytes = interval_history::encode_bounded(&report, REPORT_CAP)?;
            admission::publish_exclusive(&paths.output, &captured.bundle)?;
            admission::publish_exclusive(&report_paths.output, &report_bytes)
        }
    }
}
/// Offline replay: the bundle supplies proof bytes only. The fixed independently
/// authenticated headers supply all roots/identities. No database is opened.
pub fn replay(bundle_input: &Path, report_output: &Path) -> Result<()> {
    let output = output_path(report_output)?;
    let input = fs::canonicalize(bundle_input)?;
    admission::outside_db(&input)?;
    ensure!(
        fs::metadata(&input)?.is_file() && input != output,
        "bundle input shape"
    );
    let plan = load_fixed_plan()?;
    let source_profile = source_profile()?;
    let bytes = admission::read_capped(fs::File::open(&input)?, BUNDLE_CAP)?;
    let verified = match replay_diagnostic_bundle(&bytes, &plan, ConcreteWitnessLimits::default()) {
        Ok(verified) => verified,
        Err(failure) => {
            let mut report = failure_report(&failure);
            report["source_profile"] = source_profile;
            publish_report(&output, &report)?;
            anyhow::bail!("fixed native replay stopped; see failure report")
        }
    };
    let totals = match decode_totals(&verified.logical) {
        Ok(totals) => totals,
        Err(error) => {
            publish_report(
                &output,
                &json!({"schema":1,"status":"native_precondition_rejected","source_profile":source_profile,
                "bounds":policy(),"actual":usage(verified.usage),"failure":{"phase":"NativeDecode",
                    "reason":error.to_string()},"totals":null,
                "qualification":{"all_96_authenticated":true,"historical_totals_qualified":false,
                    "owner_ready":false,"production_routing_authorized":false}}),
            )?;
            anyhow::bail!("native precondition rejected; see failure report")
        }
    };
    let report = json!({"schema":1,"profile":"fixed_n4_native_total_jail","status":"all_96_authenticated_native_totals","source_profile":source_profile,
        "bounds":policy(),"actual":usage(verified.usage),"bundle":{"path":input,
            "bytes":bytes.len(),"sha256":admission::sha(&bytes)},
        "totals":totals.iter().enumerate().map(|(i,total)|json!({"identity":identity_json(plan.delayed[i]),
            "total_votes":total})).collect::<Vec<_>>(),
        "qualification":{"all_96_authenticated":true,"historical_totals_qualified":true,
            "voter_weights_qualified":false,"reward_cache_equal":false,"owner_ready":false,
            "producer_verified":false,"independent_public_finality":false,"production_routing_authorized":false}});
    publish_report(&output, &report)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn logical() -> [ConcreteWitnessLogical; 96] {
        std::array::from_fn(|i| {
            if i < 48 {
                ConcreteWitnessLogical::Member(vec![1])
            } else {
                ConcreteWitnessLogical::NonMember
            }
        })
    }
    #[test]
    fn exact_native_manifest_uses_reference_logical_keys_and_order() {
        let ids = std::array::from_fn(|i| ConcreteStateIdentity {
            period: (FIRST + i as u64).into(),
            state_root: [i as u8; 32],
        });
        let tuples = manifest(ids);
        for i in 0..48 {
            assert_eq!(tuples[i].identity, ids[i]);
            assert_eq!(tuples[i + 48].identity, ids[i]);
            assert_eq!(tuples[i].key.0, keccak(&[4]));
            assert_eq!(tuples[i].address[19], 0xfe);
            assert_eq!(tuples[i + 48].address[19], 0xee);
            assert_eq!(tuples[i + 48].key.0[31], 2);
        }
        let mut wrong = [0; 32];
        wrong[31] = 4;
        assert_ne!(tuples[0].key.0, wrong);
    }
    #[test]
    fn canonical_totals_and_empty_jails_reuse_native_kernel() {
        let mut values = logical();
        values[0] = ConcreteWitnessLogical::Member(vec![1, 0]);
        values[48] = ConcreteWitnessLogical::Member(vec![0xc0]);
        assert_eq!(decode_totals(&values).unwrap()[0], 256);
        assert_eq!(decode_totals(&values).unwrap()[47], 1);
    }
    #[test]
    fn missing_zero_noncanonical_totals_and_nonempty_jails_never_qualify() {
        for bytes in [vec![], vec![0], vec![0, 1], vec![1; 9]] {
            let mut values = logical();
            values[47] = ConcreteWitnessLogical::Member(bytes);
            assert!(decode_totals(&values).is_err());
        }
        let mut values = logical();
        values[0] = ConcreteWitnessLogical::NonMember;
        assert!(decode_totals(&values).is_err());
        for bytes in [vec![], vec![0x80], vec![0xc1, 0x80], vec![0xc0, 0]] {
            let mut values = logical();
            values[95] = ConcreteWitnessLogical::Member(bytes);
            assert!(decode_totals(&values).is_err());
        }
    }
    #[test]
    fn fixed_admitted_headers_reauthenticate_offline_and_reject_pin_edits() {
        let plan = load_fixed_plan().unwrap();
        assert_eq!(plan.delayed[0].period.as_u64(), FIRST);
        assert_eq!(plan.delayed[47].period.as_u64(), FIRST + 47);
        let evidence = delayed_headers::Evidence::load_fixed().unwrap();
        let raw = fs::read(DELAYED_PATH).unwrap();
        let mut value = admission::parse_unique(&raw).unwrap();
        value["source_pins"]["current_compiled"] = json!([]);
        assert!(
            delayed_headers::replay_fixed_historical(
                &evidence,
                &serde_json::to_vec(&value).unwrap()
            )
            .is_err()
        );
        assert!(delayed_headers::replay_fixed_historical(&evidence, b"{}").is_err());
    }
}
