//! CXX bridge wrappers for deterministic slashing proof planning.
//!
//! The bridge takes one normalized double-vote evidence payload and submitter
//! wallet candidate facts from the C++ executor edge, then returns a
//! deterministic plan that describes whether to construct a slashing
//! transaction, the target contract+gas limit, and ABI call payload.
use crate::ffi::rustaxa_ffi::{
    DoubleVotingProofInput, DoubleVotingProofPlan, DoubleVotingProofSubmissionReport,
    SlashingSubmitterFact,
};
use crate::ffi::BridgeSlashingProofPlanner;
use anyhow::{anyhow, Result};
use ethereum_types::{H256, U256};
use rustaxa_consensus::slashing::{
    DoubleVotingProofPlanStatus, SlashingProofPlanner,
    SlashingSubmitterFact as ConsensusSubmitterFact,
};

const SLASHING_PROOF_CACHE_MAX_SIZE: usize = 1000;
const SLASHING_PROOF_CACHE_DELETE_STEP: usize = 100;

/// Creates a deterministic slashing planner with legacy-compatible cache limits.
pub fn create_slashing_proof_planner(
    report_malicious_behaviour: bool,
) -> Result<Box<BridgeSlashingProofPlanner>> {
    Ok(Box::new(BridgeSlashingProofPlanner(std::sync::Mutex::new(
        SlashingProofPlanner::new(
            report_malicious_behaviour,
            SLASHING_PROOF_CACHE_MAX_SIZE,
            SLASHING_PROOF_CACHE_DELETE_STEP,
        )?,
    ))))
}

impl BridgeSlashingProofPlanner {
    /// Builds one deterministic slashing transaction plan from C++ vote payloads.
    pub fn slashing_plan_double_voting_proof(
        &self,
        input: DoubleVotingProofInput,
    ) -> Result<DoubleVotingProofPlan> {
        Ok(self.lock()?.plan_double_voting_proof(input.into()).into())
    }

    /// Applies a typed transaction executor report to Rust duplicate protection.
    pub fn slashing_report_double_voting_proof_submission(
        &self,
        report: DoubleVotingProofSubmissionReport,
    ) -> Result<bool> {
        Ok(self
            .lock()?
            .report_double_voting_proof_submission(
                H256::from(report.proof_hash),
                report.transaction_inserted,
            )
            .submitted)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SlashingProofPlanner>> {
        self.0
            .lock()
            .map_err(|_| anyhow!("slashing proof planner mutex poisoned"))
    }
}

impl From<SlashingSubmitterFact> for ConsensusSubmitterFact {
    fn from(fact: SlashingSubmitterFact) -> Self {
        Self {
            wallet_index: fact.wallet_index,
            nonce: U256::from_big_endian(&fact.nonce),
            balance: U256::from_big_endian(&fact.balance),
        }
    }
}

impl From<DoubleVotingProofInput> for rustaxa_consensus::DoubleVotingProofInput {
    fn from(input: DoubleVotingProofInput) -> Self {
        Self {
            vote_a_hash: H256::from(input.vote_a_hash),
            vote_b_hash: H256::from(input.vote_b_hash),
            vote_a_period: input.period,
            vote_b_period: input.period,
            vote_a_round: input.round,
            vote_b_round: input.round,
            vote_a_step: input.step,
            vote_b_step: input.step,
            vote_a_rlp: input.vote_a_rlp,
            vote_b_rlp: input.vote_b_rlp,
            submitters: input.submitters.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<rustaxa_consensus::DoubleVotingProofPlan> for DoubleVotingProofPlan {
    fn from(plan: rustaxa_consensus::DoubleVotingProofPlan) -> Self {
        Self {
            status: double_voting_proof_plan_status_code(plan.status),
            should_submit: plan.should_submit,
            proof_hash: plan.proof_hash.0,
            contract_address: plan.contract_address,
            value: u256_to_bytes(plan.value),
            gas_limit: plan.gas_limit,
            call_data: plan.call_data,
            wallet_index: plan.wallet_index,
            nonce: u256_to_bytes(plan.nonce),
        }
    }
}

fn double_voting_proof_plan_status_code(status: DoubleVotingProofPlanStatus) -> u8 {
    match status {
        DoubleVotingProofPlanStatus::Planned => 0,
        DoubleVotingProofPlanStatus::Disabled => 1,
        DoubleVotingProofPlanStatus::MismatchedVoteCoordinates => 2,
        DoubleVotingProofPlanStatus::DuplicateProof => 3,
        DoubleVotingProofPlanStatus::NoFundedSubmitter => 4,
    }
}

fn u256_to_bytes(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustaxa_consensus::DoubleVotingProofPlanStatus;

    fn h256(byte: u8) -> H256 {
        H256([byte; 32])
    }

    fn submitter(wallet_index: usize, has_balance: bool, nonce: u8) -> SlashingSubmitterFact {
        SlashingSubmitterFact {
            wallet_index,
            nonce: {
                let mut bytes = [0u8; 32];
                bytes[31] = nonce;
                bytes
            },
            balance: if has_balance {
                let mut bytes = [0u8; 32];
                bytes[31] = 1;
                bytes
            } else {
                [0u8; 32]
            },
        }
    }

    fn proof_input(a: u8, b: u8, submitters: Vec<SlashingSubmitterFact>) -> DoubleVotingProofInput {
        DoubleVotingProofInput {
            vote_a_hash: h256(a).0,
            vote_b_hash: h256(b).0,
            period: 100,
            round: 2,
            step: 3,
            vote_a_rlp: vec![0xc1, 0x01],
            vote_b_rlp: vec![0xc1, 0x02],
            submitters,
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

    fn h256_hex(value: &str) -> [u8; 32] {
        hex_bytes(value).try_into().unwrap()
    }

    #[test]
    fn bridges_planner_plan_output() {
        let planner = create_slashing_proof_planner(true).unwrap();
        let input = proof_input(2, 1, vec![submitter(0, true, 9)]);

        let plan = planner.slashing_plan_double_voting_proof(input).unwrap();

        assert_eq!(
            plan.status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Planned)
        );
        assert!(plan.should_submit);
        assert_eq!(plan.wallet_index, 0);
        assert_eq!(plan.nonce, {
            let mut bytes = [0u8; 32];
            bytes[31] = 9;
            bytes
        });
        assert!(!plan.call_data.is_empty());
        assert_eq!(plan.value, [0u8; 32]);
        assert_eq!(plan.gas_limit, 100_000);
    }

    #[test]
    fn bridge_output_matches_slashing_fixture_bytes() {
        let planner = create_slashing_proof_planner(true).unwrap();

        let plan = planner
            .slashing_plan_double_voting_proof(proof_input(0x22, 0x11, vec![submitter(3, true, 9)]))
            .unwrap();

        assert_eq!(
            plan.proof_hash,
            h256_hex("3adcdeea9dd9a4219614e50270f3aba4ab10f39f111bfb028dadeee274cdabd9")
        );
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
        assert_eq!(plan.wallet_index, 3);
        assert_eq!(plan.nonce, {
            let mut bytes = [0u8; 32];
            bytes[31] = 9;
            bytes
        });
        assert_eq!(plan.contract_address[19], 0xee);
        assert_eq!(plan.value, [0u8; 32]);
    }

    #[test]
    fn bridge_report_marks_submission_once() {
        let planner = create_slashing_proof_planner(true).unwrap();
        let plan = planner
            .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
            .unwrap();

        let submitted = planner
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: true,
            })
            .unwrap();
        assert!(submitted);
        assert_eq!(
            planner
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::DuplicateProof),
        );

        let duplicate = planner
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: true,
            })
            .unwrap();
        assert!(!duplicate);
    }

    #[test]
    fn bridge_reports_submission_executor_outcome() {
        let planner = create_slashing_proof_planner(true).unwrap();
        let plan = planner
            .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
            .unwrap();

        let rejected = planner
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: false,
            })
            .unwrap();
        assert!(!rejected);
        assert_eq!(
            planner
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Planned),
        );

        let accepted = planner
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: true,
            })
            .unwrap();
        assert!(accepted);
        assert_eq!(
            planner
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::DuplicateProof),
        );
    }
}
