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
use crate::ffi::BridgePbftService;
use anyhow::Result;
use ethereum_types::{H256, U256};
use rustaxa_consensus::slashing::{
    DoubleVotingProofPlanStatus, SlashingSubmitterFact as ConsensusSubmitterFact,
};

impl BridgePbftService {
    /// Builds one deterministic slashing transaction plan from C++ vote payloads.
    ///
    /// The method locks only service-owned slashing state. Malformed evidence is
    /// represented by the existing typed plan status rather than mutating
    /// duplicate protection.
    pub fn slashing_plan_double_voting_proof(
        &self,
        input: DoubleVotingProofInput,
    ) -> Result<DoubleVotingProofPlan> {
        Ok(self
            .slashing()
            .plan_double_voting_proof(input.into())?
            .into())
    }

    /// Applies a typed transaction executor report to Rust duplicate protection.
    ///
    /// Accepted submissions enter the bounded duplicate cache; rejected
    /// submissions remain retryable. Reports lock only slashing state and never
    /// retain a Rust guard across C++ transaction execution.
    pub fn slashing_report_double_voting_proof_submission(
        &self,
        report: DoubleVotingProofSubmissionReport,
    ) -> Result<bool> {
        Ok(self
            .slashing()
            .report_double_voting_proof_submission(
                H256::from(report.proof_hash),
                report.transaction_inserted,
            )?
            .submitted)
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
        DoubleVotingProofPlanStatus::BeforeMagnoliaActivation => 5,
    }
}

fn u256_to_bytes(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::rustaxa_ffi::PbftServiceConfig;
    use crate::storage::create_storage;
    use rustaxa_consensus::DoubleVotingProofPlanStatus;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn service(
        report_malicious_behaviour: bool,
        magnolia_activation_period: u64,
    ) -> (Box<BridgePbftService>, PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustaxa_bridge_slashing_{nonce}"));
        let storage = create_storage(path.to_str().expect("UTF-8 temp path"))
            .expect("storage should initialize");
        let service = crate::pbft_manager::create_pbft_service_from_storage(
            &storage,
            PbftServiceConfig {
                genesis_lambda_ms: 100,
                cacti_lambda_max_ms: 100,
                cacti_lambda_default_ms: 100,
                cacti_block: u64::MAX,
                max_exponential_lambda_ms: 60_000,
                max_steps: 13,
                deadline_ms: 400,
                polling_interval_ms: 100,
                report_malicious_behaviour,
                magnolia_activation_period,
            },
        )
        .expect("PBFT service should initialize");
        (service, path)
    }

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
        let (service, path) = service(true, 0);
        let input = proof_input(2, 1, vec![submitter(0, true, 9)]);

        let plan = service.slashing_plan_double_voting_proof(input).unwrap();

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
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn bridge_propagates_magnolia_activation_period() {
        let (before, before_path) = service(true, 101);
        let plan = before
            .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
            .unwrap();
        assert_eq!(
            plan.status,
            double_voting_proof_plan_status_code(
                DoubleVotingProofPlanStatus::BeforeMagnoliaActivation
            )
        );
        assert!(!plan.should_submit);

        let (at_activation, activation_path) = service(true, 100);
        assert_eq!(
            at_activation
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)],))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Planned)
        );
        drop(before);
        drop(at_activation);
        std::fs::remove_dir_all(before_path).unwrap();
        std::fs::remove_dir_all(activation_path).unwrap();
    }

    #[test]
    fn bridge_plan_status_codes_remain_stable() {
        assert_eq!(
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Planned),
            0
        );
        assert_eq!(
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Disabled),
            1
        );
        assert_eq!(
            double_voting_proof_plan_status_code(
                DoubleVotingProofPlanStatus::MismatchedVoteCoordinates
            ),
            2
        );
        assert_eq!(
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::DuplicateProof),
            3
        );
        assert_eq!(
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::NoFundedSubmitter),
            4
        );
        assert_eq!(
            double_voting_proof_plan_status_code(
                DoubleVotingProofPlanStatus::BeforeMagnoliaActivation
            ),
            5
        );
    }

    #[test]
    fn bridge_output_matches_slashing_fixture_bytes() {
        let (service, path) = service(true, 0);

        let plan = service
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
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn bridge_reports_submission_executor_outcome() {
        let (service, path) = service(true, 0);
        let plan = service
            .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
            .unwrap();

        let rejected = service
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: false,
            })
            .unwrap();
        assert!(!rejected);
        assert_eq!(
            service
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::Planned),
        );

        let accepted = service
            .slashing_report_double_voting_proof_submission(DoubleVotingProofSubmissionReport {
                proof_hash: plan.proof_hash,
                transaction_inserted: true,
            })
            .unwrap();
        assert!(accepted);
        assert_eq!(
            service
                .slashing_plan_double_voting_proof(proof_input(1, 2, vec![submitter(0, true, 1)]))
                .unwrap()
                .status,
            double_voting_proof_plan_status_code(DoubleVotingProofPlanStatus::DuplicateProof),
        );
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
