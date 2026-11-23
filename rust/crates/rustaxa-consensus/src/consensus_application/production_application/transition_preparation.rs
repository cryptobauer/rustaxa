//! Closed acquisition for unpublished Rust transition computation.
use super::*;
use crate::consensus_application_runtime::{ScheduledSyncedCertificate, SyncedCertificateRejected};
use crate::final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan;
use crate::native_session::FinalChainPreparedNativeSession;
use crate::pbft_execution_inputs::{PbftCandidateInputs, PbftInputCapture};
use crate::pbft_execution_validation::*;
use crate::rewards_stats::{
    FinalizedRewardsPeriodFact, RewardCertVoteFact, RewardDagBlockFact, RewardTransactionFact,
};
use rustaxa_storage::ConcreteTransitionBuilder;
use rustaxa_types::codec::rlp::{dag::DagBlockRlp, pbft::SignedPbftBlockRlp};
use rustaxa_types::{DagBlock, PbftBlockMetadata};
use std::sync::atomic::{AtomicU64, Ordering};
use tiny_keccak::{Hasher, Keccak};

static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct ProductionPreparationUnavailable(pub String);
impl std::fmt::Display for ProductionPreparationUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PRODUCTION_PREPARATION_UNAVAILABLE: {}", self.0)
    }
}
impl std::error::Error for ProductionPreparationUnavailable {}
fn unavailable(reason: impl Into<String>) -> anyhow::Error {
    ProductionPreparationUnavailable(reason.into()).into()
}

/// A proof qualification refusal, distinct from unavailable native state.
#[derive(Debug)]
pub struct ProductionCertificateProofError {
    vote_hash: H256,
    code: &'static str,
}
impl std::fmt::Display for ProductionCertificateProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PRODUCTION_CERTIFICATE_PROOF_NOT_QUALIFIED: {}: {:#x}",
            self.code, self.vote_hash
        )
    }
}
impl std::error::Error for ProductionCertificateProofError {}

/// A scheduler selection that changed before it could publish any state.
#[derive(Debug)]
pub struct ScheduledCertificateStale;
impl std::fmt::Display for ScheduledCertificateStale {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str("PRODUCTION_SCHEDULED_CERTIFICATE_STALE")
    }
}
impl std::error::Error for ScheduledCertificateStale {}

#[derive(Debug)]
struct ProposalPreparationStale;

impl std::fmt::Display for ProposalPreparationStale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PRODUCTION_PROPOSAL_PREPARATION_STALE")
    }
}

impl std::error::Error for ProposalPreparationStale {}

/// Ordered execution evidence from one application owner and read generation.
pub struct CheckedProposalExecution {
    issuer: Arc<()>,
    execution: ValidatedPbftExecution,
}

pub enum ProposalExecutionCheck {
    Checked(CheckedProposalExecution),
    Invalid,
    Unavailable(PbftExecutionUnavailable),
}

impl ApplicationScopeSource {
    pub(crate) fn check_proposal_vote(
        &self,
        proof: &CheckedProposalExecution,
        scope: &ApplicationOperationScope<'_>,
        block_rlp: &[u8],
        period: u64,
        admission: bool,
    ) -> Result<bool> {
        let request = proof.execution.request();
        let facts = request.context().facts();
        ensure!(
            Arc::ptr_eq(&self.identity, &proof.issuer)
                && self.schedule == facts.schedule
                && facts.period.as_u64() == period
                && request.snapshot().block_rlp() == block_rlp,
            "PRODUCTION_PROPOSAL_VOTE_BINDING_MISMATCH"
        );
        Ok(match scope {
            ApplicationOperationScope::Read(action) if !admission => {
                std::ptr::eq(action.source, self)
                    && action.generation == facts.owner_generation
                    && action.guard.is_current()
            }
            ApplicationOperationScope::Mutation(source, action) if admission => {
                std::ptr::eq(*source, self)
                    && facts.owner_generation.checked_add(1) == Some(action.generation())
            }
            _ => false,
        })
    }
}

/// Checked input evidence. It grants no voting or publication authority.
pub struct VerifiedCertificateEvidence {
    pub(super) request: PbftExecutionRequest,
    pub(super) encoded: Vec<u8>,
    pub(super) issuer: Arc<()>,
    pub(super) scheduler_selected: bool,
    pub(super) synced: Option<ScheduledSyncedCertificate>,
}
impl VerifiedCertificateEvidence {
    pub fn request_digest(&self) -> [u8; 32] {
        self.request.digest()
    }
    pub fn certificate_digest(&self) -> [u8; 32] {
        digest(&self.encoded)
    }
    pub fn owner_epoch(&self) -> u64 {
        self.request.context().facts().state_api_epoch
    }
    pub fn prior_identity(&self) -> rustaxa_types::concrete_state::ConcreteStateIdentity {
        self.request.context().facts().prior
    }
    pub fn native_source_digest(&self) -> [u8; 32] {
        self.request.context().facts().native_prior_digest
    }
}

#[derive(Clone, Copy)]
pub(super) enum CertificateInput<'a> {
    Admitted(u64),
    Verified(u64),
    Scheduled(&'a crate::consensus_application_runtime::ScheduledCertificate<'a>),
    Synced(&'a ScheduledSyncedCertificate),
}

impl CertificateInput<'_> {
    fn round(self) -> u64 {
        match self {
            Self::Admitted(round) | Self::Verified(round) => round,
            Self::Scheduled(selection) => selection.round,
            Self::Synced(selection) => selection.round,
        }
    }
}

pub(super) struct TransitionPolicy {
    pub bridge: [u8; 20],
    pub ficus: u64,
    pub interval: u64,
    pub delay: u64,
    blocks_per_year: u32,
    committee_size: u64,
    number_of_proposers: u64,
    pub(super) epoch: u64,
}
impl TransitionPolicy {
    pub(super) fn new(config: &ConsensusApplicationBootstrap) -> Result<Self> {
        Ok(Self {
            bridge: config.final_chain.bridge_contract_address,
            ficus: config.consensus.pbft.ficus_activation_period,
            interval: config.consensus.pbft.pillar_blocks_interval,
            delay: config.final_chain.genesis_dpos.delegation_delay,
            blocks_per_year: config
                .consensus
                .pbft
                .process_synced_policy
                .dpos_blocks_per_year,
            committee_size: config.consensus.pbft.committee_size,
            number_of_proposers: config.consensus.pbft.number_of_proposers,
            epoch: NEXT_EPOCH
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .map_err(|_| unavailable("epoch exhausted"))?,
        })
    }
    fn digest(&self) -> [u8; 32] {
        let mut out = RlpStream::new_list(7);
        out.append(&b"rustaxa-pbft-system-policy".as_slice())
            .append(&1_u8)
            .append(&self.bridge.as_slice())
            .append(&self.ficus)
            .append(&self.interval)
            .append(&self.delay)
            .append(&10_000_000_u64);
        digest(&out.out())
    }
}

/// One borrowed computation source. It exposes no service, lifecycle or writer.
pub struct ApplicationTransitionContext<'a> {
    application: &'a BoundConsensusApplication,
    request: PbftExecutionRequest,
    observation: ConcreteLifecycleObservation,
    reward_votes: Vec<RewardCertVoteFact>,
    blocks_per_year: u32,
    verified_certificate_context: Option<Vec<u8>>,
    scheduler_selected: bool,
    synced: Option<ScheduledSyncedCertificate>,
    bridge_simulation: Option<crate::native_session::FinalChainNativeSimulation<'a>>,
    readers: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
}
impl<'a> ApplicationTransitionContext<'a> {
    pub fn into_verified_certificate(self) -> Option<VerifiedCertificateEvidence> {
        let Self {
            application,
            request,
            verified_certificate_context,
            scheduler_selected,
            synced,
            ..
        } = self;
        verified_certificate_context.map(|encoded| VerifiedCertificateEvidence {
            request,
            encoded,
            issuer: Arc::clone(&application.source.identity),
            scheduler_selected,
            synced,
        })
    }

    pub fn concrete_observation(&self) -> &ConcreteLifecycleObservation {
        &self.observation
    }

    pub fn request(&self) -> &PbftExecutionRequest {
        &self.request
    }
    pub fn take_bridge_simulation(
        &mut self,
    ) -> Option<crate::native_session::FinalChainNativeSimulation<'a>> {
        self.bridge_simulation.take()
    }

    pub fn bridge_simulation_inputs(
        &self,
    ) -> (
        [u8; 20],
        &rustaxa_storage::ConcreteCheckpointReaders,
        crate::pbft_execution_validation::PbftInstructionPhase,
        bool,
    ) {
        let prior = self.observation.committed.period;
        (
            self.application.transition_policy.bridge,
            &self.readers,
            self.application
                .core
                .final_chain
                .pbft_execution_instruction_phase(prior),
            self.application
                .core
                .final_chain
                .pbft_execution_cornus(prior),
        )
    }

    pub fn bridge_simulation_header(&self) -> Result<Vec<u8>> {
        self.application
            .core
            .final_chain
            .block_header(self.observation.committed.period)?
            .context("PRODUCTION_BRIDGE_CHECKPOINT_HEADER_MISSING")
    }

    pub fn block_hash(&self, number: FinalChainBlockNumber) -> Result<Option<Vec<u8>>> {
        self.application.core.final_chain.block_hash(number)
    }
    pub fn is_native(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.application
            .core
            .final_chain
            .native_contract_kind(period, address)
            .is_some()
    }
    pub fn code_updates(&self, empty: bool) -> Vec<rustaxa_types::native_code::NativeCodeUpdate> {
        self.application
            .core
            .final_chain
            .native_code_updates(self.request.context().facts().period, empty)
    }
    pub fn credit_fees(&self) -> bool {
        self.application
            .core
            .final_chain
            .pre_magnolia_fee_reward_period(self.request.context().facts().period)
    }
    pub fn plan_rewards(
        &self,
        transactions: Vec<RewardTransactionFact>,
    ) -> Result<FinalChainPreparedExternalEvmRewardsStatsPlan> {
        let expected = self.request.snapshot().transactions();
        ensure!(
            transactions.len() == expected.len(),
            "PRODUCTION_REWARDS_TRANSACTION_COUNT"
        );
        for (fact, tx) in transactions.iter().zip(expected) {
            ensure!(
                fact.hash == tx.hash && fact.gas_price == Rlp::new(&tx.rlp).val_at::<U256>(1)?,
                "PRODUCTION_REWARDS_TRANSACTION_MISMATCH"
            );
        }
        let dag_blocks = self
            .request
            .snapshot()
            .dag_blocks()
            .iter()
            .map(|entry| {
                let block = DagBlock::try_from(DagBlockRlp::new(&entry.block_rlp))?;
                let author = block.recover_sender().context("PRODUCTION_DAG_SIGNATURE")?;
                let difficulty = crate::dag::dag_manager_block_from_rlp(&entry.block_rlp)?
                    .difficulty
                    .try_into()?;
                Ok(RewardDagBlockFact {
                    author,
                    difficulty,
                    transaction_hashes: block.transactions,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let chain = &self.application.core.final_chain;
        let eligible_votes = if let Some(vote) = self.reward_votes.first() {
            chain.dpos_eligible_total_vote_count(vote.period.saturating_sub(1).into())?
        } else {
            u64::from(
                chain
                    .native_execution_configuration()
                    .rewards()
                    .committee_size,
            )
        };
        self.application
            .core
            .final_chain
            .plan_external_evm_rewards_stats(
                self.request.digest(),
                FinalizedRewardsPeriodFact {
                    period: self.request.context().facts().period.as_u64(),
                    block_author: self.request.snapshot().metadata().author,
                    blocks_per_year: self.blocks_per_year,
                    dpos_eligible_total_vote_count: eligible_votes,
                    transactions,
                    dag_blocks,
                    cert_votes: self.reward_votes.clone(),
                },
            )
    }
    pub fn execution_marker(&self) -> Result<Vec<u8>> {
        use rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker;
        use rustaxa_types::concrete_lifecycle::{
            FinalChainConcreteExecutionMarker, FinalChainConcreteState,
        };
        let prior = self.observation.committed;
        Ok(encode_concrete_execution_marker(
            &FinalChainConcreteExecutionMarker {
                identity: self.observation.identity,
                generation: self
                    .observation
                    .generation
                    .checked_add(1)
                    .context("PRODUCTION_GENERATION_OVERFLOW")?,
                plan_hash: self.request.digest(),
                period: self.request.context().facts().period.as_u64(),
                prior_state: FinalChainConcreteState {
                    period: prior.period.as_u64(),
                    root: prior.state_root,
                },
                transactions_hash: self.request.snapshot().digest(),
                rewards_hash: digest(&self.request.snapshot().reward_votes().concat()),
            },
        ))
    }
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    let mut h = Keccak::v256();
    let mut out = [0; 32];
    h.update(bytes);
    h.finalize(&mut out);
    out
}
pub(super) fn limits() -> PbftExecutionInputLimits {
    PbftExecutionInputLimits {
        maximum_bytes: 16 * 1024 * 1024,
        maximum_dag_blocks: 4096,
        maximum_transactions: 65536,
    }
}
pub(super) fn capture(
    application: &BoundConsensusApplication,
    block: &[u8],
    synced: Option<&ScheduledSyncedCertificate>,
) -> Result<PbftCandidateInputs> {
    let dag = &application.core.dag_transaction;
    match if let Some(selection) = synced {
        dag.snapshot_synced_pbft_execution_inputs(
            block,
            &selection.queue.period_data_rlp,
            limits(),
        )?
    } else {
        dag.snapshot_pbft_execution_inputs(block, limits())?
    } {
        PbftInputCapture::Ready(inputs) => Ok(inputs),
        PbftInputCapture::Unavailable(
            reason @ crate::pbft_execution_inputs::PbftInputUnavailable::MissingTransaction(_),
        ) if synced.is_some() => Err(SyncedCertificateRejected(format!("{reason:?}")).into()),
        PbftInputCapture::Unavailable(reason) => Err(unavailable(format!("{reason:?}"))),
    }
}
pub(super) fn reward_votes(
    application: &BoundConsensusApplication,
    block: &[u8],
) -> Result<(Vec<Vec<u8>>, Vec<RewardCertVoteFact>)> {
    let rlp = Rlp::new(block);
    let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(block))?;
    let hashes: Vec<H256> = rlp.at(6)?.as_list()?;
    let selected = application
        .core
        .pbft
        .select_reward_vote_payloads(metadata.period, hashes)?;
    if !selected.accepted {
        return Err(unavailable(format!("{:?}", selected.status)));
    }
    let mut bytes = Vec::new();
    let mut facts = Vec::new();
    for record in selected.selected_records {
        let vote = crate::pbft_vote_validation::inspect_canonical_pbft_vote(&record.vote_rlp)?;
        ensure!(
            vote.signature_valid
                && vote.has_embedded_weight
                && vote.embedded_weight > 0
                && vote.vote_hash == record.hash,
            "PRODUCTION_REWARD_VOTE_SOURCE"
        );
        facts.push(RewardCertVoteFact {
            voter: vote.recovered_voter,
            weight: vote.embedded_weight,
            period: vote.period,
        });
        bytes.push(record.vote_rlp);
    }
    Ok((bytes, facts))
}

#[derive(Eq, PartialEq)]
pub(super) struct RetainedCertificateContext {
    pub(super) encoded: Vec<u8>,
    pub(super) blocks_per_year: u32,
    qualification: Option<Vec<u8>>,
}

pub(super) fn retained_certificate_context(
    application: &BoundConsensusApplication,
    block: &[u8],
    selection: CertificateInput<'_>,
) -> Result<RetainedCertificateContext> {
    use crate::pbft_vote_validation::{
        PbftCanonicalVoteInspectionStatus, inspect_canonical_pbft_vote,
    };
    use crate::verified_votes::{PbftVoteType, TwoTPlusOneVotedBlockType};
    let round = selection.round();
    let link = rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(block))?;
    if application.core.pbft.verified_votes_size()? > 65_536 {
        return Err(unavailable(
            "retained certificate source exceeds local capacity",
        ));
    }
    let retained;
    let (step, votes) = if let CertificateInput::Synced(selection) = selection {
        ensure!(
            selection.queue.entry_period == link.period
                && selection.queue.block_hash == link.block_hash,
            "PRODUCTION_SYNC_CERTIFICATE_TARGET_MISMATCH"
        );
        (selection.step, selection.votes.as_slice())
    } else {
        retained = application
            .core
            .pbft
            .verified_votes_get_two_t_plus_one_voted_block_payloads(
                link.period,
                round,
                TwoTPlusOneVotedBlockType::CertVotedBlock,
            )?
            .ok_or_else(|| unavailable("current certificate unavailable"))?;
        if retained.block_hash != link.block_hash || retained.votes.is_empty() {
            return Err(unavailable(
                "current certificate does not match the candidate",
            ));
        }
        if let CertificateInput::Scheduled(selection) = selection {
            ensure!(
                selection.period == link.period && selection.votes == retained.votes.as_slice(),
                "PRODUCTION_SCHEDULED_CERTIFICATE_CHANGED"
            );
        }
        (retained.step, retained.votes.as_slice())
    };
    let mut voters = std::collections::BTreeSet::new();
    let mut size = 0_usize;
    for record in votes {
        size = size
            .checked_add(record.vote_rlp.len())
            .context("PRODUCTION_CERTIFICATE_SIZE_OVERFLOW")?;
        if size > 16 * 1024 * 1024 {
            return Err(unavailable(
                "retained certificate bytes exceed local capacity",
            ));
        }
        let vote = inspect_canonical_pbft_vote(&record.vote_rlp)?;
        ensure!(
            vote.status == PbftCanonicalVoteInspectionStatus::Valid
                && vote.signature_valid
                && vote.vote_hash == record.hash
                && vote.block_hash == link.block_hash
                && vote.period == link.period
                && vote.round == round
                && vote.step == step
                && vote.vote_type == PbftVoteType::Cert
                && vote.has_embedded_weight
                && vote.embedded_weight > 0
                && voters.insert(vote.recovered_voter),
            "PRODUCTION_CERTIFICATE_SOURCE_MISMATCH"
        );
    }
    let qualification = match selection {
        CertificateInput::Admitted(_) => None,
        CertificateInput::Verified(_)
        | CertificateInput::Scheduled(_)
        | CertificateInput::Synced(_) => {
            Some(qualify_certificate(application, link.period, votes)?)
        }
    };
    let dynamic = crate::pbft_application_finalization::prepare_certified_dynamic_lambda(
        &application.core.pbft,
        link.period,
        round,
    )?;
    let plan = &dynamic.plan;
    let verified = qualification.is_some();
    let mut encoded = RlpStream::new_list(if verified { 10 } else { 9 });
    let domain = if verified {
        b"rustaxa-verified-certificate-context".as_slice()
    } else {
        b"rustaxa-retained-certificate-context".as_slice()
    };
    encoded
        .append(&domain)
        .append(&if verified { 2_u8 } else { 1_u8 })
        .append(&link.period)
        .append(&round)
        .append(&link.block_hash)
        .append(&step)
        .begin_list(votes.len());
    for record in votes {
        encoded.append(&record.vote_rlp);
    }
    encoded
        .begin_list(8)
        .append(&plan.apply_dynamic_lambda_update)
        .append(&plan.period_lambda)
        .append(&plan.blocks_per_year)
        .append(&plan.rounds_count_dynamic_lambda)
        .append(&plan.dynamic_lambda)
        .append(&plan.decreased_dynamic_lambda)
        .append(&plan.increased_dynamic_lambda)
        .append(&plan.status.as_u8())
        .begin_list(2)
        .append(&dynamic.last_saved_period_lambda.found)
        .append(&dynamic.last_saved_period_lambda.value);
    if let Some(facts) = &qualification {
        encoded.append(facts);
    }
    Ok(RetainedCertificateContext {
        encoded: encoded.out().to_vec(),
        blocks_per_year: plan.blocks_per_year,
        qualification,
    })
}

fn qualify_certificate(
    application: &BoundConsensusApplication,
    period: u64,
    records: &[crate::pbft_vote_payload::PbftVotePayloadRecord],
) -> Result<Vec<u8>> {
    qualify_certificate_from_chain(
        &application.core.final_chain,
        &application.transition_policy,
        application.core.pbft.pbft_chain_head().size,
        period,
        records,
    )
}

pub(super) fn qualify_certificate_from_chain(
    final_chain: &FinalChain,
    policy: &TransitionPolicy,
    chain_size: u64,
    period: u64,
    records: &[crate::pbft_vote_payload::PbftVotePayloadRecord],
) -> Result<Vec<u8>> {
    use crate::pbft_thresholds::{
        PbftTwoTPlusOneThresholdFact, PbftTwoTPlusOneThresholdRuntime,
        PbftTwoTPlusOneThresholdStatus,
    };
    use crate::pbft_vote_validation::{
        PbftVoteValidationExternalFacts, inspect_canonical_pbft_vote, validate_canonical_pbft_vote,
    };
    use crate::verified_votes::PbftVoteType;
    let native_period = period
        .checked_sub(1)
        .ok_or_else(|| unavailable("certificate period zero"))?;
    let total = final_chain
        .pbft_dpos_eligible_total_vote_count(native_period)
        .context("PRODUCTION_CERTIFICATE_NATIVE_TOTAL")?
        .ok_or_else(|| unavailable("certificate native total is not ready"))?;
    if total == 0 {
        return Err(unavailable("certificate native total is zero"));
    }
    let threshold =
        PbftTwoTPlusOneThresholdRuntime::new().plan_threshold(PbftTwoTPlusOneThresholdFact {
            pbft_period: native_period,
            vote_type: PbftVoteType::Cert,
            current_pbft_chain_size: chain_size,
            committee_size: policy.committee_size,
            number_of_proposers: policy.number_of_proposers,
            has_total_dpos_votes_count: true,
            total_dpos_votes_count: total,
            future_dpos_state: false,
            unknown_error: false,
        });
    if threshold.status != PbftTwoTPlusOneThresholdStatus::Available
        || !threshold.has_threshold
        || threshold.threshold == 0
    {
        return Err(unavailable(format!(
            "certificate threshold unavailable: {}",
            threshold.error_code
        )));
    }
    let mut voters = RlpStream::new_list(records.len());
    let mut weight = 0_u64;
    for record in records {
        let inspection = inspect_canonical_pbft_vote(&record.vote_rlp)?;
        let voter = inspection.recovered_voter.0;
        let stake = final_chain
            .pbft_dpos_eligible_vote_count(native_period, voter)
            .context("PRODUCTION_CERTIFICATE_NATIVE_VOTER")?
            .ok_or_else(|| unavailable("certificate voter state is not ready"))?;
        if stake == 0 {
            return Err(ProductionCertificateProofError {
                vote_hash: record.hash,
                code: "PBFT_CERTIFICATE_VOTER_INELIGIBLE",
            }
            .into());
        }
        let key = final_chain
            .pbft_vrf_key_with_fallback(native_period, voter)
            .context("PRODUCTION_CERTIFICATE_NATIVE_KEY")?
            .ok_or_else(|| unavailable("eligible certificate voter key is missing"))?;
        let checked = validate_canonical_pbft_vote(
            &record.vote_rlp,
            PbftVoteValidationExternalFacts {
                voter_dpos_ready: true,
                voter_dpos_vote_count: stake,
                total_dpos_ready: true,
                total_dpos_vote_count: total,
                future_dpos_state: false,
                unknown_error: false,
                vrf_key_ready: true,
                has_vrf_key: true,
                vrf_public_key: key,
                strict_vrf: true,
                committee_size: policy.committee_size,
                number_of_proposers: policy.number_of_proposers,
                has_preverified_weight: false,
                preverified_weight: 0,
            },
        )
        .context("PRODUCTION_CERTIFICATE_CRYPTO_CHECK")?;
        if !checked.accepted || !checked.weight_calculated || checked.calculated_weight == 0 {
            return Err(ProductionCertificateProofError {
                vote_hash: record.hash,
                code: checked.error_code,
            }
            .into());
        }
        if checked.calculated_weight != inspection.embedded_weight {
            return Err(unavailable(
                "certificate sidecar differs from native weight",
            ));
        }
        weight = weight
            .checked_add(checked.calculated_weight)
            .ok_or_else(|| unavailable("certificate weight sum overflow"))?;
        voters
            .begin_list(6)
            .append(&inspection.recovered_voter)
            .append(&stake)
            .append(&key.as_slice())
            .append(&checked.calculated_weight)
            .append(&checked.status.as_u8())
            .append(&record.hash);
    }
    if weight < threshold.threshold {
        return Err(unavailable("certificate quorum below native threshold"));
    }
    let mut encoded = RlpStream::new_list(7);
    encoded
        .append(&b"rustaxa-strict-certificate-facts-v1".as_slice())
        .append(&policy.committee_size)
        .append(&policy.number_of_proposers)
        .append(&total)
        .append(&threshold.threshold)
        .append(&weight)
        .append_raw(&voters.out(), 1);
    Ok(encoded.out().to_vec())
}

impl BoundConsensusApplication {
    /// Executes a local Rust leaf while all actual predecessor sources stay bound.
    /// The callback can prepare state but cannot publish through this capability.
    pub fn with_transition_preparation<R>(
        &self,
        block_rlp: &[u8],
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<R> {
        self.with_transition_inputs(block_rlp, None, None, operation)
    }

    /// Checks a signed proposal without crossing a change in owner generation.
    pub fn with_proposal_transition_preparation(
        &self,
        authority: &ProductionOwnerPublicationAuthority,
        block_rlp: &[u8],
        owner_generation: u64,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<PbftExecutionValidation>,
    ) -> Result<Option<ProposalExecutionCheck>> {
        match self.with_transition_inputs(
            block_rlp,
            None,
            Some(owner_generation),
            |context, concrete, native| {
                authority.validate(self)?;
                let request = Arc::new(context.request().clone());
                let report = operation(context, concrete, native)?;
                let proof = ValidatedPbftExecution::from_report(request, report.clone())?;
                Ok(match report {
                    PbftExecutionValidation::Invalid { .. } => ProposalExecutionCheck::Invalid,
                    PbftExecutionValidation::Unavailable { reason, .. } => {
                        ProposalExecutionCheck::Unavailable(reason)
                    }
                    PbftExecutionValidation::Valid { .. } => {
                        ProposalExecutionCheck::Checked(CheckedProposalExecution {
                            issuer: Arc::clone(&self.source.identity),
                            execution: proof
                                .context("PRODUCTION_PROPOSAL_EXECUTION_PROOF_MISSING")?,
                        })
                    }
                })
            },
        ) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is::<ProposalPreparationStale>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Computes with the actual retained current certificate; grants no effects.
    pub fn with_certified_transition_preparation<R>(
        &self,
        block_rlp: &[u8],
        certified_round: u64,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<R> {
        self.with_transition_inputs(
            block_rlp,
            Some(CertificateInput::Admitted(certified_round)),
            None,
            operation,
        )
    }

    /// Computes fully checked certificate evidence without granting effects.
    pub fn with_verified_certified_transition_preparation<R>(
        &self,
        block_rlp: &[u8],
        certified_round: u64,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<R> {
        self.with_transition_inputs(
            block_rlp,
            Some(CertificateInput::Verified(certified_round)),
            None,
            operation,
        )
    }

    pub fn with_scheduled_certificate_preparation<R>(
        &self,
        selection: &crate::consensus_application_runtime::ScheduledCertificate<'_>,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<Option<R>> {
        match self.with_transition_inputs(
            selection.block_rlp,
            Some(CertificateInput::Scheduled(selection)),
            None,
            operation,
        ) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is::<ScheduledCertificateStale>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Prepares paired state from the exact admitted queue head without consuming it.
    pub fn with_synced_certificate_preparation<R>(
        &self,
        selection: &ScheduledSyncedCertificate,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<Option<R>> {
        let period_data = Rlp::new(&selection.queue.period_data_rlp);
        match self.with_transition_inputs(
            period_data.at(0)?.as_raw(),
            Some(CertificateInput::Synced(selection)),
            None,
            operation,
        ) {
            Ok(value) => Ok(Some(value)),
            Err(error) if error.is::<ScheduledCertificateStale>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn with_transition_inputs<R>(
        &self,
        block_rlp: &[u8],
        certified_round: Option<CertificateInput<'_>>,
        proposal_generation: Option<u64>,
        operation: impl for<'state> FnOnce(
            ApplicationTransitionContext<'state>,
            ConcreteTransitionBuilder<'state>,
            FinalChainPreparedNativeSession,
        ) -> Result<R>,
    ) -> Result<R> {
        let action = self.source.read()?;
        action.validate(self)?;
        if proposal_generation.is_some_and(|expected| expected != action.generation) {
            return Err(ProposalPreparationStale.into());
        }
        if let Some(CertificateInput::Scheduled(selection)) = certified_round {
            if selection.owner_generation != action.generation {
                return Err(ScheduledCertificateStale.into());
            }
            let candidate =
                rustaxa_types::pbft::PbftBlockLink::try_from(SignedPbftBlockRlp::new(block_rlp))?;
            self.core.pbft.qualify_retained_certified_proposal(
                &self.core.final_chain,
                &self.core.dag_transaction,
                selection.period,
                candidate.block_hash,
                block_rlp,
            )?;
        }
        let synced = match certified_round {
            Some(CertificateInput::Synced(selection)) => {
                if selection.owner_generation != action.generation
                    || !self.core.pbft.sync_front_matches(&selection.queue)
                {
                    return Err(ScheduledCertificateStale.into());
                }
                Some(selection)
            }
            _ => None,
        };
        let observation = self.source.concrete_state.observation(&action.guard)?;
        if !self.is_current_concrete_observation(&observation)?
            || read_committed_identity(&self.core.storage)? != observation.committed
        {
            return Err(unavailable("committed state changed"));
        }
        let inputs = capture(self, block_rlp, synced)?;
        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(block_rlp))?;
        let period = FinalChainBlockNumber::new(metadata.period);
        let native = match self.core.final_chain.capture_native_predecessor(
            period,
            NativePredecessorLimits {
                max_retained_source_bytes: NATIVE_SOURCE_LIMIT,
            },
        )? {
            NativePredecessorCapture::Ready(native) => native,
            NativePredecessorCapture::Unavailable(reason) => {
                return Err(unavailable(format!("{reason:?}")));
            }
        };
        if native.committed_identity() != observation.committed
            || native.pairing_identity() != observation.identity
            || native.pairing_storage_genesis()
                != self
                    .core
                    .execution_configuration()
                    .map_err(Clone::clone)?
                    .full_genesis()
                    .0
        {
            return Err(unavailable("native/concrete pairing mismatch"));
        }
        let native_digest = native.source_digest();
        let delayed = native.delayed_identity();
        let (reward_bytes, rewards) = reward_votes(self, block_rlp)?;
        let certificate = certified_round
            .map(|round| retained_certificate_context(self, block_rlp, round))
            .transpose()?;
        let blocks_per_year = certificate
            .as_ref()
            .map_or(self.transition_policy.blocks_per_year, |c| {
                c.blocks_per_year
            });
        let system_policy_digest = match &certificate {
            None => self.transition_policy.digest(),
            Some(cert) => {
                let mut bound = RlpStream::new_list(3);
                bound
                    .append(&b"rustaxa-certified-system-policy".as_slice())
                    .append(&self.transition_policy.digest().as_slice())
                    .append(&cert.encoded);
                digest(&bound.out())
            }
        };

        let context = PbftExecutionContext::new(PbftExecutionContextFacts {
            schedule: self.source.schedule.clone(),
            period,
            previous_pbft_hash: rustaxa_types::pbft::PbftBlockLink::try_from(
                SignedPbftBlockRlp::new(block_rlp),
            )?
            .prev_block_hash
            .0,
            prior: observation.committed,
            native_prior_digest: native_digest,
            delayed,
            concrete_provenance_digest: digest(&observation.provenance_rlp),
            concrete_generation: observation.generation,
            owner_generation: action.generation,
            state_api_epoch: self.transition_policy.epoch,
            instruction_phase: self
                .core
                .final_chain
                .pbft_execution_instruction_phase(period),
            cornus: self.core.final_chain.pbft_execution_cornus(period),
            author: metadata.author.0,
            timestamp: metadata.timestamp,
            gas_limit: self
                .core
                .final_chain
                .pbft_execution_block_gas_limit()
                .as_u64(),
            system_policy_digest,
        })?;
        let snapshot = inputs
            .clone()
            .into_snapshot(&context, reward_bytes.clone(), limits())
            .map_err(|error| {
                if synced.is_some() && !matches!(error, PbftExecutionInputError::LocalLimit) {
                    anyhow::Error::new(SyncedCertificateRejected(error.to_string()))
                } else {
                    error.into()
                }
            })?;
        let request = PbftExecutionRequest::new(Arc::new(snapshot), context)?;
        let policy = &self.transition_policy;
        ensure!(policy.interval != 0, "PRODUCTION_PILLAR_INTERVAL_ZERO");
        let selected = period
            .as_u64()
            .checked_add(policy.delay)
            .context("PRODUCTION_PILLAR_PERIOD_OVERFLOW")?;
        let pillar =
            selected >= policy.ficus.max(policy.interval) && selected % policy.interval == 0;
        let bridge_has_code = if pillar {
            self.with_finalized_state_in_action(&action, Some(observation.committed.period.as_u64()), |readers, identity| {
                Ok(matches!(readers.account_at(identity, policy.bridge)?,
                    rustaxa_types::concrete_state::ConcreteRead::Present(row) if row.account.code_size != 0))
            })?.context("PRODUCTION_BRIDGE_CHECKPOINT_UNAVAILABLE")?
        } else {
            false
        };
        let bridge_simulation = if bridge_has_code {
            Some(
                self.core
                    .final_chain
                    .begin_native_simulation_with_slashing_accounts(
                        observation.committed.period,
                        |at| {
                            self.with_finalized_state_in_action(
                                &action,
                                Some(at.as_u64()),
                                |readers, identity| {
                                    Ok(matches!(
                                        readers.account_at(
                                            identity,
                                            crate::final_chain::SLASHING_CONTRACT_ADDRESS
                                        )?,
                                        rustaxa_types::concrete_state::ConcreteRead::Present(_)
                                    ))
                                },
                            )
                            .and_then(|found| {
                                found.context("PRODUCTION_BRIDGE_SLASHING_CHECKPOINT_UNAVAILABLE")
                            })
                            .map_err(|error| {
                                crate::native_session::FinalChainNativeSessionError::Domain(
                                    error.to_string(),
                                )
                            })
                        },
                    )?,
            )
        } else {
            None
        };
        let result = self.source.concrete_state.with_preparation(
            &action.guard,
            &observation,
            period,
            delayed,
            |builder, readers| {
                let native = native.into_owned_transition(
                    &self.core.final_chain,
                    request.digest(),
                    self.transition_policy.epoch,
                    observation
                        .generation
                        .checked_add(1)
                        .context("PRODUCTION_GENERATION_OVERFLOW")?,
                    Arc::clone(&readers),
                )?;
                operation(
                    ApplicationTransitionContext {
                        application: self,
                        request,
                        observation: observation.clone(),
                        reward_votes: rewards,
                        blocks_per_year,
                        scheduler_selected: matches!(
                            certified_round,
                            Some(CertificateInput::Scheduled(_) | CertificateInput::Synced(_))
                        ),
                        synced: synced.cloned(),
                        bridge_simulation,
                        readers,
                        verified_certificate_context: certificate
                            .as_ref()
                            .filter(|certificate| certificate.qualification.is_some())
                            .map(|certificate| certificate.encoded.clone()),
                    },
                    builder,
                    native,
                )
            },
        );
        action.validate(self)?;
        if capture(self, block_rlp, synced)? != inputs
            || reward_votes(self, block_rlp)?.0 != reward_bytes
        {
            return Err(unavailable("ordered input source changed"));
        }
        let after = match self.core.final_chain.capture_native_predecessor(
            period,
            NativePredecessorLimits {
                max_retained_source_bytes: NATIVE_SOURCE_LIMIT,
            },
        )? {
            NativePredecessorCapture::Ready(native) => native,
            NativePredecessorCapture::Unavailable(reason) => {
                return Err(unavailable(format!("{reason:?}")));
            }
        };
        if after.source_digest() != native_digest {
            return Err(unavailable("native source changed"));
        }
        let after_certificate = certified_round
            .map(|round| retained_certificate_context(self, block_rlp, round))
            .transpose()?;
        if after_certificate != certificate {
            return Err(unavailable("certified reward source changed"));
        }
        result
    }
}
