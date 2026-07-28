//! Leaf CXX conversions retained for the native DAG runtime.
//!
//! Native [`rustaxa_consensus`] services own DAG state, storage-backed queries,
//! proposer sessions, verification, and finalization. This module retains only
//! the worker-loop command and legacy VDF byte boundaries that are still
//! consumed by C++ executors.

use crate::ffi::rustaxa_ffi::{DagHash, DagProposerWorkerCommand, DagProposerWorkerCommandInput};
use ethereum_types::H256;
use rustaxa_consensus::dag::{
    construct_dag_vdf_message, plan_dag_proposer_worker_command,
    DagProposerWorkerCommandInput as DomainDagProposerWorkerCommandInput,
};
use rustaxa_consensus::sortition::{SortitionParams, VdfParams, VrfParams};

pub(crate) const DAG_PROPOSER_SESSION_ACTION_NONE: u8 = 0;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_PACK_TRANSACTIONS: u8 = 1;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_START_VDF: u8 = 2;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_CANCEL_VDF: u8 = 3;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_STALE_PROOF_SLEEP: u8 = 4;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_SIGN_BLOCK: u8 = 5;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_ADD_BLOCK: u8 = 6;
pub(crate) const DAG_PROPOSER_SESSION_ACTION_COLLECT_EXTERNAL_PROPOSAL_FACTS: u8 = 7;

/// Converts live C++ worker facts into one native proposer-loop command.
///
/// C++ retains the thread, network-pressure observations, and timer. The
/// native planner owns whether to attempt a proposal and whether the executor
/// should sleep after the tick. This function is deterministic and performs no
/// I/O; every input combination returns one complete command.
pub fn dag_plan_proposer_worker_command(
    input: DagProposerWorkerCommandInput,
) -> DagProposerWorkerCommand {
    let command = plan_dag_proposer_worker_command(DomainDagProposerWorkerCommandInput {
        pbft_syncing: input.pbft_syncing,
        packet_queue_over_limit: input.packet_queue_over_limit,
        has_attempt_result: input.has_attempt_result,
        attempt_returned_proposed: input.attempt_returned_proposed,
    });
    DagProposerWorkerCommand {
        attempt_proposal: command.attempt_proposal,
        sleep_after_tick: command.sleep_after_tick,
        sleep_ms: command.sleep_ms,
        reason_code: command.reason_code,
    }
}

/// Converts a pivot and ordered transaction hashes into legacy DAG VDF bytes.
///
/// The output is the canonical concatenation of RLP items expected by the C++
/// VDF executor: pivot first, followed by transaction hashes in supplied order.
/// Empty transaction input emits only the pivot item.
pub fn dag_vdf_message(pivot: &[u8; 32], transaction_hashes: Vec<DagHash>) -> Vec<u8> {
    let hashes = transaction_hashes
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    construct_dag_vdf_message(H256::from(*pivot), &hashes)
}

/// Returns zeroed legacy sortition parameters for terminal CXX responses.
pub(crate) fn empty_sortition_params() -> SortitionParams {
    SortitionParams {
        vrf: VrfParams { threshold_upper: 0 },
        vdf: VdfParams {
            difficulty_min: 0,
            difficulty_max: 0,
            difficulty_stale: 0,
            lambda_bound: 0,
        },
    }
}

/// Converts native sortition parameters into the retained CXX carrier.
pub(crate) fn legacy_sortition_params(
    params: SortitionParams,
) -> crate::ffi::rustaxa_ffi::LegacySortitionParams {
    crate::ffi::rustaxa_ffi::LegacySortitionParams {
        vrf_threshold_upper: params.vrf.threshold_upper,
        vdf_difficulty_min: params.vdf.difficulty_min,
        vdf_difficulty_max: params.vdf.difficulty_max,
        vdf_difficulty_stale: params.vdf.difficulty_stale,
        vdf_lambda_bound: params.vdf.lambda_bound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rlp::RlpStream;

    #[test]
    fn proposer_worker_command_converts_native_attempt_and_backoff_plans() {
        let attempt = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(attempt.attempt_proposal);
        assert!(!attempt.sleep_after_tick);

        let throttle = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: true,
            packet_queue_over_limit: false,
            has_attempt_result: false,
            attempt_returned_proposed: false,
        });
        assert!(!throttle.attempt_proposal);
        assert!(throttle.sleep_after_tick);
        assert_eq!(throttle.sleep_ms, 100);

        let no_block = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: false,
        });
        assert!(!no_block.attempt_proposal);
        assert!(no_block.sleep_after_tick);
        assert_eq!(no_block.sleep_ms, 100);

        let proposed = dag_plan_proposer_worker_command(DagProposerWorkerCommandInput {
            pbft_syncing: false,
            packet_queue_over_limit: false,
            has_attempt_result: true,
            attempt_returned_proposed: true,
        });
        assert!(!proposed.attempt_proposal);
        assert!(!proposed.sleep_after_tick);
    }

    #[test]
    fn vdf_message_conversion_preserves_legacy_rlp_order() {
        let pivot = [0x11_u8; 32];
        let tx_hashes = vec![
            DagHash {
                hash: [0x22_u8; 32],
            },
            DagHash {
                hash: [0x33_u8; 32],
            },
        ];

        let mut expected = RlpStream::new();
        expected.append(&H256::from(pivot));
        expected.append(&H256::from(tx_hashes[0].hash));
        expected.append(&H256::from(tx_hashes[1].hash));

        assert_eq!(dag_vdf_message(&pivot, tx_hashes), expected.out().to_vec());
    }
}
