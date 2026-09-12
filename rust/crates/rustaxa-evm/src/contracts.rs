//! Execution-side contracts for Taraxa's concrete EVM backend.
//!
//! These types preserve the wider Go transaction/account domain above REVM's
//! 256-bit operand stack. They also keep application block history and native
//! business execution behind narrow ports. FinalChain remains responsible for
//! ordered inputs, native-kernel ownership, persistence approval and publication.

use num_bigint::{BigInt, BigUint, Sign};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{ConcreteAccountBalance, ConcreteRead, ConcreteReadError, ConcreteStorageKey},
};

/// Arbitrary-width non-negative gas price used by transaction-envelope math.
///
/// Up-front fees and refunds use the full value. The EVM `GASPRICE` opcode uses
/// [`ExecutionGasPrice::low_word`], matching the reference's ignored-overflow
/// conversion to a 256-bit stack item.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExecutionGasPrice(BigUint);

impl ExecutionGasPrice {
    /// Wraps an unsigned gas price without imposing an EVM-word width.
    pub fn new(value: BigUint) -> Self {
        Self(value)
    }

    /// Borrows the value used by full-width envelope arithmetic.
    pub fn value(&self) -> &BigUint {
        &self.0
    }

    /// Returns the low 256 bits used by the EVM `GASPRICE` opcode.
    pub fn low_word(&self) -> [u8; 32] {
        low_u256_word(&self.0)
    }
}

/// Arbitrary-width non-negative value carried by a transaction or call frame.
///
/// Balance admission and transfer use the full value. `CALLVALUE` exposes only
/// the low 256 bits because the pinned Go interpreter ignores conversion
/// overflow at the operand-stack boundary.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExecutionValue(BigUint);

impl ExecutionValue {
    /// Wraps an unsigned execution value without imposing an EVM-word width.
    pub fn new(value: BigUint) -> Self {
        Self(value)
    }

    /// Borrows the value used by full-width balance arithmetic.
    pub fn value(&self) -> &BigUint {
        &self.0
    }

    /// Returns the low 256 bits used by the EVM `CALLVALUE` opcode.
    pub fn low_word(&self) -> [u8; 32] {
        low_u256_word(&self.0)
    }
}

/// Signed arbitrary-width account balance used inside an execution journal.
///
/// Persisted balances are unsigned, but the reference's zero/system sender
/// bypasses affordability checks and can have a negative intermediate balance.
/// Callers must use [`ExecutionBalance::try_to_persisted`] at a persistence
/// boundary instead of casting or taking an absolute value.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ExecutionBalance(BigInt);

impl ExecutionBalance {
    /// Wraps a signed journal balance without narrowing it.
    pub fn new(value: BigInt) -> Self {
        Self(value)
    }

    /// Converts an unsigned committed balance into the signed journal domain.
    pub fn from_persisted(value: &ConcreteAccountBalance) -> Self {
        Self(BigInt::from(value.value().clone()))
    }

    /// Borrows the value used by envelope and frame balance arithmetic.
    pub fn value(&self) -> &BigInt {
        &self.0
    }

    /// Returns the balance modulo 2^256 for EVM balance opcodes.
    ///
    /// The pinned Go `uint256.Int::SetFromBig` loads the low magnitude limbs
    /// and applies two's-complement negation when the source is negative.
    pub fn low_word(&self) -> [u8; 32] {
        let (sign, bytes) = self.0.to_bytes_be();
        let mut word = low_u256_word(&BigUint::from_bytes_be(&bytes));
        if sign == Sign::Minus {
            twos_complement_negate(&mut word);
        }
        word
    }

    /// Converts a settled non-negative journal balance to the shared persisted domain.
    pub fn try_to_persisted(&self) -> Result<ConcreteAccountBalance, BalanceConversionError> {
        match self.0.to_bytes_be() {
            (Sign::Minus, _) => Err(BalanceConversionError::Negative),
            (_, bytes) => Ok(ConcreteAccountBalance::new(BigUint::from_bytes_be(&bytes))),
        }
    }
}

/// Failure to project a signed journal balance into an unsigned boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BalanceConversionError {
    /// The reference-compatible journal value is currently below zero.
    Negative,
}

impl std::fmt::Display for BalanceConversionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Negative => formatter.write_str("execution balance is negative"),
        }
    }
}

impl std::error::Error for BalanceConversionError {}

/// Classification of one transaction in the ordered execution stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionTransactionKind {
    /// A call to an externally owned account or bytecode/native contract.
    Call,
    /// Top-level contract creation.
    Create,
    /// Application-generated system transaction.
    System,
}

/// General transaction input consumed by the Taraxa envelope.
///
/// Nonce, gas price and value retain the reference widths. `canonical_rlp` is
/// optional because read-only simulations may not originate from a signed wire
/// envelope. Finalized adapters must supply it and bind `hash` to those bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionTransaction {
    /// Position in the ordered period stream.
    pub position: FinalChainTransactionPosition,
    /// Canonical transaction hash or application-assigned simulation identity.
    pub hash: [u8; 32],
    /// Authoritative sender address.
    pub sender: [u8; 20],
    /// Receiver address; `None` selects top-level creation.
    pub receiver: Option<[u8; 20]>,
    /// Arbitrary-width transaction nonce.
    pub nonce: FinalChainNonce,
    /// Arbitrary-width price used by envelope charging.
    pub gas_price: ExecutionGasPrice,
    /// `u64` transaction gas cap.
    pub gas_limit: FinalChainGas,
    /// Arbitrary-width transferred value.
    pub value: ExecutionValue,
    /// Calldata or creation initcode.
    pub input: Vec<u8>,
    /// Canonical signed/system RLP when the request originates from a transaction.
    pub canonical_rlp: Option<Vec<u8>>,
    /// Call/create/system behavior selected by the application adapter.
    pub kind: ExecutionTransactionKind,
}

/// Immutable block facts used by one transaction execution.
///
/// Profile activation is derived separately from `period`; this type does not
/// select an Ethereum hardfork or grant authority to choose consensus inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionBlockContext {
    /// Finalized period being executed.
    pub period: FinalChainBlockNumber,
    /// Block author exposed by `COINBASE`.
    pub author: [u8; 20],
    /// Block timestamp exposed by `TIMESTAMP`.
    pub timestamp: u64,
    /// Block gas limit exposed by `GASLIMIT`.
    pub gas_limit: FinalChainGas,
    /// Chain identifier exposed by `CHAINID`.
    pub chain_id: u64,
    /// Arbitrary-width difficulty before its explicit EVM-word projection.
    pub difficulty: BigUint,
}

/// One ordered EVM log retained by a successful transaction/frame journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionLog {
    /// Emitting account address.
    pub address: [u8; 20],
    /// Ordered event signature and indexed-argument topics.
    pub topics: Vec<[u8; 32]>,
    /// Unindexed log data.
    pub data: Vec<u8>,
}

/// Code/frame failure after transaction admission succeeded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodeExecutionError {
    /// The frame returned explicit REVERT data and retained unused gas.
    Revert,
    /// The frame exhausted its available gas.
    OutOfGas,
    /// The interpreter encountered an invalid instruction byte.
    InvalidOpcode(u8),
    /// Stack operands were unavailable.
    StackUnderflow,
    /// The EVM stack exceeded its limit.
    StackOverflow,
    /// A state-changing instruction violated effective static execution.
    StaticViolation,
    /// Call/create depth exceeded the Taraxa limit.
    Depth,
    /// CREATE/CREATE2 found nonzero nonce or nonempty code at the target.
    CreateCollision,
    /// Created runtime code exceeded the accepted size.
    ContractSize,
    /// Runtime-code deposit could not be paid from child gas.
    CodeDepositOutOfGas,
    /// A jump destination was invalid.
    InvalidJump,
}

/// Result status for a transaction that passed consensus-envelope admission.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodeExecutionStatus {
    /// Frame execution and settlement succeeded.
    Success,
    /// Frame execution failed after admission; output follows reference rules.
    Failure(CodeExecutionError),
}

/// Completed code/frame execution after envelope admission.
///
/// `gas_used` is settled transaction gas, not child supplied/remaining gas.
/// The separate [`ConsensusFailureResult`] type prevents code failures from
/// sharing an ambiguous error or charging path with admission failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutedTransactionResult {
    /// Code/frame completion status.
    pub status: CodeExecutionStatus,
    /// Settled gas charged to the transaction.
    pub gas_used: FinalChainGas,
    /// Return or revert bytes retained by the reference settlement rule.
    pub output: Vec<u8>,
    /// Address created by a successful top-level creation.
    pub new_contract_address: Option<[u8; 20]>,
    /// Logs surviving frame settlement.
    pub logs: Vec<ExecutionLog>,
}

/// Consensus/envelope failure before a normal code result is accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConsensusFailure {
    /// Sender cannot pay the requested gas cap at the full gas price.
    InsufficientBalanceForGas,
    /// Transaction nonce is below the authoritative account nonce.
    NonceTooLow,
    /// Intrinsic gas calculation overflowed its `u64` domain.
    IntrinsicGasOverflow,
    /// Supplied gas is below intrinsic gas.
    IntrinsicGas,
    /// Caller cannot transfer the requested full-width value.
    InsufficientBalanceForTransfer,
}

/// Charged outcome for a consensus/envelope failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusFailureResult {
    /// Stable failure classification used for compatibility mapping.
    pub error: ConsensusFailure,
    /// Gas charged by the exact pre/post-activation envelope rule.
    pub gas_used: FinalChainGas,
}

/// Mutually exclusive terminal transaction outcomes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionExecutionResult {
    /// Transaction passed admission and produced a code/frame result.
    Executed(ExecutedTransactionResult),
    /// Transaction stopped in consensus/envelope admission.
    ConsensusFailure(ConsensusFailureResult),
}

/// Failure to load a canonical historical block hash from application state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockHashReadError {
    /// The application does not retain the requested canonical block.
    HistoryUnavailable(FinalChainBlockNumber),
    /// Stored history failed structural or hash validation.
    Corrupt(String),
    /// Physical history access failed.
    Io(String),
}

impl std::fmt::Display for BlockHashReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "block hash read: {self:?}")
    }
}

impl std::error::Error for BlockHashReadError {}

/// Application-owned canonical block-hash access for the EVM `BLOCKHASH` opcode.
///
/// The host handles current/future periods and the protocol window as zero
/// without invoking this port. An in-window prior period must return its exact
/// hash or an infrastructure error; missing history must not silently become
/// zero after the host has established that the request is in range.
pub trait BlockHashRead {
    /// Loads one canonical, in-window prior-period hash.
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError>;
}

/// Monotonic identity of a native call within one pending period execution.
///
/// The identity is for ordering and audit facts. It does not authorize retries,
/// cache outcomes or substitute for validating returned raw/domain mutations.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NativeInvocationId {
    /// Ordered transaction containing the call.
    pub transaction: FinalChainTransactionPosition,
    /// Zero-based call sequence across the pending period.
    pub sequence: u64,
}

/// CALL-family operation that reached a native contract address.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeCallKind {
    /// Ordinary CALL.
    Call,
    /// CALLCODE with caller storage/address context.
    CallCode,
    /// DELEGATECALL with inherited caller and value.
    DelegateCall,
    /// STATICCALL. Historical native mutation exceptions remain profile-owned.
    StaticCall,
}

/// Immutable facts used to quote and execute one native invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeInvocation {
    /// Monotonic period-local identity.
    pub id: NativeInvocationId,
    /// Historical period used to select native rules and encodings.
    pub period: FinalChainBlockNumber,
    /// Zero-based frame depth.
    pub depth: u16,
    /// CALL-family operation that selected the contract.
    pub kind: NativeCallKind,
    /// Effective static mode inherited from all enclosing frames.
    pub is_static: bool,
    /// Effective caller supplied to the native business method.
    pub caller: [u8; 20],
    /// Native code/precompile address used for dispatch.
    pub contract: [u8; 20],
    /// Account address whose ordinary state is the call context.
    pub state_address: [u8; 20],
    /// Full call value before EVM-word projection.
    pub value: ExecutionValue,
    /// ABI input bytes.
    pub input: Vec<u8>,
    /// Child/action gas supplied by frame settlement.
    pub supplied_gas: FinalChainGas,
}

/// Side-effect-free gas quote for one exact native invocation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeGasQuote {
    /// Gas charged when the quoted native invocation is admitted.
    pub required_gas: FinalChainGas,
}

/// Contract-level completion of a native business method.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeStatus {
    /// The method succeeded and its returned mutations are valid.
    Success,
    /// The method rejected input/state under normal contract semantics.
    ContractFailure,
}

/// One ordinary balance mutation produced by an existing native kernel.
///
/// Expected and replacement balances let the journal reject stale or duplicate
/// kernel output. These mutations use the ordinary checkpoint lane and therefore
/// revert with their frame even when associated native raw writes survive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeBalanceMutation {
    /// Account whose ordinary balance changes.
    pub address: [u8; 20],
    /// Balance the adapter observed before the mutation.
    pub expected: ExecutionBalance,
    /// Balance after the native business transition.
    pub replacement: ExecutionBalance,
}

/// Exact native raw-storage operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeRawOperation {
    /// Store these exact bytes, including a nonempty all-zero count.
    Put(Vec<u8>),
    /// Delete the row; this is distinct from storing zero bytes.
    Delete,
}

/// One ordered raw mutation emitted by a native business transition.
///
/// `expected` binds the serializer to the raw overlay visible before this
/// operation. It preserves absent/tombstone/value distinctions and prevents an
/// adapter from regenerating untouched rows from a normalized projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeRawMutation {
    /// Native contract whose storage trie owns the row.
    pub address: [u8; 20],
    /// Logical unhashed storage key.
    pub key: ConcreteStorageKey,
    /// Raw value expected before this operation.
    pub expected: ConcreteRead<Vec<u8>>,
    /// Exact put/delete operation to apply in vector order.
    pub operation: NativeRawOperation,
}

/// Completed admitted native invocation.
///
/// The adapter retains its staged semantic kernel state directly. No opaque
/// successor id can replace validation of these account/raw/log facts against
/// that domain state and the concrete projection at the period boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeOutcome {
    /// Contract success/failure; both consume the quoted gas when invoked.
    pub status: NativeStatus,
    /// Gas charged by the native call. It must equal the accepted quote.
    pub gas_used: FinalChainGas,
    /// Contract return bytes.
    pub output: Vec<u8>,
    /// Ordinary balance effects applied through the frame journal.
    pub balance_mutations: Vec<NativeBalanceMutation>,
    /// Ordered historically irreversible raw effects.
    pub raw_mutations: Vec<NativeRawMutation>,
    /// Logs applied through the ordinary frame journal.
    pub logs: Vec<ExecutionLog>,
    /// Optional stable diagnostic for observation only, never a consensus input.
    pub diagnostic: Option<String>,
}

/// Result of the mutating native invocation phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeInvocationResult {
    /// Supplied gas was below the current quote. The port made no semantic,
    /// account, raw or log mutation.
    InsufficientGas {
        /// Current required gas returned for settlement and diagnostics.
        required_gas: FinalChainGas,
    },
    /// The native kernel ran exactly once and returned its complete effects.
    Completed(NativeOutcome),
}

/// Infrastructure or contract-integrity failure at the native port boundary.
///
/// Returning this error aborts the pending period. Implementations must expose
/// no partial semantic or concrete mutation when returning an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativePortError {
    /// Invocation identity was not the next task-local sequence.
    OutOfSequence {
        /// Sequence expected by the staged native session.
        expected: u64,
        /// Sequence supplied by the executor.
        actual: u64,
    },
    /// The quote changed before invocation or did not describe the request.
    QuoteMismatch,
    /// Concrete raw state could not be read safely.
    State(ConcreteReadError),
    /// Native semantic state was unavailable or inconsistent.
    Domain(String),
    /// Physical adapter execution failed.
    Infrastructure(String),
}

impl std::fmt::Display for NativePortError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "native execution port: {self:?}")
    }
}

impl std::error::Error for NativePortError {}

/// Staged FinalChain-native execution used by the EVM frame driver.
///
/// [`NativeExecutionPort::quote`] is read-only. `invoke` must recompute or
/// validate the quote against the same invocation before any side effect. When
/// `supplied_gas < quote.required_gas`, it returns
/// [`NativeInvocationResult::InsufficientGas`] without running the business
/// kernel. A completed call advances the adapter's staged semantic state once;
/// its ordinary balance/log effects are still applied by the EVM journal, while
/// raw effects use the separate historical native lane. Any returned error
/// aborts the pending period and exposes no partial mutation.
pub trait NativeExecutionPort {
    /// Computes required gas without changing semantic or concrete state.
    fn quote(&self, invocation: &NativeInvocation) -> Result<NativeGasQuote, NativePortError>;

    /// Runs one sufficiently funded invocation or reports insufficient gas without mutation.
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
    ) -> Result<NativeInvocationResult, NativePortError>;
}

/// Failure to reconcile a native result with its request and side-effect-free quote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeResultValidationError {
    /// The result classified a funded call as insufficient or an unfunded call as completed.
    AdmissionMismatch,
    /// A completed call did not charge exactly the quoted required gas.
    ChargedGasMismatch,
    /// The insufficient-gas result did not echo the accepted required gas.
    RequiredGasMismatch,
}

impl std::fmt::Display for NativeResultValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "native result validation: {self:?}")
    }
}

impl std::error::Error for NativeResultValidationError {}

impl NativeInvocationResult {
    /// Validates gas admission/charging before the frame applies any returned effect.
    pub fn validate(
        &self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
    ) -> Result<(), NativeResultValidationError> {
        let sufficiently_funded = invocation.supplied_gas >= quote.required_gas;
        match self {
            Self::InsufficientGas { required_gas } => {
                if sufficiently_funded {
                    return Err(NativeResultValidationError::AdmissionMismatch);
                }
                if *required_gas != quote.required_gas {
                    return Err(NativeResultValidationError::RequiredGasMismatch);
                }
            }
            Self::Completed(outcome) => {
                if !sufficiently_funded {
                    return Err(NativeResultValidationError::AdmissionMismatch);
                }
                if outcome.gas_used != quote.required_gas {
                    return Err(NativeResultValidationError::ChargedGasMismatch);
                }
            }
        }
        Ok(())
    }
}

fn low_u256_word(value: &BigUint) -> [u8; 32] {
    let bytes = value.to_bytes_be();
    let low = &bytes[bytes.len().saturating_sub(32)..];
    let mut word = [0_u8; 32];
    word[32 - low.len()..].copy_from_slice(low);
    word
}

fn twos_complement_negate(word: &mut [u8; 32]) {
    for byte in word.iter_mut() {
        *byte = !*byte;
    }
    for byte in word.iter_mut().rev() {
        let (next, overflow) = byte.overflowing_add(1);
        *byte = next;
        if !overflow {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gas(value: u64) -> FinalChainGas {
        FinalChainGas::new(value)
    }

    fn native_invocation(supplied_gas: u64) -> NativeInvocation {
        NativeInvocation {
            id: NativeInvocationId {
                transaction: FinalChainTransactionPosition::from(0_u32),
                sequence: 0,
            },
            period: FinalChainBlockNumber::new(1),
            depth: 1,
            kind: NativeCallKind::Call,
            is_static: false,
            caller: [0xaa; 20],
            contract: [0xfe; 20],
            state_address: [0xfe; 20],
            value: ExecutionValue::default(),
            input: Vec::new(),
            supplied_gas: gas(supplied_gas),
        }
    }

    #[test]
    fn opcode_projection_retains_the_low_256_bits_of_wide_values() {
        let wide: BigUint = (BigUint::from(1_u8) << 256_usize) + BigUint::from(0xabu8);
        let gas_price = ExecutionGasPrice::new(wide.clone());
        let value = ExecutionValue::new(wide);
        let mut expected = [0_u8; 32];
        expected[31] = 0xab;
        assert_eq!(gas_price.low_word(), expected);
        assert_eq!(value.low_word(), expected);
    }

    #[test]
    fn negative_zero_sender_intermediate_projects_but_cannot_be_persisted_unsigned() {
        let balance = ExecutionBalance::new(BigInt::from(-1));
        assert_eq!(balance.low_word(), [0xff; 32]);
        assert_eq!(
            balance.try_to_persisted(),
            Err(BalanceConversionError::Negative)
        );
    }

    #[test]
    fn native_result_validation_separates_gas_admission_from_execution() {
        let quote = NativeGasQuote {
            required_gas: gas(20_000),
        };
        let insufficient = NativeInvocationResult::InsufficientGas {
            required_gas: gas(20_000),
        };
        assert!(
            insufficient
                .validate(&native_invocation(19_999), quote)
                .is_ok()
        );
        assert_eq!(
            insufficient.validate(&native_invocation(20_000), quote),
            Err(NativeResultValidationError::AdmissionMismatch)
        );

        let completed = NativeInvocationResult::Completed(NativeOutcome {
            status: NativeStatus::ContractFailure,
            gas_used: gas(20_000),
            output: Vec::new(),
            balance_mutations: Vec::new(),
            raw_mutations: Vec::new(),
            logs: Vec::new(),
            diagnostic: None,
        });
        assert!(
            completed
                .validate(&native_invocation(20_000), quote)
                .is_ok()
        );
        assert_eq!(
            completed.validate(&native_invocation(19_999), quote),
            Err(NativeResultValidationError::AdmissionMismatch)
        );
    }

    #[test]
    fn code_and_consensus_failures_have_distinct_result_shapes() {
        let code = TransactionExecutionResult::Executed(ExecutedTransactionResult {
            status: CodeExecutionStatus::Failure(CodeExecutionError::Revert),
            gas_used: gas(21_001),
            output: vec![0xab],
            new_contract_address: None,
            logs: Vec::new(),
        });
        let consensus = TransactionExecutionResult::ConsensusFailure(ConsensusFailureResult {
            error: ConsensusFailure::NonceTooLow,
            gas_used: gas(60_000),
        });
        assert!(matches!(code, TransactionExecutionResult::Executed(_)));
        assert!(matches!(
            consensus,
            TransactionExecutionResult::ConsensusFailure(_)
        ));
    }
}
