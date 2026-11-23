//! Private pending-period native execution with no publication capability.
use super::*;
use std::sync::Arc;

/// Detached pending-period state. Drop discards all native changes.
///
/// This capability cannot finish rewards or expose its inner session.
///
/// ```compile_fail
/// use rustaxa_consensus::native_session::FinalChainNativeExecutionProbe;
/// fn inner(probe: FinalChainNativeExecutionProbe<'_>) {
///     let _session = probe.session;
/// }
/// ```
///
/// ```compile_fail
/// use rustaxa_consensus::native_session::{FinalChainNativeExecutionProbe, FinalChainNativeStateRead};
/// use rustaxa_consensus::final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan;
/// fn rewards(mut probe: FinalChainNativeExecutionProbe<'_>, plan: &FinalChainPreparedExternalEvmRewardsStatsPlan, read: &dyn FinalChainNativeStateRead) {
///     let _ = probe.finish_rewards(plan, read);
/// }
/// ```
pub struct FinalChainNativeExecutionProbe<'a> {
    session: FinalChainNativeSession<'a>,
    context: CapturedNativeContext,
}

pub(in crate::final_chain) struct CapturedNativeContext {
    scheduler: reward_scheduler::FinalChainRewardSchedulerRuntime,
    committed: FinalChainExternalEvmCommittedStateDescriptor,
}

impl CapturedNativeContext {
    pub(in crate::final_chain) fn install(
        session: &mut FinalChainNativeSession<'_>,
        committed: FinalChainExternalEvmCommittedStateDescriptor,
        selected: rustaxa_types::concrete_state::ConcreteStateIdentity,
        state_api_epoch: u64,
        generation: u64,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> Result<Self, FinalChainNativeSessionError> {
        let domain = |error: anyhow::Error| FinalChainNativeSessionError::Domain(error.to_string());
        let owner = &session.final_chain;
        let request_id = session.request_id.ok_or_else(|| {
            FinalChainNativeSessionError::Domain("native probe request unavailable".into())
        })?;
        let pending_period = session.pending_period;
        if session.origin != NativeSessionOrigin::PendingPeriod
            || committed.period.checked_next() != Some(pending_period)
            || selected.period != session.eligibility_period
        {
            return Err(FinalChainNativeSessionError::PendingPeriodMismatch);
        }
        let scheduler = owner
            .reward_scheduler_runtime
            .lock()
            .map_err(|_| {
                FinalChainNativeSessionError::Domain("reward scheduler lock poisoned".into())
            })?
            .clone();
        let basis = scheduler
            .probe_basis(
                state_api_epoch,
                committed.period,
                pending_period,
                request_id,
            )
            .map_err(domain)?;
        let identity = reader.committed_identity();
        if owner.committed_state_descriptor().map_err(domain)? != committed
            || identity.period != committed.period
            || identity.state_root != committed.state_root
            || !reader.retained_identities().contains(&identity)
        {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe committed checkpoint mismatch".into(),
            ));
        }
        let delayed = std::rc::Rc::new(
            frozen_eligibility::FrozenEligibility::from_owner(
                reader,
                selected,
                request_id,
                (state_api_epoch, generation),
                committed.period.as_u64(),
                pending_period.as_u64(),
                basis,
            )
            .map_err(domain)?,
        );
        session.reward_scheduler_basis = Some(basis);
        session
            .install_pending_delayed_view(delayed)
            .map_err(domain)?;
        let context = Self {
            scheduler,
            committed,
        };
        context.validate(&session.final_chain)?;
        Ok(context)
    }

    pub(in crate::final_chain) fn validate(
        &self,
        owner: &FinalChain,
    ) -> Result<(), FinalChainNativeSessionError> {
        let current = owner.reward_scheduler_runtime.lock().map_err(|_| {
            FinalChainNativeSessionError::Domain("reward scheduler lock poisoned".into())
        })?;
        if *current != self.scheduler {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe scheduler context changed".into(),
            ));
        }
        drop(current);
        if owner
            .committed_state_descriptor()
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?
            != self.committed
        {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe committed context changed".into(),
            ));
        }
        Ok(())
    }
}

impl<'a> FinalChainNativeExecutionProbe<'a> {
    pub(in crate::final_chain) fn from_captured_session(
        mut session: FinalChainNativeSession<'a>,
        committed: FinalChainExternalEvmCommittedStateDescriptor,
        selected: rustaxa_types::concrete_state::ConcreteStateIdentity,
        state_api_epoch: u64,
        generation: u64,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> Result<Self, FinalChainNativeSessionError> {
        let context = CapturedNativeContext::install(
            &mut session,
            committed,
            selected,
            state_api_epoch,
            generation,
            reader,
        )?;
        let probe = Self { session, context };
        probe.check_context()?;
        Ok(probe)
    }
}

impl FinalChain {
    /// Opens a detached session over the exact finalized parent.
    ///
    /// The caller must qualify the checkpoint source and epoch/generation with
    /// its concrete owner. These integers alone do not prove owner authority.
    /// The caller must also recheck the full context before using any result.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_native_execution_probe(
        &self,
        request_id: [u8; 32],
        state_api_epoch: u64,
        generation: u64,
        pending_period: FinalChainBlockNumber,
        expected_parent: FinalChainBlockNumber,
        selected_stage: rustaxa_types::evm_correction::CorrectionStage,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> Result<FinalChainNativeExecutionProbe<'_>, FinalChainNativeSessionError> {
        let domain = |error: anyhow::Error| FinalChainNativeSessionError::Domain(error.to_string());
        if selected_stage != self.correction_stage_at(pending_period) {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe correction stage unavailable".into(),
            ));
        }
        let scheduler = self
            .reward_scheduler_runtime
            .lock()
            .map_err(|_| {
                FinalChainNativeSessionError::Domain("reward scheduler lock poisoned".into())
            })?
            .clone();
        let basis = scheduler
            .probe_basis(state_api_epoch, expected_parent, pending_period, request_id)
            .map_err(domain)?;
        let committed = self.committed_state_descriptor().map_err(domain)?;
        let identity = reader.committed_identity();
        if committed.period != expected_parent
            || identity.period != committed.period
            || identity.state_root != committed.state_root
            || !reader.retained_identities().contains(&identity)
        {
            return Err(FinalChainNativeSessionError::Domain(
                "native probe committed checkpoint mismatch".into(),
            ));
        }
        let selected = self
            .pending_delayed_identity(pending_period, expected_parent)
            .map_err(domain)?;
        let delayed = std::rc::Rc::new(
            frozen_eligibility::FrozenEligibility::from_owner(
                reader,
                selected,
                request_id,
                (state_api_epoch, generation),
                expected_parent.as_u64(),
                pending_period.as_u64(),
                basis,
            )
            .map_err(domain)?,
        );
        let mut session =
            self.begin_native_session_inner(Some(request_id), pending_period, expected_parent)?;
        session.reward_scheduler_basis = Some(basis);
        session
            .install_pending_delayed_view(delayed)
            .map_err(domain)?;
        let probe = FinalChainNativeExecutionProbe {
            session,
            context: CapturedNativeContext {
                scheduler,
                committed,
            },
        };
        probe.check_context()?;
        Ok(probe)
    }
}

impl FinalChainNativeExecutionProbe<'_> {
    /// Checks the detached basis even when no native invocation took place.
    pub fn validate_context(&mut self) -> Result<(), FinalChainNativeSessionError> {
        self.before()
    }

    fn check_context(&self) -> Result<(), FinalChainNativeSessionError> {
        self.context.validate(&self.session.final_chain)
    }

    fn before(&mut self) -> Result<(), FinalChainNativeSessionError> {
        if self.session.aborted {
            return Err(FinalChainNativeSessionError::Aborted);
        }
        self.check_context()
            .inspect_err(|_| self.session.aborted = true)
    }

    fn after<T>(
        &mut self,
        result: Result<T, FinalChainNativeSessionError>,
    ) -> Result<T, FinalChainNativeSessionError> {
        let result = self.check_context().and(result);
        result.inspect_err(|_| self.session.aborted = true)
    }

    /// Returns the immutable owner stage checked at construction.
    pub fn correction_stage(&self) -> rustaxa_types::evm_correction::CorrectionStage {
        self.session.correction_stage()
    }

    /// Prepares one invocation against the current transaction journal.
    pub fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.prepare(request, state);
        self.after(result)
    }

    /// Runs one prepared invocation and retains only private semantic state.
    pub fn invoke(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.invoke(request, quote, state);
        self.after(result)
    }

    /// Takes exact aliases before input mutations consume completion.
    pub fn input_aliases(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<Vec<FinalChainNativeInputAlias>, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.input_aliases(id);
        self.after(result)
    }

    /// Takes input changes from the last completed invocation.
    pub fn input_mutations(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<Vec<FinalChainNativeInputMutation>, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.input_mutations(id);
        self.after(result)
    }
}
