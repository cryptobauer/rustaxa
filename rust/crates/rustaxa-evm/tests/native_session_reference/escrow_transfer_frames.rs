//! Exact active escrow entry: native no-op and frame-owned value/rollback.

use super::*;
use rustaxa_evm::simulation::simulate_with_native;

#[allow(dead_code)]
#[path = "../support/mixed_native.rs"]
mod mixed_native;

const WRAPPER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xa1,
];

fn cases() -> Vec<Value> {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_transfer/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_escrow_transfer/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| {
            !matches!(
                row["name"].as_str().unwrap(),
                "before_phala" | "at_phala" | "trailing" | "inactive_after_cornus"
            )
        })
        .cloned()
        .collect()
}

fn state(row: &Value) -> FixtureState {
    let state = FixtureState::from_case(row);
    {
        let mut rows = state.rows.borrow_mut();
        let mut wrapper = rows.accounts.remove(&TARGET).unwrap();
        wrapper.balance = ConcreteAccountBalance::new(1_000_000_u64.into());
        rows.accounts.insert(WRAPPER, wrapper);
    }
    state
}

fn chain(storage: Arc<Storage>, row: &Value) -> FinalChain {
    FinalChain::new_with_rewards_config(
        storage,
        1_000_000.into(),
        0,
        Vec::new(),
        Vec::new(),
        GenesisDposConfig::default(),
        FinalChainRewardsConfig {
            cornus_period: 0.into(),
            phalaenopsis_period: 0.into(),
            fix_redelegate_block_num: if row["fix"] == 2 { 2.into() } else { 0.into() },
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            ..Default::default()
        },
    )
    .unwrap()
}

fn transaction(row: &Value) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(if row["direct"].as_bool().unwrap() {
            DPOS
        } else {
            WRAPPER
        }),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: 200_000.into(),
        value: ExecutionValue::new(row["value"].as_u64().unwrap().into()),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn block(period: FinalChainBlockNumber) -> ExecutionBlockContext {
    ExecutionBlockContext {
        period,
        author: [0; 20],
        timestamp: 0,
        gas_limit: 1_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}

fn assert_execution(row: &Value, result: TransactionExecutionResult) {
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("Go admitted {}", row["name"])
    };
    assert_eq!(
        result.status,
        if row["parent_error"] == "" {
            CodeExecutionStatus::Success
        } else {
            assert_eq!(row["parent_error"], "execution reverted");
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        },
        "{}",
        row["name"]
    );
    assert_eq!(
        result.gas_used.as_u64(),
        row["transaction_gas_used"].as_u64().unwrap(),
        "{}",
        row["name"]
    );
    assert_eq!(
        result.output,
        bytes(&row["parent_output"]),
        "{}",
        row["name"]
    );
    assert!(result.logs.is_empty());
    assert_eq!(row["logs_count"], 0);
}

#[test]
fn escrow_frames_match_go_value_transfer_and_parent_rollback_without_native_writes() {
    let cases = cases();
    assert_eq!(cases.len(), 8);
    for row in cases {
        let state = state(&row);
        let before = state.rows.borrow().storage.clone();
        let path = temp_db_path("escrow-frame");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone(), &row);
        {
            let mut port = SessionPort {
                session: chain.begin_native_session(1.into(), 0.into()).unwrap(),
                prepared: Vec::new(),
                invoked: Vec::new(),
            };
            let mut journal = ExecutionJournal::new(state.clone());
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &DposNative,
                &DposNative,
                &mut port,
                &mut sequence,
                &block(1.into()),
                &transaction(&row),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            assert_execution(&row, result);
            assert_eq!(sequence.next_sequence(), 1);
            assert_eq!(port.prepared.len(), 1);
            assert_eq!(port.invoked.len(), 1);
            let (invocation, quote) = &port.prepared[0];
            assert_eq!(invocation, &port.invoked[0]);
            assert_eq!(quote.as_u64(), row["required_gas"].as_u64().unwrap());
            assert_eq!(
                invocation.supplied_gas.as_u64(),
                row["supplied_native_gas"].as_u64().unwrap()
            );
            assert_eq!(invocation.caller, address(row["caller"].as_str().unwrap()));
            assert_eq!(invocation.depth, row["depth"].as_u64().unwrap() as u16);
            assert_eq!(
                invocation.value.value(),
                &BigUint::from(if row["static"].as_bool().unwrap() {
                    0
                } else {
                    row["value"].as_u64().unwrap()
                })
            );
            let settled = journal.settle_transaction().unwrap();
            assert_eq!(settled.native_invocations.len(), 1);
            let fact = &settled.native_invocations[0];
            assert!(fact.output.is_empty());
            assert!(fact.logs.is_empty());
            let expected_disposition =
                if !row["native_called"].as_bool().unwrap() || row["native_error"] != "" {
                    ConsensusNativeDisposition::OwnFrameReverted
                } else if row["parent_revert"].as_bool().unwrap() {
                    ConsensusNativeDisposition::OuterFrameReverted
                } else {
                    ConsensusNativeDisposition::Normal
                };
            assert_eq!(fact.disposition, expected_disposition);
            assert!(settled.writes.raw_storage.is_empty());
            assert!(settled.writes.ordinary_storage.is_empty());
            state.apply(settled.writes);
            assert_eq!(state.rows.borrow().storage, before);
            for actor in [SENDER, WRAPPER, DPOS] {
                let expected = &row["accounts"][hex::encode(actor)];
                let ConcreteRead::Present(actual) = state.account(actor).unwrap() else {
                    panic!("missing account")
                };
                assert_eq!(
                    BigUint::from_bytes_be(&actual.account.nonce.to_bytes()),
                    number(expected["nonce"].as_str().unwrap()),
                    "{} nonce",
                    row["name"]
                );
                assert_eq!(
                    actual.account.balance.value(),
                    &number(expected["balance"].as_str().unwrap()),
                    "{} balance",
                    row["name"]
                );
            }
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn escrow_historical_probes_match_go_frames_and_discard_all_account_changes() {
    for row in cases() {
        let reader = state(&row);
        let before = reader.rows.borrow().storage.clone();
        let accounts = [SENDER, WRAPPER, DPOS].map(|actor| reader.account(actor).unwrap());
        let code = reader.rows.borrow().code.clone();
        let path = temp_db_path("escrow-simulation");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone(), &row);
        for _ in 0..2 {
            let result = simulate_with_native(
                &reader,
                &NoHistory,
                &DposNative,
                &DposNative,
                |selected| {
                    assert_eq!(selected, reader.identity());
                    Ok(mixed_native::SimulationNativeExecutionPort::new(
                        chain.begin_native_simulation(selected.period).unwrap(),
                    ))
                },
                &block(reader.identity().period),
                &transaction(&row),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            assert_eq!(result.state, reader.identity());
            assert_execution(&row, result.execution);
            assert_eq!(reader.rows.borrow().storage, before);
            assert_eq!(
                [SENDER, WRAPPER, DPOS].map(|actor| reader.account(actor).unwrap()),
                accounts
            );
            assert_eq!(reader.rows.borrow().code, code);
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
