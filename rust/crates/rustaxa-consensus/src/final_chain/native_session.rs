//! Ordered, unpublished execution of selected FinalChain native mutations.
//!
//! This module exposes a consensus-owned staged session for the post-Cornus
//! `setCommission`, `delegate`, `undelegateV2`, and `confirmUndelegateV2` DPoS
//! operations. The session reuses the existing decoder, gas policy, and
//! mutation kernels. It binds each quote to an exact period-local invocation
//! and validates operation-owned raw rows before advancing its private DPoS
//! snapshot. Transaction fees, CALL value transfer, nonces, frame rollback,
//! receipts, and publication remain outside this boundary.

use super::*;
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStorageKey};

pub(super) mod account;
pub(super) mod context_replay;
pub(super) mod custody;
pub(super) mod raw;
pub(super) mod rewards;

pub use account::{FinalChainNativeAccount, FinalChainNativeOrdinaryMutation};
pub use raw::{
    FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRawValue,
    FinalChainNativeRawValueError,
};
pub use rewards::FinalChainNativeRewardsOutcome;

/// Arbitrary-width unsigned value carried by a native child call.
///
/// Payability and admission checks use the complete value. `delegate` passes
/// these exact bytes to the existing DPoS kernel; values outside that kernel's
/// native `u256` domain fail explicitly instead of being narrowed.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainNativeValue(BigUint);

impl FinalChainNativeValue {
    /// Wraps an arbitrary-width unsigned call value.
    pub fn new(value: BigUint) -> Self {
        Self(value)
    }

    /// Borrows the complete unsigned value.
    pub fn value(&self) -> &BigUint {
        &self.0
    }

    /// Reports whether the complete value is zero.
    pub fn is_zero(&self) -> bool {
        self.0 == BigUint::default()
    }
}

/// Monotonic identity of one native call in a pending period.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FinalChainNativeInvocationId {
    /// Ordered transaction containing the call.
    pub transaction: FinalChainTransactionPosition,
    /// Zero-based native-port call sequence across the pending period.
    pub sequence: u64,
}

/// CALL-family operation presented to the FinalChain native session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinalChainNativeCallKind {
    /// Ordinary CALL.
    Call,
    /// CALLCODE, currently rejected by this bounded session.
    CallCode,
    /// DELEGATECALL, currently rejected by this bounded session.
    DelegateCall,
    /// STATICCALL, including the historical native-mutation exception.
    StaticCall,
}

/// Immutable facts bound by native preparation and invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRequest {
    /// Monotonic pending-period invocation identity.
    pub id: FinalChainNativeInvocationId,
    /// Pending FinalChain period whose native rules apply.
    pub period: FinalChainBlockNumber,
    /// Zero-based EVM frame depth, retained in the exact request binding.
    pub depth: u16,
    /// CALL-family operation used to reach native code.
    pub kind: FinalChainNativeCallKind,
    /// Effective static mode inherited from enclosing frames.
    pub is_static: bool,
    /// Effective native method caller.
    pub caller: [u8; 20],
    /// Native code address selected by dispatch.
    pub contract: [u8; 20],
    /// Account whose state is the call context.
    pub state_address: [u8; 20],
    /// Complete unsigned CALL value.
    pub value: FinalChainNativeValue,
    /// Exact ABI input.
    pub input: Vec<u8>,
    /// Child/action gas supplied by the frame driver.
    pub supplied_gas: FinalChainGas,
}

/// Action-gas requirement bound to one exact native request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FinalChainNativeGasQuote {
    /// Invocation whose preparation produced this quote.
    pub invocation: FinalChainNativeInvocationId,
    /// Native action gas required for invocation.
    pub required_gas: FinalChainGas,
}

/// Exact current-state access required by the bounded native session.
///
/// Implementations expose authoritative ordinary account facts and the current
/// raw lane, including earlier same-period native puts and tombstones. Reads
/// must observe all preceding journal effects. Errors abort the pending period;
/// they are never converted to absence.
pub trait FinalChainNativeStateRead {
    /// Loads exact current ordinary-account facts for one native kernel read.
    ///
    /// The default keeps the original `setCommission`-only readers source
    /// compatible. Account-consuming operations fail explicitly instead of
    /// interpreting an unavailable read as an absent account.
    fn account(
        &self,
        _address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        Err(FinalChainNativeStateReadError::Invariant(
            "current native account reads are unavailable".to_owned(),
        ))
    }

    /// Loads one native logical storage key from the current raw lane.
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError>;
}

/// Failure to read authoritative current state for native preparation or invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeStateReadError {
    /// The immutable concrete-state fallback failed.
    State(ConcreteReadError),
    /// Current journal overlays or lifecycle facts are inconsistent.
    Invariant(String),
}

impl std::fmt::Display for FinalChainNativeStateReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "FinalChain native state read: {self:?}")
    }
}

impl std::error::Error for FinalChainNativeStateReadError {}

impl From<ConcreteReadError> for FinalChainNativeStateReadError {
    fn from(error: ConcreteReadError) -> Self {
        Self::State(error)
    }
}

/// Contract-level status of one admitted native operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeStatus {
    /// The business mutation succeeded.
    Success,
    /// The native contract rejected the operation under normal semantics.
    ContractFailure {
        /// Exact legacy diagnostic exposed by native call execution.
        error: String,
    },
}

/// Complete result of an admitted native operation.
///
/// Ordered raw mutations survive ordinary EVM frame rollback. Account
/// mutations and logs belong to the enclosing ordinary frame and are discarded
/// when that frame reverts. Business failure exposes none of these effects.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeOutcome {
    /// Business status returned by the reused FinalChain kernel.
    pub status: FinalChainNativeStatus,
    /// Native action gas charged by the admitted call.
    pub gas_used: FinalChainGas,
    /// Native return bytes.
    pub output: Vec<u8>,
    /// Ordered ordinary account effects owned by the enclosing EVM frame.
    pub account_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Ordered exact raw replacements. This is empty on business failure.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
    /// Ordered EVM-compatible logs. These are empty on business failure.
    pub logs: Vec<FinalChainCallLog>,
}

/// Terminal result of the invocation phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeInvocationResult {
    /// Supplied child gas was below the prepared requirement. No kernel ran and
    /// no effect was produced.
    InsufficientGas {
        /// Required native gas returned to the frame driver.
        required_gas: FinalChainGas,
    },
    /// The request completed as a success or ordinary contract failure.
    Completed(FinalChainNativeOutcome),
}

/// Integrity or sequencing failure at the staged native boundary.
///
/// State-read, raw-integrity, kernel and serializer failures poison the session
/// and expose no partial semantic or raw mutation. Request/order/unsupported
/// errors occur before preparation. A quote mismatch retains the original
/// preparation so its exact request can still invoke it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeSessionError {
    /// A prior integrity or infrastructure failure invalidated the session.
    Aborted,
    /// The requested parent is not the currently selected finalized head.
    ParentMismatch {
        /// Finalized parent requested by the caller.
        expected: FinalChainBlockNumber,
        /// Current finalized head observed by FinalChain.
        actual: FinalChainBlockNumber,
    },
    /// The pending period is not exactly one after its finalized parent.
    PendingPeriodMismatch,
    /// This first adapter does not implement pre-Cornus native behavior.
    PreCornusUnsupported,
    /// A request targeted a different pending period.
    PeriodMismatch,
    /// The request sequence is not the next session sequence.
    OutOfSequence {
        /// Sequence expected by the session.
        expected: u64,
        /// Sequence supplied by the request.
        actual: u64,
    },
    /// Preparation was attempted while a prior quote remained outstanding.
    QuoteOutstanding,
    /// Invocation was attempted without a prepared request.
    NotPrepared,
    /// Invocation request or quote differs from the exact prepared values.
    QuoteMismatch,
    /// This bounded session does not support the requested CALL-family kind.
    UnsupportedCallKind,
    /// The code or state address is outside the bounded DPoS route.
    UnsupportedAddress,
    /// The ABI input is outside the selected staged operation set.
    UnsupportedOperation,
    /// Authoritative current state could not be read safely.
    StateRead(FinalChainNativeStateReadError),
    /// Rewards require the request identity retained by the bound constructor.
    UnboundRewards,
    /// The opaque rewards plan belongs to another request or period.
    RewardsPlanMismatch,
    /// The terminal rewards phase was already completed.
    RewardsAlreadyFinished,
    /// The prepared rewards profile is outside this first bounded adapter.
    RewardsScopeUnsupported,
    /// The selected custody transition is outside this first bounded adapter.
    CustodyScopeUnsupported,
    /// Current raw bytes do not exactly encode the staged semantic state.
    RawIntegrity(String),
    /// Existing FinalChain state or kernel execution failed.
    Domain(String),
}

impl std::fmt::Display for FinalChainNativeSessionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "FinalChain native session: {self:?}")
    }
}

impl std::error::Error for FinalChainNativeSessionError {}

impl From<FinalChainNativeStateReadError> for FinalChainNativeSessionError {
    fn from(error: FinalChainNativeStateReadError) -> Self {
        Self::StateRead(error)
    }
}

#[derive(Clone)]
struct PreparedSetCommission {
    transaction: DposTransaction,
    validator: [u8; 20],
    validator_key: ConcreteStorageKey,
    validator_read: ConcreteRead<Vec<u8>>,
    owner_key: ConcreteStorageKey,
    owner_read: ConcreteRead<Vec<u8>>,
}

#[derive(Clone)]
enum PreparedKind {
    NestedCallRejected,
    NonPayable,
    InsufficientGas,
    SetCommission(Box<PreparedSetCommission>),
    SelectedCustody(DposTransaction),
}

#[derive(Clone)]
struct PreparedCall {
    request: FinalChainNativeRequest,
    quote: FinalChainNativeGasQuote,
    kind: PreparedKind,
}

/// Single-period staged native kernel state.
///
/// The session begins from an exact finalized DPoS snapshot, advances the
/// reward-reference graph to the pending period, and never publishes its
/// state. Successful raw mutations advance this state even when a surrounding
/// ordinary EVM frame later rolls back.
pub struct FinalChainNativeSession<'a> {
    final_chain: &'a FinalChain,
    request_id: Option<[u8; 32]>,
    pending_period: FinalChainBlockNumber,
    period_start_total_vote_count: u64,
    period_start_amount_delegated: U256,
    next_sequence: u64,
    dpos_state: DposSnapshot,
    prepared: Option<PreparedCall>,
    aborted: bool,
    finished_rewards: bool,
}

impl FinalChain {
    /// Begins an unpublished native session for the block immediately after an
    /// exact currently finalized parent.
    ///
    /// The call rejects stale parents, skipped periods, and pre-Cornus periods.
    /// It clones and advances DPoS state but does not acquire publication
    /// authority or alter FinalChain snapshots.
    pub fn begin_native_session(
        &self,
        pending_period: FinalChainBlockNumber,
        expected_parent: FinalChainBlockNumber,
    ) -> std::result::Result<FinalChainNativeSession<'_>, FinalChainNativeSessionError> {
        self.begin_native_session_inner(None, pending_period, expected_parent)
    }

    /// Begins an unpublished native session bound to one exact external-EVM
    /// application request.
    ///
    /// Invocation execution is identical to [`Self::begin_native_session`]. The
    /// non-optional binding is retained for the terminal rewards phase, which
    /// rejects a plan prepared for another request.
    pub fn begin_native_session_bound(
        &self,
        request_id: [u8; 32],
        pending_period: FinalChainBlockNumber,
        expected_parent: FinalChainBlockNumber,
    ) -> std::result::Result<FinalChainNativeSession<'_>, FinalChainNativeSessionError> {
        self.begin_native_session_inner(Some(request_id), pending_period, expected_parent)
    }

    fn begin_native_session_inner(
        &self,
        request_id: Option<[u8; 32]>,
        pending_period: FinalChainBlockNumber,
        expected_parent: FinalChainBlockNumber,
    ) -> std::result::Result<FinalChainNativeSession<'_>, FinalChainNativeSessionError> {
        let actual_parent = self
            .last_block_number_typed()
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        if actual_parent != expected_parent {
            return Err(FinalChainNativeSessionError::ParentMismatch {
                expected: expected_parent,
                actual: actual_parent,
            });
        }
        if expected_parent.checked_next() != Some(pending_period) {
            return Err(FinalChainNativeSessionError::PendingPeriodMismatch);
        }
        if pending_period < self.dpos_cornus_period {
            return Err(FinalChainNativeSessionError::PreCornusUnsupported);
        }
        let mut dpos_state = self
            .dpos_snapshot_at_finalized_block(expected_parent)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        self.advance_reward_reference_graph_block(&mut dpos_state, pending_period)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let period_start_total_vote_count = dpos_state.total_vote_count;
        let period_start_amount_delegated = total_staked_amount(&dpos_state)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        Ok(FinalChainNativeSession {
            final_chain: self,
            request_id,
            pending_period,
            period_start_total_vote_count,
            period_start_amount_delegated,
            next_sequence: 0,
            dpos_state,
            prepared: None,
            aborted: false,
            finished_rewards: false,
        })
    }
}

impl FinalChainNativeSession<'_> {
    /// Prepares one exact supported native call and returns its gas quote.
    ///
    /// Nonzero value and insufficient-gas paths bind a terminal result before
    /// any raw read. `setCommission` validates and retains its validator and
    /// owner observations during preparation. Selected custody calls defer raw
    /// and ordinary-account reads until invocation so business preflight errors
    /// preserve the reference read order.
    pub fn prepare(
        &mut self,
        request: &FinalChainNativeRequest,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        if self.aborted {
            return Err(FinalChainNativeSessionError::Aborted);
        }
        if self.finished_rewards {
            return Err(FinalChainNativeSessionError::RewardsAlreadyFinished);
        }
        if self.prepared.is_some() {
            return Err(FinalChainNativeSessionError::QuoteOutstanding);
        }
        self.validate_request(request)?;

        let mut transaction = decode_dpos_transaction_for_execution(
            &request.input,
            request.caller,
            request.period,
            self.final_chain.rewards_config.fix_claim_all_block_num,
            self.final_chain.dpos_cornus_period,
            self.final_chain.rewards_config.phalaenopsis_period,
        );
        let validator = match &mut transaction {
            DposTransaction::SetCommission { validator, .. } => Some(*validator),
            DposTransaction::Delegate { amount, .. } => {
                *amount = request.value.value().to_bytes_be();
                None
            }
            DposTransaction::UndelegateV2 { .. } | DposTransaction::ConfirmUndelegateV2 { .. } => {
                None
            }
            _ => return Err(FinalChainNativeSessionError::UnsupportedOperation),
        };

        let admission = match self.final_chain.native_invocation_admission(
            &transaction,
            request.period,
            request.depth,
            request.value.value(),
            request.supplied_gas,
            None,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.aborted = true;
                return Err(FinalChainNativeSessionError::Domain(error.to_string()));
            }
        };
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: admission.required_gas,
        };
        if let Some(failure) = admission.failure {
            use super::native_admission::NativeAdmissionFailure as Failure;
            self.prepared = Some(PreparedCall {
                request: request.clone(),
                quote,
                kind: match failure {
                    Failure::InsufficientGas => PreparedKind::InsufficientGas,
                    Failure::NestedBeforeFix => PreparedKind::NestedCallRejected,
                    Failure::NonPayable => PreparedKind::NonPayable,
                },
            });
            return Ok(quote);
        }

        let Some(validator) = validator else {
            self.prepared = Some(PreparedCall {
                request: request.clone(),
                quote,
                kind: PreparedKind::SelectedCustody(transaction),
            });
            return Ok(quote);
        };
        let validator_key = ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &validator]));
        let owner_key = ConcreteStorageKey(concrete_storage_key(&[&[0, 3], &validator]));
        let validation = (|| {
            let validator_read = state.raw_storage(DPOS_CONTRACT_ADDRESS, &validator_key)?;
            let owner_read = state.raw_storage(DPOS_CONTRACT_ADDRESS, &owner_key)?;
            self.validate_raw_validator(validator, &validator_read)?;
            self.validate_raw_owner(validator, &owner_read)?;
            Ok::<_, FinalChainNativeSessionError>((validator_read, owner_read))
        })();
        let (validator_read, owner_read) = match validation {
            Ok(reads) => reads,
            Err(error) => {
                self.aborted = true;
                return Err(error);
            }
        };

        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind: PreparedKind::SetCommission(Box::new(PreparedSetCommission {
                transaction,
                validator,
                validator_key,
                validator_read,
                owner_key,
                owner_read,
            })),
        });
        Ok(quote)
    }

    /// Invokes the one exact prepared call.
    ///
    /// A mismatched request or quote leaves the valid preparation available for
    /// its original invocation. Normal terminal results consume the quote and
    /// sequence. Admitted execution runs the selected existing kernel against a
    /// cloned DPoS snapshot and invocation-local account overlay. The session
    /// advances semantic state only after exact ordered raw serialization
    /// succeeds; infrastructure or integrity failure poisons the session.
    pub fn invoke(
        &mut self,
        request: &FinalChainNativeRequest,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        if self.aborted {
            return Err(FinalChainNativeSessionError::Aborted);
        }
        if self.finished_rewards {
            return Err(FinalChainNativeSessionError::RewardsAlreadyFinished);
        }
        let Some(prepared) = self.prepared.as_ref() else {
            return Err(FinalChainNativeSessionError::NotPrepared);
        };
        if prepared.request != *request || prepared.quote != quote {
            return Err(FinalChainNativeSessionError::QuoteMismatch);
        }
        let Some(next_sequence) = self.next_sequence.checked_add(1) else {
            self.prepared = None;
            self.aborted = true;
            return Err(FinalChainNativeSessionError::Domain(
                "native invocation sequence overflow".to_owned(),
            ));
        };
        let prepared = prepared.clone();

        let result = match prepared.kind {
            PreparedKind::NestedCallRejected => Ok(FinalChainNativeInvocationResult::Completed(
                FinalChainNativeOutcome {
                    status: FinalChainNativeStatus::ContractFailure {
                        error: "only top-level calls are allowed".to_owned(),
                    },
                    gas_used: quote.required_gas,
                    output: Vec::new(),
                    account_mutations: Vec::new(),
                    raw_mutations: Vec::new(),
                    logs: Vec::new(),
                },
            )),
            PreparedKind::NonPayable => Ok(FinalChainNativeInvocationResult::Completed(
                FinalChainNativeOutcome {
                    status: FinalChainNativeStatus::ContractFailure {
                        error: "Method is not payable".to_owned(),
                    },
                    gas_used: FinalChainGas::ZERO,
                    output: Vec::new(),
                    account_mutations: Vec::new(),
                    raw_mutations: Vec::new(),
                    logs: Vec::new(),
                },
            )),
            PreparedKind::InsufficientGas => {
                Ok(FinalChainNativeInvocationResult::InsufficientGas {
                    required_gas: quote.required_gas,
                })
            }
            PreparedKind::SetCommission(prepared) => {
                self.invoke_set_commission(prepared, quote, state)
            }
            PreparedKind::SelectedCustody(transaction) => {
                self.invoke_selected_custody(transaction, quote, state)
            }
        };
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                self.prepared = None;
                self.aborted = true;
                return Err(error);
            }
        };
        self.prepared = None;
        self.next_sequence = next_sequence;
        Ok(result)
    }

    fn invoke_set_commission(
        &mut self,
        prepared: Box<PreparedSetCommission>,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> std::result::Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let current_validator =
            state.raw_storage(DPOS_CONTRACT_ADDRESS, &prepared.validator_key)?;
        let current_owner = state.raw_storage(DPOS_CONTRACT_ADDRESS, &prepared.owner_key)?;
        if current_validator != prepared.validator_read || current_owner != prepared.owner_read {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission raw observations changed after preparation".to_owned(),
            ));
        }

        let mut next_state = self.dpos_state.clone();
        let outcome = self
            .final_chain
            .apply_dpos_mutation_transaction(
                self.pending_period,
                prepared.transaction,
                &mut next_state,
                &mut HashMap::new(),
            )
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let status = if outcome.status_code == 1 {
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: outcome
                    .contract_error
                    .as_ref()
                    .map(DposContractError::legacy_message)
                    .unwrap_or_default(),
            }
        };
        let mut raw_mutations = Vec::new();
        if outcome.status_code == 1 {
            let replacement = self.encode_validator_row(&next_state, prepared.validator)?;
            if replacement.is_empty() {
                return Err(FinalChainNativeSessionError::Domain(
                    "setCommission serializer produced an empty put".to_owned(),
                ));
            }
            raw_mutations.push(FinalChainNativeRawMutation {
                address: DPOS_CONTRACT_ADDRESS,
                key: prepared.validator_key,
                expected: prepared.validator_read,
                operation: FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(replacement)
                        .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?,
                ),
            });
            self.dpos_state = next_state;
        }
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status,
                gas_used: quote.required_gas,
                output: outcome.code_retval,
                account_mutations: Vec::new(),
                raw_mutations,
                logs: outcome
                    .logs
                    .into_iter()
                    .map(|log| FinalChainCallLog {
                        address: log.address,
                        topics: log.topics,
                        data: log.data,
                    })
                    .collect(),
            },
        ))
    }

    fn validate_request(
        &self,
        request: &FinalChainNativeRequest,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        if request.period != self.pending_period {
            return Err(FinalChainNativeSessionError::PeriodMismatch);
        }
        if request.id.sequence != self.next_sequence {
            return Err(FinalChainNativeSessionError::OutOfSequence {
                expected: self.next_sequence,
                actual: request.id.sequence,
            });
        }
        if !matches!(
            request.kind,
            FinalChainNativeCallKind::Call | FinalChainNativeCallKind::StaticCall
        ) {
            return Err(FinalChainNativeSessionError::UnsupportedCallKind);
        }
        if request.contract != DPOS_CONTRACT_ADDRESS
            || request.state_address != DPOS_CONTRACT_ADDRESS
        {
            return Err(FinalChainNativeSessionError::UnsupportedAddress);
        }
        Ok(())
    }

    fn validate_raw_validator(
        &self,
        validator: [u8; 20],
        read: &ConcreteRead<Vec<u8>>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let ConcreteRead::Present(bytes) = read else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator raw row is not present".to_owned(),
            ));
        };
        let expected = self.validator_facts(&self.dpos_state, validator)?;
        let rlp = exact_rlp(bytes, "setCommission validator row")?;
        let observed = if self.final_chain.magnolia_active(self.pending_period) {
            match rlp
                .item_count()
                .map_err(|error| raw_integrity("validator item count", error))?
            {
                4 if expected.4 == 0 => decode_legacy_validator(&rlp, 0)?,
                2 => {
                    let legacy = rlp
                        .at(0)
                        .map_err(|error| raw_integrity("extended validator body", error))?;
                    let count = rlp
                        .val_at(1)
                        .map_err(|error| raw_integrity("extended undelegation count", error))?;
                    decode_legacy_validator(&legacy, count)?
                }
                _ => {
                    return Err(FinalChainNativeSessionError::RawIntegrity(
                        "setCommission validator row has the wrong extended shape".to_owned(),
                    ));
                }
            }
        } else {
            if rlp
                .item_count()
                .map_err(|error| raw_integrity("validator item count", error))?
                != 4
            {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "setCommission validator row has the wrong legacy shape".to_owned(),
                ));
            }
            decode_legacy_validator(&rlp, 0)?
        };
        if observed != expected {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator raw/domain facts disagree".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_raw_owner(
        &self,
        validator: [u8; 20],
        read: &ConcreteRead<Vec<u8>>,
    ) -> std::result::Result<(), FinalChainNativeSessionError> {
        let expected = self
            .dpos_state
            .validator_metadata
            .get(&validator)
            .ok_or_else(|| {
                FinalChainNativeSessionError::RawIntegrity(
                    "setCommission validator metadata is absent".to_owned(),
                )
            })?
            .owner;
        let ConcreteRead::Present(bytes) = read else {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw row is not present".to_owned(),
            ));
        };
        if bytes.as_slice() != expected {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "setCommission owner raw/domain facts disagree".to_owned(),
            ));
        }
        Ok(())
    }

    fn validator_facts(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> std::result::Result<(U256, u16, u64, u64, u16), FinalChainNativeSessionError> {
        let stake = snapshot
            .total_stakes
            .get(&validator)
            .ok_or_else(|| {
                FinalChainNativeSessionError::RawIntegrity(
                    "setCommission validator stake is absent".to_owned(),
                )
            })?
            .as_u256();
        let metadata = snapshot.validator_metadata.get(&validator).ok_or_else(|| {
            FinalChainNativeSessionError::RawIntegrity(
                "setCommission validator metadata is absent".to_owned(),
            )
        })?;
        let reward_head = snapshot
            .reward_reference_graph
            .read_validator_head(&validator)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        Ok((
            stake,
            metadata.commission,
            metadata.last_commission_change,
            reward_head,
            checked_undelegations_count(snapshot, validator)?,
        ))
    }

    fn encode_validator_row(
        &self,
        snapshot: &DposSnapshot,
        validator: [u8; 20],
    ) -> std::result::Result<Vec<u8>, FinalChainNativeSessionError> {
        let (stake, commission, last_change, reward_head, undelegations_count) =
            self.validator_facts(snapshot, validator)?;
        let mut legacy = rlp::RlpStream::new_list(4);
        legacy
            .append(&stake)
            .append(&commission)
            .append(&last_change)
            .append(&reward_head);
        let legacy = legacy.out().to_vec();
        if !self.final_chain.magnolia_active(self.pending_period) {
            return Ok(legacy);
        }
        let mut extended = rlp::RlpStream::new_list(2);
        extended.append_raw(&legacy, 1).append(&undelegations_count);
        Ok(extended.out().to_vec())
    }
}

type ValidatorFacts = (U256, u16, u64, u64, u16);

fn exact_rlp<'a>(
    bytes: &'a [u8],
    label: &str,
) -> std::result::Result<Rlp<'a>, FinalChainNativeSessionError> {
    let rlp = Rlp::new(bytes);
    if rlp
        .payload_info()
        .map_err(|error| raw_integrity(label, error))?
        .total()
        != bytes.len()
    {
        return Err(FinalChainNativeSessionError::RawIntegrity(format!(
            "{label} has trailing bytes"
        )));
    }
    Ok(rlp)
}

fn decode_legacy_validator(
    rlp: &Rlp<'_>,
    undelegations_count: u16,
) -> std::result::Result<ValidatorFacts, FinalChainNativeSessionError> {
    if rlp
        .item_count()
        .map_err(|error| raw_integrity("legacy validator item count", error))?
        != 4
    {
        return Err(FinalChainNativeSessionError::RawIntegrity(
            "setCommission embedded validator row has the wrong shape".to_owned(),
        ));
    }
    Ok((
        rlp.val_at(0)
            .map_err(|error| raw_integrity("validator stake", error))?,
        rlp.val_at(1)
            .map_err(|error| raw_integrity("validator commission", error))?,
        rlp.val_at(2)
            .map_err(|error| raw_integrity("validator last commission change", error))?,
        rlp.val_at(3)
            .map_err(|error| raw_integrity("validator reward head", error))?,
        undelegations_count,
    ))
}

fn raw_integrity(label: &str, error: impl std::fmt::Display) -> FinalChainNativeSessionError {
    FinalChainNativeSessionError::RawIntegrity(format!("{label}: {error}"))
}

fn checked_undelegations_count(
    snapshot: &DposSnapshot,
    validator: [u8; 20],
) -> std::result::Result<u16, FinalChainNativeSessionError> {
    let v1_count = snapshot
        .undelegations
        .values()
        .flat_map(|entries| entries.iter())
        .filter(|entry| entry.validator == validator)
        .count();
    let v2_count = snapshot
        .undelegations_v2
        .values()
        .flat_map(|entries| entries.iter())
        .filter(|entry| entry.validator == validator)
        .try_fold(0usize, |count, group| {
            count.checked_add(group.entries.len())
        })
        .ok_or_else(|| {
            FinalChainNativeSessionError::Domain(
                "DPoS validator undelegation count overflow".to_owned(),
            )
        })?;
    let count = v1_count.checked_add(v2_count).ok_or_else(|| {
        FinalChainNativeSessionError::Domain(
            "DPoS validator undelegation count overflow".to_owned(),
        )
    })?;
    u16::try_from(count).map_err(|_| {
        FinalChainNativeSessionError::Domain(
            "DPoS validator undelegation count exceeds uint16".to_owned(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethereum_types::H160;
    use rustaxa_storage::Config;
    use rustaxa_types::GenesisValidatorMetadata;
    use std::cell::{Cell, RefCell};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const OWNER: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbb,
    ];
    const WRONG_OWNER: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
    ];
    const VALIDATOR: [u8; 20] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31,
    ];
    const VALIDATOR_KEY: [u8; 32] = [
        0x4e, 0xc8, 0xe9, 0x87, 0xae, 0x8c, 0x32, 0xd5, 0xe6, 0xa9, 0x3d, 0x7f, 0x1a, 0x07, 0xe4,
        0x06, 0x8c, 0xaa, 0x26, 0x97, 0xc4, 0xe2, 0x6d, 0x84, 0x44, 0xfd, 0x89, 0x96, 0x5b, 0xf0,
        0x66, 0x0a,
    ];
    const LEGACY_VALIDATOR_100: [u8; 7] = [0xc6, 0x82, 0x27, 0x10, 0x64, 0x80, 0x80];
    const EXTENDED_VALIDATOR_200: [u8; 10] =
        [0xc9, 0xc7, 0x82, 0x27, 0x10, 0x81, 0xc8, 0x01, 0x80, 0x80];
    const LEGACY_VALIDATOR_200: [u8; 8] = [0xc7, 0x82, 0x27, 0x10, 0x81, 0xc8, 0x01, 0x80];

    type RawRows = BTreeMap<([u8; 20], [u8; 32]), ConcreteRead<Vec<u8>>>;

    #[derive(Default)]
    struct RawState {
        rows: RefCell<RawRows>,
        accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
        reads: Cell<usize>,
        account_reads: Cell<usize>,
        fail_reads: bool,
    }

    impl RawState {
        fn fixture() -> Self {
            let state = Self::default();
            state.set(
                DPOS_CONTRACT_ADDRESS,
                VALIDATOR_KEY,
                ConcreteRead::Present(LEGACY_VALIDATOR_100.to_vec()),
            );
            state.set(
                DPOS_CONTRACT_ADDRESS,
                concrete_storage_key(&[&[0, 3], &VALIDATOR]),
                ConcreteRead::Present(OWNER.to_vec()),
            );
            state
        }

        fn erroring() -> Self {
            Self {
                fail_reads: true,
                ..Self::default()
            }
        }

        fn reward_fixture(total_vote_count: u64, amount_delegated: U256) -> Self {
            let state = Self {
                accounts: BTreeMap::from([(
                    DPOS_CONTRACT_ADDRESS,
                    FinalChainNativeAccount {
                        exists: true,
                        nonce: FinalChainNonce::zero(),
                        balance: (BigInt::from(1_u8) << 300_usize) + BigInt::from(10_000_u64),
                    },
                )]),
                ..Self::default()
            };
            state.set(
                DPOS_CONTRACT_ADDRESS,
                concrete_storage_key(&[&[0, 2], &VALIDATOR]),
                ConcreteRead::Present(vec![0xc2, 0x80, 0x80]),
            );
            state.set(
                DPOS_CONTRACT_ADDRESS,
                concrete_storage_key(&[&[4]]),
                ConcreteRead::Present(concrete_compact_u64(total_vote_count)),
            );
            state.set(
                DPOS_CONTRACT_ADDRESS,
                concrete_storage_key(&[&[5]]),
                ConcreteRead::Present(concrete_u256_bytes(amount_delegated)),
            );
            state
        }

        fn from_snapshot(
            snapshot: &DposSnapshot,
            accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
        ) -> Self {
            let rows = canonical_concrete_precompile_storage(snapshot, true)
                .unwrap()
                .into_iter()
                .map(|(identity, candidates)| {
                    let value = candidates
                        .into_iter()
                        .next()
                        .expect("canonical storage has at least one candidate");
                    (identity, ConcreteRead::Present(value))
                })
                .collect();
            Self {
                rows: RefCell::new(rows),
                accounts,
                ..Self::default()
            }
        }

        fn set(&self, address: [u8; 20], key: [u8; 32], value: ConcreteRead<Vec<u8>>) {
            self.rows.borrow_mut().insert((address, key), value);
        }

        fn apply(&self, mutation: &FinalChainNativeRawMutation) {
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => {
                    ConcreteRead::Present(value.as_bytes().to_vec())
                }
                FinalChainNativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            };
            self.set(mutation.address, mutation.key.0, value);
        }
    }

    impl FinalChainNativeStateRead for RawState {
        fn account(
            &self,
            address: [u8; 20],
        ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
            self.account_reads.set(self.account_reads.get() + 1);
            self.accounts.get(&address).cloned().ok_or_else(|| {
                FinalChainNativeStateReadError::Invariant(format!(
                    "fixture account is unavailable: {address:?}"
                ))
            })
        }

        fn raw_storage(
            &self,
            address: [u8; 20],
            key: &ConcreteStorageKey,
        ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            self.reads.set(self.reads.get() + 1);
            if self.fail_reads {
                return Err(FinalChainNativeStateReadError::Invariant(
                    "reader must not run on this path".to_owned(),
                ));
            }
            Ok(self
                .rows
                .borrow()
                .get(&(address, key.0))
                .cloned()
                .unwrap_or(ConcreteRead::Absent))
        }
    }

    fn temp_db_path(test_name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa-consensus-native-session-{test_name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn with_chain(
        test_name: &str,
        fix_redelegate_block_num: FinalChainBlockNumber,
        test: impl FnOnce(&FinalChain),
    ) {
        with_chain_config(
            test_name,
            FinalChainRewardsConfig {
                magnolia_period: FinalChainBlockNumber::GENESIS,
                cornus_period: FinalChainBlockNumber::GENESIS,
                fix_redelegate_block_num,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            test,
        );
    }

    fn with_chain_ficus(
        test_name: &str,
        fix_redelegate_block_num: FinalChainBlockNumber,
        test: impl FnOnce(&FinalChain),
    ) {
        with_chain_config_and_ficus(
            test_name,
            FinalChainRewardsConfig {
                magnolia_period: FinalChainBlockNumber::GENESIS,
                cornus_period: FinalChainBlockNumber::GENESIS,
                fix_redelegate_block_num,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            FinalChainBlockNumber::GENESIS,
            test,
        );
    }

    fn with_chain_config(
        test_name: &str,
        rewards_config: FinalChainRewardsConfig,
        test: impl FnOnce(&FinalChain),
    ) {
        with_chain_config_and_ficus(test_name, rewards_config, FinalChainBlockNumber::MAX, test);
    }

    fn with_chain_config_and_ficus(
        test_name: &str,
        rewards_config: FinalChainRewardsConfig,
        ficus_activation_period: FinalChainBlockNumber,
        test: impl FnOnce(&FinalChain),
    ) {
        let path = temp_db_path(test_name);
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let final_chain = FinalChain::new_with_rewards_config_and_ficus_activation(
            storage.clone(),
            1_000_000.into(),
            0,
            Vec::new(),
            vec![GenesisValidator {
                address: VALIDATOR,
                vrf_key: [0; 32],
                total_stake: U256::from(10_000).to_big_endian().to_vec(),
                delegations: vec![(VALIDATOR, U256::from(10_000).to_big_endian().to_vec())],
                metadata: GenesisValidatorMetadata {
                    owner: OWNER,
                    commission: 100,
                    ..Default::default()
                },
            }],
            GenesisDposConfig {
                eligibility_balance_threshold: U256::from(1_000).into(),
                vote_eligibility_balance_step: U256::from(1_000).into(),
                validator_maximum_stake: U256::from(30_000).into(),
                commission_change_delta: 0,
                commission_change_frequency: 0,
                ..Default::default()
            },
            rewards_config,
            ficus_activation_period,
        )
        .unwrap();
        test(&final_chain);
        drop(final_chain);
        drop(storage);
        let _ = std::fs::remove_dir_all(path);
    }

    fn commission_input(validator: [u8; 20], commission: u16) -> Vec<u8> {
        let mut input = Vec::with_capacity(68);
        input.extend_from_slice(&DPOS_SET_COMMISSION_SELECTOR);
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&validator);
        input.extend_from_slice(&[0; 30]);
        input.extend_from_slice(&commission.to_be_bytes());
        input
    }

    fn address_word_input(selector: [u8; 4], validator: [u8; 20]) -> Vec<u8> {
        let mut input = Vec::with_capacity(36);
        input.extend_from_slice(&selector);
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&validator);
        input
    }

    fn address_amount_input(selector: [u8; 4], validator: [u8; 20], amount: U256) -> Vec<u8> {
        let mut input = address_word_input(selector, validator);
        input.extend_from_slice(&amount.to_big_endian());
        input
    }

    fn address_id_input(selector: [u8; 4], validator: [u8; 20], id: u64) -> Vec<u8> {
        let mut input = address_word_input(selector, validator);
        input.extend_from_slice(&U256::from(id).to_big_endian());
        input
    }

    fn custody_request(
        sequence: u64,
        caller: [u8; 20],
        value: u64,
        input: Vec<u8>,
        gas: u64,
    ) -> FinalChainNativeRequest {
        FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: FinalChainTransactionPosition::new(sequence as u32),
                sequence,
            },
            period: FinalChainBlockNumber::new(1),
            depth: 1,
            kind: FinalChainNativeCallKind::Call,
            is_static: false,
            caller,
            contract: DPOS_CONTRACT_ADDRESS,
            state_address: DPOS_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::new(BigUint::from(value)),
            input,
            supplied_gas: gas.into(),
        }
    }

    fn request(
        sequence: u64,
        caller: [u8; 20],
        commission: u16,
        supplied_gas: u64,
    ) -> FinalChainNativeRequest {
        FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: FinalChainTransactionPosition::new(0),
                sequence,
            },
            period: FinalChainBlockNumber::new(1),
            depth: 1,
            kind: FinalChainNativeCallKind::Call,
            is_static: false,
            caller,
            contract: DPOS_CONTRACT_ADDRESS,
            state_address: DPOS_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::default(),
            input: commission_input(VALIDATOR, commission),
            supplied_gas: supplied_gas.into(),
        }
    }

    fn completed(result: FinalChainNativeInvocationResult) -> FinalChainNativeOutcome {
        match result {
            FinalChainNativeInvocationResult::Completed(outcome) => outcome,
            FinalChainNativeInvocationResult::InsufficientGas { .. } => {
                panic!("expected completed native result")
            }
        }
    }

    fn invoke_custody(
        session: &mut FinalChainNativeSession<'_>,
        state: &RawState,
        request: &FinalChainNativeRequest,
    ) -> FinalChainNativeOutcome {
        let quote = session.prepare(request, state).unwrap();
        completed(session.invoke(request, quote, state).unwrap())
    }

    fn apply_raw_mutations(state: &RawState, outcome: &FinalChainNativeOutcome) {
        for mutation in &outcome.raw_mutations {
            state.apply(mutation);
        }
    }

    #[test]
    fn real_kernel_matches_public_set_commission_fixture_and_survives_ordinary_rollback() {
        with_chain("fixture", FinalChainBlockNumber::GENESIS, |final_chain| {
            let state = RawState::fixture();
            let mut session = final_chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let first = request(0, OWNER, 200, 79_000);
            let quote = session.prepare(&first, &state).unwrap();
            assert_eq!(quote.required_gas, 20_000.into());
            assert_eq!(state.reads.get(), 2);
            let outcome = completed(session.invoke(&first, quote, &state).unwrap());
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            assert_eq!(outcome.gas_used, 20_000.into());
            assert!(outcome.output.is_empty());
            assert_eq!(outcome.raw_mutations.len(), 1);
            let mutation = &outcome.raw_mutations[0];
            assert_eq!(mutation.address, DPOS_CONTRACT_ADDRESS);
            assert_eq!(mutation.key, ConcreteStorageKey(VALIDATOR_KEY));
            assert_eq!(
                mutation.expected,
                ConcreteRead::Present(LEGACY_VALIDATOR_100.to_vec())
            );
            assert_eq!(
                mutation.operation,
                FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(EXTENDED_VALIDATOR_200.to_vec()).unwrap()
                )
            );
            assert_eq!(outcome.logs.len(), 1);
            assert_eq!(
                outcome.logs[0],
                FinalChainCallLog {
                    address: DPOS_CONTRACT_ADDRESS,
                    topics: vec![DPOS_COMMISSION_SET_TOPIC, address_topic(VALIDATOR)],
                    data: abi_word_from_u64(200).to_vec(),
                }
            );

            // The outer frame may discard the returned log, but its raw lane
            // applies the put and the session's semantic DPoS state stays live.
            state.apply(mutation);
            let mut second = request(1, OWNER, 300, 20_000);
            second.kind = FinalChainNativeCallKind::StaticCall;
            second.is_static = true;
            let second_quote = session.prepare(&second, &state).unwrap();
            let second_outcome = completed(session.invoke(&second, second_quote, &state).unwrap());
            assert_eq!(second_outcome.status, FinalChainNativeStatus::Success);
            assert_eq!(second_outcome.raw_mutations.len(), 1);
            assert_eq!(
                second_outcome.raw_mutations[0].expected,
                ConcreteRead::Present(EXTENDED_VALIDATOR_200.to_vec())
            );

            state.apply(&second_outcome.raw_mutations[0]);
            let overflow = request(2, OWNER, 10_001, 20_000);
            let overflow_quote = session.prepare(&overflow, &state).unwrap();
            let overflow_outcome =
                completed(session.invoke(&overflow, overflow_quote, &state).unwrap());
            assert_eq!(
                overflow_outcome.status,
                FinalChainNativeStatus::ContractFailure {
                    error: "Commission is bigger than maximum value".to_owned(),
                }
            );
            assert_eq!(overflow_outcome.gas_used, 20_000.into());
            assert!(overflow_outcome.raw_mutations.is_empty());
            assert!(overflow_outcome.logs.is_empty());

            let wrong_owner = request(3, WRONG_OWNER, 400, 20_000);
            let wrong_owner_quote = session.prepare(&wrong_owner, &state).unwrap();
            let wrong_owner_outcome = completed(
                session
                    .invoke(&wrong_owner, wrong_owner_quote, &state)
                    .unwrap(),
            );
            assert_eq!(
                wrong_owner_outcome.status,
                FinalChainNativeStatus::ContractFailure {
                    error: "This account is not owner of specified validator".to_owned(),
                }
            );
            assert!(wrong_owner_outcome.raw_mutations.is_empty());
            assert!(wrong_owner_outcome.logs.is_empty());
        });
    }

    #[test]
    fn nonpayable_and_insufficient_gas_finish_before_erroring_raw_reader() {
        with_chain(
            "early-terminal",
            FinalChainBlockNumber::GENESIS,
            |final_chain| {
                let state = RawState::erroring();
                let mut session = final_chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();

                let mut nonpayable = request(0, OWNER, 200, 0);
                nonpayable.value = FinalChainNativeValue::new((BigUint::from(1u8) << 300) + 7u8);
                assert!(nonpayable.value.value().bits() > 256);
                let quote = session.prepare(&nonpayable, &state).unwrap();
                assert_eq!(quote.required_gas, FinalChainGas::ZERO);
                let outcome = completed(session.invoke(&nonpayable, quote, &state).unwrap());
                assert_eq!(
                    outcome.status,
                    FinalChainNativeStatus::ContractFailure {
                        error: "Method is not payable".to_owned(),
                    }
                );
                assert_eq!(outcome.gas_used, FinalChainGas::ZERO);
                assert!(outcome.raw_mutations.is_empty());
                assert!(outcome.logs.is_empty());
                assert_eq!(state.reads.get(), 0);

                let insufficient = request(1, OWNER, 200, 19_999);
                let quote = session.prepare(&insufficient, &state).unwrap();
                assert_eq!(quote.required_gas, 20_000.into());
                assert_eq!(
                    session.invoke(&insufficient, quote, &state).unwrap(),
                    FinalChainNativeInvocationResult::InsufficientGas {
                        required_gas: 20_000.into(),
                    }
                );
                assert_eq!(state.reads.get(), 0);
            },
        );
    }

    #[test]
    fn quote_mismatch_is_retryable_but_stale_raw_aborts_the_session() {
        with_chain("stale", FinalChainBlockNumber::GENESIS, |final_chain| {
            let state = RawState::fixture();
            let mut session = final_chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();
            let original = request(0, OWNER, 200, 20_000);
            let quote = session.prepare(&original, &state).unwrap();

            let mut changed_request = original.clone();
            changed_request.depth = 2;
            assert_eq!(
                session.invoke(&changed_request, quote, &state).unwrap_err(),
                FinalChainNativeSessionError::QuoteMismatch
            );
            let stale_quote = FinalChainNativeGasQuote {
                required_gas: 19_999.into(),
                ..quote
            };
            assert_eq!(
                session.invoke(&original, stale_quote, &state).unwrap_err(),
                FinalChainNativeSessionError::QuoteMismatch
            );

            state.set(
                DPOS_CONTRACT_ADDRESS,
                VALIDATOR_KEY,
                ConcreteRead::Present(vec![0x80]),
            );
            assert!(matches!(
                session.invoke(&original, quote, &state),
                Err(FinalChainNativeSessionError::RawIntegrity(_))
            ));

            // Raw-integrity failure aborts the pending period and cannot retry.
            assert_eq!(
                session.invoke(&original, quote, &state).unwrap_err(),
                FinalChainNativeSessionError::Aborted
            );
            assert_eq!(
                session.prepare(&original, &state).unwrap_err(),
                FinalChainNativeSessionError::Aborted
            );
        });
    }

    #[test]
    fn pre_fix_nested_rejection_is_quoted_and_precedes_raw_reads_and_nonpayability() {
        with_chain("pre-fix", FinalChainBlockNumber::new(2), |final_chain| {
            let state = RawState::erroring();
            let mut session = final_chain
                .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                .unwrap();

            let underfunded_nested = request(0, OWNER, 200, 19_999);
            let quote = session.prepare(&underfunded_nested, &state).unwrap();
            assert_eq!(quote.required_gas, 20_000.into());
            assert_eq!(
                session.invoke(&underfunded_nested, quote, &state).unwrap(),
                FinalChainNativeInvocationResult::InsufficientGas {
                    required_gas: 20_000.into(),
                }
            );
            assert_eq!(state.reads.get(), 0);

            let nested = request(1, OWNER, 200, 20_000);
            let quote = session.prepare(&nested, &state).unwrap();
            assert_eq!(quote.required_gas, 20_000.into());
            let outcome = completed(session.invoke(&nested, quote, &state).unwrap());
            assert_eq!(
                outcome.status,
                FinalChainNativeStatus::ContractFailure {
                    error: "only top-level calls are allowed".to_owned(),
                }
            );
            assert_eq!(outcome.gas_used, 20_000.into());
            assert!(outcome.raw_mutations.is_empty());
            assert!(outcome.logs.is_empty());
            assert_eq!(state.reads.get(), 0);

            let mut nonpayable_nested = request(2, OWNER, 200, 0);
            nonpayable_nested.value = FinalChainNativeValue::new((BigUint::from(1u8) << 300) + 7u8);
            let quote = session.prepare(&nonpayable_nested, &state).unwrap();
            assert_eq!(quote.required_gas, FinalChainGas::ZERO);
            let outcome = completed(session.invoke(&nonpayable_nested, quote, &state).unwrap());
            assert_eq!(
                outcome.status,
                FinalChainNativeStatus::ContractFailure {
                    error: "only top-level calls are allowed".to_owned(),
                }
            );
            assert_eq!(outcome.gas_used, FinalChainGas::ZERO);
            assert_eq!(state.reads.get(), 0);
        });

        with_chain(
            "at-fix-boundary",
            FinalChainBlockNumber::new(1),
            |final_chain| {
                let state = RawState::fixture();
                let mut session = final_chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let nested = request(0, OWNER, 200, 20_000);
                let quote = session.prepare(&nested, &state).unwrap();
                let outcome = completed(session.invoke(&nested, quote, &state).unwrap());
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(
                    outcome.raw_mutations[0].operation,
                    FinalChainNativeRawOperation::Put(
                        FinalChainNativeRawValue::new(EXTENDED_VALIDATOR_200.to_vec()).unwrap()
                    )
                );
            },
        );
    }

    #[test]
    fn pre_magnolia_uses_legacy_rows_and_rejects_extended_input() {
        with_chain_config(
            "pre-magnolia",
            FinalChainRewardsConfig {
                magnolia_period: FinalChainBlockNumber::new(2),
                cornus_period: FinalChainBlockNumber::GENESIS,
                fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            |final_chain| {
                let state = RawState::fixture();
                let mut session = final_chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let invocation = request(0, OWNER, 200, 20_000);
                let quote = session.prepare(&invocation, &state).unwrap();
                let outcome = completed(session.invoke(&invocation, quote, &state).unwrap());
                assert_eq!(
                    outcome.raw_mutations[0].operation,
                    FinalChainNativeRawOperation::Put(
                        FinalChainNativeRawValue::new(LEGACY_VALIDATOR_200.to_vec()).unwrap()
                    )
                );

                let extended_state = RawState::fixture();
                extended_state.set(
                    DPOS_CONTRACT_ADDRESS,
                    VALIDATOR_KEY,
                    ConcreteRead::Present(vec![
                        0xc8, 0xc6, 0x82, 0x27, 0x10, 0x64, 0x80, 0x80, 0x80,
                    ]),
                );
                let mut extended_session = final_chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                assert!(matches!(
                    extended_session.prepare(&invocation, &extended_state),
                    Err(FinalChainNativeSessionError::RawIntegrity(_))
                ));
                assert_eq!(
                    extended_session
                        .prepare(&invocation, &extended_state)
                        .unwrap_err(),
                    FinalChainNativeSessionError::Aborted
                );
            },
        );
    }

    fn replay_invocation(
        chain: &FinalChain,
        invocation: &FinalChainConcreteInvocation,
    ) -> Result<()> {
        let mut snapshot =
            chain.dpos_snapshot_at_finalized_block(FinalChainBlockNumber::GENESIS)?;
        chain.advance_reward_reference_graph_block(&mut snapshot, 1.into())?;
        let mut gas_snapshot = snapshot.clone();
        let mut accounts = chain.account_snapshot_map_at_block(FinalChainBlockNumber::GENESIS)?;
        chain.replay_concrete_precompile_invocation(
            1.into(),
            invocation,
            &mut snapshot,
            &mut gas_snapshot,
            &mut accounts,
            FinalChainBlockNumber::GENESIS,
            &mut None,
            None,
            &mut None,
        )
    }

    #[test]
    fn concrete_replay_shares_full_width_admission_and_exact_failure_text() {
        with_chain("concrete-admission", 2.into(), |chain| {
            let mut invocation = FinalChainConcreteInvocation {
                transaction_index: 0,
                sequence: 0,
                depth: 1,
                call_type: 0,
                caller: OWNER,
                contract: DPOS_CONTRACT_ADDRESS,
                value: (BigUint::from(1_u8) << 264_usize).to_bytes_be(),
                input: commission_input(VALIDATOR, 200),
                output: Vec::new(),
                supplied_gas: 0,
                required_gas: 0,
                gas_used: 0,
                error: "only top-level calls are allowed".to_owned(),
                logs: Vec::new(),
                disposition: FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED,
            };
            replay_invocation(chain, &invocation).unwrap();
            invocation.error = "Method is not payable".to_owned();
            assert!(
                replay_invocation(chain, &invocation)
                    .unwrap_err()
                    .to_string()
                    .contains("ERROR_MISMATCH")
            );
            invocation.depth = 0;
            replay_invocation(chain, &invocation).unwrap();
            invocation.value.clear();
            invocation.depth = 1;
            invocation.required_gas = DPOS_SET_COMMISSION_GAS;
            invocation.supplied_gas = DPOS_SET_COMMISSION_GAS - 1;
            invocation.error = "out of gas".to_owned();
            replay_invocation(chain, &invocation).unwrap();
            invocation.error = "only top-level calls are allowed".to_owned();
            assert!(replay_invocation(chain, &invocation).is_err());
            invocation.supplied_gas = DPOS_SET_COMMISSION_GAS;
            invocation.gas_used = DPOS_SET_COMMISSION_GAS;
            replay_invocation(chain, &invocation).unwrap();
        });
        with_chain("concrete-kernel-error", 0.into(), |chain| {
            let mut invocation = FinalChainConcreteInvocation {
                transaction_index: 0,
                sequence: 0,
                depth: 1,
                call_type: 0,
                caller: [0xab; 20],
                contract: DPOS_CONTRACT_ADDRESS,
                value: Vec::new(),
                input: commission_input(VALIDATOR, 200),
                output: Vec::new(),
                supplied_gas: DPOS_SET_COMMISSION_GAS,
                required_gas: DPOS_SET_COMMISSION_GAS,
                gas_used: DPOS_SET_COMMISSION_GAS,
                error: "This account is not owner of specified validator".to_owned(),
                logs: Vec::new(),
                disposition: FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED,
            };
            replay_invocation(chain, &invocation).unwrap();
            invocation.error = "some other failure".to_owned();
            assert!(
                replay_invocation(chain, &invocation)
                    .unwrap_err()
                    .to_string()
                    .contains("ERROR_MISMATCH")
            );
        });
    }

    #[test]
    fn concrete_replay_classifies_wide_delegate_without_low_word_aliasing() {
        with_chain("concrete-wide-delegate", 0.into(), |chain| {
            let mut input = DPOS_DELEGATE_SELECTOR.to_vec();
            input.extend_from_slice(&[0; 12]);
            input.extend_from_slice(&VALIDATOR);
            let invocation = FinalChainConcreteInvocation {
                transaction_index: 0,
                sequence: 0,
                depth: 0,
                call_type: 0,
                caller: OWNER,
                contract: DPOS_CONTRACT_ADDRESS,
                value: ((BigUint::from(1_u8) << 256_usize) + BigUint::from(7_u8)).to_bytes_be(),
                input,
                output: Vec::new(),
                supplied_gas: DPOS_DELEGATE_GAS,
                required_gas: DPOS_DELEGATE_GAS,
                gas_used: DPOS_DELEGATE_GAS,
                error: "Validator's max stake exceeded".to_owned(),
                logs: Vec::new(),
                disposition: FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED,
            };
            replay_invocation(chain, &invocation).unwrap();
        });
    }

    #[test]
    fn concrete_replay_normalizes_leading_zero_delegate_value() {
        with_chain("concrete-leading-zero-delegate", 0.into(), |chain| {
            let mut input = DPOS_DELEGATE_SELECTOR.to_vec();
            input.extend_from_slice(&[0; 12]);
            input.extend_from_slice(&VALIDATOR);
            // Bind expected ordinary kernel output for amount seven, then
            // replay the same numeric value with oversized leading-zero bytes.
            let mut snapshot = chain.dpos_snapshot_at_finalized_block(0.into()).unwrap();
            chain
                .advance_reward_reference_graph_block(&mut snapshot, 1.into())
                .unwrap();
            let mut accounts = chain.account_snapshot_map_at_block(0.into()).unwrap();
            let outcome = chain
                .apply_dpos_delegate(&mut snapshot, &mut accounts, VALIDATOR, VALIDATOR, vec![7])
                .unwrap();
            assert_eq!(outcome.status_code, 1);
            let mut value = vec![0; 40];
            value.push(7);
            let invocation = FinalChainConcreteInvocation {
                transaction_index: 0,
                sequence: 0,
                depth: 0,
                call_type: 0,
                caller: VALIDATOR,
                contract: DPOS_CONTRACT_ADDRESS,
                value,
                input,
                output: outcome.code_retval,
                supplied_gas: DPOS_DELEGATE_GAS,
                required_gas: DPOS_DELEGATE_GAS,
                gas_used: DPOS_DELEGATE_GAS,
                error: String::new(),
                logs: outcome
                    .logs
                    .into_iter()
                    .map(|log| crate::final_chain_execution::FinalChainEvmLog {
                        address: log.address,
                        topics: log
                            .topics
                            .into_iter()
                            .map(
                                |topic| crate::final_chain_execution::FinalChainEvmLogTopic {
                                    topic,
                                },
                            )
                            .collect(),
                        data: log.data,
                    })
                    .collect(),
                disposition: FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL,
            };
            replay_invocation(chain, &invocation).unwrap();
        });
    }

    #[test]
    fn selected_custody_reuses_kernels_and_preserves_raw_operation_order() {
        const DELEGATOR: [u8; 20] = [0x50; 20];
        with_chain_ficus(
            "selected-custody",
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let wide = (BigInt::from(1_u8) << 300_usize) + BigInt::from(10_000_u64);
                let state = RawState::from_snapshot(
                    &session.dpos_state,
                    BTreeMap::from([
                        (
                            DPOS_CONTRACT_ADDRESS,
                            FinalChainNativeAccount {
                                exists: true,
                                nonce: FinalChainNonce::zero(),
                                balance: wide.clone(),
                            },
                        ),
                        (
                            DELEGATOR,
                            FinalChainNativeAccount {
                                exists: false,
                                nonce: FinalChainNonce::zero(),
                                balance: BigInt::default(),
                            },
                        ),
                    ]),
                );
                // Untouched Magnolia-era validators may still retain the legacy
                // four-field row until this custody mutation rewrites them.
                state.set(
                    DPOS_CONTRACT_ADDRESS,
                    concrete_storage_key(&[&[0, 0], &VALIDATOR]),
                    ConcreteRead::Present(LEGACY_VALIDATOR_100.to_vec()),
                );

                let delegate = custody_request(
                    0,
                    DELEGATOR,
                    500,
                    address_word_input(DPOS_DELEGATE_SELECTOR, VALIDATOR),
                    40_000,
                );
                let quote = session.prepare(&delegate, &state).unwrap();
                assert_eq!(quote.required_gas, 40_000.into());
                let delegated = completed(session.invoke(&delegate, quote, &state).unwrap());
                assert_eq!(delegated.status, FinalChainNativeStatus::Success);
                assert!(delegated.account_mutations.is_empty());
                assert_eq!(state.account_reads.get(), 0);
                assert_eq!(delegated.raw_mutations.len(), 8);
                let delegate_keys = delegated
                    .raw_mutations
                    .iter()
                    .map(|mutation| mutation.key)
                    .collect::<Vec<_>>();
                assert_eq!(
                    delegate_keys,
                    vec![
                        ConcreteStorageKey(concrete_storage_key(&[&[1], &VALIDATOR, &[]])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &[2, 0],
                            &VALIDATOR,
                            &DELEGATOR,
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &[2, 1],
                            &DELEGATOR,
                            &[2],
                            &1_u32.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &[2, 1],
                            &DELEGATOR,
                            &[2],
                            &VALIDATOR,
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[&[2, 1], &DELEGATOR, &[1]])),
                        ConcreteStorageKey(concrete_storage_key(&[&[1], &VALIDATOR, &[1]])),
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &VALIDATOR])),
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &VALIDATOR])),
                    ]
                );
                for mutation in &delegated.raw_mutations {
                    state.apply(mutation);
                }

                let undelegate = custody_request(
                    1,
                    DELEGATOR,
                    0,
                    address_amount_input(DPOS_UNDELEGATE_V2_SELECTOR, VALIDATOR, U256::from(500)),
                    60_000,
                );
                let quote = session.prepare(&undelegate, &state).unwrap();
                let undelegated = completed(session.invoke(&undelegate, quote, &state).unwrap());
                assert_eq!(undelegated.status, FinalChainNativeStatus::Success);
                assert_eq!(undelegated.output, abi_word_from_u64(1));
                assert!(undelegated.account_mutations.is_empty());
                assert_eq!(state.account_reads.get(), 1);
                assert_eq!(undelegated.raw_mutations.len(), 16);
                assert_eq!(
                    undelegated.raw_mutations[0].key,
                    undelegated.raw_mutations[5].key
                );
                let object = undelegated
                    .raw_mutations
                    .iter()
                    .find(|mutation| {
                        mutation.key
                            == ConcreteStorageKey(concrete_storage_key(&[
                                &[3, 0],
                                &DELEGATOR,
                                &VALIDATOR,
                                &1_u64.to_le_bytes(),
                            ]))
                    })
                    .expect("V2 object write is present");
                assert_eq!(
                    object.operation,
                    FinalChainNativeRawOperation::Put(
                        FinalChainNativeRawValue::new(vec![
                            0xc6, 0xc4, 0x82, 0x01, 0xf4, 0x01, 0x01
                        ])
                        .unwrap()
                    )
                );
                for mutation in &undelegated.raw_mutations {
                    state.apply(mutation);
                }

                let confirm = custody_request(
                    2,
                    DELEGATOR,
                    0,
                    address_id_input(DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR, VALIDATOR, 1),
                    20_000,
                );
                let quote = session.prepare(&confirm, &state).unwrap();
                let confirmed = completed(session.invoke(&confirm, quote, &state).unwrap());
                assert_eq!(confirmed.status, FinalChainNativeStatus::Success);
                assert_eq!(confirmed.raw_mutations.len(), 8);
                assert_eq!(state.account_reads.get(), 3);
                assert_eq!(
                    confirmed.account_mutations,
                    vec![
                        FinalChainNativeOrdinaryMutation::BalanceReplace {
                            address: DPOS_CONTRACT_ADDRESS,
                            expected_exists: true,
                            expected: wide.clone(),
                            replacement: &wide - BigInt::from(500_u64),
                        },
                        FinalChainNativeOrdinaryMutation::BalanceReplace {
                            address: DELEGATOR,
                            expected_exists: false,
                            expected: BigInt::default(),
                            replacement: BigInt::from(500_u64),
                        },
                    ]
                );
                assert!(
                    confirmed.raw_mutations[..7]
                        .iter()
                        .all(
                            |mutation| mutation.operation == FinalChainNativeRawOperation::Delete
                                || matches!(
                                    &mutation.operation,
                                    FinalChainNativeRawOperation::Put(value)
                                        if value.as_bytes() == [0, 0, 0, 0]
                                )
                        )
                );
            },
        );
    }

    #[test]
    fn selected_custody_preserves_existing_partial_and_v2_swap_remove_order() {
        const DELEGATOR: [u8; 20] = [0x51; 20];
        with_chain_ficus(
            "selected-custody-existing-partial",
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let state = RawState::from_snapshot(
                    &session.dpos_state,
                    BTreeMap::from([
                        (
                            DPOS_CONTRACT_ADDRESS,
                            FinalChainNativeAccount {
                                exists: true,
                                nonce: FinalChainNonce::zero(),
                                balance: BigInt::from(10_000_u64),
                            },
                        ),
                        (
                            DELEGATOR,
                            FinalChainNativeAccount {
                                exists: false,
                                nonce: FinalChainNonce::zero(),
                                balance: BigInt::default(),
                            },
                        ),
                    ]),
                );

                let first_delegate = custody_request(
                    0,
                    DELEGATOR,
                    1_000,
                    address_word_input(DPOS_DELEGATE_SELECTOR, VALIDATOR),
                    DPOS_DELEGATE_GAS,
                );
                let outcome = invoke_custody(&mut session, &state, &first_delegate);
                apply_raw_mutations(&state, &outcome);

                let existing_delegate = custody_request(
                    1,
                    DELEGATOR,
                    500,
                    address_word_input(DPOS_DELEGATE_SELECTOR, VALIDATOR),
                    DPOS_DELEGATE_GAS,
                );
                let outcome = invoke_custody(&mut session, &state, &existing_delegate);
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.raw_mutations.len(), 5);
                let current_node = ConcreteStorageKey(concrete_storage_key(&[
                    &[1],
                    &VALIDATOR,
                    &concrete_bigint_u64_bytes(1),
                ]));
                let delegation =
                    ConcreteStorageKey(concrete_storage_key(&[&[2, 0], &VALIDATOR, &DELEGATOR]));
                assert_eq!(
                    outcome
                        .raw_mutations
                        .iter()
                        .map(|mutation| mutation.key)
                        .collect::<Vec<_>>(),
                    vec![
                        current_node,
                        delegation,
                        current_node,
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &VALIDATOR])),
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &VALIDATOR])),
                    ]
                );
                apply_raw_mutations(&state, &outcome);

                let first_undelegate = custody_request(
                    2,
                    DELEGATOR,
                    0,
                    address_amount_input(DPOS_UNDELEGATE_V2_SELECTOR, VALIDATOR, U256::from(300)),
                    DPOS_UNDELEGATE_GAS,
                );
                let outcome = invoke_custody(&mut session, &state, &first_undelegate);
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.raw_mutations.len(), 13);
                assert_eq!(outcome.raw_mutations[0].key, current_node);
                assert_eq!(outcome.raw_mutations[1].key, delegation);
                assert_eq!(outcome.raw_mutations[2].key, current_node);
                assert!(matches!(
                    &outcome.raw_mutations[1].operation,
                    FinalChainNativeRawOperation::Put(value)
                        if value.as_bytes() == [0xc4, 0x82, 0x04, 0xb0, 0x01]
                ));
                apply_raw_mutations(&state, &outcome);

                let second_undelegate = custody_request(
                    3,
                    DELEGATOR,
                    0,
                    address_amount_input(DPOS_UNDELEGATE_V2_SELECTOR, VALIDATOR, U256::from(200)),
                    DPOS_UNDELEGATE_GAS,
                );
                let outcome = invoke_custody(&mut session, &state, &second_undelegate);
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.raw_mutations.len(), 10);
                apply_raw_mutations(&state, &outcome);

                let confirm_first = custody_request(
                    4,
                    DELEGATOR,
                    0,
                    address_id_input(DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR, VALIDATOR, 1),
                    DPOS_SET_COMMISSION_GAS,
                );
                let outcome = invoke_custody(&mut session, &state, &confirm_first);
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.raw_mutations.len(), 7);
                let ids_prefix = [&[3, 3][..], &DELEGATOR, &VALIDATOR].concat();
                assert_eq!(
                    outcome
                        .raw_mutations
                        .iter()
                        .map(|mutation| mutation.key)
                        .collect::<Vec<_>>(),
                    vec![
                        ConcreteStorageKey(concrete_storage_key(&[
                            &[3, 0],
                            &DELEGATOR,
                            &VALIDATOR,
                            &1_u64.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &ids_prefix,
                            &[2],
                            &2_u64.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &ids_prefix,
                            &[2],
                            &1_u32.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &ids_prefix,
                            &[2],
                            &1_u64.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[
                            &ids_prefix,
                            &[2],
                            &2_u32.to_le_bytes(),
                        ])),
                        ConcreteStorageKey(concrete_storage_key(&[&ids_prefix, &[1]])),
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &VALIDATOR])),
                    ]
                );
                assert!(matches!(
                    &outcome.raw_mutations[1].operation,
                    FinalChainNativeRawOperation::Put(value)
                        if value.as_bytes() == 1_u32.to_le_bytes()
                ));
                assert!(matches!(
                    &outcome.raw_mutations[2].operation,
                    FinalChainNativeRawOperation::Put(value)
                        if value.as_bytes() == 2_u64.to_le_bytes()
                ));
                assert_eq!(
                    outcome.raw_mutations[3].operation,
                    FinalChainNativeRawOperation::Delete
                );
                assert_eq!(
                    outcome.raw_mutations[4].operation,
                    FinalChainNativeRawOperation::Delete
                );
                assert!(matches!(
                    &outcome.raw_mutations[5].operation,
                    FinalChainNativeRawOperation::Put(value)
                        if value.as_bytes() == 1_u32.to_le_bytes()
                ));
            },
        );
    }

    #[test]
    fn ficus_boundary_controls_surviving_validator_write() {
        const DELEGATOR: [u8; 20] = [0x53; 20];
        for (name, ficus_activation, expected_active) in [
            ("selected-custody-ficus-h-minus-one", 2.into(), false),
            ("selected-custody-ficus-h", 1.into(), true),
        ] {
            with_chain_config_and_ficus(
                name,
                FinalChainRewardsConfig {
                    magnolia_period: FinalChainBlockNumber::GENESIS,
                    cornus_period: FinalChainBlockNumber::GENESIS,
                    fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
                    aspen_part_two_period: FinalChainBlockNumber::MAX,
                    cacti_period: FinalChainBlockNumber::MAX,
                    ..Default::default()
                },
                ficus_activation,
                |chain| {
                    assert_eq!(chain.ficus_active_at(1.into()), expected_active);
                    let mut session = chain
                        .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                        .unwrap();
                    let state = RawState::from_snapshot(
                        &session.dpos_state,
                        BTreeMap::from([
                            (
                                DPOS_CONTRACT_ADDRESS,
                                FinalChainNativeAccount {
                                    exists: true,
                                    nonce: FinalChainNonce::zero(),
                                    balance: BigInt::from(10_000_u64),
                                },
                            ),
                            (
                                DELEGATOR,
                                FinalChainNativeAccount {
                                    exists: false,
                                    nonce: FinalChainNonce::zero(),
                                    balance: BigInt::default(),
                                },
                            ),
                        ]),
                    );

                    let delegate = custody_request(
                        0,
                        DELEGATOR,
                        500,
                        address_word_input(DPOS_DELEGATE_SELECTOR, VALIDATOR),
                        DPOS_DELEGATE_GAS,
                    );
                    let outcome = invoke_custody(&mut session, &state, &delegate);
                    apply_raw_mutations(&state, &outcome);
                    let undelegate = custody_request(
                        1,
                        DELEGATOR,
                        0,
                        address_amount_input(
                            DPOS_UNDELEGATE_V2_SELECTOR,
                            VALIDATOR,
                            U256::from(500),
                        ),
                        DPOS_UNDELEGATE_GAS,
                    );
                    let outcome = invoke_custody(&mut session, &state, &undelegate);
                    apply_raw_mutations(&state, &outcome);

                    let confirm = custody_request(
                        2,
                        DELEGATOR,
                        0,
                        address_id_input(DPOS_CONFIRM_UNDELEGATE_V2_SELECTOR, VALIDATOR, 1),
                        DPOS_SET_COMMISSION_GAS,
                    );
                    let outcome = invoke_custody(&mut session, &state, &confirm);
                    assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                    let validator_key =
                        ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &VALIDATOR]));
                    let validator_writes = outcome
                        .raw_mutations
                        .iter()
                        .filter(|mutation| mutation.key == validator_key)
                        .count();
                    assert_eq!(validator_writes, usize::from(expected_active));
                    assert_eq!(outcome.raw_mutations.len(), 7 + validator_writes);
                },
            );
        }
    }

    #[test]
    fn cornus_boundary_rejects_v2_before_activation_and_runs_it_at_activation() {
        with_chain_config(
            "selected-custody-cornus-h-minus-one",
            FinalChainRewardsConfig {
                magnolia_period: FinalChainBlockNumber::GENESIS,
                cornus_period: 2.into(),
                fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            |chain| {
                assert!(matches!(
                    chain.begin_native_session(1.into(), FinalChainBlockNumber::GENESIS),
                    Err(FinalChainNativeSessionError::PreCornusUnsupported)
                ));
            },
        );
        with_chain_config_and_ficus(
            "selected-custody-cornus-h",
            FinalChainRewardsConfig {
                magnolia_period: FinalChainBlockNumber::GENESIS,
                cornus_period: 1.into(),
                fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                ..Default::default()
            },
            1.into(),
            |chain| {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let state = RawState::from_snapshot(
                    &session.dpos_state,
                    BTreeMap::from([(
                        DPOS_CONTRACT_ADDRESS,
                        FinalChainNativeAccount {
                            exists: true,
                            nonce: FinalChainNonce::zero(),
                            balance: BigInt::from(10_000_u64),
                        },
                    )]),
                );
                let request = custody_request(
                    0,
                    VALIDATOR,
                    0,
                    address_amount_input(DPOS_UNDELEGATE_V2_SELECTOR, VALIDATOR, U256::from(500)),
                    DPOS_UNDELEGATE_GAS,
                );

                let outcome = invoke_custody(&mut session, &state, &request);
                assert_eq!(outcome.status, FinalChainNativeStatus::Success);
                assert_eq!(outcome.output, abi_word_from_u64(1));
                assert!(!outcome.raw_mutations.is_empty());
            },
        );
    }

    #[test]
    fn selected_custody_late_raw_corruption_aborts_without_advancing_state() {
        const DELEGATOR: [u8; 20] = [0x52; 20];
        with_chain(
            "selected-custody-late-corruption",
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let before = session.dpos_state.clone();
                let state = RawState::from_snapshot(&before, BTreeMap::new());
                state.set(
                    DPOS_CONTRACT_ADDRESS,
                    concrete_storage_key(&[&[0, 2], &VALIDATOR]),
                    ConcreteRead::Present(vec![0xff]),
                );
                let request = custody_request(
                    0,
                    DELEGATOR,
                    500,
                    address_word_input(DPOS_DELEGATE_SELECTOR, VALIDATOR),
                    DPOS_DELEGATE_GAS,
                );
                let quote = session.prepare(&request, &state).unwrap();

                assert!(matches!(
                    session.invoke(&request, quote, &state),
                    Err(FinalChainNativeSessionError::RawIntegrity(message))
                        if message == "validator rewards raw/domain facts disagree"
                ));
                assert_eq!(session.dpos_state, before);
                assert_eq!(state.reads.get(), 8);
                assert_eq!(
                    session.prepare(&request, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                );
            },
        );
    }

    #[test]
    fn custody_business_preflight_precedes_lazy_account_and_raw_reads() {
        with_chain(
            "custody-preflight",
            FinalChainBlockNumber::GENESIS,
            |chain| {
                let mut session = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                let state = RawState::erroring();
                let request = custody_request(
                    0,
                    [0x44; 20],
                    500,
                    address_word_input(DPOS_DELEGATE_SELECTOR, [0x99; 20]),
                    40_000,
                );

                let quote = session.prepare(&request, &state).unwrap();
                let outcome = completed(session.invoke(&request, quote, &state).unwrap());

                assert_eq!(
                    outcome.status,
                    FinalChainNativeStatus::ContractFailure {
                        error: DposContractError::NonExistentValidator
                            .legacy_message()
                            .to_owned(),
                    }
                );
                assert!(outcome.account_mutations.is_empty());
                assert!(outcome.raw_mutations.is_empty());
                assert_eq!(state.account_reads.get(), 0);
                assert_eq!(state.reads.get(), 0);
            },
        );
    }

    #[test]
    fn bound_rewards_reject_pre_magnolia_scope_before_state_reads() {
        with_chain_config(
            "bound-rewards-pre-magnolia",
            FinalChainRewardsConfig {
                committee_size: 1,
                magnolia_period: 2.into(),
                cornus_period: FinalChainBlockNumber::GENESIS,
                aspen_part_one_period: FinalChainBlockNumber::GENESIS,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                max_block_author_reward_percent: 20,
                dag_proposers_reward_percent: 50,
                yield_percentage: 20,
                dpos_blocks_per_year: 1,
                rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
                ..Default::default()
            },
            |chain| {
                let request_id = [0x57; 32];
                let transaction_hash = H256::from([0x78; 32]);
                let plan = chain
                    .plan_external_evm_rewards_stats(
                        request_id,
                        FinalizedRewardsPeriodFact {
                            period: 1,
                            block_author: H160::from(VALIDATOR),
                            blocks_per_year: 1,
                            dpos_eligible_total_vote_count: 10,
                            transactions: vec![RewardTransactionFact {
                                hash: transaction_hash,
                                gas_price: U256::one(),
                                gas_used: 21_000_u64.into(),
                            }],
                            dag_blocks: vec![RewardDagBlockFact {
                                author: H160::from(VALIDATOR),
                                difficulty: 1,
                                transaction_hashes: vec![transaction_hash],
                            }],
                            cert_votes: vec![RewardCertVoteFact {
                                voter: H160::from(VALIDATOR),
                                weight: 1,
                                period: 1,
                            }],
                        },
                    )
                    .unwrap();
                let mut session = chain
                    .begin_native_session_bound(
                        request_id,
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                let state = RawState::erroring();

                assert_eq!(
                    session.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::RewardsScopeUnsupported)
                );
                assert_eq!(state.account_reads.get(), 0);
                assert_eq!(state.reads.get(), 0);
            },
        );
    }

    #[test]
    fn bound_rewards_preserve_repeated_raw_rows_and_ordered_full_width_custody() {
        with_chain_config(
            "bound-rewards",
            FinalChainRewardsConfig {
                committee_size: 1,
                magnolia_period: FinalChainBlockNumber::GENESIS,
                cornus_period: FinalChainBlockNumber::GENESIS,
                fix_redelegate_block_num: FinalChainBlockNumber::GENESIS,
                aspen_part_one_period: FinalChainBlockNumber::GENESIS,
                aspen_part_two_period: FinalChainBlockNumber::MAX,
                cacti_period: FinalChainBlockNumber::MAX,
                max_block_author_reward_percent: 20,
                dag_proposers_reward_percent: 50,
                yield_percentage: 20,
                dpos_blocks_per_year: 1,
                rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
                ..Default::default()
            },
            |chain| {
                let transaction_hash = H256::from([0x77; 32]);
                let plan = chain
                    .plan_external_evm_rewards_stats(
                        [0x55; 32],
                        FinalizedRewardsPeriodFact {
                            period: 1,
                            block_author: H160::from(VALIDATOR),
                            blocks_per_year: 1,
                            dpos_eligible_total_vote_count: 10,
                            transactions: vec![RewardTransactionFact {
                                hash: transaction_hash,
                                gas_price: U256::one(),
                                gas_used: 21_000_u64.into(),
                            }],
                            dag_blocks: vec![RewardDagBlockFact {
                                author: H160::from(VALIDATOR),
                                difficulty: 1,
                                transaction_hashes: vec![transaction_hash],
                            }],
                            cert_votes: vec![RewardCertVoteFact {
                                voter: H160::from(VALIDATOR),
                                weight: 1,
                                period: 1,
                            }],
                        },
                    )
                    .unwrap();
                let state = RawState::reward_fixture(10, U256::from(10_000_u64));
                let mut session = chain
                    .begin_native_session_bound(
                        [0x55; 32],
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();

                let outcome = session.finish_rewards(&plan, &state).unwrap();

                assert_eq!(outcome.total_reward.as_u256(), U256::from(2_000_u64));
                let wide = (BigInt::from(1_u8) << 300_usize) + BigInt::from(10_000_u64);
                assert_eq!(
                    outcome.account_mutations,
                    vec![
                        FinalChainNativeOrdinaryMutation::BalanceReplace {
                            address: DPOS_CONTRACT_ADDRESS,
                            expected_exists: true,
                            expected: wide.clone(),
                            replacement: &wide + BigInt::from(21_000_u64),
                        },
                        FinalChainNativeOrdinaryMutation::BalanceReplace {
                            address: DPOS_CONTRACT_ADDRESS,
                            expected_exists: true,
                            expected: &wide + BigInt::from(21_000_u64),
                            replacement: &wide + BigInt::from(23_000_u64),
                        },
                    ]
                );
                assert_eq!(outcome.raw_mutations.len(), 3);
                let reward_key = ConcreteStorageKey(concrete_storage_key(&[&[0, 2], &VALIDATOR]));
                assert_eq!(outcome.raw_mutations[0].key, reward_key);
                assert_eq!(
                    outcome.raw_mutations[0].expected,
                    ConcreteRead::Present(vec![0xc2, 0x80, 0x80])
                );
                assert_eq!(outcome.raw_mutations[1].key, reward_key);
                let mut bonus_row = rlp::RlpStream::new_list(2);
                bonus_row.append(&U256::from(396_u64));
                bonus_row.append(&U256::from(4_u64));
                assert_eq!(
                    outcome.raw_mutations[1].expected,
                    ConcreteRead::Present(bonus_row.out().to_vec())
                );
                let puts = outcome
                    .raw_mutations
                    .iter()
                    .map(|mutation| match &mutation.operation {
                        FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
                        FinalChainNativeRawOperation::Delete => Vec::new(),
                    })
                    .collect::<Vec<_>>();
                let mut final_reward_row = rlp::RlpStream::new_list(2);
                final_reward_row.append(&U256::from(1_980_u64));
                final_reward_row.append(&U256::from(21_020_u64));
                assert_eq!(puts[1], final_reward_row.out().to_vec());
                assert_eq!(puts[2], vec![0x07, 0xd0]);
                assert_eq!(
                    session.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                );
                assert_eq!(
                    session.prepare(&request(0, OWNER, 200, 20_000), &state),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                );
                let post_finish_request = request(0, OWNER, 200, 20_000);
                assert_eq!(
                    session.invoke(
                        &post_finish_request,
                        FinalChainNativeGasQuote {
                            invocation: post_finish_request.id,
                            required_gas: 20_000.into(),
                        },
                        &state,
                    ),
                    Err(FinalChainNativeSessionError::RewardsAlreadyFinished)
                );

                let mismatch_state = RawState::reward_fixture(10, U256::from(10_000_u64));
                let mut mismatched = chain
                    .begin_native_session_bound(
                        [0x56; 32],
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                assert_eq!(
                    mismatched.finish_rewards(&plan, &mismatch_state),
                    Err(FinalChainNativeSessionError::RewardsPlanMismatch)
                );
                assert_eq!(mismatch_state.reads.get(), 0);

                let set_commission_state = RawState::fixture();
                let mut outstanding = chain
                    .begin_native_session_bound(
                        [0x55; 32],
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                let invocation = request(0, OWNER, 200, 20_000);
                let outstanding_quote = outstanding
                    .prepare(&invocation, &set_commission_state)
                    .unwrap();
                assert_eq!(
                    outstanding.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::QuoteOutstanding)
                );
                assert_eq!(
                    completed(
                        outstanding
                            .invoke(&invocation, outstanding_quote, &set_commission_state)
                            .unwrap()
                    )
                    .status,
                    FinalChainNativeStatus::Success
                );

                let mut unbound = chain
                    .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
                    .unwrap();
                assert_eq!(
                    unbound.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::UnboundRewards)
                );

                let corrupt = RawState::reward_fixture(10, U256::from(10_000_u64));
                corrupt.set(
                    DPOS_CONTRACT_ADDRESS,
                    concrete_storage_key(&[&[0, 2], &VALIDATOR]),
                    ConcreteRead::Present(vec![0xff]),
                );
                let mut corrupt_session = chain
                    .begin_native_session_bound(
                        [0x55; 32],
                        1.into(),
                        FinalChainBlockNumber::GENESIS,
                    )
                    .unwrap();
                assert!(matches!(
                    corrupt_session.finish_rewards(&plan, &corrupt),
                    Err(FinalChainNativeSessionError::RawIntegrity(_))
                ));
                assert_eq!(
                    corrupt_session.finish_rewards(&plan, &state),
                    Err(FinalChainNativeSessionError::Aborted)
                );
            },
        );
    }
}
