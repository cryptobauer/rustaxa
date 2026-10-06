//! Actual two-transaction existing-destination current-node frame composition with one public native
//! session and period sequence across real prefix settlement. Go cumulative logs
//! compare to concatenated Rust receipts; the second journal compares target suffix.
//! Live dirty-row authority proves ordered effects and rollback, not trie/root parity.

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
        // Preserve LAST effective writes across all successes, including before a
        // new target journal and after a later normal failure with no mutations.
        let reduced = self
            .outcomes
            .iter()
            .filter_map(|result| match result {
                NativeInvocationResult::Completed(outcome) => Some(outcome),
                _ => None,
            })
            .flat_map(|outcome| outcome.raw_mutations.iter())
            .map(|write| ((write.address, write.key), &write.operation))
            .collect::<BTreeMap<_, _>>();
        for ((address, key), operation) in reduced {
            let expected = match operation {
                NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
                NativeRawOperation::Delete => None,
            };
            assert_eq!(
                logical(journal.raw_storage(address, &key).unwrap()),
                expected
            );
        }
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

// Go deletes and the raw trace's empty marker denote the same logical absence.
fn logical(read: ConcreteRead<Vec<u8>>) -> Option<Vec<u8>> {
    match read {
        ConcreteRead::Present(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

fn assert_swapped_membership(
    ordered: &[&NativeRawMutation],
    final_plan: &BTreeMap<([u8; 20], ConcreteStorageKey), NativeRawOperation>,
) {
    let prefix = [&[2, 1][..], &addr(0xd1)].concat();
    let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
    let moved_item = key(&[&prefix, &[2], &1_u32.to_le_bytes()]);
    let moved_position = key(&[&prefix, &[2], &addr(0x32)]);
    let item = key(&[&prefix, &[2], &2_u32.to_le_bytes()]);
    let count = key(&[&prefix, &[1]]);
    let source_position = key(&[&prefix, &[2], &addr(0x31)]);
    let destination_position = key(&[&prefix, &[2], &addr(0x32)]);
    let value = |operation: &NativeRawOperation| match operation {
        NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
        NativeRawOperation::Delete => None,
    };
    let operations = |key| {
        ordered
            .iter()
            .filter(|write| write.key == key)
            .map(|write| value(&write.operation))
            .collect::<Vec<_>>()
    };
    assert_eq!(operations(item), vec![None]);
    assert_eq!(operations(count), vec![Some(1_u32.to_le_bytes().to_vec())]);
    assert_eq!(operations(source_position), vec![None]);
    assert_eq!(operations(moved_item), vec![Some(addr(0x32).to_vec())]);
    assert_eq!(
        operations(moved_position),
        vec![Some(1_u32.to_le_bytes().to_vec())]
    );
    assert_eq!(
        operations(destination_position),
        vec![Some(1_u32.to_le_bytes().to_vec())]
    );
    for (key, expected) in [
        (item, None),
        (moved_item, Some(addr(0x32).to_vec())),
        (moved_position, Some(1_u32.to_le_bytes().to_vec())),
        (count, Some(1_u32.to_le_bytes().to_vec())),
        (source_position, None),
        (destination_position, Some(1_u32.to_le_bytes().to_vec())),
    ] {
        assert_eq!(value(final_plan.get(&(DPOS, key)).unwrap()), expected);
    }
}

fn addr(last: u8) -> [u8; 20] {
    let mut address = [0; 20];
    address[19] = last;
    address
}

fn corpus() -> Vec<Value> {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_existing_current_frames/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_existing_current_frames/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public["cases"].as_array().unwrap().clone()
}

fn state(row: &Value) -> FixtureState {
    let mut initial = row.clone();
    initial["prior_raw"] = row["prefix"]["prior_raw"].clone();
    let state = FixtureState::from_case(&initial);
    {
        let mut rows = state.rows.borrow_mut();
        let mut wrapper = rows.accounts.remove(&TARGET).unwrap();
        wrapper.balance = ConcreteAccountBalance::new(1_000_000_u64.into());
        rows.accounts.insert(addr(0xd1), wrapper);
        rows.accounts.get_mut(&DPOS).unwrap().balance =
            ConcreteAccountBalance::new(4000_u64.into());
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
                vrf_key: [match last {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: ethereum_types::U256::from(2000).to_big_endian().to_vec(),
                delegations: vec![(
                    addr(0xa1),
                    ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                )]
                .into_iter()
                .chain(Some((
                    addr(0xd1),
                    ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                )))
                .collect(),
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
            aspen_part_one_period: FinalChainBlockNumber::MAX,
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

fn transaction(row: &Value, position: u32) -> ExecutionTransaction {
    ExecutionTransaction {
        position: position.into(),
        hash: [if position == 0 { 0x11 } else { 0x22 }; 32],
        sender: address(row["from"].as_str().unwrap()),
        receiver: Some(address(row["to"].as_str().unwrap())),
        nonce: FinalChainNonce::from_u64(row["nonce"].as_u64().unwrap()),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: row["gas"].as_u64().unwrap().into(),
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

fn writes_json(ordered: &[&NativeRawMutation]) -> Value {
    serde_json::json!(ordered.iter().map(|write| {
        let value = match &write.operation { NativeRawOperation::Put(value) => value.as_bytes(), NativeRawOperation::Delete => &[] };
        serde_json::json!({"address":hex::encode(write.address),"key":hex::encode(write.key.0),"value":hex::encode(value)})
    }).collect::<Vec<_>>())
}

#[test]
fn existing_current_two_transaction_frames_match_actual_prefix_and_target() {
    let cases = corpus();
    assert_eq!(cases.len(), 7);
    for row in cases {
        let state = state(&row);
        let path = temp_db_path("existing-current-frames");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let chain = chain(storage.clone(), &row);
        let committed_principal = chain.dpos_total_amount_delegated(0.into()).unwrap();
        let committed = chain
            .dpos_validators_total_stakes(0.into())
            .unwrap()
            .into_iter()
            .map(|row| (row.address, row.stake))
            .collect::<Vec<_>>();
        {
            let mut port = RecordingPort {
                inner: SessionPort {
                    session: chain.begin_native_session(1.into(), 0.into()).unwrap(),
                    prepared: Vec::new(),
                    invoked: Vec::new(),
                },
                outcomes: Vec::new(),
            };
            let mut sequence = PeriodConsensusSequence::new(1.into());
            let mut receipt_logs = Vec::new();
            let mut prefix_row = row["prefix"].clone();
            for (key, value) in [
                ("name", row["name"].clone()),
                ("static", false.into()),
                ("parent_revert", false.into()),
                ("two_calls", false.into()),
            ] {
                prefix_row[key] = value;
            }
            for (position, row) in [prefix_row, row].into_iter().enumerate() {
                let start = port.outcomes.len();
                let mut journal = ExecutionJournal::new(state.clone());
                let execution = execute_top_level_call_with_native(
                    &mut journal,
                    &NoHistory,
                    &DposNative,
                    &DposNative,
                    &mut port,
                    &mut sequence,
                    &block(),
                    &transaction(&row, position as u32),
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
                assert_eq!(execution.output, bytes(&row["parent_output"]));
                assert_eq!(execution.logs, fixture_logs(&row["logs"]));
                match row["parent_error"].as_str().unwrap() {
                    "" => assert_eq!(execution.status, CodeExecutionStatus::Success),
                    "execution reverted" => assert_eq!(
                        execution.status,
                        CodeExecutionStatus::Failure(CodeExecutionError::Revert)
                    ),
                    error => panic!("unexpected reference parent error {error}"),
                }
                let calls = row["calls"].as_array().unwrap();
                assert_eq!(
                    calls.len(),
                    if row["two_calls"].as_bool().unwrap() {
                        2
                    } else {
                        1
                    }
                );
                assert_eq!(port.inner.prepared.len() - start, calls.len());
                assert_eq!(port.inner.invoked.len() - start, calls.len());
                assert_eq!(port.outcomes.len() - start, calls.len());
                assert_eq!(sequence.next_sequence(), (start + calls.len()) as u64);
                let settled = journal.settle_transaction().unwrap();
                assert_eq!(settled.native_invocations.len(), calls.len());
                let mut ordered = Vec::new();
                let mut prefix = row["prior_raw"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .filter_map(|(key, value)| {
                        let value = bytes(value);
                        (!value.is_empty()).then_some((ConcreteStorageKey(hash(key)), value))
                    })
                    .collect::<BTreeMap<_, _>>();
                for (index, call) in calls.iter().enumerate() {
                    let (invocation, quote) = &port.inner.prepared[start + index];
                    assert_eq!(invocation, &port.inner.invoked[start + index]);
                    assert_eq!(invocation.id.sequence, (start + index) as u64);
                    assert_eq!(invocation.id.transaction, (position as u32).into());
                    assert_eq!(invocation.period, 1.into());
                    assert_eq!(
                        invocation.kind,
                        if call["route_staticcall"] == true {
                            NativeCallKind::StaticCall
                        } else {
                            NativeCallKind::Call
                        }
                    );
                    assert_eq!(quote.as_u64(), call["required_gas"].as_u64().unwrap());
                    assert_eq!(
                        invocation.supplied_gas.as_u64(),
                        call["supplied_native_gas"].as_u64().unwrap()
                    );
                    assert_eq!(invocation.depth, call["depth"].as_u64().unwrap() as u16);
                    assert_eq!(invocation.caller, address(call["caller"].as_str().unwrap()));
                    assert_eq!(invocation.input, bytes(&call["input"]));
                    assert_eq!(
                        invocation.value.value(),
                        &BigUint::from(row["value"].as_u64().unwrap())
                    );
                    assert_eq!(
                        invocation.is_static,
                        call["route_staticcall"].as_bool().unwrap()
                    );
                    let success =
                        call["native_called"].as_bool().unwrap() && call["native_error"] == "";
                    let fact = &settled.native_invocations[index];
                    assert_eq!(
                        fact.disposition,
                        if !success {
                            ConsensusNativeDisposition::OwnFrameReverted
                        } else if row["parent_revert"].as_bool().unwrap() {
                            ConsensusNativeDisposition::OuterFrameReverted
                        } else {
                            ConsensusNativeDisposition::Normal
                        }
                    );
                    assert_eq!(fact.logs, fixture_logs(&call["logs"]));
                    assert_eq!(fact.output, bytes(&call["native_output"]));
                    assert_eq!(
                        fact.gas_used,
                        if call["native_called"].as_bool().unwrap() {
                            *quote
                        } else {
                            FinalChainGas::ZERO
                        }
                    );
                    if !call["native_called"].as_bool().unwrap() {
                        assert_eq!(
                            fact.status,
                            CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                        );
                        assert!(matches!(
                            port.outcomes[start + index],
                            NativeInvocationResult::InsufficientGas { .. }
                        ));
                        assert_eq!(call["ordered_raw_writes"], serde_json::json!([]));
                        continue;
                    }
                    if success {
                        assert_eq!(fact.status, CodeExecutionStatus::Success);
                    } else {
                        let CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) =
                            &fact.status
                        else {
                            panic!("expected native failure")
                        };
                        assert_eq!(error.error, call["native_error"].as_str().unwrap());
                    }
                    let NativeInvocationResult::Completed(outcome) = &port.outcomes[start + index]
                    else {
                        panic!("expected funded native outcome")
                    };
                    assert!(outcome.account_mutations.is_empty());
                    assert_eq!(outcome.output, bytes(&call["native_output"]));
                    assert_eq!(outcome.logs, fixture_logs(&call["logs"]));
                    let current = outcome.raw_mutations.iter().collect::<Vec<_>>();
                    assert_eq!(
                        writes_json(&current),
                        call["ordered_raw_writes"],
                        "{} call{index}",
                        row["name"]
                    );
                    for write in &current {
                        assert_eq!(
                            logical(write.expected.clone()),
                            prefix.get(&write.key).cloned(),
                            "{} call{index} intermediate {}",
                            row["name"],
                            hex::encode(write.key.0)
                        );
                        match &write.operation {
                            NativeRawOperation::Put(value) => {
                                prefix.insert(write.key, value.as_bytes().to_vec());
                            }
                            NativeRawOperation::Delete => {
                                prefix.remove(&write.key);
                            }
                        }
                    }
                    ordered.extend(current);
                }
                assert_eq!(writes_json(&ordered), row["ordered_raw_writes"]);
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
                if position == 1
                    && row["calls"][0]["native_called"] == true
                    && row["calls"][0]["native_error"] == ""
                {
                    let source_key =
                        ConcreteStorageKey(hash(row["source_delegation_key"].as_str().unwrap()));
                    assert_eq!(
                        final_plan.get(&(DPOS, source_key)),
                        Some(&NativeRawOperation::Delete)
                    );
                }
                if position == 1
                    && row["calls"][0]["native_called"] == true
                    && row["calls"][0]["native_error"] == ""
                {
                    assert_swapped_membership(&ordered, &final_plan);
                    let source_current =
                        ConcreteStorageKey(hash(row["source_current_key"].as_str().unwrap()));
                    let operations = ordered
                        .iter()
                        .filter(|write| write.key == source_current)
                        .map(|write| match &write.operation {
                            NativeRawOperation::Put(value) => hex::encode(value.as_bytes()),
                            NativeRawOperation::Delete => String::new(),
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(operations, vec!["c28001", "c28002"]);
                }
                assert_eq!(settled.logs, fixture_logs(&row["logs"]));
                assert!(settled.writes.ordinary_storage.is_empty());
                if row["two_calls"].as_bool().unwrap() {
                    assert_eq!(ordered.len(), 15);
                    assert!(settled.logs.is_empty());
                    assert_eq!(
                        settled.native_invocations[0].disposition,
                        ConsensusNativeDisposition::OuterFrameReverted
                    );
                    assert_eq!(
                        settled.native_invocations[1].disposition,
                        ConsensusNativeDisposition::OwnFrameReverted
                    );
                    assert_eq!(
                        settled.native_invocations[0].status,
                        CodeExecutionStatus::Success
                    );
                    assert_eq!(row["calls"][1]["native_error"], "Delegation does not exist");
                    assert_eq!(row["calls"][1]["ordered_raw_writes"], serde_json::json!([]));
                }
                receipt_logs.extend(settled.logs.clone());
                assert_eq!(receipt_logs, fixture_logs(&row["cumulative_logs"]));
                assert_eq!(row["refund_before"], 0);
                assert_eq!(row["refund_after"], 0);
                state.apply(settled.writes);
                assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
                assert_eq!(
                    chain.dpos_total_amount_delegated(0.into()).unwrap(),
                    committed_principal
                );
                for (key, value) in row["after_raw"].as_object().unwrap() {
                    let actual = state.storage(DPOS, ConcreteStorageKey(hash(key))).unwrap();
                    let actual = match actual {
                        ConcreteRead::Present(bytes) => bytes,
                        ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
                    };
                    assert_eq!(actual, bytes(value), "{} raw{key}", row["name"]);
                }
                for actor in [SENDER, addr(0xd1), DPOS] {
                    let expected = &row["accounts"][hex::encode(actor)];
                    let ConcreteRead::Present(actual) = state.account(actor).unwrap() else {
                        panic!("missing account")
                    };
                    assert_eq!(
                        BigUint::from_bytes_be(&actual.account.nonce.to_bytes()),
                        number(expected["nonce"].as_str().unwrap())
                    );
                    assert_eq!(
                        actual.account.balance.value(),
                        &number(expected["balance"].as_str().unwrap())
                    );
                }
                assert_eq!(
                    chain
                        .dpos_validators_total_stakes(0.into())
                        .unwrap()
                        .into_iter()
                        .map(|row| (row.address, row.stake))
                        .collect::<Vec<_>>(),
                    committed
                );
            }
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
