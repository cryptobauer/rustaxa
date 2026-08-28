//! Native cross-sibling PBFT leader selection.
//!
//! Leader selection snapshots proposal votes together with proposed-block and
//! finalized-chain membership, releases every native lock while composed
//! FinalChain/DAG validation runs, then rebuilds and fingerprints the same state
//! before publishing proposed-block validity. This module owns that complete
//! stale-safe task. The PBFT manager serialization domain protects finalized
//! membership writes while the sibling locks protect their live state; CXX
//! adapters only convert owned inputs and outputs.

use crate::{
    FinalChain,
    dag_transaction_service::DagTransactionService,
    pbft_manager::{
        PbftManagerBlockValidationAction, PbftManagerLeaderBlockValidationStatus,
        PbftManagerLeaderCandidateInputFact, PbftManagerLeaderSelectionStatus,
        plan_pbft_manager_leader_candidates,
    },
    pbft_service::{self, PbftProposedBlockAdmissionRequest, PbftService},
    pbft_vote_storage::PbftVoteStorageRecord,
    pbft_vote_validation::inspect_canonical_pbft_vote,
    proposed_blocks::{ProposedBlockEntry, ProposedBlocks},
    verified_votes::PbftVoteType,
};
use anyhow::{Result, anyhow};
use ethereum_types::H256;
use rustaxa_vdf::vrf;
use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};

/// Stable outcome of a native PBFT leader-selection task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PbftLeaderSelectionStatus {
    /// A snapshot is ready or a leader was selected.
    Selected,
    /// No proposal votes exist for the requested period and round.
    NoCandidates,
    /// Candidates exist but none is eligible after validation.
    NoEligible,
    /// Native state changed while the external validator was running.
    StaleSnapshot,
    /// The external report set or native planner output is invalid.
    InvalidValidationReport,
}

impl PbftLeaderSelectionStatus {
    /// Returns the stable CXX status code.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Typed external validation status for one prepared candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PbftLeaderCandidateValidationStatus {
    /// The retained external validator accepted the proposed block.
    Validated,
    /// The retained external validator rejected the proposed block.
    Rejected,
    /// The CXX adapter received an unknown status code.
    Invalid,
}

/// Owned proposal-vote and proposed-block facts captured before validation.
///
/// Candidate payloads remain valid after native locks are released. Every
/// field except the derived `needs_external_validation` flag participates in
/// the enclosing snapshot fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftLeaderCandidateSnapshot {
    /// Canonical proposal-vote hash.
    pub vote_hash: H256,
    /// Block hash named by the proposal vote.
    pub block_hash: H256,
    /// Retained weighted proposal-vote payload.
    pub vote_record: PbftVoteStorageRecord,
    /// Whether the proposed-block owner contains the block.
    pub proposed_block_found: bool,
    /// Whether the proposed block was already marked valid.
    pub proposed_block_is_valid: bool,
    /// Canonical proposed-block RLP, or empty when absent.
    pub proposed_block_rlp: Vec<u8>,
    /// Proposed block pivot hash, or zero when absent.
    pub pivot_hash: H256,
    /// Whether finalized PBFT storage already contains the block.
    pub block_in_chain: bool,
    /// Whether the retained C++ validator must inspect this candidate.
    pub needs_external_validation: bool,
}

/// Coherent candidate snapshot prepared for unlocked external validation.
///
/// Candidates are proposal-step votes for exactly `period` and `round`, sorted
/// by vote hash. `snapshot_fingerprint` binds their payloads, proposal state,
/// and finalized-chain membership so finish can reject stale work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftLeaderSelectionSnapshot {
    /// Snapshot status; ready snapshots use `Selected`.
    pub status: PbftLeaderSelectionStatus,
    /// Stable diagnostic string.
    pub error_code: String,
    /// Requested PBFT period.
    pub period: u64,
    /// Requested PBFT round.
    pub round: u64,
    /// V1 content fingerprint used for stale revalidation.
    pub snapshot_fingerprint: [u8; 32],
    /// Owned candidate payloads in deterministic vote-hash order.
    pub candidates: Vec<PbftLeaderCandidateSnapshot>,
}

/// External validation report for one candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftLeaderCandidateValidation {
    /// Vote identity from the prepared snapshot.
    pub vote_hash: H256,
    /// Block identity from the prepared snapshot.
    pub block_hash: H256,
    /// Typed external validation outcome.
    pub status: PbftLeaderCandidateValidationStatus,
}

/// Finish request bound to one exact prepared snapshot.
///
/// Finish requires exactly one report for each candidate whose snapshot asked
/// for external validation and no reports for any other candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftLeaderSelectionFinishRequest {
    /// Prepared PBFT period.
    pub period: u64,
    /// Prepared PBFT round.
    pub round: u64,
    /// Prepared V1 snapshot fingerprint.
    pub snapshot_fingerprint: [u8; 32],
    /// Exact external validation report set.
    pub validations: Vec<PbftLeaderCandidateValidation>,
}

/// Policy input for one complete authoritative leader-selection operation.
///
/// The period and round select already-verified proposal votes. Gas and
/// hardfork policy configure canonical proposed-block validation after the
/// native snapshot releases its locks. The operation returns typed empty,
/// ineligible, or stale results; malformed payloads, storage failures, and
/// non-terminal validation contracts are errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftComposedLeaderSelectionRequest {
    /// Requested PBFT period.
    pub period: u64,
    /// Requested PBFT round.
    pub round: u64,
    /// Gas limit used by DAG preparation.
    pub pbft_gas_limit: u64,
    /// Whether the active hardfork requires decodable block extra-data.
    pub extra_data_required: bool,
    /// Whether a local pillar anchor is required.
    pub pillar_block_required: bool,
}

/// Owned result of finishing one leader selection.
///
/// Selected results own the weighted vote and canonical block payload. Every
/// other status returns empty payloads and publishes no proposed-block validity
/// unless the native planner had already validated the complete command set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PbftLeaderSelectionResult {
    /// Terminal selection status.
    pub status: PbftLeaderSelectionStatus,
    /// Stable diagnostic string.
    pub error_code: String,
    /// Whether a leader payload is present.
    pub selected: bool,
    /// Selected weighted vote, or an empty record.
    pub selected_vote: PbftVoteStorageRecord,
    /// Selected canonical proposed-block RLP, or empty.
    pub selected_block_rlp: Vec<u8>,
}

impl PbftService {
    /// Prepares one coherent leader-selection snapshot.
    ///
    /// The task acquires the PBFT manager serialization domain before verified
    /// votes, proposed blocks, and PBFT chain state. The manager domain
    /// serializes finalized-membership storage writes while the sibling locks
    /// protect their live state. All guards are held while the task builds the
    /// candidate snapshot and V1 fingerprint, and released before callers run
    /// external validation. Manager-lock poison follows the manager service's
    /// invariant panic policy; sibling-lock poison is returned as an
    /// operational error. No state is mutated.
    pub fn prepare_leader_selection(
        &self,
        period: u64,
        round: u64,
    ) -> Result<PbftLeaderSelectionSnapshot> {
        let _manager = self.manager_state();
        let votes = self.verified_votes();
        let runtime = votes.lock()?;
        let proposed = self
            .proposed_blocks()
            .read()
            .map_err(|_| anyhow!("PBFT_SERVICE_PROPOSED_BLOCKS_LOCK_POISONED"))?;
        let _chain = self
            .chain()
            .read()
            .map_err(|_| anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?;
        build_leader_selection_snapshot(self, &runtime, &proposed, period, round)
    }

    /// Revalidates and finishes one prepared leader selection.
    ///
    /// The task reacquires the manager, verified-votes, proposed-blocks, then
    /// PBFT-chain domains, rebuilds the complete fingerprint, validates the
    /// exact external report set, invokes the native leader planner, and
    /// prevalidates every selected identity and mark-valid command before
    /// publishing any validity change. Stale or malformed input returns a
    /// typed non-selected result.
    pub fn finish_leader_selection(
        &self,
        request: PbftLeaderSelectionFinishRequest,
    ) -> Result<PbftLeaderSelectionResult> {
        let _manager = self.manager_state();
        let votes = self.verified_votes();
        let runtime = votes.lock()?;
        let mut proposed = self
            .proposed_blocks()
            .write()
            .map_err(|_| anyhow!("PBFT_SERVICE_PROPOSED_BLOCKS_LOCK_POISONED"))?;
        let _chain = self
            .chain()
            .read()
            .map_err(|_| anyhow!("PBFT_SERVICE_CHAIN_LOCK_POISONED"))?;
        let snapshot = build_leader_selection_snapshot(
            self,
            &runtime,
            &proposed,
            request.period,
            request.round,
        )?;
        if snapshot.snapshot_fingerprint != request.snapshot_fingerprint {
            return Ok(empty_result(
                PbftLeaderSelectionStatus::StaleSnapshot,
                "PBFT_LEADER_SELECTION_STALE_SNAPSHOT",
            ));
        }

        let validations = match validate_reports(&snapshot.candidates, request.validations) {
            Ok(validations) => validations,
            Err(()) => {
                return Ok(empty_result(
                    PbftLeaderSelectionStatus::InvalidValidationReport,
                    "PBFT_LEADER_SELECTION_INVALID_VALIDATION_REPORT",
                ));
            }
        };

        let mut facts = Vec::with_capacity(snapshot.candidates.len());
        for candidate in &snapshot.candidates {
            let inspection = inspect_canonical_pbft_vote(&candidate.vote_record.vote_rlp)?;
            let validation_status = if candidate.proposed_block_is_valid {
                PbftManagerLeaderBlockValidationStatus::AlreadyValid
            } else {
                match validations.get(&candidate.vote_hash).copied() {
                    Some(PbftLeaderCandidateValidationStatus::Validated) => {
                        PbftManagerLeaderBlockValidationStatus::Validated
                    }
                    Some(PbftLeaderCandidateValidationStatus::Rejected) | None => {
                        PbftManagerLeaderBlockValidationStatus::Rejected
                    }
                    Some(PbftLeaderCandidateValidationStatus::Invalid) => {
                        unreachable!("invalid statuses were rejected")
                    }
                }
            };
            facts.push(PbftManagerLeaderCandidateInputFact {
                vote_hash: candidate.vote_hash,
                block_hash: candidate.block_hash,
                period: request.period,
                credential: vrf::proof_to_hash(&inspection.vrf_proof)?,
                voter_public_key: inspection.recovered_public_key,
                weight_found: inspection.has_embedded_weight,
                weight: inspection.embedded_weight,
                block_in_chain: candidate.block_in_chain,
                proposed_block_found: candidate.proposed_block_found,
                block_validation_status: validation_status,
                pivot_hash: candidate.pivot_hash,
            });
        }

        let plan = plan_pbft_manager_leader_candidates(facts);
        if plan.status == PbftManagerLeaderSelectionStatus::InvalidFact {
            return Ok(empty_result(
                PbftLeaderSelectionStatus::InvalidValidationReport,
                plan.error_code,
            ));
        }

        let selected = if plan.selected {
            snapshot.candidates.iter().find(|candidate| {
                candidate.vote_hash == plan.selected_vote_hash
                    && candidate.block_hash == plan.selected_block_hash
            })
        } else {
            None
        };
        if plan.selected && selected.is_none() {
            return Err(anyhow!(
                "PBFT_LEADER_SELECTION_PLANNER_SELECTED_UNKNOWN_CANDIDATE"
            ));
        }

        // Validate the whole publication set before mutating any entry.
        for command in &plan.valid_blocks {
            let Some(candidate) = snapshot.candidates.iter().find(|candidate| {
                candidate.block_hash == command.block_hash
                    && request.period == command.period
                    && candidate.proposed_block_found
            }) else {
                return Err(anyhow!(
                    "PBFT_LEADER_SELECTION_PLANNER_MARKED_UNKNOWN_CANDIDATE"
                ));
            };
            if proposed.get(command.period, command.block_hash).is_none()
                || candidate.block_in_chain
            {
                return Err(anyhow!(
                    "PBFT_LEADER_SELECTION_PLANNER_MARK_VALID_PRECONDITION_FAILED"
                ));
            }
        }
        for command in &plan.valid_blocks {
            proposed.mark_valid(command.period, command.block_hash)?;
        }

        let Some(selected) = selected else {
            return Ok(empty_result(
                if snapshot.candidates.is_empty() {
                    PbftLeaderSelectionStatus::NoCandidates
                } else {
                    PbftLeaderSelectionStatus::NoEligible
                },
                plan.error_code,
            ));
        };
        Ok(PbftLeaderSelectionResult {
            status: PbftLeaderSelectionStatus::Selected,
            error_code: plan.error_code.to_owned(),
            selected: true,
            selected_vote: selected.vote_record.clone(),
            selected_block_rlp: selected.proposed_block_rlp.clone(),
        })
    }

    /// Selects one leader candidate in a single composed request.
    ///
    /// The method reuses snapshot and finish operations while validating each
    /// externally validated candidate through canonical block decoding and native
    /// composed validation against FinalChain and DAG.
    pub fn select_leader_composed(
        &self,
        final_chain: &FinalChain,
        dag_transaction_service: &DagTransactionService,
        request: PbftComposedLeaderSelectionRequest,
    ) -> Result<PbftLeaderSelectionResult> {
        let snapshot = self.prepare_leader_selection(request.period, request.round)?;
        let mut validations = Vec::with_capacity(snapshot.candidates.len());

        for candidate in &snapshot.candidates {
            if !candidate.needs_external_validation {
                continue;
            }
            let entry = ProposedBlockEntry {
                period: request.period,
                block_hash: candidate.block_hash,
                block_rlp: candidate.proposed_block_rlp.clone(),
                pivot_hash: candidate.pivot_hash,
                is_valid: candidate.proposed_block_is_valid,
            };
            let composed = pbft_service::proposed_block_validation_candidate(
                &entry,
                PbftProposedBlockAdmissionRequest {
                    period: request.period,
                    block_hash: candidate.block_hash,
                    pbft_gas_limit: request.pbft_gas_limit,
                    extra_data_required: request.extra_data_required,
                    pillar_block_required: request.pillar_block_required,
                },
            )?;
            let validation_plan =
                self.validate_pbft_block_composed(final_chain, dag_transaction_service, composed)?;
            let status = match validation_plan.action {
                PbftManagerBlockValidationAction::Accept => {
                    PbftLeaderCandidateValidationStatus::Validated
                }
                PbftManagerBlockValidationAction::Reject
                | PbftManagerBlockValidationAction::WaitForFinalization => {
                    PbftLeaderCandidateValidationStatus::Rejected
                }
                PbftManagerBlockValidationAction::ContractError => {
                    return Err(anyhow!(
                        "PBFT_COMPOSED_LEADER_SELECTION_VALIDATION_CONTRACT_ERROR: {}",
                        validation_plan.error_code
                    ));
                }
                PbftManagerBlockValidationAction::RunCheck => {
                    return Err(anyhow!(
                        "PBFT_COMPOSED_LEADER_SELECTION_VALIDATION_NON_TERMINAL: {}",
                        validation_plan.error_code
                    ));
                }
            };
            validations.push(PbftLeaderCandidateValidation {
                vote_hash: candidate.vote_hash,
                block_hash: candidate.block_hash,
                status,
            });
        }

        self.finish_leader_selection(PbftLeaderSelectionFinishRequest {
            period: request.period,
            round: request.round,
            snapshot_fingerprint: snapshot.snapshot_fingerprint,
            validations,
        })
    }
}

fn build_leader_selection_snapshot(
    service: &PbftService,
    runtime: &crate::pbft_vote_runtime::PbftVoteAdmissionRuntime,
    proposed: &ProposedBlocks,
    period: u64,
    round: u64,
) -> Result<PbftLeaderSelectionSnapshot> {
    let mut candidates = Vec::new();
    for vote in runtime
        .verified_votes()
        .snapshot_votes()
        .into_iter()
        .filter(|vote| {
            vote.period == period
                && vote.round == round
                && vote.step == 1
                && vote.vote_type == PbftVoteType::Propose
        })
    {
        let weighted_payload = runtime
            .weighted_payload(vote.vote_hash)
            .cloned()
            .ok_or_else(|| anyhow!("PBFT_LEADER_SELECTION_MISSING_VOTE_PAYLOAD"))?;
        let vote_record = PbftVoteStorageRecord {
            hash: weighted_payload.hash,
            vote_rlp: weighted_payload.vote_rlp,
        };
        let proposed_block = proposed.get(period, vote.block_hash);
        let block_in_chain = if vote.block_hash.is_zero() {
            false
        } else {
            service.chain().block_exists(vote.block_hash)?
        };
        let proposed_block_found = proposed_block.is_some();
        let proposed_block_is_valid = proposed_block
            .as_ref()
            .map(|block| block.is_valid)
            .unwrap_or(false);
        candidates.push(PbftLeaderCandidateSnapshot {
            vote_hash: vote.vote_hash,
            block_hash: vote.block_hash,
            vote_record,
            proposed_block_found,
            proposed_block_is_valid,
            proposed_block_rlp: proposed_block
                .as_ref()
                .map(|block| block.block_rlp.clone())
                .unwrap_or_default(),
            pivot_hash: proposed_block
                .as_ref()
                .map(|block| block.pivot_hash)
                .unwrap_or_default(),
            block_in_chain,
            needs_external_validation: !vote.block_hash.is_zero()
                && !block_in_chain
                && proposed_block_found
                && !proposed_block_is_valid,
        });
    }
    candidates.sort_by_key(|candidate| candidate.vote_hash);
    let snapshot_fingerprint = fingerprint(period, round, &candidates);
    Ok(PbftLeaderSelectionSnapshot {
        status: if candidates.is_empty() {
            PbftLeaderSelectionStatus::NoCandidates
        } else {
            PbftLeaderSelectionStatus::Selected
        },
        error_code: if candidates.is_empty() {
            "PBFT_LEADER_SELECTION_NO_CANDIDATES"
        } else {
            "PBFT_LEADER_SELECTION_READY"
        }
        .to_owned(),
        period,
        round,
        snapshot_fingerprint,
        candidates,
    })
}

fn validate_reports(
    candidates: &[PbftLeaderCandidateSnapshot],
    reports: Vec<PbftLeaderCandidateValidation>,
) -> std::result::Result<BTreeMap<H256, PbftLeaderCandidateValidationStatus>, ()> {
    let expected = candidates
        .iter()
        .filter(|candidate| candidate.needs_external_validation)
        .map(|candidate| (candidate.vote_hash, candidate.block_hash))
        .collect::<BTreeMap<_, _>>();
    if reports.len() != expected.len() {
        return Err(());
    }
    let mut seen = BTreeSet::new();
    let mut validated = BTreeMap::new();
    for report in reports {
        if !seen.insert(report.vote_hash)
            || expected.get(&report.vote_hash).copied() != Some(report.block_hash)
            || report.status == PbftLeaderCandidateValidationStatus::Invalid
        {
            return Err(());
        }
        validated.insert(report.vote_hash, report.status);
    }
    Ok(validated)
}

fn fingerprint(period: u64, round: u64, candidates: &[PbftLeaderCandidateSnapshot]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(b"RUSTAXA_PBFT_LEADER_SELECTION_V1");
    hasher.update(&period.to_be_bytes());
    hasher.update(&round.to_be_bytes());
    hasher.update(&(candidates.len() as u64).to_be_bytes());
    for candidate in candidates {
        hasher.update(candidate.vote_hash.as_bytes());
        hasher.update(candidate.block_hash.as_bytes());
        hasher.update(candidate.vote_record.hash.as_bytes());
        hasher.update(&keccak256(&candidate.vote_record.vote_rlp));
        hasher.update(&[u8::from(candidate.proposed_block_found)]);
        hasher.update(&[u8::from(candidate.proposed_block_is_valid)]);
        hasher.update(candidate.pivot_hash.as_bytes());
        hasher.update(&keccak256(&candidate.proposed_block_rlp));
        hasher.update(&[u8::from(candidate.block_in_chain)]);
    }
    let mut output = [0; 32];
    hasher.finalize(&mut output);
    output
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut output = [0; 32];
    hasher.finalize(&mut output);
    output
}

fn empty_result(status: PbftLeaderSelectionStatus, error_code: &str) -> PbftLeaderSelectionResult {
    PbftLeaderSelectionResult {
        status,
        error_code: error_code.to_owned(),
        selected: false,
        selected_vote: PbftVoteStorageRecord {
            hash: H256::zero(),
            vote_rlp: Vec::new(),
        },
        selected_block_rlp: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dag::{dag_manager_block_from_rlp, save_dag_block_to_storage},
        dag_service::DagServiceConfig,
        dag_transaction_service::{DagTransactionService, DagTransactionServiceConfig},
        gas_pricer::GasPricerConfig,
        pbft_service::PbftServiceConfig,
        pbft_vote_generation::{PbftVoteGenerationInput, generate_pbft_vote},
        pbft_vote_payload::build_weighted_pbft_vote_payload,
        sortition::{SortitionConfig, SortitionParams, VdfParams, VrfParams},
        transaction_service::TransactionServiceConfig,
        verified_votes::VerifiedVote,
    };
    use ethereum_types::{H160, U256};
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_storage::{Config, Storage};
    use rustaxa_types::{
        DposTokenAmount, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
        codec::rlp::pbft::SignedPbftBlockRlp, pbft::PbftBlockLink,
    };
    use std::time::Duration;
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    const NODE_SECRET: [u8; 32] = [0x35; 32];
    const NODE_SECRET_TWO: [u8; 32] = [0x42; 32];
    const VRF_SECRET: [u8; 64] = [
        0x90, 0xf5, 0x9a, 0x7e, 0xe7, 0xa3, 0x92, 0xc8, 0x11, 0xc5, 0xd2, 0x99, 0xb5, 0x57, 0xa4,
        0xe0, 0x9e, 0x61, 0x0d, 0xe7, 0xd1, 0x09, 0xd6, 0xb3, 0xfc, 0xb1, 0x9a, 0xb8, 0xd5, 0x1c,
        0x9a, 0x0d, 0x93, 0x1f, 0x5e, 0x7d, 0xb0, 0x7c, 0x99, 0x69, 0xe4, 0x38, 0xdb, 0x7e, 0x28,
        0x7e, 0xab, 0xba, 0xac, 0xa4, 0x9c, 0xa4, 0x14, 0xf5, 0xf3, 0xa4, 0x02, 0xea, 0x69, 0x97,
        0xad, 0xe4, 0x00, 0x81,
    ];

    fn test_service(name: &str) -> (PbftService, Arc<Storage>) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rustaxa_consensus_pbft_leader_{name}_{}_{}",
            std::process::id(),
            nonce
        ));
        let storage = Arc::new(Storage::new(Config::new(path)).unwrap());
        let service = PbftService::restore(
            storage.clone(),
            PbftServiceConfig {
                genesis_lambda_ms: 100,
                cacti_lambda_max_ms: 100,
                cacti_lambda_default_ms: 100,
                cacti_block: u64::MAX,
                max_exponential_lambda_ms: 60_000,
                max_steps: 13,
                deadline_ms: 400,
                polling_interval_ms: 100,
                report_malicious_behaviour: true,
                magnolia_activation_period: 0,
                ficus_activation_period: 0,
                pillar_blocks_interval: 10,
                sync_level_size: 10,
                deep_syncing_threshold: 5,
                is_light_node: false,
                light_node_history: 0,
                network_identity: crate::network_api::NetworkNodeIdentity::default(),
                committee_size: 1,
                number_of_proposers: 1,
                dag_blocks_size: 50,
                ghost_path_move_back: 0,
                node_version: (0, 0, 0, 0),
                node_version_suffix: b"T".to_vec(),
                default_pbft_gas_limit: 1_000_000,
                cornus_activation_period: u64::MAX,
                cornus_pbft_gas_limit: 1_000_000,
                process_synced_policy: crate::pbft_service::PbftProcessSyncedPolicy {
                    chain_id: 2999,
                    lambda_min_ms: 100,
                    lambda_change_interval: 10,
                    lambda_change_ms: 10,
                    consensus_delay_ms: 400,
                    dpos_blocks_per_year: 500,
                    recently_finalized_factor: 3,
                },
            },
        )
        .unwrap();
        (service, storage)
    }

    fn voter_from_secret(secret: &[u8; 32]) -> H160 {
        let key = SigningKey::from_slice(secret).unwrap();
        let public_key = key.verifying_key().to_encoded_point(false);
        H160::from_slice(&keccak256(&public_key.as_bytes()[1..])[12..])
    }

    fn insert_proposal_vote(
        service: &PbftService,
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        weight: u64,
    ) -> H256 {
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Propose,
            period: 12,
            round: 2,
            step: 1,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        let vote = VerifiedVote::new(
            generated.vote_hash,
            generated.block_hash,
            generated.voter,
            generated.period,
            generated.round,
            generated.step,
            generated.vote_type,
            weight,
        )
        .unwrap();
        let weighted = build_weighted_pbft_vote_payload(&generated.vote_rlp, weight).unwrap();
        let mut runtime = service.verified_votes().lock().unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote.clone(), None)
            .unwrap();
        runtime.retain_weighted_payload(&vote, weighted).unwrap();
        generated.vote_hash
    }

    fn insert_proposed_block(
        service: &PbftService,
        block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        block_rlp: Vec<u8>,
    ) {
        insert_proposed_block_with_period(service, 12, block_hash, pivot_hash, block_rlp);
    }

    fn insert_proposed_block_with_period(
        service: &PbftService,
        period: u64,
        block_hash: [u8; 32],
        pivot_hash: [u8; 32],
        block_rlp: Vec<u8>,
    ) {
        assert!(service.proposed_blocks().write().unwrap().push(
            period,
            H256::from(block_hash),
            H256::from(pivot_hash),
            block_rlp,
        ));
    }

    fn insert_proposal_vote_with_period(
        service: &PbftService,
        block_hash: [u8; 32],
        node_secret: [u8; 32],
        weight: u64,
        period: u64,
    ) -> H256 {
        let generated = generate_pbft_vote(PbftVoteGenerationInput {
            block_hash: block_hash.into(),
            vote_type: PbftVoteType::Propose,
            period,
            round: 2,
            step: 1,
            node_secret,
            vrf_secret: VRF_SECRET,
            expected_voter: voter_from_secret(&node_secret),
            expected_vrf_public_key: vrf::public_key_from_secret(&VRF_SECRET).unwrap(),
        })
        .unwrap();
        let vote = VerifiedVote::new(
            generated.vote_hash,
            generated.block_hash,
            generated.voter,
            generated.period,
            generated.round,
            generated.step,
            generated.vote_type,
            weight,
        )
        .unwrap();
        let weighted = build_weighted_pbft_vote_payload(&generated.vote_rlp, weight).unwrap();
        let mut runtime = service.verified_votes().lock().unwrap();
        runtime
            .verified_votes_mut()
            .add_verified_vote(vote.clone(), None)
            .unwrap();
        runtime.retain_weighted_payload(&vote, weighted).unwrap();
        generated.vote_hash
    }

    fn test_dag_transaction_service_config() -> DagTransactionServiceConfig {
        DagTransactionServiceConfig {
            transaction: TransactionServiceConfig {
                queue_max_size: 16,
                gas_pricer_config: GasPricerConfig {
                    percentile: 50,
                    minimum_price: U256::one(),
                    history_blocks: 0,
                    is_light_node: false,
                    blocks_gas_pricer: false,
                },
                proposal_dag_gas_limit: 1_000_000,
            },
            dag: DagServiceConfig {
                genesis_hash: H256::repeat_byte(1),
                dag_expiry_limit: 32,
                max_levels_per_period: 100,
            },
            sortition: SortitionConfig {
                params: SortitionParams {
                    vrf: VrfParams {
                        threshold_upper: 0x100,
                    },
                    vdf: VdfParams {
                        difficulty_min: 1,
                        difficulty_max: 10,
                        difficulty_stale: 5,
                        lambda_bound: 100,
                    },
                },
                changes_count_for_average: 8,
                dag_efficiency_targets: (5_000, 10_000),
                changing_interval: 10,
                computation_interval: 5,
            },
        }
    }

    fn test_dag_transaction_service(storage: &Arc<Storage>) -> DagTransactionService {
        DagTransactionService::restore(storage.clone(), test_dag_transaction_service_config())
            .unwrap()
    }

    fn test_final_chain(storage: &Arc<Storage>) -> FinalChain {
        let stake = U256::from(5_000u64).to_big_endian().to_vec();
        FinalChain::new(
            storage.clone(),
            0.into(),
            0,
            Vec::new(),
            vec![GenesisValidator {
                address: [1u8; 20],
                vrf_key: [1u8; 32],
                total_stake: stake.clone(),
                delegations: vec![([1u8; 20], stake)],
                metadata: GenesisValidatorMetadata {
                    owner: [1u8; 20],
                    commission: 0,
                    description: String::new(),
                    endpoint: String::new(),
                },
            }],
            GenesisDposConfig {
                eligibility_balance_threshold: DposTokenAmount::from(U256::from(1_000u64)),
                vote_eligibility_balance_step: DposTokenAmount::from(U256::from(1_000u64)),
                validator_maximum_stake: DposTokenAmount::from(U256::from(30_000u64)),
                minimum_deposit: DposTokenAmount::zero(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                delegation_delay: 2,
                dag_vdf_sortition_total_vote_count_until_period: 0.into(),
            },
        )
        .unwrap()
    }

    fn test_dag_order_hash(
        payload: &crate::dag_service::DagRuntimeNonFinalizedSyncPayload,
    ) -> H256 {
        let mut stream = RlpStream::new_list(1);
        stream.begin_list(payload.storage.blocks.len());
        for block in &payload.storage.blocks {
            let bytes: &[u8] = block.hash.as_bytes();
            stream.append(&bytes);
        }
        let mut out = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&stream.out());
        hasher.finalize(&mut out);
        H256(out)
    }

    fn dag_block_with_payload(
        pivot_hash: H256,
        level: u64,
        transactions: &[H256],
        gas_estimation: u64,
    ) -> Vec<u8> {
        let mut vdf = RlpStream::new_list(4);
        vdf.append(&vec![0x11_u8; 80]);
        vdf.append(&vec![0x22_u8]);
        vdf.append(&vec![0x33_u8]);
        vdf.append(&1_u16);

        let mut block = RlpStream::new_list(8);
        block.append(&pivot_hash);
        block.append(&level);
        block.append(&0_u64);
        block.append(&vdf.out().to_vec());
        block.begin_list(0);
        block.begin_list(transactions.len());
        for hash in transactions {
            block.append(hash);
        }
        block.append(&&[0_u8; 65][..]);
        block.append(&gas_estimation);
        block.out().to_vec()
    }

    fn composed_signed_pbft_block(period: u64) -> (Vec<u8>, PbftBlockLink) {
        let append_unsigned = |stream: &mut RlpStream| {
            stream.append(&H256::from_low_u64_be(period));
            stream.append(&H256::from_low_u64_be(period + 1));
            stream.append(&H256::from_low_u64_be(period + 2));
            stream.append(&H256::from_low_u64_be(period + 2));
            stream.append(&period);
            stream.append(&1_u64);
            stream.begin_list(0);
        };
        let mut unsigned = RlpStream::new_list(7);
        append_unsigned(&mut unsigned);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let signing_key = SigningKey::from_slice(&[0x63; 32]).unwrap();
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id.to_byte());

        let mut stream = RlpStream::new_list(8);
        append_unsigned(&mut stream);
        stream.append(&signature);
        let block_rlp = stream.out().to_vec();
        let link =
            PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp)).expect("block link parse");
        (block_rlp, link)
    }

    fn composed_signed_pbft_block_with_parts(
        period: u64,
        pivot_hash: H256,
        order_hash: H256,
        final_chain_hash: H256,
    ) -> (Vec<u8>, PbftBlockLink) {
        let append_unsigned = |stream: &mut RlpStream| {
            stream.append(&H256::zero());
            stream.append(&pivot_hash);
            stream.append(&order_hash);
            stream.append(&final_chain_hash);
            stream.append(&period);
            stream.append(&0_u64);
            stream.begin_list(0);
        };
        let mut unsigned = RlpStream::new_list(7);
        append_unsigned(&mut unsigned);
        let mut digest = [0_u8; 32];
        let mut hasher = Keccak::v256();
        hasher.update(&unsigned.out());
        hasher.finalize(&mut digest);
        let signing_key = SigningKey::from_slice(&[0x63; 32]).unwrap();
        let (signature, recovery_id) = signing_key.sign_prehash_recoverable(&digest).unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery_id.to_byte());

        let mut stream = RlpStream::new_list(8);
        append_unsigned(&mut stream);
        stream.append(&signature);
        let block_rlp = stream.out().to_vec();
        let link =
            PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp)).expect("block link parse");
        (block_rlp, link)
    }

    fn validation(
        candidate: &PbftLeaderCandidateSnapshot,
        status: PbftLeaderCandidateValidationStatus,
    ) -> PbftLeaderCandidateValidation {
        PbftLeaderCandidateValidation {
            vote_hash: candidate.vote_hash,
            block_hash: candidate.block_hash,
            status,
        }
    }

    fn finish_request(
        snapshot: &PbftLeaderSelectionSnapshot,
        validations: Vec<PbftLeaderCandidateValidation>,
    ) -> PbftLeaderSelectionFinishRequest {
        PbftLeaderSelectionFinishRequest {
            period: snapshot.period,
            round: snapshot.round,
            snapshot_fingerprint: snapshot.snapshot_fingerprint,
            validations,
        }
    }

    #[test]
    fn select_leader_composed_no_candidate() {
        let (service, storage) = test_service("select_composed_no_candidate");
        let final_chain = test_final_chain(&storage);
        let dag_transaction_service = test_dag_transaction_service(&storage);

        let result = service
            .select_leader_composed(
                &final_chain,
                &dag_transaction_service,
                PbftComposedLeaderSelectionRequest {
                    period: 12,
                    round: 2,
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            )
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::NoCandidates);
        assert!(!result.selected);
    }

    #[test]
    fn select_leader_composed_selected() {
        let (service, storage) = test_service("select_composed_selected");
        let final_chain = test_final_chain(&storage);
        let dag_transaction_service = test_dag_transaction_service(&storage);
        let selected_period = 1;
        let dag_block_rlp = dag_block_with_payload(H256::repeat_byte(1), 1, &[], 0);
        let dag_block = dag_manager_block_from_rlp(&dag_block_rlp).unwrap();
        dag_transaction_service
            .lock_dag()
            .unwrap()
            .state
            .add_block(dag_manager_block_from_rlp(&dag_block_rlp).unwrap())
            .unwrap();
        let dag_block_hash = dag_block.hash;
        save_dag_block_to_storage(storage.as_ref(), dag_block_hash, 1, 0, &dag_block_rlp).unwrap();
        let prepared = dag_transaction_service
            .prepare_pbft_candidate_payload(selected_period, dag_block_hash)
            .unwrap()
            .expect("dag candidate payload");
        let order_hash = test_dag_order_hash(&prepared.payload);

        let (block_rlp, link) = composed_signed_pbft_block_with_parts(
            selected_period,
            dag_block_hash,
            order_hash,
            H256::zero(),
        );
        let block_hash: [u8; 32] = link.block_hash.into();
        let pivot_hash: [u8; 32] = link.pivot_dag_block_hash.into();
        let vote_hash =
            insert_proposal_vote_with_period(&service, block_hash, NODE_SECRET, 4, selected_period);
        insert_proposed_block_with_period(
            &service,
            selected_period,
            block_hash,
            pivot_hash,
            block_rlp.clone(),
        );

        let entry = service
            .proposed_blocks()
            .read()
            .unwrap()
            .get(selected_period, link.block_hash)
            .unwrap()
            .clone();
        let candidate = pbft_service::proposed_block_validation_candidate(
            &entry,
            PbftProposedBlockAdmissionRequest {
                period: selected_period,
                block_hash: link.block_hash,
                pbft_gas_limit: 1_000_000,
                extra_data_required: false,
                pillar_block_required: false,
            },
        )
        .unwrap();
        let validation = service
            .validate_pbft_block_composed(&final_chain, &dag_transaction_service, candidate)
            .unwrap();
        assert_eq!(
            validation.action,
            PbftManagerBlockValidationAction::Accept,
            "{validation:?}"
        );

        let result = service
            .select_leader_composed(
                &final_chain,
                &dag_transaction_service,
                PbftComposedLeaderSelectionRequest {
                    period: selected_period,
                    round: 2,
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            )
            .unwrap();
        assert_eq!(
            result.status,
            PbftLeaderSelectionStatus::Selected,
            "{result:?}"
        );
        assert!(result.selected);
        assert_eq!(result.selected_vote.hash, vote_hash);
        assert_eq!(result.selected_block_rlp, block_rlp);
        assert!(
            service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(selected_period, link.block_hash)
                .unwrap()
                .is_valid
        );
    }

    #[test]
    fn select_leader_composed_rejected() {
        let (service, storage) = test_service("select_composed_rejected");
        let final_chain = test_final_chain(&storage);
        let dag_transaction_service = test_dag_transaction_service(&storage);
        let (block_rlp, link) = composed_signed_pbft_block(12);
        let block_hash: [u8; 32] = link.block_hash.into();

        insert_proposal_vote(&service, block_hash, NODE_SECRET, 4);
        insert_proposed_block(
            &service,
            block_hash,
            link.pivot_dag_block_hash.into(),
            block_rlp,
        );

        let result = service
            .select_leader_composed(
                &final_chain,
                &dag_transaction_service,
                PbftComposedLeaderSelectionRequest {
                    period: 12,
                    round: 2,
                    pbft_gas_limit: 1_000_000,
                    extra_data_required: false,
                    pillar_block_required: false,
                },
            )
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::NoEligible);
        assert!(!result.selected);
        assert!(
            !service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(12, H256::from(block_hash))
                .unwrap()
                .is_valid
        );
    }

    #[test]
    fn prepare_empty_and_deterministically_fingerprints_sorted_candidates() {
        let (service, _storage) = test_service("prepare_order");
        let empty = service.prepare_leader_selection(12, 2).unwrap();
        assert_eq!(empty.status, PbftLeaderSelectionStatus::NoCandidates);
        assert!(empty.candidates.is_empty());
        let empty_result = service
            .finish_leader_selection(finish_request(&empty, Vec::new()))
            .unwrap();
        assert_eq!(empty_result.status, PbftLeaderSelectionStatus::NoCandidates);

        insert_proposal_vote(&service, [0x41; 32], NODE_SECRET_TWO, 2);
        insert_proposal_vote(&service, [0x42; 32], NODE_SECRET, 3);
        insert_proposed_block(&service, [0x41; 32], [0x51; 32], vec![0x41, 0x01]);
        insert_proposed_block(&service, [0x42; 32], [0x52; 32], vec![0x42, 0x02]);
        let first = service.prepare_leader_selection(12, 2).unwrap();
        let second = service.prepare_leader_selection(12, 2).unwrap();
        assert_eq!(first.snapshot_fingerprint, second.snapshot_fingerprint);
        assert_eq!(first.candidates.len(), 2);
        assert!(first.candidates[0].vote_hash < first.candidates[1].vote_hash);
        assert!(
            first
                .candidates
                .iter()
                .all(|candidate| candidate.needs_external_validation)
        );
    }

    #[test]
    fn prepare_preserves_missing_already_valid_and_in_chain_states() {
        let (service, storage) = test_service("prepare_states");
        let missing_vote = insert_proposal_vote(&service, [0x43; 32], NODE_SECRET, 2);
        let valid_vote = insert_proposal_vote(&service, [0x44; 32], NODE_SECRET_TWO, 2);
        insert_proposed_block(&service, [0x44; 32], [0x54; 32], vec![0x44]);
        service
            .proposed_blocks()
            .write()
            .unwrap()
            .mark_valid(12, H256::from([0x44; 32]))
            .unwrap();
        let in_chain_vote = insert_proposal_vote(&service, [0x45; 32], [0x52; 32], 2);
        insert_proposed_block(&service, [0x45; 32], [0x55; 32], vec![0x45]);
        storage
            .period()
            .write_pbft_period(H256::from([0x45; 32]), 12)
            .unwrap();

        let snapshot = service.prepare_leader_selection(12, 2).unwrap();
        let missing = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == missing_vote)
            .unwrap();
        assert!(!missing.proposed_block_found);
        assert!(!missing.needs_external_validation);
        let valid = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == valid_vote)
            .unwrap();
        assert!(valid.proposed_block_is_valid);
        assert!(!valid.needs_external_validation);
        let in_chain = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == in_chain_vote)
            .unwrap();
        assert!(in_chain.block_in_chain);
        assert!(!in_chain.needs_external_validation);

        let result = service
            .finish_leader_selection(finish_request(&snapshot, Vec::new()))
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::Selected);
        assert_eq!(result.selected_vote.hash, valid_vote);
    }

    #[test]
    fn finish_accepts_or_rejects_without_extra_materialization_reads() {
        let (accepted_service, _storage) = test_service("finish_accepted");
        insert_proposal_vote(&accepted_service, [0x46; 32], NODE_SECRET, 4);
        insert_proposed_block(&accepted_service, [0x46; 32], [0x56; 32], vec![0x46, 0x99]);
        let snapshot = accepted_service.prepare_leader_selection(12, 2).unwrap();
        let result = accepted_service
            .finish_leader_selection(finish_request(
                &snapshot,
                vec![validation(
                    &snapshot.candidates[0],
                    PbftLeaderCandidateValidationStatus::Validated,
                )],
            ))
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::Selected);
        assert!(result.selected);
        assert_eq!(result.selected_vote.hash, snapshot.candidates[0].vote_hash);
        assert_eq!(result.selected_block_rlp, vec![0x46, 0x99]);
        assert!(
            accepted_service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(12, H256::from([0x46; 32]))
                .unwrap()
                .is_valid
        );

        let (rejected_service, _storage) = test_service("finish_rejected");
        insert_proposal_vote(&rejected_service, [0x47; 32], NODE_SECRET, 4);
        insert_proposed_block(&rejected_service, [0x47; 32], [0x57; 32], vec![0x47]);
        let snapshot = rejected_service.prepare_leader_selection(12, 2).unwrap();
        let result = rejected_service
            .finish_leader_selection(finish_request(
                &snapshot,
                vec![validation(
                    &snapshot.candidates[0],
                    PbftLeaderCandidateValidationStatus::Rejected,
                )],
            ))
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::NoEligible);
        assert!(!result.selected);
        assert!(
            !rejected_service
                .proposed_blocks()
                .read()
                .unwrap()
                .get(12, H256::from([0x47; 32]))
                .unwrap()
                .is_valid
        );
    }

    #[test]
    fn finish_publishes_the_complete_prevalidated_command_set() {
        let (service, _storage) = test_service("finish_complete_publication");
        insert_proposal_vote(&service, [0x61; 32], NODE_SECRET, 4);
        insert_proposal_vote(&service, [0x62; 32], NODE_SECRET_TWO, 3);
        insert_proposed_block(&service, [0x61; 32], [0x71; 32], vec![0x61]);
        insert_proposed_block(&service, [0x62; 32], [0x72; 32], vec![0x62]);
        let snapshot = service.prepare_leader_selection(12, 2).unwrap();
        let validations = snapshot
            .candidates
            .iter()
            .map(|candidate| validation(candidate, PbftLeaderCandidateValidationStatus::Validated))
            .collect();

        let result = service
            .finish_leader_selection(finish_request(&snapshot, validations))
            .unwrap();
        assert_eq!(result.status, PbftLeaderSelectionStatus::Selected);
        let proposed = service.proposed_blocks().read().unwrap();
        assert!(proposed.get(12, H256::from([0x61; 32])).unwrap().is_valid);
        assert!(proposed.get(12, H256::from([0x62; 32])).unwrap().is_valid);
    }

    #[test]
    fn finish_rejects_invalid_reports_without_marking_valid() {
        let (service, _storage) = test_service("invalid_reports");
        insert_proposal_vote(&service, [0x48; 32], NODE_SECRET, 3);
        insert_proposed_block(&service, [0x48; 32], [0x58; 32], vec![0x48]);
        let snapshot = service.prepare_leader_selection(12, 2).unwrap();
        let candidate = &snapshot.candidates[0];
        let invalid_cases = vec![
            Vec::new(),
            vec![validation(
                candidate,
                PbftLeaderCandidateValidationStatus::Invalid,
            )],
            vec![
                validation(candidate, PbftLeaderCandidateValidationStatus::Validated),
                validation(candidate, PbftLeaderCandidateValidationStatus::Validated),
            ],
            vec![PbftLeaderCandidateValidation {
                vote_hash: candidate.vote_hash,
                block_hash: H256::from([0x99; 32]),
                status: PbftLeaderCandidateValidationStatus::Validated,
            }],
            vec![PbftLeaderCandidateValidation {
                vote_hash: H256::from([0x98; 32]),
                block_hash: candidate.block_hash,
                status: PbftLeaderCandidateValidationStatus::Validated,
            }],
        ];
        for validations in invalid_cases {
            let result = service
                .finish_leader_selection(finish_request(&snapshot, validations))
                .unwrap();
            assert_eq!(
                result.status,
                PbftLeaderSelectionStatus::InvalidValidationReport
            );
            assert!(
                !service
                    .proposed_blocks()
                    .read()
                    .unwrap()
                    .get(12, H256::from([0x48; 32]))
                    .unwrap()
                    .is_valid
            );
        }
    }

    #[test]
    fn finish_detects_vote_proposed_and_chain_staleness_without_mutation() {
        for scenario in ["vote", "proposed", "chain"] {
            let (service, storage) = test_service(&format!("stale_{scenario}"));
            insert_proposal_vote(&service, [0x49; 32], NODE_SECRET, 3);
            insert_proposed_block(&service, [0x49; 32], [0x59; 32], vec![0x49]);
            let snapshot = service.prepare_leader_selection(12, 2).unwrap();
            match scenario {
                "vote" => {
                    insert_proposal_vote(&service, [0x4A; 32], NODE_SECRET_TWO, 2);
                }
                "proposed" => {
                    let mut proposed = service.proposed_blocks().write().unwrap();
                    proposed.cleanup_before(13);
                    assert!(proposed.push(
                        12,
                        H256::from([0x49; 32]),
                        H256::from([0x5A; 32]),
                        vec![0x49, 0x01],
                    ));
                }
                "chain" => storage
                    .period()
                    .write_pbft_period(H256::from([0x49; 32]), 12)
                    .unwrap(),
                _ => unreachable!(),
            }
            let result = service
                .finish_leader_selection(finish_request(
                    &snapshot,
                    vec![validation(
                        &snapshot.candidates[0],
                        PbftLeaderCandidateValidationStatus::Validated,
                    )],
                ))
                .unwrap();
            assert_eq!(result.status, PbftLeaderSelectionStatus::StaleSnapshot);
            assert!(
                !service
                    .proposed_blocks()
                    .read()
                    .unwrap()
                    .get(12, H256::from([0x49; 32]))
                    .unwrap()
                    .is_valid
            );
        }
    }

    #[test]
    fn prepare_serializes_finalized_membership_storage_with_manager_domain() {
        let (service, storage) = test_service("manager_membership_serialization");
        let service = Arc::new(service);
        let vote_hash = insert_proposal_vote(&service, [0x4B; 32], NODE_SECRET, 3);
        insert_proposed_block(&service, [0x4B; 32], [0x5B; 32], vec![0x4B]);
        let worker_service = service.clone();
        let manager = service.manager_state();
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(worker_service.prepare_leader_selection(12, 2))
                .unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            result_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "leader snapshot must wait behind the manager-owned finalization domain"
        );
        storage
            .period()
            .write_pbft_period(H256::from([0x4B; 32]), 12)
            .unwrap();
        drop(manager);

        let snapshot = result_rx.recv().unwrap().unwrap();
        worker.join().unwrap();
        let candidate = snapshot
            .candidates
            .iter()
            .find(|candidate| candidate.vote_hash == vote_hash)
            .unwrap();
        assert!(candidate.block_in_chain);
        assert!(!candidate.needs_external_validation);
    }
}
