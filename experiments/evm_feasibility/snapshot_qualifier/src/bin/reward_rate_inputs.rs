//! Bounded independent annualized reward-rate diagnostic for the retained head.
//!
//! Shared filesystem and paired identity qualification precede any probe. One
//! repository predecessor lookup yields at most one persisted period-lambda value
//! at or before H; the repository does not expose its selected update key;
//! no inventory or live dynamic-lambda state is read. The checked-in mainnet
//! Cacti policy is a candidate policy, not producer configuration evidence.
//! Retained BlockStats is read only after deriving the independent input and
//! serves solely as an expected-output comparison. Reports are exclusive files.
use anyhow::{Context, Result, ensure};
use rustaxa_consensus::pbft_finalize::calc_blocks_per_year;
use rustaxa_consensus::{RewardsStatsPeriodRlp, decode_rewards_block_distributions};
use rustaxa_snapshot_qualifier::{paths, qualification};
use rustaxa_storage::MetadataRepository;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

const GENESIS: &[u8] = include_bytes!(
    "../../../../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json"
);
const STATS_SHA: &str = "18bb350916c3d85a52ec401cb2b898d2696b57e3859d5ff9dda1218a8ed05070";
const STATS_LEN: usize = 566;

/// Candidate policy decoded from checked-in JSON. Period lambda allows a
/// dynamic round-one value in the inclusive bounds OR the later-round default;
/// this union does not identify the finalized round or the producer's policy.
#[derive(Debug, Clone, Copy)]
struct Policy {
    activation: u64,
    min: u32,
    max: u32,
    default: u32,
    delay: u32,
}

impl Policy {
    /// Requires explicit integral fields, legal nonzero bounds, known delay and
    /// Cacti activation at H. Missing or malformed inputs never acquire defaults.
    fn decode(bytes: &[u8], head: u64) -> Result<Self> {
        let config: Value = serde_json::from_slice(bytes)?;
        let cacti = &config["hardforks"]["cacti_hf"];
        let field = |key: &str| {
            cacti[key]
                .as_u64()
                .with_context(|| format!("missing/inexact Cacti {key}"))
        };
        let narrow = |key: &str| {
            u32::try_from(field(key)?).with_context(|| format!("Cacti {key} exceeds u32"))
        };
        let policy = Self {
            activation: field("block_num")?,
            min: narrow("lambda_min")?,
            max: narrow("lambda_max")?,
            default: narrow("lambda_default")?,
            delay: narrow("consensus_delay")?,
        };
        ensure!(
            policy.activation > 0 && head >= policy.activation,
            "Cacti not active at target H"
        );
        ensure!(
            policy.min > 0 && policy.min <= policy.max && policy.default > 0,
            "illegal candidate lambda policy"
        );
        ensure!(
            policy.delay == 400,
            "candidate mainnet consensus delay drift"
        );
        Ok(policy)
    }

    fn accepts(self, lambda: u32) -> bool {
        (self.min..=self.max).contains(&lambda) || lambda == self.default
    }
}

/// Validates the repository-owned predecessor lookup's returned lambda.
/// Missing physical predecessor remains unavailable; no zero/default inference.
/// The existing repository exposes the value but not the selected update key.
fn predecessor_rate(lambda: Option<u32>, head: u64, policy: Policy) -> Result<Value> {
    let Some(lambda) = lambda else {
        return Ok(
            json!({"status": "unavailable", "reason": "no persisted PeriodLambda predecessor at or before H", "blocks_per_year": null}),
        );
    };
    ensure!(head >= policy.activation, "Cacti inactive at lookup bound");
    ensure!(
        policy.accepts(lambda),
        "PeriodLambda outside candidate dynamic bounds/default union"
    );
    let rate = calc_blocks_per_year(lambda, policy.delay)
        .context("annualized rate cannot fit u32 or has zero denominator")?;
    Ok(json!({
        "status": "available_under_candidate_policy", "source": "MetadataRepository::period_lambda(H,true); independent of BlockStats",
        "lookup_upper_bound_period": head, "lookup_upper_bound_key_le_hex": hex::encode(head.to_le_bytes()),
        "selected_update_period": null, "selected_raw_key": null,
        "provenance_limit": "repository returns decoded lambda only; selected update period/key and original value bytes are not exposed",
        "canonical_u32_le_encoding_hex": hex::encode(lambda.to_le_bytes()),
        "lambda_ms": lambda, "consensus_delay_ms": policy.delay, "blocks_per_year": rate,
        "finalized_round_qualified": false
    }))
}

/// Pins the retained expected output before decoding its rate; it supplies no
/// input to predecessor decoding or the annualization calculation.
fn compare_stats(raw: &[u8], independent: &Value) -> Result<Value> {
    ensure!(
        raw.len() == STATS_LEN && hex::encode(Sha256::digest(raw)) == STATS_SHA,
        "retained H BlockStats fingerprint drift"
    );
    let stats = decode_rewards_block_distributions(&[RewardsStatsPeriodRlp {
        period: qualification::HEAD,
        data: raw.to_vec(),
    }])?
    .pop()
    .context("retained BlockStats decoded no row")?;
    let computed = independent["blocks_per_year"].as_u64();
    Ok(json!({
        "period": stats.period, "raw_bytes": raw.len(), "raw_sha256": STATS_SHA,
        "observed_blocks_per_year": stats.blocks_per_year,
        "independent_blocks_per_year": computed,
        "matches": computed.map(|value| value == u64::from(stats.blocks_per_year)),
        "purpose": "expected-output comparison only; existing full raw/typed parity evidence remains unchanged"
    }))
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: reward_rate_inputs COPY OUTPUT")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: reward_rate_inputs COPY OUTPUT")?,
    );
    ensure!(args.next().is_none(), "unexpected argument");
    let paths = paths::validate(&input, &output)?;
    let policy = Policy::decode(GENESIS, qualification::HEAD)?;
    let (_readers, _identity, pair) = qualification::qualify(&paths.application, &paths.state)?;
    let application = qualification::open_application(&paths.application)?;
    let lambda =
        MetadataRepository::new(application.clone()).period_lambda(qualification::HEAD, true)?;
    let independent = predecessor_rate(lambda, qualification::HEAD, policy)?;
    let column = application
        .cf_handle("block_rewards_stats")
        .context("missing BlockStats column")?;
    let raw = application
        .get_cf(&column, qualification::HEAD.to_le_bytes())?
        .context("missing retained H BlockStats")?;
    let comparison = compare_stats(&raw, &independent)?;
    let report = json!({
        "input_copy": paths.input, "pair_qualification": pair,
        "tool_source_sha256": tool_source_sha256(),
        "candidate_policy": {
            "source": "libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json",
            "source_sha256": hex::encode(Sha256::digest(GENESIS)),
            "cacti_activation": policy.activation, "lambda_min_ms": policy.min,
            "lambda_max_ms": policy.max, "later_round_lambda_default_ms": policy.default,
            "consensus_delay_ms": policy.delay, "cacti_active_at_h": true,
            "legal_lambda": "inclusive [lambda_min,lambda_max] OR lambda_default; no finalized-round inference",
            "producer_configuration_qualified": false
        },
        "read_bound": {
            "pair_application_point_reads": 4, "concrete_reads": "existing descriptor/current/prior-root qualification only",
            "period_lambda_reverse_seeks": 1, "maximum_period_lambda_rows_returned": 1,
            "block_stats_point_reads": 1, "application_logical_reads_total": 6,
            "broad_scan_performed": false, "live_dynamic_lambda_read": false
        },
        "independent_rate_input": independent, "retained_output_comparison": comparison,
        "source_semantics": {
            "formula": "libraries/config/src/genesis.cpp:103; 365-day year milliseconds / (2*lambda_ms + delay_ms)",
            "period_lambda_persistence": "libraries/core_libs/consensus/src/pbft/pbft_manager.cpp:2080-2100; period lambda saved before dynamic update",
            "round_lambda_policy": "libraries/core_libs/consensus/src/pbft/pbft_manager.cpp:507; dynamic round-one lambda, default for later rounds",
            "rust_calculator": "rust/crates/rustaxa-consensus/src/pbft_finalize.rs::calc_blocks_per_year; existing checked u64 arithmetic"
        },
        "qualification": {
            "independent_period_lambda_available": independent["blocks_per_year"].is_number(),
            "producer_configuration_qualified": false, "producer_binary_qualified": false,
            "live_dynamic_lambda_state_qualified": false, "historical_vote_weights_qualified": false,
            "reward_transition_or_root_qualified": false, "adoption_authorized": false
        }
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
        include_bytes!("reward_rate_inputs.rs").as_slice(),
        include_bytes!("../paths.rs"),
        include_bytes!("../qualification.rs"),
        include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/pbft_finalize.rs"),
    ] {
        digest.update(source);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_policy_requires_activation_widths_and_explicit_fields() {
        let policy = Policy::decode(GENESIS, qualification::HEAD).unwrap();
        assert!(Policy::decode(GENESIS, policy.activation - 1).is_err());
        assert!(Policy::decode(b"{}", qualification::HEAD).is_err());
        let mut config: Value = serde_json::from_slice(GENESIS).unwrap();
        for (field, value) in [
            ("consensus_delay", json!(401)),
            ("lambda_min", json!(0)),
            ("lambda_max", json!(u64::from(u32::MAX) + 1)),
            ("lambda_default", Value::Null),
        ] {
            let original = config["hardforks"]["cacti_hf"][field].clone();
            config["hardforks"]["cacti_hf"][field] = value;
            assert!(
                Policy::decode(&serde_json::to_vec(&config).unwrap(), qualification::HEAD).is_err()
            );
            config["hardforks"]["cacti_hf"][field] = original;
        }
    }

    #[test]
    fn missing_lambda_is_unavailable_without_default() {
        let result = predecessor_rate(
            None,
            qualification::HEAD,
            Policy::decode(GENESIS, qualification::HEAD).unwrap(),
        )
        .unwrap();
        assert_eq!(result["status"], "unavailable");
        assert!(result["blocks_per_year"].is_null());
    }

    #[test]
    fn predecessor_reports_value_only_and_rejects_candidate_policy_drift() {
        let policy = Policy::decode(GENESIS, qualification::HEAD).unwrap();
        for lambda in [policy.min, policy.max, policy.default] {
            let result = predecessor_rate(Some(lambda), qualification::HEAD, policy).unwrap();
            assert!(result["selected_update_period"].is_null());
            assert!(result["selected_raw_key"].is_null());
            assert_eq!(
                result["canonical_u32_le_encoding_hex"],
                hex::encode(lambda.to_le_bytes())
            );
            assert_eq!(
                result["blocks_per_year"].as_u64(),
                calc_blocks_per_year(lambda, policy.delay).map(u64::from)
            );
        }
        for lambda in [0, 499, 1501, 1999, 2001, u32::MAX] {
            assert!(predecessor_rate(Some(lambda), qualification::HEAD, policy).is_err());
        }
        assert!(predecessor_rate(Some(500), policy.activation - 1, policy).is_err());
    }

    #[test]
    fn retained_output_drift_rejected_before_decode() {
        assert!(compare_stats(&[0; STATS_LEN], &json!({"blocks_per_year": 1})).is_err());
        assert!(compare_stats(&[], &json!({"blocks_per_year": null})).is_err());
    }
}
