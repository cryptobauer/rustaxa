//! Conditional legacy native-effects witness at the retained head.
//!
//! Pinned historical transaction execution, retained metadata and independent
//! reward planning are prerequisites. Three owner calls at the authenticated
//! parent root prove positive DPoS code size and the exact empty jailed list.
//! The witness applies a source-derived conditional argument and a Rust system
//! planner; it never executes Rust EndBlock or reconstructs complete native
//! state. Missing dependencies and proof mismatches cannot become empty effects.
use anyhow::{Context, Result, ensure};
use rustaxa_consensus::final_chain_execution::{
    FinalChainSystemTransactionPlanFact, plan_external_evm_system_transactions,
};
use rustaxa_snapshot_qualifier::{paths, qualification};
use rustaxa_storage::ConcreteStoragePath;
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce,
    concrete_state::{ConcreteRead, ConcreteStateIdentity, ConcreteStorageKey},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{env, fs::OpenOptions, io::Write, path::PathBuf};

const H: u64 = qualification::HEAD;
const PRIOR_ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const HEAD_ROOT: &str = "b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227";
const PREFLIGHT: &[u8] = include_bytes!("../../../../../doc/evm_research/n4_replay_preflight.json");
const PREFLIGHT_SHA: &str = "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437";
const PREFLIGHT_SOURCE_SHA: &str =
    "0cbcc3c97cea6eed0784be84fafbf401a43ceb6ae2e3bfc647ccde2c386ee66c";
const CONFIG: &[u8] =
    include_bytes!("../../../../../doc/evm_research/n4_retained_dpos_config.json");
const CONFIG_SHA: &str = "3c817ff7974da68e0d5865b34e92765c7e50c71a6892c81d299f28c579e38634";
const CANDIDATE: &[u8] = include_bytes!(
    "../../../../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json"
);
const CANDIDATE_SHA: &str = "7931f2b9b92e018dad154becbf20058d2f97f8ad5d9cc0d4b443da7c72ae3ce7";
const REWARD_REPORT: &[u8] =
    include_bytes!("../../../../../doc/evm_research/n4_independent_reward_plan.json");
const REWARD_SHA: &str = "f88d67ecbd06d6c483466290262fabe8fb03e4532164cf0311fdd63232bcb0e9";

fn pinned(bytes: &[u8], expected: &str) -> Result<Value> {
    ensure!(
        hex::encode(Sha256::digest(bytes)) == expected,
        "embedded artifact fingerprint drift/unavailable pin"
    );
    Ok(serde_json::from_slice(bytes)?)
}

/// Validates reused historical ordinary-transfer evidence, including exact root
/// identities and no-code/native classification. Signatures are not re-executed.
fn preflight_gate(report: &Value) -> Result<()> {
    ensure!(
        report["tool_source_sha256"] == PREFLIGHT_SOURCE_SHA
            && report["period"] == H
            && report["prior_period"] == H - 1
            && report["prior_root_hex"] == PRIOR_ROOT,
        "historical preflight identity drift"
    );
    let classes = &report["envelope_classification"];
    ensure!(
        classes["expected_count"] == 19 && classes["exact_count"] == 19,
        "ordinary transaction count drift"
    );
    for field in [
        "every_signature_decoded",
        "every_chain_id_841",
        "every_call",
        "every_input_empty",
        "every_receiver_non_native",
        "every_receiver_has_no_code",
    ] {
        ensure!(
            classes[field] == true,
            "ordinary transfer classification drift: {field}"
        );
    }
    let entries = classes["entries"]
        .as_array()
        .context("missing transaction entries")?;
    ensure!(entries.len() == 19, "ordinary transfer entries count drift");
    for (position, entry) in entries.iter().enumerate() {
        ensure!(
            entry["position"] == position
                && entry["kind"] == "call"
                && entry["input_hex"] == ""
                && entry["chain_id"] == 841
                && entry["gas_limit"] == 21000,
            "ordinary transfer entry drift"
        );
    }
    ensure!(
        report["execution_context"]["block_context_observed_by_bytecode"] == false
            && report["prior_state_dependencies"]["code"]
                .as_array()
                .is_some_and(Vec::is_empty)
            && report["prior_state_dependencies"]["slots"]
                .as_array()
                .is_some_and(Vec::is_empty),
        "historical preflight has code/storage dependencies"
    );
    let preparation = &report["transaction_root_preparation"];
    ensure!(
        preparation["derived_root_hex"] == HEAD_ROOT
            && preparation["retained_head_root_hex"] == HEAD_ROOT
            && preparation["roots_match"] == true
            && preparation["database_mutated"] == false
            && preparation["includes_rewards"] == false,
        "historical transaction-root witness drift"
    );
    for field in [
        "exact_ordered_ordinary_execution",
        "demanded_prior_state_reads_authenticated",
        "exact_receipts_reproduced",
        "touched_read_closure_qualified",
        "transaction_only_state_root_reproduced",
    ] {
        ensure!(
            report["qualification"][field] == true,
            "historical execution qualification drift"
        );
    }
    Ok(())
}

/// Requires the sole retained baseline, full bounded metadata exhaustion and
/// delay five, corroborating candidate delay at H as well as the earlier Q.
fn config_gate(report: &Value) -> Result<()> {
    ensure!(
        report["pair_qualification"]["head"] == H
            && report["pair_qualification"]["state_root_hex"] == HEAD_ROOT,
        "retained config identity drift"
    );
    let config = &report["retained_dpos_configuration"];
    ensure!(
        config["exhausted_within_caps"] == true
            && config["record_count"] == 1
            && config["updates_numeric_order"]
                .as_array()
                .is_some_and(|rows| rows.len() == 1)
            && config["selected"]["update_period"] == 0
            && config["selected"]["raw_key_hex"] == "",
        "sole baseline DPoS metadata required"
    );
    ensure!(
        config["selected"]["full_record_codec_validated"] == true
            && config["selected"]["scalar_fields"]["delegation_delay"]["unsigned_decimal"] == "5"
            && config["candidate_comparison"]["candidate_source_sha256"] == CANDIDATE_SHA
            && config["candidate_comparison"]["fields"]["delegation_delay"]["matches"] == true,
        "retained delay/candidate provenance drift"
    );
    Ok(())
}

/// Requires the independently populated planner and its noncircular artifact
/// provenance, while binding fresh application row identities to the historical
/// transfer witness. Header total_reward is never a substitute for planning.
fn reward_gate(report: &Value, preflight: &Value) -> Result<()> {
    let planner = &report["planner_candidate"];
    ensure!(
        report["period"] == H
            && planner["cache_current_period"] == true
            && planner["clear_cached_stats"] == false
            && planner["distribution_stats_count"] == 0
            && planner["concrete_rewards_input_hex"] == "c0",
        "independent empty reward planning prerequisite unavailable"
    );
    let inputs = &report["independent_artifacts"];
    ensure!(
        inputs["mode"] == "independent_artifacts"
            && inputs["expected_stats_used_for_weight_rate_total_or_author"] == false
            && inputs["retained_config_artifact_sha256"] == CONFIG_SHA
            && inputs["candidate_config_sha256"] == CANDIDATE_SHA,
        "reward planner noncircular artifact provenance drift"
    );
    ensure!(
        inputs["certificate_period"] == H - 1
            && inputs["request_period"] == H - 2
            && inputs["storage_and_eligibility_period"] == H - 7
            && inputs["independent_total_vote_count"] == 534879
            && inputs["independent_blocks_per_year"] == 9275294,
        "independent reward input identity drift"
    );
    ensure!(
        inputs["historical_source_commit"] == "e63be522d"
            && inputs["vote_artifact_sha256"]
                == "719d1cf2c06f9e1390584d97c8b033b9cf62d34cec64250a367c3a6f824d1132"
            && inputs["rate_artifact_sha256"]
                == "b4f3bbe26e680722a5171a2d68a58dcb990cff5f12221cd5dc2f5478f3b5b8d5"
            && inputs["delayed_state_root_hex"]
                == "dc4c6771a9ed227db8179b68783eefc78bb0f6a443c573d71457c8d516f8d041"
            && inputs["fresh_concrete_authentication_performed"] == false
            && inputs["producer_configuration_qualified"] == false
            && inputs["historical_cache_closure_qualified"] == false,
        "independent historical source/provenance drift"
    );
    ensure!(
        report["planner_comparison"]["typed_full_distribution_match"] == true,
        "independent planner typed comparison failed"
    );
    ensure!(
        report["rows"]["header"]["sha256"]
            == "1bebb6391e97caa4e8d34db4336810ec71eae83ef975d48bfb05e0b56e81363f"
            && report["rows"]["period_data"]["sha256"]
                == "bcb4b16bed154c41156ed87f6d020f5ed7fad96f0c7562af778aeefd474de11b"
            && report["rows"]["receipts"]["sha256"] == preflight["receipt_row_sha256"],
        "fresh planner application rows differ from historical transfer witness"
    );
    Ok(())
}

/// Computes the candidate non-pillar schedule and invokes the existing Rust
/// planner. All unread bridge/nonce facts are explicitly dead-branch placeholders.
fn system_plan(candidate: &Value) -> Result<Value> {
    let forks = &candidate["hardforks"];
    ensure!(
        forks["ficus_hf"]["block_num"] == 11_616_000
            && forks["ficus_hf"]["pillar_blocks_interval"] == 4000
            && forks["fix_redelegate_block_num"] == 3_091_000
            && forks["cornus_hf"]["block_num"] == 15_610_000
            && forks["aspen_hf"]["block_num_part_one"] == 8_118_000,
        "candidate native hardfork policy drift"
    );
    ensure!(
        H >= 11_616_000 && H != 3_091_000 && H != 15_610_000 && H >= 8_118_000,
        "candidate exceptional native transition period"
    );
    let scheduled = H.checked_add(5).context("system schedule overflow")?;
    let remainder = scheduled % 4000;
    ensure!(
        remainder == 2954,
        "candidate schedule is not expected non-pillar branch"
    );
    let plan = plan_external_evm_system_transactions(FinalChainSystemTransactionPlanFact {
        request_id: [0; 32],
        period: FinalChainBlockNumber::new(H),
        is_pillar_block_period: false,
        bridge_contract_address: [0; 20],
        bridge_contract_found: false,
        bridge_contract_has_code: false,
        should_finalize_epoch: false,
        system_account_nonce: FinalChainNonce::default(),
        block_gas_limit: FinalChainGas::new(0),
    })?;
    ensure!(
        plan.transactions.is_empty(),
        "non-pillar Rust system planner produced transactions"
    );
    Ok(
        json!({"candidate_schedule_period": scheduled, "pillar_interval": 4000, "remainder": remainder, "system_transaction_count": plan.transactions.len(), "planner_called": true, "bridge_address_presence_code_epoch_nonce_and_gas": "UNOBSERVED dead-branch placeholders; no account absence claim", "system_transactions_executed": false}),
    )
}

/// Requires exact authenticated Member(c0) and matching physical bytes. Neither
/// nonmembership, missing physical history nor tombstones qualify this witness.
fn empty_jail_gate(physical: &ConcreteRead<Vec<u8>>, proof: &ConcreteStoragePath) -> Result<()> {
    ensure!(
        matches!((physical, proof), (ConcreteRead::Present(raw), ConcreteStoragePath::Member(member)) if raw.as_slice() == [0xc0] && raw == member),
        "jailed-list target must be physical/authenticated Member(c0)"
    );
    Ok(())
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: empty_native_effects COPY OUTPUT")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: empty_native_effects COPY OUTPUT")?,
    );
    ensure!(args.next().is_none(), "unexpected argument");
    let paths = paths::validate(&input, &output)?;
    let preflight = pinned(PREFLIGHT, PREFLIGHT_SHA)?;
    preflight_gate(&preflight)?;
    let config = pinned(CONFIG, CONFIG_SHA)?;
    config_gate(&config)?;
    let reward = pinned(REWARD_REPORT, REWARD_SHA)?;
    reward_gate(&reward, &preflight)?;
    for artifact in [&config, &reward] {
        ensure!(
            artifact["input_copy"] == paths.input.display().to_string(),
            "current artifact belongs to a different working copy"
        );
    }
    let system = system_plan(&pinned(CANDIDATE, CANDIDATE_SHA)?)?;
    let (readers, _identity, pair) = qualification::qualify(&paths.application, &paths.state)?;
    let prior = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(H - 1),
        state_root: hex::decode(PRIOR_ROOT)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("prior root width"))?,
    };
    let mut dpos = [0_u8; 20];
    dpos[19] = 0xfe;
    let mut slashing = [0_u8; 20];
    slashing[19] = 0xee;
    let mut key = [0_u8; 32];
    key[31] = 2;
    let account = readers.account_at(prior, dpos);
    let physical = readers.storage_at(prior, slashing, ConcreteStorageKey(key));
    let proof = readers.verify_storage_path_at(prior, slashing, ConcreteStorageKey(key));
    let account_ok =
        matches!(&account, Ok(ConcreteRead::Present(record)) if record.account.code_size > 0);
    let account_code_size = match &account {
        Ok(ConcreteRead::Present(record)) => Some(record.account.code_size),
        _ => None,
    };
    let account_physical_rlp_hex = match &account {
        Ok(ConcreteRead::Present(record)) => Some(hex::encode(&record.physical_rlp)),
        _ => None,
    };
    let jail_ok = match (&physical, &proof) {
        (Ok(raw), Ok(path)) => empty_jail_gate(raw, path).is_ok(),
        _ => false,
    };
    let report = json!({
        "input_copy": paths.input, "pair_qualification": pair, "period": H,
        "prior_identity": {"period": H - 1, "state_root_hex": PRIOR_ROOT}, "tool_source_sha256": tool_source_sha256(),
        "pinned_evidence": {"historical_preflight_sha256": PREFLIGHT_SHA, "historical_preflight_source_sha256": PREFLIGHT_SOURCE_SHA, "retained_dpos_config_sha256": CONFIG_SHA, "candidate_mainnet_sha256": CANDIDATE_SHA, "independent_reward_planner_sha256": REWARD_SHA, "transaction_replay_performed": false, "historical_preflight_input_path": preflight["input_copy"], "reuse_scope": "historical transfer evidence and independently observed config/planner reports reused; this command freshly reads only the paired qualification and three native-owner targets"},
        "read_bound": {"logical_targets": 2, "owner_top_level_calls": 3, "account_at_calls": 1, "storage_at_calls": 1, "verify_storage_path_at_calls": 1, "additional_reads": "fixed pair qualification plus existing owner-internal root/path authentication", "separate_code_nonce_supply_counter_other_jail_inventory_calls": false, "broad_scan_performed": false},
        "parent_dpos_account": {"address_hex": hex::encode(dpos), "positive_code_size_required": true, "qualified": account_ok, "code_size": account_code_size, "physical_rlp_hex": account_physical_rlp_hex, "outcome": format!("{account:?}")},
        "parent_jailed_list": {"address_hex": hex::encode(slashing), "key_hex": hex::encode(key), "required_value_hex": "c0", "qualified": jail_ok, "physical_outcome": format!("{physical:?}"), "authenticated_outcome": format!("{proof:?}")},
        "system_planning": system,
        "conditional_source_argument": {"immutable_go_revision": "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b", "contract": "doc/evm_research/n4_empty_native_effects_contract.md",
            "immutable_source_sha256": {"taraxa/C/state.go": "b6effa29e1d465a0bcc1badcf398a57e982d16b0d6f4f9b14844632306f7e3ec", "taraxa/state/state_transition/state_transition.go": "d447ac4599f0107ccad4379f95a7269a6d5729283d8dcf00db850ea8fe119e90", "taraxa/state/state_transition/state_hardforks.go": "1432c0e39e7ecaee8683c3e9fd28571e97f816e906c3938b0f38fa886c246aae", "taraxa/state/contracts/dpos/precompiled/dpos_contract.go": "28bb58adb9d99d604fdacf0c284deea49c6887eee789381a5c5a231b5c89f565", "taraxa/state/contracts/slashing/precompiled/slashing_contract.go": "2ac21dbe9ece16e11f1ab85d2670488d24a2a0e8940d2c181ced142511c6edfb"}, "begin_block": "positive parent DPoS code excludes Aspen install; H differs from exact Cornus and redelegation correction heights", "slashing_initialization": "memory registration only; nonce/storage changes occur only on jail mutation", "empty_rewards": "independent planner distributions empty; EndBlock still required", "end_block": "ordinary transfers and empty rewards leave deferred DPoS counters untouched; empty parent jail list yields no cleanup durable effects for cold or warm timer"},
        "qualification": {"conditional_legacy_no_additional_durable_effects_witness": account_ok && jail_ok, "partial_required_native_probes": !(account_ok && jail_ok), "actual_rust_end_block_executed": false, "complete_dpos_snapshot_reconstructed": false, "producer_configuration_qualified": false, "producer_binary_qualified": false, "reward_transition_or_root_qualified": false, "final_state_root_qualified": false, "full_snapshot_qualified": false, "adoption_authorized": false, "production_routing_qualified": false}
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    ensure!(
        account_ok && jail_ok,
        "required native probes failed; partial report written"
    );
    Ok(())
}

fn tool_source_sha256() -> String {
    let mut digest = Sha256::new();
    for source in [
        include_bytes!("empty_native_effects.rs").as_slice(),
        include_bytes!("../paths.rs"),
        include_bytes!("../qualification.rs"),
        include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/final_chain_execution.rs"),
    ] {
        digest.update(source);
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_pins_and_historical_transfer_gates_reject_drift() {
        let mut preflight = pinned(PREFLIGHT, PREFLIGHT_SHA).unwrap();
        preflight_gate(&preflight).unwrap();
        assert!(pinned(PREFLIGHT, CONFIG_SHA).is_err());
        preflight["envelope_classification"]["every_receiver_has_no_code"] = json!(false);
        assert!(preflight_gate(&preflight).is_err());
        let mut config = pinned(CONFIG, CONFIG_SHA).unwrap();
        config_gate(&config).unwrap();
        config["retained_dpos_configuration"]["record_count"] = json!(2);
        assert!(config_gate(&config).is_err());
    }
    #[test]
    fn authenticated_empty_jail_is_the_only_accepted_storage_outcome() {
        let raw = ConcreteRead::Present(vec![0xc0]);
        let member = ConcreteStoragePath::Member(vec![0xc0]);
        empty_jail_gate(&raw, &member).unwrap();
        for physical in [
            ConcreteRead::Absent,
            ConcreteRead::Tombstone,
            ConcreteRead::Present(vec![]),
            ConcreteRead::Present(vec![0xc1, 0x80]),
        ] {
            assert!(empty_jail_gate(&physical, &member).is_err());
        }
        assert!(empty_jail_gate(&raw, &ConcreteStoragePath::NonMember).is_err());
        assert!(empty_jail_gate(&raw, &ConcreteStoragePath::Member(vec![0x80])).is_err());
    }
    #[test]
    fn non_pillar_system_branch_requires_candidate_policy_without_bridge_observations() {
        let mut candidate = pinned(CANDIDATE, CANDIDATE_SHA).unwrap();
        assert_eq!(
            system_plan(&candidate).unwrap()["system_transaction_count"],
            0
        );
        candidate["hardforks"]["ficus_hf"]["pillar_blocks_interval"] = json!(2954);
        assert!(system_plan(&candidate).is_err());
    }
    #[test]
    fn frozen_reward_plan_accepts_empty_cache_and_rejects_distribution_or_cache_drift() {
        let preflight = pinned(PREFLIGHT, PREFLIGHT_SHA).unwrap();
        let mut reward = pinned(REWARD_REPORT, REWARD_SHA).unwrap();
        reward_gate(&reward, &preflight).unwrap();
        reward["planner_candidate"]["distribution_stats_count"] = json!(1);
        assert!(reward_gate(&reward, &preflight).is_err());
        reward["planner_candidate"]["distribution_stats_count"] = json!(0);
        reward["planner_candidate"]["cache_current_period"] = json!(false);
        assert!(reward_gate(&reward, &preflight).is_err());
    }

    #[test]
    fn missing_or_circular_reward_planning_cannot_become_empty_effects() {
        let preflight = pinned(PREFLIGHT, PREFLIGHT_SHA).unwrap();
        assert!(reward_gate(&json!({}), &preflight).is_err());
        let report = json!({"period": H, "planner_candidate": {"cache_current_period": true, "clear_cached_stats": false, "distribution_stats_count": 0, "concrete_rewards_input_hex": "c0"}, "independent_artifacts": {"mode": "independent_artifacts", "expected_stats_used_for_weight_rate_total_or_author": true}});
        assert!(reward_gate(&report, &preflight).is_err());
    }
}
