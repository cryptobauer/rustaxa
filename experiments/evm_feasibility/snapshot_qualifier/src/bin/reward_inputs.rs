//! Extracts one bounded retained-head reward-input candidate without state reads.
//!
//! The tool accepts only the already-qualified independent snapshot copy and
//! opens only its application database through RocksDB's read-only API. Exact
//! retained `PeriodData`, receipt, header, and current `BlockStats` rows are
//! decoded with Rust compatibility types. The `BlockStats` row captures the
//! producer-observed blocks-per-year and vote weights, so values derived from it
//! remain candidate reconstruction inputs rather than independent prior-state
//! proof. No state database, range iterator, execution, or publication handle is
//! opened.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, ensure};
use ethereum_types::H160;
use rlp::Rlp;
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
use rustaxa_consensus::{
    FinalizedRewardsPeriodFact, RewardCertVoteFact, RewardDagBlockFact, RewardTransactionFact,
    RewardsFrequencyRule, RewardsStatsConfig, RewardsStatsPeriodRlp, RewardsStatsRuntime,
    RewardsStatsStatus, build_weighted_pbft_vote_bundle, build_weighted_pbft_vote_payload,
    decode_rewards_block_distributions, encode_concrete_rewards_input, inspect_canonical_pbft_vote,
};
use rustaxa_storage::{Column, FinalChainRepository, MetadataRepository, PeriodRepository};
use rustaxa_types::codec::rlp::dag::{DagBlockRlp, FinalizedDagBlockBundleRlp};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    DagBlock, FinalChainGas, LegacyTransactionEnvelope, PbftBlockMetadata,
    StoredFinalChainBlockHeader,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use rustaxa_snapshot_qualifier::reward_votes::{ReconstructedVotes, reconstruct_cert_votes};

const TARGET_PERIOD: u64 = 25_706_949;
const EXPECTED_PERIOD_DATA_BYTES: usize = 7_286;
const EXPECTED_PERIOD_DATA_SHA256: &str =
    "bcb4b16bed154c41156ed87f6d020f5ed7fad96f0c7562af778aeefd474de11b";
const EXPECTED_REWARDS_STATS_BYTES: usize = 566;
const EXPECTED_REWARDS_STATS_SHA256: &str =
    "18bb350916c3d85a52ec401cb2b898d2696b57e3859d5ff9dda1218a8ed05070";
const EXPECTED_RECEIPTS_BYTES: usize = 208;
const EXPECTED_RECEIPTS_SHA256: &str =
    "a0cf47cce427e5517939c1eeaefc347184899e5fef60d6a580db109d30260aa1";
const EXPECTED_HEADER_BYTES: usize = 399;
const EXPECTED_HEADER_SHA256: &str =
    "1bebb6391e97caa4e8d34db4336810ec71eae83ef975d48bfb05e0b56e81363f";
const EXPECTED_BLOCK_HASH: &str =
    "e5b61be65207997d9d30536500b4bd8be3fc2bbe3ec00c38343f20282c6e5065";
const DOCUMENTED_MAINNET_GENESIS: &str =
    "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377";
const MAINNET_COMMITTEE_SIZE: u32 = 1_000;
const MAINNET_MAGNOLIA_PERIOD: u64 = 5_730_000;
const MAINNET_ASPEN_ONE_PERIOD: u64 = 8_118_000;
const MAINNET_REWARDS_FREQUENCY_FROM: u64 = 5_730_000;
const MAINNET_REWARDS_FREQUENCY: u32 = 100;
const REQUIRED_APPLICATION_COLUMNS: &[&str] = &[
    "period_data",
    "genesis",
    "final_chain_meta",
    "final_chain_blk_by_number",
    "final_chain_blk_hash_by_number",
    "final_chain_receipt_by_period",
    "block_rewards_stats",
];
type Database = DBWithThreadMode<MultiThreaded>;

#[derive(Serialize)]
struct Report {
    schema: u32,
    tool_package_version: &'static str,
    tool_source_sha256: String,
    input_copy: String,
    read_contract: ReadContract,
    period: u64,
    rows: Rows,
    observed_current_stats: ObservedStats,
    cert_vote_candidate: CertVoteCandidate,
    planner_candidate: PlannerCandidate,
    planner_comparison: PlannerComparison,
    qualification: Qualification,
}

#[derive(Serialize)]
struct ReadContract {
    mode: &'static str,
    application_only: bool,
    exact_key_reads: Vec<&'static str>,
    range_or_iterator_reads: bool,
    application_column_family_count: usize,
}

#[derive(Serialize)]
struct Rows {
    period_data: RawFact,
    receipts: RawFact,
    header: RawFact,
    current_block_stats: RawFact,
    genesis_hash_hex: String,
    block_hash_hex: String,
}

#[derive(Serialize)]
struct RawFact {
    bytes: usize,
    sha256: String,
}

#[derive(Serialize)]
struct ObservedStats {
    block_author_hex: String,
    blocks_per_year: u32,
    validator_rows: usize,
    total_dag_blocks_count: u32,
    total_votes_weight: u64,
    max_votes_weight: u64,
}

#[derive(Serialize)]
struct CertVoteCandidate {
    certified_period: u64,
    round: u64,
    step: u64,
    count: usize,
    unique_voters: bool,
    every_voter_has_observed_nonzero_weight: bool,
    no_unpaired_observed_nonzero_weights: bool,
    observed_weights_sum_matches_stats: bool,
    weighted_bundle_bytes: usize,
    weighted_bundle_sha256: String,
    weight_source: &'static str,
    independent_prior_state_weights_qualified: bool,
}

#[derive(Serialize)]
struct PlannerCandidate {
    config_source: &'static str,
    checked_in_mainnet_config_producer_qualified: bool,
    capped_dpos_total_vote_projection: u64,
    exact_dpos_total_vote_count_qualified: bool,
    exact_current_block_stats_match: bool,
    cache_current_period: bool,
    clear_cached_stats: bool,
    distribution_stats_count: usize,
    concrete_rewards_input_hex: String,
    concrete_rewards_input_sha256: String,
}

#[derive(Serialize)]
struct PlannerComparison {
    candidate_row: RawFact,
    exact_rlp_bytes_match: bool,
    typed_full_distribution_match: bool,
    block_author_match: bool,
    blocks_per_year_match: bool,
    validators_stats_match: bool,
    total_dag_blocks_count_match: bool,
    total_votes_weight_match: bool,
    max_votes_weight_match: bool,
    retained_validator_order_hex: Vec<String>,
    candidate_validator_order_hex: Vec<String>,
    validator_order_match: bool,
    legacy_unordered_map_order_qualified: bool,
}

#[derive(Serialize)]
struct Qualification {
    exact_retained_rows_match_recorded_hashes: bool,
    captured_candidate_sidecar_reconstructed: bool,
    candidate_planner_typed_fields_match_retained_row: bool,
    candidate_planner_exact_bytes_match_retained_row: bool,
    historical_vrf_or_prior_vote_inputs_qualified: bool,
    producer_compatible_reward_request_qualified: bool,
    state_api_or_end_block_executed: bool,
    reward_transition_or_root_qualified: bool,
    producer_binary_or_global_hardfork_config_qualified: bool,
    note: &'static str,
}

fn main() -> Result<()> {
    let (input, output) = validated_paths()?;
    let app_path = canonical_application_path(&input)?;
    let (application, columns) = open_application_read_only(&app_path)?;
    let application = Arc::new(application);
    let periods = PeriodRepository::new(application.clone());
    let final_chain = FinalChainRepository::new(application.clone());
    let metadata = MetadataRepository::new(application.clone());

    let head = final_chain
        .meta_value(1)?
        .context("missing FinalChain head metadata")?;
    ensure!(exact_le_u64(&head, "FinalChain head")? == TARGET_PERIOD);
    let genesis = metadata.genesis_hash()?.context("missing genesis hash")?;
    ensure!(hex::encode(&genesis) == DOCUMENTED_MAINNET_GENESIS);

    let period_data = periods.data_raw(TARGET_PERIOD)?;
    gate_row(
        &period_data,
        EXPECTED_PERIOD_DATA_BYTES,
        EXPECTED_PERIOD_DATA_SHA256,
        "target PeriodData",
    )?;
    let receipts = periods.receipt(TARGET_PERIOD)?;
    gate_row(
        &receipts,
        EXPECTED_RECEIPTS_BYTES,
        EXPECTED_RECEIPTS_SHA256,
        "target receipts",
    )?;
    let header = final_chain
        .block_header_raw(TARGET_PERIOD)?
        .context("missing target FinalChain header")?;
    gate_row(
        &header,
        EXPECTED_HEADER_BYTES,
        EXPECTED_HEADER_SHA256,
        "target header",
    )?;
    let block_hash = final_chain
        .block_hash_by_number(TARGET_PERIOD)?
        .context("missing target block hash")?;
    ensure!(hex::encode(&block_hash) == EXPECTED_BLOCK_HASH);
    let current_stats = raw_cf(
        &application,
        "block_rewards_stats",
        &TARGET_PERIOD.to_le_bytes(),
    )?
    .context("missing target current BlockStats")?;
    gate_row(
        &current_stats,
        EXPECTED_REWARDS_STATS_BYTES,
        EXPECTED_REWARDS_STATS_SHA256,
        "target current BlockStats",
    )?;

    let stored_header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header))?;
    ensure!(stored_header.total_reward.is_zero());
    let observed = decode_rewards_block_distributions(&[RewardsStatsPeriodRlp {
        period: TARGET_PERIOD,
        data: current_stats.clone(),
    }])?
    .pop()
    .context("current BlockStats decoder returned no row")?;
    ensure!(observed.max_votes_weight <= u64::from(MAINNET_COMMITTEE_SIZE));
    let pbft = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(
        Rlp::new(&period_data).at(0)?.as_raw(),
    ))?;
    ensure!(pbft.period == TARGET_PERIOD && pbft.author == observed.block_author);

    let votes = reconstruct_cert_votes(&period_data)?;
    let (cert_facts, weighted_bundle, cert_report) = candidate_vote_weights(
        &votes,
        &observed.validators_stats,
        observed.total_votes_weight,
    )?;
    let transaction_facts = transaction_facts(&period_data, &receipts)?;
    let dag_facts = dag_facts(&period_data)?;
    let fact = FinalizedRewardsPeriodFact {
        period: TARGET_PERIOD,
        block_author: observed.block_author,
        blocks_per_year: observed.blocks_per_year,
        dpos_eligible_total_vote_count: observed.max_votes_weight,
        transactions: transaction_facts,
        dag_blocks: dag_facts,
        cert_votes: cert_facts,
    };
    let mut runtime = RewardsStatsRuntime::new(
        RewardsStatsConfig {
            committee_size: MAINNET_COMMITTEE_SIZE,
            magnolia_period: MAINNET_MAGNOLIA_PERIOD,
            aspen_part_one_period: MAINNET_ASPEN_ONE_PERIOD,
        },
        vec![RewardsFrequencyRule {
            from_period: MAINNET_REWARDS_FREQUENCY_FROM,
            frequency: MAINNET_REWARDS_FREQUENCY,
        }],
        Vec::new(),
    )?;
    let plan = runtime.process_period(fact);
    ensure!(plan.status == RewardsStatsStatus::Applied);
    let exact_current_block_stats_match = plan.current_block_stats_rlp == current_stats;
    ensure!(plan.cache_current_period && !plan.clear_cached_stats);
    ensure!(plan.distribution_stats.is_empty());
    let concrete_rewards_input = encode_concrete_rewards_input(&plan.distribution_stats);
    let candidate = decode_rewards_block_distributions(&[RewardsStatsPeriodRlp {
        period: TARGET_PERIOD,
        data: plan.current_block_stats_rlp.clone(),
    }])?
    .pop()
    .context("candidate BlockStats decoder returned no row")?;
    let retained_validator_order_hex = validator_order(&current_stats)?;
    let candidate_validator_order_hex = validator_order(&plan.current_block_stats_rlp)?;
    let typed_full_distribution_match = candidate == observed;

    let report = Report {
        schema: 1,
        tool_package_version: env!("CARGO_PKG_VERSION"),
        tool_source_sha256: sha256_hex(
            &[
                include_bytes!("reward_inputs.rs").as_slice(),
                include_bytes!("../reward_votes.rs"),
            ]
            .concat(),
        ),
        input_copy: input.display().to_string(),
        read_contract: ReadContract {
            mode: "DB::open_cf_descriptors_read_only",
            application_only: true,
            exact_key_reads: vec![
                "final_chain_meta[1]",
                "genesis[0]",
                "period_data[25706949]",
                "final_chain_receipt_by_period[25706949]",
                "final_chain_blk_by_number[25706949]",
                "final_chain_blk_hash_by_number[25706949]",
                "block_rewards_stats[25706949]",
            ],
            range_or_iterator_reads: false,
            application_column_family_count: columns.len(),
        },
        period: TARGET_PERIOD,
        rows: Rows {
            period_data: raw_fact(&period_data),
            receipts: raw_fact(&receipts),
            header: raw_fact(&header),
            current_block_stats: raw_fact(&current_stats),
            genesis_hash_hex: hex::encode(genesis),
            block_hash_hex: hex::encode(block_hash),
        },
        observed_current_stats: ObservedStats {
            block_author_hex: hex::encode(observed.block_author),
            blocks_per_year: observed.blocks_per_year,
            validator_rows: observed.validators_stats.len(),
            total_dag_blocks_count: observed.total_dag_blocks_count,
            total_votes_weight: observed.total_votes_weight,
            max_votes_weight: observed.max_votes_weight,
        },
        cert_vote_candidate: CertVoteCandidate {
            weighted_bundle_bytes: weighted_bundle.len(),
            weighted_bundle_sha256: sha256_hex(&weighted_bundle),
            ..cert_report
        },
        planner_candidate: PlannerCandidate {
            config_source: "checked-in mainnet config; target-period behavior candidate only",
            checked_in_mainnet_config_producer_qualified: false,
            capped_dpos_total_vote_projection: observed.max_votes_weight,
            exact_dpos_total_vote_count_qualified: false,
            exact_current_block_stats_match,
            cache_current_period: plan.cache_current_period,
            clear_cached_stats: plan.clear_cached_stats,
            distribution_stats_count: plan.distribution_stats.len(),
            concrete_rewards_input_hex: hex::encode(&concrete_rewards_input),
            concrete_rewards_input_sha256: sha256_hex(&concrete_rewards_input),
        },
        planner_comparison: PlannerComparison {
            candidate_row: raw_fact(&plan.current_block_stats_rlp),
            exact_rlp_bytes_match: exact_current_block_stats_match,
            typed_full_distribution_match,
            block_author_match: candidate.block_author == observed.block_author,
            blocks_per_year_match: candidate.blocks_per_year == observed.blocks_per_year,
            validators_stats_match: candidate.validators_stats == observed.validators_stats,
            total_dag_blocks_count_match: candidate.total_dag_blocks_count
                == observed.total_dag_blocks_count,
            total_votes_weight_match: candidate.total_votes_weight == observed.total_votes_weight,
            max_votes_weight_match: candidate.max_votes_weight == observed.max_votes_weight,
            validator_order_match: candidate_validator_order_hex == retained_validator_order_hex,
            retained_validator_order_hex,
            candidate_validator_order_hex,
            legacy_unordered_map_order_qualified: false,
        },
        qualification: Qualification {
            exact_retained_rows_match_recorded_hashes: true,
            captured_candidate_sidecar_reconstructed: true,
            candidate_planner_typed_fields_match_retained_row: typed_full_distribution_match,
            candidate_planner_exact_bytes_match_retained_row: exact_current_block_stats_match,
            historical_vrf_or_prior_vote_inputs_qualified: false,
            producer_compatible_reward_request_qualified: false,
            state_api_or_end_block_executed: false,
            reward_transition_or_root_qualified: false,
            producer_binary_or_global_hardfork_config_qualified: false,
            note: "vote weights and blocks_per_year come from the retained current BlockStats expected output; typed equality is circular for those fields and does not independently prove historical VRF validation, prior-state vote weights, other prior-state reward inputs, or legacy unordered-map serialization order",
        },
    };
    write_report(&output, &report)
}

fn candidate_vote_weights(
    votes: &ReconstructedVotes,
    stats: &BTreeMap<[u8; 20], rustaxa_consensus::RewardsValidatorDistribution>,
    expected_total: u64,
) -> Result<(Vec<RewardCertVoteFact>, Vec<u8>, CertVoteCandidate)> {
    let mut voters = BTreeSet::new();
    let mut facts = Vec::with_capacity(votes.canonical.len());
    let mut weighted = Vec::with_capacity(votes.canonical.len());
    let mut sum = 0_u64;
    for canonical in &votes.canonical {
        let inspection = inspect_canonical_pbft_vote(canonical)?;
        ensure!(inspection.signature_valid && inspection.period == votes.certified_period);
        let voter = inspection.recovered_voter.0;
        ensure!(voters.insert(voter), "duplicate reconstructed cert voter");
        let weight = stats
            .get(&voter)
            .map(|row| row.vote_weight)
            .filter(|weight| *weight > 0)
            .context("reconstructed cert voter has no observed BlockStats weight")?;
        sum = sum
            .checked_add(weight)
            .context("cert weight sum overflow")?;
        facts.push(RewardCertVoteFact {
            voter: inspection.recovered_voter,
            weight,
            period: inspection.period,
        });
        weighted.push(build_weighted_pbft_vote_payload(canonical, weight)?);
    }
    let observed_weight_voters = stats
        .iter()
        .filter_map(|(voter, row)| (row.vote_weight > 0).then_some(*voter))
        .collect::<BTreeSet<_>>();
    ensure!(voters == observed_weight_voters);
    ensure!(sum == expected_total);
    let bundle = build_weighted_pbft_vote_bundle(&weighted)?;
    Ok((
        facts,
        bundle,
        CertVoteCandidate {
            certified_period: votes.certified_period,
            round: votes.round,
            step: votes.step,
            count: votes.canonical.len(),
            unique_voters: true,
            every_voter_has_observed_nonzero_weight: true,
            no_unpaired_observed_nonzero_weights: true,
            observed_weights_sum_matches_stats: true,
            weighted_bundle_bytes: 0,
            weighted_bundle_sha256: String::new(),
            weight_source: "retained current BlockStats validator_stats.vote_weight expected output",
            independent_prior_state_weights_qualified: false,
        },
    ))
}

fn transaction_facts(period_data: &[u8], receipts: &[u8]) -> Result<Vec<RewardTransactionFact>> {
    let transactions = Rlp::new(period_data).at(3)?;
    let receipts = Rlp::new(receipts);
    ensure!(transactions.item_count()? == receipts.item_count()?);
    transactions
        .iter()
        .zip(receipts.iter())
        .map(|(transaction, receipt)| {
            let envelope = LegacyTransactionEnvelope::decode(transaction.as_raw())?;
            ensure!(receipt.item_count()? == 5);
            Ok(RewardTransactionFact {
                hash: envelope.hash,
                gas_price: envelope.gas_price,
                gas_used: FinalChainGas::new(receipt.val_at(1)?),
            })
        })
        .collect()
}

fn dag_facts(period_data: &[u8]) -> Result<Vec<RewardDagBlockFact>> {
    let dag_bundle = Rlp::new(period_data).at(2)?;
    if dag_bundle.is_empty() {
        return Ok(Vec::new());
    }
    let bundle = FinalizedDagBlockBundleRlp::new(dag_bundle.as_raw());
    (0..dag_bundle.at(2)?.item_count()?)
        .map(|position| {
            let canonical = bundle.canonical_block_rlp(position)?;
            let block = DagBlock::try_from(DagBlockRlp::new(&canonical))?;
            Ok(RewardDagBlockFact {
                author: block
                    .recover_sender()
                    .context("DAG author recovery failed")?,
                difficulty: Rlp::new(&block.vdf).val_at(3)?,
                transaction_hashes: block.transactions,
            })
        })
        .collect()
}

fn validated_paths() -> Result<(PathBuf, PathBuf)> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: reward_inputs QUALIFIED_COPY OUTPUT_JSON")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: reward_inputs QUALIFIED_COPY OUTPUT_JSON")?,
    );
    ensure!(args.next().is_none());
    let paths = rustaxa_snapshot_qualifier::paths::validate(&input, &output)?;
    Ok((paths.input, paths.output))
}

fn canonical_application_path(input: &Path) -> Result<PathBuf> {
    // Both DB children were checked together before any open.
    Ok(input.join("db/db"))
}

fn open_application_read_only(path: &Path) -> Result<(Database, Vec<String>)> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let columns = Database::list_cf(&options, path)?;
    for required in REQUIRED_APPLICATION_COLUMNS {
        ensure!(columns.iter().any(|column| column == required));
    }
    let descriptors = columns.iter().map(|name| {
        Column::from_name(name).map_or_else(
            |_| ColumnFamilyDescriptor::new(name, Options::default()),
            |column| column.descriptor(&Options::default()),
        )
    });
    Ok((
        Database::open_cf_descriptors_read_only(&options, path, descriptors, false)?,
        columns,
    ))
}

fn raw_cf(db: &Database, column: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
    let handle = db
        .cf_handle(column)
        .with_context(|| format!("missing column family {column}"))?;
    Ok(db.get_cf(&handle, key)?)
}

fn gate_row(bytes: &[u8], expected_bytes: usize, expected_sha256: &str, label: &str) -> Result<()> {
    ensure!(bytes.len() == expected_bytes, "{label} length mismatch");
    ensure!(
        sha256_hex(bytes) == expected_sha256,
        "{label} hash mismatch"
    );
    Ok(())
}

fn raw_fact(bytes: &[u8]) -> RawFact {
    RawFact {
        bytes: bytes.len(),
        sha256: sha256_hex(bytes),
    }
}

fn validator_order(block_stats: &[u8]) -> Result<Vec<String>> {
    let validators = Rlp::new(block_stats).at(2)?;
    validators
        .iter()
        .map(|entry| Ok(hex::encode(entry.val_at::<H160>(0)?)))
        .collect()
}

fn exact_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .try_into()
            .with_context(|| format!("{label} is not eight bytes"))?,
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn write_report(path: &Path, report: &Report) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut output, report)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H256;
    use rlp::RlpStream;

    #[test]
    fn optimized_bundle_reconstructs_canonical_vote_shape() {
        let proof = [0x22; 80];
        let signature = [0x33; 65];
        let mut optimized = RlpStream::new_list(1);
        optimized.begin_list(2);
        optimized.append(&proof.as_slice());
        optimized.append(&signature.as_slice());
        let mut bundle = RlpStream::new_list(5);
        bundle.append(&H256::repeat_byte(0x11));
        bundle.append(&9_u64);
        bundle.append(&2_u64);
        bundle.append(&3_u64);
        bundle.append_raw(&optimized.out(), 1);
        let mut period = RlpStream::new_list(4);
        period.append_empty_data();
        period.append_raw(&bundle.out(), 1);
        period.append_empty_data();
        period.begin_list(0);

        let decoded = reconstruct_cert_votes(&period.out()).unwrap();
        assert_eq!(decoded.certified_period, 9);
        assert_eq!(decoded.round, 2);
        assert_eq!(decoded.step, 3);
        let vote = Rlp::new(&decoded.canonical[0]);
        assert_eq!(vote.item_count().unwrap(), 3);
        assert_eq!(vote.val_at::<H256>(0).unwrap(), H256::repeat_byte(0x11));
        let sortition_bytes = vote.val_at::<Vec<u8>>(1).unwrap();
        let sortition = Rlp::new(&sortition_bytes);
        assert_eq!(sortition.val_at::<u64>(0).unwrap(), 9);
        assert_eq!(sortition.val_at::<u64>(1).unwrap(), 2);
        assert_eq!(sortition.val_at::<u64>(2).unwrap(), 3);
        assert_eq!(sortition.at(3).unwrap().data().unwrap(), proof);
        assert_eq!(vote.at(2).unwrap().data().unwrap(), signature);
    }

    #[test]
    fn row_gate_rejects_drift() {
        let bytes = b"bounded row";
        gate_row(bytes, bytes.len(), &sha256_hex(bytes), "test").unwrap();
        assert!(gate_row(bytes, bytes.len() + 1, &sha256_hex(bytes), "test").is_err());
        assert!(gate_row(bytes, bytes.len(), &"00".repeat(32), "test").is_err());
    }
}
