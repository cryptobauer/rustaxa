//! Pinned Go opcode-fixture checks for the bounded Taraxa REVM profile.

#[path = "../../../../experiments/evm_feasibility/src/host.rs"]
mod fixture_host;

use fixture_host::ProbeHost;
use revm::{
    bytecode::Bytecode,
    interpreter::{Gas, Interpreter, InterpreterAction},
    primitives::{U256, hardfork::SpecId},
};
use rustaxa_evm::profile::TaraxaProfile;

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
        interpreter.runtime_flag.spec_id = SpecId::ISTANBUL;

        let InterpreterAction::Return(mut result) =
            interpreter.run_plain(&table, &costs, &mut host)
        else {
            panic!("single-frame fixture unexpectedly requested another frame");
        };
        assert_eq!(result.result.is_ok(), row["error"] == "", "{}", row["case"]);
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
