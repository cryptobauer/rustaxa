//! Ordered, detached execution over a closed or immutably borrowed concrete owner.
use std::sync::Arc;

use anyhow::{Result, ensure};
use num_bigint::BigUint;
use rlp::RlpStream;
use rustaxa_consensus::{
    FinalChain,
    final_chain_execution::{
        FinalChainSystemTransactionPlanFact, plan_external_evm_system_transactions,
    },
    pbft_execution_validation::*,
};
use rustaxa_evm::{
    contracts::*,
    driver::NativeAddressClassifier,
    envelope::EnvelopeRules,
    input::LegacyInputKind,
    profile::{TaraxaPhase, TaraxaProfile},
};
use rustaxa_storage::{
    BorrowedConcreteExecutionProbe, ConcreteCheckpointReaders, ConcreteExecutionProbe,
    ConcreteExecutionProbeView, ConcreteObserverPhaseDelta,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, TARAXA_SYSTEM_ACCOUNT,
    concrete_state::execution::ConcreteExecutionRead,
    concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStateIdentity},
};
use tiny_keccak::{Hasher, Keccak};

use crate::native::ProbeNativeExecutionPort;

mod private {
    pub trait Sealed {}
    impl Sealed for rustaxa_storage::ConcreteExecutionProbe {}
    impl Sealed for rustaxa_storage::BorrowedConcreteExecutionProbe<'_> {}
}

/// The two storage capabilities approved for memory-only ordered execution.
/// Implementations cannot supply another backend or expose publication state.
///
/// ```compile_fail
/// use rustaxa_execution::period_probe::PbftConcreteExecutionProbe;
/// use rustaxa_storage::{ConcreteExecutionProbeView, ConcreteObserverPhaseDelta};
/// use rustaxa_types::concrete_state::{ConcreteReadError, ConcreteStateIdentity};
/// struct Foreign;
/// impl PbftConcreteExecutionProbe for Foreign {
///     fn committed_identity(&self) -> ConcreteStateIdentity { unimplemented!() }
///     fn historical_identity(&self) -> ConcreteStateIdentity { unimplemented!() }
///     fn prepare_observer_phase(&mut self, _: ConcreteObserverPhaseDelta) -> Result<(), ConcreteReadError> { unimplemented!() }
///     fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError> { unimplemented!() }
/// }
/// ```
pub trait PbftConcreteExecutionProbe: private::Sealed {
    fn committed_identity(&self) -> ConcreteStateIdentity;
    fn historical_identity(&self) -> ConcreteStateIdentity;
    fn prepare_observer_phase(
        &mut self,
        delta: ConcreteObserverPhaseDelta,
    ) -> Result<(), ConcreteReadError>;
    fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError>;
}

macro_rules! impl_concrete_probe {
    ($probe:ty) => {
        impl PbftConcreteExecutionProbe for $probe {
            fn committed_identity(&self) -> ConcreteStateIdentity {
                <$probe>::committed_identity(self)
            }
            fn historical_identity(&self) -> ConcreteStateIdentity {
                <$probe>::historical_identity(self)
            }
            fn prepare_observer_phase(
                &mut self,
                delta: ConcreteObserverPhaseDelta,
            ) -> Result<(), ConcreteReadError> {
                <$probe>::prepare_observer_phase(self, delta)
            }
            fn current_view(&self) -> Result<ConcreteExecutionProbeView<'_>, ConcreteReadError> {
                <$probe>::current_view(self)
            }
        }
    };
}
impl_concrete_probe!(ConcreteExecutionProbe);
impl_concrete_probe!(BorrowedConcreteExecutionProbe<'_>);

struct ProbeState<'a, C>(&'a mut C);
impl<C: PbftConcreteExecutionProbe> crate::period_kernel::PeriodState for ProbeState<'_, C> {
    type View<'a>
        = ConcreteExecutionProbeView<'a>
    where
        Self: 'a;
    fn view(&self) -> Result<Self::View<'_>> {
        Ok(self.0.current_view()?)
    }
    fn apply_phase(&mut self, delta: ConcreteObserverPhaseDelta) -> Result<()> {
        self.0.prepare_observer_phase(delta)?;
        Ok(())
    }
    fn identity(&self) -> Result<ConcreteStateIdentity> {
        Ok(self.0.current_view()?.identity())
    }
}

/// Immutable network policy. The qualified owner must authenticate these facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PbftSystemExecutionPolicy {
    bridge: [u8; 20],
    ficus: u64,
    interval: u64,
    delay: u64,
}
impl PbftSystemExecutionPolicy {
    pub fn new(bridge: [u8; 20], ficus: u64, interval: u64, delay: u64) -> Result<Self> {
        ensure!(interval != 0, "PBFT_PROBE_PILLAR_INTERVAL_ZERO");
        Ok(Self {
            bridge,
            ficus,
            interval,
            delay,
        })
    }
    pub fn digest(&self) -> [u8; 32] {
        let mut out = RlpStream::new_list(7);
        out.append(&b"rustaxa-pbft-system-policy".as_slice())
            .append(&1_u8)
            .append(&self.bridge.as_slice())
            .append(&self.ficus)
            .append(&self.interval)
            .append(&self.delay)
            .append(&10_000_000_u64);
        hash(&out.out())
    }
    fn pillar(&self, period: u64) -> Result<bool> {
        let selected = period
            .checked_add(self.delay)
            .ok_or_else(|| anyhow::anyhow!("PBFT_PROBE_PILLAR_PERIOD_OVERFLOW"))?;
        let first = if self.ficus == 0 {
            self.interval
        } else {
            self.ficus
        };
        Ok(selected >= self.ficus && selected >= first && selected % self.interval == 0)
    }
}

/// Validates the source, provenance, configuration, lifecycle and generation.
/// Implementations must acquire current owner facts, not echo request claims.
pub trait PbftProbeOwnerBinding {
    fn validate(&self, context: &PbftExecutionContext) -> Result<Option<PbftExecutionUnavailable>>;
}

/// Passive diagnostic facts from the actual unpublished execution path.
/// Events are provisional until the complete probe result is returned. The
/// observer receives no journal, storage owner, port or publication authority.
pub trait PbftExecutionObserver {
    fn transaction_started(&mut self, _transaction: &ExecutionTransaction) {}
    fn transaction_completed(
        &mut self,
        _transaction: &ExecutionTransaction,
        _result: &TransactionExecutionResult,
        _native: &[ConsensusNativeObservation],
        _raw_writes: usize,
        _state: ConcreteStateIdentity,
    ) {
    }
}

impl PbftExecutionObserver for () {}

/// Runs the complete transaction stream with no publication capability.
/// An owned probe requires a closed writable owner. A borrowed probe and delayed
/// snapshot must come from the same clean lifecycle, which stays borrowed.
/// The caller authenticates schedule enrollment and all owner bindings through
/// `binding`; this function checks local identities before and after execution.
pub fn probe_period(
    chain: &FinalChain,
    concrete: impl PbftConcreteExecutionProbe,
    delayed: Arc<ConcreteCheckpointReaders>,
    request: &PbftExecutionRequest,
    policy: &PbftSystemExecutionPolicy,
    binding: &impl PbftProbeOwnerBinding,
) -> Result<PbftExecutionValidation> {
    probe_period_with_observer(chain, concrete, delayed, request, policy, binding, &mut ())
}

/// Runs the same probe with passive transaction diagnostics. The trusted caller
/// must keep the observer free of owner mutations. Both entries use identical
/// execution and owner checks, including the check after a failed attempt.
pub fn probe_period_with_observer(
    chain: &FinalChain,
    concrete: impl PbftConcreteExecutionProbe,
    delayed: Arc<ConcreteCheckpointReaders>,
    request: &PbftExecutionRequest,
    policy: &PbftSystemExecutionPolicy,
    binding: &impl PbftProbeOwnerBinding,
    observer: &mut impl PbftExecutionObserver,
) -> Result<PbftExecutionValidation> {
    if let Some(reason) = binding.validate(request.context())? {
        return Ok(PbftExecutionValidation::Unavailable {
            request_digest: request.digest(),
            reason,
        });
    }
    let result = probe_period_inner(chain, concrete, delayed, request, policy, binding, observer);
    if let Some(reason) = binding.validate(request.context())? {
        return Ok(PbftExecutionValidation::Unavailable {
            request_digest: request.digest(),
            reason,
        });
    }
    result
}

fn probe_period_inner(
    chain: &FinalChain,
    mut concrete: impl PbftConcreteExecutionProbe,
    delayed: Arc<ConcreteCheckpointReaders>,
    request: &PbftExecutionRequest,
    policy: &PbftSystemExecutionPolicy,
    binding: &impl PbftProbeOwnerBinding,
    observer: &mut impl PbftExecutionObserver,
) -> Result<PbftExecutionValidation> {
    let unavailable = |reason| PbftExecutionValidation::Unavailable {
        request_digest: request.digest(),
        reason,
    };
    if let Some(reason) = binding.validate(request.context())? {
        return Ok(unavailable(reason));
    }
    let facts = request.context().facts();
    if policy.digest() != facts.system_policy_digest
        || chain.pbft_execution_instruction_phase(facts.period) != facts.instruction_phase
        || chain.pbft_execution_cornus(facts.period) != facts.cornus
        || chain.pbft_execution_block_gas_limit() != FinalChainGas::new(facts.gas_limit)
        || chain.correction_stage_at(facts.period) != request.context().correction_stage()
    {
        return Ok(unavailable(PbftExecutionUnavailable::UnsupportedRules));
    }
    if concrete.committed_identity() != facts.prior
        || concrete.historical_identity() != facts.prior
        || delayed.committed_identity() != facts.prior
        || chain.last_block_number_typed()? != facts.prior.period
    {
        return Ok(unavailable(PbftExecutionUnavailable::StateChanged));
    }
    if chain.pending_delayed_identity(facts.period, facts.prior.period)? != facts.delayed
        || !delayed.retained_identities().contains(&facts.delayed)
    {
        return Ok(unavailable(
            PbftExecutionUnavailable::DelayedStateUnavailable,
        ));
    }
    let profile = profile(facts.instruction_phase)
        .with_correction_stage(request.context().correction_stage());
    let system = match system_inputs(chain, &concrete, request, policy)? {
        SystemInputs::Ready(inputs) => inputs,
        SystemInputs::Unavailable(reason) => return Ok(unavailable(reason)),
    };
    let dpos = {
        let mut a = [0; 20];
        a[19] = 0xfe;
        a
    };
    let empty = match concrete.current_view()?.account(dpos)? {
        ConcreteRead::Present(row) => row.account.code_size == 0,
        _ => true,
    };
    crate::period_kernel::prepare_native_code(
        &mut ProbeState(&mut concrete),
        chain
            .native_code_updates(facts.period, empty)
            .into_iter()
            .map(|update| crate::period_kernel::NativeCodeUpdate {
                address: update.address,
                code: update.code.to_vec(),
            }),
    )?;
    let native = chain.begin_native_execution_probe(
        request.digest(),
        facts.state_api_epoch,
        facts.concrete_generation,
        facts.period,
        facts.prior.period,
        request.context().correction_stage(),
        Arc::clone(&delayed),
    )?;
    let mut native = ProbeNativeExecutionPort::new(native);
    let block = ExecutionBlockContext {
        period: facts.period,
        author: facts.author,
        timestamp: facts.timestamp,
        gas_limit: facts.gas_limit.into(),
        chain_id: facts.schedule.chain_id(),
        difficulty: BigUint::default(),
    };
    let completed = crate::period_kernel::execute_transaction_stream(
        &mut ProbeState(&mut concrete),
        &mut native,
        &block,
        profile,
        facts.cornus,
        chain.pre_magnolia_fee_reward_period(facts.period),
        request
            .snapshot()
            .transactions()
            .iter()
            .map(|t| (t.rlp.as_slice(), LegacyInputKind::Signed))
            .chain(
                system
                    .iter()
                    .map(|t| (t.as_slice(), LegacyInputKind::System)),
            ),
        &ChainHashes(chain),
        &AllNatives(profile, chain),
        &ConsensusNatives(chain),
        observer,
    )?;
    let completed = match completed {
        crate::period_kernel::PeriodKernelOutcome::Completed(completed) => completed,
        crate::period_kernel::PeriodKernelOutcome::MalformedProof {
            transaction_position,
            native_sequence,
            message,
        } => {
            if let Some(reason) = binding.validate(request.context())? {
                return Ok(unavailable(reason));
            }
            return Ok(PbftExecutionValidation::Invalid {
                request_digest: request.digest(),
                transaction_position,
                native_sequence,
                cause: PbftExecutionHardError::MalformedNestedSlashingProof(message),
            });
        }
    };
    let mut results = RlpStream::new_list(4);
    results
        .append(&b"rustaxa-pbft-probe-results".as_slice())
        .append(&1_u8)
        .append(&request.digest().as_slice());
    results.begin_list(completed.len());
    for transaction in &completed {
        append_result(
            &mut results,
            &transaction.transaction,
            &transaction.result,
            transaction.state,
        );
    }
    native.validate_context()?;
    if let Some(reason) = binding.validate(request.context())? {
        return Ok(unavailable(reason));
    }
    ensure!(
        chain.last_block_number_typed()? == facts.prior.period
            && delayed.committed_identity() == facts.prior,
        "PBFT_PROBE_CONTEXT_CHANGED"
    );
    Ok(PbftExecutionValidation::Valid {
        request_digest: request.digest(),
        executed_transactions: (request.snapshot().transactions().len() + system.len()) as u64,
        system_transactions: system,
        results_digest: hash(&results.out()),
    })
}

enum SystemInputs {
    Ready(Vec<Vec<u8>>),
    Unavailable(PbftExecutionUnavailable),
}
fn system_inputs(
    _chain: &FinalChain,
    concrete: &impl PbftConcreteExecutionProbe,
    request: &PbftExecutionRequest,
    policy: &PbftSystemExecutionPolicy,
) -> Result<SystemInputs> {
    let facts = request.context().facts();
    let pillar = policy.pillar(facts.period.as_u64())?;
    let bridge = concrete.current_view()?.account(policy.bridge)?;
    let found = matches!(&bridge, ConcreteRead::Present(_));
    let has_code = matches!(&bridge, ConcreteRead::Present(row) if row.account.code_size != 0);
    let should_finalize = false;
    if pillar && has_code {
        // Historical native and delayed physical sources need a qualified factory.
        return Ok(SystemInputs::Unavailable(
            PbftExecutionUnavailable::UnsupportedRules,
        ));
    }
    let nonce = if should_finalize {
        match concrete.current_view()?.account(TARAXA_SYSTEM_ACCOUNT)? {
            ConcreteRead::Present(row) => row.account.nonce,
            _ => FinalChainNonce::zero(),
        }
    } else {
        FinalChainNonce::zero()
    };
    let plan = plan_external_evm_system_transactions(FinalChainSystemTransactionPlanFact {
        request_id: request.digest(),
        period: facts.period,
        is_pillar_block_period: pillar,
        bridge_contract_address: policy.bridge,
        bridge_contract_found: found,
        bridge_contract_has_code: has_code,
        should_finalize_epoch: should_finalize,
        system_account_nonce: nonce,
        block_gas_limit: facts.gas_limit.into(),
    })?;
    Ok(SystemInputs::Ready(plan.transactions))
}

/// Applies the post-checkpoint fee phase, including a zero-amount touch.
/// The transaction phase must already be settled in this detached probe.
pub fn apply_probe_transaction_fee(
    concrete: &mut impl PbftConcreteExecutionProbe,
    author: [u8; 20],
    price: &BigUint,
    gas: u64,
    stage: rustaxa_evm::profile::CorrectionStage,
) -> Result<()> {
    crate::period_kernel::apply_transaction_fee(
        &mut ProbeState(concrete),
        author,
        price,
        gas,
        stage,
    )
}

fn profile(phase: PbftInstructionPhase) -> TaraxaProfile {
    TaraxaProfile::for_phase(match phase {
        PbftInstructionPhase::Californicum => TaraxaPhase::Californicum,
        PbftInstructionPhase::Ficus => TaraxaPhase::Ficus,
        PbftInstructionPhase::Cacti => TaraxaPhase::Cacti,
    })
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut result = [0; 32];
    let mut h = Keccak::v256();
    h.update(bytes);
    h.finalize(&mut result);
    result
}
pub(super) fn gas_used(result: &TransactionExecutionResult) -> u64 {
    match result {
        TransactionExecutionResult::Executed(r) => r.gas_used.as_u64(),
        TransactionExecutionResult::ConsensusFailure(r) => r.gas_used.as_u64(),
    }
}
pub(crate) fn append_result(
    out: &mut RlpStream,
    transaction: &ExecutionTransaction,
    result: &TransactionExecutionResult,
    identity: ConcreteStateIdentity,
) {
    out.begin_list(9)
        .append(&transaction.hash.as_slice())
        .append(&gas_used(result))
        .append(&identity.state_root.as_slice());
    let (category, status, detail, output, refund, created, logs) = match result {
        TransactionExecutionResult::Executed(r) => {
            let (status, detail) = match &r.status {
                CodeExecutionStatus::Success => (0_u8, vec![]),
                CodeExecutionStatus::Failure(error) => code_status(error),
            };
            (
                0_u8,
                status,
                detail,
                &r.output,
                r.refund_applied.as_u64(),
                r.attempted_contract_address,
                r.logs.as_slice(),
            )
        }
        TransactionExecutionResult::ConsensusFailure(r) => {
            let status = match r.error {
                ConsensusFailure::InsufficientBalanceForGas => 0_u8,
                ConsensusFailure::NonceTooLow => 1,
                ConsensusFailure::IntrinsicGasOverflow => 2,
                ConsensusFailure::IntrinsicGas => 3,
                ConsensusFailure::InsufficientBalanceForTransfer => 4,
            };
            (
                1_u8,
                status,
                vec![],
                &r.output,
                0,
                r.attempted_contract_address,
                &[] as &[ExecutionLog],
            )
        }
    };
    out.begin_list(3)
        .append(&category)
        .append(&status)
        .append(&detail);
    out.append(output).append(&refund);
    out.begin_list(usize::from(created.is_some()));
    if let Some(created) = created {
        out.append(&created.as_slice());
    }
    out.begin_list(logs.len());
    for log in logs {
        out.begin_list(3)
            .append(&log.address.as_slice())
            .begin_list(log.topics.len());
        for topic in &log.topics {
            out.append(&topic.as_slice());
        }
        out.append(&log.data);
    }
    out.append(&transaction.position.as_u32());
}
fn code_status(error: &CodeExecutionError) -> (u8, Vec<u8>) {
    use CodeExecutionError::*;
    match error {
        Revert => (1, vec![]),
        OutOfGas => (2, vec![]),
        InvalidOpcode(opcode) => (3, vec![*opcode]),
        StackUnderflow => (4, vec![]),
        StackOverflow => (5, vec![]),
        StaticViolation => (6, vec![]),
        Depth => (7, vec![]),
        CreateCollision => (8, vec![]),
        ContractSize => (9, vec![]),
        CodeDepositOutOfGas => (10, vec![]),
        InvalidJump => (11, vec![]),
        ReturnDataOutOfBounds => (12, vec![]),
        GasUintOverflow => (13, vec![]),
        Native(error) => (14, error.error.as_bytes().to_vec()),
    }
}
struct ChainHashes<'a>(&'a FinalChain);
impl BlockHashRead for ChainHashes<'_> {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        self.0
            .block_hash(number)
            .map_err(|e| BlockHashReadError::Io(e.to_string()))?
            .ok_or(BlockHashReadError::HistoryUnavailable(number))?
            .try_into()
            .map_err(|_| BlockHashReadError::Corrupt("block hash width".into()))
    }
}
struct ConsensusNatives<'a>(&'a FinalChain);
impl NativeAddressClassifier for ConsensusNatives<'_> {
    fn is_native_address(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.0.native_contract_kind(period, address).is_some()
    }
}
struct AllNatives<'a>(TaraxaProfile, &'a FinalChain);
impl NativeAddressClassifier for AllNatives<'_> {
    fn is_native_address(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        use rustaxa_evm::bls::{BlsPrecompile, BlsRegistry};
        self.1.native_contract_kind(period, address).is_some()
            || (address[..19] == [0; 19]
                && (matches!(address[19], 1..=8) || self.0.ficus() && address[19] == 9))
            || self.0.ficus()
                && BlsPrecompile::at_address(
                    if self.0.cacti() {
                        BlsRegistry::Cacti
                    } else {
                        BlsRegistry::Ficus
                    },
                    address,
                )
                .is_some()
            || self.0.cacti()
                && matches!(
                    address,
                    rustaxa_evm::p256::P256_VERIFY_ADDRESS
                        | rustaxa_evm::falcon::FALCON_VERIFY_ADDRESS
                )
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../rustaxa-evm/tests/support/mixed_genesis.rs"]
mod mixed_genesis;
#[cfg(test)]
#[path = "../../rustaxa-storage/src/test_paths.rs"]
mod test_paths;
#[cfg(test)]
mod fee_tests {
    use super::*;
    use crate::period_kernel::settlement_delta;
    use rustaxa_evm::{contracts::ExecutionBalance, journal::ExecutionJournal};
    use rustaxa_types::concrete_state::ConcreteStorageKey;

    fn with_probe(test: impl FnOnce(&mut ConcreteExecutionProbe, [u8; 20])) {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/mixed_workload_local_observer.json"
        ))
        .unwrap();
        let directory = super::test_paths::new_path("probe-fee");
        std::fs::create_dir_all(&directory).unwrap();
        let (application, chain) =
            mixed_genesis::open_chain(&directory.join("application"), &fixture).unwrap();
        let owner =
            mixed_genesis::fresh_concrete(&directory.join("concrete"), &chain, &fixture).unwrap();
        let prior = owner.observation().unwrap().committed;
        let author: [u8; 20] = hex::decode(fixture["inputs"]["sender"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        drop(owner);
        drop(chain);
        drop(application);
        let mut probe =
            ConcreteExecutionProbe::open_read_only(directory.join("concrete"), prior, prior)
                .unwrap();
        test(&mut probe, author);
        drop(probe);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deleted_code_author_is_recreated_after_checkpoint_before_fee() {
        with_probe(|probe, author| {
            let key = ConcreteStorageKey([1; 32]);
            let mut journal = ExecutionJournal::new(probe.current_view().unwrap());
            journal.set_code(author, vec![0x60, 0, 0x50, 0]).unwrap();
            journal
                .set_ordinary_storage(author, key, BigUint::from(7_u8))
                .unwrap();
            let settled = journal.settle_transaction().unwrap();
            drop(journal);
            probe
                .prepare_observer_phase(settlement_delta(settled.writes))
                .unwrap();
            let mut journal = ExecutionJournal::new(probe.current_view().unwrap());
            journal.selfdestruct(author, [0x45; 20]).unwrap();
            let settled = journal.settle_transaction().unwrap();
            drop(journal);
            probe
                .prepare_observer_phase(settlement_delta(settled.writes))
                .unwrap();
            assert!(matches!(
                probe.current_view().unwrap().account(author).unwrap(),
                ConcreteRead::Tombstone | ConcreteRead::Absent
            ));
            apply_probe_transaction_fee(
                probe,
                author,
                &BigUint::from(3_u8),
                7,
                rustaxa_evm::profile::CorrectionStage::Historical,
            )
            .unwrap();
            let ConcreteRead::Present(account) =
                probe.current_view().unwrap().account(author).unwrap()
            else {
                panic!("fee must recreate author")
            };
            assert_eq!(account.account.balance.value(), &BigUint::from(21_u8));
            assert_eq!(account.account.nonce, FinalChainNonce::zero());
            assert_eq!(account.account.code_hash, None);
            assert_eq!(account.account.code_size, 0);
            assert_eq!(account.account.storage_root, None);
            let journal = ExecutionJournal::new(probe.current_view().unwrap());
            assert_eq!(
                journal.ordinary_storage(author, key).unwrap(),
                (BigUint::default(), BigUint::default())
            );
            drop(journal);
            apply_probe_transaction_fee(
                probe,
                author,
                &BigUint::from(3_u8),
                7,
                rustaxa_evm::profile::CorrectionStage::Historical,
            )
            .unwrap();
            let ConcreteRead::Present(account) =
                probe.current_view().unwrap().account(author).unwrap()
            else {
                panic!("second fee must retain author")
            };
            assert_eq!(account.account.balance.value(), &BigUint::from(42_u8));
        });
    }

    #[test]
    fn empty_author_zero_fee_is_pruned_after_both_checkpoints() {
        with_probe(|probe, author| {
            let mut journal = ExecutionJournal::new(probe.current_view().unwrap());
            journal
                .set_balance(author, ExecutionBalance::default())
                .unwrap();
            journal.set_nonce(author, FinalChainNonce::zero()).unwrap();
            let settled = journal.settle_transaction().unwrap();
            drop(journal);
            probe
                .prepare_observer_phase(settlement_delta(settled.writes))
                .unwrap();
            apply_probe_transaction_fee(
                probe,
                author,
                &BigUint::default(),
                21_000,
                rustaxa_evm::profile::CorrectionStage::Historical,
            )
            .unwrap();
            assert!(matches!(
                probe.current_view().unwrap().account(author).unwrap(),
                ConcreteRead::Absent | ConcreteRead::Tombstone
            ));
        });
    }
}
