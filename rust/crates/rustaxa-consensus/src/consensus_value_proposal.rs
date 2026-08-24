//! Canonical native PBFT value-proposal block signing.

use crate::dag_transaction_service::{DagGhostPathRoot, DagTransactionService};
use crate::pbft_manager::{
    PbftManagerProposalAction, PbftManagerProposalInitialFact, PbftManagerProposalWalletFact,
};
use crate::pbft_service::PbftProposedBlockAdmissionStatus;
use crate::verified_votes::TwoTPlusOneVotedBlockType;
use crate::{FinalChain, PbftService};
use anyhow::{Result, anyhow, ensure};
use ethereum_types::H256;
use rlp::RlpStream;
use rustaxa_types::PbftBlockMetadata;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::pbft::PbftBlockLink;
use tiny_keccak::{Hasher, Keccak};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusValueProposalInput {
    pub previous_pbft_block_hash: H256,
    pub anchor_hash: H256,
    pub order_hash: H256,
    pub final_chain_hash: H256,
    pub period: u64,
    pub timestamp: u64,
    /// Complete canonical reward-votes RLP list at legacy field index 6.
    pub reward_votes_rlp: Vec<u8>,
    /// Optional Ficus extra-data bytes at field index 7.
    pub extra_data: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusUnsignedValueProposal {
    pub input: ConsensusValueProposalInput,
    pub signing_hash: [u8; 32],
}

/// Root-owned terminal decision for one PBFT value-proposal action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsensusValueProposalAction {
    /// No certified prior-round value/null decision or no eligible proposer.
    NoWork,
    /// Construct and sign a new block after a first-round or next-null decision.
    Build {
        eligible_wallet_indices: Vec<u64>,
        unsigned: ConsensusUnsignedValueProposal,
        reward_votes_bundle_rlp: Vec<u8>,
    },
    /// Re-propose the retained, validated previous-round next value unchanged.
    Repropose {
        eligible_wallet_indices: Vec<u64>,
        block_rlp: Vec<u8>,
        reward_votes_bundle_rlp: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PriorRoundValueDecision {
    Build,
    Repropose(H256),
    NoWork,
}

fn decide_prior_round_value(
    round: u64,
    next_value: Option<H256>,
    next_null: bool,
) -> PriorRoundValueDecision {
    if round <= 1 {
        PriorRoundValueDecision::Build
    } else if let Some(block_hash) = next_value {
        PriorRoundValueDecision::Repropose(block_hash)
    } else if next_null {
        PriorRoundValueDecision::Build
    } else {
        PriorRoundValueDecision::NoWork
    }
}

fn ensure_reward_snapshot_current(
    proposal_period: u64,
    cursor_found: bool,
    cursor_period: u64,
    records_empty: bool,
) -> Result<()> {
    if proposal_period > 1 {
        ensure!(
            cursor_found && cursor_period == proposal_period - 1 && !records_empty,
            "CONSENSUS_VALUE_PROPOSAL_REWARD_VOTES_NOT_CURRENT"
        );
    }
    Ok(())
}

fn build_reward_votes_network_bundle(
    records: &[crate::pbft_vote_payload::PbftVotePayloadRecord],
) -> Result<Vec<u8>> {
    let Some(first) = records.first() else {
        return Ok(Vec::new());
    };
    let identity = crate::pbft_vote_validation::inspect_canonical_pbft_vote(&first.vote_rlp)?;
    Ok(crate::pbft_vote_payload::build_optimized_pbft_vote_bundle(
        records,
        identity.block_hash,
        identity.period,
        identity.round,
        identity.step,
    )?
    .bundle_rlp)
}

/// Composes native chain, FinalChain, DAG order and reward-vote state into one signing request.
pub fn compose_value_proposal(
    service: &PbftService,
    final_chain: &FinalChain,
    dag: &DagTransactionService,
    timestamp: u64,
    eligible_wallet_indices: Vec<u64>,
) -> Result<ConsensusValueProposalAction> {
    let live = service.manager_snapshot();
    if eligible_wallet_indices.is_empty() {
        return Ok(ConsensusValueProposalAction::NoWork);
    }
    let previous_value = if live.round > 1 {
        service.verified_votes_get_two_t_plus_one_voted_block(
            live.period,
            live.round - 1,
            TwoTPlusOneVotedBlockType::NextVotedBlock,
        )?
    } else {
        None
    };
    let previous_null = if live.round > 1 && previous_value.is_none() {
        service
            .verified_votes_get_two_t_plus_one_voted_block(
                live.period,
                live.round - 1,
                TwoTPlusOneVotedBlockType::NextVotedNullBlock,
            )?
            .is_some()
    } else {
        false
    };
    match decide_prior_round_value(
        live.round,
        previous_value.as_ref().map(|value| value.block_hash),
        previous_null,
    ) {
        PriorRoundValueDecision::Repropose(block_hash) => {
            let admission = service.admit_proposed_block(
                final_chain,
                dag,
                service.value_proposal_admission_request(live.period, block_hash),
            )?;
            return Ok(match admission.status {
                PbftProposedBlockAdmissionStatus::AcceptedAlreadyValid
                | PbftProposedBlockAdmissionStatus::AcceptedNewlyValidated => {
                    let block = rlp::Rlp::new(&admission.block_rlp);
                    let rewards = block.at(6)?;
                    ensure!(
                        rewards.is_list(),
                        "CONSENSUS_REPROPOSAL_REWARD_VOTES_NOT_LIST"
                    );
                    let mut hashes = Vec::with_capacity(rewards.item_count()?);
                    for index in 0..rewards.item_count()? {
                        hashes.push(rewards.val_at(index)?);
                    }
                    let selected = service.select_reward_vote_payloads(live.period, hashes)?;
                    ensure!(
                        selected.accepted,
                        "CONSENSUS_REPROPOSAL_REWARD_VOTES_UNAVAILABLE"
                    );
                    ConsensusValueProposalAction::Repropose {
                        eligible_wallet_indices,
                        block_rlp: admission.block_rlp,
                        reward_votes_bundle_rlp: build_reward_votes_network_bundle(
                            &selected.selected_records,
                        )?,
                    }
                }
                PbftProposedBlockAdmissionStatus::Missing
                | PbftProposedBlockAdmissionStatus::Rejected => {
                    ConsensusValueProposalAction::NoWork
                }
            });
        }
        PriorRoundValueDecision::NoWork => {
            return Ok(ConsensusValueProposalAction::NoWork);
        }
        PriorRoundValueDecision::Build => {}
    }
    let reward_snapshot = service.current_reward_vote_snapshot()?;
    ensure_reward_snapshot_current(
        live.period,
        reward_snapshot.cursor.found,
        reward_snapshot.cursor.period,
        reward_snapshot.records.is_empty(),
    )?;
    let reward_votes_bundle_rlp = build_reward_votes_network_bundle(&reward_snapshot.records)?;
    let head = service.chain().head();
    let dag_genesis_hash = dag.dag_genesis_hash();
    let (dag_blocks_size, ghost_path_move_back, extra_data) =
        service.value_proposal_policy(live.period)?;
    let last_anchor = if head.last_non_null_pbft_dag_anchor_hash == H256::zero() {
        dag_genesis_hash
    } else {
        head.last_non_null_pbft_dag_anchor_hash
    };
    let ghost_path = dag.dag_ghost_path(DagGhostPathRoot::Block(last_anchor))?;
    let index = dag.dag_non_finalized_index()?;
    let fallback = index
        .levels
        .last()
        .and_then(|level| level.hashes.first())
        .copied();
    let final_hash = final_chain.pbft_final_chain_hash(live.period)?;
    let wallets = eligible_wallet_indices
        .iter()
        .map(|wallet_index| PbftManagerProposalWalletFact {
            wallet_index: *wallet_index,
            dpos_eligible: true,
            sortition_valid: true,
        })
        .collect();
    service.begin_proposal_session(PbftManagerProposalInitialFact {
        period: live.period,
        round: live.round,
        previous_pbft_block_hash: head.last_pbft_block_hash,
        last_period_dag_anchor_hash: last_anchor,
        dag_genesis_hash,
        dag_blocks_size,
        ghost_path_move_back,
        pbft_gas_limit: service.pbft_gas_limit_for_period(live.period),
        extra_data_required: extra_data.is_some(),
        extra_data_available: true,
        final_chain_hash_valid: final_hash.is_some(),
        final_chain_hash: H256(final_hash.unwrap_or_default()),
        wallets,
        ghost_path,
        has_non_finalized_fallback: fallback.is_some(),
        non_finalized_fallback_hash: fallback.unwrap_or_default(),
    });
    let Some(step) = service.proposal_session_next_with_dag(dag)? else {
        return Ok(ConsensusValueProposalAction::NoWork);
    };
    if step.action != PbftManagerProposalAction::BuildProposal {
        return Ok(ConsensusValueProposalAction::NoWork);
    }
    let mut reward_stream = RlpStream::new_list(reward_snapshot.records.len());
    for record in reward_snapshot.records {
        reward_stream.append(&record.hash);
    }
    let unsigned = prepare_value_proposal_signing(ConsensusValueProposalInput {
        previous_pbft_block_hash: step.previous_pbft_block_hash,
        anchor_hash: step.anchor_hash,
        order_hash: step.order_hash,
        final_chain_hash: step.final_chain_hash,
        period: live.period,
        timestamp,
        reward_votes_rlp: reward_stream.out().to_vec(),
        extra_data,
    })?;
    Ok(ConsensusValueProposalAction::Build {
        eligible_wallet_indices: step.eligible_wallet_indices,
        unsigned,
        reward_votes_bundle_rlp,
    })
}

pub fn prepare_value_proposal_signing(
    input: ConsensusValueProposalInput,
) -> Result<ConsensusUnsignedValueProposal> {
    ensure!(
        !input.reward_votes_rlp.is_empty(),
        "CONSENSUS_VALUE_PROPOSAL_REWARD_VOTES_EMPTY"
    );
    let unsigned = unsigned_rlp(&input);
    let mut hash = [0; 32];
    let mut keccak = Keccak::v256();
    keccak.update(&unsigned);
    keccak.finalize(&mut hash);
    Ok(ConsensusUnsignedValueProposal {
        input,
        signing_hash: hash,
    })
}

pub fn complete_value_proposal_signing(
    request: ConsensusUnsignedValueProposal,
    signature: Vec<u8>,
) -> Result<Vec<u8>> {
    let signature: [u8; 65] = signature
        .try_into()
        .map_err(|_| anyhow!("CONSENSUS_VALUE_PROPOSAL_INVALID_SIGNATURE_LENGTH"))?;
    let mut stream = RlpStream::new_list(if request.input.extra_data.is_some() {
        9
    } else {
        8
    });
    append_unsigned_fields(&mut stream, &request.input);
    stream.append(&signature.as_slice());
    let block = stream.out().to_vec();
    let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))?;
    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&block))?;
    ensure!(
        link.period == request.input.period,
        "CONSENSUS_VALUE_PROPOSAL_IDENTITY_MISMATCH"
    );
    ensure!(
        metadata.timestamp == request.input.timestamp,
        "CONSENSUS_VALUE_PROPOSAL_TIMESTAMP_MISMATCH"
    );
    Ok(block)
}

fn unsigned_rlp(input: &ConsensusValueProposalInput) -> Vec<u8> {
    let mut stream = RlpStream::new_list(if input.extra_data.is_some() { 8 } else { 7 });
    append_unsigned_fields(&mut stream, input);
    stream.out().to_vec()
}

fn append_unsigned_fields(stream: &mut RlpStream, input: &ConsensusValueProposalInput) {
    stream.append(&input.previous_pbft_block_hash);
    stream.append(&input.anchor_hash);
    stream.append(&input.order_hash);
    stream.append(&input.final_chain_hash);
    stream.append(&input.period);
    stream.append(&input.timestamp);
    stream.append_raw(&input.reward_votes_rlp, 1);
    if let Some(extra_data) = &input.extra_data {
        stream.append(extra_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    #[test]
    fn canonical_block_signing_round_trip() {
        let request = prepare_value_proposal_signing(ConsensusValueProposalInput {
            previous_pbft_block_hash: H256::repeat_byte(1),
            anchor_hash: H256::repeat_byte(2),
            order_hash: H256::repeat_byte(3),
            final_chain_hash: H256::repeat_byte(4),
            period: 7,
            timestamp: 9,
            reward_votes_rlp: vec![0xc0],
            extra_data: None,
        })
        .unwrap();
        let key = SigningKey::from_slice(&[0x42; 32]).unwrap();
        let (sig, recovery) = key.sign_prehash_recoverable(&request.signing_hash).unwrap();
        let mut bytes = sig.to_bytes().to_vec();
        bytes.push(recovery.to_byte());
        let block = complete_value_proposal_signing(request, bytes).unwrap();
        assert_eq!(
            PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block))
                .unwrap()
                .period,
            7
        );
        assert_eq!(rlp::Rlp::new(&block).item_count().unwrap(), 8);
        assert_eq!(
            PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&block))
                .unwrap()
                .timestamp,
            9
        );
    }

    #[test]
    fn ficus_block_places_extra_data_after_reward_votes() {
        let mut extra = RlpStream::new_list(6);
        extra
            .append(&1_u16)
            .append(&2_u16)
            .append(&3_u16)
            .append(&4_u16);
        extra.append(&b"T".to_vec()).append(&H256::repeat_byte(9));
        let extra = extra.out().to_vec();
        let request = prepare_value_proposal_signing(ConsensusValueProposalInput {
            previous_pbft_block_hash: H256::repeat_byte(1),
            anchor_hash: H256::repeat_byte(2),
            order_hash: H256::repeat_byte(3),
            final_chain_hash: H256::repeat_byte(4),
            period: 8,
            timestamp: 9,
            reward_votes_rlp: vec![0xc0],
            extra_data: Some(extra.clone()),
        })
        .unwrap();
        let key = SigningKey::from_slice(&[0x42; 32]).unwrap();
        let (sig, recovery) = key.sign_prehash_recoverable(&request.signing_hash).unwrap();
        let mut bytes = sig.to_bytes().to_vec();
        bytes.push(recovery.to_byte());
        let block = complete_value_proposal_signing(request, bytes).unwrap();
        let rlp = rlp::Rlp::new(&block);
        assert_eq!(rlp.item_count().unwrap(), 9);
        assert_eq!(rlp.at(6).unwrap().item_count().unwrap(), 0);
        assert_eq!(rlp.at(7).unwrap().data().unwrap(), extra.as_slice());
    }

    #[test]
    fn later_round_reuses_next_value_and_only_next_null_builds() {
        let certified = H256::repeat_byte(0x44);
        assert_eq!(
            decide_prior_round_value(2, Some(certified), false),
            PriorRoundValueDecision::Repropose(certified)
        );
        assert_eq!(
            decide_prior_round_value(2, None, true),
            PriorRoundValueDecision::Build
        );
        assert_eq!(
            decide_prior_round_value(2, None, false),
            PriorRoundValueDecision::NoWork
        );
        assert_eq!(
            decide_prior_round_value(1, None, false),
            PriorRoundValueDecision::Build
        );
    }

    #[test]
    fn later_period_rejects_empty_or_stale_reward_votes() {
        assert!(ensure_reward_snapshot_current(2, true, 1, true).is_err());
        assert!(ensure_reward_snapshot_current(3, true, 1, false).is_err());
        assert!(ensure_reward_snapshot_current(3, false, 0, false).is_err());
        assert!(ensure_reward_snapshot_current(3, true, 2, false).is_ok());
        assert!(ensure_reward_snapshot_current(1, false, 0, true).is_ok());
    }
}
