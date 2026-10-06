//! Actual source-last genesis full removal: ordered parity and atomicity.
use super::*;
fn swap_append_chain() -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "redelegate-composition-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        [0x33, 0x31, 0x32]
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
                        (validator != 0x32)
                            .then_some((address(0xd1), U256::from(1000).to_big_endian().to_vec())),
                    )
                    .collect(),
                metadata: GenesisValidatorMetadata {
                    owner: address(0xa1),
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
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
        for validator in [address(0x31), address(0x32), address(0x33)] {
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

fn oracle_case() -> Value {
    let public:Value=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../../experiments/evm_feasibility/fixtures/native_redelegate_source_last_absent/public.json"))).unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_source_last_absent/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    public["cases"][0].clone()
}
fn request(case: &Value, attempt: usize, sequence: u64) -> FinalChainNativeRequest {
    let mut request = staged_request(case, sequence);
    request.input = unhex(case["attempts"][attempt]["input"].as_str().unwrap());
    request
}
fn raw_view(view: &Value) -> AuthenticatedRaw {
    let mut raw = AuthenticatedRaw::from_attempt(&json!({"ordered_reads":[]}));
    for (key, row) in view.as_object().unwrap() {
        if row["present"] == true {
            raw.rows.insert(
                ConcreteStorageKey(unhex(key).try_into().unwrap()),
                unhex(row["value"].as_str().unwrap()),
            );
        }
    }
    raw
}
fn assert_raw(raw: &AuthenticatedRaw, view: &Value) {
    for (key, row) in view.as_object().unwrap() {
        let key = ConcreteStorageKey(unhex(key).try_into().unwrap());
        let value = raw.rows.get(&key).filter(|value| !value.is_empty());
        assert_eq!(value.is_some(), row["present"] == true, "{key:?}");
        assert_eq!(
            value.cloned().unwrap_or_default(),
            unhex(row["value"].as_str().unwrap())
        );
    }
}
fn invoke(
    session: &mut FinalChainNativeSession<'_>,
    request: &FinalChainNativeRequest,
    raw: &AuthenticatedRaw,
) -> FinalChainNativeOutcome {
    let quote = session.prepare(request, raw).unwrap();
    assert_eq!(quote.required_gas.as_u64(), 80000);
    let outcome = completed(session.invoke(request, quote, raw).unwrap());
    assert_eq!(outcome.gas_used.as_u64(), 80000);
    outcome
}

fn setup<'a>(
    chain: &'a FinalChain,
    case: &Value,
    mode: u8,
) -> (FinalChainNativeSession<'a>, AuthenticatedRaw, usize, u64) {
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut raw = raw_view(&case["attempts"][0]["raw_before"]);
    if mode == 1 {
        let result = invoke(&mut session, &request(case, 0, 0), &raw);
        raw.apply(&result.raw_mutations);
    }
    raw.reads.borrow_mut().clear();
    (session, raw, mode as usize, mode as u64)
}

#[test]
fn redelegate_source_last_absent_predicate_excludes_other_shapes() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let (mut session, _, _, _) = setup(&chain, &case, 0);
    let original = session.dpos_state.clone();
    let caller = address(0xd1);
    let source = address(0x31);
    let dest = address(0x32);
    let retained = address(0x33);
    // Mutate every node/count/index/pool/mirror/head bound separately.
    for validator in [source, dest, retained] {
        for shape in 0..10 {
            session.dpos_state = original.clone();
            let state = &mut session.dpos_state;
            match shape {
                0 | 1 => {
                    let key = NodeKey {
                        validator,
                        block: 0,
                    };
                    let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                    if shape == 0 {
                        node.count += 1;
                    } else {
                        node.reward_per_stake = DposRewardIndex::from(BigUint::from(1u8));
                    }
                    state
                        .reward_reference_graph
                        .restore_loaded_node(key, node)
                        .unwrap();
                }
                2 => {
                    let node = Node {
                        count: 1,
                        reward_per_stake: DposRewardIndex::zero(),
                    };
                    state
                        .reward_reference_graph
                        .bootstrap_node(
                            NodeKey {
                                validator,
                                block: 1,
                            },
                            node,
                            false,
                            &[],
                        )
                        .unwrap();
                }
                3 => {
                    state
                        .reward_reference_graph
                        .overwrite_validator_head_stale(validator, 0)
                        .unwrap();
                }
                4 => {
                    state
                        .delegation_reward_cursors
                        .entry(validator)
                        .or_default()
                        .insert(caller, vec![1].into());
                }
                5 => {
                    state
                        .validator_reward_per_stake
                        .insert(validator, vec![1].into());
                }
                6 => {
                    state.delegator_rewards.insert(
                        validator,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                    );
                }
                7 => {
                    state.commission_rewards.insert(
                        validator,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                    );
                }
                8 => {
                    state.redelegate_same_validator_corruption.insert(validator);
                }
                9 => {
                    if validator == dest {
                        state
                            .delegation_reward_cursors
                            .entry(dest)
                            .or_default()
                            .insert(caller, vec![].into());
                    } else {
                        state
                            .delegation_reward_cursors
                            .get_mut(&validator)
                            .unwrap()
                            .remove(&caller);
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                !session
                    .bounded_source_last_absent(caller, source, dest, U256::from(1000))
                    .unwrap(),
                "validator={validator:?} shape={shape}"
            );
        }
    }
    for shape in 0..16 {
        session.dpos_state = original.clone();
        session.pending_period = 1.into();
        let state = &mut session.dpos_state;
        match shape {
            0 => {
                state
                    .delegator_validators
                    .insert(caller, vec![source, retained]);
            }
            1 => {
                state
                    .delegator_validators
                    .insert(caller, vec![retained, source, dest]);
            }
            2 => {
                state
                    .delegator_validators
                    .insert(caller, vec![source, source]);
            }
            3 => {
                state.delegator_validators.remove(&caller);
            }
            4 => {
                state
                    .delegations
                    .get_mut(&retained)
                    .unwrap()
                    .remove(&caller);
            }
            5 => {
                state.delegations.get_mut(&retained).unwrap().insert(
                    caller,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
                );
            }
            6 => {
                state.delegations.entry(dest).or_default().insert(
                    caller,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            7 => {
                session.pending_period = 2.into();
            }
            8 => {
                state.delegation_ledger_history_complete = false;
            }
            9 => {
                state.redelegate_same_validator_history_complete = false;
            }
            10 => {
                state.reward_reference_graph = DposRewardGraph::incomplete();
            }
            11 => {
                state.total_stakes.insert(
                    source,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1000)),
                );
            }
            12 => {
                state
                    .reward_reference_graph
                    .bootstrap_node(
                        NodeKey {
                            validator: source,
                            block: 2,
                        },
                        Node {
                            count: 1,
                            reward_per_stake: DposRewardIndex::zero(),
                        },
                        false,
                        &[],
                    )
                    .unwrap();
                state
                    .reward_reference_graph
                    .write_cursor(source, caller, 2)
                    .unwrap();
            }
            13 => {
                state
                    .reward_reference_graph
                    .write_cursor(dest, caller, 0)
                    .unwrap();
            }
            14 => {
                state
                    .reward_reference_graph
                    .bootstrap_node(
                        NodeKey {
                            validator: retained,
                            block: 2,
                        },
                        Node {
                            count: 1,
                            reward_per_stake: DposRewardIndex::zero(),
                        },
                        false,
                        &[],
                    )
                    .unwrap();
                state
                    .reward_reference_graph
                    .write_cursor(retained, caller, 2)
                    .unwrap();
            }
            15 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&retained)
                    .unwrap()
                    .insert(caller, vec![1].into());
            }
            _ => unreachable!(),
        }
        let result = session.bounded_source_last_absent(caller, source, dest, U256::from(1000));
        if [8, 9, 10].contains(&shape) {
            assert!(result.is_err());
        } else if [12, 14].contains(&shape) {
            assert!(result.is_err() || !result.unwrap());
        } else {
            assert!(!result.unwrap(), "shape={shape}");
        }
    }
    session.dpos_state = original;
    session.pending_period = 1.into();
    for amount in [0, 999, 1001] {
        assert!(
            !session
                .bounded_source_last_absent(caller, source, dest, U256::from(amount))
                .unwrap()
        );
    }
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_source_last_absent_kernel_and_cold_warm_ordered_parity() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let (mut session, mut raw, _, _) = setup(&chain, &case, 0);
    let before = session.dpos_state.clone();
    assert!(before.validator_reward_per_stake.is_empty());
    assert_facts(&before, &case["facts_before"]);
    assert!(
        session
            .bounded_source_last_absent(
                address(0xd1),
                address(0x31),
                address(0x32),
                U256::from(1000)
            )
            .unwrap()
    );
    let mut kernel = before.clone();
    let mut accounts = StagedDposAccountPort::new([]).unwrap();
    let result = chain
        .apply_dpos_redelegate(
            &mut kernel,
            &mut accounts,
            address(0xd1),
            address(0x31),
            address(0x32),
            U256::from(1000).to_big_endian().to_vec(),
            1.into(),
        )
        .unwrap();
    assert_eq!(result.status_code, 1);
    assert!(accounts.into_mutations().is_empty());
    let outcome = invoke(&mut session, &request(&case, 0, 0), &raw);
    assert_eq!(outcome.status, FinalChainNativeStatus::Success);
    assert_eq!(session.dpos_state, kernel);
    assert_eq!(
        write_json(&outcome.raw_mutations),
        case["attempts"][0]["ordered_raw_writes"]
    );
    assert_eq!(log_json(&outcome.logs), case["attempts"][0]["logs"]);
    assert_eq!(outcome.raw_mutations.len(), 17);
    assert!(outcome.output.is_empty() && outcome.account_mutations.is_empty());
    for key in case["preserved_target_keys"].as_array().unwrap() {
        let key = ConcreteStorageKey(unhex(key.as_str().unwrap()).try_into().unwrap());
        assert!(outcome.raw_mutations.iter().all(|write| write.key != key));
    }
    raw.apply(&outcome.raw_mutations);
    assert_raw(&raw, &case["attempts"][0]["raw_after"]);
    assert_raw(&raw, &case["final_raw"]);
    assert_facts(&kernel, &case["facts_after"]);
    for (key, node) in kernel.reward_reference_graph.live_nodes().unwrap() {
        if let Some(observed) =
            case["final_raw"].get(hex(&reward_node_key(key.validator, key.block).0))
        {
            assert_eq!(
                encode_node(&node),
                unhex(observed["value"].as_str().unwrap())
            );
        }
    }
    let failure = invoke(&mut session, &request(&case, 1, 1), &raw);
    assert_eq!(
        failure.status,
        FinalChainNativeStatus::ContractFailure {
            error: "Delegation does not exist".into()
        }
    );
    assert!(
        failure.raw_mutations.is_empty()
            && failure.logs.is_empty()
            && failure.account_mutations.is_empty()
    );
    assert_eq!(session.dpos_state, kernel);
    assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
    assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_source_last_absent_explicit_zero_indices_match_genesis_omission() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let (mut session, raw, _, _) = setup(&chain, &case, 0);
    assert!(session.dpos_state.validator_reward_per_stake.is_empty());
    for validator in [address(0x31), address(0x32), address(0x33)] {
        session
            .dpos_state
            .validator_reward_per_stake
            .insert(validator, Vec::<u8>::new().into());
    }
    let outcome = invoke(&mut session, &request(&case, 0, 0), &raw);
    assert_eq!(outcome.status, FinalChainNativeStatus::Success);
    assert_eq!(
        write_json(&outcome.raw_mutations),
        case["attempts"][0]["ordered_raw_writes"]
    );
    assert_eq!(log_json(&outcome.logs), case["attempts"][0]["logs"]);
    assert_facts(&session.dpos_state, &case["facts_after"]);
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_source_last_absent_wrong_intermediate_is_atomic() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let (session, raw, _, _) = setup(&chain, &case, 0);
    let before = session.dpos_state.clone();
    let original = raw.rows.clone();
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
    let mut trace = FinalChainNativeRawTrace::new(&raw);
    session
        .serialize_undelegate_principal(address(0xd1), address(0x31), &before, &next, &mut trace)
        .unwrap();
    assert!(matches!(
        session.serialize_delegate(address(0xd1), address(0x32), &before, &next, &mut trace),
        Err(FinalChainNativeSessionError::RawIntegrity(_))
    ));
    drop(trace);
    assert_eq!(session.dpos_state, before);
    assert_eq!(session.next_sequence, 0);
    assert_eq!(raw.rows, original);
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_source_last_absent_cold_warm_auth_failure_is_atomic() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    for mode in 0..2 {
        let (mut probe, raw, attempt, seq) = setup(&chain, &case, mode);
        let out = invoke(&mut probe, &request(&case, attempt, seq), &raw);
        assert_eq!(
            out.status,
            if mode == 0 {
                FinalChainNativeStatus::Success
            } else {
                FinalChainNativeStatus::ContractFailure {
                    error: "Delegation does not exist".into(),
                }
            }
        );
        let keys = raw.reads.borrow().clone();
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            keys.len()
        );
        println!(
            "current-source mode={mode}: {} unique Rust auth keys",
            keys.len()
        );
        for (index, key) in keys.iter().enumerate() {
            for read_error in [false, true] {
                let (mut session, mut raw, attempt, seq) = setup(&chain, &case, mode);
                let before = session.dpos_state.clone();
                if read_error {
                    raw.fail_at = Some(index + 1);
                } else {
                    raw.rows.insert(*key, vec![0xff]);
                }
                let backing = raw.rows.clone();
                let req = request(&case, attempt, seq);
                let quote = session.prepare(&req, &raw).unwrap();
                let error = session.invoke(&req, quote, &raw).unwrap_err();
                assert!(
                    if read_error {
                        matches!(error, FinalChainNativeSessionError::StateRead(_))
                    } else {
                        matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                    },
                    "mode={mode} {key:?} {error:?}"
                );
                assert_aborted(&session, &before, seq);
                assert_eq!(raw.rows, backing);
                assert!(session.prepare(&req, &raw).is_err());
                assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
                assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
                let (mut retry, retry_raw, attempt, seq) = setup(&chain, &case, mode);
                let result = invoke(&mut retry, &request(&case, attempt, seq), &retry_raw);
                assert_eq!(
                    write_json(&result.raw_mutations),
                    case["attempts"][attempt]["ordered_raw_writes"]
                        .as_array()
                        .cloned()
                        .map(Value::Array)
                        .unwrap_or_else(|| json!([]))
                );
            }
        }
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
