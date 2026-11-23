//! Real native domain conversion for detached pending-period execution.
use std::{cell::RefCell, collections::BTreeMap};

use rustaxa_consensus::native_projection_context::{
    FinalChainNativeInvocationContext, FinalChainNativeRawRead, FinalChainNativeRewardsContext,
};
use rustaxa_consensus::native_session::{
    FinalChainFinishedNativeTransition, FinalChainNativeAccount, FinalChainNativeCallKind,
    FinalChainNativeExecutionProbe, FinalChainNativeGasQuote, FinalChainNativeInvocationId,
    FinalChainNativeInvocationResult, FinalChainNativeOrdinaryMutation, FinalChainNativeOutcome,
    FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRequest,
    FinalChainNativeSessionError, FinalChainNativeSimulation, FinalChainNativeStateRead,
    FinalChainNativeStateReadError, FinalChainNativeStatus, FinalChainNativeValue,
    FinalChainPreparedNativeSession,
};
use rustaxa_evm::contracts::{
    ExecutionBalance, ExecutionLog, NativeCallKind, NativeContractFailure, NativeExecutionPort,
    NativeGasQuote, NativeInvocation, NativeInvocationId, NativeInvocationResult,
    NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation, NativeOutcome,
    NativePortError, NativeRawMutation, NativeRawOperation, NativeRawValue, NativeStatus,
};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};

type RawIdentity = ([u8; 20], ConcreteStorageKey);

#[derive(Clone, Debug, Default)]
struct RecordedReads {
    accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
    raw: BTreeMap<RawIdentity, ConcreteRead<Vec<u8>>>,
}

impl RecordedReads {
    fn raw_reads(
        raw: BTreeMap<RawIdentity, ConcreteRead<Vec<u8>>>,
    ) -> Vec<FinalChainNativeRawRead> {
        raw.into_iter()
            .map(|((address, key), value)| FinalChainNativeRawRead {
                address,
                key,
                value,
            })
            .collect()
    }
}

struct RecordingRead<'a> {
    journal: &'a dyn NativeJournalRead,
    reads: RefCell<RecordedReads>,
}

impl<'a> RecordingRead<'a> {
    fn new(journal: &'a dyn NativeJournalRead, reads: RecordedReads) -> Self {
        Self {
            journal,
            reads: RefCell::new(reads),
        }
    }

    fn into_reads(self) -> RecordedReads {
        self.reads.into_inner()
    }

    fn validate_previous(&self) -> Result<(), FinalChainNativeStateReadError> {
        let previous = self.reads.borrow().clone();
        for address in previous.accounts.keys() {
            self.account(*address)?;
        }
        for (address, key) in previous.raw.keys() {
            self.raw_storage(*address, key)?;
        }
        Ok(())
    }
}

impl FinalChainNativeStateRead for RecordingRead<'_> {
    fn borrowed_raw_storage(&self, address: [u8; 20], key: &ConcreteStorageKey) -> Option<Vec<u8>> {
        self.journal.borrowed_raw_storage(address, key)
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        let account = self.journal.account(address).map_err(state_read_error)?;
        let account = FinalChainNativeAccount {
            exists: account.exists,
            nonce: account.nonce,
            balance: account.balance.value().clone(),
        };
        let mut reads = self.reads.borrow_mut();
        if let Some(previous) = reads.accounts.get(&address) {
            if previous != &account {
                return Err(FinalChainNativeStateReadError::Invariant(format!(
                    "native account read changed during one phase: {address:?}"
                )));
            }
        } else {
            reads.accounts.insert(address, account.clone());
        }
        Ok(account)
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let value = self
            .journal
            .raw_storage(address, key)
            .map_err(state_read_error)?;
        let identity = (address, *key);
        let mut reads = self.reads.borrow_mut();
        if let Some(previous) = reads.raw.get(&identity) {
            if previous != &value {
                return Err(FinalChainNativeStateReadError::Invariant(format!(
                    "native raw read changed during one phase: {address:?}/{key:?}"
                )));
            }
        } else {
            reads.raw.insert(identity, value.clone());
        }
        Ok(value)
    }
}

struct PendingInvocation {
    request: FinalChainNativeRequest,
    quote: NativeGasQuote,
    reads: RecordedReads,
}

trait NativeBackend {
    fn correction_stage(&self) -> rustaxa_evm::profile::CorrectionStage;
    fn validate_context(&mut self) -> Result<(), FinalChainNativeSessionError>;
    fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError>;
    fn invoke(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError>;
    fn input_aliases(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputAlias>,
        FinalChainNativeSessionError,
    >;
    fn input_mutations(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputMutation>,
        FinalChainNativeSessionError,
    >;
}

struct NativePortCore<B> {
    backend: B,
    pending: Option<PendingInvocation>,
    failed: bool,
    invocation_contexts: Option<Vec<FinalChainNativeInvocationContext>>,
}

impl<B: NativeBackend> NativePortCore<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            pending: None,
            failed: false,
            invocation_contexts: None,
        }
    }

    fn validate_context(&mut self) -> Result<(), NativePortError> {
        self.checked(|this| this.backend.validate_context().map_err(native_port_error))
    }
    fn checked<T>(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<T, NativePortError>,
    ) -> Result<T, NativePortError> {
        if self.failed {
            return Err(native_port_error(FinalChainNativeSessionError::Aborted));
        }
        let result = action(self);
        if result.is_err() {
            self.failed = true;
            self.pending = None;
        }
        result
    }
}

/// Native port with private pending-period state and no publication authority.
/// The caller must drop the complete candidate on a driver hard error.
pub struct ProbeNativeExecutionPort<'a> {
    core: NativePortCore<ProbeBackend<'a>>,
    simulation_binding: Option<rustaxa_evm::simulation::SimulationBinding>,
}

impl<'a> ProbeNativeExecutionPort<'a> {
    /// Takes a qualified detached native probe.
    pub fn new(probe: FinalChainNativeExecutionProbe<'a>) -> Self {
        Self {
            core: NativePortCore::new(ProbeBackend::Pending(probe)),
            simulation_binding: None,
        }
    }

    /// Uses a detached historical session without adding source qualification.
    /// The caller must bind native and delayed physical sources separately.
    /// This constructor does not qualify the pending-period bridge policy.
    pub fn historical(probe: FinalChainNativeSimulation<'a>) -> Self {
        Self {
            core: NativePortCore::new(ProbeBackend::Historical(probe)),
            simulation_binding: None,
        }
    }

    pub(crate) fn bind_finalized_simulation(
        mut self,
        binding: rustaxa_evm::simulation::SimulationBinding,
    ) -> Self {
        self.simulation_binding = Some(binding);
        self
    }

    /// Rechecks the detached owner's descriptor and scheduler before use.
    pub fn validate_context(&mut self) -> Result<(), NativePortError> {
        self.core.validate_context()
    }
}

impl rustaxa_evm::simulation::CorrectedSimulationNative for ProbeNativeExecutionPort<'_> {
    fn simulation_binding(&self) -> &rustaxa_evm::simulation::SimulationBinding {
        self.simulation_binding
            .as_ref()
            .expect("finalized simulation must carry a binding")
    }
}

pub(crate) struct PreparedNativeExecutionPort {
    core: NativePortCore<FinalChainPreparedNativeSession>,
}

impl PreparedNativeExecutionPort {
    pub(crate) fn new(session: FinalChainPreparedNativeSession) -> Self {
        let mut core = NativePortCore::new(session);
        core.invocation_contexts = Some(Vec::new());
        Self { core }
    }

    pub(crate) fn validate_context(&mut self) -> Result<(), NativePortError> {
        self.core.validate_context()
    }

    pub(crate) fn finish(
        self,
        plan: rustaxa_consensus::final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan,
        journal: &dyn NativeJournalRead,
    ) -> Result<
        (
            FinalChainFinishedNativeTransition,
            Vec<FinalChainNativeInvocationContext>,
            FinalChainNativeRewardsContext,
        ),
        NativePortError,
    > {
        if self.core.failed {
            return Err(native_port_error(FinalChainNativeSessionError::Aborted));
        }
        if self.core.pending.is_some() {
            return Err(native_port_error(
                FinalChainNativeSessionError::QuoteOutstanding,
            ));
        }
        let read = RecordingRead::new(journal, RecordedReads::default());
        let transition = self
            .core
            .backend
            .finish_rewards(plan, &read)
            .map_err(native_port_error)?;
        let reads = read.into_reads();
        let reward = transition.rewards();
        let context = FinalChainNativeRewardsContext {
            boundaries: reward.boundaries.clone(),
            accounts: reads.accounts.into_iter().collect(),
            raw_reads: RecordedReads::raw_reads(reads.raw),
            ordinary_mutations: reward.account_mutations.clone(),
            raw_mutations: reward.raw_mutations.clone(),
        };
        Ok((
            transition,
            self.core.invocation_contexts.unwrap_or_default(),
            context,
        ))
    }
}

enum ProbeBackend<'a> {
    Pending(FinalChainNativeExecutionProbe<'a>),
    Historical(FinalChainNativeSimulation<'a>),
}

impl NativeBackend for ProbeBackend<'_> {
    fn validate_context(&mut self) -> Result<(), FinalChainNativeSessionError> {
        match self {
            Self::Pending(probe) => probe.validate_context(),
            Self::Historical(_) => Ok(()),
        }
    }
    fn correction_stage(&self) -> rustaxa_evm::profile::CorrectionStage {
        match self {
            Self::Pending(probe) => probe.correction_stage(),
            Self::Historical(probe) => probe.correction_stage(),
        }
    }
    fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        read: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        match self {
            Self::Pending(probe) => probe.prepare(request, read),
            Self::Historical(probe) => probe.prepare(request, read),
        }
    }
    fn invoke(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        read: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        match self {
            Self::Pending(probe) => probe.invoke(request, quote, read),
            Self::Historical(probe) => probe.invoke(request, quote, read),
        }
    }
    fn input_aliases(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputAlias>,
        FinalChainNativeSessionError,
    > {
        match self {
            Self::Pending(probe) => probe.input_aliases(id),
            Self::Historical(probe) => probe.input_aliases(id),
        }
    }
    fn input_mutations(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputMutation>,
        FinalChainNativeSessionError,
    > {
        match self {
            Self::Pending(probe) => probe.input_mutations(id),
            Self::Historical(probe) => probe.input_mutations(id),
        }
    }
}

impl NativeBackend for FinalChainPreparedNativeSession {
    fn correction_stage(&self) -> rustaxa_evm::profile::CorrectionStage {
        self.correction_stage()
    }
    fn validate_context(&mut self) -> Result<(), FinalChainNativeSessionError> {
        self.validate_context()
    }
    fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        self.prepare(request, state)
    }
    fn invoke(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        self.invoke(request, quote, state)
    }
    fn input_aliases(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputAlias>,
        FinalChainNativeSessionError,
    > {
        self.input_aliases(id)
    }
    fn input_mutations(
        &mut self,
        id: FinalChainNativeInvocationId,
    ) -> Result<
        Vec<rustaxa_consensus::native_session::FinalChainNativeInputMutation>,
        FinalChainNativeSessionError,
    > {
        self.input_mutations(id)
    }
}

impl<B: NativeBackend> NativeExecutionPort for NativePortCore<B> {
    fn correction_stage(&self) -> Option<rustaxa_evm::profile::CorrectionStage> {
        Some(self.backend.correction_stage())
    }

    fn input_aliases(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputAlias>, NativePortError> {
        self.checked(|this| {
            this.backend
                .input_aliases(consensus_id(id))
                .map(|rows| rows.into_iter().map(native_alias).collect())
                .map_err(native_port_error)
        })
    }

    fn input_mutations(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputMutation>, NativePortError> {
        self.checked(|this| {
            this.backend
                .input_mutations(consensus_id(id))
                .map(|rows| rows.into_iter().map(native_input_mutation).collect())
                .map_err(native_port_error)
        })
    }

    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.checked(|this| {
            if this.pending.is_some() {
                return Err(native_port_error(
                    FinalChainNativeSessionError::QuoteOutstanding,
                ));
            }
            let request = consensus_request(invocation);
            let read = RecordingRead::new(journal, RecordedReads::default());
            let quote = this
                .backend
                .prepare(&request, &read)
                .map_err(native_port_error)?;
            let quote = NativeGasQuote {
                invocation: native_id(quote.invocation),
                required_gas: quote.required_gas,
            };
            this.pending = Some(PendingInvocation {
                request,
                quote,
                reads: read.into_reads(),
            });
            Ok(quote)
        })
    }

    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.checked(|this| {
            let request = consensus_request(invocation);
            let pending = this
                .pending
                .take()
                .ok_or_else(|| native_port_error(FinalChainNativeSessionError::NotPrepared))?;
            if pending.request != request || pending.quote != quote {
                return Err(NativePortError::QuoteMismatch);
            }
            let read = RecordingRead::new(journal, pending.reads);
            read.validate_previous().map_err(|error| {
                native_port_error(FinalChainNativeSessionError::StateRead(error))
            })?;
            let result = this
                .backend
                .invoke(
                    &request,
                    FinalChainNativeGasQuote {
                        invocation: consensus_id(quote.invocation),
                        required_gas: quote.required_gas,
                    },
                    &read,
                )
                .map_err(native_port_error)?;
            if let Some(contexts) = &mut this.invocation_contexts {
                let (ordinary_mutations, raw_mutations) = match &result {
                    FinalChainNativeInvocationResult::InsufficientGas { .. } => {
                        (Vec::new(), Vec::new())
                    }
                    FinalChainNativeInvocationResult::Completed(outcome) => (
                        outcome.account_mutations.clone(),
                        outcome.raw_mutations.clone(),
                    ),
                };
                let reads = read.into_reads();
                contexts.push(FinalChainNativeInvocationContext {
                    request,
                    accounts: reads.accounts.into_iter().collect(),
                    raw_reads: RecordedReads::raw_reads(reads.raw),
                    ordinary_mutations,
                    raw_mutations,
                });
            }
            match result {
                FinalChainNativeInvocationResult::InsufficientGas { required_gas } => {
                    Ok(NativeInvocationResult::InsufficientGas { required_gas })
                }
                FinalChainNativeInvocationResult::Completed(outcome) => {
                    Ok(NativeInvocationResult::Completed(native_outcome(&outcome)?))
                }
            }
        })
    }
}

impl NativeExecutionPort for ProbeNativeExecutionPort<'_> {
    fn correction_stage(&self) -> Option<rustaxa_evm::profile::CorrectionStage> {
        self.core.correction_stage()
    }
    fn input_aliases(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputAlias>, NativePortError> {
        self.core.input_aliases(id)
    }
    fn input_mutations(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputMutation>, NativePortError> {
        self.core.input_mutations(id)
    }
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.core.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.core.invoke(invocation, quote, journal)
    }
}

impl NativeExecutionPort for PreparedNativeExecutionPort {
    fn correction_stage(&self) -> Option<rustaxa_evm::profile::CorrectionStage> {
        self.core.correction_stage()
    }
    fn input_aliases(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputAlias>, NativePortError> {
        self.core.input_aliases(id)
    }
    fn input_mutations(
        &mut self,
        id: NativeInvocationId,
    ) -> Result<Vec<rustaxa_evm::contracts::NativeInputMutation>, NativePortError> {
        self.core.input_mutations(id)
    }
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.core.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.core.invoke(invocation, quote, journal)
    }
}

fn native_alias(
    alias: rustaxa_consensus::native_session::FinalChainNativeInputAlias,
) -> rustaxa_evm::contracts::NativeInputAlias {
    use rustaxa_consensus::native_session::FinalChainNativeInputAliasScope as C;
    use rustaxa_evm::contracts::NativeInputAliasScope as E;
    rustaxa_evm::contracts::NativeInputAlias {
        scope: match alias.scope {
            C::OwnedInputOnly => E::OwnedInputOnly,
            C::SharedJournal => E::SharedJournal,
        },
        address: alias.address,
        key: alias.key,
        offset: alias.offset,
        length: alias.length,
    }
}

fn native_input_mutation(
    mutation: rustaxa_consensus::native_session::FinalChainNativeInputMutation,
) -> rustaxa_evm::contracts::NativeInputMutation {
    rustaxa_evm::contracts::NativeInputMutation {
        offset: mutation.offset,
        expected: mutation.expected,
        replacement: mutation.replacement,
    }
}

fn consensus_request(invocation: &NativeInvocation) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: consensus_id(invocation.id),
        period: invocation.period,
        depth: invocation.depth,
        kind: match invocation.kind {
            NativeCallKind::Call => FinalChainNativeCallKind::Call,
            NativeCallKind::CallCode => FinalChainNativeCallKind::CallCode,
            NativeCallKind::DelegateCall => FinalChainNativeCallKind::DelegateCall,
            NativeCallKind::StaticCall => FinalChainNativeCallKind::StaticCall,
        },
        is_static: invocation.is_static,
        caller: invocation.caller,
        contract: invocation.contract,
        state_address: invocation.state_address,
        value: FinalChainNativeValue::new(invocation.value.value().clone()),
        input: invocation.input.clone(),
        supplied_gas: invocation.supplied_gas,
    }
}

fn consensus_id(id: NativeInvocationId) -> FinalChainNativeInvocationId {
    FinalChainNativeInvocationId {
        transaction: id.transaction,
        sequence: id.sequence,
    }
}

fn native_id(id: FinalChainNativeInvocationId) -> NativeInvocationId {
    NativeInvocationId {
        transaction: id.transaction,
        sequence: id.sequence,
    }
}

fn native_outcome(outcome: &FinalChainNativeOutcome) -> Result<NativeOutcome, NativePortError> {
    Ok(NativeOutcome {
        status: native_status(&outcome.status),
        gas_used: outcome.gas_used,
        output: outcome.output.clone(),
        account_mutations: ordinary_mutations(&outcome.account_mutations),
        raw_mutations: raw_mutations(&outcome.raw_mutations)?,
        logs: outcome
            .logs
            .iter()
            .map(|log| ExecutionLog {
                address: log.address,
                topics: log.topics.clone(),
                data: log.data.clone(),
            })
            .collect(),
        diagnostic: None,
    })
}

fn native_status(status: &FinalChainNativeStatus) -> NativeStatus {
    match status {
        FinalChainNativeStatus::Success => NativeStatus::Success,
        FinalChainNativeStatus::ContractFailure { error } => {
            NativeStatus::ContractFailure(NativeContractFailure {
                error: error.clone(),
            })
        }
    }
}

pub(crate) fn ordinary_mutations(
    mutations: &[FinalChainNativeOrdinaryMutation],
) -> Vec<NativeOrdinaryAccountMutation> {
    mutations
        .iter()
        .map(|mutation| match mutation {
            FinalChainNativeOrdinaryMutation::NonceReplace {
                address,
                expected_exists,
                expected,
                replacement,
            } => NativeOrdinaryAccountMutation::Nonce {
                address: *address,
                expected_exists: *expected_exists,
                expected: expected.clone(),
                replacement: replacement.clone(),
            },
            FinalChainNativeOrdinaryMutation::EnsureExists {
                address,
                expected_exists,
            } => NativeOrdinaryAccountMutation::EnsureExists {
                address: *address,
                expected_exists: *expected_exists,
            },
            FinalChainNativeOrdinaryMutation::Touch {
                address,
                expected_exists,
            } => NativeOrdinaryAccountMutation::Touch {
                address: *address,
                expected_exists: *expected_exists,
            },
            FinalChainNativeOrdinaryMutation::BalanceReplace {
                address,
                expected_exists,
                expected,
                replacement,
            } => NativeOrdinaryAccountMutation::Balance {
                address: *address,
                expected_exists: *expected_exists,
                expected: ExecutionBalance::new(expected.clone()),
                replacement: ExecutionBalance::new(replacement.clone()),
            },
        })
        .collect()
}

pub(crate) fn raw_mutations(
    mutations: &[FinalChainNativeRawMutation],
) -> Result<Vec<NativeRawMutation>, NativePortError> {
    mutations
        .iter()
        .map(|mutation| {
            let operation = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => NativeRawOperation::Put(
                    NativeRawValue::new(value.as_bytes().to_vec())
                        .map_err(|error| NativePortError::Infrastructure(error.to_string()))?,
                ),
                FinalChainNativeRawOperation::Delete => NativeRawOperation::Delete,
            };
            Ok(NativeRawMutation {
                address: mutation.address,
                key: mutation.key,
                expected: mutation.expected.clone(),
                operation,
            })
        })
        .collect()
}

fn state_read_error(error: NativeJournalReadError) -> FinalChainNativeStateReadError {
    match error {
        NativeJournalReadError::State(error) => FinalChainNativeStateReadError::State(error),
        NativeJournalReadError::Invariant(error) => {
            FinalChainNativeStateReadError::Invariant(error)
        }
    }
}

fn native_port_error(error: FinalChainNativeSessionError) -> NativePortError {
    match error {
        FinalChainNativeSessionError::OutOfSequence { expected, actual } => {
            NativePortError::OutOfSequence { expected, actual }
        }
        FinalChainNativeSessionError::QuoteMismatch => NativePortError::QuoteMismatch,
        FinalChainNativeSessionError::StateRead(error) => NativePortError::Journal(match error {
            FinalChainNativeStateReadError::State(error) => NativeJournalReadError::State(error),
            FinalChainNativeStateReadError::Invariant(error) => {
                NativeJournalReadError::Invariant(error)
            }
        }),
        FinalChainNativeSessionError::MalformedNestedSlashingProof(error) => {
            NativePortError::MalformedNestedSlashingProof(error)
        }
        error => NativePortError::Domain(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use rustaxa_types::{FinalChainGas, FinalChainNonce};

    struct ChangingJournal {
        account_reads: RefCell<Vec<rustaxa_evm::contracts::NativeJournalAccount>>,
        raw_reads: RefCell<Vec<ConcreteRead<Vec<u8>>>>,
    }

    impl NativeJournalRead for ChangingJournal {
        fn account(
            &self,
            _address: [u8; 20],
        ) -> Result<rustaxa_evm::contracts::NativeJournalAccount, NativeJournalReadError> {
            self.account_reads
                .borrow_mut()
                .pop()
                .ok_or_else(|| NativeJournalReadError::Invariant("missing account read".into()))
        }

        fn raw_storage(
            &self,
            _address: [u8; 20],
            _key: &ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
            self.raw_reads
                .borrow_mut()
                .pop()
                .ok_or_else(|| NativeJournalReadError::Invariant("missing raw read".into()))
        }
    }

    fn journal_account(balance: i64) -> rustaxa_evm::contracts::NativeJournalAccount {
        rustaxa_evm::contracts::NativeJournalAccount {
            exists: true,
            nonce: FinalChainNonce::from_u64(3),
            balance: ExecutionBalance::new(BigInt::from(balance)),
        }
    }

    #[test]
    fn recorder_rejects_changed_account_and_raw_repeats() {
        let journal = ChangingJournal {
            account_reads: RefCell::new(vec![journal_account(8), journal_account(7)]),
            raw_reads: RefCell::new(vec![
                ConcreteRead::Tombstone,
                ConcreteRead::Present(Vec::new()),
            ]),
        };
        let read = RecordingRead::new(&journal, RecordedReads::default());
        assert_eq!(read.account([1; 20]).unwrap().balance, BigInt::from(7));
        assert!(matches!(
            read.account([1; 20]),
            Err(FinalChainNativeStateReadError::Invariant(error))
                if error.contains("account read changed")
        ));
        let key = ConcreteStorageKey([2; 32]);
        assert_eq!(
            read.raw_storage([3; 20], &key).unwrap(),
            ConcreteRead::Present(Vec::new())
        );
        assert!(matches!(
            read.raw_storage([3; 20], &key),
            Err(FinalChainNativeStateReadError::Invariant(error))
                if error.contains("raw read changed")
        ));
    }

    #[test]
    fn prior_read_validation_rejects_change_before_native_invoke() {
        let journal = ChangingJournal {
            account_reads: RefCell::new(vec![journal_account(8), journal_account(7)]),
            raw_reads: RefCell::new(vec![]),
        };
        let prepare = RecordingRead::new(&journal, RecordedReads::default());
        prepare.account([1; 20]).unwrap();
        let invoke = RecordingRead::new(&journal, prepare.into_reads());
        assert!(matches!(
            invoke.validate_previous(),
            Err(FinalChainNativeStateReadError::Invariant(_))
        ));
    }

    #[test]
    fn malformed_proof_mapping_does_not_classify_diagnostic_text() {
        let text = "malformed nested slashing proof RLP".to_owned();
        assert_eq!(
            native_port_error(FinalChainNativeSessionError::MalformedNestedSlashingProof(
                text.clone()
            )),
            NativePortError::MalformedNestedSlashingProof(text.clone()),
        );
        assert!(matches!(
            native_port_error(FinalChainNativeSessionError::Domain(text.clone())),
            NativePortError::Domain(_)
        ));
        assert!(matches!(
            native_port_error(FinalChainNativeSessionError::RawIntegrity(text)),
            NativePortError::Domain(_)
        ));
        assert!(matches!(
            native_port_error(FinalChainNativeSessionError::StateRead(
                FinalChainNativeStateReadError::Invariant("read failed".into())
            )),
            NativePortError::Journal(_)
        ));
    }

    #[test]
    fn input_conversion_keeps_alias_scope_and_mutation_bytes() {
        use rustaxa_consensus::native_session::{
            FinalChainNativeInputAlias, FinalChainNativeInputAliasScope,
            FinalChainNativeInputMutation,
        };
        for (scope, expected_scope) in [
            (
                FinalChainNativeInputAliasScope::OwnedInputOnly,
                rustaxa_evm::contracts::NativeInputAliasScope::OwnedInputOnly,
            ),
            (
                FinalChainNativeInputAliasScope::SharedJournal,
                rustaxa_evm::contracts::NativeInputAliasScope::SharedJournal,
            ),
        ] {
            let converted = native_alias(FinalChainNativeInputAlias {
                scope,
                address: [3; 20],
                key: ConcreteStorageKey([4; 32]),
                offset: 17,
                length: 31,
            });
            assert_eq!(converted.scope, expected_scope);
            assert_eq!(converted.address, [3; 20]);
            assert_eq!(converted.key, ConcreteStorageKey([4; 32]));
            assert_eq!((converted.offset, converted.length), (17, 31));
        }
        let converted = native_input_mutation(FinalChainNativeInputMutation {
            offset: 41,
            expected: 0xff,
            replacement: 0,
        });
        assert_eq!(
            (converted.offset, converted.expected, converted.replacement),
            (41, 0xff, 0)
        );
    }

    #[test]
    fn raw_conversion_keeps_repeated_key_order_and_exact_bytes() {
        let key = ConcreteStorageKey([7; 32]);
        let rows = vec![
            FinalChainNativeRawMutation {
                address: [5; 20],
                key,
                expected: ConcreteRead::Present(vec![1]),
                operation: FinalChainNativeRawOperation::Delete,
            },
            FinalChainNativeRawMutation {
                address: [5; 20],
                key,
                expected: ConcreteRead::Tombstone,
                operation: FinalChainNativeRawOperation::Put(
                    rustaxa_consensus::native_session::FinalChainNativeRawValue::new(vec![0, 9])
                        .unwrap(),
                ),
            },
        ];
        let converted = raw_mutations(&rows).unwrap();
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].key, converted[1].key);
        assert!(matches!(converted[0].operation, NativeRawOperation::Delete));
        assert!(
            matches!(&converted[1].operation, NativeRawOperation::Put(value) if value.as_bytes() == [0, 9])
        );
    }

    #[test]
    fn conversion_preserves_signed_balances_and_raw_classification() {
        let wide = -(BigInt::from(1_u8) << 300_usize) + BigInt::from(5_u8);
        let outcome = FinalChainNativeOutcome {
            status: FinalChainNativeStatus::Success,
            gas_used: FinalChainGas::new(9),
            output: vec![1],
            account_mutations: vec![
                FinalChainNativeOrdinaryMutation::EnsureExists {
                    address: [3; 20],
                    expected_exists: false,
                },
                FinalChainNativeOrdinaryMutation::Touch {
                    address: [4; 20],
                    expected_exists: true,
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: [4; 20],
                    expected_exists: true,
                    expected: wide.clone(),
                    replacement: &wide + BigInt::from(11_u8),
                },
            ],
            raw_mutations: vec![
                FinalChainNativeRawMutation {
                    address: [5; 20],
                    key: ConcreteStorageKey([6; 32]),
                    expected: ConcreteRead::Tombstone,
                    operation: FinalChainNativeRawOperation::Delete,
                },
                FinalChainNativeRawMutation {
                    address: [5; 20],
                    key: ConcreteStorageKey([7; 32]),
                    expected: ConcreteRead::Present(Vec::new()),
                    operation: FinalChainNativeRawOperation::Put(
                        rustaxa_consensus::native_session::FinalChainNativeRawValue::new(vec![
                            0, 9,
                        ])
                        .unwrap(),
                    ),
                },
            ],
            logs: Vec::new(),
        };
        let converted = native_outcome(&outcome).unwrap();
        assert_eq!(converted.gas_used, FinalChainGas::new(9));
        assert!(matches!(
            &converted.account_mutations[2],
            NativeOrdinaryAccountMutation::Balance {
                expected,
                replacement,
                ..
            } if expected.value() == &wide
                && replacement.value() == &(&wide + BigInt::from(11_u8))
        ));
        assert!(matches!(
            &converted.raw_mutations[0],
            NativeRawMutation {
                expected: ConcreteRead::Tombstone,
                operation: NativeRawOperation::Delete,
                ..
            }
        ));
        assert!(matches!(
            &converted.account_mutations[..2],
            [
                NativeOrdinaryAccountMutation::EnsureExists {
                    expected_exists: false,
                    ..
                },
                NativeOrdinaryAccountMutation::Touch {
                    expected_exists: true,
                    ..
                }
            ]
        ));
        assert!(matches!(
            &converted.raw_mutations[1],
            NativeRawMutation {
                expected: ConcreteRead::Present(empty),
                operation: NativeRawOperation::Put(value),
                ..
            } if empty.is_empty() && value.as_bytes() == [0, 9]
        ));
    }
}
