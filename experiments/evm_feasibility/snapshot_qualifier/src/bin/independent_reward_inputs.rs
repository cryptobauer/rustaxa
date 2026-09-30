//! Bounded candidate reward vote reconstruction from authenticated delayed state.
//! At most59 logical keys are consulted, each via one physical read and one
//! authenticated proof. Failure writes explicit partial diagnostic evidence;
//! it never expands the candidate set, enumerates children or publishes state.
use anyhow::{Context, Result, ensure};
use rustaxa_consensus::native_session::vote_inputs::{
    HistoricalVotePolicy, historical_total_votes, historical_voter_inputs,
};
use rustaxa_consensus::{
    PbftVoteValidationExternalFacts, RewardsStatsPeriodRlp, decode_rewards_block_distributions,
    inspect_canonical_pbft_vote, validate_canonical_pbft_vote,
};
use rustaxa_snapshot_qualifier::{paths, qualification, reward_votes::reconstruct_cert_votes};
use rustaxa_storage::{
    ConcreteCheckpointReaders, ConcreteStoragePath, FinalChainRepository, PeriodRepository,
};
use rustaxa_types::{
    DposTokenAmount, FinalChainBlockNumber, StoredFinalChainBlockHeader,
    codec::rlp::final_chain::StoredBlockHeaderRlp,
    concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStateIdentity, ConcreteStorageKey},
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    env,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};
use tiny_keccak::{Hasher, Keccak};

const CERT_PERIOD: u64 = 25_706_948;
const REQUEST_PERIOD: u64 = 25_706_947;
const DELAYED_PERIOD: u64 = 25_706_942;
const MAX_KEYS: usize = 59;
const CONFIG: &[u8] = include_bytes!(
    "../../../../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json"
);

/// One physical/proof pair. Unknown physical history can become logical absence
/// only through successful NonMember evidence; proof errors never default values.
#[derive(Serialize)]
struct Observation {
    family: String,
    account_hex: String,
    logical_key_hex: String,
    physical_result: String,
    physical_value_hex: Option<String>,
    proof_result: String,
    proof_value_hex: Option<String>,
    reconciled_logical_result: Option<&'static str>,
}

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: independent_reward_inputs COPY OUTPUT")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: independent_reward_inputs COPY OUTPUT")?,
    );
    ensure!(args.next().is_none());
    let paths = paths::validate(&input, &output)?;
    let mut observations = Vec::new();
    let mut details = json!({});
    let result = reconstruct(
        &paths.application,
        &paths.state,
        &mut observations,
        &mut details,
    );
    let error = result.as_ref().err().map(|error| format!("{error:#}"));
    let report = json!({
        "schema": 1, "mode": "independent_reward_vote_inputs", "status": if error.is_some() { "gate_failed" } else { "candidate_reconstructed" }, "error": error,
        "input_copy": paths.input, "tool_source_sha256": source_hash(),
        "candidate_config": { "reference": "checked-in mainnet_genesis.json; not verified producer/historical config", "sha256": sha(CONFIG), "exact_bytes_hex": hex::encode(CONFIG), "selected_at_request_period": REQUEST_PERIOD, "producer_qualified": false },
        "head": qualification::HEAD, "certificate_period": CERT_PERIOD, "request_period": REQUEST_PERIOD, "candidate_delegation_delay": 5, "storage_and_eligibility_period": DELAYED_PERIOD,
        "read_contract": { "maximum_unique_logical_keys": MAX_KEYS, "maximum_raw_and_proof_calls": 118, "completed_or_attempted_pairs": observations.len(), "range_inventory": false, "child_enumeration": false, "owner_internal_reads_counted_as_logical_calls": false },
        "details": details, "observations": observations,
        "qualification": { "producer_config_or_binary_qualified": false, "prior_vote_inputs_candidate_only": true, "complete_dpos_snapshot": false, "reward_transition_or_root_qualified": false, "publication_or_adoption_authorized": false, "production_routing_authorized": false }
    });
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(paths.output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    result
}

fn reconstruct(
    application: &Path,
    state: &Path,
    observations: &mut Vec<Observation>,
    details: &mut Value,
) -> Result<()> {
    let (_, head_identity, pair) = qualification::qualify(application, state)?;
    details["pair"] = serde_json::to_value(pair)?;
    let config: Value = serde_json::from_slice(CONFIG)?;
    ensure!(config["dpos"]["delegation_delay"].as_str() == Some("0x5"));
    let policy = HistoricalVotePolicy {
        threshold: amount(&config["dpos"]["eligibility_balance_threshold"])?,
        step: amount(&config["dpos"]["vote_eligibility_balance_step"])?,
        maximum_stake: amount(&config["dpos"]["validator_maximum_stake"])?,
        magnolia_period: config["hardforks"]["magnolia_hf"]["block_num"]
            .as_u64()
            .context("missing Magnolia")?,
        cacti_period: config["hardforks"]["cacti_hf"]["block_num"]
            .as_u64()
            .context("missing Cacti")?,
    };
    policy.validate()?;
    let committee = hex_u64(&config["pbft"]["committee_size"])?;
    let proposers = hex_u64(&config["pbft"]["number_of_proposers"])?;
    ensure!(committee == 1000 && proposers == 20);
    let app = qualification::open_application(application)?;
    let chain = FinalChainRepository::new(app.clone());
    let periods = PeriodRepository::new(app.clone());
    let delayed_header = chain
        .block_header_raw(DELAYED_PERIOD)?
        .context("missing delayed header")?;
    let decoded =
        StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&delayed_header))?;
    let delayed_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(DELAYED_PERIOD),
        state_root: decoded.state_root.into(),
    };
    details["delayed_identity"] = json!({ "period": DELAYED_PERIOD, "state_root_hex": hex::encode(delayed_identity.state_root), "application_header_sha256": sha(&delayed_header) });
    let readers = ConcreteCheckpointReaders::open_read_only(
        state,
        head_identity,
        [head_identity, delayed_identity],
    )?;
    let period_data = periods.data_raw(qualification::HEAD)?;
    ensure!(
        period_data.len() == 7286
            && sha(&period_data)
                == "bcb4b16bed154c41156ed87f6d020f5ed7fad96f0c7562af778aeefd474de11b",
        "PeriodData differs from historical target"
    );
    let votes = reconstruct_cert_votes(&period_data)?;
    ensure!(
        votes.certified_period == CERT_PERIOD
            && votes.round == 1
            && votes.step == 3
            && votes.canonical.len() == 19,
        "certificate identity/cardinality differs"
    );
    let mut voters = BTreeSet::new();
    let mut inspections = Vec::new();
    for vote in &votes.canonical {
        let inspection = inspect_canonical_pbft_vote(vote)?;
        ensure!(
            inspection.signature_valid
                && inspection.period == CERT_PERIOD
                && inspection.round == 1
                && inspection.step == 3
        );
        ensure!(
            voters.insert(inspection.recovered_voter.0),
            "duplicate cert voter"
        );
        inspections.push(inspection);
    }
    details["certificate"] = json!({ "period": CERT_PERIOD, "round": 1, "step": 3, "unique_voters": voters.len(), "block_hash_hex": hex::encode(inspections[0].block_hash), "period_data_sha256": sha(&period_data) });
    // The empty-list gate precedes all per-voter reads. No full-map total helper.
    let dpos = address(0xfe);
    let slashing = address(0xee);
    let total = read_reconciled(
        &readers,
        delayed_identity,
        dpos,
        key(&[4]),
        "stored_total_votes",
        observations,
    )?;
    let mut list_key = [0; 32];
    list_key[31] = 2;
    let jailed_list = read_reconciled(
        &readers,
        delayed_identity,
        slashing,
        ConcreteStorageKey(list_key),
        "jailed_validator_list",
        observations,
    )?;
    let total_count = historical_total_votes(total.as_deref(), jailed_list.as_deref())?;
    details["independent_total_vote_count"] = total_count.into();
    let mut reconstructed = Vec::new();
    for (vote, inspection) in votes.canonical.iter().zip(inspections) {
        let voter = inspection.recovered_voter.0;
        let validator = read_reconciled(
            &readers,
            delayed_identity,
            dpos,
            key(&[&[0, 0][..], &voter].concat()),
            "validator",
            observations,
        )?;
        let vrf = read_reconciled(
            &readers,
            delayed_identity,
            dpos,
            key(&[&[0, 4][..], &voter].concat()),
            "vrf_key",
            observations,
        )?;
        let jail = read_reconciled(
            &readers,
            delayed_identity,
            slashing,
            key(&[&[0][..], &voter].concat()),
            "jail_block",
            observations,
        )?;
        let facts = historical_voter_inputs(
            policy,
            DELAYED_PERIOD,
            validator.as_deref(),
            vrf.as_deref(),
            jail.as_deref(),
        )?;
        let vrf_key = facts
            .vrf_key
            .context("certificate voter VRF key absent or zero")?;
        ensure!(
            facts.eligible_vote_count > 0 && facts.eligible_vote_count <= total_count,
            "certificate voter not eligible or exceeds total"
        );
        let validation = validate_canonical_pbft_vote(
            vote,
            PbftVoteValidationExternalFacts {
                voter_dpos_ready: true,
                voter_dpos_vote_count: facts.eligible_vote_count,
                total_dpos_ready: true,
                total_dpos_vote_count: total_count,
                future_dpos_state: false,
                unknown_error: false,
                vrf_key_ready: true,
                has_vrf_key: true,
                vrf_public_key: vrf_key,
                strict_vrf: true,
                committee_size: committee,
                number_of_proposers: proposers,
                has_preverified_weight: false,
                preverified_weight: 0,
            },
        )?;
        ensure!(
            validation.accepted
                && validation.signature_valid
                && validation.vrf_valid
                && validation.weight_calculated
                && validation.calculated_weight > 0,
            "independent certificate validation failed: {}",
            validation.error_code
        );
        reconstructed.push(json!({ "voter_hex": hex::encode(voter), "stake_be_hex": hex::encode(facts.stake_be), "eligible_vote_count": facts.eligible_vote_count, "vrf_key_hex": hex::encode(vrf_key), "jail_until": facts.jail_until, "jailed_at_delayed_period": facts.jailed_at_effective_period, "signature_valid": validation.signature_valid, "strict_vrf_valid": validation.vrf_valid, "calculated_weight": validation.calculated_weight }));
        details["independently_reconstructed_voters"] = serde_json::to_value(&reconstructed)?;
    }
    details["independently_reconstructed_voters"] = serde_json::to_value(&reconstructed)?;
    ensure!(observations.len() == MAX_KEYS);
    // Retained expected weights enter only after independent reconstruction.
    let stats = app
        .get_cf(
            &app.cf_handle("block_rewards_stats")
                .context("missing stats column")?,
            qualification::HEAD.to_le_bytes(),
        )?
        .context("missing retained BlockStats")?;
    ensure!(
        stats.len() == 566
            && sha(&stats) == "18bb350916c3d85a52ec401cb2b898d2696b57e3859d5ff9dda1218a8ed05070",
        "retained BlockStats differs from historical row"
    );
    let observed = decode_rewards_block_distributions(&[RewardsStatsPeriodRlp {
        period: qualification::HEAD,
        data: stats.clone(),
    }])?
    .pop()
    .context("no retained stats")?;
    let mut sum = 0_u64;
    let mut matches = true;
    for row in &mut reconstructed {
        let voter: [u8; 20] = hex::decode(row["voter_hex"].as_str().unwrap())?
            .try_into()
            .unwrap();
        let retained = observed
            .validators_stats
            .get(&voter)
            .context("reconstructed voter missing retained expected stats")?
            .vote_weight;
        let derived = row["calculated_weight"].as_u64().unwrap();
        sum = sum.checked_add(derived).context("weight sum overflow")?;
        row["retained_expected_weight"] = retained.into();
        row["expected_weight_matches"] = (derived == retained).into();
        matches &= derived == retained;
    }
    details["voters"] = reconstructed.into();
    details["comparison"] = json!({ "all_independent_weights_match_retained": matches, "independent_weight_sum": sum, "retained_weight_sum": observed.total_votes_weight, "sum_matches": sum == observed.total_votes_weight, "retained_stats_sha256": sha(&stats), "expected_stats_used_to_derive_weights": false, "strict_vrf_is_diagnostic_policy_not_recovered_legacy_sample_rate": true });
    Ok(())
}

fn read_reconciled(
    readers: &ConcreteCheckpointReaders,
    identity: ConcreteStateIdentity,
    account: [u8; 20],
    key: ConcreteStorageKey,
    family: &str,
    rows: &mut Vec<Observation>,
) -> Result<Option<Vec<u8>>> {
    guard_key(rows, account, key)?;
    let raw = readers.storage_at(identity, account, key);
    let proof = readers.verify_storage_path_at(identity, account, key);
    let mut row = Observation {
        family: family.into(),
        account_hex: hex::encode(account),
        logical_key_hex: hex::encode(key.0),
        physical_result: raw_status(&raw),
        physical_value_hex: match &raw {
            Ok(ConcreteRead::Present(bytes)) => Some(hex::encode(bytes)),
            _ => None,
        },
        proof_result: proof_status(&proof),
        proof_value_hex: match &proof {
            Ok(ConcreteStoragePath::Member(bytes)) => Some(hex::encode(bytes)),
            _ => None,
        },
        reconciled_logical_result: None,
    };
    let result = reconcile(identity, raw, proof);
    if let Ok(value) = &result {
        row.reconciled_logical_result = Some(if value.is_some() {
            "member"
        } else {
            "proof_backed_absence"
        });
    }
    rows.push(row);
    result.with_context(|| format!("reconcile {family}"))
}

fn guard_key(rows: &[Observation], account: [u8; 20], key: ConcreteStorageKey) -> Result<()> {
    ensure!(rows.len() < MAX_KEYS, "logical key bound reached");
    ensure!(
        !rows
            .iter()
            .any(|row| row.account_hex == hex::encode(account)
                && row.logical_key_hex == hex::encode(key.0)),
        "duplicate logical key"
    );
    Ok(())
}

fn reconcile(
    identity: ConcreteStateIdentity,
    raw: Result<ConcreteRead<Vec<u8>>, ConcreteReadError>,
    proof: Result<ConcreteStoragePath, ConcreteReadError>,
) -> Result<Option<Vec<u8>>> {
    match (raw, proof?) {
        (Ok(ConcreteRead::Present(raw)), ConcreteStoragePath::Member(proved)) if raw == proved => {
            Ok(Some(proved))
        }
        (Ok(ConcreteRead::Absent | ConcreteRead::Tombstone), ConcreteStoragePath::NonMember) => {
            Ok(None)
        }
        (Err(ConcreteReadError::HistoryUnavailable(observed)), ConcreteStoragePath::NonMember)
            if observed == identity =>
        {
            Ok(None)
        }
        (Err(error), _) => Err(error.into()),
        _ => anyhow::bail!("physical/proof membership or value mismatch"),
    }
}
fn raw_status(raw: &Result<ConcreteRead<Vec<u8>>, ConcreteReadError>) -> String {
    match raw {
        Ok(ConcreteRead::Present(_)) => "present".into(),
        Ok(ConcreteRead::Absent) => "absent".into(),
        Ok(ConcreteRead::Tombstone) => "tombstone".into(),
        Err(error) => format!("{error:?}"),
    }
}
fn proof_status(proof: &Result<ConcreteStoragePath, ConcreteReadError>) -> String {
    match proof {
        Ok(ConcreteStoragePath::Member(_)) => "member".into(),
        Ok(ConcreteStoragePath::NonMember) => "nonmember".into(),
        Err(error) => format!("{error:?}"),
    }
}
fn address(last: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = last;
    address
}
fn key(bytes: &[u8]) -> ConcreteStorageKey {
    let mut hash = [0; 32];
    let mut k = Keccak::v256();
    k.update(bytes);
    k.finalize(&mut hash);
    ConcreteStorageKey(hash)
}
fn amount(value: &Value) -> Result<DposTokenAmount> {
    let text = value
        .as_str()
        .context("missing candidate amount")?
        .trim_start_matches("0x");
    let padded = if text.len() % 2 == 1 {
        format!("0{text}")
    } else {
        text.to_owned()
    };
    Ok(DposTokenAmount::try_from_be_slice(&hex::decode(padded)?)?)
}
fn hex_u64(value: &Value) -> Result<u64> {
    Ok(u64::from_str_radix(
        value
            .as_str()
            .context("missing candidate scalar")?
            .trim_start_matches("0x"),
        16,
    )?)
}
fn sha(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn source_hash() -> String {
    sha(&[include_bytes!("independent_reward_inputs.rs").as_slice(),include_bytes!("../reward_votes.rs"),include_bytes!("../qualification.rs"),include_bytes!("../paths.rs"),include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/final_chain/native_session/vote_inputs.rs"),include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/final_chain/native_session/semantic_port.rs"),include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/final_chain/native_session.rs"),include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/final_chain.rs"),include_bytes!("../../../../../rust/crates/rustaxa-consensus/src/pbft_vote_validation.rs")].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: DELAYED_PERIOD.into(),
            state_root: [1; 32],
        }
    }
    #[test]
    fn reconciliation_requires_proof_and_exact_member_bytes() {
        assert_eq!(
            reconcile(
                identity(),
                Ok(ConcreteRead::Present(vec![1, 2])),
                Ok(ConcreteStoragePath::Member(vec![1, 2]))
            )
            .unwrap(),
            Some(vec![1, 2])
        );
        assert!(
            reconcile(
                identity(),
                Ok(ConcreteRead::Present(vec![1])),
                Ok(ConcreteStoragePath::Member(vec![2]))
            )
            .is_err()
        );
        assert!(
            reconcile(
                identity(),
                Ok(ConcreteRead::Present(vec![1])),
                Ok(ConcreteStoragePath::NonMember)
            )
            .is_err()
        );
        assert!(
            reconcile(
                identity(),
                Err(ConcreteReadError::HistoryUnavailable(identity())),
                Ok(ConcreteStoragePath::Member(vec![1]))
            )
            .is_err()
        );
        assert!(
            reconcile(
                identity(),
                Err(ConcreteReadError::HistoryUnavailable(identity())),
                Err(ConcreteReadError::HistoryUnavailable(identity()))
            )
            .is_err()
        );
        assert_eq!(
            reconcile(
                identity(),
                Err(ConcreteReadError::HistoryUnavailable(identity())),
                Ok(ConcreteStoragePath::NonMember)
            )
            .unwrap(),
            None
        );
        assert_eq!(
            reconcile(
                identity(),
                Ok(ConcreteRead::Tombstone),
                Ok(ConcreteStoragePath::NonMember)
            )
            .unwrap(),
            None
        );
        assert!(
            reconcile(
                identity(),
                Ok(ConcreteRead::Absent),
                Err(ConcreteReadError::Corrupt("missing node".into()))
            )
            .is_err()
        );
    }

    #[test]
    fn unavailable_identity_and_bounds_fail_before_more_reads() {
        for other in [
            ConcreteStateIdentity {
                period: (DELAYED_PERIOD - 1).into(),
                ..identity()
            },
            ConcreteStateIdentity {
                state_root: [2; 32],
                ..identity()
            },
        ] {
            assert!(
                reconcile(
                    identity(),
                    Err(ConcreteReadError::HistoryUnavailable(other)),
                    Ok(ConcreteStoragePath::NonMember)
                )
                .is_err()
            );
        }
        assert!(
            reconcile(
                identity(),
                Err(ConcreteReadError::Pruned(identity())),
                Ok(ConcreteStoragePath::NonMember)
            )
            .is_err()
        );
        let make = |index: u8| Observation {
            family: "fixture".into(),
            account_hex: hex::encode(address(0xfe)),
            logical_key_hex: hex::encode(key(&[index]).0),
            physical_result: "absent".into(),
            physical_value_hex: None,
            proof_result: "nonmember".into(),
            proof_value_hex: None,
            reconciled_logical_result: Some("proof_backed_absence"),
        };
        let rows = (0..59).map(make).collect::<Vec<_>>();
        assert!(guard_key(&rows, address(0xee), key(&[100])).is_err());
        assert!(guard_key(&rows[..1], address(0xfe), key(&[0])).is_err());
        assert!(guard_key(&rows[..58], address(0xee), key(&[100])).is_ok());
    }
}
