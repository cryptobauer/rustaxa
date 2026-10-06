//! Actual full source removal and new destination insertion for a one-member caller.
//! The complete synthetic native seed grants test-only absence authority. This
//! staged boundary does not adopt a concrete checkpoint or claim public API parity.
use super::*;

fn full_new_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_full_new/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_full_new/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    public
}

fn full_new_raw(case: &Value) -> AuthenticatedRaw {
    let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    for row in case["seed_raw"].as_array().unwrap() {
        if row["present"] == true {
            raw.rows.insert(
                ConcreteStorageKey(unhex(row["key"].as_str().unwrap()).try_into().unwrap()),
                unhex(row["value"].as_str().unwrap()),
            );
        }
    }
    raw
}

fn assert_facts(snapshot: &DposSnapshot, facts: &Value) {
    for owner in [address(0xd1), address(0xa1)] {
        let members = snapshot
            .delegator_validators
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            json!(members.iter().map(|member| hex(member)).collect::<Vec<_>>()),
            facts["memberships"][hex(&owner)]
        );
        for validator in [address(0x31), address(0x32)] {
            let row = &facts["delegations"][format!("{}/{}", hex(&owner), hex(&validator))];
            if row.is_null() {
                assert!(delegation(snapshot, validator, owner).is_none());
                assert!(matches!(
                    snapshot
                        .reward_reference_graph
                        .read_cursor(&validator, &owner),
                    Err(DposRewardGraphError::MissingCursor { .. })
                ));
                assert!(
                    snapshot
                        .delegation_reward_cursors
                        .get(&validator)
                        .is_none_or(|rows| !rows.contains_key(&owner))
                );
            } else {
                assert_eq!(
                    delegation(snapshot, validator, owner).unwrap().to_string(),
                    row["stake"]
                );
                assert_eq!(
                    snapshot
                        .reward_reference_graph
                        .read_cursor(&validator, &owner)
                        .unwrap(),
                    row["last_updated"].as_u64().unwrap()
                );
            }
        }
    }
}

fn assert_aborted(session: &FinalChainNativeSession<'_>, before: &DposSnapshot, sequence: u64) {
    assert_eq!(&session.dpos_state, before);
    assert_eq!(session.next_sequence, sequence);
    assert!(session.aborted && session.prepared.is_none());
}

#[test]
fn redelegate_full_new_matches_actual_ordered_effects_and_warm_failure() {
    let oracle = full_new_oracle();
    let case = &oracle["cases"][0];
    assert_eq!(case["name"], "full_source_only_item_new_destination");
    let (chain, storage, path) = kernel_chain_profile(1_000_000, address(0xa1), Some(0x31));
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    assert_facts(&session.dpos_state, &case["facts_before"]);
    let before = session.dpos_state.clone();
    let mut next = before.clone();
    let mut accounts = StagedDposAccountPort::new([]).unwrap();
    let kernel = chain
        .apply_dpos_redelegate(
            &mut next,
            &mut accounts,
            address(0xd1),
            address(0x31),
            address(0x32),
            U256::from(1000).to_big_endian().to_vec(),
            1.into(),
        )
        .unwrap();
    assert_eq!(kernel.status_code, 1);
    assert!(accounts.into_mutations().is_empty());
    let mut raw = full_new_raw(case);
    let request = staged_request(case, 0);
    let quote = session.prepare(&request, &raw).unwrap();
    assert_eq!(quote.required_gas, 80_000.into());
    assert!(raw.reads.borrow().is_empty());
    let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
    assert_eq!(outcome.status, FinalChainNativeStatus::Success);
    assert_eq!(session.dpos_state, next);
    assert_eq!(
        write_json(&outcome.raw_mutations),
        case["attempts"][0]["ordered_raw_writes"]
    );
    assert_eq!(log_json(&outcome.logs), case["attempts"][0]["logs"]);
    assert_eq!(
        outcome.output,
        unhex(case["attempts"][0]["output"].as_str().unwrap())
    );
    assert!(outcome.account_mutations.is_empty());
    assert_eq!(session.dpos_state.validator_order, before.validator_order);
    assert_eq!(session.dpos_state.total_vote_count, before.total_vote_count);
    assert_facts(&session.dpos_state, &case["facts_after"]);
    assert_eq!(
        session.dpos_state.total_stakes[&address(0x31)].as_u256(),
        U256::from(1000)
    );
    assert_eq!(
        session.dpos_state.total_stakes[&address(0x32)].as_u256(),
        U256::from(2000)
    );
    // The same count and item keys are deleted/reset, then reused by insertion.
    // Apply every intermediate expectation in order, without collapsing effects.
    raw.apply(&outcome.raw_mutations);
    raw.rows.retain(|_, value| !value.is_empty());
    for (key, observed) in case["final_raw"].as_object().unwrap() {
        let key = ConcreteStorageKey(unhex(key).try_into().unwrap());
        assert_eq!(
            raw.rows.contains_key(&key),
            observed["present"].as_bool().unwrap()
        );
        assert_eq!(
            raw.rows.get(&key).cloned().unwrap_or_default(),
            unhex(observed["value"].as_str().unwrap())
        );
    }
    for (key, node) in session
        .dpos_state
        .reward_reference_graph
        .live_nodes()
        .unwrap()
    {
        let raw_key = reward_node_key(key.validator, key.block);
        if let Some(observed) = case["final_raw"].get(hex(&raw_key.0)) {
            assert_eq!(
                encode_node(&node),
                unhex(observed["value"].as_str().unwrap())
            );
        }
    }
    assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
    assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    raw.reads.borrow_mut().clear();
    let warm = session.dpos_state.clone();
    let request = staged_request(case, 1);
    let quote = session.prepare(&request, &raw).unwrap();
    let failure = completed(session.invoke(&request, quote, &raw).unwrap());
    assert_eq!(
        failure.status,
        FinalChainNativeStatus::ContractFailure {
            error: case["attempts"][1]["execution_error"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    );
    assert!(
        failure.raw_mutations.is_empty()
            && failure.account_mutations.is_empty()
            && failure.logs.is_empty()
    );
    assert_eq!(session.dpos_state, warm);
    assert_eq!(session.next_sequence, 2);
    assert!(!session.aborted);
    assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
    println!(
        "full+new warm failure: {} fresh Rust authentication reads",
        raw.reads.borrow().len()
    );
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_full_new_all_cold_and_warm_authentication_failures_are_atomic() {
    let oracle = full_new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_profile(1_000_000, address(0xa1), Some(0x31));
    for warm in [false, true] {
        let mut success = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut reference = full_new_raw(case);
        let first = staged_request(case, 0);
        let quote = success.prepare(&first, &reference).unwrap();
        let first_outcome = completed(success.invoke(&first, quote, &reference).unwrap());
        let cold_keys = reference.reads.borrow().clone();
        reference.apply(&first_outcome.raw_mutations);
        reference.rows.retain(|_, value| !value.is_empty());
        reference.reads.borrow_mut().clear();
        let second = staged_request(case, 1);
        let quote = success.prepare(&second, &reference).unwrap();
        success.invoke(&second, quote, &reference).unwrap();
        let keys = if warm {
            reference.reads.borrow().clone()
        } else {
            cold_keys
        };
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            keys.len()
        );
        println!(
            "full+new warm={warm}: {} fresh Rust authentication reads",
            keys.len()
        );
        for (index, key) in keys.iter().enumerate() {
            for reader_failure in [false, true] {
                let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let mut raw = full_new_raw(case);
                if warm {
                    let quote = session.prepare(&first, &raw).unwrap();
                    let outcome = completed(session.invoke(&first, quote, &raw).unwrap());
                    raw.apply(&outcome.raw_mutations);
                    raw.rows.retain(|_, value| !value.is_empty());
                }
                let before = session.dpos_state.clone();
                let sequence = u64::from(warm);
                raw.reads.borrow_mut().clear();
                if reader_failure {
                    raw.fail_at = Some(index + 1);
                } else {
                    raw.rows.insert(*key, vec![0xff]);
                }
                let request = staged_request(case, sequence);
                let quote = session.prepare(&request, &raw).unwrap();
                let error = session.invoke(&request, quote, &raw).unwrap_err();
                assert!(
                    if reader_failure {
                        matches!(error, FinalChainNativeSessionError::StateRead(_))
                    } else {
                        matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                    },
                    "warm={warm} {key:?}: {error:?}"
                );
                assert_aborted(&session, &before, sequence);
            }
        }
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_full_new_rejects_unobserved_or_incomplete_membership_shapes() {
    let oracle = full_new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_profile(1_000_000, address(0xa1), Some(0x31));
    for shape in 0..7 {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        match shape {
            0 => {
                session
                    .dpos_state
                    .delegator_validators
                    .remove(&address(0xd1));
            }
            1 => {
                session
                    .dpos_state
                    .delegator_validators
                    .insert(address(0xd1), vec![address(0x31), address(0x99)]);
            }
            2 => {
                session
                    .dpos_state
                    .delegator_validators
                    .insert(address(0xd1), vec![address(0x31), address(0x31)]);
            }
            3 => {
                session
                    .dpos_state
                    .delegator_validators
                    .insert(address(0xd1), vec![address(0x32)]);
            }
            4 => {
                session.dpos_state.delegation_ledger_history_complete = false;
            }
            5 | 6 => {
                let validator = address(if shape == 5 { 0x31 } else { 0x32 });
                let graph = &mut session.dpos_state.reward_reference_graph;
                let mut node = graph
                    .load_node(&NodeKey {
                        validator,
                        block: 0,
                    })
                    .unwrap();
                node.count = 1;
                graph
                    .bootstrap_node(
                        NodeKey {
                            validator,
                            block: graph.current_block().unwrap(),
                        },
                        node,
                        false,
                        &[],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let before = session.dpos_state.clone();
        let raw = full_new_raw(case);
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        assert_eq!(
            session.invoke(&request, quote, &raw).unwrap_err(),
            FinalChainNativeSessionError::CustodyScopeUnsupported
        );
        assert_aborted(&session, &before, 0);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_full_new_destination_failure_discards_partial_local_serialization() {
    let oracle = full_new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_profile(1_000_000, address(0xa1), Some(0x31));
    let session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let before = session.dpos_state.clone();
    let mut next = before.clone();
    let mut accounts = StagedDposAccountPort::new([]).unwrap();
    chain
        .apply_dpos_redelegate(
            &mut next,
            &mut accounts,
            address(0xd1),
            address(0x31),
            address(0x32),
            U256::from(1000).to_big_endian().to_vec(),
            1.into(),
        )
        .unwrap();
    let raw = full_new_raw(case);
    let original_rows = raw.rows.clone();
    let mut trace = FinalChainNativeRawTrace::new(&raw);
    session
        .serialize_undelegate_principal(address(0xd1), address(0x31), &before, &next, &mut trace)
        .unwrap();
    assert_eq!(
        trace
            .current(
                DPOS_CONTRACT_ADDRESS,
                iterable_count_key(&delegator_validators_prefix(address(0xd1)))
            )
            .unwrap(),
        ConcreteRead::Present(0_u32.to_le_bytes().to_vec())
    );
    // The former composition uses the original membership and fails after
    // source effects exist locally. No local trace is published on that error.
    assert!(matches!(
        session.serialize_delegate(address(0xd1), address(0x32), &before, &next, &mut trace),
        Err(FinalChainNativeSessionError::RawIntegrity(_))
    ));
    drop(trace);
    assert_eq!(session.dpos_state, before);
    assert_eq!(session.next_sequence, 0);
    assert_eq!(raw.rows, original_rows);
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
