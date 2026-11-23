//! Checks for the instruction profile and its fault paths.

#[path = "support/profile_host.rs"]
mod fixture_host;

use fixture_host::ProbeHost;
use revm::{
    bytecode::Bytecode,
    interpreter::{Gas, InstructionResult, Interpreter, InterpreterAction},
    primitives::{U256, hardfork::SpecId},
};
use rustaxa_evm::profile::{TaraxaPhase, TaraxaProfile};

/// Confirms the profile runtime rejects unlisted newer instructions and keeps its
/// base spec after both success and error exits from locally admitted aliases.
#[test]
fn bounded_profile_keeps_unlisted_opcodes_unavailable_and_restores_after_errors() {
    let profile = TaraxaProfile::new(true);
    let (table, costs) = profile.instruction_table::<ProbeHost>();

    for (name, code, is_static, expected) in [
        (
            "BASEFEE",
            [0x48, 0x00].as_slice(),
            false,
            InstructionResult::NotActivated,
        ),
        (
            "TSTORE stack underflow",
            [0xb4, 0x00].as_slice(),
            false,
            InstructionResult::StackUnderflow,
        ),
        (
            "TSTORE static violation",
            [0x60, 0x55, 0x60, 0x01, 0xb4, 0x00].as_slice(),
            true,
            InstructionResult::StateChangeDuringStaticCall,
        ),
    ] {
        let mut host = ProbeHost {
            price: U256::from(1),
            gas: profile.gas_params(),
            native_load: false,
            slots: Some(Default::default()),
            transient: Some(Default::default()),
        };
        let mut interpreter =
            Interpreter::default().with_bytecode(Bytecode::new_raw(code.to_vec().into()));
        interpreter.gas = Gas::new(79_000);
        profile.configure_interpreter(&mut interpreter);
        interpreter.runtime_flag.is_static = is_static;

        let InterpreterAction::Return(result) = interpreter.run_plain(&table, &costs, &mut host)
        else {
            panic!("{name} unexpectedly requested a frame");
        };
        assert_eq!(result.result, expected, "{name} must fail as expected");
        assert_eq!(
            interpreter.runtime_flag.spec_id,
            SpecId::ISTANBUL,
            "{name} must restore the bounded runtime base"
        );
    }

    let profile = TaraxaProfile::for_phase(TaraxaPhase::Californicum);
    let (table, costs) = profile.instruction_table::<ProbeHost>();
    let mut host = ProbeHost {
        price: U256::from(1),
        gas: profile.gas_params(),
        native_load: false,
        slots: Some(Default::default()),
        transient: Some(Default::default()),
    };
    let code = [0x60, 0x00, 0x60, 0x00, 0x60, 0x00, 0x5e, 0x00];
    let mut interpreter =
        Interpreter::default().with_bytecode(Bytecode::new_raw(code.to_vec().into()));
    interpreter.gas = Gas::new(79_000);
    profile.configure_interpreter(&mut interpreter);
    let InterpreterAction::Return(result) = interpreter.run_plain(&table, &costs, &mut host) else {
        panic!("Californicum MCOPY unexpectedly requested a frame");
    };
    assert_eq!(result.result, InstructionResult::NotActivated);
    assert_eq!(interpreter.runtime_flag.spec_id, SpecId::ISTANBUL);
}
