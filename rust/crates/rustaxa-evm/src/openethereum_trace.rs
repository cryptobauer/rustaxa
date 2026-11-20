//! OpenEthereum JSON from the ordered callbacks used by the historical replay.

use num_bigint::BigUint;
use revm::interpreter::InstructionResult;
use serde_json::{Value, json};

use crate::{
    contracts::CodeExecutionError,
    structured_trace::opcode_name,
    trace::{
        ExecutionTraceObserver, TraceEvent, TraceFrameEnter, TraceFrameExit, TraceFrameKind,
        TraceOpcode, TraceOpcodePhase, TraceWord,
    },
};

pub(crate) struct OpenEthereumTrace {
    with_vm: bool,
    calls: Vec<Value>,
    frames: Vec<Frame>,
    vm: Vec<VmFrame>,
    last_op: Option<Location>,
    returned: Option<(Location, Option<MemoryRange>)>,
    memory: Option<MemoryRange>,
}

struct Frame {
    call: Option<usize>,
    address: Vec<usize>,
    subtraces: usize,
    vm: usize,
    caller_op: Option<Location>,
    fault_stack: Option<(InstructionResult, u8, usize)>,
    jump_target: Option<TraceWord>,
}

#[derive(Default)]
struct VmFrame {
    code: Vec<u8>,
    index: String,
    ops: Vec<VmOp>,
}

struct VmOp {
    opcode: u8,
    cost: i64,
    pc: u64,
    ex: Option<VmEffect>,
    sub: Option<usize>,
    call_memory: Option<MemoryRange>,
}

struct VmEffect {
    used: i64,
    push: Vec<String>,
    mem: Value,
    store: Value,
}

type Location = (usize, usize);
type MemoryRange = (usize, usize);

impl OpenEthereumTrace {
    pub(crate) fn new(with_vm: bool) -> Self {
        Self {
            with_vm,
            calls: Vec::new(),
            frames: Vec::new(),
            vm: vec![VmFrame::default()],
            last_op: None,
            returned: None,
            memory: None,
        }
    }

    pub(crate) fn finish(self, output: &[u8]) -> Value {
        let vm_trace = if self.with_vm {
            self.serialize_vm(0)
        } else {
            Value::Null
        };
        let mut result = json!({"output": bytes(output), "stateDiff": null});
        result["vmTrace"] = vm_trace;
        result["trace"] = Value::Array(self.calls);
        result
    }

    fn enter(&mut self, frame: TraceFrameEnter) {
        let create = matches!(frame.kind, TraceFrameKind::Create | TraceFrameKind::Create2);
        let caller_op = self.frames.last().and(self.last_op);
        let vm = if self.with_vm
            && let Some((parent, op)) = caller_op
        {
            let vm = self.vm.len();
            let index = format!("{}{op}-", self.vm[parent].index);
            self.vm.push(VmFrame {
                index,
                ..VmFrame::default()
            });
            let caller = &mut self.vm[parent].ops[op];
            caller.sub = Some(vm);
            if create {
                caller.cost = caller.cost.wrapping_add(frame.gas as i64);
            }
            vm
        } else {
            0
        };
        self.vm[vm].code = frame.code;

        let mut path = self
            .frames
            .last()
            .map_or_else(Vec::new, |parent| parent.address.clone());
        let hidden = frame.depth > 1
            && frame.precompile
            && (frame.kind == TraceFrameKind::DelegateCall
                || frame.value.iter().all(|&byte| byte == 0));
        let call = if hidden {
            None
        } else {
            let value = if frame.kind == TraceFrameKind::DelegateCall {
                self.frames
                    .last()
                    .and_then(|parent| parent.call)
                    .map(|index| self.calls[index]["action"]["value"].clone())
                    .unwrap_or_else(|| json!("0x0"))
            } else {
                json!(format!("0x{:x}", BigUint::from_bytes_be(&frame.value)))
            };
            if let Some(parent) = self.frames.last_mut() {
                path.push(parent.subtraces);
                parent.subtraces += 1;
            }
            let gas = quantity(if frame.gas > 500_000_000 {
                frame
                    .gas
                    .wrapping_sub(0x8000_0000_0000_0000)
                    .wrapping_add(500_000_001)
            } else {
                frame.gas
            });
            let (action, result, kind) = if create {
                (
                    json!({"from": bytes(&frame.caller), "gas": gas, "init": bytes(&frame.input), "value": value}),
                    json!({"address": bytes(&frame.state_address)}),
                    "create",
                )
            } else {
                let call_type = match frame.kind {
                    TraceFrameKind::Call => "call",
                    TraceFrameKind::CallCode => "callcode",
                    TraceFrameKind::DelegateCall => "delegatecall",
                    TraceFrameKind::StaticCall => "staticcall",
                    _ => unreachable!("creation has a separate action"),
                };
                let to = frame.code_address.unwrap_or(frame.state_address);
                (
                    json!({"from": bytes(&frame.caller), "to": bytes(&to), "gas": gas,
                        "input": bytes(&frame.input), "value": value, "callType": call_type}),
                    json!({}),
                    "call",
                )
            };
            let index = self.calls.len();
            self.calls.push(json!({"action": action, "result": result, "traceAddress": path, "subtraces": 0, "type": kind}));
            Some(index)
        };
        self.frames.push(Frame {
            call,
            address: path,
            subtraces: 0,
            vm,
            caller_op,
            fault_stack: None,
            jump_target: None,
        });
    }

    fn exit(&mut self, exit: TraceFrameExit) {
        let frame = self.frames.pop().expect("frame exit follows entry");
        if self.with_vm
            && let Some(location) = frame.caller_op
        {
            let caller = &self.vm[location.0].ops[location.1];
            self.returned = Some((location, caller.call_memory));
            self.memory = caller.call_memory;
        }
        let Some(index) = frame.call else { return };
        let row = &mut self.calls[index];
        row["subtraces"] = json!(frame.subtraces);
        if let Some(error) = exit.error.as_ref() {
            row["error"] = json!(error_text(error, &frame, &self.vm[frame.vm].code));
            if *error != CodeExecutionError::Revert {
                row["result"] = Value::Null;
                return;
            }
        }
        let key = if row["type"] == "create" {
            "code"
        } else {
            "output"
        };
        row["result"][key] = json!(bytes(&exit.output));
        row["result"]["gasUsed"] =
            json!(quantity(exit.supplied_gas.wrapping_sub(exit.remaining_gas)));
    }

    fn opcode(&mut self, row: TraceOpcode) {
        let frame = self
            .frames
            .last_mut()
            .expect("opcode belongs to an active frame");
        match row.phase {
            TraceOpcodePhase::Fault(result) => {
                frame.fault_stack = Some((result, row.opcode, row.stack.len()))
            }
            TraceOpcodePhase::BeforeExecution if matches!(row.opcode, 0x56 | 0x57) => {
                frame.jump_target = row.stack.last().copied();
            }
            TraceOpcodePhase::BeforeExecution => {}
        }
        if self.with_vm {
            if let Some((vm, previous)) = self.last_op {
                let op = &mut self.vm[vm].ops[previous];
                if let Some(effect) = op.ex.as_mut() {
                    let count = stack_outputs(op.opcode).min(row.stack.len());
                    effect
                        .push
                        .extend(row.stack[row.stack.len() - count..].iter().map(decimal));
                    if matches!(op.opcode, 0x37 | 0x39 | 0x3e | 0x51..=0x53) {
                        effect.mem = memory_value(&row.memory, self.memory);
                    }
                }
            }
            if let Some(((vm, previous), range)) = self.returned.take()
                && let Some(effect) = self.vm[vm].ops[previous].ex.as_mut()
            {
                effect.used = row.gas as i64;
                effect.push = row.stack.last().map(decimal).into_iter().collect();
                effect.mem = memory_value(&row.memory, range);
            }
            if row.opcode == 0
                && self.frames.len() == 1
                && self
                    .last_op
                    .is_some_and(|(vm, op)| self.vm[vm].ops[op].opcode == 0)
            {
                return;
            }
            let frame = self.frames.last().expect("active frame");
            let vm = &mut self.vm[frame.vm];
            let used = (row.gas as i64).wrapping_sub(row.gas_cost as i64);
            let range = match row.opcode {
                0x37 | 0x39 | 0x3e => stack_range(&row.stack, 0, 2),
                0x51 | 0x52 => stack_word(&row.stack, 0).map(|word| (low_usize(word), 32)),
                0x53 => stack_word(&row.stack, 0).map(|word| (low_usize(word), 1)),
                _ => None,
            };
            if range.is_some() {
                self.memory = range;
            }
            let call_memory = match row.opcode {
                0xf1 | 0xf2 => stack_range(&row.stack, 5, 6),
                0xf4 | 0xfa => stack_range(&row.stack, 4, 5),
                _ => None,
            };
            self.last_op = Some((frame.vm, vm.ops.len()));
            vm.ops.push(VmOp {
                opcode: row.opcode,
                cost: row.gas_cost as i64,
                pc: row.pc,
                ex: (used >= 0).then(|| VmEffect {
                    used,
                    push: Vec::new(),
                    mem: Value::Null,
                    store: row.attempted_sstore.map_or(
                        Value::Null,
                        |store| json!({"key": decimal(&store.key), "val": decimal(&store.value)}),
                    ),
                }),
                sub: None,
                call_memory,
            });
        }
    }

    fn serialize_vm(&self, index: usize) -> Value {
        let frame = &self.vm[index];
        let ops = frame
            .ops
            .iter()
            .enumerate()
            .map(|(index, op)| {
                let effect = op.ex.as_ref().map(|ex| {
                    json!({
                        "used": ex.used, "push": ex.push, "mem": ex.mem, "store": ex.store,
                    })
                });
                let mut row = json!({"cost": op.cost, "ex": effect, "pc": op.pc,
                "op": opcode_name(op.opcode), "idx": format!("{}{index}", frame.index)});
                row["sub"] = op.sub.map_or(Value::Null, |child| self.serialize_vm(child));
                row
            })
            .collect();
        let mut value = json!({"code": bytes(&frame.code)});
        value["ops"] = Value::Array(ops);
        value
    }
}

impl ExecutionTraceObserver for OpenEthereumTrace {
    fn call_frames(&self) -> bool {
        true
    }

    fn observe(&mut self, event: TraceEvent) {
        match event {
            TraceEvent::FrameEnter(frame) => self.enter(frame),
            TraceEvent::FrameExit(frame) => self.exit(frame),
            TraceEvent::Opcode(row) => self.opcode(row),
        }
    }
}

fn bytes(data: &[u8]) -> String {
    let mut output = String::with_capacity(data.len().saturating_mul(2).saturating_add(2));
    output.push_str("0x");
    for byte in data {
        use std::fmt::Write;
        write!(output, "{byte:02x}").expect("writing to String succeeds");
    }
    output
}

fn quantity(value: u64) -> String {
    format!("0x{value:x}")
}

fn decimal(word: &TraceWord) -> String {
    BigUint::from_bytes_be(word).to_str_radix(10)
}

fn stack_word(stack: &[TraceWord], offset: usize) -> Option<&TraceWord> {
    stack.iter().rev().nth(offset)
}

fn low_usize(word: &TraceWord) -> usize {
    u64::from_be_bytes(word[24..].try_into().expect("word contains eight bytes")) as usize
}

fn stack_range(stack: &[TraceWord], start: usize, len: usize) -> Option<MemoryRange> {
    Some((
        low_usize(stack_word(stack, start)?),
        low_usize(stack_word(stack, len)?),
    ))
}

fn memory_value(memory: &[u8], range: Option<MemoryRange>) -> Value {
    let Some((off, len)) = range.filter(|&(_, len)| len != 0) else {
        return Value::Null;
    };
    if let Some(data) = off.checked_add(len).and_then(|end| memory.get(off..end)) {
        return json!({"data": bytes(data), "off": off});
    }
    Value::Null
}

fn stack_outputs(opcode: u8) -> usize {
    match opcode {
        0x60..=0x7f => 1,
        0x80..=0x8f => usize::from(opcode - 0x80) + 2,
        0x90..=0x9f => usize::from(opcode - 0x90) + 2,
        0x01..=0x0b
        | 0x10..=0x1d
        | 0x20
        | 0x30..=0x36
        | 0x38
        | 0x3a..=0x3b
        | 0x3d
        | 0x3f..=0x47
        | 0x51
        | 0x54
        | 0x58..=0x5a => 1,
        _ => 0,
    }
}

pub(crate) fn stack_effect(opcode: u8) -> Option<(usize, usize)> {
    match opcode {
        0xb3 => Some((1, 1)),
        0xb4 => Some((2, 0)),
        _ => revm::bytecode::opcode::OpCode::new(opcode)
            .map(|op| (usize::from(op.inputs()), usize::from(op.outputs()))),
    }
}

fn error_text(error: &CodeExecutionError, frame: &Frame, code: &[u8]) -> String {
    use CodeExecutionError::*;
    if let Some((InstructionResult::NotActivated, opcode, _)) = frame.fault_stack {
        return format!("invalid opcode 0x{opcode:x}");
    }
    let error = match frame.fault_stack.map(|(result, _, _)| result) {
        Some(InstructionResult::StackUnderflow) => &StackUnderflow,
        Some(InstructionResult::StackOverflow) => &StackOverflow,
        Some(InstructionResult::StateChangeDuringStaticCall) => &StaticViolation,
        _ => error,
    };
    match error {
        Revert => "Reverted".into(),
        OutOfGas | CreateCollision | CodeDepositOutOfGas | GasUintOverflow => "Out of gas".into(),
        InvalidOpcode(op) => format!("invalid opcode 0x{op:x}"),
        StackUnderflow => {
            let (opcode, have) = frame
                .fault_stack
                .map(|(_, op, len)| (op, len))
                .unwrap_or_default();
            let need = stack_effect(opcode).map_or(0, |(inputs, _)| inputs);
            format!("stack underflow ({have} <=> {need})")
        }
        StackOverflow => format!(
            "stack limit reached {} (1024)",
            frame.fault_stack.map_or(1024, |(_, _, len)| len)
        ),
        StaticViolation => "Mutable Call In Static Context".into(),
        Depth => "max call depth exceeded".into(),
        ContractSize => "max code size exceeded".into(),
        InvalidJump => {
            let pos = frame.jump_target.unwrap_or_default();
            let op = code.get(low_usize(&pos)).copied().unwrap_or(0);
            let mut words = pos
                .as_chunks::<8>()
                .0
                .iter()
                .rev()
                .map(|word| u64::from_be_bytes(*word));
            let [low, middle_low, middle_high, high] =
                std::array::from_fn(|_| words.next().expect("a word has four limbs"));
            format!(
                "invalid jump destination ({}) [{low} {middle_low} {middle_high} {high}]",
                opcode_name(op)
            )
        }
        ReturnDataOutOfBounds => "return data out of bounds".into(),
        Native(failure) => failure.error.clone(),
    }
}
