//! Bounded admission for exact PBFT candidates. Production owner enrollment,
//! full route coverage and protected finalization consumption remain separate.

use std::sync::Arc;

use anyhow::{Result, ensure};
use ethereum_types::H256;
use rustaxa_types::FinalChainBlockNumber;

use crate::consensus_application_runtime::ConsensusExecutionPort;
use crate::pbft_execution_validation::{
    PbftExecutionHardError, PbftExecutionRequest, PbftExecutionSchedule, PbftExecutionUnavailable,
    PbftExecutionValidation, ValidatedPbftExecution,
};

/// Trusted owner leaf. Implementations must acquire exact inputs and authenticate
/// all context facts from one owner epoch. Caller facts alone are insufficient.
pub trait PbftExecutionContextSource {
    fn acquire_pbft_execution_request(
        &self,
        _period: FinalChainBlockNumber,
        _block_rlp: &[u8],
    ) -> Result<PbftExecutionRequestAcquisition> {
        Ok(PbftExecutionRequestAcquisition::Unavailable(
            PbftExecutionUnavailable::ExecutorUnavailable,
        ))
    }

    /// Rechecks all context bindings after the detached execution call.
    fn recheck_pbft_execution_context(
        &self,
        _request: &PbftExecutionRequest,
    ) -> Result<Option<PbftExecutionUnavailable>> {
        Ok(Some(PbftExecutionUnavailable::ExecutorUnavailable))
    }
}

#[derive(Clone, Debug)]
pub enum PbftExecutionRequestAcquisition {
    Ready(Arc<PbftExecutionRequest>),
    Unavailable(PbftExecutionUnavailable),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PbftExecutionRefusal {
    Invalid {
        transaction_position: u64,
        native_sequence: Option<u64>,
        cause: PbftExecutionHardError,
    },
    Unavailable(PbftExecutionUnavailable),
}

/// No public constructor or public variant can grant historical bypass.
#[derive(Clone, Debug)]
pub struct PbftExecutionAdmission {
    issuer: Arc<()>,
    schedule_digest: [u8; 32],
    period: FinalChainBlockNumber,
    block_rlp: Vec<u8>,
    inner: AdmissionKind,
}

#[derive(Clone, Debug)]
enum AdmissionKind {
    Historical,
    Validated(ValidatedPbftExecution),
}

impl PbftExecutionAdmission {
    /// Retains the exact snapshot through the vote and certificate stages.
    pub fn request(&self) -> Option<&Arc<PbftExecutionRequest>> {
        match &self.inner {
            AdmissionKind::Historical => None,
            AdmissionKind::Validated(validated) => Some(validated.request()),
        }
    }
}

/// Immutable schedule authority for a bounded owner composition.
/// This type does not enroll an application store or activate a network.
pub trait PbftExecutionOwner: PbftExecutionContextSource + ConsensusExecutionPort {}
impl<T: PbftExecutionContextSource + ConsensusExecutionPort> PbftExecutionOwner for T {}

pub struct PbftExecutionGuard {
    issuer: Arc<()>,
    schedule: PbftExecutionSchedule,
    source: Arc<dyn PbftExecutionOwner + Send + Sync>,
}

impl PbftExecutionGuard {
    /// Isolated use only. The same trusted owner supplies schedule enrollment,
    /// exact context acquisition and execution. This does not read a live store.
    pub fn for_offline_owner(
        schedule: PbftExecutionSchedule,
        enrolled: Option<[u8; 32]>,
        source: Arc<dyn PbftExecutionOwner + Send + Sync>,
    ) -> Result<Self> {
        schedule.verify_enrollment(enrolled)?;
        Ok(Self {
            issuer: Arc::new(()),
            schedule,
            source,
        })
    }

    pub fn requires_validation(&self, period: FinalChainBlockNumber) -> bool {
        self.schedule.requires_validation(period)
    }

    pub fn require(
        &self,
        period: FinalChainBlockNumber,
        block_rlp: &[u8],
    ) -> Result<std::result::Result<PbftExecutionAdmission, PbftExecutionRefusal>> {
        let admission = |inner| PbftExecutionAdmission {
            issuer: Arc::clone(&self.issuer),
            schedule_digest: self.schedule.digest(),
            period,
            block_rlp: block_rlp.to_vec(),
            inner,
        };
        if !self.schedule.requires_validation(period) {
            return Ok(Ok(admission(AdmissionKind::Historical)));
        }
        let request = match self
            .source
            .acquire_pbft_execution_request(period, block_rlp)?
        {
            PbftExecutionRequestAcquisition::Ready(request) => request,
            PbftExecutionRequestAcquisition::Unavailable(reason) => {
                return Ok(Err(PbftExecutionRefusal::Unavailable(reason)));
            }
        };
        ensure!(
            request.context().facts().schedule == self.schedule
                && request.context().facts().period == period
                && request.snapshot().block_rlp() == block_rlp,
            "PBFT_EXECUTION_ACQUIRED_REQUEST_MISMATCH"
        );
        if let Some(reason) = self.source.recheck_pbft_execution_context(&request)? {
            return Ok(Err(PbftExecutionRefusal::Unavailable(reason)));
        }
        let report = self.source.validate_pbft_execution(&request)?;
        if let Some(reason) = self.source.recheck_pbft_execution_context(&request)? {
            return Ok(Err(PbftExecutionRefusal::Unavailable(reason)));
        }
        match &report {
            PbftExecutionValidation::Invalid {
                request_digest,
                transaction_position,
                native_sequence,
                cause,
            } => {
                ensure!(
                    *request_digest == request.digest(),
                    "PBFT_EXECUTION_REPORT_BINDING_MISMATCH"
                );
                return Ok(Err(PbftExecutionRefusal::Invalid {
                    transaction_position: *transaction_position,
                    native_sequence: *native_sequence,
                    cause: cause.clone(),
                }));
            }
            PbftExecutionValidation::Unavailable {
                request_digest,
                reason,
            } => {
                ensure!(
                    *request_digest == request.digest(),
                    "PBFT_EXECUTION_REPORT_BINDING_MISMATCH"
                );
                return Ok(Err(PbftExecutionRefusal::Unavailable(reason.clone())));
            }
            PbftExecutionValidation::Valid { .. } => {}
        }
        let validated = ValidatedPbftExecution::from_report(request.clone(), report)?
            .expect("a Valid report must produce validated evidence");
        if let Some(reason) = self.source.recheck_pbft_execution_context(&request)? {
            return Ok(Err(PbftExecutionRefusal::Unavailable(reason)));
        }
        Ok(Ok(admission(AdmissionKind::Validated(validated))))
    }

    /// Use immediately before a guarded effect. This is a context check, not an
    /// owner reservation across separately mutable state owners.
    pub fn check_vote(
        &self,
        admission: &PbftExecutionAdmission,
        period: FinalChainBlockNumber,
        block_hash: H256,
    ) -> Result<std::result::Result<(), PbftExecutionRefusal>> {
        ensure!(
            Arc::ptr_eq(&admission.issuer, &self.issuer),
            "PBFT_EXECUTION_ADMISSION_ISSUER_MISMATCH"
        );
        ensure!(
            admission.schedule_digest == self.schedule.digest() && admission.period == period,
            "PBFT_EXECUTION_ADMISSION_SCHEDULE_MISMATCH"
        );
        ensure!(
            candidate_hash(&admission.block_rlp) == block_hash,
            "PBFT_EXECUTION_ADMISSION_BLOCK_MISMATCH"
        );
        match &admission.inner {
            AdmissionKind::Historical => {
                ensure!(
                    !self.schedule.requires_validation(period),
                    "PBFT_EXECUTION_HISTORICAL_BYPASS_REFUSED"
                );
            }
            AdmissionKind::Validated(validated) => {
                if let Some(reason) = self
                    .source
                    .recheck_pbft_execution_context(validated.request())?
                {
                    return Ok(Err(PbftExecutionRefusal::Unavailable(reason)));
                }
            }
        }
        Ok(Ok(()))
    }
}

fn candidate_hash(bytes: &[u8]) -> H256 {
    use tiny_keccak::{Hasher, Keccak};
    let mut hash = [0; 32];
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    hasher.finalize(&mut hash);
    H256(hash)
}

/// Transaction admission cannot authorize certificate-dependent rewards or
/// primary writes. No successful constructor exists before owner reservation.
#[derive(Debug)]
pub struct CertifiedPbftExecutionPreflight {
    _private: (),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CertifiedPbftTransitionValidation {
    Unavailable(PbftExecutionUnavailable),
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dag::DagSyncBlockRlp;
    use crate::pbft_execution_validation::{
        PbftExecutionContext, PbftExecutionContextFacts, PbftExecutionInputLimits,
        PbftExecutionSnapshot, PbftInstructionPhase,
    };
    use k256::ecdsa::SigningKey;
    use rlp::RlpStream;
    use rustaxa_types::concrete_state::ConcreteStateIdentity;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn hash(bytes: &[u8]) -> [u8; 32] {
        candidate_hash(bytes).0
    }
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
    pub(crate) fn empty_request() -> Arc<PbftExecutionRequest> {
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

    pub(crate) struct Owner {
        pub request: Arc<PbftExecutionRequest>,
        pub mode: u8,
        pub executions: AtomicUsize,
        pub checks: AtomicUsize,
        pub stale_after: usize,
    }
    impl Owner {
        pub(crate) fn new(mode: u8, stale_after: usize) -> Arc<Self> {
            Arc::new(Self {
                request: empty_request(),
                mode,
                executions: AtomicUsize::new(0),
                checks: AtomicUsize::new(0),
                stale_after,
            })
        }
        pub(crate) fn guard(self: &Arc<Self>) -> Arc<PbftExecutionGuard> {
            let schedule = self.request.context().facts().schedule.clone();
            Arc::new(
                PbftExecutionGuard::for_offline_owner(
                    schedule.clone(),
                    Some(schedule.digest()),
                    self.clone(),
                )
                .unwrap(),
            )
        }
    }
    impl PbftExecutionContextSource for Owner {
        fn acquire_pbft_execution_request(
            &self,
            _: FinalChainBlockNumber,
            _: &[u8],
        ) -> Result<PbftExecutionRequestAcquisition> {
            Ok(PbftExecutionRequestAcquisition::Ready(self.request.clone()))
        }
        fn recheck_pbft_execution_context(
            &self,
            _: &PbftExecutionRequest,
        ) -> Result<Option<PbftExecutionUnavailable>> {
            let count = self.checks.fetch_add(1, Ordering::SeqCst);
            Ok((count >= self.stale_after).then_some(PbftExecutionUnavailable::StateChanged))
        }
    }
    impl ConsensusExecutionPort for Owner {
        fn load_pillar_anchor_state(
            &self,
            _: &crate::PillarAnchorStateRequest,
        ) -> Result<crate::PillarAnchorStateReport> {
            anyhow::bail!("unused")
        }
        fn validate_pbft_execution(
            &self,
            request: &PbftExecutionRequest,
        ) -> Result<PbftExecutionValidation> {
            self.executions.fetch_add(1, Ordering::SeqCst);
            Ok(match self.mode {
                1 => PbftExecutionValidation::Invalid {
                    request_digest: request.digest(),
                    transaction_position: 0,
                    native_sequence: Some(1),
                    cause: PbftExecutionHardError::MalformedNestedSlashingProof("proof".into()),
                },
                2 => PbftExecutionValidation::Unavailable {
                    request_digest: request.digest(),
                    reason: PbftExecutionUnavailable::ExecutorUnavailable,
                },
                _ => PbftExecutionValidation::Valid {
                    request_digest: if self.mode == 3 {
                        [0; 32]
                    } else {
                        request.digest()
                    },
                    executed_transactions: 0,
                    system_transactions: vec![],
                    results_digest: [3; 32],
                },
            })
        }
    }

    #[test]
    fn default_context_source_cannot_issue_admission() {
        struct MissingOwner;
        impl PbftExecutionContextSource for MissingOwner {}
        impl ConsensusExecutionPort for MissingOwner {
            fn load_pillar_anchor_state(
                &self,
                _: &crate::PillarAnchorStateRequest,
            ) -> Result<crate::PillarAnchorStateReport> {
                anyhow::bail!("unused")
            }
        }
        let schedule = context().facts().schedule.clone();
        let guard = PbftExecutionGuard::for_offline_owner(
            schedule.clone(),
            Some(schedule.digest()),
            Arc::new(MissingOwner),
        )
        .unwrap();
        assert!(matches!(
            guard.require(10.into(), &[0xc0]).unwrap(),
            Err(PbftExecutionRefusal::Unavailable(
                PbftExecutionUnavailable::ExecutorUnavailable
            ))
        ));
    }

    #[test]
    fn invalid_does_not_issue_admission() {
        let owner = Owner::new(1, usize::MAX);
        let result = owner
            .guard()
            .require(10.into(), owner.request.snapshot().block_rlp())
            .unwrap();
        assert!(matches!(result, Err(PbftExecutionRefusal::Invalid { .. })));
        assert_eq!(owner.executions.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unavailable_is_distinct_from_invalid() {
        let owner = Owner::new(2, usize::MAX);
        assert!(matches!(
            owner
                .guard()
                .require(10.into(), owner.request.snapshot().block_rlp())
                .unwrap(),
            Err(PbftExecutionRefusal::Unavailable(
                PbftExecutionUnavailable::ExecutorUnavailable
            ))
        ));
    }

    #[test]
    fn changed_context_overrides_invalid() {
        let owner = Owner::new(1, 1);
        assert!(matches!(
            owner
                .guard()
                .require(10.into(), owner.request.snapshot().block_rlp())
                .unwrap(),
            Err(PbftExecutionRefusal::Unavailable(
                PbftExecutionUnavailable::StateChanged
            ))
        ));
    }

    #[test]
    fn wrong_candidate_cannot_use_acquired_request() {
        let owner = Owner::new(0, usize::MAX);
        assert!(owner.guard().require(10.into(), &[0xc0]).is_err());
        assert_eq!(owner.executions.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wrong_execution_digest_does_not_issue_admission() {
        let owner = Owner::new(3, usize::MAX);
        assert!(
            owner
                .guard()
                .require(10.into(), owner.request.snapshot().block_rlp())
                .is_err()
        );
    }

    #[test]
    fn admission_retains_snapshot_and_refuses_changed_context_at_use() {
        let owner = Owner::new(0, 3);
        let guard = owner.guard();
        let admission = guard
            .require(10.into(), owner.request.snapshot().block_rlp())
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(admission.request().unwrap(), &owner.request));
        assert!(matches!(
            guard
                .check_vote(
                    &admission,
                    10.into(),
                    candidate_hash(owner.request.snapshot().block_rlp())
                )
                .unwrap(),
            Err(PbftExecutionRefusal::Unavailable(
                PbftExecutionUnavailable::StateChanged
            ))
        ));
    }

    #[test]
    fn historical_schedule_does_not_call_executor() {
        let owner = Owner::new(1, usize::MAX);
        let guard = owner.guard();
        let admission = guard.require(9.into(), &[0xc0]).unwrap().unwrap();
        guard
            .check_vote(&admission, 9.into(), candidate_hash(&[0xc0]))
            .unwrap()
            .unwrap();
        assert_eq!(owner.executions.load(Ordering::SeqCst), 0);
        assert!(admission.request().is_none());
        assert!(
            guard
                .check_vote(&admission, 10.into(), candidate_hash(&[0xc0]))
                .is_err()
        );
    }

    #[test]
    fn same_schedule_foreign_guard_cannot_supply_admission() {
        let owner = Owner::new(0, usize::MAX);
        let issuer = owner.guard();
        let consumer = owner.guard();
        let bytes = owner.request.snapshot().block_rlp();
        let admission = issuer.require(10.into(), bytes).unwrap().unwrap();
        assert!(
            consumer
                .check_vote(&admission, 10.into(), candidate_hash(bytes))
                .unwrap_err()
                .to_string()
                .contains("PBFT_EXECUTION_ADMISSION_ISSUER_MISMATCH")
        );
        issuer
            .check_vote(&admission, 10.into(), candidate_hash(bytes))
            .unwrap()
            .unwrap();
        let historical = issuer.require(9.into(), &[0xc0]).unwrap().unwrap();
        assert!(
            consumer
                .check_vote(&historical, 9.into(), candidate_hash(&[0xc0]))
                .is_err()
        );
    }

    #[test]
    fn transaction_admission_cannot_authorize_certified_transition() {
        let owner = Owner::new(0, usize::MAX);
        let admission = owner
            .guard()
            .require(10.into(), owner.request.snapshot().block_rlp())
            .unwrap()
            .unwrap();
        assert_eq!(
            owner
                .validate_certified_pbft_transition(&admission, &[0xc0])
                .unwrap(),
            CertifiedPbftTransitionValidation::Unavailable(
                PbftExecutionUnavailable::ExecutorUnavailable
            )
        );
    }
}
