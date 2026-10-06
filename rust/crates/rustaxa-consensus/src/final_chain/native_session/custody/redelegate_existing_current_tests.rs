//! Actual full/existing current nodes: source-copy restoration and local atomicity.
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
        [0x31, 0x32]
            .into_iter()
            .map(|validator| GenesisValidator {
                address: address(validator),
                vrf_key: [match validator {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: U256::from(2000).to_big_endian().to_vec(),
                delegations: vec![(address(0xa1), U256::from(1000).to_big_endian().to_vec())]
                    .into_iter()
                    .chain(Some((
                        address(0xd1),
                        U256::from(1000).to_big_endian().to_vec(),
                    )))
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
    let public:Value=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../../experiments/evm_feasibility/fixtures/native_redelegate_existing_current/public.json"))).unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_existing_current/local.json"
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
    completed(session.invoke(request, quote, raw).unwrap())
}

#[test]
fn redelegate_existing_current_kernel_and_cold_warm_ordered_parity() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let mut prefix_session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut prefix_raw = raw_view(&case["attempts"][0]["raw_before"]);
    let initial = prefix_session.dpos_state.clone();
    let prefix = invoke(&mut prefix_session, &request(&case, 0, 0), &prefix_raw);
    assert_eq!(
        write_json(&prefix.raw_mutations),
        case["attempts"][0]["ordered_raw_writes"]
    );
    prefix_raw.apply(&prefix.raw_mutations);
    assert_raw(&prefix_raw, &case["attempts"][0]["raw_after"]);
    let before = prefix_session.dpos_state.clone();
    assert_facts(&before, &case["attempts"][1]["facts_before"]);
    let key = NodeKey {
        validator: address(0x31),
        block: 1,
    };
    assert_eq!(
        before.reward_reference_graph.load_node(&key).unwrap().count,
        2
    );
    assert_eq!(
        before
            .reward_reference_graph
            .read_validator_head(&address(0x31))
            .unwrap(),
        1
    );
    assert_eq!(
        before
            .reward_reference_graph
            .read_cursor(&address(0x31), &address(0xd1))
            .unwrap(),
        1
    );
    assert!(
        chain
            .bounded_existing_destination_loaded_source_node(
                &before,
                address(0xd1),
                address(0x31),
                address(0x32),
                U256::from(700),
                1.into()
            )
            .unwrap()
            .is_some()
    );
    // Independently execute the real semantic prefix and target through the kernel.
    let mut kernel = initial;
    for (to, amount) in [(0x32, 300), (0x32, 700)] {
        let mut accounts = StagedDposAccountPort::new([]).unwrap();
        let result = chain
            .apply_dpos_redelegate(
                &mut kernel,
                &mut accounts,
                address(0xd1),
                address(0x31),
                address(to),
                U256::from(amount).to_big_endian().to_vec(),
                1.into(),
            )
            .unwrap();
        assert_eq!(result.status_code, 1);
        assert!(accounts.into_mutations().is_empty());
        if amount == 300 {
            assert_eq!(kernel, before);
        }
    }
    assert_eq!(
        kernel.reward_reference_graph.load_node(&key).unwrap().count,
        2
    );
    assert_eq!(
        kernel
            .reward_reference_graph
            .read_validator_head(&address(0x31))
            .unwrap(),
        1
    );
    assert!(matches!(
        kernel
            .reward_reference_graph
            .read_cursor(&address(0x31), &address(0xd1)),
        Err(DposRewardGraphError::MissingCursor { .. })
    ));
    assert_eq!(
        kernel
            .reward_reference_graph
            .load_node(&NodeKey {
                validator: address(0x32),
                block: 1
            })
            .unwrap()
            .count,
        3
    );
    assert_facts(&kernel, &case["facts_after"]);
    for cold in [true, false] {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut raw = if cold {
            raw_view(&case["attempts"][1]["raw_before"])
        } else {
            raw_view(&case["attempts"][0]["raw_before"])
        };
        if cold {
            session.dpos_state = before.clone();
        } else {
            let out = invoke(&mut session, &request(&case, 0, 0), &raw);
            raw.apply(&out.raw_mutations);
        }
        raw.reads.borrow_mut().clear();
        let seq = u64::from(!cold);
        let req = request(&case, 1, seq);
        let quote = session.prepare(&req, &raw).unwrap();
        assert_eq!(quote.required_gas, 80000.into());
        let outcome = completed(session.invoke(&req, quote, &raw).unwrap());
        assert_eq!(outcome.status, FinalChainNativeStatus::Success);
        assert_eq!(session.dpos_state, kernel);
        assert_eq!(
            write_json(&outcome.raw_mutations),
            case["attempts"][1]["ordered_raw_writes"]
        );
        assert_eq!(log_json(&outcome.logs), case["attempts"][1]["logs"]);
        assert!(outcome.output.is_empty() && outcome.account_mutations.is_empty());
        assert_eq!(outcome.raw_mutations.len(), 15);
        println!(
            "existing-current cold={cold}: {} Rust reads",
            raw.reads.borrow().len()
        );
        raw.apply(&outcome.raw_mutations);
        assert_raw(&raw, &case["attempts"][1]["raw_after"]);
        assert_raw(&raw, &case["final_raw"]);
        for (node_key, node) in session
            .dpos_state
            .reward_reference_graph
            .live_nodes()
            .unwrap()
        {
            if let Some(observed) =
                case["final_raw"].get(hex(&reward_node_key(node_key.validator, node_key.block).0))
            {
                assert_eq!(
                    encode_node(&node),
                    unhex(observed["value"].as_str().unwrap())
                );
            }
        }
        let after = session.dpos_state.clone();
        let failure = invoke(&mut session, &request(&case, 2, seq + 1), &raw);
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
        assert_eq!(session.dpos_state, after);
        assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
        assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    }
    drop(prefix_session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

/// Seed cold targets from the actual successful staged prefix; live targets keep
/// that session and its sequence. No desired node count is manually inserted.
fn setup<'a>(
    chain: &'a FinalChain,
    case: &Value,
    mode: u8,
) -> (FinalChainNativeSession<'a>, AuthenticatedRaw, usize, u64) {
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut raw = raw_view(&case["attempts"][0]["raw_before"]);
    if mode > 0 {
        let outcome = invoke(&mut session, &request(case, 0, 0), &raw);
        raw.apply(&outcome.raw_mutations);
        assert_raw(&raw, &case["attempts"][1]["raw_before"]);
        if mode == 1 {
            let before = session.dpos_state.clone();
            drop(session);
            session = chain.begin_native_session(1.into(), 0.into()).unwrap();
            session.dpos_state = before;
            raw = raw_view(&case["attempts"][1]["raw_before"]);
        }
    }
    raw.reads.borrow_mut().clear();
    (session, raw, usize::from(mode > 0), u64::from(mode == 2))
}

#[test]
fn redelegate_existing_current_every_prefix_cold_warm_auth_failure_is_atomic() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    for mode in 0..3 {
        let (mut probe, raw, attempt, seq) = setup(&chain, &case, mode);
        let out = invoke(&mut probe, &request(&case, attempt, seq), &raw);
        assert_eq!(out.status, FinalChainNativeStatus::Success);
        let keys = raw.reads.borrow().clone();
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            keys.len()
        );
        println!(
            "existing-current mode={mode}: {} unique Rust auth keys",
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
                );
            }
        }
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_existing_current_shared_predicate_excludes_unmeasured_authority() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let (session, _, _, _) = setup(&chain, &case, 2);
    let original = session.dpos_state.clone();
    let source = address(0x31);
    let dest = address(0x32);
    let caller = address(0xd1);
    for shape in 0..25 {
        let mut state = original.clone();
        let mut block = 1.into();
        match shape {
            0 => {
                state
                    .reward_reference_graph
                    .write_cursor(source, caller, 0)
                    .unwrap();
            }
            1 => {
                state
                    .reward_reference_graph
                    .overwrite_validator_head_stale(source, 1)
                    .unwrap();
            }
            2 => {
                let key = NodeKey {
                    validator: source,
                    block: 1,
                };
                let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                node.count = 3;
                state
                    .reward_reference_graph
                    .restore_loaded_node(key, node)
                    .unwrap();
            }
            3 => {
                let key = NodeKey {
                    validator: source,
                    block: 1,
                };
                let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                node.reward_per_stake = DposRewardIndex::from(BigUint::from(1u8));
                state
                    .reward_reference_graph
                    .restore_loaded_node(key, node)
                    .unwrap();
            }
            4 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&source)
                    .unwrap()
                    .remove(&caller);
            }
            5 => {
                state.validator_reward_per_stake.remove(&source);
            }
            6 => {
                state.delegator_rewards.insert(
                    source,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            7 => {
                state.commission_rewards.insert(
                    dest,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            8 => {
                let key = NodeKey {
                    validator: dest,
                    block: 1,
                };
                let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                node.count = 4;
                state
                    .reward_reference_graph
                    .restore_loaded_node(key, node)
                    .unwrap();
            }
            9 => {
                state
                    .reward_reference_graph
                    .write_cursor(dest, caller, 0)
                    .unwrap();
            }
            10 => {
                state
                    .delegation_reward_cursors
                    .entry(dest)
                    .or_default()
                    .insert(caller, vec![1].into());
            }
            11 => {
                state.delegator_validators.insert(caller, vec![source]);
            }
            12 => {
                state
                    .delegator_validators
                    .insert(caller, vec![dest, source]);
            }
            13 => {
                state
                    .delegator_validators
                    .insert(caller, vec![source, address(0x33), dest]);
            }
            14 => {
                state.redelegate_same_validator_corruption.insert(source);
            }
            15 => {
                block = 2.into();
            }
            16 => {
                state.delegation_ledger_history_complete = false;
            }
            17 => {
                state.redelegate_same_validator_history_complete = false;
            }
            18 => {
                state
                    .validator_reward_per_stake
                    .insert(dest, vec![1].into());
            }
            19 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&source)
                    .unwrap()
                    .insert(caller, vec![1].into());
            }
            20 => {
                state
                    .reward_reference_graph
                    .overwrite_validator_head_stale(dest, 1)
                    .unwrap();
            }
            21 => {
                state.delegations.entry(address(0x99)).or_default().insert(
                    caller,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            22 => {
                let key = NodeKey {
                    validator: dest,
                    block: 1,
                };
                let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                node.count = 3;
                state
                    .reward_reference_graph
                    .restore_loaded_node(key, node)
                    .unwrap();
            }
            23 => {
                let key = NodeKey {
                    validator: dest,
                    block: 1,
                };
                let mut node = state.reward_reference_graph.load_node(&key).unwrap();
                node.reward_per_stake = DposRewardIndex::from(BigUint::from(1u8));
                state
                    .reward_reference_graph
                    .restore_loaded_node(key, node)
                    .unwrap();
            }
            24 => {
                state.reward_reference_graph = DposRewardGraph::incomplete();
            }
            _ => unreachable!(),
        }
        let result = chain.bounded_existing_destination_loaded_source_node(
            &state,
            caller,
            source,
            dest,
            U256::from(700),
            block,
        );
        if [16, 17, 24].contains(&shape) {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_none(), "shape{shape}");
        }
    }
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_existing_current_late_serialization_failure_discards_local_restore() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let (session, mut raw, _, seq) = setup(&chain, &case, 2);
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
            U256::from(700).to_big_endian().to_vec(),
            1.into(),
        )
        .unwrap();
    raw.rows
        .insert(delegation_key(address(0x32), address(0xd1)), vec![0xff]);
    let backing = raw.rows.clone();
    let mut trace = FinalChainNativeRawTrace::new(&raw);
    session
        .serialize_undelegate_principal(address(0xd1), address(0x31), &before, &next, &mut trace)
        .unwrap();
    assert_eq!(
        trace
            .current(DPOS_CONTRACT_ADDRESS, reward_node_key(address(0x31), 1))
            .unwrap(),
        ConcreteRead::Present(vec![0xc2, 0x80, 2])
    );
    assert!(matches!(
        session.serialize_delegate(address(0xd1), address(0x32), &before, &next, &mut trace),
        Err(FinalChainNativeSessionError::RawIntegrity(_))
    ));
    drop(trace);
    assert_eq!(session.dpos_state, before);
    assert_eq!(session.next_sequence, seq);
    assert_eq!(raw.rows, backing);
    assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
    assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
