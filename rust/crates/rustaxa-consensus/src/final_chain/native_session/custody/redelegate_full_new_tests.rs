//! Actual full source removal and new destination insertion for a one-member caller.
//! The complete synthetic native seed grants test-only absence authority. This
//! staged boundary does not adopt a concrete checkpoint or claim public API parity.
use super::*;

fn full_new_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/native_redelegate_full_new/public.json"
    )))
    .unwrap();
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

fn assert_aborted(session: &FinalChainNativeSession<'_>, before: &DposSnapshot, sequence: u64) {
    assert_eq!(&session.dpos_state, before);
    assert_eq!(session.next_sequence, sequence);
    assert!(session.aborted && session.prepared.is_none());
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
