//! Detached native execution and its consumed terminal result.
use super::execution_probe::CapturedNativeContext;
use super::*;
use crate::final_chain::native_predecessor::CapturedNativePredecessor;
use std::sync::Arc;

/// Native execution from an actual captured predecessor. Drop discards changes.
pub struct FinalChainPreparedNativeSession {
    session: FinalChainNativeSession<'static>,
    context: CapturedNativeContext,
    source: CapturedNativePredecessor,
    state_api_epoch: u64,
    generation: u64,
}

/// One completed native transition. Copied reward data cannot create this token.
/// Independent projection checks and certified admission are still required.
pub struct FinalChainFinishedNativeTransition {
    pub(in crate::final_chain) owner: Arc<FinalChain>,
    pub(in crate::final_chain) plan: FinalChainPreparedExternalEvmRewardsStatsPlan,
    pub(in crate::final_chain) outcome: FinalChainNativeRewardsOutcome,
    pub(in crate::final_chain) source: CapturedNativePredecessor,
    pub(in crate::final_chain) context: CapturedNativeContext,
    pub(in crate::final_chain) state_api_epoch: u64,
    pub(in crate::final_chain) generation: u64,
    pub(in crate::final_chain) request_id: [u8; 32],
    pub(in crate::final_chain) basis: reward_scheduler::FinalChainRewardSchedulerBasis,
    pub(in crate::final_chain) delayed: std::rc::Rc<frozen_eligibility::FrozenEligibility>,
}

impl FinalChainPreparedNativeSession {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::final_chain) fn from_captured_session(
        mut session: FinalChainNativeSession<'_>,
        owner: &Arc<FinalChain>,
        source: CapturedNativePredecessor,
        parent: Arc<DposSnapshot>,
        committed: FinalChainExternalEvmCommittedStateDescriptor,
        selected: rustaxa_types::concrete_state::ConcreteStateIdentity,
        state_api_epoch: u64,
        generation: u64,
        reader: Arc<rustaxa_storage::ConcreteCheckpointReaders>,
    ) -> Result<Self, FinalChainNativeSessionError> {
        source.validate(owner)?;
        session.captured_rewards_parent = Some(parent);
        let context = CapturedNativeContext::install(
            &mut session,
            committed,
            selected,
            state_api_epoch,
            generation,
            reader,
        )?;
        let transition = Self {
            session: session.into_owned(owner)?,
            context,
            source,
            state_api_epoch,
            generation,
        };
        transition.check_context()?;
        Ok(transition)
    }

    fn check_context(&self) -> Result<(), FinalChainNativeSessionError> {
        self.context.validate(&self.session.final_chain)?;
        self.source.validate(&self.session.final_chain)?;
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

    /// Checks the retained owner and source before the next operation.
    pub fn validate_context(&mut self) -> Result<(), FinalChainNativeSessionError> {
        self.before()
    }

    /// Returns the correction identity of the captured source.
    pub fn correction_stage(&self) -> rustaxa_types::evm_correction::CorrectionStage {
        self.session.correction_stage()
    }

    /// Prepares one invocation against the current journal.
    pub fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.prepare(request, state);
        self.after(result)
    }

    /// Runs one exact prepared invocation in private native state.
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

    /// Takes aliases before input changes consume the completed invocation.
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

    /// Finishes rewards once and retains the exact plan and source.
    pub fn finish_rewards(
        mut self,
        plan: FinalChainPreparedExternalEvmRewardsStatsPlan,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainFinishedNativeTransition, FinalChainNativeSessionError> {
        self.before()?;
        let result = self.session.finish_rewards(&plan, state);
        let outcome = self.after(result)?;
        let request_id = self
            .session
            .request_id
            .ok_or(FinalChainNativeSessionError::UnboundRewards)?;
        let basis = self
            .session
            .reward_scheduler_basis
            .ok_or(FinalChainNativeSessionError::UnboundRewards)?;
        let delayed = self.session.delayed_physical.take().ok_or_else(|| {
            FinalChainNativeSessionError::Domain("native delayed source unavailable".into())
        })?;
        let NativeSessionOwner::Owned(owner) = self.session.final_chain else {
            return Err(FinalChainNativeSessionError::Domain(
                "native session owner mismatch".into(),
            ));
        };
        Ok(FinalChainFinishedNativeTransition {
            owner,
            plan,
            outcome,
            source: self.source,
            context: self.context,
            state_api_epoch: self.state_api_epoch,
            generation: self.generation,
            request_id,
            basis,
            delayed,
        })
    }
}

impl FinalChainFinishedNativeTransition {
    /// Borrows terminal effects. These effects do not authorize publication.
    /// Borrows the exact canonical reward inputs from the completed plan.
    pub fn rewards_input(&self) -> Vec<u8> {
        crate::concrete_state_projection::encode_concrete_rewards_input(
            &self.plan.distribution_stats,
        )
    }

    pub fn rewards(&self) -> &FinalChainNativeRewardsOutcome {
        &self.outcome
    }
}
