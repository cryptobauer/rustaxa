//! Mixed-rule instruction extension feasibility. Only the six single-frame
//! opcode fixtures are claimed; initial ordinary/transient storage is empty.
//! Narrow wrappers enable individual opcodes without changing the frame SpecId.
use super::host::ProbeHost;
use revm::{
    bytecode::Bytecode,
    context_interface::cfg::{GasId, GasParams},
    interpreter::{
        instructions::{self, gas_table_spec},
        interpreter::EthInterpreter,
        *,
    },
    primitives::{U256, hardfork::SpecId},
};
fn push0(ctx: InstructionContext<'_, ProbeHost, EthInterpreter>) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::SHANGHAI;
    let i = ctx.interpreter;
    let result = instructions::stack::push0(InstructionContext {
        interpreter: i,
        host: ctx.host,
    });
    i.runtime_flag.spec_id = prior;
    result
}
fn transient<const STORE: bool>(
    ctx: InstructionContext<'_, ProbeHost, EthInterpreter>,
) -> InstructionExecResult {
    let prior = ctx.interpreter.runtime_flag.spec_id;
    ctx.interpreter.runtime_flag.spec_id = SpecId::CANCUN;
    let i = ctx.interpreter;
    let result = if STORE {
        instructions::host::tstore(InstructionContext {
            interpreter: i,
            host: ctx.host,
        })
    } else {
        instructions::host::tload(InstructionContext {
            interpreter: i,
            host: ctx.host,
        })
    };
    i.runtime_flag.spec_id = prior;
    result
}
#[test]
fn mixed_sstore_and_opcode_profile_matches_go() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/local.json")).unwrap();
    let mut count = 0;
    for row in f["opcodes"].as_array().unwrap() {
        if row["case"] == "nested-transient-revert" {
            continue;
        }
        count += 1;
        let mut gas = GasParams::new_spec(SpecId::ISTANBUL);
        gas.override_gas([
            (GasId::sstore_static(), 200),
            (GasId::sstore_set_without_load_cost(), 19800),
            (GasId::sstore_reset_without_cold_load_cost(), 4800),
            (GasId::sstore_set_refund(), 19800),
            (GasId::sstore_reset_refund(), 4800),
        ]);
        let mut host = ProbeHost {
            price: U256::from(1),
            gas,
            native_load: false,
            slots: Some(Default::default()),
            transient: Some(Default::default()),
        };
        let mut table = instruction_table();
        let mut costs = gas_table_spec(SpecId::ISTANBUL);
        table[0x5f] = Instruction::new(push0);
        costs[0x5f] = 2;
        {
            let (load, store) = (0xb3, 0xb4);
            table[load] = Instruction::new(transient::<false>);
            table[store] = Instruction::new(transient::<true>);
            costs[load] = 100;
            costs[store] = 100;
        }
        if row["cacti"] == true {
            table[0x5c] = Instruction::new(transient::<false>);
            table[0x5d] = Instruction::new(transient::<true>);
            costs[0x5c] = 100;
            costs[0x5d] = 100;
        }
        let mut i = Interpreter::default().with_bytecode(Bytecode::new_raw(
            hex::decode(row["code"].as_str().unwrap()).unwrap().into(),
        ));
        i.gas = Gas::new(79000);
        i.runtime_flag.spec_id = SpecId::ISTANBUL;
        let InterpreterAction::Return(mut result) = i.run_plain(&table, &costs, &mut host) else {
            panic!("unexpected frame")
        };
        assert_eq!(result.result.is_ok(), row["error"] == "");
        if !result.result.is_ok() {
            result.gas.spend_all()
        }
        let refund = result.gas.refunded() as u64;
        assert_eq!(refund, row["refund"].as_u64().unwrap());
        let spent = 100000 - result.gas.remaining();
        assert_eq!(
            spent - refund.min(spent / 2),
            row["gas_used"].as_u64().unwrap(),
            "{}",
            row["case"]
        );
        let value = host
            .transient
            .unwrap()
            .get(&U256::from(1))
            .copied()
            .unwrap_or_default();
        assert_eq!(
            hex::encode(value.to_be_bytes::<32>()),
            row["transient"].as_str().unwrap()
        );
        assert_eq!(
            host.slots
                .unwrap()
                .get(&U256::ZERO)
                .copied()
                .unwrap_or_default()
                .to_string(),
            row["ordinary_slot_zero"].as_str().unwrap()
        );
    }
    assert_eq!(count, 6);
}
