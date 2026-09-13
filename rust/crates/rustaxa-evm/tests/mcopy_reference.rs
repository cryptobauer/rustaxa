//! Differential Ficus/Cacti MCOPY coverage against both pinned Go revisions.
//!
//! The corpus executes complete single-frame EVM programs. It covers the
//! Taraxa activation boundary, Cacti inheritance, zero-length full-width
//! offsets, overlapping copies, memory expansion, dynamic gas, and stack
//! errors. Frame journaling, state, and native-contract routing are outside
//! this instruction-profile test.

#[path = "../../../../experiments/evm_feasibility/src/host.rs"]
mod fixture_host;

use fixture_host::ProbeHost;
use revm::{
    bytecode::Bytecode,
    interpreter::{Gas, InstructionResult, Interpreter, InterpreterAction},
    primitives::{U256, hardfork::SpecId},
};
use rustaxa_evm::profile::{TaraxaPhase, TaraxaProfile};
use serde_json::Value;

const TRANSACTION_GAS_LIMIT: u64 = 100_000;
const INTRINSIC_GAS: u64 = 21_000;

fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/mcopy/public.json"
    ))
    .expect("pinned public MCOPY fixture must be valid JSON");
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/mcopy/local.json"
    ))
    .expect("pinned local MCOPY fixture must be valid JSON");
    assert_eq!(public, local, "both Go pins must emit identical MCOPY rows");
    public
}

fn phase(name: &str) -> TaraxaPhase {
    match name {
        "californicum" => TaraxaPhase::Californicum,
        "ficus" => TaraxaPhase::Ficus,
        "cacti" => TaraxaPhase::Cacti,
        other => panic!("unknown fixture phase: {other}"),
    }
}

fn host(profile: TaraxaProfile) -> ProbeHost {
    ProbeHost {
        price: U256::from(1),
        gas: profile.gas_params(),
        native_load: false,
        slots: Some(Default::default()),
        transient: Some(Default::default()),
    }
}

/// Runs every pinned Go program through the matching explicit Taraxa phase.
#[test]
fn mcopy_matches_go_activation_output_gas_and_errors() {
    let corpus = corpus();
    let rows = corpus["mcopy"].as_array().expect("MCOPY row array");
    assert_eq!(rows.len(), 10);

    for row in rows {
        let name = row["name"].as_str().expect("case name");
        let profile =
            TaraxaProfile::for_phase(phase(row["phase"].as_str().expect("Taraxa phase name")));
        let (table, costs) = profile.instruction_table::<ProbeHost>();
        let code = hex::decode(row["code"].as_str().expect("hex code"))
            .expect("fixture code must be valid hex");
        let mut interpreter = Interpreter::default().with_bytecode(Bytecode::new_raw(code.into()));
        interpreter.gas = Gas::new(TRANSACTION_GAS_LIMIT - INTRINSIC_GAS);
        profile.configure_interpreter(&mut interpreter);
        let mut host = host(profile);

        let InterpreterAction::Return(mut result) =
            interpreter.run_plain(&table, &costs, &mut host)
        else {
            panic!("{name} unexpectedly requested another frame");
        };
        assert_eq!(
            interpreter.runtime_flag.spec_id,
            SpecId::ISTANBUL,
            "{name} must restore the bounded runtime spec"
        );
        assert_eq!(
            result.result.is_ok(),
            row["execution_error"] == "",
            "{name}: Rust completion must match Go"
        );
        assert_eq!(
            hex::encode(&result.output),
            row["output"].as_str().expect("hex output"),
            "{name}: returned memory"
        );

        match row["execution_error"].as_str().expect("execution error") {
            "" => {}
            error if error.starts_with("invalid opcode") => {
                assert_eq!(result.result, InstructionResult::NotActivated, "{name}");
            }
            "out of gas" => {
                assert!(
                    matches!(
                        result.result,
                        InstructionResult::MemoryOOG | InstructionResult::OutOfGas
                    ),
                    "{name}: unexpected Rust gas error {:?}",
                    result.result
                );
            }
            error if error.starts_with("stack underflow") => {
                assert_eq!(result.result, InstructionResult::StackUnderflow, "{name}");
            }
            error => panic!("{name}: unhandled Go error {error}"),
        }

        if !result.result.is_ok() {
            result.gas.spend_all();
        }
        assert_eq!(
            TRANSACTION_GAS_LIMIT - result.gas.remaining(),
            row["gas_used"].as_u64().expect("gas used"),
            "{name}: total transaction gas"
        );
    }
}

/// Pins the cumulative phase API while preserving the legacy boolean creator.
#[test]
fn explicit_phase_api_is_cumulative_and_compatible() {
    let californicum = TaraxaProfile::for_phase(TaraxaPhase::Californicum);
    let ficus = TaraxaProfile::for_phase(TaraxaPhase::Ficus);
    let cacti = TaraxaProfile::for_phase(TaraxaPhase::Cacti);

    assert_eq!(TaraxaProfile::new(false), californicum);
    assert_eq!(TaraxaProfile::new(true), cacti);
    assert_eq!(californicum.phase(), TaraxaPhase::Californicum);
    assert_eq!(ficus.phase(), TaraxaPhase::Ficus);
    assert_eq!(cacti.phase(), TaraxaPhase::Cacti);
    assert!(!californicum.ficus());
    assert!(ficus.ficus());
    assert!(cacti.ficus());
    assert!(!californicum.cacti());
    assert!(!ficus.cacti());
    assert!(cacti.cacti());
}
