//! Exact historical-artifact inputs for the bounded independent planner mode.
//! Byte pins and structural identity checks precede fact extraction. Only
//! independently derived weights, total and rate enter planner inputs; expected
//! comparison fields remain unread. This module opens no database and makes no
//! fresh-state-authentication or producer-configuration claim.
use anyhow::{Context, Result, ensure};
use rustaxa_consensus::{
    PbftCanonicalVoteInspection, RewardCertVoteFact, inspect_canonical_pbft_vote,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

const VOTES: &[u8] =
    include_bytes!("../../../../../../doc/evm_research/n4_independent_reward_vote_inputs.json");
const RATE: &[u8] = include_bytes!("../../../../../../doc/evm_research/n4_reward_rate_inputs.json");
const CONFIG: &[u8] =
    include_bytes!("../../../../../../doc/evm_research/n4_retained_dpos_config.json");
const CANDIDATE_CONFIG: &[u8] = include_bytes!(
    "../../../../../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json"
);
const VOTES_SHA: &str = "719d1cf2c06f9e1390584d97c8b033b9cf62d34cec64250a367c3a6f824d1132";
const RATE_SHA: &str = "b4f3bbe26e680722a5171a2d68a58dcb990cff5f12221cd5dc2f5478f3b5b8d5";
const CONFIG_SHA: &str = "3c817ff7974da68e0d5865b34e92765c7e50c71a6892c81d299f28c579e38634";
const CANDIDATE_SHA: &str = "7931f2b9b92e018dad154becbf20058d2f97f8ad5d9cc0d4b443da7c72ae3ce7";
const H: u64 = 25_706_949;
const P: u64 = H - 1;
const Q: u64 = H - 2;
const D: u64 = H - 7;
const ROOT: &str = "b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227";
const PRIOR_ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const HEADER_SHA: &str = "1bebb6391e97caa4e8d34db4336810ec71eae83ef975d48bfb05e0b56e81363f";
const DELAYED_ROOT: &str = "dc4c6771a9ed227db8179b68783eefc78bb0f6a443c573d71457c8d516f8d041";
const DELAYED_HEADER_SHA: &str = "52ca059f6c683387df49ad47a04e91501a94cba892481af0a7eeb4af04bf7786";
const CERT_BLOCK_HASH: &str = "e0922e40b22925af2a1e9360c566a6fbdd517adafd5500183fc5287e48613589";
const GENESIS: &str = "8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377";

/// Selected inputs, with no expected-output fields exposed. Weights are bound
/// to fresh canonical vote inspection before they can become planner facts.
pub(super) struct IndependentInputs {
    pub blocks_per_year: u32,
    pub total_vote_count: u64,
    weights: BTreeMap<[u8; 20], u64>,
    pub evidence: IndependentEvidence,
}

/// Provenance retained by the report. Exact historical state evidence is reused;
/// the seven fresh application reads do not reauthenticate concrete state.
#[derive(Serialize)]
pub(super) struct IndependentEvidence {
    pub mode: &'static str,
    pub historical_source_commit: &'static str,
    pub vote_artifact_sha256: &'static str,
    pub rate_artifact_sha256: &'static str,
    pub retained_config_artifact_sha256: &'static str,
    pub candidate_config_sha256: &'static str,
    pub certificate_period: u64,
    pub request_period: u64,
    pub storage_and_eligibility_period: u64,
    pub delayed_state_root_hex: String,
    pub independent_total_vote_count: u64,
    pub independent_blocks_per_year: u32,
    pub expected_stats_used_for_weight_rate_total_or_author: bool,
    pub fresh_concrete_authentication_performed: bool,
    pub producer_configuration_qualified: bool,
    pub historical_cache_closure_qualified: bool,
}

/// Validates the exact embedded reports, candidate bytes and supplied fresh
/// application identity. Returns only independent selected fact inputs.
pub(super) fn load(input: &Path, current_root: &[u8], header: &[u8]) -> Result<IndependentInputs> {
    let votes = pinned_json(VOTES, VOTES_SHA)?;
    let rate = pinned_json(RATE, RATE_SHA)?;
    let config = pinned_json(CONFIG, CONFIG_SHA)?;
    ensure!(
        sha(CANDIDATE_CONFIG) == CANDIDATE_SHA,
        "candidate config byte identity differs"
    );
    ensure!(
        hex::encode(current_root) == ROOT && sha(header) == HEADER_SHA,
        "fresh application identity differs from artifacts"
    );
    derive(&votes, &rate, &config, &input.display().to_string())
}

fn pinned_json(bytes: &[u8], expected: &str) -> Result<Value> {
    ensure!(
        sha(bytes) == expected,
        "historical artifact byte identity differs"
    );
    Ok(serde_json::from_slice(bytes)?)
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn scalar(value: &Value, label: &str) -> Result<u64> {
    value.as_u64().with_context(|| format!("missing {label}"))
}
fn text<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    value.as_str().with_context(|| format!("missing {label}"))
}

fn bind_pair(pair: &Value) -> Result<()> {
    ensure!(
        scalar(&pair["head"], "pair head")? == H
            && scalar(&pair["prior_period"], "prior period")? == P,
        "mixed pair period"
    );
    ensure!(
        text(&pair["state_root_hex"], "root")? == ROOT
            && text(&pair["prior_state_root_hex"], "prior root")? == PRIOR_ROOT,
        "mixed pair root"
    );
    ensure!(
        text(&pair["genesis_hash_hex"], "genesis")? == GENESIS
            && text(&pair["header_sha256"], "header hash")? == HEADER_SHA,
        "mixed pair network/header"
    );
    ensure!(
        pair["descriptor_matches_header"] == true
            && pair["current_and_prior_roots_authenticated"] == true,
        "incomplete historical pair"
    );
    Ok(())
}

fn derive(votes: &Value, rate: &Value, config: &Value, input: &str) -> Result<IndependentInputs> {
    for artifact in [votes, rate, config] {
        ensure!(
            text(&artifact["input_copy"], "input copy")? == input,
            "mixed artifact copy identity"
        );
    }
    bind_pair(&votes["details"]["pair"])?;
    bind_pair(&rate["pair_qualification"])?;
    bind_pair(&config["pair_qualification"])?;
    ensure!(
        votes["status"] == "candidate_reconstructed" && votes["error"].is_null(),
        "vote artifact failed its gate"
    );
    ensure!(
        scalar(&votes["head"], "head")? == H
            && scalar(&votes["certificate_period"], "P")? == P
            && scalar(&votes["request_period"], "Q")? == Q
            && scalar(&votes["storage_and_eligibility_period"], "D")? == D
            && scalar(&votes["candidate_delegation_delay"], "delay")? == 5,
        "mixed vote period/delay"
    );
    ensure!(
        scalar(
            &votes["details"]["certificate"]["period"],
            "certificate period"
        )? == P
            && scalar(&votes["details"]["certificate"]["round"], "round")? == 1
            && scalar(&votes["details"]["certificate"]["step"], "step")? == 3,
        "mixed certificate identity"
    );
    ensure!(
        scalar(
            &votes["details"]["delayed_identity"]["period"],
            "delayed root period"
        )? == D,
        "mixed delayed root period"
    );
    let delayed_root = text(
        &votes["details"]["delayed_identity"]["state_root_hex"],
        "delayed root",
    )?;
    ensure!(
        delayed_root == DELAYED_ROOT
            && votes["details"]["delayed_identity"]["application_header_sha256"]
                == DELAYED_HEADER_SHA,
        "mixed delayed root/header"
    );
    ensure!(
        votes["details"]["certificate"]["block_hash_hex"] == CERT_BLOCK_HASH
            && votes["details"]["certificate"]["period_data_sha256"]
                == "bcb4b16bed154c41156ed87f6d020f5ed7fad96f0c7562af778aeefd474de11b",
        "mixed certificate source"
    );
    ensure!(
        votes["candidate_config"]["sha256"] == CANDIDATE_SHA
            && rate["candidate_policy"]["source_sha256"] == CANDIDATE_SHA
            && config["retained_dpos_configuration"]["candidate_comparison"]["candidate_source_sha256"]
                == CANDIDATE_SHA,
        "mixed candidate configuration"
    );
    ensure!(
        hex::decode(text(
            &votes["candidate_config"]["exact_bytes_hex"],
            "candidate bytes"
        )?)? == CANDIDATE_CONFIG,
        "vote config bytes differ"
    );
    ensure!(
        scalar(
            &votes["candidate_config"]["selected_at_request_period"],
            "candidate Q"
        )? == Q
            && scalar(
                &config["retained_dpos_configuration"]["query_period"],
                "retained query"
            )? == Q,
        "mixed configuration request period"
    );
    let fields = &config["retained_dpos_configuration"]["selected"]["scalar_fields"];
    for (field, expected) in [
        ("delegation_delay", "5"),
        ("eligibility_balance_threshold", "500000000000000000000000"),
        ("vote_eligibility_balance_step", "1000000000000000000000"),
        ("validator_maximum_stake", "80000000000000000000000000"),
    ] {
        ensure!(
            text(&fields[field]["unsigned_decimal"], field)? == expected,
            "retained candidate scalar differs"
        );
    }
    let rate_input = &rate["independent_rate_input"];
    ensure!(
        rate_input["status"] == "available_under_candidate_policy"
            && scalar(&rate_input["lookup_upper_bound_period"], "lambda bound")? == H,
        "rate gate/period differs"
    );
    let blocks_per_year = u32::try_from(scalar(&rate_input["blocks_per_year"], "rate")?)?;
    let total_vote_count = scalar(&votes["details"]["independent_total_vote_count"], "total")?;
    ensure!(blocks_per_year > 0 && total_vote_count > 0);
    let rows = votes["details"]["independently_reconstructed_voters"]
        .as_array()
        .context("missing independent voters")?;
    ensure!(rows.len() == 19, "independent voter count differs");
    let mut weights = BTreeMap::new();
    for row in rows {
        ensure!(
            row["signature_valid"] == true && row["strict_vrf_valid"] == true,
            "independent voter cryptographic evidence incomplete"
        );
        let voter: [u8; 20] = hex::decode(text(&row["voter_hex"], "voter")?)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("bad voter width"))?;
        let weight = scalar(&row["calculated_weight"], "calculated weight")?;
        ensure!(
            weight > 0 && weight <= 1000,
            "invalid independently calculated weight"
        );
        ensure!(
            weights.insert(voter, weight).is_none(),
            "duplicate artifact voter"
        );
    }
    Ok(IndependentInputs {
        blocks_per_year,
        total_vote_count,
        weights,
        evidence: IndependentEvidence {
            mode: "independent_artifacts",
            historical_source_commit: "e63be522d",
            vote_artifact_sha256: VOTES_SHA,
            rate_artifact_sha256: RATE_SHA,
            retained_config_artifact_sha256: CONFIG_SHA,
            candidate_config_sha256: CANDIDATE_SHA,
            certificate_period: P,
            request_period: Q,
            storage_and_eligibility_period: D,
            delayed_state_root_hex: delayed_root.into(),
            independent_total_vote_count: total_vote_count,
            independent_blocks_per_year: blocks_per_year,
            expected_stats_used_for_weight_rate_total_or_author: false,
            fresh_concrete_authentication_performed: false,
            producer_configuration_qualified: false,
            historical_cache_closure_qualified: false,
        },
    })
}

impl IndependentInputs {
    /// Matches all19 artifact voters to freshly inspected signed canonical
    /// votes. Missing, duplicate, substituted or wrong-period signed identities
    /// fail; no positional association or expected-weight map is consulted.
    pub(super) fn cert_facts(&self, canonical: &[Vec<u8>]) -> Result<Vec<RewardCertVoteFact>> {
        let inspections = canonical
            .iter()
            .map(|bytes| inspect_canonical_pbft_vote(bytes))
            .collect::<Result<Vec<_>>>()?;
        self.match_signed_voters(&inspections)
    }
    fn match_signed_voters(
        &self,
        inspections: &[PbftCanonicalVoteInspection],
    ) -> Result<Vec<RewardCertVoteFact>> {
        ensure!(inspections.len() == 19, "signed voter count differs");
        let mut seen = BTreeSet::new();
        let mut facts = Vec::new();
        for vote in inspections {
            ensure!(
                vote.signature_valid && vote.period == P && vote.round == 1 && vote.step == 3,
                "invalid signed voter identity"
            );
            ensure!(
                hex::encode(vote.block_hash) == CERT_BLOCK_HASH,
                "signed certificate block hash differs"
            );
            ensure!(
                seen.insert(vote.recovered_voter.0),
                "duplicate signed voter"
            );
            let weight = *self
                .weights
                .get(&vote.recovered_voter.0)
                .context("signed voter absent from independent artifact")?;
            facts.push(RewardCertVoteFact {
                voter: vote.recovered_voter,
                weight,
                period: vote.period,
            });
        }
        ensure!(
            seen == self.weights.keys().copied().collect(),
            "artifact/signed voter set differs"
        );
        Ok(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reports() -> (Value, Value, Value) {
        (
            serde_json::from_slice(VOTES).unwrap(),
            serde_json::from_slice(RATE).unwrap(),
            serde_json::from_slice(CONFIG).unwrap(),
        )
    }
    fn selected(v: &Value, r: &Value, c: &Value) -> Result<IndependentInputs> {
        derive(v, r, c, v["input_copy"].as_str().unwrap())
    }
    #[test]
    fn byte_mutations_and_mixed_identities_are_rejected() {
        let (v, r, c) = reports();
        selected(&v, &r, &c).unwrap();
        for (bytes, hash) in [(VOTES, VOTES_SHA), (RATE, RATE_SHA), (CONFIG, CONFIG_SHA)] {
            pinned_json(bytes, hash).unwrap();
            let mut changed = bytes.to_vec();
            changed.push(b' ');
            assert!(pinned_json(&changed, hash).is_err());
        }
        for pointer in [
            "/head",
            "/certificate_period",
            "/request_period",
            "/storage_and_eligibility_period",
            "/details/delayed_identity/period",
            "/candidate_config/selected_at_request_period",
        ] {
            let mut changed = v.clone();
            *changed.pointer_mut(pointer).unwrap() = 0.into();
            assert!(selected(&changed, &r, &c).is_err());
        }
        for pointer in [
            "/details/pair/state_root_hex",
            "/details/pair/prior_state_root_hex",
            "/candidate_config/sha256",
            "/details/delayed_identity/state_root_hex",
            "/details/delayed_identity/application_header_sha256",
            "/details/certificate/block_hash_hex",
        ] {
            let mut changed = v.clone();
            *changed.pointer_mut(pointer).unwrap() = "different".into();
            assert!(selected(&changed, &r, &c).is_err());
        }
        let mut changed = r.clone();
        changed["input_copy"] = "other".into();
        assert!(selected(&v, &changed, &c).is_err());
        let mut changed = r.clone();
        changed["independent_rate_input"]["lookup_upper_bound_period"] = 0.into();
        assert!(selected(&v, &changed, &c).is_err());
    }
    #[test]
    fn expected_fields_cannot_supply_independent_inputs() {
        let (mut v, mut r, c) = reports();
        let before = selected(&v, &r, &c).unwrap();
        assert_eq!(before.total_vote_count, 534879);
        assert_eq!(before.blocks_per_year, 9275294);
        v["details"]["comparison"] = Value::Null;
        v["details"]["voters"] = Value::Null;
        r["retained_output_comparison"] = Value::Null;
        let after = selected(&v, &r, &c).unwrap();
        assert_eq!(before.weights, after.weights);
        assert_eq!(before.total_vote_count, after.total_vote_count);
        assert_eq!(before.blocks_per_year, after.blocks_per_year);
    }
    #[test]
    fn signed_voter_mapping_uses_identity_not_order_and_rejects_substitution() {
        let (v, r, c) = reports();
        let inputs = selected(&v, &r, &c).unwrap();
        // Synthetic inspected identities isolate set mapping; production always
        // obtains these facts from the existing signature-inspection owner.
        let mut template = inspect_canonical_pbft_vote(&[]).unwrap();
        template.signature_valid = true;
        template.period = P;
        template.round = 1;
        template.step = 3;
        template.block_hash =
            ethereum_types::H256::from_slice(&hex::decode(CERT_BLOCK_HASH).unwrap());
        let inspections = inputs
            .weights
            .keys()
            .rev()
            .map(|address| {
                let mut row = template.clone();
                row.recovered_voter = (*address).into();
                row
            })
            .collect::<Vec<_>>();
        let facts = inputs.match_signed_voters(&inspections).unwrap();
        for fact in facts {
            assert_eq!(fact.weight, inputs.weights[&fact.voter.0]);
        }
        let mut substituted = inspections.clone();
        substituted[0].recovered_voter = [0; 20].into();
        assert!(inputs.match_signed_voters(&substituted).is_err());
        let mut duplicate = inspections.clone();
        duplicate[0] = duplicate[1].clone();
        assert!(inputs.match_signed_voters(&duplicate).is_err());
        let mut unsigned = inspections.clone();
        unsigned[0].signature_valid = false;
        assert!(inputs.match_signed_voters(&unsigned).is_err());
        let mut changed_hash = inspections.clone();
        changed_hash[0].block_hash = ethereum_types::H256::zero();
        assert!(inputs.match_signed_voters(&changed_hash).is_err());
        assert!(inputs.match_signed_voters(&inspections[..18]).is_err());
        assert!(inputs.cert_facts(&vec![vec![]; 19]).is_err());
    }
}
