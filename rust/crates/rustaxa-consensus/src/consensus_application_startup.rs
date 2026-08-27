//! Native startup recovery for the application-owned consensus runtime.
//!
//! Recovery derives its range from the native PBFT and FinalChain heads,
//! preserves canonical period bytes for the exact external-EVM leaf, hydrates
//! recently-finalized transaction sidecars, reconstructs a due pillar anchor,
//! and only then publishes PBFT bootstrap readiness. No manager object, host
//! policy fact, or private key crosses this module boundary.

use crate::FinalChain;
use crate::consensus_application_runtime::{EvmFinalizationRequest, PillarAnchorStateReport};
use crate::dag_transaction_service::DagTransactionService;
use crate::pbft_manager::{
    PbftManagerStartupReplayRangeFact, plan_pbft_manager_startup_replay_ranges,
};
use crate::pbft_service::PbftService;
use crate::pillar_chain::PillarCurrentAnchorDecisionRequest;
use crate::pillar_chain_service::PillarBlockCreationRequest;
use crate::pillar_vote_service::{
    PillarVoteSingleAdmissionApplyInput, PillarVoteSingleAdmissionContext,
};
use crate::transaction_service::{
    TransactionServicePayload, finalized_status_facts_from_period_data,
};
use anyhow::{Context, Result, ensure};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::{
    CurrentPillarBlockDataDb, PillarBlock, PillarVote, ValidatorVoteCount, ValidatorVoteCountChange,
};

/// Immutable, fully loaded work for one monotonic application bootstrap.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusStartupPlan {
    /// Ordered FinalChain catch-up requests. Effect identities are assigned by
    /// the owning runtime immediately before calling the external leaf.
    pub finalizations: Vec<EvmFinalizationRequest>,
    recent_periods: Vec<(u64, Vec<u8>)>,
    /// Finalized period whose concrete state must be loaded to reconstruct a
    /// pillar block after a crash, or `None` when no restart work is due.
    pub pillar_anchor_state_period: Option<u64>,
    pub(crate) current_period: u64,
    /// Durable own-pillar vote that must be re-admitted before PBFT readiness.
    pub persisted_pillar_vote: Option<StartupPersistedPillarVote>,
}

/// Canonical persisted own-vote identity and its required external DPoS row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPersistedPillarVote {
    pub vote_rlp: Vec<u8>,
    pub period: u64,
    pub dpos_period: u64,
    pub voter: [u8; 20],
}

/// Pillar vote material prepared after exact anchor-state reconstruction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupPillarVoteDraft {
    pub period: u64,
    pub block_hash: H256,
    pub digest: H256,
    pub wallet_index: u64,
    pub expected_voter: [u8; 20],
    pub validator_vote_count: u64,
    pub total_eligible_vote_count: u64,
}

/// Derives and loads all startup work before any external effect executes.
///
/// Missing replay rows, absent Cacti lambda rows, FinalChain/PBFT head
/// inversion, overflow, and unavailable wallet snapshots fail closed. Pillar
/// readiness is published before its native planner is queried, but PBFT live
/// readiness remains pending until [`complete_consensus_startup`] succeeds.
/// The PBFT cursor itself is not reset here: [`PbftService::restore`] already
/// restores and normalizes the persisted period/round/step, executed/next-vote
/// flags, dynamic lambda, proposed-block index, and cert-voted metadata before
/// the application root is published. Runtime-owned round/period clocks start
/// only after this recovery completes, replacing the deleted C++ `initialState`
/// mirrors without a second authority or duplicate persistence transition.
pub fn prepare_consensus_startup(
    pbft: &PbftService,
    dag_transaction: &DagTransactionService,
    final_chain: &FinalChain,
    signing_addresses: &[[u8; 20]],
) -> Result<ConsensusStartupPlan> {
    ensure!(!pbft.is_ready(), "CONSENSUS_STARTUP_ALREADY_COMPLETE");
    let persisted_pillar_vote_rlp = pbft
        .own_pillar_block_vote()
        .context("CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_LOAD")?;
    pbft.complete_pillar_bootstrap()
        .context("CONSENSUS_STARTUP_PILLAR_RESTORE")?;
    restore_cert_voted_metadata(pbft)?;

    let current_period = pbft.pbft_chain_head().size;
    let final_chain_last_block = final_chain.last_block_number()?;
    let delegation_delay = final_chain.dpos_delegation_delay();
    let policy = pbft.process_synced_policy();
    let ranges = plan_pbft_manager_startup_replay_ranges(PbftManagerStartupReplayRangeFact {
        final_chain_last_block,
        pbft_chain_size: current_period,
        delegation_delay,
        recently_finalized_factor: policy.recently_finalized_factor,
    });
    ensure!(ranges.accepted, "{}", ranges.error_code);

    let mut finalizations = Vec::new();
    if ranges.has_finalization_range {
        for period in ranges.finalization_from_period..=ranges.finalization_to_period {
            let cacti = period >= pbft.cacti_block();
            let replay = pbft.load_startup_replay_period(period, cacti)?;
            ensure!(
                replay.found,
                "CONSENSUS_STARTUP_REPLAY_PERIOD_MISSING:{period}"
            );
            let period_rlp = Rlp::new(&replay.period_data_rlp);
            let pbft_block_rlp = period_rlp
                .at(0)
                .context("CONSENSUS_STARTUP_PERIOD_PBFT_BLOCK")?
                .as_raw();
            let link = rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(
                pbft_block_rlp,
            ))?;
            ensure!(
                link.period == period,
                "CONSENSUS_STARTUP_PBFT_PERIOD_MISMATCH"
            );
            let previous_cert_vote_rlps = pbft.validate_startup_replay_cert_votes(
                final_chain,
                period,
                &replay.period_data_rlp,
            )?;
            let blocks_per_year = if cacti {
                let lambda = replay
                    .period_lambda
                    .with_context(|| format!("CONSENSUS_STARTUP_PERIOD_LAMBDA_MISSING:{period}"))?;
                blocks_per_year(lambda, policy.consensus_delay_ms)?
            } else {
                policy.dpos_blocks_per_year
            };
            let anchor_block_rlp = if link.pivot_dag_block_hash.is_zero() {
                Vec::new()
            } else {
                dag_transaction
                    .canonical_dag_block_rlp(link.pivot_dag_block_hash)?
                    .with_context(|| {
                        format!("CONSENSUS_STARTUP_ANCHOR_DAG_BLOCK_MISSING:{period}")
                    })?
            };
            finalizations.push(EvmFinalizationRequest {
                effect_id: Default::default(),
                period_data_rlp: replay.period_data_rlp,
                previous_cert_vote_rlps,
                previous_cert_vote_weights: Vec::new(),
                finalized_dag_hashes: replay
                    .finalized_dag_hashes
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                blocks_per_year,
                synchronous: period == ranges.finalization_to_period,
                anchor_block_rlp,
            });
        }
    }

    let mut recent_periods = Vec::new();
    if ranges.recent_from_period <= ranges.recent_to_period {
        for period in ranges.recent_from_period..=ranges.recent_to_period {
            let replay = pbft.load_startup_replay_period(period, false)?;
            ensure!(
                replay.found,
                "CONSENSUS_STARTUP_RECENT_PERIOD_MISSING:{period}"
            );
            recent_periods.push((period, replay.period_data_rlp));
        }
    }

    if !signing_addresses.is_empty() {
        ensure!(
            final_chain
                .pbft_dpos_eligible_wallet_vote_counts(current_period, signing_addresses)?
                .is_some(),
            "CONSENSUS_STARTUP_WALLET_ELIGIBILITY_UNAVAILABLE"
        );
    }

    let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
    let restart = pbft.plan_pillar_current_anchor_decision(
        PillarCurrentAnchorDecisionRequest::RestartPostProcessing {
            pbft_period: current_period,
            pillar_blocks_interval: pillar_interval,
        },
    )?;
    let pillar_anchor_state_period = if restart.plan.selected {
        Some(
            current_period
                .checked_sub(delegation_delay)
                .context("CONSENSUS_STARTUP_PILLAR_REQUEST_PERIOD_UNDERFLOW")?,
        )
    } else {
        None
    };
    // A disabled Ficus schedule has no current anchor and therefore cannot be
    // selected. Retain this check to reject malformed enabled configuration.
    ensure!(
        ficus_activation == u64::MAX || pillar_interval > 0,
        "CONSENSUS_STARTUP_PILLAR_INTERVAL_ZERO"
    );

    let persisted_pillar_vote = if persisted_pillar_vote_rlp.is_empty() {
        None
    } else {
        let vote = PillarVote::decode_rlp(&persisted_pillar_vote_rlp)
            .context("CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_DECODE")?;
        let voter = vote
            .recover_voter_address()
            .context("CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_SIGNATURE")?;
        Some(StartupPersistedPillarVote {
            vote_rlp: persisted_pillar_vote_rlp,
            period: vote.period,
            dpos_period: vote
                .period
                .checked_sub(1)
                .context("CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_PERIOD_UNDERFLOW")?,
            voter: voter.into(),
        })
    };

    Ok(ConsensusStartupPlan {
        finalizations,
        recent_periods,
        pillar_anchor_state_period,
        current_period,
        persisted_pillar_vote,
    })
}

fn restore_cert_voted_metadata(pbft: &PbftService) -> Result<()> {
    let payload = pbft.cert_voted_block_in_round()?;
    if payload.is_empty() {
        return Ok(());
    }
    let payload = Rlp::new(&payload);
    ensure!(
        payload.item_count()? == 2,
        "CONSENSUS_STARTUP_CERT_VOTED_PAYLOAD_SHAPE"
    );
    let round: u64 = payload.val_at(0)?;
    let block_rlp = payload
        .at(1)
        .context("CONSENSUS_STARTUP_CERT_VOTED_BLOCK")?
        .as_raw()
        .to_vec();
    let link = rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp))?;
    pbft.publish_proposed_block_effect(block_rlp)?;
    let snapshot = pbft.manager_snapshot();
    if snapshot.period == link.period && snapshot.round == round {
        pbft.apply_cert_voted_block_metadata(
            link.period,
            u32::try_from(round).context("CONSENSUS_STARTUP_CERT_VOTED_ROUND_OVERFLOW")?,
            link.block_hash,
        );
    }
    Ok(())
}

fn blocks_per_year(lambda_ms: u32, delay_ms: u32) -> Result<u32> {
    let expected = u64::from(lambda_ms)
        .checked_mul(2)
        .and_then(|value| value.checked_add(u64::from(delay_ms)))
        .context("CONSENSUS_STARTUP_BLOCK_TIME_OVERFLOW")?;
    ensure!(expected > 0, "CONSENSUS_STARTUP_BLOCK_TIME_ZERO");
    u32::try_from(365_u64 * 24 * 60 * 60 * 1000 / expected)
        .context("CONSENSUS_STARTUP_BLOCKS_PER_YEAR_OVERFLOW")
}

/// Hydrates the recently-finalized transaction sidecar in period order.
pub fn hydrate_recently_finalized_transactions(
    plan: &ConsensusStartupPlan,
    dag_transaction: &DagTransactionService,
) -> Result<()> {
    for (period, period_data_rlp) in &plan.recent_periods {
        let payloads = finalized_status_facts_from_period_data(period_data_rlp)?
            .into_iter()
            .map(|fact| TransactionServicePayload {
                hash: fact.hash,
                transaction_rlp: fact.tx_rlp,
            })
            .collect();
        dag_transaction.transaction_initialize_recently_finalized(*period, payloads)?;
    }
    Ok(())
}

/// Reconstructs and persists a due pillar block from one exact EVM report.
///
/// The report is accepted only for a plan that requested the leaf. Epochs must
/// fit the legacy native pillar type. The returned drafts contain only public
/// signer identities and unsigned digests; signature bytes are supplied later
/// by the exact signing leaf.
pub fn apply_startup_pillar_anchor_state(
    plan: &ConsensusStartupPlan,
    report: &PillarAnchorStateReport,
    pbft: &PbftService,
    signing_identities: &[(u64, [u8; 20])],
) -> Result<Vec<StartupPillarVoteDraft>> {
    ensure!(
        plan.pillar_anchor_state_period.is_some(),
        "CONSENSUS_STARTUP_PILLAR_STATE_UNREQUESTED"
    );
    ensure!(
        report.succeeded,
        "CONSENSUS_STARTUP_PILLAR_STATE_FAILED:{}",
        report.error_code
    );
    ensure!(
        !report.block_header_rlp.is_empty(),
        "CONSENSUS_STARTUP_PILLAR_HEADER_EMPTY"
    );

    let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
    let first_pillar_period = if ficus_activation == 0 {
        pillar_interval
    } else {
        ficus_activation
    };
    let creation = pbft.plan_pillar_block_creation_with_vote_counts(
        PillarBlockCreationRequest {
            pillar_block_period: plan.current_period,
            state_root: report.state_root.into(),
            bridge_root: report.bridge_root.into(),
            bridge_epoch: report.bridge_epoch.into(),
            first_pillar_block_period: first_pillar_period,
            pillar_blocks_interval: pillar_interval,
        },
        report
            .validator_vote_counts
            .iter()
            .map(|count| crate::pillar_chain::PillarValidatorVoteCount {
                address: count.address.into(),
                vote_count: count.vote_count,
            })
            .collect(),
    )?;
    ensure!(
        creation.creation.valid,
        "CONSENSUS_STARTUP_PILLAR_LINKAGE_REJECTED"
    );
    let block = PillarBlock {
        period: plan.current_period,
        state_root: creation.creation.state_root,
        previous_pillar_block_hash: creation.creation.previous_pillar_block_hash,
        bridge_root: creation.creation.bridge_root,
        epoch: ethereum_types::U256::from_big_endian(&report.bridge_epoch),
        validator_vote_count_changes: creation
            .vote_count_changes
            .into_iter()
            .map(|change| ValidatorVoteCountChange {
                address: change.address,
                vote_count_change: change.vote_count_change,
            })
            .collect(),
    };
    let block_hash = block.hash();
    let data = CurrentPillarBlockDataDb {
        pillar_block: block,
        vote_counts: creation
            .current_vote_counts
            .into_iter()
            .map(|count| ValidatorVoteCount {
                address: count.address,
                vote_count: count.vote_count,
            })
            .collect(),
    };
    pbft.apply_pillar_current_block_data_for_generation(
        data.encode_rlp(),
        creation.anchor_generation,
    )?;

    let vote_period = plan
        .current_period
        .checked_add(1)
        .context("CONSENSUS_STARTUP_PILLAR_VOTE_PERIOD_OVERFLOW")?;
    ensure!(
        report.signer_vote_counts.len() == signing_identities.len(),
        "CONSENSUS_STARTUP_PILLAR_SIGNER_FACT_COUNT_MISMATCH"
    );
    Ok(signing_identities
        .iter()
        .zip(report.signer_vote_counts.iter().copied())
        .filter(|(_, vote_count)| *vote_count > 0)
        .map(|((wallet_index, address), validator_vote_count)| {
            let unsigned = PillarVote {
                period: vote_period,
                block_hash,
                signature: [0; 65],
            };
            StartupPillarVoteDraft {
                period: vote_period,
                block_hash,
                digest: unsigned.hash(false),
                wallet_index: *wallet_index,
                expected_voter: *address,
                validator_vote_count,
                total_eligible_vote_count: report.total_eligible_vote_count,
            }
        })
        .collect())
}

/// Verifies, admits, and persists one host-signed startup pillar vote.
pub fn apply_startup_pillar_vote(
    draft: &StartupPillarVoteDraft,
    signature: &[u8],
    pbft: &PbftService,
) -> Result<Vec<u8>> {
    let signature: [u8; 65] = signature
        .try_into()
        .context("CONSENSUS_STARTUP_PILLAR_SIGNATURE_SIZE")?;
    let vote = PillarVote {
        period: draft.period,
        block_hash: draft.block_hash,
        signature,
    };
    ensure!(
        vote.recover_voter_address().map(Into::<[u8; 20]>::into) == Some(draft.expected_voter),
        "CONSENSUS_STARTUP_PILLAR_SIGNATURE_IDENTITY_MISMATCH"
    );
    let vote_rlp = vote.encode_rlp();
    let (ficus_activation, pillar_interval) = pbft.pillar_schedule();
    let first_pillar_period = if ficus_activation == 0 {
        pillar_interval
    } else {
        ficus_activation
    };
    let prepared = pbft.prepare_single_pillar_vote_external_facts(
        vote_rlp.clone(),
        PillarVoteSingleAdmissionContext {
            first_pillar_block_period: first_pillar_period,
            pillar_blocks_interval: pillar_interval,
        },
        true,
    )?;
    ensure!(
        prepared.can_query_dpos,
        "CONSENSUS_STARTUP_PILLAR_VOTE_PREPARATION_REJECTED"
    );
    let applied = pbft.apply_prepared_single_pillar_vote_external_facts(
        PillarVoteSingleAdmissionApplyInput {
            vote_hash: prepared.vote_hash,
            validator_vote_count: draft.validator_vote_count,
            has_total_eligible_vote_count: prepared.needs_threshold,
            total_eligible_vote_count: if prepared.needs_threshold {
                draft.total_eligible_vote_count
            } else {
                0
            },
        },
    )?;
    ensure!(
        applied.accepted || applied.duplicate,
        "CONSENSUS_STARTUP_PILLAR_VOTE_REJECTED"
    );
    pbft.apply_own_pillar_vote(vote_rlp.clone())?;
    Ok(vote_rlp)
}

/// Re-admits one durable own-pillar vote using exact external DPoS facts.
///
/// The vote is already persisted, so this operation only restores native
/// aggregation state. Its canonical identity must match the retained startup
/// plan, and zero validator weight is rejected instead of silently publishing
/// bootstrap readiness without the vote.
pub fn apply_startup_persisted_pillar_vote(
    persisted: &StartupPersistedPillarVote,
    validator_vote_count: u64,
    total_eligible_vote_count: u64,
    pbft: &PbftService,
) -> Result<()> {
    let prepared = pbft.prepare_single_pillar_vote_external_facts(
        persisted.vote_rlp.clone(),
        PillarVoteSingleAdmissionContext {
            first_pillar_block_period: 0,
            pillar_blocks_interval: 0,
        },
        true,
    )?;
    ensure!(
        prepared.can_query_dpos,
        "CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_PREPARATION_REJECTED"
    );
    ensure!(
        prepared.period == persisted.period && prepared.voter == persisted.voter,
        "CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_IDENTITY_MISMATCH"
    );
    let applied = pbft.apply_prepared_single_pillar_vote_external_facts(
        PillarVoteSingleAdmissionApplyInput {
            vote_hash: prepared.vote_hash,
            validator_vote_count,
            has_total_eligible_vote_count: prepared.needs_threshold,
            total_eligible_vote_count: if prepared.needs_threshold {
                total_eligible_vote_count
            } else {
                0
            },
        },
    )?;
    ensure!(
        applied.accepted || applied.duplicate,
        "CONSENSUS_STARTUP_PERSISTED_PILLAR_VOTE_REJECTED:{}",
        applied.status
    );
    Ok(())
}

/// Publishes the one-way PBFT bootstrap transition after every startup effect.
pub fn complete_consensus_startup(pbft: &PbftService) -> Result<()> {
    ensure!(pbft.pillar_is_ready(), "CONSENSUS_STARTUP_PILLAR_NOT_READY");
    pbft.complete_bootstrap();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_blocks_per_year_matches_legacy_formula() {
        assert_eq!(blocks_per_year(2_000, 700).unwrap(), 6_709_787);
        assert!(blocks_per_year(0, 0).is_err());
    }

    #[test]
    fn startup_pillar_vote_draft_hashes_unsigned_canonical_vote() {
        let vote = PillarVote {
            period: 8,
            block_hash: H256::repeat_byte(0x44),
            signature: [0; 65],
        };
        let draft = StartupPillarVoteDraft {
            period: vote.period,
            block_hash: vote.block_hash,
            digest: vote.hash(false),
            wallet_index: 2,
            expected_voter: [0x55; 20],
            validator_vote_count: 3,
            total_eligible_vote_count: 5,
        };
        assert_eq!(draft.digest, vote.hash(false));
        assert_ne!(draft.digest, vote.hash(true));
    }
}
