//! Actual Go ABI/frame composition through the staged session and EVM journal.
//! The synthetic profile retains two validators and an existing delegation.
//! Parent reversion removes logs/account transfers but preserves native raw
//! mutations and semantic advancement. This does not claim DryRunner parity.

use super::*;

/// Records ordered effects before the journal reduces them to final writes.
struct RecordingPort<'a> {
    inner: SessionPort<'a>,
    outcomes: Vec<NativeInvocationResult>,
}
impl NativeExecutionPort for RecordingPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let result = self.inner.invoke(invocation, quote, journal)?;
        self.outcomes.push(result.clone());
        Ok(result)
    }
}

fn addr(last: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = last;
    address
}

fn corpus() -> Vec<Value> {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_frames/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_frames/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public["cases"].as_array().unwrap().clone()
}

fn state(row: &Value) -> FixtureState {
    let state = FixtureState::from_case(row);
    {
        let mut rows = state.rows.borrow_mut();
        let mut wrapper = rows.accounts.remove(&TARGET).unwrap();
        wrapper.balance = ConcreteAccountBalance::new(1_000_000_u64.into());
        rows.accounts.insert(addr(0xd1), wrapper);
        rows.accounts.get_mut(&DPOS).unwrap().balance =
            ConcreteAccountBalance::new(2000_u64.into());
    }
    state
}

fn chain(storage: Arc<Storage>, row: &Value) -> FinalChain {
    FinalChain::new_with_rewards_config_and_ficus_activation(
        storage,
        1_000_000.into(),
        0,
        Vec::new(),
        [0x31, 0x32]
            .into_iter()
            .map(|last| GenesisValidator {
                address: addr(last),
                vrf_key: [if last == 0x31 { 0x44 } else { 0x55 }; 32],
                total_stake: ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                delegations: vec![(
                    addr(0xd1),
                    ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                )],
                metadata: GenesisValidatorMetadata {
                    owner: addr(0xa1),
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
        GenesisDposConfig {
            eligibility_balance_threshold: ethereum_types::U256::from(100).into(),
            vote_eligibility_balance_step: ethereum_types::U256::from(10).into(),
            validator_maximum_stake: ethereum_types::U256::from(1_000_000).into(),
            minimum_deposit: ethereum_types::U256::from(100).into(),
            delegation_delay: 1,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: row["fix"].as_u64().unwrap().into(),
            aspen_part_two_period: if row["aspen_zero"].as_bool().unwrap() {
                1.into()
            } else {
                FinalChainBlockNumber::MAX
            },
            yield_percentage: 0,
            aspen_max_supply: ethereum_types::U256::from(1_000_000_000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap()
}

fn transaction(row: &Value) -> ExecutionTransaction {
    ExecutionTransaction {
        position: 0.into(),
        hash: [0x11; 32],
        sender: if row["direct"].as_bool().unwrap() {
            addr(0xd1)
        } else {
            SENDER
        },
        receiver: Some(if row["direct"].as_bool().unwrap() {
            DPOS
        } else {
            addr(0xd1)
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

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: 1.into(),
        author: [0; 20],
        timestamp: 0,
        gas_limit: 1_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}

#[test]
fn redelegate_frames_match_actual_go_funding_value_errors_and_irreversible_parent_rollback() {
    let cases = corpus();
    assert_eq!(cases.len(), 21);
    let native_success_logs = fixture_logs(
        &cases
            .iter()
            .find(|row| row["name"] == "direct_partial")
            .unwrap()["logs"],
    );
    for row in cases {
        let state = state(&row);
        let path = temp_db_path("redelegate-frames");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone(), &row);
        {
            let mut port = RecordingPort {
                inner: SessionPort {
                    session: chain.begin_native_session(1.into(), 0.into()).unwrap(),
                    prepared: Vec::new(),
                    invoked: Vec::new(),
                },
                outcomes: Vec::new(),
            };
            let mut journal = ExecutionJournal::new(state.clone());
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let execution = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &DposNative,
                &DposNative,
                &mut port,
                &mut sequence,
                &block(),
                &transaction(&row),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            let TransactionExecutionResult::Executed(execution) = execution else {
                panic!("Go admitted {}", row["name"])
            };
            assert_eq!(
                execution.gas_used.as_u64(),
                row["transaction_gas_used"].as_u64().unwrap(),
                "{}",
                row["name"]
            );
            assert_eq!(
                execution.output,
                bytes(&row["parent_output"]),
                "{}",
                row["name"]
            );
            assert_eq!(execution.logs, fixture_logs(&row["logs"]));
            match row["parent_error"].as_str().unwrap() {
                "" => assert_eq!(execution.status, CodeExecutionStatus::Success),
                "execution reverted" => assert_eq!(
                    execution.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::Revert)
                ),
                error => {
                    let CodeExecutionStatus::Failure(CodeExecutionError::Native(failure)) =
                        &execution.status
                    else {
                        panic!("{}: {:?}", row["name"], execution.status)
                    };
                    assert_eq!(failure.error, error);
                }
            }
            assert_eq!(port.inner.prepared.len(), 1);
            assert_eq!(port.inner.invoked.len(), 1);
            assert_eq!(sequence.next_sequence(), 1);
            let (invocation, quote) = &port.inner.prepared[0];
            assert_eq!(invocation, &port.inner.invoked[0]);
            assert_eq!(quote.as_u64(), row["required_gas"].as_u64().unwrap());
            assert_eq!(
                invocation.supplied_gas.as_u64(),
                row["supplied_native_gas"].as_u64().unwrap()
            );
            assert_eq!(invocation.depth, row["depth"].as_u64().unwrap() as u16);
            assert_eq!(invocation.caller, address(row["caller"].as_str().unwrap()));
            assert_eq!(
                invocation.value.value(),
                &BigUint::from(row["value"].as_u64().unwrap())
            );
            assert_eq!(invocation.is_static, row["static"].as_bool().unwrap());
            let settled = journal.settle_transaction().unwrap();
            assert_eq!(settled.native_invocations.len(), 1);
            let fact = &settled.native_invocations[0];
            let success = row["native_called"].as_bool().unwrap() && row["native_error"] == "";
            let disposition = if !success {
                ConsensusNativeDisposition::OwnFrameReverted
            } else if row["parent_revert"].as_bool().unwrap() {
                ConsensusNativeDisposition::OuterFrameReverted
            } else {
                ConsensusNativeDisposition::Normal
            };
            assert_eq!(fact.disposition, disposition);
            assert_eq!(
                fact.logs,
                if success {
                    native_success_logs.clone()
                } else {
                    Vec::new()
                }
            );
            assert_eq!(fact.output, bytes(&row["native_output"]));
            assert_eq!(settled.logs, fixture_logs(&row["logs"]));
            assert!(settled.writes.ordinary_storage.is_empty());
            assert_eq!(port.outcomes.len(), 1);
            let ordered = match &port.outcomes[0] {
                NativeInvocationResult::Completed(outcome) => &outcome.raw_mutations[..],
                NativeInvocationResult::InsufficientGas { .. } => &[],
            };
            let writes = ordered.iter().map(|write| {
                let value = match &write.operation { NativeRawOperation::Put(value) => value.as_bytes(), NativeRawOperation::Delete => &[] };
                serde_json::json!({"address":hex::encode(write.address),"key":hex::encode(write.key.0),"value":hex::encode(value)})
            }).collect::<Vec<_>>();
            assert_eq!(
                writes,
                *row["ordered_raw_writes"].as_array().unwrap(),
                "{}",
                row["name"]
            );
            // The publication plan intentionally retains only the final value
            // per key. Ordered native effects are checked at the port boundary.
            let final_native = ordered
                .iter()
                .map(|write| ((write.address, write.key), write.operation.clone()))
                .collect::<BTreeMap<_, _>>();
            let final_plan = settled
                .writes
                .raw_storage
                .iter()
                .map(|write| ((write.address, write.key), write.operation.clone()))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(final_plan, final_native);
            assert_eq!(
                fact.gas_used,
                if row["native_called"].as_bool().unwrap() {
                    *quote
                } else {
                    FinalChainGas::ZERO
                }
            );
            if row["native_called"].as_bool().unwrap() {
                if success {
                    assert_eq!(fact.status, CodeExecutionStatus::Success);
                } else {
                    let CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) =
                        &fact.status
                    else {
                        panic!("expected native failure")
                    };
                    assert_eq!(error.error, row["native_error"].as_str().unwrap());
                }
            } else {
                assert_eq!(
                    fact.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                );
            }
            state.apply(settled.writes);
            for (key, value) in row["after_raw"].as_object().unwrap() {
                let actual = state.storage(DPOS, ConcreteStorageKey(hash(key))).unwrap();
                let actual = match actual {
                    ConcreteRead::Present(bytes) => bytes,
                    ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
                };
                assert_eq!(actual, bytes(value), "{} raw {key}", row["name"]);
            }
            for actor in [SENDER, addr(0xd1), DPOS] {
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
            // Committed semantic state never changes in this private session.
            assert_eq!(
                chain.dpos_total_amount_delegated(0.into()).unwrap(),
                vec![7, 0xd0]
            );
            if row["parent_revert"].as_bool().unwrap() {
                // A second call must authenticate the advanced semantic state
                // against the irreversible overlay left by the reverted parent.
                let mut next = transaction(&row);
                next.position = 1.into();
                next.nonce = FinalChainNonce::from_u64(2);
                let mut journal = ExecutionJournal::new(state.clone());
                let result = execute_top_level_call_with_native(
                    &mut journal,
                    &NoHistory,
                    &DposNative,
                    &DposNative,
                    &mut port,
                    &mut sequence,
                    &block(),
                    &next,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )
                .unwrap();
                let TransactionExecutionResult::Executed(result) = result else {
                    panic!("repeat admission")
                };
                assert_eq!(
                    result.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::Revert)
                );
                let NativeInvocationResult::Completed(outcome) = &port.outcomes[1] else {
                    panic!("repeat native funding")
                };
                assert_eq!(outcome.status, NativeStatus::Success);
                let oracle: Value = serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_observation/public.json")).unwrap();
                let actual = outcome.raw_mutations.iter().map(|write| {
                    let value = match &write.operation { NativeRawOperation::Put(value) => value.as_bytes(), NativeRawOperation::Delete => &[] };
                    serde_json::json!({"address":hex::encode(write.address),"key":hex::encode(write.key.0),"value":hex::encode(value)})
                }).collect::<Vec<_>>();
                assert_eq!(
                    actual,
                    *oracle["cases"][0]["attempts"][1]["ordered_raw_writes"]
                        .as_array()
                        .unwrap()
                );
                let settled = journal.settle_transaction().unwrap();
                assert!(settled.logs.is_empty());
                assert_eq!(
                    settled.native_invocations[0].disposition,
                    ConsensusNativeDisposition::OuterFrameReverted
                );
            }
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
