//! Deterministic slashing-proof planning for Rust-backed consensus shims.
//!
//! The planner owns the consensus-facing decision for double-vote proof
//! submission: it validates that two votes describe the same PBFT slot,
//! canonicalizes their hashes, builds the slashing contract calldata, and
//! tracks proofs already submitted by this node. It deliberately does not own
//! wallet/account lookup, gas pricing, transaction signing, or transaction pool
//! insertion; those remain live-node responsibilities on the C++ side until the
//! surrounding transaction pipeline is moved to Rust.

use anyhow::{Result, ensure};
use ethereum_types::{H256, U256};
use rlp::RlpStream;
use std::collections::{HashSet, VecDeque};
use tiny_keccak::{Hasher, Keccak};

const DOUBLE_VOTING_PROOF_FUNCTION: &str = "commitDoubleVotingProof(bytes,bytes)";
const DOUBLE_VOTING_GAS_LIMIT: u64 = 100_000;
const WORD_SIZE: usize = 32;
const SLASHING_CONTRACT: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xEE,
];

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
    MismatchedVoteCoordinates,
    DuplicateProof,
    NoFundedSubmitter,
}

/// Rust decision returned for one double-vote proof attempt.
///
/// `should_submit` is false when reporting is disabled, the votes are for
/// different PBFT slots, or the proof hash was already submitted. When it is
/// true, `proof_hash` is the canonical duplicate-cache key and `call_data` is
/// the byte-for-byte slashing contract call payload C++ should place into the
/// transaction input.
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

/// Double-voting proof planner with a bounded submitted-proof cache.
///
/// The cache mirrors the legacy `ExpirationCache` shape: insertion is FIFO and
/// when the size exceeds `max_submitted_proofs`, `delete_step` oldest entries
/// are evicted. Proofs are marked submitted only after C++ successfully inserts
/// the generated transaction, matching the legacy side-effect ordering.
pub struct SlashingProofPlanner {
    report_malicious_behaviour: bool,
    submitted_proofs: HashSet<H256>,
    submitted_order: VecDeque<H256>,
    max_submitted_proofs: usize,
    delete_step: usize,
}

impl SlashingProofPlanner {
    /// Creates an empty planner for local slashing proof submission.
    ///
    /// `report_malicious_behaviour` disables proof generation when false.
    /// Cache limits must be non-zero so duplicate detection has the same
    /// bounded behavior as the legacy manager.
    pub fn new(
        report_malicious_behaviour: bool,
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
            submitted_proofs: HashSet::new(),
            submitted_order: VecDeque::new(),
            max_submitted_proofs,
            delete_step,
        })
    }

    /// Builds a double-voting proof transaction plan when the proof is new.
    ///
    /// The method does not mutate the submitted-proof cache; callers must invoke
    /// [`SlashingProofPlanner::mark_submitted`] only after the transaction was
    /// accepted for insertion.
    pub fn plan_double_voting_proof(&self, input: DoubleVotingProofInput) -> DoubleVotingProofPlan {
        if !self.report_malicious_behaviour {
            return DoubleVotingProofPlan::not_submitted(DoubleVotingProofPlanStatus::Disabled);
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

    #[test]
    fn plans_call_data_and_canonical_hash_for_matching_votes() {
        let planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();

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
    fn rejects_disabled_reporting_or_mismatched_slot() {
        let disabled = SlashingProofPlanner::new(false, 1000, 100).unwrap();
        assert_eq!(
            disabled.plan_double_voting_proof(input(1, 2)).status,
            DoubleVotingProofPlanStatus::Disabled
        );

        let planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();
        let mut mismatched = input(1, 2);
        mismatched.vote_b_round = 3;
        assert_eq!(
            planner.plan_double_voting_proof(mismatched).status,
            DoubleVotingProofPlanStatus::MismatchedVoteCoordinates
        );
    }

    #[test]
    fn selects_first_funded_submitter() {
        let planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();
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
        let planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();
        let mut proof = input(1, 2);
        proof.submitters[0].balance = U256::zero();

        let plan = planner.plan_double_voting_proof(proof);

        assert_eq!(plan.status, DoubleVotingProofPlanStatus::NoFundedSubmitter);
        assert!(!plan.should_submit);
    }

    #[test]
    fn marks_submitted_only_after_success_and_rejects_duplicates() {
        let mut planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();
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
        let mut planner = SlashingProofPlanner::new(true, 3, 2).unwrap();
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
        let planner = SlashingProofPlanner::new(true, 1000, 100).unwrap();
        let mut proof = input(1, 2);
        proof.vote_a_rlp = vec![0x55; WORD_SIZE];
        proof.vote_b_rlp = vec![0xaa; WORD_SIZE];

        let plan = planner.plan_double_voting_proof(proof);

        assert!(plan.should_submit);
        assert_eq!(
            plan.call_data.len(),
            4 + 2 * WORD_SIZE + (WORD_SIZE + WORD_SIZE + WORD_SIZE) * 2
        );
    }
}
