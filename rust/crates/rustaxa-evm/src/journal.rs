//! Transaction-local concrete execution state and rollback rules.
//!
//! The journal keeps ordinary account/storage changes, native raw bytes and
//! transient state in distinct lanes because the pinned Taraxa EVM gives them
//! different read, rollback and persistence behavior. It reads immutable state
//! through [`ConcreteExecutionRead`] and emits a typed write plan; it never writes a
//! database or publishes a FinalChain generation.

use std::collections::{BTreeMap, BTreeSet};

/// Test-only observations of the existing slashing-list read branch.
#[cfg(feature = "synthetic-fixtures")]
pub mod cleanup_diagnostic_observer {
    use rustaxa_types::concrete_state::{ConcreteRead, ConcreteReadError, ConcreteStorageKey};
    use std::cell::Cell;
    use std::fmt::{self, Write};
    use std::marker::PhantomData;
    use std::rc::Rc;

    const MAX_DETAIL: usize = 1024;
    const MAX_VALUE: usize = 128;
    #[derive(Clone, Copy)]
    struct Hooks {
        active: fn() -> bool,
        event: fn(&str, &str),
        overflow: fn(),
        origin: bool,
    }
    thread_local! { static HOOKS: Cell<Option<Hooks>> = const { Cell::new(None) }; }
    /// Removes one thread-local observer when dropped.
    pub struct Guard {
        _same_thread: PhantomData<Rc<()>>,
    }
    /// Install one observer on the current thread while its collector is active.
    pub fn install(
        active: fn() -> bool,
        event: fn(&str, &str),
        overflow: fn(),
        origin: bool,
    ) -> Result<Guard, &'static str> {
        if !active() {
            return Err("diagnostic collector is not active");
        }
        HOOKS.with(|slot| {
            if slot.get().is_some() {
                return Err("diagnostic observer already installed");
            }
            slot.set(Some(Hooks {
                active,
                event,
                overflow,
                origin,
            }));
            Ok(Guard {
                _same_thread: PhantomData,
            })
        })
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            HOOKS.with(|slot| slot.set(None));
        }
    }
    struct Bounded {
        text: String,
    }
    impl Write for Bounded {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            if value.len() > MAX_DETAIL - self.text.len() {
                return Err(fmt::Error);
            }
            self.text.push_str(value);
            Ok(())
        }
    }
    pub(super) fn target(address: [u8; 20], key: ConcreteStorageKey) -> bool {
        address[..19] == [0_u8; 19]
            && address[19] == 0xee
            && key.0[..31] == [0_u8; 31]
            && key.0[31] == 2
    }
    fn active_hooks() -> Option<Hooks> {
        HOOKS
            .with(|slot| slot.get())
            .filter(|hooks| (hooks.active)())
    }
    fn write_result(hooks: Hooks, branch: &str, result: &impl fmt::Debug) {
        let mut output = Bounded {
            text: String::with_capacity(MAX_DETAIL),
        };
        if write!(&mut output, "scope=active address=00000000000000000000000000000000000000ee key=0000000000000000000000000000000000000000000000000000000000000002 branch={branch} result={result:?}").is_err() {
            (hooks.overflow)();
        } else {
            (hooks.event)("evm_raw_storage", &output.text);
        }
    }
    pub(super) fn value(branch: &str, value: &ConcreteRead<Vec<u8>>) {
        let Some(hooks) = active_hooks() else {
            return;
        };
        if matches!(value, ConcreteRead::Present(bytes) if bytes.len() > MAX_VALUE) {
            (hooks.overflow)();
            return;
        }
        write_result(hooks, branch, value);
    }
    pub(super) fn origin(branch: &str, value: &impl fmt::Debug) {
        if let Some(hooks) = active_hooks().filter(|hooks| hooks.origin) {
            write_result(hooks, branch, value);
        }
    }
    pub(super) fn active() -> bool {
        active_hooks().is_some()
    }
    pub(super) fn absent_account_raw(overlay: bool, operation: Option<&super::NativeRawOperation>) {
        let Some(hooks) = active_hooks() else {
            return;
        };
        let source = if overlay { "overlay" } else { "backing" };
        let mut output = Bounded {
            text: String::with_capacity(MAX_DETAIL),
        };
        let result = match operation {
            Some(super::NativeRawOperation::Put(value)) => {
                let bytes = value.as_bytes();
                if bytes.len() > MAX_VALUE {
                    (hooks.overflow)();
                    return;
                }
                write!(
                    &mut output,
                    "source={source} exists=false raw=put bytes={bytes:?}"
                )
            }
            Some(super::NativeRawOperation::Delete) => {
                write!(&mut output, "source={source} exists=false raw=delete")
            }
            None => write!(&mut output, "source={source} exists=false raw=missing"),
        };
        if result.is_err() {
            (hooks.overflow)();
        } else {
            (hooks.event)("evm_absent_account_raw", &output.text);
        }
    }
    pub(super) fn error(branch: &str, error: &ConcreteReadError) {
        let Some(hooks) = active_hooks() else {
            return;
        };
        if matches!(error, ConcreteReadError::Corrupt(message) | ConcreteReadError::Io(message) if message.len() > MAX_VALUE)
        {
            (hooks.overflow)();
            return;
        }
        write_result(hooks, branch, error);
    }
}

use num_bigint::{BigInt, BigUint};
use rustaxa_types::{
    FinalChainNonce,
    concrete_state::{ConcreteAccountBalance, ConcreteRead, ConcreteReadError, ConcreteStorageKey},
};

use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;

use crate::contracts::{
    BalanceConversionError, ConsensusNativeDisposition, ConsensusNativeObservation,
    ExecutionBalance, ExecutionLog, NativeJournalAccount, NativeJournalRead,
    NativeJournalReadError, NativeOrdinaryAccountMutation, NativeRawOperation,
};

/// Unhashed EVM account address.
pub type JournalAddress = [u8; 20];
/// One bounded observation of an actual ordinary journal mutation or restore.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DurationJournalEvent {
    pub kind: &'static str,
    pub address: JournalAddress,
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}
struct DurationJournalCapture {
    events: Vec<DurationJournalEvent>,
    charge: usize,
    cap: usize,
    overflow: bool,
}
thread_local! {
    static DURATION_JOURNAL_CAPTURE: std::cell::RefCell<Option<DurationJournalCapture>> = const { std::cell::RefCell::new(None) };
}
/// Arms one synchronous owner observation with a finite charge cap.
pub fn duration_journal_arm(cap: usize) -> Result<(), &'static str> {
    if cap == 0 {
        return Err("zero journal capture cap");
    }
    DURATION_JOURNAL_CAPTURE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_some() {
            return Err("journal capture already armed");
        }
        *slot = Some(DurationJournalCapture {
            events: Vec::new(),
            charge: 0,
            cap,
            overflow: false,
        });
        Ok(())
    })
}
/// Takes complete observations; overflow and missing ownership fail closed.
pub fn duration_journal_take() -> Result<Vec<DurationJournalEvent>, &'static str> {
    DURATION_JOURNAL_CAPTURE.with(|cell| {
        let capture = cell
            .borrow_mut()
            .take()
            .ok_or("journal capture not armed")?;
        if capture.overflow {
            return Err("journal capture cap exceeded");
        }
        Ok(capture.events)
    })
}
fn duration_journal_event(kind: &'static str, address: JournalAddress, key: &[u8], value: &[u8]) {
    DURATION_JOURNAL_CAPTURE.with(|cell| {
        if let Some(capture) = cell.borrow_mut().as_mut() {
            let Some(charge) = key
                .len()
                .checked_add(value.len())
                .and_then(|n| n.checked_add(256))
                .and_then(|n| n.checked_add(capture.charge))
            else {
                capture.overflow = true;
                return;
            };
            if charge > capture.cap {
                capture.overflow = true;
                return;
            }
            if capture.events.try_reserve_exact(1).is_err() {
                capture.overflow = true;
                return;
            }
            let unused = capture
                .events
                .capacity()
                .saturating_sub(capture.events.len().saturating_add(1));
            let Some(peak) = unused
                .checked_mul(std::mem::size_of::<DurationJournalEvent>())
                .and_then(|n| charge.checked_add(n))
            else {
                capture.overflow = true;
                return;
            };
            if peak > capture.cap {
                capture.overflow = true;
                return;
            }
            capture.charge = charge;
            capture.events.push(DurationJournalEvent {
                kind,
                address,
                key: key.to_vec(),
                value: value.to_vec(),
            });
        }
    });
}
/// Reports whether a bounded duration observation is active on this thread.
pub fn duration_journal_is_armed() -> bool {
    DURATION_JOURNAL_CAPTURE.with(|cell| cell.borrow().is_some())
}
fn duration_native_field(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value);
}
fn duration_native_signed(out: &mut Vec<u8>, value: &BigInt) {
    let (sign, magnitude) = value.to_bytes_be();
    out.push(match sign {
        num_bigint::Sign::Minus => 0,
        num_bigint::Sign::NoSign => 1,
        num_bigint::Sign::Plus => 2,
    });
    duration_native_field(
        out,
        if sign == num_bigint::Sign::NoSign {
            &[]
        } else {
            &magnitude
        },
    );
}
pub(crate) fn duration_native_insufficient_gas() {
    duration_journal_event("outcome_insufficient_gas", [0; 20], &[], &[]);
}
pub(crate) fn duration_native_outcome_status(
    invocation: &crate::contracts::NativeInvocation,
    status: &crate::contracts::NativeStatus,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let tag = match status {
        crate::contracts::NativeStatus::Success => 0,
        crate::contracts::NativeStatus::ContractFailure(_) => 1,
    };
    duration_journal_event(
        "outcome_status",
        [0; 20],
        &duration_native_identity(invocation),
        &[1, tag],
    );
}

/// Marks the limits of one native outcome record.
pub fn duration_native_outcome_boundary(begin: bool) {
    duration_journal_event(
        if begin {
            "outcome_begin"
        } else {
            "outcome_end"
        },
        [0; 20],
        &[],
        &[],
    );
}
fn duration_native_account_encoding(
    mutation: &NativeOrdinaryAccountMutation,
) -> (JournalAddress, Vec<u8>) {
    let mut value = Vec::new();
    let address = match mutation {
        NativeOrdinaryAccountMutation::Nonce {
            address,
            expected_exists,
            expected,
            replacement,
        } => {
            value.extend([0, u8::from(*expected_exists)]);
            duration_native_field(&mut value, &expected.to_bytes());
            duration_native_field(&mut value, &replacement.to_bytes());
            *address
        }
        NativeOrdinaryAccountMutation::EnsureExists {
            address,
            expected_exists,
        } => {
            value.extend([1, u8::from(*expected_exists)]);
            *address
        }
        NativeOrdinaryAccountMutation::Touch {
            address,
            expected_exists,
        } => {
            value.extend([2, u8::from(*expected_exists)]);
            *address
        }
        NativeOrdinaryAccountMutation::Balance {
            address,
            expected_exists,
            expected,
            replacement,
        } => {
            value.extend([3, u8::from(*expected_exists)]);
            duration_native_signed(&mut value, expected.value());
            duration_native_signed(&mut value, replacement.value());
            *address
        }
    };
    (address, value)
}

/// Records an exact native account outcome or successful application.
pub fn duration_native_account_event(kind: &'static str, mutation: &NativeOrdinaryAccountMutation) {
    if !duration_journal_is_armed() {
        return;
    }
    let (address, value) = duration_native_account_encoding(mutation);
    duration_journal_event(kind, address, &[], &value);
}

fn duration_native_raw_encoding(mutation: &crate::contracts::NativeRawMutation) -> Vec<u8> {
    let mut value = Vec::new();
    match &mutation.expected {
        ConcreteRead::Absent => value.push(0),
        ConcreteRead::Tombstone => value.push(1),
        ConcreteRead::Present(v) => {
            value.push(2);
            duration_native_field(&mut value, v);
        }
    }
    match &mutation.operation {
        NativeRawOperation::Delete => value.push(0),
        NativeRawOperation::Put(v) => {
            value.push(1);
            duration_native_field(&mut value, v.as_bytes());
        }
    }
    value
}

/// Records an exact native raw outcome or successful application.
pub fn duration_native_raw_event(
    kind: &'static str,
    mutation: &crate::contracts::NativeRawMutation,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let value = duration_native_raw_encoding(mutation);
    duration_journal_event(kind, mutation.address, &mutation.key.0, &value);
}

fn duration_native_application_header(
    invocation: &crate::contracts::NativeInvocation,
    ordinal: u64,
) -> Vec<u8> {
    let mut value = vec![1];
    value.extend_from_slice(&duration_native_identity(invocation));
    value.extend_from_slice(&ordinal.to_be_bytes());
    value
}

/// Records the boundary of one actual native account application.
/// The native invocation and vector position identify the operation. The end
/// record is emitted only after its successful applied-account record.
pub(crate) fn duration_native_apply_account_event(
    begin: bool,
    invocation: &crate::contracts::NativeInvocation,
    ordinal: u64,
    mutation: &NativeOrdinaryAccountMutation,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let (address, body) = duration_native_account_encoding(mutation);
    let mut value = duration_native_application_header(invocation, ordinal);
    value.extend_from_slice(&body);
    duration_journal_event(
        if begin {
            "native_apply_account_begin"
        } else {
            "native_apply_account_end"
        },
        address,
        &[],
        &value,
    );
}

/// Records the boundary of one actual native raw application.
/// Begin precedes the expected-value read. End follows the successful raw write
/// and applied-raw record; failed operations retain only their partial trace.
pub(crate) fn duration_native_apply_raw_event(
    begin: bool,
    invocation: &crate::contracts::NativeInvocation,
    ordinal: u64,
    mutation: &crate::contracts::NativeRawMutation,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let body = duration_native_raw_encoding(mutation);
    let mut value = duration_native_application_header(invocation, ordinal);
    value.extend_from_slice(&body);
    duration_journal_event(
        if begin {
            "native_apply_raw_begin"
        } else {
            "native_apply_raw_end"
        },
        mutation.address,
        &mutation.key.0,
        &value,
    );
}

/// Marks the actual terminal rewards operation for duration observation.
pub fn duration_journal_reward_boundary(begin: bool, period: u64) {
    duration_journal_event(
        if begin {
            "reward_stage_begin"
        } else {
            "reward_stage_end"
        },
        [0; 20],
        &period.to_be_bytes(),
        &[],
    );
}
/// Records one actual native staging observation while duration capture is armed.
pub fn duration_journal_stage_event(
    kind: &'static str,
    address: JournalAddress,
    key: &[u8],
    value: &[u8],
) {
    assert!(matches!(
        kind,
        "reward_disabled"
            | "reward_phase"
            | "reward_map"
            | "reward_fee_source"
            | "reward_validator_exists"
            | "prepare_begin"
            | "prepare_end"
            | "invoke_begin"
            | "invoke_end"
            | "stage_account_read"
            | "stage_account_probe"
            | "stage_account_write"
            | "stage_raw_read"
            | "stage_raw_probe"
            | "stage_raw_write"
            | "slashing_delayed_list"
            | "slashing_current_get"
    ));
    duration_journal_event(kind, address, key, value);
}
/// Records an exact delta supplied to one unpublished observer phase.
pub fn duration_journal_plan_event(
    kind: &'static str,
    address: JournalAddress,
    key: &[u8],
    value: &[u8],
) {
    assert!(matches!(
        kind,
        "source_account"
            | "source_storage"
            | "source_code"
            | "plan_begin"
            | "plan_end"
            | "plan_account"
            | "plan_storage"
            | "plan_code"
            | "prepared_plan_account"
            | "prepared_storage"
            | "prepared_code"
    ));
    duration_journal_event(kind, address, key, value);
}
pub(crate) fn duration_journal_read_role(begin: bool, role: u8, address: JournalAddress) {
    duration_journal_event(
        if begin {
            "read_role_begin"
        } else {
            "read_role_end"
        },
        address,
        &[role],
        &[],
    );
}

pub(crate) fn duration_journal_ordinary_metadata(
    kind: &'static str,
    sender: JournalAddress,
    target: JournalAddress,
    amount: &BigUint,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let mut key = 0_u64.to_be_bytes().to_vec();
    key.push(0xf1);
    key.extend_from_slice(&target);
    key.extend_from_slice(&target);
    let mut value = sender.to_vec();
    value.extend_from_slice(amount.to_string().as_bytes());
    duration_journal_event(kind, target, &key, &value);
}

pub(crate) fn duration_journal_admission_terminal(sender: JournalAddress, reason: u8) {
    duration_journal_event("admission_terminal", sender, &[reason], &[]);
}

pub(crate) struct DurationTransferDecisionObservation<'a> {
    pub depth: u64,
    pub opcode: u8,
    pub sender: JournalAddress,
    pub target: JournalAddress,
    pub state_target: JournalAddress,
    pub amount: &'a BigUint,
    pub balance: Option<&'a BigInt>,
    pub checked: bool,
    pub allowed: bool,
}

pub(crate) fn duration_journal_transfer_decision(
    observation: DurationTransferDecisionObservation<'_>,
) {
    if !duration_journal_is_armed() {
        return;
    }
    let mut key = observation.depth.to_be_bytes().to_vec();
    key.push(observation.opcode);
    key.extend_from_slice(&observation.target);
    key.extend_from_slice(&observation.state_target);
    let value = format!(
        "{}|{}|{}|{}",
        observation.amount,
        observation.checked,
        observation
            .balance
            .map_or_else(|| "-".to_owned(), ToString::to_string),
        observation.allowed
    );
    duration_journal_event(
        "transfer_decision",
        observation.sender,
        &key,
        value.as_bytes(),
    );
}

/// Records a validated unpublished observer phase boundary.
pub fn duration_journal_prepared_stage(begin: bool, root: [u8; 32]) {
    duration_journal_event(
        if begin {
            "prepared_begin"
        } else {
            "prepared_end"
        },
        [0; 20],
        &root,
        &[],
    );
}
/// Records one exact account row emitted by the unpublished observer phase.
pub fn duration_journal_prepared_account(
    address: JournalAddress,
    root: [u8; 32],
    class: u8,
    row: &[u8],
) {
    let mut value = vec![class];
    value.extend_from_slice(row);
    duration_journal_event("prepared_account", address, &root, &value);
}
pub(crate) fn duration_journal_no_refund(checkpoint: JournalCheckpoint, success: bool, reason: u8) {
    duration_journal_event(
        "no_refund_frame",
        [0; 20],
        &checkpoint.0.to_be_bytes(),
        &[u8::from(success), reason],
    );
}
pub(crate) fn duration_journal_refund_root(checkpoint: JournalCheckpoint, delta: i64) {
    duration_journal_event(
        "root_refund",
        [0; 20],
        &checkpoint.0.to_be_bytes(),
        &delta.to_be_bytes(),
    );
}
pub(crate) fn duration_journal_refund_frame(
    begin: bool,
    depth: usize,
    checkpoint: JournalCheckpoint,
    parent_before: i64,
    child: i64,
    parent_after: i64,
    success: bool,
) {
    let mut key = (depth as u64).to_be_bytes().to_vec();
    key.extend_from_slice(&checkpoint.0.to_be_bytes());
    let mut value = parent_before.to_be_bytes().to_vec();
    value.extend_from_slice(&child.to_be_bytes());
    value.extend_from_slice(&parent_after.to_be_bytes());
    value.push(u8::from(success));
    duration_journal_event(
        if begin { "frame_begin" } else { "frame_end" },
        [0; 20],
        &key,
        &value,
    );
}
pub(crate) fn duration_journal_storage_consumer(
    original: bool,
    address: JournalAddress,
    key: ConcreteStorageKey,
    value: &BigUint,
) {
    let key_bytes = key
        .0
        .iter()
        .position(|byte| *byte != 0)
        .map_or(&key.0[32..], |first| &key.0[first..]);
    let bytes = if *value == BigUint::default() {
        Vec::new()
    } else {
        value.to_bytes_be()
    };
    duration_journal_event(
        if original {
            "consume_storage_original"
        } else {
            "consume_storage_current"
        },
        address,
        key_bytes,
        &bytes,
    );
}
pub(crate) struct DurationNativeScope {
    key: Vec<u8>,
}
impl Drop for DurationNativeScope {
    fn drop(&mut self) {
        duration_journal_event("native_end", [0; 20], &self.key, &[]);
    }
}
fn duration_native_identity(invocation: &crate::contracts::NativeInvocation) -> Vec<u8> {
    let mut key = u64::from(invocation.id.transaction.as_u32())
        .to_be_bytes()
        .to_vec();
    key.extend_from_slice(&invocation.id.sequence.to_be_bytes());
    key.extend_from_slice(&invocation.depth.to_be_bytes());
    key
}
pub(crate) fn duration_journal_native_scope(
    invocation: &crate::contracts::NativeInvocation,
) -> DurationNativeScope {
    let key = duration_native_identity(invocation);
    duration_journal_event("native_begin", [0; 20], &key, &[]);
    DurationNativeScope { key }
}
pub(crate) fn duration_journal_consumer(begin: bool, depth: usize, pc: u64, opcode: u8) {
    let mut key = (depth as u64).to_be_bytes().to_vec();
    key.extend_from_slice(&pc.to_be_bytes());
    key.push(opcode);
    duration_journal_event(
        if begin {
            "consumer_begin"
        } else {
            "consumer_end"
        },
        [0; 20],
        &key,
        &[],
    );
}
pub(crate) fn duration_journal_refund_opcode(
    depth: usize,
    pc: u64,
    opcode: u8,
    before: i64,
    after: i64,
    step_ok: bool,
) {
    let mut key = (depth as u64).to_be_bytes().to_vec();
    key.extend_from_slice(&pc.to_be_bytes());
    key.push(opcode);
    let mut value = before.to_be_bytes().to_vec();
    value.extend_from_slice(&after.to_be_bytes());
    value.push(u8::from(step_ok));
    duration_journal_event("refund_opcode", [0; 20], &key, &value);
}
fn duration_journal_log(log: &ExecutionLog) -> Vec<u8> {
    let mut out = (log.topics.len() as u32).to_be_bytes().to_vec();
    for topic in &log.topics {
        out.extend_from_slice(topic);
    }
    out.extend_from_slice(&log.data);
    out
}
fn duration_journal_signed(value: &BigInt) -> Vec<u8> {
    let (sign, bytes) = value.to_bytes_be();
    let mut out = vec![match sign {
        num_bigint::Sign::Minus => 0,
        num_bigint::Sign::NoSign => 1,
        num_bigint::Sign::Plus => 2,
    }];
    if sign != num_bigint::Sign::NoSign {
        out.extend(bytes);
    }
    out
}

/// Opaque token for the current top frame checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalCheckpoint(u64);

/// Failure to read, mutate, settle or flush journal state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalError {
    /// Lifecycle evidence was incomplete or inconsistent.
    InvalidLifecycleReceipt,
    /// Immutable concrete state could not be read safely.
    State(ConcreteReadError),
    /// A checkpoint was committed/reverted out of stack order.
    CheckpointOrder,
    /// Transaction settlement was requested with open frame checkpoints.
    OpenCheckpoints,
    /// A settled account balance cannot cross the unsigned persistence boundary.
    Balance(BalanceConversionError),
    /// Refund arithmetic exceeded its unsigned domain.
    RefundOverflow,
    /// A signed interpreter delta would make the cumulative refund negative.
    RefundUnderflow,
    /// The reference rejects replacing a nonce with a smaller value.
    NonceDecrease,
    /// Native ordinary output was not based on the current journal state.
    NativeExpectation {
        /// Account whose expected state was stale.
        address: JournalAddress,
        /// Field whose expectation did not match.
        field: NativeAccountField,
    },
    /// A live account references code without a physical hash.
    MissingCodeHash {
        /// Account containing the invalid reference.
        address: JournalAddress,
    },
    /// Referenced code bytes are absent or tombstoned.
    ReferencedCodeMissing {
        /// Account containing the reference.
        address: JournalAddress,
        /// Missing code hash.
        code_hash: [u8; 32],
    },
    /// Referenced code length differs from the declared account size.
    CodeSizeMismatch {
        /// Account containing the reference.
        address: JournalAddress,
        /// Declared size.
        declared: u64,
        /// Loaded size.
        actual: usize,
    },
    /// Referenced code bytes do not hash to the account's code hash.
    CodeHashMismatch {
        /// Account containing the reference.
        address: JournalAddress,
        /// Declared code hash.
        declared: [u8; 32],
        /// Hash of loaded bytes.
        actual: [u8; 32],
    },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "execution journal: {self:?}")
    }
}

impl std::error::Error for JournalError {}

impl From<ConcreteReadError> for JournalError {
    fn from(error: ConcreteReadError) -> Self {
        Self::State(error)
    }
}

/// Account field used to classify a stale native-kernel mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAccountField {
    /// Account existence/touch state.
    Existence,
    /// Signed execution balance.
    Balance,
    /// Arbitrary-width nonce.
    Nonce,
}

/// Current account metadata needed by the interpreter host.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalAccountMetadata {
    /// Whether the account currently exists.
    pub exists: bool,
    /// Exact arbitrary-width nonce retained outside REVM account metadata.
    pub nonce: FinalChainNonce,
    /// Signed arbitrary-width balance; EVM opcodes use its low word.
    pub balance: ExecutionBalance,
    /// Physical/staged code hash; absent differs from empty-code hash.
    pub code_hash: Option<[u8; 32]>,
    /// Declared code byte length.
    pub code_size: u64,
}

/// Final ordinary account operation produced by transaction settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalAccountOperation {
    /// Persist the settled account fields; storage/code roots remain sink-owned.
    Upsert {
        /// Arbitrary-width account nonce.
        nonce: FinalChainNonce,
        /// Settled non-negative balance.
        balance: ConcreteAccountBalance,
        /// Reachable code hash, if the account has code.
        code_hash: Option<[u8; 32]>,
        /// Exact reference code size.
        code_size: u64,
    },
    /// Remove a modified suicided or EIP-161-empty account.
    Delete,
}

/// One deterministic account write in a transaction write plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalAccountWrite {
    /// Account receiving the operation.
    pub address: JournalAddress,
    /// Upsert or deletion selected by lifecycle settlement.
    pub operation: JournalAccountOperation,
}

/// One ordinary semantic storage write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalStorageWrite {
    /// Account whose storage trie owns the slot.
    pub address: JournalAddress,
    /// Logical unhashed slot key.
    pub key: ConcreteStorageKey,
    /// Arbitrary-width reference value; zero is deleted by the sink.
    pub value: BigUint,
}

/// One native raw write applied after all ordinary storage writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRawWrite {
    /// Account whose storage trie owns the slot.
    pub address: JournalAddress,
    /// Logical unhashed slot key.
    pub key: ConcreteStorageKey,
    /// Exact put/delete operation.
    pub operation: NativeRawOperation,
}

/// Immutable code bytes installed by a successful creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalCodeWrite {
    /// Hash under which the unversioned code row is stored.
    pub code_hash: [u8; 32],
    /// Exact runtime code bytes.
    pub code: Vec<u8>,
}

/// Ordered persistence inputs emitted by a settled transaction.
///
/// A sink must apply `ordinary_storage` first and `raw_storage` second. This
/// makes raw bytes win when both lanes touch the same logical key. This plan is
/// a single-transaction contract and does not define same-block cache behavior.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JournalWritePlan {
    /// Deterministic account operations.
    pub accounts: Vec<JournalAccountWrite>,
    /// Deterministic ordinary slot operations.
    pub ordinary_storage: Vec<JournalStorageWrite>,
    /// Deterministic raw slot operations, applied last.
    pub raw_storage: Vec<JournalRawWrite>,
    /// Immutable code rows referenced by account upserts.
    pub code: Vec<JournalCodeWrite>,
}

/// Receipt facts plus persistence inputs returned at a transaction boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettledTransaction {
    /// Logs surviving all frame checkpoints, in emission order.
    pub logs: Vec<ExecutionLog>,
    /// Final transaction refund before the caller applies its gas cap rule.
    pub refund: u64,
    /// Ordinary-then-raw persistence plan.
    pub writes: JournalWritePlan,
    /// Ordered consensus-native facts, including rolled-back attempts/logs.
    /// These are observations, not a committed-state projection or replay permit.
    pub native_invocations: Vec<ConsensusNativeObservation>,
    /// Executor-owned lifecycle evidence, sealed at settlement.
    pub lifecycle_receipt: Option<JournalLifecycleReceipt>,
}

#[derive(Clone, Debug)]
struct JournalAccount {
    exists: bool,
    suicided: bool,
    mod_count: u32,
    times_touched: u32,
    nonce: FinalChainNonce,
    balance: ExecutionBalance,
    storage_root: Option<[u8; 32]>,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
    code: Option<Vec<u8>>,
}

fn duration_journal_account_facts(account: &JournalAccount) -> Vec<u8> {
    if !account.exists {
        return vec![0];
    }
    let mut out = vec![1];
    fn append_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
        out.extend_from_slice(bytes);
    }
    append_bytes(&mut out, &account.nonce.to_bytes());
    append_bytes(&mut out, &duration_journal_signed(account.balance.value()));
    out.extend_from_slice(&account.code_size.to_be_bytes());
    for value in [account.code_hash, account.storage_root] {
        match value {
            Some(bytes) => {
                out.push(1);
                out.extend_from_slice(&bytes);
            }
            None => out.push(0),
        }
    }
    out.push(u8::from(account.suicided));
    out
}

impl JournalAccount {
    fn empty(&self) -> bool {
        self.nonce.is_zero() && self.balance.value() == &BigInt::default() && self.code_size == 0
    }
}

#[derive(Clone, Debug)]
struct StorageCell {
    original: BigUint,
    current: BigUint,
    dirty: bool,
}

/// One journal-owned lifecycle fact. Event positions identify frames and creations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalLifecycleEvent {
    /// A new checkpoint was opened.
    FrameOpened { parent: Option<usize> },
    /// An absent account was created in the current frame, if any.
    AccountCreated {
        address: JournalAddress,
        frame: Option<usize>,
        saved_absent: bool,
    },
    /// Native preparation and application have started.
    NativeStarted {
        id: crate::contracts::NativeInvocationId,
    },
    /// A completed native observation was retained.
    NativeObserved {
        id: crate::contracts::NativeInvocationId,
    },
    /// All frame checkpoints were closed at transaction settlement.
    TransactionEnd { observations: usize },
    /// Creation was undone and its raw overlay was cleared.
    AccountReset {
        address: JournalAddress,
        creation: usize,
        rollback_frame: usize,
    },
}

/// Process-local identity for one receipt-producing execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalReceiptToken(u64);

static NEXT_RECEIPT_TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Journal-owned lifecycle evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalLifecycleReceipt {
    token: JournalReceiptToken,
    events: Vec<JournalLifecycleEvent>,
}

/// Checks the token saved before execution against a sealed receipt.
pub fn validate_lifecycle_receipt_token(
    expected: JournalReceiptToken,
    receipt: &JournalLifecycleReceipt,
) -> Result<(), JournalError> {
    if receipt.token != expected {
        return Err(JournalError::InvalidLifecycleReceipt);
    }
    Ok(())
}

impl JournalLifecycleReceipt {
    /// Returns the immutable execution token.
    pub fn token(&self) -> JournalReceiptToken {
        self.token
    }

    /// Returns the process-local receipt identity.
    pub fn token_value(&self) -> u64 {
        self.token.0
    }

    /// Returns the immutable event stream.
    pub fn events(&self) -> &[JournalLifecycleEvent] {
        &self.events
    }
    fn seal(
        token: JournalReceiptToken,
        events: Vec<JournalLifecycleEvent>,
        observations: &[ConsensusNativeObservation],
    ) -> Result<Self, JournalError> {
        let mut frames = BTreeMap::new();
        let mut creations = BTreeMap::new();
        let mut consumed = BTreeSet::new();
        let mut active = None;
        let mut seen = 0usize;
        let mut last_id = None;
        let mut ended = false;
        for (ordinal, event) in events.iter().enumerate() {
            if ended {
                return Err(JournalError::InvalidLifecycleReceipt);
            }
            match event {
                JournalLifecycleEvent::FrameOpened { parent } => {
                    if active.is_some() {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    if parent.is_some_and(|id| !frames.contains_key(&id)) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    frames.insert(ordinal, *parent);
                }
                JournalLifecycleEvent::AccountCreated {
                    address,
                    frame,
                    saved_absent,
                } => {
                    if !saved_absent || frame.is_some_and(|id| !frames.contains_key(&id)) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    creations.insert(ordinal, (*address, *frame));
                }
                JournalLifecycleEvent::AccountReset {
                    address,
                    creation,
                    rollback_frame,
                } => {
                    if active.is_some() || !consumed.insert(*creation) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    let Some((owner, mut frame)) = creations.get(creation).copied() else {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    };
                    if owner != *address || !frames.contains_key(rollback_frame) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    while let Some(id) = frame {
                        if id == *rollback_frame {
                            break;
                        }
                        frame = frames.get(&id).copied().flatten();
                    }
                    if frame != Some(*rollback_frame) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                }
                JournalLifecycleEvent::NativeStarted { id } => {
                    if active.is_some() || last_id.is_some_and(|previous| previous >= *id) {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    active = Some(*id);
                }
                JournalLifecycleEvent::NativeObserved { id } => {
                    if active.take() != Some(*id)
                        || observations
                            .get(seen)
                            .is_none_or(|o| o.invocation.id != *id)
                    {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    seen += 1;
                    last_id = Some(*id);
                }
                JournalLifecycleEvent::TransactionEnd {
                    observations: count,
                } => {
                    if active.is_some()
                        || *count != seen
                        || seen != observations.len()
                        || ordinal + 1 != events.len()
                    {
                        return Err(JournalError::InvalidLifecycleReceipt);
                    }
                    ended = true;
                }
            }
        }
        if !ended {
            return Err(JournalError::InvalidLifecycleReceipt);
        }
        Ok(Self { token, events })
    }
}

#[derive(Clone, Debug)]
enum Undo {
    Suicide {
        address: JournalAddress,
        previous: bool,
        balance: ExecutionBalance,
    },
    Creation {
        address: JournalAddress,
        previous: Option<JournalAccount>,
        lifecycle_creation: usize,
    },
    Touch {
        address: JournalAddress,
    },
    Nonce {
        address: JournalAddress,
        previous: FinalChainNonce,
    },
    Balance {
        address: JournalAddress,
        previous: ExecutionBalance,
    },
    Code {
        address: JournalAddress,
    },
    Storage {
        address: JournalAddress,
        key: ConcreteStorageKey,
        previous: StorageCell,
    },
}

#[derive(Clone, Copy, Debug)]
struct CheckpointState {
    id: JournalCheckpoint,
    undo_len: usize,
    logs_len: usize,
    native_invocations_len: usize,
    lifecycle_frame: usize,
    refund: u64,
}

/// Concrete execution journal bound to one immutable reader generation.
pub struct ExecutionJournal<R> {
    reader: R,
    accounts: BTreeMap<JournalAddress, JournalAccount>,
    ordinary: BTreeMap<(JournalAddress, ConcreteStorageKey), StorageCell>,
    raw: BTreeMap<(JournalAddress, ConcreteStorageKey), NativeRawOperation>,
    borrowed_memory: crate::borrowed_memory::BorrowedMemory<(JournalAddress, ConcreteStorageKey)>,
    transient: BTreeMap<(JournalAddress, ConcreteStorageKey), [u8; 32]>,
    logs: Vec<ExecutionLog>,
    native_invocations: Vec<ConsensusNativeObservation>,
    refund: u64,
    undo: Vec<Undo>,
    checkpoints: Vec<CheckpointState>,
    next_checkpoint: u64,
    lifecycle_events: Vec<JournalLifecycleEvent>,
    lifecycle_token: Option<JournalReceiptToken>,
}

impl<R: ConcreteExecutionRead> ExecutionJournal<R> {
    /// Creates an empty transaction journal over one immutable concrete reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            accounts: BTreeMap::new(),
            ordinary: BTreeMap::new(),
            raw: BTreeMap::new(),
            borrowed_memory: Default::default(),
            transient: BTreeMap::new(),
            logs: Vec::new(),
            native_invocations: Vec::new(),
            refund: 0,
            undo: Vec::new(),
            checkpoints: Vec::new(),
            next_checkpoint: 0,
            lifecycle_events: Vec::new(),
            lifecycle_token: None,
        }
    }

    /// Starts receipt capture and returns the expected identity before execution.
    pub fn begin_lifecycle_receipt(&mut self) -> Result<JournalReceiptToken, JournalError> {
        if self.lifecycle_token.is_some() || !self.lifecycle_events.is_empty() {
            return Err(JournalError::InvalidLifecycleReceipt);
        }
        let value = NEXT_RECEIPT_TOKEN
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |value| value.checked_add(1),
            )
            .map_err(|_| JournalError::InvalidLifecycleReceipt)?;
        let token = JournalReceiptToken(value);
        self.lifecycle_token = Some(token);
        Ok(token)
    }

    /// Borrows the immutable reader fixed for this journal.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Starts a nested frame checkpoint.
    pub fn checkpoint(&mut self) -> JournalCheckpoint {
        let id = JournalCheckpoint(self.next_checkpoint);
        let mut saved = (self.logs.len() as u64).to_be_bytes().to_vec();
        saved.extend_from_slice(&self.refund.to_be_bytes());
        duration_journal_event("snapshot", [0; 20], &id.0.to_be_bytes(), &saved);
        self.next_checkpoint = self.next_checkpoint.wrapping_add(1);
        let lifecycle_frame = self.lifecycle_events.len();
        self.lifecycle_events
            .push(JournalLifecycleEvent::FrameOpened {
                parent: self.checkpoints.last().map(|frame| frame.lifecycle_frame),
            });
        self.checkpoints.push(CheckpointState {
            id,
            undo_len: self.undo.len(),
            logs_len: self.logs.len(),
            native_invocations_len: self.native_invocations.len(),
            lifecycle_frame,
            refund: self.refund,
        });
        id
    }

    /// Commits the current frame while retaining undo records for its parent.
    pub fn commit_checkpoint(&mut self, checkpoint: JournalCheckpoint) -> Result<(), JournalError> {
        let state = self.take_top(checkpoint)?;
        let mut saved = (state.logs_len as u64).to_be_bytes().to_vec();
        saved.extend_from_slice(&state.refund.to_be_bytes());
        duration_journal_event(
            "checkpoint_commit",
            [0; 20],
            &checkpoint.0.to_be_bytes(),
            &saved,
        );
        if self.checkpoints.is_empty() {
            self.undo.clear();
        }
        Ok(())
    }

    /// Reverts ordinary account/storage/log/refund changes in the current frame.
    ///
    /// Native raw and transient writes normally survive. Reverting creation of
    /// an absent account also removes that account's raw overlay, matching the
    /// reference account-lifecycle undo callback.
    pub fn revert_checkpoint(&mut self, checkpoint: JournalCheckpoint) -> Result<(), JournalError> {
        let state = self.take_top(checkpoint)?;
        let mut saved = (state.logs_len as u64).to_be_bytes().to_vec();
        saved.extend_from_slice(&state.refund.to_be_bytes());
        duration_journal_event(
            "checkpoint_revert_begin",
            [0; 20],
            &checkpoint.0.to_be_bytes(),
            &saved,
        );
        while self.undo.len() > state.undo_len {
            match self.undo.pop().expect("undo length checked") {
                Undo::Creation {
                    address,
                    previous,
                    lifecycle_creation,
                } => {
                    #[cfg(feature = "synthetic-fixtures")]
                    {
                        let mut raw_key = [0_u8; 32];
                        raw_key[31] = 2;
                        let raw_key = ConcreteStorageKey(raw_key);
                        if cleanup_diagnostic_observer::target(address, raw_key) {
                            let before = self
                                .accounts
                                .get(&address)
                                .map(|a| (a.exists, a.suicided, a.mod_count, a.times_touched));
                            let saved = previous.as_ref().map(|a| a.exists);
                            cleanup_diagnostic_observer::origin(
                                "origin_undo_before",
                                &(
                                    checkpoint,
                                    self.checkpoints.len() + 1,
                                    saved,
                                    before,
                                    self.raw.get(&(address, raw_key)),
                                ),
                            );
                        }
                    }
                    duration_journal_event("restore_create", address, &[], &[]);
                    duration_journal_event("delete", address, &[], &[]);
                    match previous {
                        Some(account) => {
                            self.accounts.insert(address, account);
                        }
                        None => {
                            self.accounts.remove(&address);
                        }
                    }
                    self.raw.retain(|(owner, _), _| owner != &address);
                    self.lifecycle_events
                        .push(JournalLifecycleEvent::AccountReset {
                            address,
                            creation: lifecycle_creation,
                            rollback_frame: state.lifecycle_frame,
                        });
                    #[cfg(feature = "synthetic-fixtures")]
                    {
                        let mut raw_key = [0_u8; 32];
                        raw_key[31] = 2;
                        let raw_key = ConcreteStorageKey(raw_key);
                        if cleanup_diagnostic_observer::target(address, raw_key) {
                            let after = self
                                .accounts
                                .get(&address)
                                .map(|a| (a.exists, a.suicided, a.mod_count, a.times_touched));
                            cleanup_diagnostic_observer::origin(
                                "origin_undo_after",
                                &(
                                    checkpoint,
                                    self.checkpoints.len() + 1,
                                    after,
                                    self.raw.get(&(address, raw_key)),
                                ),
                            );
                        }
                    }
                    self.ordinary.retain(|(owner, _), _| owner != &address);
                }
                Undo::Touch { address } => {
                    duration_journal_event("restore_touch", address, &[], &[]);
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("touched account exists");
                    account.mod_count = account.mod_count.saturating_sub(1);
                    account.times_touched = account.times_touched.saturating_sub(1);
                }
                Undo::Nonce { address, previous } => {
                    duration_journal_event("restore_nonce", address, &[], &previous.to_bytes());
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("nonce account exists");
                    account.nonce = previous;
                    account.mod_count = account.mod_count.saturating_sub(1);
                }
                Undo::Balance { address, previous } => {
                    duration_journal_event(
                        "restore_balance",
                        address,
                        &[],
                        &duration_journal_signed(previous.value()),
                    );
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("balance account exists");
                    account.balance = previous;
                    account.mod_count = account.mod_count.saturating_sub(1);
                }
                Undo::Suicide {
                    address,
                    previous,
                    balance,
                } => {
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("suicide account exists");
                    duration_journal_event(
                        "restore_suicide",
                        address,
                        &[u8::from(previous)],
                        &duration_journal_signed(balance.value()),
                    );
                    account.suicided = previous;
                    account.balance = balance;
                    account.mod_count = account.mod_count.saturating_sub(1);
                }
                Undo::Code { address } => {
                    duration_journal_event("restore_code", address, &[], &[]);
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("code account exists");
                    account.code_hash = None;
                    account.code_size = 0;
                    account.code = None;
                    account.mod_count = account.mod_count.saturating_sub(1);
                }
                Undo::Storage {
                    address,
                    key,
                    previous,
                } => {
                    let key_bytes = key
                        .0
                        .iter()
                        .position(|byte| *byte != 0)
                        .map_or(&key.0[32..], |first| &key.0[first..]);
                    let previous_bytes = if previous.current == BigUint::default() {
                        Vec::new()
                    } else {
                        previous.current.to_bytes_be()
                    };
                    duration_journal_event("restore_storage", address, key_bytes, &previous_bytes);
                    // Go restores the value in StorageDirty, even when the
                    // first write was reverted. Emit it only if this account
                    // later flushes for some surviving mutation.
                    let mut previous = previous;
                    previous.dirty = true;
                    self.ordinary.insert((address, key), previous);
                    let account = self
                        .accounts
                        .get_mut(&address)
                        .expect("storage account exists");
                    account.mod_count = account.mod_count.saturating_sub(1);
                }
            }
        }
        for i in (state.logs_len..self.logs.len()).rev() {
            let log = &self.logs[i];
            duration_journal_event(
                "log_restore",
                log.address,
                &(i as u64).to_be_bytes(),
                &duration_journal_log(log),
            );
        }
        duration_journal_event(
            "refund_restore",
            [0; 20],
            &self.refund.to_be_bytes(),
            &state.refund.to_be_bytes(),
        );
        self.logs.truncate(state.logs_len);
        for invocation in &mut self.native_invocations[state.native_invocations_len..] {
            if invocation.disposition == ConsensusNativeDisposition::Normal {
                invocation.disposition = ConsensusNativeDisposition::OuterFrameReverted;
            }
        }
        self.refund = state.refund;
        let mut restored = (self.logs.len() as u64).to_be_bytes().to_vec();
        restored.extend_from_slice(&self.refund.to_be_bytes());
        duration_journal_event(
            "checkpoint_revert_end",
            [0; 20],
            &checkpoint.0.to_be_bytes(),
            &restored,
        );
        if self.checkpoints.is_empty() {
            self.undo.clear();
        }
        Ok(())
    }

    /// Retains one validated native fact before its caller settles the frame.
    /// Only the consensus adapter can append; stateless helpers have no access.
    pub(crate) fn begin_native_invocation(&mut self, id: crate::contracts::NativeInvocationId) {
        self.lifecycle_events
            .push(JournalLifecycleEvent::NativeStarted { id });
    }

    pub(crate) fn record_native_invocation(&mut self, observation: ConsensusNativeObservation) {
        self.lifecycle_events
            .push(JournalLifecycleEvent::NativeObserved {
                id: observation.invocation.id,
            });
        self.native_invocations.push(observation);
    }

    /// Reads current account existence, nonce and signed balance.
    pub fn account(&self, address: JournalAddress) -> Result<NativeJournalAccount, JournalError> {
        self.current_account(address).map_err(JournalError::from)
    }

    /// Reads current account metadata without narrowing nonce or balance.
    pub fn account_metadata(
        &self,
        address: JournalAddress,
    ) -> Result<JournalAccountMetadata, JournalError> {
        let account = self.current_account_state(address)?;
        Ok(JournalAccountMetadata {
            exists: account.exists,
            nonce: account.nonce,
            balance: account.balance,
            code_hash: account.code_hash,
            code_size: account.code_size,
        })
    }

    /// Loads and validates current code for one account.
    ///
    /// Absent accounts and existing zero-code accounts both return empty bytes;
    /// callers retain their distinct existence/hash facts from
    /// [`Self::account_metadata`]. Missing, corrupt, wrong-size or wrong-hash
    /// referenced code aborts execution as infrastructure failure.
    pub fn account_code(&self, address: JournalAddress) -> Result<Vec<u8>, JournalError> {
        let account = self.current_account_state(address)?;
        if !account.exists || account.code_size == 0 {
            duration_journal_event("read_code", address, &[], &[]);
            return Ok(Vec::new());
        }
        if let Some(code) = account.code {
            let result = validate_code(address, account.code_hash, account.code_size, code)?;
            duration_journal_event("read_code", address, &[], &result);
            return Ok(result);
        }
        let code_hash = account
            .code_hash
            .ok_or(JournalError::MissingCodeHash { address })?;
        let code = match self.reader.code(code_hash)? {
            ConcreteRead::Present(code) => code,
            ConcreteRead::Absent | ConcreteRead::Tombstone => {
                return Err(JournalError::ReferencedCodeMissing { address, code_hash });
            }
        };
        let result = validate_code(address, Some(code_hash), account.code_size, code)?;
        duration_journal_event("read_code", address, &[], &result);
        Ok(result)
    }

    /// Creates/touches an account through the ordinary rollback lane.
    pub fn touch_account(&mut self, address: JournalAddress) -> Result<(), JournalError> {
        self.ensure_account(address)?;
        let current = self.accounts.get(&address).expect("account was ensured");
        if !current.empty() {
            return Ok(());
        }
        duration_journal_event("touch", address, &[], &[]);
        self.undo.push(Undo::Touch { address });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was ensured");
        account.times_touched = account.times_touched.saturating_add(1);
        account.mod_count = account.mod_count.saturating_add(1);
        if address == ripemd_address() {
            account.mod_count = account.mod_count.saturating_add(1);
        }
        Ok(())
    }

    /// Replaces a nonce through the ordinary rollback lane.
    pub fn set_nonce(
        &mut self,
        address: JournalAddress,
        nonce: FinalChainNonce,
    ) -> Result<(), JournalError> {
        let helper_argument = duration_journal_is_armed().then(|| {
            duration_journal_signed(&BigInt::from_bytes_be(
                num_bigint::Sign::Plus,
                &nonce.to_bytes(),
            ))
        });
        if let Some(argument) = &helper_argument {
            duration_journal_event("helper_begin", address, &[3], argument);
            duration_journal_event("helper_operation", address, &[3], argument);
        }
        let result = (|| -> Result<(), JournalError> {
            self.ensure_account(address)?;
            if self
                .accounts
                .get(&address)
                .expect("account was ensured")
                .nonce
                > nonce
            {
                return Err(JournalError::NonceDecrease);
            }
            let previous = self
                .accounts
                .get(&address)
                .expect("account was ensured")
                .nonce
                .clone();
            duration_journal_event("nonce", address, &[], &nonce.to_bytes());
            self.undo.push(Undo::Nonce { address, previous });
            let account = self
                .accounts
                .get_mut(&address)
                .expect("account was touched");
            account.nonce = nonce;
            account.mod_count = account.mod_count.saturating_add(1);
            Ok(())
        })();
        if let Some(argument) = &helper_argument {
            let mut terminal = argument.clone();
            terminal.push(u8::from(result.is_ok()));
            duration_journal_event("helper_end", address, &[3], &terminal);
        }
        result
    }

    /// Replaces a signed balance through the ordinary rollback lane.
    pub fn set_balance(
        &mut self,
        address: JournalAddress,
        balance: ExecutionBalance,
    ) -> Result<(), JournalError> {
        self.ensure_account(address)?;
        let previous = self
            .accounts
            .get(&address)
            .expect("account was ensured")
            .balance
            .clone();
        duration_journal_event(
            "balance",
            address,
            &[],
            &duration_journal_signed(balance.value()),
        );
        self.undo.push(Undo::Balance { address, previous });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was touched");
        account.balance = balance;
        account.mod_count = account.mod_count.saturating_add(1);
        Ok(())
    }

    /// Subtracts a non-negative amount with the reference's zero-amount semantics.
    ///
    /// Zero ensures a missing account exists but does not dirty an existing
    /// account. A nonzero amount may produce a signed intermediate balance.
    pub fn subtract_balance(
        &mut self,
        address: JournalAddress,
        amount: &BigUint,
    ) -> Result<(), JournalError> {
        let helper_argument = duration_journal_is_armed()
            .then(|| duration_journal_signed(&BigInt::from(amount.clone())));
        if let Some(argument) = &helper_argument {
            duration_journal_event("helper_begin", address, &[2], argument);
            duration_journal_event("helper_operation", address, &[2], argument);
        }
        let result = (|| -> Result<(), JournalError> {
            self.ensure_account(address)?;
            if amount == &BigUint::default() {
                return Ok(());
            }
            let current = self.account(address)?.balance;
            self.set_balance(
                address,
                ExecutionBalance::new(current.value().clone() - BigInt::from(amount.clone())),
            )
        })();
        if let Some(argument) = &helper_argument {
            let mut terminal = argument.clone();
            terminal.push(u8::from(result.is_ok()));
            duration_journal_event("helper_end", address, &[2], &terminal);
        }
        result
    }

    /// Adds a non-negative amount with the reference's zero-amount touch behavior.
    pub fn add_balance(
        &mut self,
        address: JournalAddress,
        amount: &BigUint,
    ) -> Result<(), JournalError> {
        let helper_argument = duration_journal_is_armed()
            .then(|| duration_journal_signed(&BigInt::from(amount.clone())));
        if let Some(argument) = &helper_argument {
            duration_journal_event("helper_begin", address, &[1], argument);
            duration_journal_event("helper_operation", address, &[1], argument);
        }
        let result = (|| -> Result<(), JournalError> {
            if amount == &BigUint::default() {
                return self.touch_account(address);
            }
            self.ensure_account(address)?;
            let current = self.account(address)?.balance;
            self.set_balance(
                address,
                ExecutionBalance::new(current.value().clone() + BigInt::from(amount.clone())),
            )
        })();
        if let Some(argument) = &helper_argument {
            let mut terminal = argument.clone();
            terminal.push(u8::from(result.is_ok()));
            duration_journal_event("helper_end", address, &[1], &terminal);
        }
        result
    }

    /// Applies the historical account suicide after opcode gas admission.
    ///
    /// Transfers the full signed balance before zeroing the source and marks it
    /// for deletion at transaction settlement. Code, nonce and storage remain
    /// visible until settlement. All changes use ordinary frame rollback; raw
    /// and transient lanes retain their existing independent lifetimes. A missing
    /// source only touches the beneficiary. Self-beneficiary transfer still ends
    /// at zero. Read errors propagate before any mutation.
    pub fn selfdestruct(
        &mut self,
        address: JournalAddress,
        beneficiary: JournalAddress,
    ) -> Result<(), JournalError> {
        let source = self.load_account(address)?;
        let destination = self.load_account(beneficiary)?;
        if !source.exists || source.balance.value() == &BigInt::default() {
            self.touch_account(beneficiary)?;
        } else {
            self.set_balance(
                beneficiary,
                ExecutionBalance::new(destination.balance.value() + source.balance.value()),
            )?;
        }
        // Test existence after the beneficiary operation: a missing source that
        // is also its own beneficiary was created by the touch above. Go tests
        // IsNIL before that operation, so retain the original source fact.
        if !source.exists {
            return Ok(());
        }
        let account = self.accounts.get_mut(&address).expect("source was loaded");
        duration_journal_event("suicide", address, &beneficiary, &[]);
        self.undo.push(Undo::Suicide {
            address,
            previous: account.suicided,
            balance: account.balance.clone(),
        });
        account.suicided = true;
        account.balance = ExecutionBalance::default();
        account.mod_count = account.mod_count.saturating_add(1);
        Ok(())
    }

    /// Installs runtime code through the ordinary rollback lane.
    ///
    /// Computes the Keccak-256 identity from nonempty code, matching the reference.
    /// Empty input only ensures account existence and leaves existing code intact.
    /// Code bytes are emitted as an immutable sink row only on settlement.
    pub fn set_code(&mut self, address: JournalAddress, code: Vec<u8>) -> Result<(), JournalError> {
        self.ensure_account(address)?;
        if code.is_empty() {
            return Ok(());
        }
        duration_journal_event("code", address, &[], &code);
        self.undo.push(Undo::Code { address });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was touched");
        account.code_hash = Some(revm::primitives::keccak256(&code).0);
        account.code_size = code.len() as u64;
        account.code = Some(code);
        account.mod_count = account.mod_count.saturating_add(1);
        Ok(())
    }

    /// Validates and applies ordered native ordinary-account effects.
    ///
    /// Each effect observes all prior effects in the slice. A mismatch aborts
    /// the pending period; callers must not reuse the partially advanced
    /// journal. `EnsureExists` preserves zero-debit semantics without touching
    /// an existing empty account. `Touch` applies the separate zero-credit rule;
    /// either can establish existence before a later nonce/balance effect.
    pub fn apply_native_account_mutations(
        &mut self,
        mutations: &[NativeOrdinaryAccountMutation],
    ) -> Result<(), JournalError> {
        for mutation in mutations {
            match mutation {
                NativeOrdinaryAccountMutation::EnsureExists {
                    address,
                    expected_exists,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.ensure_account(*address)?;
                }
                NativeOrdinaryAccountMutation::Touch {
                    address,
                    expected_exists,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.touch_account(*address)?;
                }
                NativeOrdinaryAccountMutation::Balance {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.expect_native(
                        *address,
                        account.balance == *expected,
                        NativeAccountField::Balance,
                    )?;
                    self.set_balance(*address, replacement.clone())?;
                }
                NativeOrdinaryAccountMutation::Nonce {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.expect_native(
                        *address,
                        account.nonce == *expected,
                        NativeAccountField::Nonce,
                    )?;
                    self.set_nonce(*address, replacement.clone())?;
                }
            }
            duration_native_account_event("applied_account", mutation);
        }
        Ok(())
    }

    /// Reads original and current ordinary storage without consulting raw dirty bytes.
    pub fn ordinary_storage(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<(BigUint, BigUint), JournalError> {
        let result = if !self.current_account(address)?.exists {
            (BigUint::default(), BigUint::default())
        } else if let Some(cell) = self.ordinary.get(&(address, key)) {
            (cell.original.clone(), cell.current.clone())
        } else {
            let value = self.committed_ordinary_value(address, key)?;
            (value.clone(), value)
        };
        let key_bytes = key
            .0
            .iter()
            .position(|byte| *byte != 0)
            .map_or(&key.0[32..], |first| &key.0[first..]);
        let value = if result.1 == BigUint::default() {
            Vec::new()
        } else {
            result.1.to_bytes_be()
        };
        let original = if result.0 == BigUint::default() {
            Vec::new()
        } else {
            result.0.to_bytes_be()
        };
        duration_journal_event("read_storage_original", address, key_bytes, &original);
        duration_journal_event("read_storage_current", address, key_bytes, &value);
        Ok(result)
    }

    /// Replaces current ordinary storage while retaining its first-read original value.
    ///
    /// Undo retains the restored value as a dirty-map entry, matching Go. It is
    /// written only if a surviving mutation makes the account flush; reverting
    /// account creation removes the whole ordinary map for that account.
    pub fn set_ordinary_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        value: BigUint,
    ) -> Result<(), JournalError> {
        self.ensure_account(address)?;
        let map_key = (address, key);
        let previous = self.ordinary.get(&map_key).cloned();
        let mut cell = match previous.clone() {
            Some(cell) => cell,
            None => {
                let original = self.committed_ordinary_value(address, key)?;
                StorageCell {
                    original: original.clone(),
                    current: original,
                    dirty: false,
                }
            }
        };
        if cell.current == value {
            return Ok(());
        }
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was ensured");
        account.mod_count = account.mod_count.saturating_add(1);
        let key_bytes = key
            .0
            .iter()
            .position(|byte| *byte != 0)
            .map_or(&key.0[32..], |first| &key.0[first..]);
        let value_bytes = if value == BigUint::default() {
            Vec::new()
        } else {
            value.to_bytes_be()
        };
        duration_journal_event("storage", address, key_bytes, &value_bytes);
        self.undo.push(Undo::Storage {
            address,
            key,
            previous: cell.clone(),
        });
        cell.current = value;
        cell.dirty = true;
        self.ordinary.insert(map_key, cell);
        Ok(())
    }

    pub(crate) fn native_memory_frame(&self) -> crate::borrowed_memory::FrameMemory {
        self.borrowed_memory.frame()
    }

    pub(crate) fn synchronize_native_memory(
        &mut self,
        frame: &mut crate::borrowed_memory::FrameMemory,
        memory: &[u8],
    ) {
        self.borrowed_memory.synchronize(frame, memory);
        self.synchronize_borrowed_raw_values();
    }

    pub(crate) fn release_native_memory(&mut self, frame: crate::borrowed_memory::FrameMemory) {
        self.borrowed_memory.release(frame);
        self.synchronize_borrowed_raw_values();
    }

    fn synchronize_borrowed_raw_values(&mut self) {
        for (key, value) in self.borrowed_memory.values() {
            if let Some(NativeRawOperation::Put(current)) = self.raw.get_mut(key) {
                *current = crate::contracts::NativeRawValue::new(value.to_vec())
                    .expect("registered VRF input is nonempty");
            }
        }
    }

    pub(crate) fn bind_native_input(
        &mut self,
        alias: &crate::contracts::NativeInputAlias,
        frame: &crate::borrowed_memory::FrameMemory,
        range: std::ops::Range<usize>,
        memory: &[u8],
    ) -> bool {
        let key = (alias.address, alias.key);
        let Some(NativeRawOperation::Put(value)) = self.raw.get(&key) else {
            return false;
        };
        if memory.get(range.clone()) != Some(value.as_bytes()) {
            return false;
        }
        self.borrowed_memory.bind(key, frame, range, memory)
    }

    /// Reads exact native raw bytes, independently from ordinary dirty storage.
    pub fn raw_storage(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, JournalError> {
        let result = self.duration_raw_storage_inner(address, key);
        if let Ok(read) = &result {
            match read {
                ConcreteRead::Absent => {
                    duration_journal_event("read_raw_absent", address, &key.0, &[])
                }
                ConcreteRead::Tombstone => {
                    duration_journal_event("read_raw_tombstone", address, &key.0, &[])
                }
                ConcreteRead::Present(value) => {
                    duration_journal_event("read_raw_present", address, &key.0, value)
                }
            }
        }
        result
    }

    fn duration_raw_storage_inner(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, JournalError> {
        let account = self.current_account(address);
        #[cfg(feature = "synthetic-fixtures")]
        if cleanup_diagnostic_observer::target(address, key)
            && let Err(error) = &account
        {
            cleanup_diagnostic_observer::error("account_error", error);
        }
        if !account?.exists {
            #[cfg(feature = "synthetic-fixtures")]
            if cleanup_diagnostic_observer::target(address, key) {
                if cleanup_diagnostic_observer::active() {
                    cleanup_diagnostic_observer::absent_account_raw(
                        self.accounts.contains_key(&address),
                        self.raw.get(&(address, key)),
                    );
                }
                cleanup_diagnostic_observer::value("account_absent", &ConcreteRead::Absent);
            }
            return Ok(ConcreteRead::Absent);
        }
        if let Some(operation) = self.raw.get(&(address, key)) {
            let value = match operation {
                NativeRawOperation::Put(value) => ConcreteRead::Present(value.as_bytes().to_vec()),
                NativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            };
            #[cfg(feature = "synthetic-fixtures")]
            if cleanup_diagnostic_observer::target(address, key) {
                let branch = match operation {
                    NativeRawOperation::Put(_) => "staged_put",
                    NativeRawOperation::Delete => "staged_delete",
                };
                cleanup_diagnostic_observer::value(branch, &value);
            }
            return Ok(value);
        }
        let result = self.reader.storage(address, key);
        #[cfg(feature = "synthetic-fixtures")]
        if cleanup_diagnostic_observer::target(address, key) {
            match &result {
                Ok(value) => cleanup_diagnostic_observer::value("backing", value),
                Err(error) => cleanup_diagnostic_observer::error("backing_error", error),
            }
        }
        result.map_err(JournalError::from)
    }

    /// Applies an irreversible native raw operation for an existing account.
    pub fn set_raw_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        operation: NativeRawOperation,
    ) -> Result<(), JournalError> {
        self.ensure_account(address)?;
        #[cfg(feature = "synthetic-fixtures")]
        if cleanup_diagnostic_observer::target(address, key) {
            cleanup_diagnostic_observer::origin(
                "origin_set",
                &(
                    self.checkpoints.last().map(|c| c.id),
                    self.checkpoints.len(),
                    &operation,
                ),
            );
        }
        match &operation {
            NativeRawOperation::Put(value) => {
                duration_journal_event("raw_write", address, &key.0, value.as_bytes())
            }
            NativeRawOperation::Delete => duration_journal_event("raw_write", address, &key.0, &[]),
        }
        self.borrowed_memory.remove(&(address, key));
        self.raw.insert((address, key), operation);
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was ensured");
        account.mod_count = account.mod_count.saturating_add(1);
        Ok(())
    }

    /// Reads transaction-local transient storage; missing values are zero.
    pub fn transient_storage(&self, address: JournalAddress, key: ConcreteStorageKey) -> [u8; 32] {
        self.transient
            .get(&(address, key))
            .copied()
            .unwrap_or([0_u8; 32])
    }

    /// Writes transaction-local transient storage, which survives frame revert.
    pub fn set_transient_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        value: [u8; 32],
    ) {
        if value == [0_u8; 32] {
            self.transient.remove(&(address, key));
        } else {
            self.transient.insert((address, key), value);
        }
    }

    /// Appends a log through the ordinary frame rollback lane.
    pub fn push_log(&mut self, log: ExecutionLog) {
        duration_journal_event(
            "log_add",
            log.address,
            &(self.logs.len() as u64).to_be_bytes(),
            &duration_journal_log(&log),
        );
        self.logs.push(log);
    }

    /// Borrows logs currently visible to the transaction.
    pub fn logs(&self) -> &[ExecutionLog] {
        &self.logs
    }

    /// Returns the current refund counter.
    pub fn refund(&self) -> u64 {
        self.refund
    }

    /// Adds to the refund counter with checked arithmetic.
    pub fn add_refund(&mut self, refund: u64) -> Result<(), JournalError> {
        let before = self.refund;
        let Some(after) = before.checked_add(refund) else {
            duration_journal_event(
                "refund_add_failed",
                [0; 20],
                &before.to_be_bytes(),
                &refund.to_be_bytes(),
            );
            return Err(JournalError::RefundOverflow);
        };
        duration_journal_event(
            "refund_add",
            [0; 20],
            &before.to_be_bytes(),
            &refund.to_be_bytes(),
        );
        self.refund = after;
        Ok(())
    }

    /// Applies one successful root interpreter's signed refund change.
    ///
    /// A disposable trace sequence retains its cumulative counter between root
    /// executions, so restoring earlier storage can legitimately debit it.
    /// Underflow/overflow leaves the counter unchanged. Frame checkpoints still
    /// own rollback, and ordinary transaction settlement still resets it.
    pub(crate) fn apply_refund_delta(&mut self, delta: i64) -> Result<(), JournalError> {
        let before = self.refund;
        let amount = delta.unsigned_abs();
        let Some(after) = before.checked_add_signed(delta) else {
            duration_journal_event(
                if delta < 0 {
                    "refund_sub_failed"
                } else {
                    "refund_add_failed"
                },
                [0; 20],
                &before.to_be_bytes(),
                &amount.to_be_bytes(),
            );
            return Err(if delta < 0 {
                JournalError::RefundUnderflow
            } else {
                JournalError::RefundOverflow
            });
        };
        duration_journal_event(
            if delta < 0 {
                "refund_sub"
            } else {
                "refund_add"
            },
            [0; 20],
            &before.to_be_bytes(),
            &amount.to_be_bytes(),
        );
        self.refund = after;
        Ok(())
    }

    /// Settles one transaction and resets transaction-local facts.
    ///
    /// Modified suicided or EIP-161-empty accounts are deleted and contribute no
    /// storage writes. Storage rows, including restored ordinary values, are
    /// emitted only for accounts that flush an update. Ordinary values precede
    /// raw values. Negative
    /// balances fail explicitly at this unsupported persistence boundary.
    pub fn settle_transaction(&mut self) -> Result<SettledTransaction, JournalError> {
        if !self.checkpoints.is_empty() {
            return Err(JournalError::OpenCheckpoints);
        }

        let mut writes = JournalWritePlan::default();
        let mut updated = BTreeSet::new();
        duration_journal_event(
            "settlement_begin",
            [0; 20],
            &(self.accounts.len() as u64).to_be_bytes(),
            &[],
        );
        for (address, account) in &self.accounts {
            if !account.exists {
                continue;
            }
            duration_journal_event(
                "settle_before",
                *address,
                &[],
                &duration_journal_account_facts(account),
            );
            if account.mod_count != 0 && (account.suicided || account.empty()) {
                duration_journal_event("settle_delete", *address, &[], &[]);
                writes.accounts.push(JournalAccountWrite {
                    address: *address,
                    operation: JournalAccountOperation::Delete,
                });
            } else if account.mod_count != 0 && account.mod_count != account.times_touched {
                updated.insert(*address);
                writes.accounts.push(JournalAccountWrite {
                    address: *address,
                    operation: JournalAccountOperation::Upsert {
                        nonce: account.nonce.clone(),
                        balance: account
                            .balance
                            .try_to_persisted()
                            .map_err(JournalError::Balance)?,
                        code_hash: account.code_hash,
                        code_size: account.code_size,
                    },
                });
                if let (Some(code_hash), Some(code)) = (account.code_hash, account.code.as_ref()) {
                    writes.code.push(JournalCodeWrite {
                        code_hash,
                        code: code.clone(),
                    });
                }
            }
        }

        duration_journal_event("settlement_end", [0; 20], &[], &[]);

        for ((address, key), cell) in &self.ordinary {
            if cell.dirty && updated.contains(address) {
                writes.ordinary_storage.push(JournalStorageWrite {
                    address: *address,
                    key: *key,
                    value: cell.current.clone(),
                });
            }
        }
        for ((address, key), operation) in &self.raw {
            if updated.contains(address) {
                writes.raw_storage.push(JournalRawWrite {
                    address: *address,
                    key: *key,
                    operation: operation.clone(),
                });
            }
        }

        #[cfg(feature = "synthetic-fixtures")]
        {
            let mut address = [0_u8; 20];
            address[19] = 0xee;
            let mut key = [0_u8; 32];
            key[31] = 2;
            let key = ConcreteStorageKey(key);
            let account = self.accounts.get(&address);
            let facts = account.map(|a| {
                (
                    a.exists,
                    a.suicided,
                    a.empty(),
                    a.mod_count,
                    a.times_touched,
                )
            });
            cleanup_diagnostic_observer::origin(
                "origin_settlement",
                &(
                    facts,
                    updated.contains(&address),
                    self.raw.get(&(address, key)),
                ),
            );
        }

        self.lifecycle_events
            .push(JournalLifecycleEvent::TransactionEnd {
                observations: self.native_invocations.len(),
            });
        let events = std::mem::take(&mut self.lifecycle_events);
        let lifecycle_receipt = self
            .lifecycle_token
            .take()
            .map(|token| JournalLifecycleReceipt::seal(token, events, &self.native_invocations))
            .transpose()?;
        let settled = SettledTransaction {
            logs: std::mem::take(&mut self.logs),
            refund: std::mem::take(&mut self.refund),
            writes,
            native_invocations: std::mem::take(&mut self.native_invocations),
            lifecycle_receipt,
        };
        self.accounts.clear();
        self.ordinary.clear();
        self.raw.clear();
        self.transient.clear();
        self.undo.clear();
        Ok(settled)
    }

    fn take_top(&mut self, checkpoint: JournalCheckpoint) -> Result<CheckpointState, JournalError> {
        if self
            .checkpoints
            .last()
            .is_none_or(|state| state.id != checkpoint)
        {
            return Err(JournalError::CheckpointOrder);
        }
        Ok(self.checkpoints.pop().expect("top checkpoint checked"))
    }

    fn expect_native(
        &self,
        address: JournalAddress,
        matches: bool,
        field: NativeAccountField,
    ) -> Result<(), JournalError> {
        if matches {
            Ok(())
        } else {
            Err(JournalError::NativeExpectation { address, field })
        }
    }

    fn load_account(&mut self, address: JournalAddress) -> Result<JournalAccount, JournalError> {
        if let Some(account) = self.accounts.get(&address) {
            duration_journal_event(
                "read_account",
                address,
                &[],
                &duration_journal_account_facts(account),
            );
            return Ok(account.clone());
        }
        let account = read_account(self.reader.account(address)?);
        duration_journal_event(
            "read_account",
            address,
            &[],
            &duration_journal_account_facts(&account),
        );
        self.accounts.insert(address, account.clone());
        Ok(account)
    }

    fn ensure_account(&mut self, address: JournalAddress) -> Result<(), JournalError> {
        let current = self.load_account(address)?;
        if current.exists {
            return Ok(());
        }
        let previous = self.accounts.get(&address).cloned();
        #[cfg(feature = "synthetic-fixtures")]
        {
            let mut raw_key = [0_u8; 32];
            raw_key[31] = 2;
            let raw_key = ConcreteStorageKey(raw_key);
            if cleanup_diagnostic_observer::target(address, raw_key) {
                cleanup_diagnostic_observer::origin(
                    "origin_creation",
                    &(
                        self.checkpoints.last().map(|c| c.id),
                        self.checkpoints.len(),
                        self.undo.len(),
                        previous.as_ref().map(|a| a.exists),
                        self.raw.get(&(address, raw_key)),
                    ),
                );
            }
        }
        let lifecycle_creation = self.lifecycle_events.len();
        self.lifecycle_events
            .push(JournalLifecycleEvent::AccountCreated {
                address,
                frame: self.checkpoints.last().map(|frame| frame.lifecycle_frame),
                saved_absent: !current.exists,
            });
        duration_journal_event("create", address, &[], &[]);
        self.undo.push(Undo::Creation {
            address,
            previous,
            lifecycle_creation,
        });
        self.accounts.insert(
            address,
            JournalAccount {
                exists: true,
                suicided: false,
                mod_count: 1,
                times_touched: 0,
                nonce: FinalChainNonce::zero(),
                balance: ExecutionBalance::default(),
                storage_root: None,
                code_hash: None,
                code_size: 0,
                code: None,
            },
        );
        Ok(())
    }

    fn committed_ordinary_value(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<BigUint, JournalError> {
        let storage_root = match self.accounts.get(&address) {
            Some(account) => account.storage_root,
            None => match self.reader.account(address)? {
                ConcreteRead::Present(record) => record.account.storage_root,
                ConcreteRead::Absent | ConcreteRead::Tombstone => None,
            },
        };
        if storage_root.is_none() {
            return Ok(BigUint::default());
        }
        Ok(read_storage_integer(self.reader.storage(address, key)?))
    }

    fn current_account(
        &self,
        address: JournalAddress,
    ) -> Result<NativeJournalAccount, ConcreteReadError> {
        let account = self.current_account_state(address)?;
        Ok(NativeJournalAccount {
            exists: account.exists,
            nonce: account.nonce,
            balance: account.balance,
        })
    }

    fn current_account_state(
        &self,
        address: JournalAddress,
    ) -> Result<JournalAccount, ConcreteReadError> {
        let account = match self.accounts.get(&address) {
            Some(account) => account.clone(),
            None => read_account(self.reader.account(address)?),
        };
        duration_journal_event(
            "read_account",
            address,
            &[],
            &duration_journal_account_facts(&account),
        );
        Ok(account)
    }
}

impl<R: ConcreteExecutionRead> NativeJournalRead for ExecutionJournal<R> {
    fn borrowed_raw_storage(&self, address: [u8; 20], key: &ConcreteStorageKey) -> Option<Vec<u8>> {
        self.borrowed_memory
            .value(&(address, *key))
            .map(<[u8]>::to_vec)
    }

    fn account(
        &self,
        address: JournalAddress,
    ) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.current_account(address)
            .map_err(NativeJournalReadError::State)
    }

    fn raw_storage(
        &self,
        address: JournalAddress,
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        ExecutionJournal::raw_storage(self, address, *key).map_err(|error| match error {
            JournalError::State(error) => NativeJournalReadError::State(error),
            other => NativeJournalReadError::Invariant(other.to_string()),
        })
    }
}

fn read_account(
    read: ConcreteRead<rustaxa_types::concrete_state::ConcreteAccountRecord>,
) -> JournalAccount {
    match read {
        ConcreteRead::Present(record) => JournalAccount {
            exists: true,
            suicided: false,
            mod_count: 0,
            times_touched: 0,
            nonce: record.account.nonce,
            balance: ExecutionBalance::from_persisted(&record.account.balance),
            storage_root: record.account.storage_root,
            code_hash: record.account.code_hash,
            code_size: record.account.code_size,
            code: None,
        },
        ConcreteRead::Absent | ConcreteRead::Tombstone => JournalAccount {
            exists: false,
            suicided: false,
            mod_count: 0,
            times_touched: 0,
            nonce: FinalChainNonce::zero(),
            balance: ExecutionBalance::default(),
            storage_root: None,
            code_hash: None,
            code_size: 0,
            code: None,
        },
    }
}

fn read_storage_integer(read: ConcreteRead<Vec<u8>>) -> BigUint {
    match read {
        ConcreteRead::Present(bytes) => BigUint::from_bytes_be(&bytes),
        ConcreteRead::Absent | ConcreteRead::Tombstone => BigUint::default(),
    }
}

fn ripemd_address() -> JournalAddress {
    let mut address = [0_u8; 20];
    address[19] = 3;
    address
}

fn validate_code(
    address: JournalAddress,
    code_hash: Option<[u8; 32]>,
    declared_size: u64,
    code: Vec<u8>,
) -> Result<Vec<u8>, JournalError> {
    if code.len() as u64 != declared_size {
        return Err(JournalError::CodeSizeMismatch {
            address,
            declared: declared_size,
            actual: code.len(),
        });
    }
    let declared = code_hash.ok_or(JournalError::MissingCodeHash { address })?;
    let actual: [u8; 32] = revm::primitives::keccak256(&code).into();
    if actual != declared {
        return Err(JournalError::CodeHashMismatch {
            address,
            declared,
            actual,
        });
    }
    Ok(code)
}
