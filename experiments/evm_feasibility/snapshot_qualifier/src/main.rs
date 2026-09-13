//! Read-only qualification helper for paired Taraxa application/state snapshots.
//!
//! The helper refuses the supplied evidence path, opens every column family
//! through RocksDB's read-only API, and emits compact JSON evidence. It never
//! enables create, repair, migration, compaction, or write paths.

use rocksdb::{ColumnFamilyDescriptor, DB, Options};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tiny_keccak::{Hasher, Keccak};

const SUPPLIED_EVIDENCE: &str = "/tmp/snapshot-litenode";
const APP_UINT64_COLUMNS: &[&str] = &[
    "period_data",
    "dag_blocks_level",
    "proposal_period_levels_map",
    "sortition_params_change",
    "block_rewards_stats",
    "pillar_block",
    "final_chain_receipt_by_period",
    "period_lambda",
];

#[derive(Serialize)]
struct Report {
    schema: u32,
    tool_version: &'static str,
    tool_source_sha256: String,
    linked_rocksdb: &'static str,
    input: String,
    rocksdb_open: RocksDbOpen,
    application: ApplicationReport,
    concrete_state: StateReport,
    pairing: PairingReport,
    fixtures: FixtureReport,
    gaps: Vec<String>,
}

#[derive(Serialize)]
struct RocksDbOpen {
    mode: &'static str,
    error_if_log_file_exists: bool,
    create_if_missing: bool,
    create_missing_column_families: bool,
    application_column_families: Vec<ColumnSummary>,
    state_column_families: Vec<ColumnSummary>,
}

#[derive(Serialize)]
struct ColumnSummary {
    name: String,
    first_key_hex: Option<String>,
    last_key_hex: Option<String>,
}

#[derive(Serialize)]
struct ApplicationReport {
    genesis_hash_hex: Option<String>,
    finalized_head: u64,
    finalized_header_state_root_hex: String,
    finalized_header_sha256: String,
    finalized_hash_column_hex: Option<String>,
    block_range: NumericRange,
    period_data_range: NumericRange,
    receipt_range: NumericRange,
    transaction_location_range: ScanRange,
    replay_window: ReplayWindow,
}

#[derive(Serialize)]
struct StateReport {
    descriptor_period: u64,
    descriptor_root_hex: String,
    descriptor_sha256: String,
    root_node_present: bool,
    previous_period: Option<u64>,
    previous_header_state_root_hex: Option<String>,
    previous_root_node_present: Option<bool>,
    main_values: VersionedRange,
    storage_values: VersionedRange,
    code_rows: u64,
    main_nodes: u64,
    storage_nodes: u64,
    rustaxa_provenance_present: bool,
    rustaxa_pending_present: bool,
    rustaxa_catalog_present: bool,
}

#[derive(Serialize)]
struct PairingReport {
    periods_equal: bool,
    roots_equal: bool,
    qualified_common_period: Option<u64>,
}

#[derive(Serialize, Default)]
struct NumericRange {
    first: Option<u64>,
    last: Option<u64>,
}

#[derive(Serialize, Default)]
struct ScanRange {
    rows: u64,
    first_period: Option<u64>,
    last_period: Option<u64>,
    malformed_rows: u64,
}

#[derive(Serialize, Default)]
struct VersionedRange {
    rows: u64,
    tombstones: u64,
    first_period: Option<u64>,
    last_period: Option<u64>,
    malformed_keys: u64,
}

#[derive(Serialize, Default)]
struct FixtureReport {
    dpos_account: Option<AccountFixture>,
    dpos_account_prior: Option<AccountFixture>,
    dpos_code: Option<CodeFixture>,
    dpos_code_size_matches_account: Option<bool>,
    dpos_native_rows: Vec<NativeStorageFixture>,
    dpos_native_rows_prior: Vec<NativeStorageFixture>,
    first_main_value: Option<PhysicalFixture>,
    first_storage_value: Option<PhysicalFixture>,
    first_code: Option<CodeFixture>,
}

#[derive(Serialize)]
struct AccountFixture {
    address_hex: String,
    target_period: u64,
    physical_key_prefix_hex: String,
    selected_period: u64,
    physical_rlp_hex: String,
    nonce_hex: String,
    balance_hex: String,
    storage_root_hex: Option<String>,
    code_hash_hex: Option<String>,
    code_size: u64,
    storage_root_node_present: Option<bool>,
}

#[derive(Serialize)]
struct PhysicalFixture {
    physical_key_hex: String,
    period: u64,
    value_hex: String,
}

#[derive(Serialize)]
struct CodeFixture {
    code_hash_hex: String,
    keccak_matches_key: bool,
    byte_length: usize,
    sha256: String,
    bytes_hex: String,
}

#[derive(Serialize)]
struct NativeStorageFixture {
    name: &'static str,
    logical_key_hex: String,
    physical_key_prefix_hex: String,
    selected_period: Option<u64>,
    value_hex: Option<String>,
    tombstone: bool,
}

#[derive(Serialize)]
struct ReplayWindow {
    period: u64,
    prior_period: Option<u64>,
    prior_state_root_hex: Option<String>,
    period_data_bytes: usize,
    period_data_sha256: String,
    receipts_bytes: usize,
    receipts_sha256: String,
    rewards_stats_bytes: Option<usize>,
    rewards_stats_sha256: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or("usage: rustaxa-snapshot-qualifier SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    let output = PathBuf::from(
        args.next()
            .ok_or("usage: rustaxa-snapshot-qualifier SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    if args.next().is_some() {
        return Err("usage: rustaxa-snapshot-qualifier SNAPSHOT_COPY OUTPUT_JSON".into());
    }
    let canonical = fs::canonicalize(&input)?;
    if canonical == fs::canonicalize(SUPPLIED_EVIDENCE)? || canonical.starts_with(SUPPLIED_EVIDENCE)
    {
        return Err(
            "refusing to open the supplied evidence snapshot; pass an independent copy".into(),
        );
    }

    let app_path = canonical.join("db/db");
    let state_path = canonical.join("db/state_db");
    let (app, app_columns) = open_read_only(&app_path, true)?;
    let (state, state_columns) = open_read_only(&state_path, false)?;

    let app_cf_summaries = summarize_columns(&app, &app_columns)?;
    let state_cf_summaries = summarize_columns(&state, &state_columns)?;

    let genesis_hash = get_cf(&app, "genesis", &0_i32.to_le_bytes())?;
    let head_raw = get_cf(&app, "final_chain_meta", &1_u32.to_le_bytes())?
        .ok_or("missing final-chain head metadata")?;
    let finalized_head = decode_le_u64(&head_raw, "final-chain head")?;
    let header_raw = get_cf(
        &app,
        "final_chain_blk_by_number",
        &finalized_head.to_le_bytes(),
    )?
    .ok_or("missing finalized-head header")?;
    let header_rlp = rlp::Rlp::new(&header_raw);
    if header_rlp.item_count()? != 7 {
        return Err("finalized-head stored header is not seven-field RLP".into());
    }
    let header_root = exact_32(header_rlp.at(1)?.data()?, "header state root")?;
    let hash_column = get_cf(
        &app,
        "final_chain_blk_hash_by_number",
        &finalized_head.to_le_bytes(),
    )?;
    let period_data = get_cf(&app, "period_data", &finalized_head.to_le_bytes())?
        .ok_or("missing finalized-head period data")?;
    let receipts = get_cf(
        &app,
        "final_chain_receipt_by_period",
        &finalized_head.to_le_bytes(),
    )?
    .ok_or("missing finalized-head receipts")?;
    let rewards_stats = get_cf(&app, "block_rewards_stats", &finalized_head.to_le_bytes())?;

    let descriptor_raw = state
        .get(b"last_committed_descriptor")?
        .ok_or("missing concrete state descriptor")?;
    let descriptor_rlp = rlp::Rlp::new(&descriptor_raw);
    if descriptor_rlp.item_count()? != 2 {
        return Err("concrete descriptor is not two-field RLP".into());
    }
    let descriptor_period = descriptor_rlp.val_at::<u64>(0)?;
    let descriptor_root = exact_32(descriptor_rlp.at(1)?.data()?, "descriptor state root")?;

    let root_node_present = get_cf(&state, "2", &descriptor_root)?.is_some();
    let previous_period = descriptor_period.checked_sub(1);
    let previous_header_root = previous_period
        .map(
            |period| -> Result<Option<[u8; 32]>, Box<dyn std::error::Error>> {
                let Some(raw) = get_cf(&app, "final_chain_blk_by_number", &period.to_le_bytes())?
                else {
                    return Ok(None);
                };
                let stored = rlp::Rlp::new(&raw);
                if stored.item_count()? != 7 {
                    return Err("previous stored header is not seven-field RLP".into());
                }
                Ok(Some(exact_32(
                    stored.at(1)?.data()?,
                    "previous header state root",
                )?))
            },
        )
        .transpose()?
        .flatten();
    let previous_root_node_present = previous_header_root
        .map(|root| get_cf(&state, "2", &root).map(|value| value.is_some()))
        .transpose()?;
    let main_values = scan_versioned(&state, "3")?;
    let storage_values = scan_versioned(&state, "5")?;
    let code_rows = count_rows(&state, "1")?;
    let main_nodes = count_rows(&state, "2")?;
    let storage_nodes = count_rows(&state, "4")?;

    let dpos_address = {
        let mut address = [0_u8; 20];
        address[19] = 0xfe;
        address
    };
    let dpos_account = account_at(&state, descriptor_period, dpos_address)?;
    let dpos_account_prior = previous_period
        .map(|period| account_at(&state, period, dpos_address))
        .transpose()?
        .flatten();
    let dpos_code = dpos_account
        .as_ref()
        .and_then(|account| account.code_hash_hex.as_deref())
        .map(
            |hash| -> Result<Option<CodeFixture>, Box<dyn std::error::Error>> {
                let decoded = hex::decode(hash)?;
                let hash: [u8; 32] = decoded.try_into().map_err(|_| "invalid DPoS code hash")?;
                code_by_hash(&state, hash)
            },
        )
        .transpose()?
        .flatten();
    let dpos_native_rows = [("minted_tokens", 6_u8), ("total_supply", 7), ("yield", 8)]
        .into_iter()
        .map(|(name, field)| {
            native_storage_at(&state, descriptor_period, dpos_address, name, &[field])
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dpos_native_rows_prior = previous_period
        .map(|period| {
            [("minted_tokens", 6_u8), ("total_supply", 7), ("yield", 8)]
                .into_iter()
                .map(|(name, field)| {
                    native_storage_at(&state, period, dpos_address, name, &[field])
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let dpos_code_size_matches_account = dpos_account.as_ref().and_then(|account| {
        dpos_code
            .as_ref()
            .map(|code| code.byte_length == account.code_size as usize)
    });
    let first_main_value = first_nonempty_versioned(&state, "3")?;
    let first_storage_value = first_nonempty_versioned(&state, "5")?;
    let first_code = first_code(&state)?;

    let periods_equal = finalized_head == descriptor_period;
    let roots_equal = header_root == descriptor_root;
    let qualified_common_period =
        (periods_equal && roots_equal && root_node_present).then_some(finalized_head);
    let mut gaps = Vec::new();
    if !periods_equal {
        gaps.push("application and concrete-state periods differ".to_owned());
    }
    if !roots_equal {
        gaps.push("application header and concrete descriptor roots differ".to_owned());
    }
    if !root_node_present {
        gaps.push("concrete descriptor root node is absent".to_owned());
    }
    gaps.push("producer binary revision and exact paired checkpoint timing are not encoded in these databases".to_owned());
    gaps.push(
        "a light snapshot cannot establish full historical or activation-boundary replay coverage"
            .to_owned(),
    );

    let report = Report {
        schema: 1,
        tool_version: env!("CARGO_PKG_VERSION"),
        tool_source_sha256: sha256_hex(include_bytes!("main.rs")),
        linked_rocksdb: "rocksdb crate 0.24.0 / librocksdb 10.4.2",
        input: canonical.display().to_string(),
        rocksdb_open: RocksDbOpen {
            mode: "DB::open_cf_descriptors_read_only",
            error_if_log_file_exists: false,
            create_if_missing: false,
            create_missing_column_families: false,
            application_column_families: app_cf_summaries,
            state_column_families: state_cf_summaries,
        },
        application: ApplicationReport {
            genesis_hash_hex: genesis_hash.map(hex::encode),
            finalized_head,
            finalized_header_state_root_hex: hex::encode(header_root),
            finalized_header_sha256: sha256_hex(&header_raw),
            finalized_hash_column_hex: hash_column.map(hex::encode),
            block_range: numeric_range(&app, "final_chain_blk_by_number")?,
            period_data_range: numeric_range(&app, "period_data")?,
            receipt_range: numeric_range(&app, "final_chain_receipt_by_period")?,
            transaction_location_range: scan_transaction_locations(&app)?,
            replay_window: ReplayWindow {
                period: finalized_head,
                prior_period: previous_period,
                prior_state_root_hex: previous_header_root.map(hex::encode),
                period_data_bytes: period_data.len(),
                period_data_sha256: sha256_hex(&period_data),
                receipts_bytes: receipts.len(),
                receipts_sha256: sha256_hex(&receipts),
                rewards_stats_bytes: rewards_stats.as_ref().map(Vec::len),
                rewards_stats_sha256: rewards_stats.as_ref().map(|bytes| sha256_hex(bytes)),
            },
        },
        concrete_state: StateReport {
            descriptor_period,
            descriptor_root_hex: hex::encode(descriptor_root),
            descriptor_sha256: sha256_hex(&descriptor_raw),
            root_node_present,
            previous_period,
            previous_header_state_root_hex: previous_header_root.map(hex::encode),
            previous_root_node_present,
            main_values,
            storage_values,
            code_rows,
            main_nodes,
            storage_nodes,
            rustaxa_provenance_present: state
                .get(b"rustaxa_concrete_state_provenance_v1")?
                .is_some(),
            rustaxa_pending_present: state
                .get(b"rustaxa_concrete_execution_pending_v1")?
                .is_some(),
            rustaxa_catalog_present: state.get(b"rustaxa_concrete_storage_catalog_v1")?.is_some(),
        },
        pairing: PairingReport {
            periods_equal,
            roots_equal,
            qualified_common_period,
        },
        fixtures: FixtureReport {
            dpos_account,
            dpos_account_prior,
            dpos_code,
            dpos_code_size_matches_account,
            dpos_native_rows,
            dpos_native_rows_prior,
            first_main_value,
            first_storage_value,
            first_code,
        },
        gaps,
    };
    fs::write(output, serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}

fn open_read_only(
    path: &Path,
    application: bool,
) -> Result<(DB, Vec<String>), Box<dyn std::error::Error>> {
    let mut db_options = Options::default();
    db_options.create_if_missing(false);
    db_options.create_missing_column_families(false);
    db_options.set_max_open_files(128);
    let columns = DB::list_cf(&db_options, path)?;
    let descriptors = columns.iter().map(|name| {
        let mut options = Options::default();
        if application && APP_UINT64_COLUMNS.contains(&name.as_str()) {
            options.set_comparator(
                "taraxa.UintComparator",
                Box::new(|a: &[u8], b: &[u8]| match (a.try_into(), b.try_into()) {
                    (Ok(a), Ok(b)) => u64::from_le_bytes(a).cmp(&u64::from_le_bytes(b)),
                    _ => a.cmp(b),
                }),
            );
        }
        ColumnFamilyDescriptor::new(name, options)
    });
    let db = DB::open_cf_descriptors_read_only(&db_options, path, descriptors, false)?;
    Ok((db, columns))
}

fn summarize_columns(
    db: &DB,
    columns: &[String],
) -> Result<Vec<ColumnSummary>, Box<dyn std::error::Error>> {
    columns
        .iter()
        .map(|name| {
            let cf = db.cf_handle(name).ok_or("missing opened column family")?;
            let mut it = db.raw_iterator_cf(&cf);
            it.seek_to_first();
            let first_key_hex = it.key().map(hex::encode);
            it.status()?;
            it.seek_to_last();
            let last_key_hex = it.key().map(hex::encode);
            it.status()?;
            Ok(ColumnSummary {
                name: name.clone(),
                first_key_hex,
                last_key_hex,
            })
        })
        .collect()
}

fn get_cf(db: &DB, name: &str, key: &[u8]) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let cf = db.cf_handle(name).ok_or("missing column family")?;
    Ok(db.get_cf(&cf, key)?)
}

fn numeric_range(db: &DB, name: &str) -> Result<NumericRange, Box<dyn std::error::Error>> {
    let cf = db.cf_handle(name).ok_or("missing numeric column family")?;
    let mut first = None;
    let mut last = None;
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, _) = entry?;
        let value = decode_le_u64(&key, name)?;
        first = Some(first.map_or(value, |old: u64| old.min(value)));
        last = Some(last.map_or(value, |old: u64| old.max(value)));
    }
    Ok(NumericRange { first, last })
}

fn scan_versioned(db: &DB, name: &str) -> Result<VersionedRange, Box<dyn std::error::Error>> {
    let cf = db
        .cf_handle(name)
        .ok_or("missing versioned column family")?;
    let mut report = VersionedRange::default();
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = entry?;
        report.rows += 1;
        if value.is_empty() {
            report.tombstones += 1;
        }
        if key.len() != 40 {
            report.malformed_keys += 1;
            continue;
        }
        let period = u64::from_be_bytes(key[32..].try_into()?);
        report.first_period = Some(report.first_period.map_or(period, |old| old.min(period)));
        report.last_period = Some(report.last_period.map_or(period, |old| old.max(period)));
    }
    Ok(report)
}

fn count_rows(db: &DB, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let cf = db.cf_handle(name).ok_or("missing column family")?;
    let mut rows = 0;
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        entry?;
        rows += 1;
    }
    Ok(rows)
}

fn scan_transaction_locations(db: &DB) -> Result<ScanRange, Box<dyn std::error::Error>> {
    let cf = db
        .cf_handle("trx_period")
        .ok_or("missing trx_period column family")?;
    let mut report = ScanRange::default();
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (_, value) = entry?;
        report.rows += 1;
        match rlp::Rlp::new(&value).val_at::<u64>(0) {
            Ok(period) => {
                report.first_period =
                    Some(report.first_period.map_or(period, |old| old.min(period)));
                report.last_period = Some(report.last_period.map_or(period, |old| old.max(period)));
            }
            Err(_) => report.malformed_rows += 1,
        }
    }
    Ok(report)
}

fn first_nonempty_versioned(
    db: &DB,
    name: &str,
) -> Result<Option<PhysicalFixture>, Box<dyn std::error::Error>> {
    let cf = db
        .cf_handle(name)
        .ok_or("missing versioned column family")?;
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = entry?;
        if key.len() == 40 && !value.is_empty() && value.len() <= 1024 {
            return Ok(Some(PhysicalFixture {
                physical_key_hex: hex::encode(&key),
                period: u64::from_be_bytes(key[32..].try_into()?),
                value_hex: hex::encode(value),
            }));
        }
    }
    Ok(None)
}

fn account_at(
    db: &DB,
    period: u64,
    address: [u8; 20],
) -> Result<Option<AccountFixture>, Box<dyn std::error::Error>> {
    let prefix = keccak256(&address);
    let mut target = [0_u8; 40];
    target[..32].copy_from_slice(&prefix);
    target[32..].copy_from_slice(&period.to_be_bytes());
    let cf = db
        .cf_handle("3")
        .ok_or("missing main-value column family")?;
    let mut it = db.raw_iterator_cf(&cf);
    it.seek_for_prev(target);
    let Some((key, value)) = it.item() else {
        it.status()?;
        return Ok(None);
    };
    if key.len() != 40 || key[..32] != prefix || value.is_empty() {
        return Ok(None);
    }
    let selected_period = u64::from_be_bytes(key[32..].try_into()?);
    let rlp = rlp::Rlp::new(value);
    if rlp.item_count()? != 5 {
        return Err("account fixture is not five-field RLP".into());
    }
    let optional_hash = |position| -> Result<Option<String>, Box<dyn std::error::Error>> {
        let bytes = rlp.at(position)?.data()?;
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(hex::encode(exact_32(bytes, "account hash")?)))
        }
    };
    Ok(Some(AccountFixture {
        address_hex: hex::encode(address),
        target_period: period,
        physical_key_prefix_hex: hex::encode(prefix),
        selected_period,
        physical_rlp_hex: hex::encode(value),
        nonce_hex: hex::encode(rlp.at(0)?.data()?),
        balance_hex: hex::encode(rlp.at(1)?.data()?),
        storage_root_hex: optional_hash(2)?,
        code_hash_hex: optional_hash(3)?,
        code_size: rlp.val_at(4)?,
        storage_root_node_present: optional_hash(2)?
            .map(|root| {
                let decoded = hex::decode(root)?;
                get_cf(db, "4", &decoded).map(|value| value.is_some())
            })
            .transpose()?,
    }))
}

fn first_code(db: &DB) -> Result<Option<CodeFixture>, Box<dyn std::error::Error>> {
    let cf = db.cf_handle("1").ok_or("missing code column family")?;
    for entry in db.iterator_cf(&cf, rocksdb::IteratorMode::Start) {
        let (key, value) = entry?;
        if key.len() == 32 && value.len() <= 4096 {
            return Ok(Some(CodeFixture {
                code_hash_hex: hex::encode(&key),
                keccak_matches_key: keccak256(&value).as_slice() == key.as_ref(),
                byte_length: value.len(),
                sha256: sha256_hex(&value),
                bytes_hex: hex::encode(value),
            }));
        }
    }
    Ok(None)
}

fn code_by_hash(
    db: &DB,
    code_hash: [u8; 32],
) -> Result<Option<CodeFixture>, Box<dyn std::error::Error>> {
    let Some(value) = get_cf(db, "1", &code_hash)? else {
        return Ok(None);
    };
    Ok(Some(CodeFixture {
        code_hash_hex: hex::encode(code_hash),
        keccak_matches_key: keccak256(&value) == code_hash,
        byte_length: value.len(),
        sha256: sha256_hex(&value),
        bytes_hex: hex::encode(value),
    }))
}

fn native_storage_at(
    db: &DB,
    period: u64,
    address: [u8; 20],
    name: &'static str,
    field: &[u8],
) -> Result<NativeStorageFixture, Box<dyn std::error::Error>> {
    let logical_key = keccak256(field);
    let slot_path = keccak256(&logical_key);
    let mut physical_input = Vec::with_capacity(52);
    physical_input.extend_from_slice(&address);
    physical_input.extend_from_slice(&slot_path);
    let physical_prefix = keccak256(&physical_input);
    let mut target = [0_u8; 40];
    target[..32].copy_from_slice(&physical_prefix);
    target[32..].copy_from_slice(&period.to_be_bytes());
    let cf = db
        .cf_handle("5")
        .ok_or("missing storage-value column family")?;
    let mut it = db.raw_iterator_cf(&cf);
    it.seek_for_prev(target);
    let Some((key, value)) = it.item() else {
        it.status()?;
        return Ok(NativeStorageFixture {
            name,
            logical_key_hex: hex::encode(logical_key),
            physical_key_prefix_hex: hex::encode(physical_prefix),
            selected_period: None,
            value_hex: None,
            tombstone: false,
        });
    };
    if key.len() != 40 || key[..32] != physical_prefix {
        return Ok(NativeStorageFixture {
            name,
            logical_key_hex: hex::encode(logical_key),
            physical_key_prefix_hex: hex::encode(physical_prefix),
            selected_period: None,
            value_hex: None,
            tombstone: false,
        });
    }
    Ok(NativeStorageFixture {
        name,
        logical_key_hex: hex::encode(logical_key),
        physical_key_prefix_hex: hex::encode(physical_prefix),
        selected_period: Some(u64::from_be_bytes(key[32..].try_into()?)),
        value_hex: (!value.is_empty()).then(|| hex::encode(value)),
        tombstone: value.is_empty(),
    })
}

fn decode_le_u64(bytes: &[u8], label: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let exact: [u8; 8] = bytes
        .try_into()
        .map_err(|_| format!("{label} is not eight bytes"))?;
    Ok(u64::from_le_bytes(exact))
}

fn exact_32(bytes: &[u8], label: &str) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    bytes
        .try_into()
        .map_err(|_| format!("{label} is not 32 bytes").into())
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut out);
    out
}
