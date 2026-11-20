//! Immutable inputs for ordered execution before PBFT votes.
//!
//! Input construction checks structure and identity. It grants no execution,
//! vote, storage or publication authority.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use anyhow::{Result, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};
use rustaxa_types::{
    FinalChainBlockNumber, LegacyTransactionEnvelope,
    codec::rlp::{dag::DagBlockRlp, pbft::SignedPbftBlockRlp},
    concrete_state::ConcreteStateIdentity,
    dag::DagBlock,
    evm_correction::CorrectionStage,
    pbft::{PbftBlockLink, PbftBlockMetadata},
};
use tiny_keccak::{Hasher, Keccak};

use crate::dag::DagSyncBlockRlp;

/// One network's immutable correction schedule and complete configuration hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbftExecutionSchedule {
    chain_id: u64,
    genesis: [u8; 32],
    configuration: [u8; 32],
    activation: Option<FinalChainBlockNumber>,
}

impl PbftExecutionSchedule {
    /// Binds authenticated network/configuration facts and period B.
    /// `None` selects historical rules; it cannot replace durable enrollment.
    pub fn new(
        chain_id: u64,
        genesis: [u8; 32],
        configuration: [u8; 32],
        activation: Option<FinalChainBlockNumber>,
    ) -> Self {
        Self {
            chain_id,
            genesis,
            configuration,
            activation,
        }
    }
    pub fn correction_stage(&self, period: FinalChainBlockNumber) -> CorrectionStage {
        if self.activation.is_some_and(|boundary| period >= boundary) {
            CorrectionStage::SlashingHardError
        } else {
            CorrectionStage::Historical
        }
    }
    pub fn requires_validation(&self, period: FinalChainBlockNumber) -> bool {
        self.correction_stage(period) == CorrectionStage::SlashingHardError
    }
    pub fn genesis(&self) -> [u8; 32] {
        self.genesis
    }
    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }
    pub fn activation_period(&self) -> Option<FinalChainBlockNumber> {
        self.activation
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut out = RlpStream::new_list(6);
        out.append(&b"rustaxa-pbft-execution-schedule".as_slice())
            .append(&1_u8)
            .append(&self.chain_id)
            .append(&self.genesis.as_slice())
            .append(&self.configuration.as_slice());
        out.begin_list(usize::from(self.activation.is_some()));
        if let Some(period) = self.activation {
            out.append(&period.as_u64());
        }
        hash(&out.out())
    }
    /// The caller reads `enrolled` from the qualified paired application store.
    pub fn verify_enrollment(&self, enrolled: Option<[u8; 32]>) -> Result<()> {
        ensure!(
            self.activation.is_none() || enrolled.is_some(),
            "PBFT_EXECUTION_SCHEDULE_NOT_ENROLLED"
        );
        if let Some(enrolled) = enrolled {
            ensure!(
                self.activation.is_some() && enrolled == self.digest(),
                "PBFT_EXECUTION_SCHEDULE_MISMATCH"
            );
        }
        Ok(())
    }
}

/// Instruction rules are independent of the correction stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PbftInstructionPhase {
    Californicum,
    Ficus,
    Cacti,
}

/// Facts acquired from one qualified application/state owner.
/// These values are not network claims or a state proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbftExecutionContextFacts {
    pub schedule: PbftExecutionSchedule,
    pub period: FinalChainBlockNumber,
    pub previous_pbft_hash: [u8; 32],
    pub prior: ConcreteStateIdentity,
    pub native_prior_digest: [u8; 32],
    pub delayed: ConcreteStateIdentity,
    pub concrete_provenance_digest: [u8; 32],
    pub concrete_generation: u64,
    pub owner_generation: u64,
    pub state_api_epoch: u64,
    pub instruction_phase: PbftInstructionPhase,
    pub cornus: bool,
    pub author: [u8; 20],
    pub timestamp: u64,
    pub gas_limit: u64,
    pub system_policy_digest: [u8; 32],
}

/// Immutable context shared by execution, voting and finalization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbftExecutionContext {
    facts: PbftExecutionContextFacts,
    digest: [u8; 32],
}

impl PbftExecutionContext {
    /// Checks predecessor and epoch before binding supplied facts.
    /// Authentication and completion rechecks remain application owner duties.
    pub fn new(facts: PbftExecutionContextFacts) -> Result<Self> {
        ensure!(
            facts.prior.period.checked_next() == Some(facts.period),
            "PBFT_EXECUTION_PRIOR_PERIOD_MISMATCH"
        );
        ensure!(
            facts.delayed.period <= facts.prior.period,
            "PBFT_EXECUTION_DELAYED_PERIOD_MISMATCH"
        );
        ensure!(facts.state_api_epoch != 0, "PBFT_EXECUTION_EPOCH_MISSING");
        let mut out = RlpStream::new_list(19);
        out.append(&b"rustaxa-pbft-execution-context".as_slice())
            .append(&1_u8)
            .append(&facts.schedule.digest().as_slice())
            .append(&facts.period.as_u64())
            .append(&facts.previous_pbft_hash.as_slice());
        identity(&mut out, facts.prior);
        out.append(&facts.native_prior_digest.as_slice());
        identity(&mut out, facts.delayed);
        out.append(&facts.concrete_provenance_digest.as_slice())
            .append(&facts.concrete_generation)
            .append(&facts.owner_generation)
            .append(&facts.state_api_epoch)
            .append(&match facts.instruction_phase {
                PbftInstructionPhase::Californicum => 0_u8,
                PbftInstructionPhase::Ficus => 1,
                PbftInstructionPhase::Cacti => 2,
            })
            .append(&stage_tag(facts.schedule.correction_stage(facts.period)))
            .append(&u8::from(facts.cornus))
            .append(&facts.author.as_slice())
            .append(&facts.timestamp)
            .append(&facts.gas_limit)
            .append(&facts.system_policy_digest.as_slice());
        let digest = hash(&out.out());
        Ok(Self { facts, digest })
    }
    pub fn facts(&self) -> &PbftExecutionContextFacts {
        &self.facts
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn correction_stage(&self) -> CorrectionStage {
        self.facts.schedule.correction_stage(self.facts.period)
    }
}

/// One lookup against the native predecessor. Finalized exclusions must come
/// from durable finalized transaction metadata, never a network supplied flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionTransactionLookup {
    Included {
        hash: H256,
        rlp: Vec<u8>,
    },
    Finalized {
        hash: H256,
        period: FinalChainBlockNumber,
        location_rlp: Vec<u8>,
    },
    Missing {
        hash: H256,
    },
}
impl PbftExecutionTransactionLookup {
    fn hash(&self) -> H256 {
        match self {
            Self::Included { hash, .. } | Self::Finalized { hash, .. } | Self::Missing { hash } => {
                *hash
            }
        }
    }
}

/// Local capacity bounds. Exhaustion means unavailable, not invalidity.
#[derive(Clone, Copy, Debug)]
pub struct PbftExecutionInputLimits {
    pub maximum_bytes: u64,
    pub maximum_transactions: u64,
    pub maximum_dag_blocks: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionInputError {
    MissingTransaction(H256),
    LocalLimit,
    InvalidStructure(String),
}

impl std::fmt::Display for PbftExecutionInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for PbftExecutionInputError {}

/// Canonical bytes retained through voting and certified finalization.
#[derive(Clone, Debug)]
pub struct PbftExecutionSnapshot {
    block_rlp: Vec<u8>,
    link: PbftBlockLink,
    metadata: PbftBlockMetadata,
    dag: Vec<DagSyncBlockRlp>,
    transactions: Vec<LegacyTransactionEnvelope>,
    finalized: Vec<(H256, FinalChainBlockNumber, Vec<u8>)>,
    reward_votes: Vec<Vec<u8>>,
    context_digest: [u8; 32],
    digest: [u8; 32],
}

impl PbftExecutionSnapshot {
    /// Copies one trusted lookup epoch in first-occurrence DAG order.
    /// Missing bytes remain unavailable and cannot be silently omitted.
    pub fn new(
        block_rlp: Vec<u8>,
        dag: Vec<DagSyncBlockRlp>,
        lookups: Vec<PbftExecutionTransactionLookup>,
        reward_votes: Vec<Vec<u8>>,
        context: &PbftExecutionContext,
        limits: PbftExecutionInputLimits,
    ) -> std::result::Result<Self, PbftExecutionInputError> {
        let mut bytes = block_rlp.len() as u64;
        let mut charge = |amount: usize| -> std::result::Result<(), PbftExecutionInputError> {
            bytes = bytes
                .checked_add(amount as u64)
                .ok_or(PbftExecutionInputError::LocalLimit)?;
            if bytes > limits.maximum_bytes {
                return Err(PbftExecutionInputError::LocalLimit);
            }
            Ok(())
        };
        charge(0)?;
        for block in &dag {
            charge(block.block_rlp.len())?;
        }
        for lookup in &lookups {
            charge(match lookup {
                PbftExecutionTransactionLookup::Included { rlp, .. } => rlp.len(),
                PbftExecutionTransactionLookup::Finalized { location_rlp, .. } => {
                    location_rlp.len()
                }
                PbftExecutionTransactionLookup::Missing { .. } => 0,
            })?;
        }
        for vote in &reward_votes {
            charge(vote.len())?;
        }
        if dag.len() as u64 > limits.maximum_dag_blocks
            || lookups.len() as u64 > limits.maximum_transactions
        {
            return Err(PbftExecutionInputError::LocalLimit);
        }
        Self::resolve(block_rlp, dag, lookups, reward_votes, context).map_err(|error| {
            match error.downcast::<MissingInput>() {
                Ok(missing) => PbftExecutionInputError::MissingTransaction(missing.0),
                Err(error) => PbftExecutionInputError::InvalidStructure(format!("{error:#}")),
            }
        })
    }

    fn resolve(
        block_rlp: Vec<u8>,
        dag: Vec<DagSyncBlockRlp>,
        lookups: Vec<PbftExecutionTransactionLookup>,
        reward_votes: Vec<Vec<u8>>,
        context: &PbftExecutionContext,
    ) -> Result<Self> {
        exact_rlp(&block_rlp)?;
        ensure!(
            matches!(Rlp::new(&block_rlp).item_count()?, 8 | 9),
            "PBFT_EXECUTION_BLOCK_SHAPE"
        );
        let link = PbftBlockLink::try_from(SignedPbftBlockRlp::new(&block_rlp))?;
        let metadata = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(&block_rlp))?;
        ensure!(
            link.period == context.facts.period.as_u64()
                && link.prev_block_hash.0 == context.facts.previous_pbft_hash,
            "PBFT_EXECUTION_BLOCK_CONTEXT_MISMATCH"
        );
        ensure!(
            metadata.author.0 == context.facts.author
                && metadata.timestamp == context.facts.timestamp,
            "PBFT_EXECUTION_BLOCK_FACTS_MISMATCH"
        );
        let expected_order: H256 = Rlp::new(&block_rlp).val_at(2)?;
        let mut order = RlpStream::new_list(1);
        order.begin_list(dag.len());
        let mut seen_blocks = BTreeSet::new();
        let mut seen_transactions = BTreeSet::new();
        let mut expected = Vec::new();
        for block in &dag {
            exact_rlp(&block.block_rlp)?;
            ensure!(
                Rlp::new(&block.block_rlp).item_count()? == 8,
                "PBFT_EXECUTION_DAG_SHAPE"
            );
            ensure!(
                hash(&block.block_rlp) == block.hash.0 && seen_blocks.insert(block.hash),
                "PBFT_EXECUTION_DAG_IDENTITY_MISMATCH"
            );
            let decoded = DagBlock::try_from(DagBlockRlp::new(&block.block_rlp))?;
            ensure!(
                decoded.recover_sender().is_some(),
                "PBFT_EXECUTION_DAG_SIGNATURE"
            );
            order.append(&block.hash.0.as_slice());
            for transaction in decoded.transactions {
                if seen_transactions.insert(transaction) {
                    expected.push(transaction);
                }
            }
        }
        let observed_order = if dag.is_empty() {
            H256::zero()
        } else {
            H256(hash(&order.out()))
        };
        ensure!(
            expected_order == observed_order,
            "PBFT_EXECUTION_DAG_ORDER_MISMATCH"
        );
        ensure!(
            (link.pivot_dag_block_hash == H256::zero() && dag.is_empty())
                || seen_blocks.contains(&link.pivot_dag_block_hash),
            "PBFT_EXECUTION_DAG_PIVOT_MISMATCH"
        );
        let mut resolved = BTreeMap::new();
        for lookup in lookups {
            ensure!(
                resolved.insert(lookup.hash(), lookup).is_none(),
                "PBFT_EXECUTION_DUPLICATE_LOOKUP"
            );
        }
        ensure!(
            resolved.keys().all(|key| seen_transactions.contains(key)),
            "PBFT_EXECUTION_UNEXPECTED_LOOKUP"
        );
        let mut transactions = Vec::new();
        let mut finalized = Vec::new();
        for transaction in expected {
            match resolved
                .remove(&transaction)
                .ok_or(MissingInput(transaction))?
            {
                PbftExecutionTransactionLookup::Included { hash, rlp } => {
                    exact_rlp(&rlp)?;
                    let envelope = LegacyTransactionEnvelope::decode(&rlp)?;
                    ensure!(
                        envelope.hash == hash
                            && envelope.signature_valid
                            && envelope.sender.is_some(),
                        "PBFT_EXECUTION_TRANSACTION_IDENTITY"
                    );
                    ensure!(
                        envelope.chain_id == context.facts.schedule.chain_id,
                        "PBFT_EXECUTION_TRANSACTION_CHAIN"
                    );
                    transactions.push(envelope);
                }
                PbftExecutionTransactionLookup::Finalized {
                    hash,
                    period,
                    location_rlp,
                } => {
                    ensure!(
                        period <= context.facts.prior.period && !location_rlp.is_empty(),
                        "PBFT_EXECUTION_FINALIZED_EXCLUSION"
                    );
                    exact_rlp(&location_rlp)?;
                    finalized.push((hash, period, location_rlp));
                }
                PbftExecutionTransactionLookup::Missing { hash } => {
                    return Err(MissingInput(hash).into());
                }
            }
        }
        let expected_votes: Vec<H256> = Rlp::new(&block_rlp).list_at(6)?;
        let mut vote_hashes = Vec::new();
        for vote in &reward_votes {
            exact_rlp(vote)?;
            let inspection = crate::pbft_vote_validation::inspect_canonical_pbft_vote(vote)?;
            ensure!(
                inspection.signature_valid,
                "PBFT_EXECUTION_REWARD_VOTE_SIGNATURE"
            );
            vote_hashes.push(inspection.vote_hash);
        }
        ensure!(
            expected_votes == vote_hashes
                && vote_hashes.iter().copied().collect::<BTreeSet<_>>().len() == vote_hashes.len(),
            "PBFT_EXECUTION_REWARD_VOTE_INPUTS"
        );
        let mut out = RlpStream::new_list(7);
        out.append(&b"rustaxa-pbft-execution-snapshot".as_slice())
            .append(&1_u8)
            .append(&block_rlp);
        out.begin_list(dag.len());
        for block in &dag {
            out.append(&block.block_rlp);
        }
        out.begin_list(transactions.len());
        for transaction in &transactions {
            out.append(&transaction.rlp);
        }
        out.begin_list(finalized.len() + reward_votes.len());
        for (hash, period, location) in &finalized {
            out.begin_list(4)
                .append(&0_u8)
                .append(&hash.0.as_slice())
                .append(&period.as_u64())
                .append(location);
        }
        for vote in &reward_votes {
            out.begin_list(2).append(&1_u8).append(vote);
        }
        out.append(&context.digest.as_slice());
        let digest = hash(&out.out());
        Ok(Self {
            block_rlp,
            link,
            metadata,
            dag,
            transactions,
            finalized,
            reward_votes,
            context_digest: context.digest,
            digest,
        })
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn block_rlp(&self) -> &[u8] {
        &self.block_rlp
    }
    pub fn link(&self) -> PbftBlockLink {
        self.link
    }
    pub fn metadata(&self) -> &PbftBlockMetadata {
        &self.metadata
    }
    pub fn dag_blocks(&self) -> &[DagSyncBlockRlp] {
        &self.dag
    }
    pub fn transactions(&self) -> &[LegacyTransactionEnvelope] {
        &self.transactions
    }
    pub fn finalized_exclusions(&self) -> &[(H256, FinalChainBlockNumber, Vec<u8>)] {
        &self.finalized
    }
    pub fn reward_votes(&self) -> &[Vec<u8>] {
        &self.reward_votes
    }
}

#[derive(Debug)]
struct MissingInput(H256);
impl std::fmt::Display for MissingInput {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "missing candidate transaction {}", self.0)
    }
}
impl std::error::Error for MissingInput {}

/// Immutable request to an application-owned read-only execution leaf.
#[derive(Clone, Debug)]
pub struct PbftExecutionRequest {
    snapshot: Arc<PbftExecutionSnapshot>,
    context: PbftExecutionContext,
    digest: [u8; 32],
}
impl PbftExecutionRequest {
    pub fn new(
        snapshot: Arc<PbftExecutionSnapshot>,
        context: PbftExecutionContext,
    ) -> Result<Self> {
        ensure!(
            snapshot.context_digest == context.digest
                && snapshot.link.period == context.facts.period.as_u64()
                && snapshot.link.prev_block_hash.0 == context.facts.previous_pbft_hash
                && snapshot.metadata.author.0 == context.facts.author
                && snapshot.metadata.timestamp == context.facts.timestamp,
            "PBFT_EXECUTION_REQUEST_CONTEXT_MISMATCH"
        );
        let mut out = RlpStream::new_list(4);
        out.append(&b"rustaxa-pbft-execution-request".as_slice())
            .append(&1_u8)
            .append(&snapshot.digest.as_slice())
            .append(&context.digest.as_slice());
        let digest = hash(&out.out());
        Ok(Self {
            snapshot,
            context,
            digest,
        })
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn context(&self) -> &PbftExecutionContext {
        &self.context
    }
    pub fn snapshot(&self) -> &Arc<PbftExecutionSnapshot> {
        &self.snapshot
    }
}

/// Deterministic attempt failure. Local state errors never use this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionHardError {
    MalformedNestedSlashingProof(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionUnavailable {
    ExecutorUnavailable,
    PredecessorUnavailable,
    StateChanged,
    DelayedStateUnavailable,
    UnsupportedRules,
    LocalLimit,
}

/// Execution facts supplied by the qualified application leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionValidation {
    Valid {
        request_digest: [u8; 32],
        executed_transactions: u64,
        system_transactions: Vec<Vec<u8>>,
        results_digest: [u8; 32],
    },
    Invalid {
        request_digest: [u8; 32],
        transaction_position: u64,
        native_sequence: Option<u64>,
        cause: PbftExecutionHardError,
    },
    Unavailable {
        request_digest: [u8; 32],
        reason: PbftExecutionUnavailable,
    },
}

/// Consensus-owned execution evidence retaining exact immutable input bytes.
#[derive(Clone, Debug)]
pub struct ValidatedPbftExecution {
    request: Arc<PbftExecutionRequest>,
    system_transactions: Vec<Vec<u8>>,
    results_digest: [u8; 32],
}
impl ValidatedPbftExecution {
    pub(crate) fn from_report(
        request: Arc<PbftExecutionRequest>,
        report: PbftExecutionValidation,
    ) -> Result<Option<Self>> {
        match report {
            PbftExecutionValidation::Valid {
                request_digest,
                executed_transactions,
                system_transactions,
                results_digest,
            } => {
                let count = (request.snapshot.transactions.len() as u64)
                    .checked_add(system_transactions.len() as u64);
                ensure!(
                    request_digest == request.digest && count == Some(executed_transactions),
                    "PBFT_EXECUTION_REPORT_BINDING_MISMATCH"
                );
                for transaction in &system_transactions {
                    exact_rlp(transaction)?;
                    LegacyTransactionEnvelope::decode_system(transaction)?;
                }
                Ok(Some(Self {
                    request,
                    system_transactions,
                    results_digest,
                }))
            }
            PbftExecutionValidation::Invalid { request_digest, .. }
            | PbftExecutionValidation::Unavailable { request_digest, .. } => {
                ensure!(
                    request_digest == request.digest,
                    "PBFT_EXECUTION_REPORT_BINDING_MISMATCH"
                );
                Ok(None)
            }
        }
    }
    pub fn request(&self) -> &Arc<PbftExecutionRequest> {
        &self.request
    }
    pub fn system_transactions(&self) -> &[Vec<u8>] {
        &self.system_transactions
    }
    pub fn results_digest(&self) -> [u8; 32] {
        self.results_digest
    }
    pub fn matches(&self, request: &PbftExecutionRequest) -> bool {
        self.request.digest == request.digest
    }
}

fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut out);
    out
}
fn identity(out: &mut RlpStream, value: ConcreteStateIdentity) {
    out.begin_list(2)
        .append(&value.period.as_u64())
        .append(&value.state_root.as_slice());
}
fn stage_tag(stage: CorrectionStage) -> u8 {
    match stage {
        CorrectionStage::Historical => 0,
        CorrectionStage::TransientRollback => 1,
        CorrectionStage::SingleSelfDestructRefund => 2,
        CorrectionStage::CheckedReturnDataCopy => 3,
        CorrectionStage::BoundedFalconAbi => 4,
        CorrectionStage::SlashingHardError => 5,
    }
}
fn exact_rlp(bytes: &[u8]) -> Result<()> {
    ensure!(
        Rlp::new(bytes).payload_info()?.total() == bytes.len(),
        "PBFT_EXECUTION_TRAILING_RLP"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::U256;
    use k256::ecdsa::SigningKey;

    fn address(key: &SigningKey) -> [u8; 20] {
        hash(&key.verifying_key().to_encoded_point(false).as_bytes()[1..])[12..]
            .try_into()
            .unwrap()
    }

    fn context() -> PbftExecutionContext {
        PbftExecutionContext::new(PbftExecutionContextFacts {
            schedule: PbftExecutionSchedule::new(841, [1; 32], [2; 32], Some(10.into())),
            period: 10.into(),
            previous_pbft_hash: [7; 32],
            prior: ConcreteStateIdentity {
                period: 9.into(),
                state_root: [3; 32],
            },
            native_prior_digest: [4; 32],
            delayed: ConcreteStateIdentity {
                period: 8.into(),
                state_root: [5; 32],
            },
            concrete_provenance_digest: [6; 32],
            concrete_generation: 9,
            owner_generation: 1,
            state_api_epoch: 11,
            instruction_phase: PbftInstructionPhase::Ficus,
            cornus: true,
            author: address(&SigningKey::from_slice(&[1; 32]).unwrap()),
            timestamp: 100,
            gas_limit: 1_000_000,
            system_policy_digest: [8; 32],
        })
        .unwrap()
    }

    fn signed_transaction(nonce: u64) -> LegacyTransactionEnvelope {
        signed_transaction_for_chain(nonce, 841)
    }

    fn signed_transaction_for_chain(nonce: u64, chain_id: u64) -> LegacyTransactionEnvelope {
        let fields = |out: &mut RlpStream| {
            out.append(&nonce)
                .append(&1_u64)
                .append(&100_000_u64)
                .append(&[2_u8; 20].as_slice())
                .append(&1_u64)
                .append_empty_data();
        };
        let mut unsigned = RlpStream::new_list(if chain_id == 0 { 6 } else { 9 });
        fields(&mut unsigned);
        if chain_id != 0 {
            unsigned.append(&chain_id).append(&0_u8).append(&0_u8);
        }
        let key = SigningKey::from_slice(&[1; 32]).unwrap();
        let (sig, recovery) = key
            .sign_prehash_recoverable(&hash(&unsigned.out()))
            .unwrap();
        let sig = sig.to_bytes();
        let mut signed = RlpStream::new_list(9);
        fields(&mut signed);
        signed
            .append(
                &(if chain_id == 0 { 27 } else { chain_id * 2 + 35 }
                    + u64::from(recovery.to_byte())),
            )
            .append(&U256::from_big_endian(&sig[..32]))
            .append(&U256::from_big_endian(&sig[32..]));
        LegacyTransactionEnvelope::decode(&signed.out()).unwrap()
    }

    fn signed_dag(transactions: &[H256], level: u64) -> DagSyncBlockRlp {
        let key = SigningKey::from_slice(&[1; 32]).unwrap();
        let mut block = DagBlock {
            pivot: H256::zero(),
            level,
            timestamp: 90,
            vdf: vec![],
            tips: vec![],
            transactions: transactions.to_vec(),
            signature: [0; 65],
            gas_estimation: 200_000,
        };
        let (signature, recovery) = key
            .sign_prehash_recoverable(block.signing_hash().as_bytes())
            .unwrap();
        block.signature[..64].copy_from_slice(&signature.to_bytes());
        block.signature[64] = recovery.to_byte();
        let mut raw = RlpStream::new_list(8);
        raw.append(&block.pivot)
            .append(&level)
            .append(&block.timestamp)
            .append(&block.vdf)
            .append_list(&block.tips)
            .append_list(&block.transactions)
            .append(&block.signature.as_slice())
            .append(&block.gas_estimation);
        let block_rlp = raw.out().to_vec();
        DagSyncBlockRlp {
            hash: H256(hash(&block_rlp)),
            block_rlp,
        }
    }

    fn signed_block(dag: &[DagSyncBlockRlp]) -> Vec<u8> {
        let order_hash = if dag.is_empty() {
            H256::zero()
        } else {
            let mut order = RlpStream::new_list(1);
            order.begin_list(dag.len());
            for block in dag {
                order.append(&block.hash.0.as_slice());
            }
            H256(hash(&order.out()))
        };
        let fields = |out: &mut RlpStream| {
            out.append(&H256([7; 32]))
                .append(&dag.last().map_or(H256::zero(), |block| block.hash))
                .append(&order_hash)
                .append(&H256([9; 32]))
                .append(&10_u64)
                .append(&100_u64)
                .begin_list(0);
        };
        let mut unsigned = RlpStream::new_list(7);
        fields(&mut unsigned);
        let key = SigningKey::from_slice(&[1; 32]).unwrap();
        let (signature, recovery) = key
            .sign_prehash_recoverable(&hash(&unsigned.out()))
            .unwrap();
        let mut signature = signature.to_bytes().to_vec();
        signature.push(recovery.to_byte());
        let mut signed = RlpStream::new_list(8);
        fields(&mut signed);
        signed.append(&signature);
        signed.out().to_vec()
    }

    fn limits() -> PbftExecutionInputLimits {
        PbftExecutionInputLimits {
            maximum_bytes: 1_048_576,
            maximum_transactions: 100,
            maximum_dag_blocks: 100,
        }
    }
    fn empty_request() -> Arc<PbftExecutionRequest> {
        let context = context();
        let snapshot = PbftExecutionSnapshot::new(
            signed_block(&[]),
            vec![],
            vec![],
            vec![],
            &context,
            limits(),
        )
        .unwrap();
        Arc::new(PbftExecutionRequest::new(Arc::new(snapshot), context).unwrap())
    }

    #[test]
    fn execution_schedule_boundary_and_enrollment_are_explicit() {
        let schedule = context().facts.schedule;
        assert!(schedule.verify_enrollment(None).is_err());
        assert_eq!(
            schedule.correction_stage(9.into()),
            CorrectionStage::Historical
        );
        assert_eq!(
            schedule.correction_stage(10.into()),
            CorrectionStage::SlashingHardError
        );
        assert_eq!(
            schedule.correction_stage(11.into()),
            CorrectionStage::SlashingHardError
        );
        schedule.verify_enrollment(Some(schedule.digest())).unwrap();
        let disabled = PbftExecutionSchedule::new(841, [1; 32], [2; 32], None);
        assert!(disabled.verify_enrollment(Some(schedule.digest())).is_err());
        let other_network = PbftExecutionSchedule::new(842, [1; 32], [2; 32], Some(10.into()));
        assert!(
            other_network
                .verify_enrollment(Some(schedule.digest()))
                .is_err()
        );
    }

    #[test]
    fn execution_context_binds_each_execution_dependency() {
        let context = context();
        for change in 0..15 {
            let mut facts = context.facts.clone();
            match change {
                0 => facts.prior.state_root[0] ^= 1,
                1 => facts.native_prior_digest[0] ^= 1,
                2 => facts.delayed.state_root[0] ^= 1,
                3 => facts.concrete_provenance_digest[0] ^= 1,
                4 => facts.concrete_generation += 1,
                5 => facts.owner_generation += 1,
                6 => facts.state_api_epoch += 1,
                7 => facts.instruction_phase = PbftInstructionPhase::Cacti,
                8 => facts.cornus = false,
                9 => facts.author[0] ^= 1,
                10 => facts.timestamp += 1,
                11 => facts.gas_limit += 1,
                12 => facts.system_policy_digest[0] ^= 1,
                13 => facts.schedule.configuration[0] ^= 1,
                14 => facts.previous_pbft_hash[0] ^= 1,
                _ => unreachable!(),
            }
            assert_ne!(
                PbftExecutionContext::new(facts).unwrap().digest(),
                context.digest(),
                "dependency {change}"
            );
        }
        let mut facts = context.facts.clone();
        facts.state_api_epoch = 0;
        assert!(PbftExecutionContext::new(facts).is_err());
        let mut facts = context.facts;
        facts.prior.period = 8.into();
        assert!(PbftExecutionContext::new(facts).is_err());
    }

    #[test]
    fn execution_snapshot_keeps_first_occurrence_and_owned_bytes() {
        let first = signed_transaction(1);
        let second = signed_transaction(2);
        let dag = vec![
            signed_dag(&[first.hash, second.hash], 1),
            signed_dag(&[first.hash], 2),
        ];
        let block = signed_block(&dag);
        let context = context();
        let mut lookups = vec![
            PbftExecutionTransactionLookup::Included {
                hash: second.hash,
                rlp: second.rlp.clone(),
            },
            PbftExecutionTransactionLookup::Included {
                hash: first.hash,
                rlp: first.rlp.clone(),
            },
        ];
        let snapshot =
            PbftExecutionSnapshot::new(block, dag, lookups.clone(), vec![], &context, limits())
                .unwrap();
        lookups.clear();
        assert_eq!(
            snapshot
                .transactions()
                .iter()
                .map(|tx| tx.hash)
                .collect::<Vec<_>>(),
            vec![first.hash, second.hash]
        );
        assert_eq!(snapshot.transactions()[0].rlp, first.rlp);
        let snapshot = Arc::new(snapshot);
        let mut changed = context.facts.clone();
        changed.state_api_epoch += 1;
        assert!(
            PbftExecutionRequest::new(snapshot, PbftExecutionContext::new(changed).unwrap())
                .is_err()
        );
    }

    #[test]
    fn execution_snapshot_missing_and_limits_are_unavailable() {
        let transaction = signed_transaction(1);
        let dag = vec![signed_dag(&[transaction.hash], 1)];
        let block = signed_block(&dag);
        let context = context();
        assert_eq!(
            PbftExecutionSnapshot::new(
                block.clone(),
                dag.clone(),
                vec![PbftExecutionTransactionLookup::Missing {
                    hash: transaction.hash
                }],
                vec![],
                &context,
                limits()
            )
            .unwrap_err(),
            PbftExecutionInputError::MissingTransaction(transaction.hash)
        );
        let small = PbftExecutionInputLimits {
            maximum_bytes: 1,
            ..limits()
        };
        assert_eq!(
            PbftExecutionSnapshot::new(block, dag, vec![], vec![], &context, small).unwrap_err(),
            PbftExecutionInputError::LocalLimit
        );
    }

    #[test]
    fn execution_snapshot_rejects_changed_bytes_and_future_exclusion() {
        let transaction = signed_transaction(1);
        let dag = vec![signed_dag(&[transaction.hash], 1)];
        let block = signed_block(&dag);
        let context = context();
        let unprotected = signed_transaction_for_chain(1, 0);
        assert!(unprotected.signature_valid);
        assert_eq!(unprotected.chain_id, 0);
        let unprotected_dag = vec![signed_dag(&[unprotected.hash], 1)];
        let result = PbftExecutionSnapshot::new(
            signed_block(&unprotected_dag),
            unprotected_dag,
            vec![PbftExecutionTransactionLookup::Included {
                hash: unprotected.hash,
                rlp: unprotected.rlp,
            }],
            vec![],
            &context,
            limits(),
        );
        assert!(
            matches!(result, Err(PbftExecutionInputError::InvalidStructure(ref message)) if message.contains("PBFT_EXECUTION_TRANSACTION_CHAIN"))
        );
        let mut extra = dag[0].clone();
        let raw = Rlp::new(&extra.block_rlp);
        let mut expanded = RlpStream::new_list(9);
        for field in raw.iter() {
            expanded.append_raw(field.as_raw(), 1);
        }
        expanded.append(&1_u8);
        extra.block_rlp = expanded.out().to_vec();
        extra.hash = H256(hash(&extra.block_rlp));
        let result = PbftExecutionSnapshot::new(
            signed_block(&[extra.clone()]),
            vec![extra],
            vec![PbftExecutionTransactionLookup::Included {
                hash: transaction.hash,
                rlp: transaction.rlp.clone(),
            }],
            vec![],
            &context,
            limits(),
        );
        assert!(
            matches!(result, Err(PbftExecutionInputError::InvalidStructure(ref message)) if message.contains("PBFT_EXECUTION_DAG_SHAPE"))
        );
        let excluded = PbftExecutionTransactionLookup::Finalized {
            hash: transaction.hash,
            period: 10.into(),
            location_rlp: vec![0xc0],
        };
        assert!(
            PbftExecutionSnapshot::new(
                block.clone(),
                dag.clone(),
                vec![excluded],
                vec![],
                &context,
                limits()
            )
            .is_err()
        );
        let wrong = signed_transaction(2);
        assert!(
            PbftExecutionSnapshot::new(
                block.clone(),
                dag.clone(),
                vec![PbftExecutionTransactionLookup::Included {
                    hash: transaction.hash,
                    rlp: wrong.rlp
                }],
                vec![],
                &context,
                limits()
            )
            .is_err()
        );
        let mut changed = dag;
        changed[0].block_rlp.push(0);
        assert!(
            PbftExecutionSnapshot::new(block, changed, vec![], vec![], &context, limits()).is_err()
        );
    }

    #[test]
    fn execution_authority_requires_bound_complete_report() {
        let request = empty_request();
        let report = PbftExecutionValidation::Valid {
            request_digest: request.digest(),
            executed_transactions: 0,
            system_transactions: vec![],
            results_digest: [1; 32],
        };
        let authority = ValidatedPbftExecution::from_report(request.clone(), report.clone())
            .unwrap()
            .unwrap();
        assert!(authority.matches(&request));
        let mut wrong = report;
        if let PbftExecutionValidation::Valid {
            ref mut executed_transactions,
            ..
        } = wrong
        {
            *executed_transactions = 1;
        }
        assert!(ValidatedPbftExecution::from_report(request.clone(), wrong).is_err());
        let unavailable = PbftExecutionValidation::Unavailable {
            request_digest: request.digest(),
            reason: PbftExecutionUnavailable::ExecutorUnavailable,
        };
        assert!(
            ValidatedPbftExecution::from_report(request, unavailable)
                .unwrap()
                .is_none()
        );
    }
}
