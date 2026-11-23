//! Actual source-first removal swaps the retained member, then inserts destination.
//! The complete synthetic native seed grants test-only absence authority. This
//! staged boundary does not adopt a concrete checkpoint or claim public API parity.
use super::*;

fn swap_append_chain() -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    let path = crate::test_paths::new_path("redelegate-composition");
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        crate::FinalChainGenesisAllocation {
            accounts: Vec::new(),
            validators: [0x31, 0x32, 0x33]
                .into_iter()
                .map(|validator| GenesisValidator {
                    address: address(validator),
                    vrf_key: [match validator {
                        0x31 => 0x44,
                        0x32 => 0x55,
                        _ => 0x66,
                    }; 32],
                    total_stake: U256::from(if validator != 0x32 { 2000 } else { 1000 })
                        .to_big_endian()
                        .to_vec(),
                    delegations: vec![(address(0xa1), U256::from(1000).to_big_endian().to_vec())]
                        .into_iter()
                        .chain(
                            (validator != 0x32).then_some((
                                address(0xd1),
                                U256::from(1000).to_big_endian().to_vec(),
                            )),
                        )
                        .collect(),
                    metadata: GenesisValidatorMetadata {
                        owner: address(0xa1),
                        commission: 100,
                        ..Default::default()
                    },
                })
                .collect(),
        },
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::from(100).into(),
            delegation_delay: 1,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            aspen_part_one_period: 0.into(),
            aspen_max_supply: U256::from(8000).into(),
            dpos_blocks_per_year: 1,
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    (chain, storage, path)
}

fn swap_append_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/native_redelegate_swap_append/public.json"
    )))
    .unwrap();
    assert_eq!(public["cases"].as_array().unwrap().len(), 1);
    public
}

fn swap_append_raw(case: &Value) -> AuthenticatedRaw {
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
fn redelegate_swap_append_all_cold_and_warm_authentication_failures_are_atomic() {
    let oracle = swap_append_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    for warm in [false, true] {
        let mut success = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut reference = swap_append_raw(case);
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
        assert_eq!(keys.len(), if warm { 5 } else { 19 });
        if !warm {
            let prefix = delegator_validators_prefix(address(0xd1));
            assert_eq!(
                &keys[16..],
                &[
                    iterable_position_key(&prefix, &address(0x33)),
                    iterable_item_key(&prefix, 1),
                    iterable_item_key(&prefix, 2),
                ]
            );
        }
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
                let mut raw = swap_append_raw(case);
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
                assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
                assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
            }
        }
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_swap_append_rejects_unobserved_or_incomplete_membership_shapes() {
    let oracle = swap_append_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    for shape in 0..10 {
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
            7 => {
                session
                    .dpos_state
                    .delegator_validators
                    .insert(address(0xd1), vec![address(0x33), address(0x31)]);
            }
            8 => {
                session.dpos_state.delegator_validators.insert(
                    address(0xd1),
                    vec![address(0x31), address(0x33), address(0x99)],
                );
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
            9 => {
                session
                    .dpos_state
                    .delegations
                    .get_mut(&address(0x33))
                    .unwrap()
                    .remove(&address(0xd1));
            }
            _ => unreachable!(),
        }
        let before = session.dpos_state.clone();
        let raw = swap_append_raw(case);
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        let error = session.invoke(&request, quote, &raw).unwrap_err();
        if shape == 7 {
            // Source-last genesis is now in scope, but these unchanged physical
            // source-first positions still fail exact raw authentication.
            assert!(matches!(
                error,
                FinalChainNativeSessionError::RawIntegrity(_)
            ));
        } else {
            assert_eq!(error, FinalChainNativeSessionError::CustodyScopeUnsupported);
        }
        assert_aborted(&session, &before, 0);
        assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
        assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_swap_append_destination_failure_discards_partial_local_serialization() {
    let oracle = swap_append_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = swap_append_chain();
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
    let raw = swap_append_raw(case);
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
        ConcreteRead::Present(1_u32.to_le_bytes().to_vec())
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
