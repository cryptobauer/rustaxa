//! Taraxa instruction and gas configuration for the REVM interpreter.
//!
//! The profile starts from Istanbul rather than a later Ethereum hardfork. It
//! enables Taraxa's independently activated additions: PUSH0 and the legacy
//! transient aliases in Californicum, MCOPY in Ficus, and the newer transient
//! aliases in Cacti. Callers keep their normal `Host` implementation; this
//! module neither owns state nor changes frame or transaction semantics.

use revm::{
    bytecode::opcode::{CALL, CALLCODE, DELEGATECALL, STATICCALL},
    context_interface::{
        Host,
        cfg::{GasId, GasParams},
    },
    interpreter::{
        Instruction, InstructionContext, InstructionExecResult, InstructionTable, Interpreter,
        instructions::{self, gas_table_spec},
        interpreter::EthInterpreter,
    },
    primitives::hardfork::SpecId,
};

/// Taraxa instruction-table phase selected by the application hardfork schedule.
///
/// Each phase includes the previous phase. This enum names only instruction and
/// precompile-table generations; it does not activate unrelated DPoS, envelope,
/// gas-limit or Ethereum fork behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TaraxaPhase {
    /// Original Californicum instruction table.
    #[default]
    Californicum,
    /// Ficus inherits Californicum and adds MCOPY at opcode `0x5e`.
    Ficus,
    /// Cacti inherits Ficus, adds transient aliases at `0x5c` and `0x5d`, and
    /// selects Cacti-era stateless registry additions during driver integration.
    Cacti,
}

/// Historical Taraxa interpreter profile for one selected instruction phase.
///
/// Every phase retains PUSH0 at `0x5f` and the legacy transient aliases at
/// `0xb3`/`0xb4`. Ficus and Cacti add MCOPY; Cacti additionally adds the newer
/// transient aliases. All remaining instructions and gas parameters retain the
/// reviewed Istanbul/Californicum base.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaraxaProfile {
    phase: TaraxaPhase,
}

impl TaraxaProfile {
    /// Creates the bounded profile for one execution under the supplied Cacti state.
    ///
    /// This compatibility creator maps `false` to Californicum and `true` to
    /// Cacti, including Cacti's inherited Ficus MCOPY instruction. New callers
    /// that must select Ficus directly use [`Self::for_phase`]. It has no effect
    /// on host behavior, frame rollback, or any unlisted opcode.
    #[must_use]
    pub const fn new(cacti: bool) -> Self {
        Self {
            phase: if cacti {
                TaraxaPhase::Cacti
            } else {
                TaraxaPhase::Californicum
            },
        }
    }

    /// Creates a profile for an explicit Taraxa instruction-table phase.
    ///
    /// Fork-height selection remains with the caller. The phase affects only
    /// the cumulative instruction additions documented by [`TaraxaPhase`].
    #[must_use]
    pub const fn for_phase(phase: TaraxaPhase) -> Self {
        Self { phase }
    }

    /// Returns the exact Taraxa instruction-table phase.
    #[must_use]
    pub const fn phase(self) -> TaraxaPhase {
        self.phase
    }

    /// Returns whether Ficus instruction additions are installed.
    ///
    /// Cacti inherits Ficus, so this is true for both phases.
    #[must_use]
    pub const fn ficus(self) -> bool {
        matches!(self.phase, TaraxaPhase::Ficus | TaraxaPhase::Cacti)
    }

    /// Returns whether the Cacti execution generation is selected.
    ///
    /// In this table it installs the Cacti transient aliases. Driver integration
    /// also uses the result to select Cacti-era stateless registry additions.
    #[must_use]
    pub const fn cacti(self) -> bool {
        matches!(self.phase, TaraxaPhase::Cacti)
    }

    /// Returns Istanbul dynamic gas parameters with the five pinned SSTORE values.
    ///
    /// The returned parameters are suitable for a `Host` passed to an interpreter
    /// using [`Self::instruction_table`]. They preserve Istanbul's remaining gas
    /// policy and override only the net-SSTORE constants observed in the Go table.
    #[must_use]
    pub fn gas_params(self) -> GasParams {
        let mut gas = GasParams::new_spec(SpecId::ISTANBUL);
        gas.override_gas([
            (GasId::sstore_static(), 200),
            (GasId::sstore_set_without_load_cost(), 19_800),
            (GasId::sstore_reset_without_cold_load_cost(), 4_800),
            (GasId::sstore_set_refund(), 19_800),
            (GasId::sstore_reset_refund(), 4_800),
        ]);
        gas
    }

    /// Configures an interpreter to use this profile's Istanbul runtime base.
    ///
    /// The input is the interpreter that will run a table from
    /// [`Self::instruction_table`]. On return, its runtime spec is Istanbul, so
    /// REVM's later Ethereum opcodes and rule changes remain unavailable unless a
    /// future, separately reviewed Taraxa profile explicitly enables them. This
    /// helper does not alter bytecode, gas, host state, or frame state and cannot
    /// validate that the supplied interpreter will subsequently use this profile's
    /// table; callers must use both APIs together.
    pub fn configure_interpreter(&self, interpreter: &mut Interpreter<EthInterpreter>) {
        interpreter.runtime_flag.spec_id = SpecId::ISTANBUL;
    }

    /// Builds a host-generic REVM instruction table and its static opcode costs.
    ///
    /// The table is parameterized by the caller's existing [`Host`] type and
    /// invokes its normal storage and transient methods. It installs the bounded
    /// aliases documented on [`TaraxaProfile`], while all other instructions and
    /// costs come from Istanbul. Call [`Self::configure_interpreter`] before
    /// execution: REVM tables contain later opcode handlers whose guards consult
    /// the interpreter runtime spec. This function does not activate Shanghai,
    /// Cancun, or any other newer Ethereum behavior globally.
    #[must_use]
    pub fn instruction_table<H: Host>(&self) -> (InstructionTable<EthInterpreter, H>, [u16; 256]) {
        let mut table = instructions::instruction_table::<EthInterpreter, H>();
        let mut costs = gas_table_spec(SpecId::ISTANBUL);

        table[0x5f] = Instruction::new(push0::<H>);
        costs[0x5f] = 2;

        table[0xb3] = Instruction::new(transient_load::<H>);
        table[0xb4] = Instruction::new(transient_store::<H>);
        costs[0xb3] = 100;
        costs[0xb4] = 100;

        table[0x55] = Instruction::new(sstore::<H>);

        if self.ficus() {
            table[0x5e] = Instruction::new(mcopy::<H>);
            costs[0x5e] = 3;
        }

        if self.cacti() {
            table[0x5c] = Instruction::new(transient_load::<H>);
            table[0x5d] = Instruction::new(transient_store::<H>);
            costs[0x5c] = 100;
            costs[0x5d] = 100;
        }
        (table, costs)
    }

    /// Builds the driver table with Taraxa frame-admission ordering.
    ///
    /// Account metadata remains authoritative during opcode gas calculation.
    /// The driver must load and validate the referenced bytes after Taraxa's
    /// depth and balance pre-entry checks and before starting a child frame.
    /// SELFDESTRUCT checks operands/static protection and computes gas facts
    /// before charging its base cost and applying account mutation.
    #[must_use]
    pub(crate) fn execution_instruction_table<
        H: Host + DeferredCallCodeLoad + DeferredSelfDestruct,
    >(
        &self,
    ) -> (InstructionTable<EthInterpreter, H>, [u16; 256]) {
        let (mut table, mut costs) = self.instruction_table::<H>();
        table[CALL as usize] = Instruction::new(call_with_deferred_code::<CALL, H>);
        table[CALLCODE as usize] = Instruction::new(call_with_deferred_code::<CALLCODE, H>);
        table[DELEGATECALL as usize] = Instruction::new(call_with_deferred_code::<DELEGATECALL, H>);
        table[STATICCALL as usize] = Instruction::new(call_with_deferred_code::<STATICCALL, H>);
        table[0xff] = Instruction::new(selfdestruct_after_gas::<H>);
        // Go validates stack/static protection and loads quote facts first.
        costs[0xff] = 0;
        (table, costs)
    }
}

/// Executes MCOPY under Taraxa's local Ficus activation.
///
/// REVM guards its implementation with Cancun. The wrapper admits that guard
/// only for this instruction and restores Istanbul on success and every error;
/// it does not expose another Cancun instruction or gas rule.
fn mcopy<H: Host>(ctx: InstructionContext<'_, H, EthInterpreter>) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::CANCUN;
    let interpreter = ctx.interpreter;
    let result = instructions::memory::mcopy(InstructionContext {
        interpreter,
        host: ctx.host,
    });
    interpreter.runtime_flag.spec_id = prior;
    result
}

/// Execution-only host control that separates SELFDESTRUCT quote from mutation.
pub(crate) trait DeferredSelfDestruct {
    /// Enables a read-only quote for the next SELFDESTRUCT opcode.
    fn begin_selfdestruct(&mut self);
    /// Discards the quote, applying its mutation only after successful gas admission.
    fn finish_selfdestruct(
        &mut self,
        admitted: bool,
    ) -> Result<(), revm::context_interface::host::LoadError>;
}

fn selfdestruct_after_gas<H: Host + DeferredSelfDestruct>(
    ctx: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let InstructionContext { interpreter, host } = ctx;
    // Go validates operands before static-write protection.
    if interpreter.stack.is_empty() {
        return Err(revm::interpreter::InstructionResult::StackUnderflow);
    }
    host.begin_selfdestruct();
    let mut result = instructions::host::selfdestruct(InstructionContext { interpreter, host });
    if matches!(
        result,
        Err(revm::interpreter::InstructionResult::SelfDestruct)
    ) && !interpreter.gas.record_regular_cost(5_000)
    {
        result = Err(revm::interpreter::InstructionResult::OutOfGas);
    }
    host.finish_selfdestruct(matches!(
        result,
        Err(revm::interpreter::InstructionResult::SelfDestruct)
    ))?;
    result
}

/// Host control used only while a CALL-family opcode prepares its frame action.
pub(crate) trait DeferredCallCodeLoad {
    /// Selects metadata-only target loading for the current CALL instruction.
    fn set_call_code_load_deferred(&mut self, deferred: bool);
}

fn call_with_deferred_code<const KIND: u8, H: Host + DeferredCallCodeLoad>(
    ctx: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let InstructionContext { interpreter, host } = ctx;
    host.set_call_code_load_deferred(true);
    let result = instructions::contract::call::<KIND, EthInterpreter, H>(InstructionContext {
        interpreter,
        host,
    });
    host.set_call_code_load_deferred(false);
    result
}

/// Executes PUSH0 under its local Taraxa availability rule.
///
/// REVM's implementation guards PUSH0 by Shanghai. This wrapper temporarily
/// admits that one guard and restores the caller's interpreter spec before it
/// returns, so no other Shanghai rule becomes active.
fn push0<H: Host>(ctx: InstructionContext<'_, H, EthInterpreter>) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::SHANGHAI;
    let interpreter = ctx.interpreter;
    let result = instructions::stack::push0(InstructionContext {
        interpreter,
        host: ctx.host,
    });
    interpreter.runtime_flag.spec_id = prior;
    result
}

/// Executes one transient opcode under its local Taraxa availability rule.
///
/// REVM's transient implementations are guarded by Cancun. The wrapper admits
/// only that guard, delegates reads and writes to the caller's [`Host`], and
/// restores the original spec before returning. Frame rollback remains the
/// responsibility of the host/journal layer.
fn transient<const STORE: bool, H: Host>(
    ctx: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::CANCUN;
    let interpreter = ctx.interpreter;
    let result = if STORE {
        instructions::host::tstore(InstructionContext {
            interpreter,
            host: ctx.host,
        })
    } else {
        instructions::host::tload(InstructionContext {
            interpreter,
            host: ctx.host,
        })
    };
    interpreter.runtime_flag.spec_id = prior;
    result
}

/// Dispatches the legacy or Cacti transient store alias.
fn transient_store<H: Host>(
    ctx: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    transient::<true, H>(ctx)
}

/// Dispatches the legacy or Cacti transient load alias.
fn transient_load<H: Host>(
    ctx: InstructionContext<'_, H, EthInterpreter>,
) -> InstructionExecResult {
    transient::<false, H>(ctx)
}

/// Executes Taraxa's Istanbul net-SSTORE accounting without REVM's stipend sentry.
///
/// The pinned Go `gasSStore` implements EIP-1283 accounting but has no
/// EIP-2200 stipend rejection. REVM's shared helper otherwise performs the
/// required static check, stack handling, storage write, and base charge, so the
/// wrapper temporarily selects a pre-Istanbul spec only for that common flow.
/// Its accounting closure restores Istanbul for REVM's existing net cost/refund
/// calculation, and both layers restore the caller's spec on every return path.
fn sstore<H: Host>(ctx: InstructionContext<'_, H, EthInterpreter>) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::PETERSBURG;
    let interpreter = ctx.interpreter;
    let result = instructions::host::sstore_with_gas_accounting(
        InstructionContext {
            interpreter,
            host: ctx.host,
        },
        |context, target, state_load| {
            let flow_spec = context.interpreter.runtime_flag.spec_id;
            context.interpreter.runtime_flag.spec_id = SpecId::ISTANBUL;
            let result =
                instructions::host::sstore_default_gas_accounting(context, target, state_load);
            context.interpreter.runtime_flag.spec_id = flow_spec;
            result
        },
    );
    interpreter.runtime_flag.spec_id = prior;
    result
}
