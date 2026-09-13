//! Bounded, read-only inventory of one candidate FinalChain replay period.
//!
//! The tool refuses the supplied snapshot, opens only an independent copy, and
//! uses the Rust storage repositories for period, transaction, receipt, header,
//! and metadata point reads. It never scans a retained range or executes a
//! transaction. The JSON report separates a complete ordered input/output
//! bundle from the additional configuration and state-closure gates required
//! before replay.

use std::collections::BTreeSet;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use keccak_hasher::KeccakHasher;
use rlp::{Rlp, RlpStream};
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
use rustaxa_storage::{
    Column, ConcreteStateReader, FinalChainRepository, MetadataRepository, PeriodRepository,
    TransactionRepository,
};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_state::ConcreteStateIdentity;
use rustaxa_types::{
    FinalChainBlockNumber, LegacyTransactionEnvelope, StoredFinalChainBlockHeader,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tiny_keccak::{Hasher, Keccak};
use triehash::ordered_trie_root;

const SUPPLIED_EVIDENCE: &str = "/tmp/snapshot-litenode";
const TARGET_PERIOD: u64 = 25_706_949;
const DOCUMENTED_MAINNET_GENESIS: &str =
    "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377";
const REQUIRED_APPLICATION_COLUMNS: &[&str] = &[
    "period_data",
    "genesis",
    "trx_period",
    "final_chain_meta",
    "final_chain_blk_by_number",
    "final_chain_blk_hash_by_number",
    "final_chain_receipt_by_trx_hash",
    "sortition_params_change",
    "block_rewards_stats",
    "system_transaction",
    "period_system_transactions",
    "final_chain_receipt_by_period",
    "period_lambda",
    "rounds_count_dynamic_lambda",
];
type Database = DBWithThreadMode<MultiThreaded>;

#[derive(Serialize)]
struct Report {
    schema: u32,
    tool_package_version: &'static str,
    tool_source_sha256: String,
    linked_rocksdb: &'static str,
    input_copy: String,
    rocksdb_open: RocksDbOpen,
    period: u64,
    application_head: u64,
    period_data: RawFact,
    period_data_children: Vec<IndexedRawFact>,
    ordered_inputs: OrderedInputs,
    receipts: ReceiptReport,
    header: HeaderReport,
    prior_state: PriorStateReport,
    configuration: ConfigurationReport,
    qualification: Qualification,
    gaps: Vec<&'static str>,
}

#[derive(Serialize)]
struct RocksDbOpen {
    mode: &'static str,
    create_if_missing: bool,
    create_missing_column_families: bool,
    application_column_family_count: usize,
    state_column_family_count: usize,
    missing_current_rust_application_columns: Vec<&'static str>,
}

#[derive(Clone, Serialize)]
struct RawFact {
    present: bool,
    bytes: usize,
    sha256: Option<String>,
}

#[derive(Serialize)]
struct IndexedRawFact {
    index: usize,
    bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct OrderedInputs {
    regular_count: usize,
    system_count: usize,
    total_count: usize,
    system_hash_list: RawFact,
    system_hash_references_complete: bool,
    storage_positions_complete: bool,
    transaction_root_hex: String,
    transaction_root_matches_header: bool,
    concrete_transaction_bundle_keccak_hex: String,
    observed_chain_ids: Vec<u64>,
    all_regular_signatures_valid: bool,
    entries: Vec<InputEntry>,
}

#[derive(Serialize)]
struct InputEntry {
    position: usize,
    kind: &'static str,
    transaction_hash_hex: String,
    rlp_bytes: usize,
    rlp_sha256: String,
    chain_id: u64,
    signature_valid: bool,
    location_present: bool,
    location_matches_order: bool,
    receipt_by_hash_present: bool,
    receipt_by_hash_matches_period: bool,
}

#[derive(Serialize)]
struct ReceiptReport {
    row: RawFact,
    count: usize,
    count_matches_inputs: bool,
    ordered_root_hex: String,
    root_matches_header: bool,
    every_hash_index_present: bool,
    every_hash_index_matches_period: bool,
    entries: Vec<IndexedRawFact>,
}

#[derive(Serialize)]
struct HeaderReport {
    row: RawFact,
    block_hash_hex: String,
    block_hash_recomputed: bool,
    parent_hash_hex: String,
    parent_matches_prior_block_hash: bool,
    state_root_hex: String,
    transactions_root_hex: String,
    receipts_root_hex: String,
    gas_used: u64,
    total_reward_hex: String,
}

#[derive(Serialize)]
struct PriorStateReport {
    current_descriptor_period: u64,
    current_descriptor_root_hex: String,
    current_descriptor_matches_head_header: bool,
    current_root_node_present: bool,
    prior_period: u64,
    prior_header: RawFact,
    prior_root_hex: String,
    prior_root_node_present: bool,
    historical_reader_constructor_accepted: bool,
    prior_descriptor_available: bool,
    complete_trie_closure_verified: bool,
}

#[derive(Serialize)]
struct ConfigurationReport {
    database_genesis_hash_hex: Option<String>,
    documented_mainnet_genesis_hash_hex: &'static str,
    documented_mainnet_genesis_matches: bool,
    period_lambda_at_or_before: Option<u32>,
    dynamic_lambda_round_count: u32,
    sortition_parameters: RawFact,
    block_rewards_stats: RawFact,
    exact_static_node_config_qualified: bool,
    previous_certificate_vote_payloads_and_weights_reconstructed: bool,
}

#[derive(Serialize)]
struct Qualification {
    ordered_inputs_complete: bool,
    receipts_complete: bool,
    header_present_and_paired: bool,
    prior_descriptor_available: bool,
    bounded_head_bundle_complete: bool,
    future_replay_ready: bool,
}

#[derive(Clone, Copy)]
struct TransactionLocation {
    period: u64,
    position: u32,
    is_system: bool,
}

fn main() -> Result<()> {
    let (input, output) = validated_paths()?;
    let app_path = canonical_child(&input, "db/db")?;
    let state_path = canonical_child(&input, "db/state_db")?;

    let (application, app_columns) = open_application_read_only(&app_path)?;
    let application = Arc::new(application);
    let periods = PeriodRepository::new(application.clone());
    let transactions = TransactionRepository::new(application.clone());
    let final_chain = FinalChainRepository::new(application.clone());
    let metadata = MetadataRepository::new(application.clone());

    let head_bytes = final_chain
        .meta_value(1)?
        .context("missing FinalChain head metadata")?;
    let application_head = exact_le_u64(&head_bytes, "FinalChain head")?;
    ensure!(
        application_head == TARGET_PERIOD,
        "expected head period {TARGET_PERIOD}, observed {application_head}"
    );

    let period_data = periods.data_raw(TARGET_PERIOD)?;
    ensure!(!period_data.is_empty(), "head period data is missing");
    let period_rlp = Rlp::new(&period_data);
    let child_count = period_rlp.item_count().context("head period data RLP")?;
    ensure!(
        matches!(child_count, 4 | 5),
        "head period data has unsupported {child_count}-field shape"
    );
    let period_data_children = (0..child_count)
        .map(|index| {
            let bytes = period_rlp.at(index)?.as_raw();
            Ok(IndexedRawFact {
                index,
                bytes: bytes.len(),
                sha256: sha256_hex(bytes),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let header_raw = final_chain
        .block_header_raw(TARGET_PERIOD)?
        .context("missing head FinalChain header")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header_raw))
        .context("decode head FinalChain header")?;
    let block_hash = exact_32(
        &final_chain
            .block_hash_by_number(TARGET_PERIOD)?
            .context("missing head FinalChain block hash")?,
        "head FinalChain block hash",
    )?;

    let prior_period = TARGET_PERIOD - 1;
    let prior_header_raw = final_chain
        .block_header_raw(prior_period)?
        .context("missing prior FinalChain header")?;
    let prior_header =
        StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&prior_header_raw))
            .context("decode prior FinalChain header")?;
    let prior_block_hash = exact_32(
        &final_chain
            .block_hash_by_number(prior_period)?
            .context("missing prior FinalChain block hash")?,
        "prior FinalChain block hash",
    )?;

    let regular_count = usize::try_from(transactions.count(TARGET_PERIOD)?)?;
    let regular_list = period_rlp.at(3).context("head transaction list")?;
    ensure!(
        regular_list.item_count()? == regular_count,
        "Rust transaction count disagrees with the period transaction list"
    );

    let mut entries = Vec::new();
    let mut ordered_rlps = Vec::new();
    let mut observed_chain_ids = BTreeSet::new();
    for position in 0..regular_count {
        let stored = transactions
            .by_period_position_rlp(TARGET_PERIOD, u32::try_from(position)?)?
            .with_context(|| format!("missing regular transaction at position {position}"))?;
        ensure!(
            stored == regular_list.at(position)?.as_raw(),
            "transaction repository bytes differ at position {position}"
        );
        let envelope = LegacyTransactionEnvelope::decode(&stored)
            .with_context(|| format!("decode regular transaction at position {position}"))?;
        observed_chain_ids.insert(envelope.chain_id);
        entries.push(input_entry(
            position,
            "regular",
            &envelope,
            transactions.location_rlp(envelope.hash)?,
            false,
        )?);
        ordered_rlps.push(stored);
    }

    let system_hash_list = transactions.period_system_hashes_rlp(TARGET_PERIOD)?;
    let system_hashes = decode_system_hashes(&system_hash_list)?;
    let mut system_hash_references_complete = true;
    for (system_index, hash) in system_hashes.iter().copied().enumerate() {
        let Some(stored) = transactions.system_rlp(hash)? else {
            system_hash_references_complete = false;
            continue;
        };
        let envelope = LegacyTransactionEnvelope::decode_system(&stored)
            .with_context(|| format!("decode system transaction {hash:#x}"))?;
        ensure!(
            envelope.hash == hash,
            "system transaction payload hash differs from its persisted reference"
        );
        observed_chain_ids.insert(envelope.chain_id);
        let position = regular_count + system_index;
        entries.push(input_entry(
            position,
            "system",
            &envelope,
            transactions.location_rlp(envelope.hash)?,
            true,
        )?);
        ordered_rlps.push(stored);
    }

    let receipts_rlp = periods.receipt(TARGET_PERIOD)?;
    ensure!(!receipts_rlp.is_empty(), "head receipt row is missing");
    let receipts = Rlp::new(&receipts_rlp);
    let receipt_count = receipts.item_count().context("head receipt list")?;
    let receipt_rows = (0..receipt_count)
        .map(|index| Ok(receipts.at(index)?.as_raw().to_vec()))
        .collect::<Result<Vec<_>>>()?;
    let receipt_entries = receipt_rows
        .iter()
        .enumerate()
        .map(|(index, bytes)| IndexedRawFact {
            index,
            bytes: bytes.len(),
            sha256: sha256_hex(bytes),
        })
        .collect::<Vec<_>>();

    for entry in &mut entries {
        let hash = H256::from_slice(&hex::decode(&entry.transaction_hash_hex)?);
        let indexed = final_chain.receipt_by_trx_hash(hash)?;
        entry.receipt_by_hash_present = indexed.is_some();
        entry.receipt_by_hash_matches_period = indexed
            .as_ref()
            .zip(receipt_rows.get(entry.position))
            .is_some_and(|(indexed, period)| indexed == period);
    }

    let transaction_root = ordered_root(&ordered_rlps);
    let receipt_root = ordered_root(&receipt_rows);
    let storage_positions_complete = entries.iter().all(|entry| entry.location_matches_order);
    let every_hash_index_present = entries.iter().all(|entry| entry.receipt_by_hash_present);
    let every_hash_index_matches_period = entries
        .iter()
        .all(|entry| entry.receipt_by_hash_matches_period);
    let all_regular_signatures_valid = entries
        .iter()
        .filter(|entry| entry.kind == "regular")
        .all(|entry| entry.signature_valid);

    let genesis_hash = metadata.genesis_hash()?;
    let sortition_parameters = metadata.params_change_for_period_rlp(TARGET_PERIOD)?;
    let period_lambda = metadata.period_lambda(TARGET_PERIOD, true)?;
    let dynamic_lambda_round_count = metadata.rounds_count_dynamic_lambda()?;
    let rewards_stats = raw_cf(
        &application,
        Column::BlockRewardsStats.name(),
        &TARGET_PERIOD.to_le_bytes(),
    )?;

    let (state, state_columns) = open_state_read_only(&state_path)?;
    let descriptor_raw = state
        .get(b"last_committed_descriptor")?
        .context("missing concrete descriptor")?;
    let descriptor = decode_descriptor(&descriptor_raw)?;
    let current_root_node_present = raw_cf(&state, "2", &descriptor.state_root)?.is_some();
    let prior_root: [u8; 32] = prior_header.state_root.into();
    let prior_root_node_present = raw_cf(&state, "2", &prior_root)?.is_some();
    drop(state);
    let historical = ConcreteStateReader::open_historical_read_only(
        &state_path,
        descriptor,
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(prior_period),
            state_root: prior_root,
        },
    );
    let historical_reader_constructor_accepted = historical.is_ok();
    drop(historical);

    let transaction_root_matches_header =
        transaction_root == <[u8; 32]>::from(header.transactions_root);
    let receipt_root_matches_header = receipt_root == <[u8; 32]>::from(header.receipts_root);
    let receipt_count_matches = receipt_count == ordered_rlps.len();
    let current_descriptor_matches_head_header = descriptor.period.as_u64() == TARGET_PERIOD
        && descriptor.state_root == <[u8; 32]>::from(header.state_root);
    let prior_descriptor_available =
        historical_reader_constructor_accepted && prior_root_node_present;
    let ordered_inputs_complete = system_hash_references_complete
        && ordered_rlps.len() == regular_count + system_hashes.len()
        && storage_positions_complete
        && all_regular_signatures_valid
        && transaction_root_matches_header;
    let receipts_complete = receipt_count_matches && receipt_root_matches_header;
    let header_present_and_paired = header.parent_hash == H256::from(prior_block_hash)
        && current_descriptor_matches_head_header;
    let bounded_head_bundle_complete = ordered_inputs_complete
        && receipts_complete
        && header_present_and_paired
        && prior_descriptor_available;
    let mut gaps = vec![
        "This bounded lookup did not recover the exact static execution, fork, gas-limit, rewards, or native-kernel configuration used by the producer.",
        "PeriodData alone does not reconstruct the weighted previous-certificate vote payloads used by the current Rust FinalChain reward request.",
        "The prior root-node key is present and the historical reader constructor accepts the descriptor pair; neither fact authenticates a replay state path or proves complete prior-trie closure.",
        "Receipts and the final header are retained expected outputs; no execution or root reproduction was run.",
        "The block hash is the stored number-to-hash index value; recomputing the contextual canonical header hash requires PBFT and configuration facts outside this bounded report.",
        "The producer binary identity and exact paired checkpoint timing remain unverified.",
    ];
    if !every_hash_index_present || !every_hash_index_matches_period {
        gaps.push(
            "Observed per-transaction receipt hash-index rows are incomplete; the complete ordered period receipt list is independently bound by the matching header receipt root.",
        );
    }

    let report = Report {
        schema: 1,
        tool_package_version: env!("CARGO_PKG_VERSION"),
        tool_source_sha256: sha256_hex(include_bytes!("head_replay_inputs.rs")),
        linked_rocksdb: "rocksdb crate 0.24.0 / librocksdb 10.4.2",
        input_copy: input.display().to_string(),
        rocksdb_open: RocksDbOpen {
            mode: "DB::open_cf_descriptors_read_only",
            create_if_missing: false,
            create_missing_column_families: false,
            application_column_family_count: app_columns.len(),
            state_column_family_count: state_columns.len(),
            missing_current_rust_application_columns: Column::all()
                .iter()
                .filter(|column| !app_columns.iter().any(|present| present == column.name()))
                .map(Column::name)
                .collect(),
        },
        period: TARGET_PERIOD,
        application_head,
        period_data: raw_fact(Some(&period_data)),
        period_data_children,
        ordered_inputs: OrderedInputs {
            regular_count,
            system_count: system_hashes.len(),
            total_count: ordered_rlps.len(),
            system_hash_list: raw_fact((!system_hash_list.is_empty()).then_some(&system_hash_list)),
            system_hash_references_complete,
            storage_positions_complete,
            transaction_root_hex: hex::encode(transaction_root),
            transaction_root_matches_header,
            concrete_transaction_bundle_keccak_hex: concrete_transaction_bundle_hash(&ordered_rlps),
            observed_chain_ids: observed_chain_ids.into_iter().collect(),
            all_regular_signatures_valid,
            entries,
        },
        receipts: ReceiptReport {
            row: raw_fact(Some(&receipts_rlp)),
            count: receipt_count,
            count_matches_inputs: receipt_count_matches,
            ordered_root_hex: hex::encode(receipt_root),
            root_matches_header: receipt_root_matches_header,
            every_hash_index_present,
            every_hash_index_matches_period,
            entries: receipt_entries,
        },
        header: HeaderReport {
            row: raw_fact(Some(&header_raw)),
            block_hash_hex: hex::encode(block_hash),
            block_hash_recomputed: false,
            parent_hash_hex: hex::encode(header.parent_hash),
            parent_matches_prior_block_hash: header.parent_hash == H256::from(prior_block_hash),
            state_root_hex: hex::encode(header.state_root),
            transactions_root_hex: hex::encode(header.transactions_root),
            receipts_root_hex: hex::encode(header.receipts_root),
            gas_used: header.gas_used.as_u64(),
            total_reward_hex: hex::encode(header.total_reward.to_fixed_be_bytes()),
        },
        prior_state: PriorStateReport {
            current_descriptor_period: descriptor.period.as_u64(),
            current_descriptor_root_hex: hex::encode(descriptor.state_root),
            current_descriptor_matches_head_header,
            current_root_node_present,
            prior_period,
            prior_header: raw_fact(Some(&prior_header_raw)),
            prior_root_hex: hex::encode(prior_header.state_root),
            prior_root_node_present,
            historical_reader_constructor_accepted,
            prior_descriptor_available,
            complete_trie_closure_verified: false,
        },
        configuration: ConfigurationReport {
            database_genesis_hash_hex: genesis_hash.as_ref().map(hex::encode),
            documented_mainnet_genesis_hash_hex: DOCUMENTED_MAINNET_GENESIS,
            documented_mainnet_genesis_matches: genesis_hash
                .as_ref()
                .is_some_and(|hash| hex::encode(hash) == DOCUMENTED_MAINNET_GENESIS),
            period_lambda_at_or_before: period_lambda,
            dynamic_lambda_round_count,
            sortition_parameters: raw_fact(sortition_parameters.as_ref()),
            block_rewards_stats: raw_fact(rewards_stats.as_ref()),
            exact_static_node_config_qualified: false,
            previous_certificate_vote_payloads_and_weights_reconstructed: false,
        },
        qualification: Qualification {
            ordered_inputs_complete,
            receipts_complete,
            header_present_and_paired,
            prior_descriptor_available,
            bounded_head_bundle_complete,
            future_replay_ready: false,
        },
        gaps,
    };

    write_report(&output, &report)
}

fn input_entry(
    position: usize,
    kind: &'static str,
    envelope: &LegacyTransactionEnvelope,
    location_rlp: Option<Vec<u8>>,
    is_system: bool,
) -> Result<InputEntry> {
    let location = location_rlp.as_deref().map(decode_location).transpose()?;
    Ok(InputEntry {
        position,
        kind,
        transaction_hash_hex: hex::encode(envelope.hash),
        rlp_bytes: envelope.rlp.len(),
        rlp_sha256: sha256_hex(&envelope.rlp),
        chain_id: envelope.chain_id,
        signature_valid: envelope.signature_valid,
        location_present: location.is_some(),
        location_matches_order: location.is_some_and(|location| {
            location.period == TARGET_PERIOD
                && location.position as usize == position
                && location.is_system == is_system
        }),
        receipt_by_hash_present: false,
        receipt_by_hash_matches_period: false,
    })
}

fn decode_location(bytes: &[u8]) -> Result<TransactionLocation> {
    let location = Rlp::new(bytes);
    let count = location.item_count()?;
    ensure!(matches!(count, 2 | 3), "invalid transaction location shape");
    Ok(TransactionLocation {
        period: location.val_at(0)?,
        position: location.val_at(1)?,
        is_system: if count == 3 {
            location.val_at(2)?
        } else {
            false
        },
    })
}

fn decode_system_hashes(bytes: &[u8]) -> Result<Vec<H256>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let list = Rlp::new(bytes);
    let count = list.item_count()?;
    let hashes = (0..count)
        .map(|index| {
            let bytes = list.at(index)?.data()?;
            ensure!(bytes.len() == 32, "system transaction hash is not 32 bytes");
            Ok(H256::from_slice(bytes))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut canonical = RlpStream::new_list(hashes.len());
    for hash in &hashes {
        canonical.append(&hash.as_bytes());
    }
    ensure!(
        canonical.out().as_ref() == bytes,
        "system hash list is not canonical RLP"
    );
    Ok(hashes)
}

fn decode_descriptor(bytes: &[u8]) -> Result<ConcreteStateIdentity> {
    let descriptor = Rlp::new(bytes);
    ensure!(
        descriptor.item_count()? == 2,
        "invalid concrete descriptor shape"
    );
    let period = descriptor.val_at::<u64>(0)?;
    let root = exact_32(descriptor.at(1)?.data()?, "concrete descriptor root")?;
    let mut canonical = RlpStream::new_list(2);
    canonical.append(&period);
    canonical.append(&root.as_slice());
    ensure!(
        canonical.out().as_ref() == bytes,
        "concrete descriptor is not canonical RLP"
    );
    Ok(ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(period),
        state_root: root,
    })
}

fn ordered_root(values: &[Vec<u8>]) -> [u8; 32] {
    ordered_trie_root::<KeccakHasher, _>(values.iter().map(Vec::as_slice))
}

fn concrete_transaction_bundle_hash(values: &[Vec<u8>]) -> String {
    let mut stream = RlpStream::new_list(values.len());
    for value in values {
        stream.append(&value.as_slice());
    }
    keccak_hex(&stream.out())
}

fn raw_fact(value: Option<&Vec<u8>>) -> RawFact {
    RawFact {
        present: value.is_some(),
        bytes: value.map_or(0, Vec::len),
        sha256: value.map(|value| sha256_hex(value)),
    }
}

fn open_application_read_only(path: &Path) -> Result<(Database, Vec<String>)> {
    let mut db_options = Options::default();
    db_options.create_if_missing(false);
    db_options.create_missing_column_families(false);
    db_options.set_max_open_files(128);
    let columns = Database::list_cf(&db_options, path)?;
    for required in REQUIRED_APPLICATION_COLUMNS {
        ensure!(
            columns.iter().any(|present| present == required),
            "application column family {:?} is missing",
            required
        );
    }
    let descriptors = columns.iter().map(|name| {
        Column::from_name(name).map_or_else(
            |_| ColumnFamilyDescriptor::new(name, Options::default()),
            |column| column.descriptor(&Options::default()),
        )
    });
    Ok((
        Database::open_cf_descriptors_read_only(&db_options, path, descriptors, false)?,
        columns,
    ))
}

fn open_state_read_only(path: &Path) -> Result<(Database, Vec<String>)> {
    let mut db_options = Options::default();
    db_options.create_if_missing(false);
    db_options.create_missing_column_families(false);
    let columns = Database::list_cf(&db_options, path)?;
    for required in ["default", "1", "2", "3", "4", "5", "6", "7", "8"] {
        ensure!(
            columns.iter().any(|present| present == required),
            "state column family {required:?} is missing"
        );
    }
    let descriptors = columns
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
    Ok((
        Database::open_cf_descriptors_read_only(&db_options, path, descriptors, false)?,
        columns,
    ))
}

fn raw_cf(db: &Database, column: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let handle = db
        .cf_handle(column)
        .with_context(|| format!("missing column family {column}"))?;
    Ok(db.get_cf(&handle, key)?)
}

fn validated_paths() -> Result<(PathBuf, PathBuf)> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: head_replay_inputs SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: head_replay_inputs SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    ensure!(
        args.next().is_none(),
        "usage: head_replay_inputs SNAPSHOT_COPY OUTPUT_JSON"
    );

    let supplied = fs::canonicalize(SUPPLIED_EVIDENCE)?;
    let input = fs::canonicalize(input)?;
    ensure!(
        input != supplied && !input.starts_with(&supplied),
        "refusing to open the supplied evidence snapshot; pass an independent copy"
    );
    ensure!(!output.exists(), "output already exists");
    let output_name = output.file_name().context("output path has no file name")?;
    let output_parent = fs::canonicalize(output.parent().unwrap_or_else(|| Path::new(".")))?;
    let output = output_parent.join(output_name);
    ensure!(
        !output.starts_with(&supplied) && !output.starts_with(&input),
        "output must be outside the supplied evidence and independent copy"
    );
    Ok((input, output))
}

fn canonical_child(input: &Path, relative: &str) -> Result<PathBuf> {
    let supplied = fs::canonicalize(SUPPLIED_EVIDENCE)?;
    let child = fs::canonicalize(input.join(relative))?;
    ensure!(
        child.starts_with(input) && !child.starts_with(supplied),
        "snapshot child resolves outside the independent copy"
    );
    Ok(child)
}

fn write_report(path: &Path, report: &Report) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut output, report)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

fn exact_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .try_into()
            .with_context(|| format!("{label} is not eight bytes"))?,
    ))
}

fn exact_32(bytes: &[u8], label: &str) -> Result<[u8; 32]> {
    bytes
        .try_into()
        .with_context(|| format!("{label} is not 32 bytes"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn keccak_hex(bytes: &[u8]) -> String {
    let mut output = [0_u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut output);
    hex::encode(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_hash_list_requires_canonical_exact_hashes() {
        let hashes = [H256::repeat_byte(1), H256::repeat_byte(2)];
        let mut list = RlpStream::new_list(hashes.len());
        for hash in hashes {
            list.append(&hash.as_bytes());
        }
        assert_eq!(decode_system_hashes(&list.out()).unwrap(), hashes);
        assert!(decode_system_hashes(&[0xc1, 0x80]).is_err());
    }

    #[test]
    fn location_preserves_system_marker_and_position() {
        let mut regular = RlpStream::new_list(2);
        regular.append(&TARGET_PERIOD);
        regular.append(&7_u32);
        assert!(!decode_location(&regular.out()).unwrap().is_system);

        let mut system = RlpStream::new_list(3);
        system.append(&TARGET_PERIOD);
        system.append(&8_u32);
        system.append(&true);
        let decoded = decode_location(&system.out()).unwrap();
        assert_eq!(decoded.position, 8);
        assert!(decoded.is_system);
    }

    #[test]
    fn descriptor_rejects_noncanonical_period_bytes() {
        let root = [3_u8; 32];
        let mut canonical = RlpStream::new_list(2);
        canonical.append(&1_u64);
        canonical.append(&root.as_slice());
        assert_eq!(
            decode_descriptor(&canonical.out()).unwrap().period.as_u64(),
            1
        );

        let mut noncanonical = vec![0xe3, 0x82, 0x00, 0x01, 0xa0];
        noncanonical.extend_from_slice(&root);
        assert!(decode_descriptor(&noncanonical).is_err());
    }
}
