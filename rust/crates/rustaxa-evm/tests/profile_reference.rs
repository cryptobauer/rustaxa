//! Pinned Go opcode-fixture checks for the bounded Taraxa REVM profile.

#[path = "../../../../experiments/evm_feasibility/src/host.rs"]
mod fixture_host;

use fixture_host::ProbeHost;
use revm::{
    bytecode::Bytecode,
    interpreter::{Gas, InstructionResult, Interpreter, InterpreterAction},
    primitives::{U256, hardfork::SpecId},
};
use rustaxa_evm::profile::{TaraxaPhase, TaraxaProfile};

/// Runs the six direct Go single-frame fixtures through the host-generic table.
///
/// The Go fixture remains the source for code, activation, errors, gas, refunds,
/// ordinary storage, and transient storage. The nested rollback row is excluded:
/// frame/journal rollback is outside this bounded profile slice.
#[test]
fn bounded_profile_matches_direct_go_opcode_fixtures() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .expect("pinned Go opcode fixtures must be valid JSON");
    let public_fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/public.json"
    ))
    .expect("pinned public Go fixtures must be valid JSON");
    assert_eq!(
        fixtures["opcodes"], public_fixtures["opcodes"],
        "public and local Go fixture exports must retain the same opcode oracle"
    );
    let mut cases = 0;

    for row in fixtures["opcodes"].as_array().expect("opcode array") {
        if row["case"] == "nested-transient-revert" {
            continue;
        }
        cases += 1;
        let profile = TaraxaProfile::new(row["cacti"].as_bool().expect("cacti flag"));
        let mut host = ProbeHost {
            price: U256::from(1),
            gas: profile.gas_params(),
            native_load: false,
            slots: Some(Default::default()),
            transient: Some(Default::default()),
        };
        let (table, costs) = profile.instruction_table::<ProbeHost>();
        let code = hex::decode(row["code"].as_str().expect("hex code")).expect("valid fixture hex");
        let mut interpreter = Interpreter::default().with_bytecode(Bytecode::new_raw(code.into()));
        interpreter.gas = Gas::new(79_000);
        profile.configure_interpreter(&mut interpreter);

        let InterpreterAction::Return(mut result) =
            interpreter.run_plain(&table, &costs, &mut host)
        else {
            panic!("single-frame fixture unexpectedly requested another frame");
        };
        assert_eq!(result.result.is_ok(), row["error"] == "", "{}", row["case"]);
        assert_eq!(
            interpreter.runtime_flag.spec_id,
            SpecId::ISTANBUL,
            "{} must restore the bounded runtime base after every opcode",
            row["case"]
        );
        if !result.result.is_ok() {
            result.gas.spend_all();
        }
        let spent = 100_000 - result.gas.remaining();
        let refund = u64::try_from(result.gas.refunded()).expect("fixture refund fits u64");
        assert_eq!(
            refund,
            row["refund"].as_u64().expect("refund"),
            "{}",
            row["case"]
        );
        assert_eq!(
            spent - refund.min(spent / 2),
            row["gas_used"].as_u64().expect("gas used"),
            "{}",
            row["case"]
        );

        let transient = host
            .transient
            .expect("fixture transient state")
            .get(&U256::from(1))
            .copied()
            .unwrap_or_default();
        assert_eq!(
            hex::encode(transient.to_be_bytes::<32>()),
            row["transient"].as_str().expect("transient value"),
            "{}",
            row["case"]
        );
        assert_eq!(
            host.slots
                .expect("fixture ordinary storage")
                .get(&U256::ZERO)
                .copied()
                .unwrap_or_default()
                .to_string(),
            row["ordinary_slot_zero"]
                .as_str()
                .expect("ordinary storage value"),
            "{}",
            row["case"]
        );
    }
    assert_eq!(cases, 6);
}

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
