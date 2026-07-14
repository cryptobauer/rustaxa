//! Deterministic slashing-proof planning for Rust-backed consensus shims.
//!
//! The planner owns the consensus-facing decision for double-vote proof
//! submission: it validates that two votes describe the same PBFT slot,
//! canonicalizes their hashes, builds the slashing contract calldata, and
//! tracks proofs already submitted by this node. It deliberately does not own
//! wallet/account lookup, gas pricing, transaction signing, or transaction pool
//! insertion; those remain live-node responsibilities on the C++ side until the
//! surrounding transaction pipeline is moved to Rust.

use anyhow::{Result, anyhow, ensure};
use ethereum_types::{H160, H256, U256};
use rlp::{Rlp, RlpStream};
use std::collections::{HashSet, VecDeque};
use tiny_keccak::{Hasher, Keccak};

const DOUBLE_VOTING_PROOF_FUNCTION: &str = "commitDoubleVotingProof(bytes,bytes)";
const DOUBLE_VOTING_GAS_LIMIT: u64 = 100_000;
const WORD_SIZE: usize = 32;
const SIGNATURE_SIZE: usize = 65;
const VRF_SORTITION_PROOF_SIZE: usize = 80;
const SLASHING_CONTRACT: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xEE,
];

/// Decoded legacy PBFT sortition fields required for double-vote slot checks.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct LegacyVrfPbftSortition {
    /// PBFT period carried inside the VRF sortition payload.
    pub period: u64,
    /// PBFT round carried inside the VRF sortition payload.
    pub round: u32,
    /// PBFT step carried inside the VRF sortition payload.
    pub step: u32,
    /// Legacy 80-byte VRF proof bytes preserved for vote-hash parity.
    pub proof: [u8; VRF_SORTITION_PROOF_SIZE],
}

/// PBFT vote metadata extracted from legacy calldata payload.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct LegacyPbftVoteMetadata {
    /// PBFT block hash that this vote targets.
    pub block_hash: H256,
    /// Legacy unsigned vote hash recovered from block hash and sortition RLP.
    pub vote_hash: H256,
    /// PBFT period extracted from the embedded sortition payload.
    pub period: u64,
    /// PBFT round extracted from the embedded sortition payload.
    pub round: u32,
    /// PBFT step extracted from the embedded sortition payload.
    pub step: u32,
    /// Optional vote weight present in some persisted legacy vote RLPs.
    pub weight: Option<u64>,
}

/// Compact verified fact for legacy PBFT double-vote proof payload inspection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct VerifiedLegacyDoubleVotingProof {
    /// Validator address recovered from both vote signatures.
    pub offender: H160,
    /// Canonical duplicate-detection key derived from sorted vote hashes.
    pub proof_key: H256,
    /// Vote hashes sorted by byte order so callers can persist stable proof facts.
    pub sorted_vote_hashes: [H256; 2],
    /// Metadata for the first ABI `bytes` vote payload.
    pub vote_a: LegacyPbftVoteMetadata,
    /// Metadata for the second ABI `bytes` vote payload.
    pub vote_b: LegacyPbftVoteMetadata,
}

/// Verifies one legacy `commitDoubleVotingProof(bytes,bytes)` calldata payload.
///
/// It performs ABI decoding, legacy PbftVote parsing, signature recovery,
/// and the same slot/hash/sanity checks that the contract enforces.
pub fn verify_legacy_double_voting_proof_call_data(
    calldata: &[u8],
) -> Result<VerifiedLegacyDoubleVotingProof> {
    let (vote_a_rlp, vote_b_rlp) = decode_commit_double_voting_call_data(calldata)?;
    let vote_a = decode_legacy_pbft_vote(vote_a_rlp)?;
    let vote_b = decode_legacy_pbft_vote(vote_b_rlp)?;

    ensure!(
        vote_a.metadata.vote_hash != vote_b.metadata.vote_hash,
        "votes are identical"
    );
    ensure!(
        vote_a.metadata.period == vote_b.metadata.period,
        "invalid votes period/round/step"
    );
    ensure!(
        vote_a.metadata.round == vote_b.metadata.round,
        "invalid votes period/round/step"
    );
    ensure!(
        vote_a.metadata.step == vote_b.metadata.step,
        "invalid votes period/round/step"
    );

    ensure!(
        vote_a.metadata.block_hash != vote_b.metadata.block_hash,
        "invalid votes block hash"
    );

    if vote_a.metadata.step >= 5 && vote_a.metadata.step % 2 == 1 {
        let vote_a_zero = vote_a.metadata.block_hash.is_zero();
        let vote_b_zero = vote_b.metadata.block_hash.is_zero();
        ensure!(
            vote_a_zero == vote_b_zero,
            "invalid mixed zero/non-zero next-vote block hashes"
        );
    }

    let vote_a_offender = recover_validator_address(&vote_a.metadata.vote_hash, &vote_a.signature)
        .ok_or_else(|| anyhow!("invalid vote signature"))?;
    let vote_b_offender = recover_validator_address(&vote_b.metadata.vote_hash, &vote_b.signature)
        .ok_or_else(|| anyhow!("invalid vote signature"))?;

    ensure!(
        vote_a_offender == vote_b_offender,
        "invalid votes validator"
    );

    let sorted_vote_hashes = if vote_a.metadata.vote_hash < vote_b.metadata.vote_hash {
        [vote_a.metadata.vote_hash, vote_b.metadata.vote_hash]
    } else {
        [vote_b.metadata.vote_hash, vote_a.metadata.vote_hash]
    };

    Ok(VerifiedLegacyDoubleVotingProof {
        offender: vote_a_offender,
        proof_key: double_voting_proof_hash(sorted_vote_hashes[0], sorted_vote_hashes[1]),
        sorted_vote_hashes,
        vote_a: vote_a.metadata,
        vote_b: vote_b.metadata,
    })
}

#[derive(Debug)]
struct DecodedLegacyPbftVote {
    metadata: LegacyPbftVoteMetadata,
    signature: [u8; SIGNATURE_SIZE],
}

fn decode_commit_double_voting_call_data(calldata: &[u8]) -> Result<(&[u8], &[u8])> {
    ensure!(
        calldata.len() >= 4,
        "commitDoubleVotingProof calldata is too short"
    );
    ensure!(
        calldata.starts_with(&function_selector(DOUBLE_VOTING_PROOF_FUNCTION)),
        "calldata selector does not match commitDoubleVotingProof(bytes,bytes)"
    );
    ensure!(
        calldata.len() >= 4 + 2 * WORD_SIZE,
        "commitDoubleVotingProof calldata must contain two argument offsets"
    );

    let offset_a = decode_call_data_offset(&calldata[4..36], "vote_a offset")?;
    let offset_b = decode_call_data_offset(&calldata[36..68], "vote_b offset")?;

    let vote_a = decode_call_data_bytes(calldata, offset_a, "vote_a")?;
    let vote_b = decode_call_data_bytes(calldata, offset_b, "vote_b")?;

    Ok((vote_a, vote_b))
}

fn decode_call_data_offset(word: &[u8], field: &str) -> Result<usize> {
    ensure!(word.len() == WORD_SIZE, "{field} must be one ABI word");
    ensure!(
        word[..24].iter().all(|byte| *byte == 0),
        "{field} exceeds supported address width"
    );
    let mut tail = [0u8; 8];
    tail.copy_from_slice(&word[24..WORD_SIZE]);
    Ok(u64::from_be_bytes(tail) as usize)
}

fn decode_call_data_bytes<'a>(calldata: &'a [u8], offset: usize, field: &str) -> Result<&'a [u8]> {
    let offset = offset
        .checked_add(4)
        .ok_or_else(|| anyhow!("{field} absolute offset overflow"))?;
    let header = offset
        .checked_add(WORD_SIZE)
        .ok_or_else(|| anyhow!("{field} offset overflow"))?;
    ensure!(
        offset >= 4 + WORD_SIZE * 2 && header <= calldata.len(),
        "{field} offset out of bounds"
    );

    let length = decode_call_data_offset(&calldata[offset..header], &format!("{field} length"))?;
    let end = header
        .checked_add(length)
        .ok_or_else(|| anyhow!("{field} payload length overflows"))?;
    ensure!(
        end <= calldata.len(),
        "{field} payload exceeds calldata bounds"
    );

    Ok(&calldata[header..end])
}

fn decode_legacy_pbft_vote(vote_rlp: &[u8]) -> Result<DecodedLegacyPbftVote> {
    let vote = Rlp::new(vote_rlp);
    let item_count = vote.item_count()?;
    ensure!(
        item_count == 3 || item_count == 4,
        "legacy PbftVote must contain block_hash, vrf_sortition and signature"
    );

    let block_hash: H256 = vote.val_at(0)?;
    let vrf_sortition = vote.val_at::<Vec<u8>>(1)?;
    let signature = vote.val_at::<Vec<u8>>(2)?;
    ensure!(
        signature.len() == SIGNATURE_SIZE,
        "legacy PbftVote signature must be exactly 65 bytes"
    );

    let mut signature_bytes = [0u8; SIGNATURE_SIZE];
    signature_bytes.copy_from_slice(&signature);

    let sortition = decode_legacy_vrf_sortition(&vrf_sortition)?;
    let vote_hash = legacy_pbft_vote_hash(block_hash, &vrf_sortition);
    let weight = if item_count == 4 {
        Some(vote.val_at(3)?)
    } else {
        None
    };

    Ok(DecodedLegacyPbftVote {
        metadata: LegacyPbftVoteMetadata {
            block_hash,
            vote_hash,
            period: sortition.period,
            round: sortition.round,
            step: sortition.step,
            weight,
        },
        signature: signature_bytes,
    })
}

fn decode_legacy_vrf_sortition(vrf_sortition: &[u8]) -> Result<LegacyVrfPbftSortition> {
    let vrf = Rlp::new(vrf_sortition);
    ensure!(
        vrf.item_count()? == 4,
        "VrfPbftSortition RLP must contain period, round, step, proof"
    );

    let period = vrf.val_at(0)?;
    let round = vrf.val_at(1)?;
    let step = vrf.val_at(2)?;
    let proof: Vec<u8> = vrf.val_at(3)?;
    ensure!(
        proof.len() == VRF_SORTITION_PROOF_SIZE,
        "VrfPbftSortition proof must be exactly 80 bytes"
    );
    let mut proof_bytes = [0u8; VRF_SORTITION_PROOF_SIZE];
    proof_bytes.copy_from_slice(&proof);

    Ok(LegacyVrfPbftSortition {
        period,
        round,
        step,
        proof: proof_bytes,
    })
}

fn recover_validator_address(hash: &H256, signature: &[u8; SIGNATURE_SIZE]) -> Option<H160> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

    let recovery_id = RecoveryId::try_from(signature[64]).ok()?;
    let signature = Signature::try_from(&signature[..SIGNATURE_SIZE - 1]).ok()?;
    let public_key =
        VerifyingKey::recover_from_prehash(hash.as_bytes(), &signature, recovery_id).ok()?;
    let uncompressed = public_key.to_encoded_point(false);
    let public_key_hash = keccak256(&uncompressed.as_bytes()[1..]);
    Some(H160::from_slice(&public_key_hash.as_bytes()[12..]))
}

fn legacy_pbft_vote_hash(block_hash: H256, vrf_sortition_rlp: &[u8]) -> H256 {
    let mut stream = RlpStream::new_list(2);
    stream.append(&block_hash);
    stream.append(&vrf_sortition_rlp);
    keccak256(&stream.out())
}

/// Account facts for a configured wallet that may submit a slashing proof.
///
/// C++ supplies these facts in configured wallet order after reading FinalChain
/// account state. Rust uses only the legacy funding rule, `balance != 0`, and
/// returns the selected wallet index to C++ for signing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashingSubmitterFact {
    pub wallet_index: usize,
    pub nonce: U256,
    pub balance: U256,
}

/// Facts from two candidate PBFT votes needed to plan a slashing proof.
///
/// Hashes and RLP payloads are supplied by the C++ vote objects for now.
/// Period, round, and step are passed as plain scalar facts so Rust can make
/// the slot-equality decision without depending on C++ vote ownership.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoubleVotingProofInput {
    pub vote_a_hash: H256,
    pub vote_b_hash: H256,
    pub vote_a_period: u64,
    pub vote_b_period: u64,
    pub vote_a_round: u64,
    pub vote_b_round: u64,
    pub vote_a_step: u64,
    pub vote_b_step: u64,
    pub vote_a_rlp: Vec<u8>,
    pub vote_b_rlp: Vec<u8>,
    pub submitters: Vec<SlashingSubmitterFact>,
}

/// Expected result code for a double-voting proof planning attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoubleVotingProofPlanStatus {
    Planned,
    Disabled,
    /// Vote A predates the configured Magnolia activation period. The legacy
    /// manager gates submission on vote A only and permits the activation
    /// period itself.
    BeforeMagnoliaActivation,
    MismatchedVoteCoordinates,
    DuplicateProof,
    NoFundedSubmitter,
}

/// Result code after the transaction executor reports a planned slashing proof
/// insertion attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DoubleVotingProofSubmissionStatus {
    /// The transaction executor accepted the planned transaction and Rust
    /// inserted the proof hash into duplicate protection.
    Accepted,
    /// The transaction executor rejected the planned transaction; Rust leaves
    /// duplicate protection unchanged so a later attempt may retry.
    RejectedByExecutor,
    /// The executor reported acceptance for a proof already present in duplicate
    /// protection. This is treated as not submitted because no new transaction
    /// should be counted.
    DuplicateProof,
}

/// Rust decision returned for one double-vote proof attempt.
///
/// `should_submit` is false when reporting is disabled, vote A predates
/// Magnolia, the votes are for different PBFT slots, the proof hash was already
/// submitted, or no funded submitter exists. When it is true, `proof_hash` is
/// the canonical duplicate-cache key and `call_data` is the byte-for-byte
/// slashing contract call payload C++ should place into the transaction input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoubleVotingProofPlan {
    pub status: DoubleVotingProofPlanStatus,
    pub should_submit: bool,
    pub proof_hash: H256,
    pub contract_address: [u8; 20],
    pub value: U256,
    pub gas_limit: u64,
    pub call_data: Vec<u8>,
    pub wallet_index: usize,
    pub nonce: U256,
}

/// Typed Rust-owned classification for a slashing transaction executor report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoubleVotingProofSubmissionPlan {
    pub status: DoubleVotingProofSubmissionStatus,
    pub submitted: bool,
    pub mark_inserted: bool,
}

/// Double-voting proof planner with a bounded submitted-proof cache.
///
/// Magnolia activation is immutable bootstrap configuration. The cache mirrors
/// the legacy `ExpirationCache` shape: insertion is FIFO and when the size
/// exceeds `max_submitted_proofs`, `delete_step` oldest entries are evicted.
/// Proofs are marked submitted only after C++ successfully inserts the generated
/// transaction, matching the legacy side-effect ordering.
pub struct SlashingProofPlanner {
    report_malicious_behaviour: bool,
    magnolia_activation_period: u64,
    submitted_proofs: HashSet<H256>,
    submitted_order: VecDeque<H256>,
    max_submitted_proofs: usize,
    delete_step: usize,
}

impl SlashingProofPlanner {
    /// Creates an empty planner for local slashing proof submission.
    ///
    /// `report_malicious_behaviour` disables proof generation when false.
    /// `magnolia_activation_period` is the first vote-A period eligible for
    /// proof submission; period zero therefore means active from genesis.
    /// Cache limits must be non-zero so duplicate detection has the same
    /// bounded behavior as the legacy manager.
    pub fn new(
        report_malicious_behaviour: bool,
        magnolia_activation_period: u64,
        max_submitted_proofs: usize,
        delete_step: usize,
    ) -> Result<Self> {
        ensure!(
            max_submitted_proofs > 0,
            "slashing proof cache max size must be non-zero"
        );
        ensure!(
            delete_step > 0,
            "slashing proof cache delete step must be non-zero"
        );
        Ok(Self {
            report_malicious_behaviour,
            magnolia_activation_period,
            submitted_proofs: HashSet::new(),
            submitted_order: VecDeque::new(),
            max_submitted_proofs,
            delete_step,
        })
    }

    /// Builds a double-voting proof transaction plan when the proof is new.
    ///
    /// Disabled reporting takes precedence over the vote-A Magnolia boundary,
    /// followed by slot validation, duplicate detection, and submitter choice.
    /// Rejected plans do not mutate the submitted-proof cache; callers must
    /// invoke [`SlashingProofPlanner::mark_submitted`] only after the transaction
    /// was accepted for insertion.
    pub fn plan_double_voting_proof(&self, input: DoubleVotingProofInput) -> DoubleVotingProofPlan {
        if !self.report_malicious_behaviour {
            return DoubleVotingProofPlan::not_submitted(DoubleVotingProofPlanStatus::Disabled);
        }
        if input.vote_a_period < self.magnolia_activation_period {
            return DoubleVotingProofPlan::not_submitted(
                DoubleVotingProofPlanStatus::BeforeMagnoliaActivation,
            );
        }
        if !same_pbft_slot(&input) {
            return DoubleVotingProofPlan::not_submitted(
                DoubleVotingProofPlanStatus::MismatchedVoteCoordinates,
            );
        }

        let proof_hash = double_voting_proof_hash(input.vote_a_hash, input.vote_b_hash);
        if self.submitted_proofs.contains(&proof_hash) {
            return DoubleVotingProofPlan::not_submitted(
                DoubleVotingProofPlanStatus::DuplicateProof,
            );
        }

        let Some(submitter) = input
            .submitters
            .iter()
            .find(|submitter| !submitter.balance.is_zero())
        else {
            return DoubleVotingProofPlan {
                status: DoubleVotingProofPlanStatus::NoFundedSubmitter,
                should_submit: false,
                proof_hash,
                contract_address: SLASHING_CONTRACT,
                value: U256::zero(),
                gas_limit: DOUBLE_VOTING_GAS_LIMIT,
                call_data: Vec::new(),
                wallet_index: 0,
                nonce: U256::zero(),
            };
        };

        DoubleVotingProofPlan {
            status: DoubleVotingProofPlanStatus::Planned,
            should_submit: true,
            proof_hash,
            contract_address: SLASHING_CONTRACT,
            value: U256::zero(),
            gas_limit: DOUBLE_VOTING_GAS_LIMIT,
            call_data: commit_double_voting_proof_call_data(&input.vote_a_rlp, &input.vote_b_rlp),
            wallet_index: submitter.wallet_index,
            nonce: submitter.nonce,
        }
    }

    /// Alias for explicit Rust/bridge semantics naming.
    ///
    /// Returns false when the proof hash already exists in the duplicate cache.
    pub fn mark_double_voting_proof_submission(&mut self, proof_hash: H256) -> bool {
        self.mark_submitted(proof_hash)
    }

    /// Classifies a transaction executor report for a planned double-voting proof.
    ///
    /// Rust owns the submitted-proof duplicate cache. C++ reports only whether
    /// the transaction executor accepted insertion for the planned transaction.
    /// Rejected executor results do not mark the proof submitted, preserving the
    /// ability to retry.
    pub fn report_double_voting_proof_submission(
        &mut self,
        proof_hash: H256,
        transaction_inserted: bool,
    ) -> DoubleVotingProofSubmissionPlan {
        if !transaction_inserted {
            return DoubleVotingProofSubmissionPlan {
                status: DoubleVotingProofSubmissionStatus::RejectedByExecutor,
                submitted: false,
                mark_inserted: false,
            };
        }
        let mark_inserted = self.mark_submitted(proof_hash);
        DoubleVotingProofSubmissionPlan {
            status: if mark_inserted {
                DoubleVotingProofSubmissionStatus::Accepted
            } else {
                DoubleVotingProofSubmissionStatus::DuplicateProof
            },
            submitted: mark_inserted,
            mark_inserted,
        }
    }

    /// Marks a proof hash as submitted and updates the bounded duplicate cache.
    ///
    /// Returns false when the proof was already known. On insertion overflow,
    /// the oldest `delete_step` entries are evicted, matching legacy cache
    /// behavior.
    pub fn mark_submitted(&mut self, proof_hash: H256) -> bool {
        if !self.submitted_proofs.insert(proof_hash) {
            return false;
        }
        self.submitted_order.push_back(proof_hash);
        if self.submitted_proofs.len() > self.max_submitted_proofs {
            for _ in 0..self.delete_step {
                let Some(expired) = self.submitted_order.pop_front() else {
                    break;
                };
                self.submitted_proofs.remove(&expired);
            }
        }
        true
    }
}

impl DoubleVotingProofPlan {
    fn not_submitted(status: DoubleVotingProofPlanStatus) -> Self {
        Self {
            status,
            should_submit: false,
            proof_hash: H256::zero(),
            contract_address: SLASHING_CONTRACT,
            value: U256::zero(),
            gas_limit: DOUBLE_VOTING_GAS_LIMIT,
            call_data: Vec::new(),
            wallet_index: 0,
            nonce: U256::zero(),
        }
    }
}

fn same_pbft_slot(input: &DoubleVotingProofInput) -> bool {
    input.vote_a_period == input.vote_b_period
        && input.vote_a_round == input.vote_b_round
        && input.vote_a_step == input.vote_b_step
}

fn double_voting_proof_hash(vote_a_hash: H256, vote_b_hash: H256) -> H256 {
    let (first, second) = if vote_a_hash < vote_b_hash {
        (vote_a_hash, vote_b_hash)
    } else {
        (vote_b_hash, vote_a_hash)
    };

    let mut stream = RlpStream::new_list(2);
    stream.append(&first);
    stream.append(&second);
    keccak256(&stream.out())
}

fn commit_double_voting_proof_call_data(vote_a_rlp: &[u8], vote_b_rlp: &[u8]) -> Vec<u8> {
    let tail_a = solidity_bytes(vote_a_rlp);
    let tail_b = solidity_bytes(vote_b_rlp);
    let offset_a = WORD_SIZE * 2;
    let offset_b = offset_a + tail_a.len();

    let mut out = function_selector(DOUBLE_VOTING_PROOF_FUNCTION).to_vec();
    out.extend_from_slice(&u256_word(offset_a));
    out.extend_from_slice(&u256_word(offset_b));
    out.extend_from_slice(&tail_a);
    out.extend_from_slice(&tail_b);
    out
}

fn solidity_bytes(value: &[u8]) -> Vec<u8> {
    let mut out = u256_word(value.len()).to_vec();
    out.extend_from_slice(value);
    let padding = WORD_SIZE - (value.len() % WORD_SIZE);
    out.extend(std::iter::repeat_n(0, padding));
    out
}

fn function_selector(function: &str) -> [u8; 4] {
    let hash = keccak256(function.as_bytes());
    [hash[0], hash[1], hash[2], hash[3]]
}

fn u256_word(value: usize) -> [u8; WORD_SIZE] {
    U256::from(value).to_big_endian()
}

fn keccak256(data: &[u8]) -> H256 {
    let mut out = [0u8; 32];
    let mut hasher = Keccak::v256();
    hasher.update(data);
    hasher.finalize(&mut out);
    H256(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;

    fn hash(byte: u8) -> H256 {
        H256([byte; 32])
    }

    fn input(hash_a: u8, hash_b: u8) -> DoubleVotingProofInput {
        DoubleVotingProofInput {
            vote_a_hash: hash(hash_a),
            vote_b_hash: hash(hash_b),
            vote_a_period: 10,
            vote_b_period: 10,
            vote_a_round: 2,
            vote_b_round: 2,
            vote_a_step: 3,
            vote_b_step: 3,
            vote_a_rlp: vec![0xc1, 0x01],
            vote_b_rlp: vec![0xc1, 0x02],
            submitters: vec![SlashingSubmitterFact {
                wallet_index: 0,
                nonce: U256::from(7),
                balance: U256::from(1),
            }],
        }
    }

    fn hex_nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => panic!("invalid hex nibble"),
        }
    }

    fn hex_bytes(value: &str) -> Vec<u8> {
        let chunks = value.as_bytes().chunks_exact(2);
        assert!(chunks.remainder().is_empty());
        chunks
            .map(|chunk| (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]))
            .collect()
    }

    fn h256_hex(value: &str) -> H256 {
        H256(hex_bytes(value).try_into().unwrap())
    }

    fn sign_vote(
        signing_key: &SigningKey,
        block_hash: H256,
        vrf_sortition_rlp: &[u8],
        signature_override: Option<[u8; SIGNATURE_SIZE]>,
        weight: Option<u64>,
    ) -> (Vec<u8>, [u8; SIGNATURE_SIZE]) {
        let vote_hash = legacy_pbft_vote_hash(block_hash, vrf_sortition_rlp);
        let signature = signature_override.unwrap_or_else(|| {
            let (sig, recovery_id) = signing_key
                .sign_prehash_recoverable(vote_hash.as_bytes())
                .unwrap();
            let signature = sig.to_bytes();
            let mut combined = [0u8; SIGNATURE_SIZE];
            combined[..64].copy_from_slice(&signature);
            combined[64] = recovery_id.to_byte();
            combined
        });

        let mut stream = RlpStream::new_list(if weight.is_some() { 4 } else { 3 });
        stream.append(&block_hash);
        stream.append(&vrf_sortition_rlp);
        stream.append(&signature.as_slice());
        if let Some(weight) = weight {
            stream.append(&weight);
        }
        (stream.out().to_vec(), signature)
    }

    fn encode_vrf_sortition(period: u64, round: u64, step: u64, proof_byte: u8) -> Vec<u8> {
        let mut sortition = RlpStream::new_list(4);
        sortition.append(&period);
        sortition.append(&round);
        sortition.append(&step);
        sortition.append(&vec![proof_byte; VRF_SORTITION_PROOF_SIZE]);
        sortition.out().to_vec()
    }

    fn address_from_signing_key(signing_key: &SigningKey) -> H160 {
        let public_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key_hash = keccak256(&public_key.as_bytes()[1..]);
        H160::from_slice(&public_key_hash.as_bytes()[12..])
    }

    #[test]
    fn verifies_valid_legacy_double_voting_call_data() {
        let signing_key = SigningKey::from_slice(&[0x11; 32]).unwrap();
        let sortition_a = encode_vrf_sortition(10, 2, 4, 0x5a);
        let sortition_b = sortition_a.clone();

        let (vote_a, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(7),
            &sortition_a,
            None,
            None,
        );
        let (vote_b, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(8),
            &sortition_b,
            None,
            Some(3),
        );
        let calldata = commit_double_voting_proof_call_data(&vote_a, &vote_b);

        let verified = verify_legacy_double_voting_proof_call_data(&calldata).unwrap();

        assert_eq!(verified.offender, address_from_signing_key(&signing_key));
        assert_eq!(verified.vote_a.period, 10);
        assert_eq!(verified.vote_b.round, 2);
        assert_eq!(verified.vote_a.block_hash, H256::from_low_u64_be(7));
        assert_eq!(verified.vote_b.block_hash, H256::from_low_u64_be(8));

        assert_eq!(
            verified.proof_key,
            double_voting_proof_hash(
                legacy_pbft_vote_hash(H256::from_low_u64_be(7), &sortition_a),
                legacy_pbft_vote_hash(H256::from_low_u64_be(8), &sortition_b),
            )
        );
    }

    #[test]
    fn rejects_legacy_double_voting_call_data_when_period_round_or_step_mismatch() {
        let signing_key = SigningKey::from_slice(&[0x22; 32]).unwrap();
        let sortition_a = encode_vrf_sortition(10, 2, 4, 0x33);
        let sortition_b = encode_vrf_sortition(11, 2, 4, 0x33);

        let (vote_a, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(11),
            &sortition_a,
            None,
            None,
        );
        let (vote_b, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(12),
            &sortition_b,
            None,
            None,
        );

        let calldata = commit_double_voting_proof_call_data(&vote_a, &vote_b);
        assert!(verify_legacy_double_voting_proof_call_data(&calldata).is_err());
    }

    #[test]
    fn rejects_legacy_double_voting_call_data_with_invalid_signature() {
        let signing_key = SigningKey::from_slice(&[0x33; 32]).unwrap();
        let sortition = encode_vrf_sortition(15, 3, 4, 0x44);

        let (vote_a, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(21),
            &sortition,
            None,
            None,
        );

        let mut invalid_signature = [0xFFu8; SIGNATURE_SIZE];
        invalid_signature[64] = 4;
        let (vote_b, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(22),
            &sortition,
            Some(invalid_signature),
            None,
        );

        let calldata = commit_double_voting_proof_call_data(&vote_a, &vote_b);
        assert!(verify_legacy_double_voting_proof_call_data(&calldata).is_err());
    }

    #[test]
    fn rejects_mixed_zero_next_vote_hashes_for_odd_steps() {
        let signing_key = SigningKey::from_slice(&[0x44; 32]).unwrap();
        let sortition = encode_vrf_sortition(9, 1, 5, 0x55);

        let (vote_a, _) = sign_vote(&signing_key, H256::zero(), &sortition, None, None);
        let (vote_b, _) = sign_vote(
            &signing_key,
            H256::from_low_u64_be(99),
            &sortition,
            None,
            None,
        );

        let calldata = commit_double_voting_proof_call_data(&vote_a, &vote_b);
        assert!(verify_legacy_double_voting_proof_call_data(&calldata).is_err());
    }

    #[test]
    fn plans_call_data_and_canonical_hash_for_matching_votes() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();

        let plan = planner.plan_double_voting_proof(input(2, 1));

        assert!(plan.should_submit);
        assert_eq!(plan.status, DoubleVotingProofPlanStatus::Planned);
        assert_eq!(plan.proof_hash, double_voting_proof_hash(hash(1), hash(2)));
        assert_eq!(plan.wallet_index, 0);
        assert_eq!(plan.nonce, U256::from(7));
        assert_eq!(
            &plan.call_data[..4],
            &function_selector(DOUBLE_VOTING_PROOF_FUNCTION)
        );
        assert_eq!(plan.contract_address, SLASHING_CONTRACT);
        assert_eq!(plan.value, U256::zero());
        assert_eq!(plan.gas_limit, DOUBLE_VOTING_GAS_LIMIT);
        assert_eq!(plan.call_data.len(), 4 + 2 * WORD_SIZE + 2 * 2 * WORD_SIZE);
    }

    #[test]
    fn fixture_canonical_proof_hash_matches_legacy_sorted_rlp_pair() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let expected = h256_hex("3adcdeea9dd9a4219614e50270f3aba4ab10f39f111bfb028dadeee274cdabd9");

        let forward = planner.plan_double_voting_proof(input(0x22, 0x11));
        let reverse = planner.plan_double_voting_proof(input(0x11, 0x22));

        assert_eq!(forward.proof_hash, expected);
        assert_eq!(reverse.proof_hash, expected);
    }

    #[test]
    fn fixture_call_data_matches_legacy_solidity_bytes_layout() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let proof = input(0x11, 0x22);

        let plan = planner.plan_double_voting_proof(proof);

        assert_eq!(
            plan.call_data,
            hex_bytes(concat!(
                "fac7c94a",
                "0000000000000000000000000000000000000000000000000000000000000040",
                "0000000000000000000000000000000000000000000000000000000000000080",
                "0000000000000000000000000000000000000000000000000000000000000002",
                "c101000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000002",
                "c102000000000000000000000000000000000000000000000000000000000000"
            ))
        );
    }

    #[test]
    fn rejects_disabled_reporting_or_mismatched_slot() {
        let disabled = SlashingProofPlanner::new(false, 11, 1000, 100).unwrap();
        assert_eq!(
            disabled.plan_double_voting_proof(input(1, 2)).status,
            DoubleVotingProofPlanStatus::Disabled
        );

        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let mut mismatched = input(1, 2);
        mismatched.vote_b_round = 3;
        assert_eq!(
            planner.plan_double_voting_proof(mismatched).status,
            DoubleVotingProofPlanStatus::MismatchedVoteCoordinates
        );
    }

    #[test]
    fn enforces_magnolia_vote_a_boundary_before_slot_validation() {
        let planner = SlashingProofPlanner::new(true, 10, 1000, 100).unwrap();

        let activation_plan = planner.plan_double_voting_proof(input(1, 2));
        assert_eq!(activation_plan.status, DoubleVotingProofPlanStatus::Planned);

        let mut before_activation = input(3, 4);
        before_activation.vote_a_period = 9;
        before_activation.vote_b_period = 8;
        assert_eq!(
            planner.plan_double_voting_proof(before_activation).status,
            DoubleVotingProofPlanStatus::BeforeMagnoliaActivation
        );

        let mut vote_b_before_activation = input(5, 6);
        vote_b_before_activation.vote_b_period = 9;
        assert_eq!(
            planner
                .plan_double_voting_proof(vote_b_before_activation)
                .status,
            DoubleVotingProofPlanStatus::MismatchedVoteCoordinates,
            "the legacy activation gate checks vote A only"
        );
        assert!(planner.submitted_proofs.is_empty());
        assert!(planner.submitted_order.is_empty());
    }

    #[test]
    fn supports_maximum_magnolia_activation_period_without_overflow() {
        let planner = SlashingProofPlanner::new(true, u64::MAX, 1000, 100).unwrap();
        assert_eq!(
            planner.plan_double_voting_proof(input(1, 2)).status,
            DoubleVotingProofPlanStatus::BeforeMagnoliaActivation
        );

        let mut at_activation = input(3, 4);
        at_activation.vote_a_period = u64::MAX;
        at_activation.vote_b_period = u64::MAX;
        assert_eq!(
            planner.plan_double_voting_proof(at_activation).status,
            DoubleVotingProofPlanStatus::Planned
        );
    }

    #[test]
    fn selects_first_funded_submitter() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let mut proof = input(1, 2);
        proof.submitters = vec![
            SlashingSubmitterFact {
                wallet_index: 0,
                nonce: U256::from(1),
                balance: U256::zero(),
            },
            SlashingSubmitterFact {
                wallet_index: 1,
                nonce: U256::from(9),
                balance: U256::from(5),
            },
        ];

        let plan = planner.plan_double_voting_proof(proof);

        assert!(plan.should_submit);
        assert_eq!(plan.wallet_index, 1);
        assert_eq!(plan.nonce, U256::from(9));
    }

    #[test]
    fn rejects_when_no_submitter_has_balance() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let mut proof = input(1, 2);
        proof.submitters[0].balance = U256::zero();

        let plan = planner.plan_double_voting_proof(proof);

        assert_eq!(plan.status, DoubleVotingProofPlanStatus::NoFundedSubmitter);
        assert!(!plan.should_submit);
    }

    #[test]
    fn marks_submitted_only_after_success_and_rejects_duplicates() {
        let mut planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let proof = input(1, 2);
        let plan = planner.plan_double_voting_proof(proof.clone());

        assert!(plan.should_submit);
        assert!(planner.mark_double_voting_proof_submission(plan.proof_hash));
        assert!(!planner.mark_submitted(plan.proof_hash));
        assert_eq!(
            planner.plan_double_voting_proof(proof).status,
            DoubleVotingProofPlanStatus::DuplicateProof
        );
        assert!(!planner.mark_double_voting_proof_submission(plan.proof_hash));
    }

    #[test]
    fn submitted_cache_evicts_oldest_entries_by_delete_step() {
        let mut planner = SlashingProofPlanner::new(true, 0, 3, 2).unwrap();
        let hashes = [hash(1), hash(2), hash(3), hash(4)];

        for proof_hash in hashes {
            assert!(planner.mark_double_voting_proof_submission(proof_hash));
        }

        assert!(!planner.submitted_proofs.contains(&hash(1)));
        assert!(!planner.submitted_proofs.contains(&hash(2)));
        assert!(planner.submitted_proofs.contains(&hash(3)));
        assert!(planner.submitted_proofs.contains(&hash(4)));
    }

    #[test]
    fn mirrors_legacy_abi_padding_for_full_words() {
        let planner = SlashingProofPlanner::new(true, 0, 1000, 100).unwrap();
        let mut proof = input(1, 2);
        proof.vote_a_rlp = vec![0x55; WORD_SIZE];
        proof.vote_b_rlp = vec![0xaa; WORD_SIZE];

        let plan = planner.plan_double_voting_proof(proof);

        assert!(plan.should_submit);
        assert_eq!(
            plan.call_data,
            hex_bytes(concat!(
                "fac7c94a",
                "0000000000000000000000000000000000000000000000000000000000000040",
                "00000000000000000000000000000000000000000000000000000000000000a0",
                "0000000000000000000000000000000000000000000000000000000000000020",
                "5555555555555555555555555555555555555555555555555555555555555555",
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0000000000000000000000000000000000000000000000000000000000000020",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "0000000000000000000000000000000000000000000000000000000000000000"
            ))
        );
        assert_eq!(
            plan.call_data.len(),
            4 + 2 * WORD_SIZE + (WORD_SIZE + WORD_SIZE + WORD_SIZE) * 2
        );
    }
}
