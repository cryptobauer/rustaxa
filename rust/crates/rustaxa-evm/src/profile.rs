//! Bounded Taraxa instruction and gas configuration for the REVM interpreter.
//!
//! The profile starts from Istanbul rather than a later Ethereum hardfork. It
//! enables only the Taraxa opcodes evidenced by the pinned Go fixtures: PUSH0,
//! the legacy transient aliases, and the Cacti transient aliases. Callers keep
//! their normal `Host` implementation; this module neither owns state nor
//! changes frame or transaction semantics.

use revm::{
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

/// The historical Taraxa rules covered by the pinned single-frame opcode fixtures.
///
/// `cacti` controls whether the newer transient aliases at opcodes `0x5c` and
/// `0x5d` are available. The legacy aliases at `0xb3` and `0xb4`, and PUSH0 at
/// `0x5f`, are available in both configurations. This is intentionally not a
/// general fork schedule or a claim that all historical Taraxa rules are known.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaraxaProfile {
    cacti: bool,
}

impl TaraxaProfile {
    /// Creates the bounded profile for one execution under the supplied Cacti state.
    ///
    /// The input selects only the `0x5c`/`0x5d` aliases. It has no effect on the
    /// base Istanbul instruction set, host behavior, frame rollback, or any
    /// unlisted opcode.
    #[must_use]
    pub const fn new(cacti: bool) -> Self {
        Self { cacti }
    }

    /// Returns whether the Cacti transient aliases are installed.
    #[must_use]
    pub const fn cacti(self) -> bool {
        self.cacti
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

        if self.cacti {
            table[0x5c] = Instruction::new(transient_load::<H>);
            table[0x5d] = Instruction::new(transient_store::<H>);
            costs[0x5c] = 100;
            costs[0x5d] = 100;
        }
        (table, costs)
    }
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
