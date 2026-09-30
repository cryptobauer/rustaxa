//! Bounded retained DPoS configuration metadata qualification at Q.
//!
//! The guarded working pair is qualified before reading only concrete CF8.
//! Exhaustion within sixteen records and 64KiB of key/value bytes is mandatory;
//! one extra validity lookahead detects overflow. Keys are canonical minimal
//! big-endian integers, numerically sorted rather than predecessor-seeked.
//! These physical metadata records are not authenticated by the state root and
//! cannot certify producer identity, hardforks, PBFT settings or execution.
use anyhow::{Context, Result, ensure};
use num_bigint::BigUint;
use rlp::Rlp;
use rocksdb::{ColumnFamilyDescriptor, DB, Options};
use rustaxa_snapshot_qualifier::{paths, qualification};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

const Q: u64 = 25_706_947;
const MAX_ROWS: usize = 16;
const MAX_BYTES: usize = 64 * 1024;
const GENESIS: &[u8] = include_bytes!(
    "../../../../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json"
);
const SOURCE_REV: &str = "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b";
const FIELDS: [&str; 12] = [
    "eligibility_balance_threshold",
    "vote_eligibility_balance_step",
    "validator_maximum_stake",
    "minimum_deposit",
    "max_block_author_reward",
    "dag_proposers_reward",
    "commission_change_delta",
    "commission_change_frequency",
    "delegation_delay",
    "delegation_locking_period",
    "blocks_per_year",
    "yield_percentage",
];

type RawRow = (Vec<u8>, Vec<u8>);

/// Requires the exact Go minimal big-endian update-period encoding: zero is an
/// empty key; nonzero keys have no leading zero and fit an unsigned u64.
fn update_period(key: &[u8]) -> Result<u64> {
    ensure!(
        key.len() <= 8 && key.first() != Some(&0),
        "noncanonical/oversized DPoS update key"
    );
    Ok(key
        .iter()
        .fold(0_u64, |period, byte| (period << 8) | u64::from(*byte)))
}

/// Validates canonical RLP framing across the complete bounded record, including
/// byte strings and nested list headers. The expected Go structure reaches at
/// most five nested levels; excess nesting is not a valid DPOSConfig shape.
fn canonical_rlp(raw: &[u8], depth: usize) -> Result<()> {
    ensure!(
        depth <= 5,
        "DPOSConfig RLP nesting exceeds structural bound"
    );
    let item = Rlp::new(raw);
    let info = item.payload_info()?;
    ensure!(info.total() == raw.len(), "incomplete/trailing RLP item");
    let mut encoded = rlp::RlpStream::new();
    if item.is_list() {
        let mut offset = info.header_len;
        let mut children = Vec::new();
        while offset < raw.len() {
            let length = Rlp::new(&raw[offset..]).payload_info()?.total();
            let end = offset
                .checked_add(length)
                .context("RLP child length overflow")?;
            ensure!(end <= raw.len(), "RLP child exceeds list payload");
            canonical_rlp(&raw[offset..end], depth + 1)?;
            children.push(&raw[offset..end]);
            offset = end;
        }
        encoded.begin_list(children.len());
        for child in children {
            encoded.append_raw(child, 1);
        }
    } else {
        encoded.append(&item.data()?);
    }
    ensure!(encoded.out().as_ref() == raw, "noncanonical RLP framing");
    Ok(())
}

/// Decodes canonical unsigned RLP scalars. Go big.Int has no U256 width limit;
/// width restrictions apply only when explicitly requested for Go u16/u32.
fn unsigned(item: &Rlp<'_>, width: Option<usize>) -> Result<Vec<u8>> {
    let bytes = item.data()?;
    ensure!(bytes.first() != Some(&0), "noncanonical unsigned integer");
    ensure!(
        width.is_none_or(|max| bytes.len() <= max),
        "unsigned scalar exceeds declared Go width"
    );
    let mut encoded = rlp::RlpStream::new();
    encoded.append(&bytes);
    ensure!(
        encoded.out().as_ref() == item.as_raw(),
        "noncanonical scalar RLP"
    );
    Ok(bytes.to_vec())
}

/// Validates the complete initial-validator tail's structural Go codec contract.
/// Strings and VRF fields are byte strings without invented UTF8/VRF restrictions.
/// Genesis semantics and cryptographic validity remain outside this diagnostic.
fn validators(item: &Rlp<'_>) -> Result<usize> {
    ensure!(item.is_list(), "initial validators must be a list");
    let count = item.item_count()?;
    for validator in item.iter() {
        ensure!(
            validator.is_list() && validator.item_count()? == 7,
            "initial validator must have seven fields"
        );
        for index in [0, 1] {
            ensure!(
                validator.at(index)?.data()?.len() == 20,
                "initial validator address must be twenty bytes"
            );
        }
        for index in [2, 4, 5] {
            validator.at(index)?.data()?;
        }
        unsigned(&validator.at(3)?, Some(2))?;
        let delegations = validator.at(6)?;
        ensure!(
            delegations.is_list(),
            "initial delegations must be a map list"
        );
        let mut addresses = BTreeSet::new();
        for pair in delegations.iter() {
            ensure!(
                pair.is_list() && pair.item_count()? == 2,
                "delegation map entry must have two fields"
            );
            let address = pair.at(0)?.data()?.to_vec();
            ensure!(
                address.len() == 20 && addresses.insert(address),
                "invalid/duplicate delegation map address"
            );
            unsigned(&pair.at(1)?, None)?;
        }
    }
    Ok(count)
}

/// Validates exact thirteen-field DPOSConfig, including nested validator/map
/// shapes and canonical scalar widths, and preserves source-row fingerprints.
/// This is codec qualification, not validation of all genesis domain semantics.
fn decode_row(key: &[u8], raw: &[u8]) -> Result<Value> {
    let period = update_period(key)?;
    canonical_rlp(raw, 0)?;
    let config = Rlp::new(raw);
    ensure!(
        config.is_list()
            && config.item_count()? == 13
            && config.payload_info()?.total() == raw.len(),
        "DPOSConfig must be one complete thirteen-field RLP list"
    );
    let mut scalars = serde_json::Map::new();
    for (index, name) in FIELDS.iter().enumerate() {
        let width = match index {
            0..=3 => None,
            4..=6 | 11 => Some(2),
            _ => Some(4),
        };
        let bytes = unsigned(&config.at(index)?, width)?;
        scalars.insert((*name).to_owned(), json!({"unsigned_decimal": BigUint::from_bytes_be(&bytes).to_string(), "canonical_unsigned_be_hex": hex::encode(&bytes), "bytes": bytes.len()}));
    }
    let validator_count = validators(&config.at(12)?)?;
    Ok(
        json!({"update_period": period, "raw_key_hex": hex::encode(key), "raw_bytes": raw.len(), "raw_sha256": hex::encode(Sha256::digest(raw)), "scalar_fields": scalars, "initial_validator_count": validator_count, "full_record_codec_validated": true, "genesis_semantics_qualified": false}),
    )
}

/// Sorts complete bounded metadata numerically, requires the baseline and picks
/// the latest update at/before Q. Future updates do not become current policy.
fn qualify_rows(rows: &[RawRow], query: u64) -> Result<Value> {
    ensure!(rows.len() <= MAX_ROWS, "DPoS config record cap exceeded");
    let total = rows.iter().try_fold(0_usize, |sum, (key, value)| {
        sum.checked_add(key.len())
            .and_then(|sum| sum.checked_add(value.len()))
            .context("metadata byte count overflow")
    })?;
    ensure!(total <= MAX_BYTES, "DPoS config byte cap exceeded");
    let mut decoded = rows
        .iter()
        .map(|(key, raw)| decode_row(key, raw))
        .collect::<Result<Vec<_>>>()?;
    decoded.sort_by_key(|row| row["update_period"].as_u64().unwrap());
    ensure!(
        decoded.first().is_some_and(|row| row["update_period"] == 0),
        "retained DPoS baseline zero missing"
    );
    ensure!(
        decoded
            .windows(2)
            .all(|pair| pair[0]["update_period"] != pair[1]["update_period"]),
        "duplicate numeric update periods"
    );
    let selected = decoded
        .iter()
        .rev()
        .find(|row| row["update_period"].as_u64().unwrap() <= query)
        .context("no DPoS configuration at Q")?;
    let policy_comparison = compare_candidate(selected)?;
    Ok(
        json!({"query_period": query, "cf": "8", "exhausted_within_caps": true, "record_count": rows.len(), "key_value_bytes": total, "updates_numeric_order": decoded, "selected": selected, "candidate_comparison": policy_comparison}),
    )
}

/// Compares independent retained scalars to checked-in candidate JSON values.
/// No producer or hardfork provenance is inferred from equality.
fn compare_candidate(selected: &Value) -> Result<Value> {
    let candidate: Value = serde_json::from_slice(GENESIS)?;
    let mut comparison = serde_json::Map::new();
    for name in [
        "eligibility_balance_threshold",
        "vote_eligibility_balance_step",
        "validator_maximum_stake",
        "delegation_delay",
    ] {
        let text = candidate["dpos"][name]
            .as_str()
            .context("missing candidate DPoS scalar")?;
        let digits = text
            .strip_prefix("0x")
            .context("candidate scalar must be hex")?;
        let decimal = BigUint::parse_bytes(digits.as_bytes(), 16)
            .context("invalid candidate hex scalar")?
            .to_string();
        comparison.insert(name.to_owned(), json!({"candidate_unsigned_decimal": decimal, "retained_unsigned_decimal": selected["scalar_fields"][name]["unsigned_decimal"], "matches": selected["scalar_fields"][name]["unsigned_decimal"] == decimal}));
    }
    Ok(
        json!({"fields": comparison, "candidate_source_sha256": hex::encode(Sha256::digest(GENESIS)), "relevant_stake_fields_fit_u256": FIELDS[..3].iter().all(|name| selected["scalar_fields"][name]["bytes"].as_u64().unwrap() <= 32), "producer_configuration_qualified": false}),
    )
}

/// Opens existing concrete DB with read-only bytewise descriptors, matching the
/// existing concrete reader owner. Only CF8 is iterated by the caller.
fn open_state(path: &Path) -> Result<DB> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let columns = DB::list_cf(&options, path)?;
    let descriptors = columns
        .iter()
        .map(|name| ColumnFamilyDescriptor::new(name, Options::default()));
    Ok(DB::open_cf_descriptors_read_only(
        &options,
        path,
        descriptors,
        false,
    )?)
}

/// Exhausts only configuration metadata within the fixed budget. Raw iterator
/// validity permits one lookahead without requesting/copying an overflow value;
/// iterator errors or cap overflow fail before any configuration qualification.
fn read_rows(db: &DB) -> Result<Vec<RawRow>> {
    let column = db.cf_handle("8").context("missing concrete config CF8")?;
    let mut iterator = db.raw_iterator_cf(column);
    iterator.seek_to_first();
    let mut rows = Vec::new();
    let mut bytes = 0_usize;
    while iterator.valid() {
        ensure!(
            rows.len() < MAX_ROWS,
            "DPoS config record cap exceeded (one extra valid entry)"
        );
        let key = iterator.key().context("valid iterator has no key")?;
        let value = iterator.value().context("valid iterator has no value")?;
        bytes = bytes
            .checked_add(key.len())
            .and_then(|sum| sum.checked_add(value.len()))
            .context("config byte count overflow")?;
        ensure!(bytes <= MAX_BYTES, "DPoS config byte cap exceeded");
        rows.push((key.to_vec(), value.to_vec()));
        iterator.next();
    }
    iterator.status()?;
    Ok(rows)
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: retained_dpos_config COPY OUTPUT")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: retained_dpos_config COPY OUTPUT")?,
    );
    ensure!(args.next().is_none(), "unexpected argument");
    let paths = paths::validate(&input, &output)?;
    let (_readers, _identity, pair) = qualification::qualify(&paths.application, &paths.state)?;
    ensure!(Q < pair.head, "Q must precede qualified head");
    let state = open_state(&paths.state)?;
    let rows = read_rows(&state)?;
    let config = qualify_rows(&rows, Q)?;
    let report = json!({
        "input_copy": paths.input, "pair_qualification": pair, "tool_source_sha256": tool_source_sha256(),
        "retained_dpos_configuration": config,
        "read_bound": {"pair_application_point_reads": 4, "concrete_qualification": "existing descriptor/current/prior-root checks", "metadata_cf": "8", "maximum_accepted_records": MAX_ROWS, "maximum_accepted_key_value_bytes": MAX_BYTES, "maximum_extra_validity_lookaheads": 1, "trie_scan_performed": false, "other_cf_iteration_performed": false},
        "source_contract": {"immutable_go_revision": SOURCE_REV, "fields": "taraxa/state/chain_config/chain_config.go:152; full thirteen-field DPOSConfig", "update_keys": "taraxa/state/state_db_rocksdb/db.go:532/551; minimal BE bytes, zero empty", "numeric_selection": "taraxa/state/api.go:39-70; sorted update keys; contracts/dpos/precompiled/api.go:83-104", "delegation_map": "rlp/decode.go:574; list of two-field pairs with duplicate rejection"},
        "qualification": {"retained_metadata_exhausted_within_caps": true, "full_record_codec_validated": true, "selected_dpos_scalar_metadata_available": true, "metadata_state_root_authenticated": false, "genesis_semantics_qualified": false, "producer_configuration_qualified": false, "producer_binary_qualified": false, "hardfork_configuration_qualified": false, "pbft_configuration_qualified": false, "vote_inputs_or_reward_transition_qualified": false, "adoption_authorized": false}
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn tool_source_sha256() -> String {
    let mut digest = Sha256::new();
    for source in [
        include_bytes!("retained_dpos_config.rs").as_slice(),
        include_bytes!("../paths.rs"),
        include_bytes!("../qualification.rs"),
    ] {
        digest.update(source);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Vec<u8> {
        let mut encoded = rlp::RlpStream::new_list(13);
        for _ in 0..12 {
            encoded.append(&1_u32);
        }
        encoded.begin_list(0);
        encoded.out().to_vec()
    }

    #[test]
    fn zero_key_and_numeric_order_match_go_instead_of_bytewise_order() {
        assert_eq!(update_period(&[]).unwrap(), 0);
        assert_eq!(update_period(&[1, 0]).unwrap(), 256);
        for key in [vec![0], vec![0, 1], vec![1; 9]] {
            assert!(update_period(&key).is_err());
        }
        let rows = vec![
            (vec![], record()),
            (vec![1, 0], record()),
            (vec![255], record()),
            (vec![2], record()),
        ];
        let result = qualify_rows(&rows, 255).unwrap();
        assert_eq!(result["selected"]["update_period"], 255);
        assert_eq!(result["updates_numeric_order"][3]["update_period"], 256);
        assert!(qualify_rows(&rows[1..], 255).is_err());
    }

    #[test]
    fn metadata_caps_fail_instead_of_qualifying_truncated_records() {
        let rows = vec![(vec![], record()); 17];
        assert!(qualify_rows(&rows, Q).is_err());
        assert!(qualify_rows(&[(vec![], vec![0; MAX_BYTES + 1])], Q).is_err());
        assert!(qualify_rows(&[], Q).is_err());
    }

    #[test]
    fn exact_full_record_rejects_trailing_width_and_integer_drift() {
        let raw = record();
        assert!(decode_row(&[], &raw).is_ok());
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(decode_row(&[], &trailing).is_err());
        assert!(decode_row(&[], &[0xc0]).is_err());
        assert!(unsigned(&Rlp::new(&[0]), None).is_err());
        assert!(unsigned(&Rlp::new(&[0x83, 1, 0, 0]), Some(2)).is_err());
        assert!(unsigned(&Rlp::new(&[0x81, 1]), None).is_err());
        assert!(unsigned(&Rlp::new(&[0x80]), Some(2)).is_ok());
    }

    #[test]
    fn recursive_framing_rejects_noncanonical_nested_bytes_and_lists() {
        assert!(canonical_rlp(&[0xc1, 0x80], 0).is_ok());
        assert!(canonical_rlp(&[0xc2, 0xb8, 0], 0).is_err());
        assert!(canonical_rlp(&[0xf8, 1, 0x80], 0).is_err());
        assert!(canonical_rlp(&[0xc1, 0xc1], 0).is_err());
        assert!(canonical_rlp(&[0xc2, 0x81, 1], 0).is_err());
    }

    #[test]
    fn full_nested_validator_record_is_codec_valid_without_genesis_semantics() {
        let mut encoded = rlp::RlpStream::new_list(13);
        for _ in 0..12 {
            encoded.append(&1_u32);
        }
        encoded.begin_list(1).begin_list(7);
        encoded
            .append(&[1_u8; 20].as_slice())
            .append(&[2_u8; 20].as_slice())
            .append(&b"arbitrary-vrf-width".as_slice())
            .append(&0_u16)
            .append(&[0xff_u8].as_slice())
            .append(&[].as_slice());
        encoded
            .begin_list(1)
            .begin_list(2)
            .append(&[3_u8; 20].as_slice())
            .append(&1_u32);
        let result = decode_row(&[], &encoded.out()).unwrap();
        assert_eq!(result["initial_validator_count"], 1);
        assert_eq!(result["full_record_codec_validated"], true);
        assert_eq!(result["genesis_semantics_qualified"], false);
        let config = qualify_rows(&[(vec![], record())], Q).unwrap();
        assert_eq!(
            config["candidate_comparison"]["fields"]["delegation_delay"]["matches"],
            false
        );
    }

    #[test]
    fn validator_tail_checks_shapes_and_duplicate_delegation_addresses() {
        let mut encoded = rlp::RlpStream::new_list(1);
        encoded.begin_list(7);
        encoded
            .append(&[1_u8; 20].as_slice())
            .append(&[2_u8; 20].as_slice())
            .append(&[].as_slice())
            .append(&0_u16)
            .append(&b"endpoint".as_slice())
            .append(&b"description".as_slice());
        encoded.begin_list(2);
        for _ in 0..2 {
            encoded
                .begin_list(2)
                .append(&[3_u8; 20].as_slice())
                .append(&1_u32);
        }
        assert!(validators(&Rlp::new(&encoded.out())).is_err());
        assert!(validators(&Rlp::new(&[0x80])).is_err());
    }
}
