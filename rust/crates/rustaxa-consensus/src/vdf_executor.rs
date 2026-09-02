//! Rust-owned asynchronous DAG VDF proof execution.
//!
//! Consensus retains scheduling and exact proposer cursor state while this
//! executor owns only CPU-heavy proof workers, job identities, cancellation,
//! and canonical legacy payload construction. Private keys never enter this
//! module: callers provide the VRF proof already returned by the signing port.

use anyhow::{Context, Result, ensure};
use rustaxa_vdf::prover::CancellationToken;
use rustaxa_vdf::vdf_sortition::{
    VdfSortitionProofOutcome, encode_vdf_sortition_payload, prove_vdf_sortition,
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};

/// Complete input for one native DAG VDF worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeVdfRequest {
    /// Previously verified legacy VRF proof embedded in the final payload.
    pub vrf_proof: Vec<u8>,
    /// Canonical message used as the Wesolowski puzzle input.
    pub vdf_message: Vec<u8>,
    /// Difficulty selected by the native proposer cursor.
    pub difficulty: u16,
    /// Nonzero hash-to-prime lambda bound selected for the proposal period.
    pub lambda_bound: u16,
}

/// Nonblocking observation of one native VDF job.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum NativeVdfPollResult {
    /// The worker is still computing and remains registered.
    Pending,
    /// The worker completed and returned canonical legacy sortition RLP.
    Completed(Vec<u8>),
    /// The worker observed cancellation before producing a payload.
    Cancelled,
}

struct NativeVdfJob {
    cancellation: CancellationToken,
    worker: JoinHandle<Result<VdfSortitionProofOutcome>>,
}

struct NativeVdfJobs {
    next_job_id: u64,
    jobs: HashMap<u64, NativeVdfJob>,
}

/// Application-owned VDF worker registry.
///
/// Job IDs are nonzero and unique for the lifetime of the executor. Polling is
/// nonblocking; completion removes and joins the exact worker. Cancellation
/// also removes, signals, and joins the worker before returning. Dropping the
/// executor cancels and joins every remaining job so no proof worker can
/// outlive the consensus application root.
pub(crate) struct NativeVdfExecutor {
    jobs: Mutex<NativeVdfJobs>,
}

impl NativeVdfExecutor {
    /// Creates an empty executor whose first issued job identity is one.
    pub(crate) fn new() -> Self {
        Self {
            jobs: Mutex::new(NativeVdfJobs {
                next_job_id: 1,
                jobs: HashMap::new(),
            }),
        }
    }

    /// Starts one proof worker and returns its nonzero native identity.
    pub(crate) fn start(&self, request: NativeVdfRequest) -> Result<u64> {
        ensure!(request.lambda_bound != 0, "CONSENSUS_DAG_VDF_LAMBDA_ZERO");
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("CONSENSUS_DAG_VDF_JOB_REGISTRY_POISONED"))?;
        let job_id = jobs.next_job_id;
        jobs.next_job_id = jobs
            .next_job_id
            .checked_add(1)
            .context("CONSENSUS_DAG_VDF_JOB_ID_EXHAUSTED")?;
        ensure!(
            job_id != 0 && !jobs.jobs.contains_key(&job_id),
            "CONSENSUS_DAG_VDF_JOB_ID_COLLISION"
        );
        let worker = thread::Builder::new()
            .name(format!("rustaxa-vdf-{job_id}"))
            .spawn(move || {
                prove_vdf_sortition(
                    &request.vrf_proof,
                    &request.vdf_message,
                    request.difficulty,
                    request.lambda_bound,
                    &worker_cancellation,
                )
            })
            .context("CONSENSUS_DAG_VDF_WORKER_SPAWN_FAILED")?;
        jobs.jobs.insert(
            job_id,
            NativeVdfJob {
                cancellation,
                worker,
            },
        );
        Ok(job_id)
    }

    /// Polls one job without blocking while it is active.
    ///
    /// A terminal observation removes and joins the worker. Unknown or already
    /// consumed identities are errors, preventing one proposer cursor from
    /// consuming another cursor's result.
    pub(crate) fn poll(&self, job_id: u64) -> Result<NativeVdfPollResult> {
        let job = {
            let mut jobs = self
                .jobs
                .lock()
                .map_err(|_| anyhow::anyhow!("CONSENSUS_DAG_VDF_JOB_REGISTRY_POISONED"))?;
            let job = jobs
                .jobs
                .get(&job_id)
                .context("CONSENSUS_DAG_VDF_JOB_NOT_FOUND")?;
            if !job.worker.is_finished() {
                return Ok(NativeVdfPollResult::Pending);
            }
            jobs.jobs
                .remove(&job_id)
                .context("CONSENSUS_DAG_VDF_JOB_NOT_FOUND")?
        };
        match join_worker(job)? {
            VdfSortitionProofOutcome::Completed(payload) => Ok(NativeVdfPollResult::Completed(
                encode_vdf_sortition_payload(&payload),
            )),
            VdfSortitionProofOutcome::Cancelled => Ok(NativeVdfPollResult::Cancelled),
        }
    }

    /// Cancels and joins one exact active worker.
    pub(crate) fn cancel(&self, job_id: u64) -> Result<()> {
        let job = self
            .jobs
            .lock()
            .map_err(|_| anyhow::anyhow!("CONSENSUS_DAG_VDF_JOB_REGISTRY_POISONED"))?
            .jobs
            .remove(&job_id)
            .context("CONSENSUS_DAG_VDF_JOB_NOT_FOUND")?;
        job.cancellation.cancel();
        let _ = join_worker(job)?;
        Ok(())
    }
}

impl Drop for NativeVdfExecutor {
    fn drop(&mut self) {
        let jobs = self
            .jobs
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (_, job) in jobs.jobs.drain() {
            job.cancellation.cancel();
            let _ = join_worker(job);
        }
    }
}

fn join_worker(job: NativeVdfJob) -> Result<VdfSortitionProofOutcome> {
    job.worker
        .join()
        .map_err(|_| anyhow::anyhow!("CONSENSUS_DAG_VDF_WORKER_PANICKED"))?
        .context("CONSENSUS_DAG_VDF_PROOF_FAILED")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn request(difficulty: u16) -> NativeVdfRequest {
        NativeVdfRequest {
            vrf_proof: vec![0x11; 80],
            vdf_message: vec![0x22, 0x33],
            difficulty,
            lambda_bound: 64,
        }
    }

    #[test]
    fn native_executor_completes_and_consumes_canonical_payload() {
        let executor = NativeVdfExecutor::new();
        let job_id = executor.start(request(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let payload = loop {
            match executor.poll(job_id).unwrap() {
                NativeVdfPollResult::Pending => {
                    assert!(Instant::now() < deadline, "native VDF worker timed out");
                    thread::yield_now();
                }
                NativeVdfPollResult::Completed(payload) => break payload,
                NativeVdfPollResult::Cancelled => panic!("completed job reported cancellation"),
            }
        };
        let decoded = rustaxa_vdf::vdf_sortition::decode_vdf_sortition_payload(&payload).unwrap();
        assert_eq!(decoded.vrf_proof, [0x11; 80]);
        assert_eq!(decoded.difficulty, 5);
        assert!(!decoded.vdf_solution_proof.is_empty());
        assert!(!decoded.vdf_solution_output.is_empty());
        assert!(executor.poll(job_id).is_err());
    }

    #[test]
    fn native_executor_cancels_and_consumes_exact_job() {
        let executor = NativeVdfExecutor::new();
        let job_id = executor.start(request(18)).unwrap();
        executor.cancel(job_id).unwrap();
        assert!(executor.poll(job_id).is_err());
        assert!(executor.cancel(job_id).is_err());
    }

    #[test]
    fn native_executor_rejects_invalid_request_before_spawning() {
        let executor = NativeVdfExecutor::new();
        let mut invalid = request(1);
        invalid.lambda_bound = 0;
        assert!(executor.start(invalid).is_err());

        let mut invalid = request(1);
        invalid.vrf_proof.pop();
        let job_id = executor.start(invalid).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match executor.poll(job_id) {
                Ok(NativeVdfPollResult::Pending) => {
                    assert!(Instant::now() < deadline, "invalid VDF worker timed out");
                    thread::yield_now();
                }
                Err(error) => {
                    assert!(error.to_string().contains("CONSENSUS_DAG_VDF_PROOF_FAILED"));
                    break;
                }
                other => panic!("invalid proof unexpectedly completed: {other:?}"),
            }
        }
    }
}
