//! Typed, append-only facts for execution trace serialization.
//!
//! The execution driver supplies facts from the same interpreter invocation that
//! executes the transaction. This module neither executes bytecode nor reads the
//! journal. In particular, [`TraceCollector`] preserves attempted `SSTORE` values
//! independently of journal checkpoints because the Go structured logger's map
//! is an observer history rather than committed storage.

use std::collections::BTreeMap;

use revm::interpreter::InstructionResult;

/// One EVM word in big-endian byte order.
pub type TraceWord = [u8; 32];

/// One EVM address.
pub type TraceAddress = [u8; 20];

/// The point at which an opcode row was captured.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceOpcodePhase {
    /// Gas admission and memory expansion succeeded; execution has not begun.
    BeforeExecution,
    /// The interpreter returned an error and the row describes its later state.
    Fault(InstructionResult),
}

/// An `SSTORE` key/value pair attempted by one captured opcode row.
///
/// The driver supplies this only when the captured opcode is `SSTORE` and that
/// row's stack has both operands. Supplying it does not state that the journal
/// accepted, committed, or retained the write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AttemptedSstore {
    /// Storage key taken from the top stack word.
    pub key: TraceWord,
    /// Storage value taken from the next stack word.
    pub value: TraceWord,
}

/// Complete source facts for one Go `CaptureState`-equivalent opcode row.
///
/// `gas_cost` and `refund` are supplied facts. The collector never derives them
/// from gas deltas: failed admission and early validation paths can retain costs
/// that are not represented by the interpreter's charged gas. For a
/// [`TraceOpcodePhase::BeforeExecution`] row, stack and memory describe the
/// pre-operation state after admitted memory expansion. A fault row describes
/// the later stack and memory visible when the error callback runs. Stack words
/// are ordered from bottom to top in both phases.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceOpcode {
    /// Program counter of the attempted instruction.
    pub pc: u64,
    /// Raw opcode byte.
    pub opcode: u8,
    /// Gas remaining before the instruction's gas calculation.
    pub gas: u64,
    /// Reference-visible cost for this capture row.
    pub gas_cost: u64,
    /// One-based depth emitted by the Go `CaptureState` callback.
    pub depth: u16,
    /// Account whose storage context the instruction uses.
    pub state_address: TraceAddress,
    /// Phase-dependent stack snapshot ordered from bottom to top.
    pub stack: Vec<TraceWord>,
    /// Phase-dependent memory snapshot at callback time.
    pub memory: Vec<u8>,
    /// Refund counter visible when this row was captured.
    pub refund: u64,
    /// Capture timing and exact fault result, if any.
    pub phase: TraceOpcodePhase,
    /// Attempted structured-logger storage update for this row.
    pub attempted_sstore: Option<AttemptedSstore>,
}

/// Kind of frame reported by the execution driver.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TraceFrameKind {
    /// Ordinary `CALL`, including a top-level call.
    Call,
    /// `CALLCODE`.
    CallCode,
    /// `DELEGATECALL`.
    DelegateCall,
    /// `STATICCALL`.
    StaticCall,
    /// `CREATE`, including a top-level creation.
    Create,
    /// `CREATE2`.
    Create2,
}

/// Facts available when one interpreter frame begins.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceFrameEnter {
    /// One-based execution-frame depth.
    pub depth: u16,
    /// Call or creation scheme.
    pub kind: TraceFrameKind,
    /// Calling account.
    pub caller: TraceAddress,
    /// Account whose balance, storage and execution context the frame owns.
    pub state_address: TraceAddress,
    /// Account from which bytecode was loaded; absent for initcode.
    pub code_address: Option<TraceAddress>,
    /// Supplied frame gas before executing its first opcode.
    pub gas: u64,
    /// Full-width unsigned call value in minimal big-endian form.
    pub value: Vec<u8>,
    /// Frame input or initcode input.
    pub input: Vec<u8>,
    /// Exact bytecode executed by the frame.
    pub code: Vec<u8>,
    /// Whether this frame is a native or stateless precompile invocation.
    pub precompile: bool,
}

/// Facts available when one interpreter frame ends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceFrameExit {
    /// One-based execution-frame depth.
    pub depth: u16,
    /// Gas supplied at frame entry.
    pub supplied_gas: u64,
    /// Gas remaining when the frame returned.
    pub remaining_gas: u64,
    /// Exact interpreter terminal category.
    pub result: InstructionResult,
    /// Returned or reverted bytes.
    pub output: Vec<u8>,
}

/// One ordered trace callback from the execution driver.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TraceEvent {
    /// A frame began.
    FrameEnter(TraceFrameEnter),
    /// An opcode capture row was emitted.
    Opcode(TraceOpcode),
    /// A frame ended.
    FrameExit(TraceFrameExit),
}

/// Optional consumer for trace facts produced by the execution driver.
///
/// Implementations must not mutate execution state. Callbacks are infallible so
/// an observer cannot change consensus execution; trace preparation that cannot
/// produce exact facts must fail in the driver before emitting a fabricated row.
pub trait ExecutionTraceObserver {
    /// Receives one event in interpreter order.
    fn observe(&mut self, event: TraceEvent);
}

/// Attempted structured storage values grouped by execution state address.
pub type AttemptedStorage = BTreeMap<TraceAddress, BTreeMap<TraceWord, TraceWord>>;

/// In-memory ordered event collector with Go structured-logger storage history.
///
/// Every supplied attempted `SSTORE` immediately updates the address-local map.
/// Frame failure and journal rollback do not rewind it. The collector performs
/// no state reads, gas calculation, opcode interpretation or serialization.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceCollector {
    events: Vec<TraceEvent>,
    attempted_storage: AttemptedStorage,
}

impl TraceCollector {
    /// Returns captured events in callback order.
    #[must_use]
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }

    /// Returns the latest attempted values grouped by state address.
    #[must_use]
    pub const fn attempted_storage(&self) -> &AttemptedStorage {
        &self.attempted_storage
    }

    /// Consumes the collector and returns ordered events plus attempted storage.
    #[must_use]
    pub fn into_parts(self) -> (Vec<TraceEvent>, AttemptedStorage) {
        (self.events, self.attempted_storage)
    }
}

impl ExecutionTraceObserver for TraceCollector {
    fn observe(&mut self, event: TraceEvent) {
        if let TraceEvent::Opcode(opcode) = &event
            && let Some(write) = opcode.attempted_sstore
        {
            self.attempted_storage
                .entry(opcode.state_address)
                .or_default()
                .insert(write.key, write.value);
        }
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opcode(
        address: TraceAddress,
        phase: TraceOpcodePhase,
        write: Option<AttemptedSstore>,
    ) -> TraceEvent {
        let stack = write
            .map(|write| vec![write.value, write.key])
            .unwrap_or_default();
        TraceEvent::Opcode(TraceOpcode {
            pc: 7,
            opcode: 0x55,
            gas: 19_800,
            gas_cost: 5_000,
            depth: 1,
            state_address: address,
            stack,
            memory: vec![0; 32],
            refund: 4_800,
            phase,
            attempted_sstore: write,
        })
    }

    #[test]
    fn collector_preserves_event_order_and_latest_attempted_values() {
        let address = [0x11; 20];
        let key = [0x22; 32];
        let first = AttemptedSstore {
            key,
            value: [0x33; 32],
        };
        let second = AttemptedSstore {
            key,
            value: [0x44; 32],
        };
        let mut collector = TraceCollector::default();

        collector.observe(opcode(
            address,
            TraceOpcodePhase::BeforeExecution,
            Some(first),
        ));
        collector.observe(opcode(
            address,
            TraceOpcodePhase::Fault(InstructionResult::OutOfGas),
            Some(second),
        ));

        assert_eq!(collector.events().len(), 2);
        assert_eq!(collector.attempted_storage()[&address][&key], second.value);
    }

    #[test]
    fn failed_frame_does_not_rewind_attempted_storage() {
        let address = [0x51; 20];
        let write = AttemptedSstore {
            key: [0x52; 32],
            value: [0x53; 32],
        };
        let mut collector = TraceCollector::default();
        collector.observe(opcode(
            address,
            TraceOpcodePhase::BeforeExecution,
            Some(write),
        ));
        collector.observe(TraceEvent::FrameExit(TraceFrameExit {
            depth: 1,
            supplied_gas: 10_000,
            remaining_gas: 0,
            result: InstructionResult::Revert,
            output: vec![],
        }));

        let (events, attempted) = collector.into_parts();
        assert_eq!(events.len(), 2);
        assert_eq!(attempted[&address][&write.key], write.value);
    }

    #[test]
    fn non_storage_rows_do_not_create_an_address_map() {
        let mut collector = TraceCollector::default();
        collector.observe(opcode([0x61; 20], TraceOpcodePhase::BeforeExecution, None));

        assert!(collector.attempted_storage().is_empty());
    }
}
