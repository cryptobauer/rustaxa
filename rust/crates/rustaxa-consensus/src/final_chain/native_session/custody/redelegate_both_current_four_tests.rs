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
                        address(0xaa),
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
    for owner in [address(0xaa), address(0xa1)] {
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
    let public:Value=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_four/public.json"))).unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_four/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    public["cases"][0].clone()
}
fn request(case: &Value, attempt: usize, sequence: u64) -> FinalChainNativeRequest {
    let mut request = staged_request(case, sequence);
    request.input = unhex(case["attempts"][attempt]["input"].as_str().unwrap());
    request.caller = unhex(case["attempts"][attempt]["caller"].as_str().unwrap())
        .try_into()
        .unwrap();
    request
}
fn raw_view(view: &Value, case: &Value) -> AuthenticatedRaw {
    let mut raw = AuthenticatedRaw::from_attempt(&json!({"ordered_reads":[]}));
    for (key, row) in view.as_object().unwrap() {
        if row["present"] == true {
            raw.rows.insert(
                ConcreteStorageKey(unhex(key).try_into().unwrap()),
                unhex(row["value"].as_str().unwrap()),
            );
        }
    }
    // Complete Go rows retain actual immutable genesis0 owner versions. These
    // rows were never changed by any real prefix/target; do not guess their bytes.
    for validator in [address(0x31), address(0x32)] {
        let key = validator_owner_key(validator);
        assert!(case["attempts"].as_array().unwrap().iter().all(|a| {
            a["ordered_raw_writes"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|w| w["key"] != hex(&key.0))
        }));
        let prefix = rustaxa_storage::storage_version_prefix(DPOS_CONTRACT_ADDRESS, key);
        let physical = rustaxa_storage::versioned_key(prefix, 0.into());
        let row = case["committed_rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["column"] == 4 && row["key"] == hex(&physical))
            .unwrap();
        raw.rows.insert(key, unhex(row["value"].as_str().unwrap()));
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

/// Cold state is derived only from real successful prefixes, not edited counts.
fn setup<'a>(
    chain: &'a FinalChain,
    case: &Value,
    attempt: usize,
    warm: bool,
) -> (FinalChainNativeSession<'a>, AuthenticatedRaw, u64) {
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut raw = raw_view(&case["attempts"][0]["raw_before"], case);
    for prefix in 0..attempt {
        let out = invoke(&mut session, &request(case, prefix, prefix as u64), &raw);
        assert_eq!(
            write_json(&out.raw_mutations),
            case["attempts"][prefix]["ordered_raw_writes"]
        );
        assert_eq!(log_json(&out.logs), case["attempts"][prefix]["logs"]);
        raw.apply(&out.raw_mutations);
        assert_raw(&raw, &case["attempts"][prefix]["raw_after"]);
        assert_facts(
            &session.dpos_state,
            &case["attempts"][prefix]["facts_after"],
        );
    }
    let seq = if warm { attempt as u64 } else { 0 };
    if !warm {
        let state = session.dpos_state.clone();
        drop(session);
        session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        session.dpos_state = state;
        raw = raw_view(&case["attempts"][attempt]["raw_before"], case);
    }
    raw.reads.borrow_mut().clear();
    (session, raw, seq)
}
#[test]
fn redelegate_both_current_four_kernel_and_cold_warm_ordered_parity() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let mut kernel = chain
        .begin_native_session(1.into(), 0.into())
        .unwrap()
        .dpos_state
        .clone();
    for attempt in 0..4 {
        let req = request(&case, attempt, attempt as u64);
        let amount = if attempt == 0 {
            300
        } else if attempt == 1 || attempt == 2 {
            100
        } else {
            700
        };
        let mut accounts = StagedDposAccountPort::new([]).unwrap();
        let result = chain
            .apply_dpos_redelegate(
                &mut kernel,
                &mut accounts,
                req.caller,
                address(0x31),
                address(0x32),
                U256::from(amount).to_big_endian().to_vec(),
                1.into(),
            )
            .unwrap();
        assert_eq!(result.status_code, 1);
        assert!(accounts.into_mutations().is_empty());
        assert_facts(&kernel, &case["attempts"][attempt]["facts_after"]);
    }
    for warm in [false, true] {
        let (mut session, mut raw, seq) = setup(&chain, &case, 3, warm);
        let before = session.dpos_state.clone();
        assert!(
            chain
                .bounded_both_current_four_loaded_source_node(
                    &before,
                    address(0xaa),
                    address(0x31),
                    address(0x32),
                    U256::from(700),
                    1.into()
                )
                .unwrap()
                .is_some()
        );
        assert!(
            chain
                .bounded_existing_destination_loaded_source_node(
                    &before,
                    address(0xaa),
                    address(0x31),
                    address(0x32),
                    U256::from(700),
                    1.into()
                )
                .unwrap()
                .is_none()
        );
        assert!(
            chain
                .bounded_both_current_three_loaded_source_node(
                    &before,
                    address(0xaa),
                    address(0x31),
                    address(0x32),
                    U256::from(700),
                    1.into(),
                )
                .unwrap()
                .is_none()
        );
        for validator in [address(0x31), address(0x32)] {
            assert_eq!(
                before
                    .reward_reference_graph
                    .load_node(&NodeKey {
                        validator,
                        block: 1
                    })
                    .unwrap()
                    .count,
                4
            );
            assert!(matches!(
                before.reward_reference_graph.load_node(&NodeKey {
                    validator,
                    block: 0
                }),
                Err(DposRewardGraphError::MissingNode { .. })
            ));
        }
        let req = request(&case, 3, seq);
        let quote = session.prepare(&req, &raw).unwrap();
        assert_eq!(quote.required_gas, 80000.into());
        let out = completed(session.invoke(&req, quote, &raw).unwrap());
        assert_eq!(out.status, FinalChainNativeStatus::Success);
        assert_eq!(out.gas_used, 80000.into());
        assert_eq!(session.dpos_state, kernel);
        assert_eq!(
            write_json(&out.raw_mutations),
            case["attempts"][3]["ordered_raw_writes"]
        );
        assert_eq!(log_json(&out.logs), case["attempts"][3]["logs"]);
        assert_eq!(out.raw_mutations.len(), 15);
        assert!(out.output.is_empty() && out.account_mutations.is_empty());
        println!(
            "both-current-four target warm={warm}: {} authentication keys",
            raw.reads.borrow().len()
        );
        raw.apply(&out.raw_mutations);
        assert_raw(&raw, &case["attempts"][3]["raw_after"]);
        assert_raw(&raw, &case["final_raw"]);
        for (validator, stake, count) in [(address(0x31), 800, 4), (address(0x32), 3200, 5)] {
            assert_eq!(
                session.dpos_state.total_stakes[&validator].as_u256(),
                U256::from(stake)
            );
            assert_eq!(
                session
                    .dpos_state
                    .reward_reference_graph
                    .load_node(&NodeKey {
                        validator,
                        block: 1
                    })
                    .unwrap()
                    .count,
                count
            );
        }
        let after = session.dpos_state.clone();
        let backing = raw.rows.clone();
        let failure = invoke(&mut session, &request(&case, 4, seq + 1), &raw);
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
        assert_eq!(raw.rows, backing);
        assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
        assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn redelegate_both_current_four_every_prefix_target_cold_warm_fault_is_atomic() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let mut failures = 0;
    for attempt in 0..4 {
        for warm in [false, true] {
            let (mut probe, raw, seq) = setup(&chain, &case, attempt, warm);
            let outcome = invoke(&mut probe, &request(&case, attempt, seq), &raw);
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            let keys = raw.reads.borrow().clone();
            assert_eq!(
                keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
                keys.len()
            );
            println!(
                "both-current-four attempt={attempt} warm={warm}: {} authentication keys",
                keys.len()
            );
            for (index, key) in keys.iter().enumerate() {
                for read_error in [false, true] {
                    let (mut session, mut raw, seq) = setup(&chain, &case, attempt, warm);
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
                        "{attempt} {warm} {key:?} {error:?}"
                    );
                    assert_aborted(&session, &before, seq);
                    assert_eq!(raw.rows, backing);
                    assert!(session.prepare(&req, &raw).is_err());
                    assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
                    assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
                    let (mut retry, mut retry_raw, seq) = setup(&chain, &case, attempt, warm);
                    let out = invoke(&mut retry, &request(&case, attempt, seq), &retry_raw);
                    assert_eq!(out.status, FinalChainNativeStatus::Success);
                    assert_eq!(
                        write_json(&out.raw_mutations),
                        case["attempts"][attempt]["ordered_raw_writes"]
                    );
                    assert_eq!(log_json(&out.logs), case["attempts"][attempt]["logs"]);
                    retry_raw.apply(&out.raw_mutations);
                    assert_raw(&retry_raw, &case["attempts"][attempt]["raw_after"]);
                    assert_facts(&retry.dpos_state, &case["attempts"][attempt]["facts_after"]);
                    failures += 1;
                }
            }
        }
    }
    println!("both-current-four {failures} faults and fresh retries");
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

/// Missing rows and independently valid wrong bytes must fail before effects.
#[test]
fn redelegate_both_current_four_valid_physical_faults_are_atomic() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let owner = address(0xa1);
    let mut failures = 0;
    for warm in [false, true] {
        let (mut probe, raw, seq) = setup(&chain, &case, 3, warm);
        invoke(&mut probe, &request(&case, 3, seq), &raw);
        let mut variants: Vec<(ConcreteStorageKey, Option<Vec<u8>>)> = raw
            .reads
            .borrow()
            .iter()
            .filter(|key| {
                raw.rows.get(key).is_some_and(|value| !value.is_empty())
                    && ![rewards_key(address(0x31)), rewards_key(address(0x32))].contains(key)
            })
            .map(|key| (*key, None))
            .collect();
        for (validator, stake) in [(address(0x31), 800_u64), (address(0x32), 1200)] {
            for (wrong_stake, wrong_cursor) in [(stake + 1, 1_u64), (stake, 0)] {
                let mut encoded = rlp::RlpStream::new_list(2);
                encoded.append(&U256::from(wrong_stake));
                encoded.append(&wrong_cursor);
                variants.push((
                    delegation_key(validator, owner),
                    Some(encoded.out().to_vec()),
                ));
            }
            variants.push((validator_owner_key(validator), Some(address(0xaa).to_vec())));
            // These are canonical RLP nodes, not malformed byte strings.
            variants.push((reward_node_key(validator, 0), Some(vec![0xc2, 0x80, 1])));
            variants.push((reward_node_key(validator, 1), Some(vec![0xc2, 0x80, 5])));
            variants.push((reward_node_key(validator, 1), Some(vec![0xc2, 1, 4])));
        }
        for prefix in [vec![0, 5], delegator_validators_prefix(owner)] {
            variants.push((
                iterable_count_key(&prefix),
                Some(3_u32.to_le_bytes().to_vec()),
            ));
            for (position, validator, wrong) in [
                (1, address(0x31), address(0x32)),
                (2, address(0x32), address(0x31)),
            ] {
                variants.push((iterable_item_key(&prefix, position), Some(wrong.to_vec())));
                variants.push((
                    iterable_position_key(&prefix, &validator),
                    Some((3_u32 - position).to_le_bytes().to_vec()),
                ));
            }
        }
        for (key, value) in variants {
            let (mut session, mut raw, seq) = setup(&chain, &case, 3, warm);
            let before = session.dpos_state.clone();
            match value {
                Some(value) => {
                    assert_ne!(raw.rows.get(&key), Some(&value));
                    raw.rows.insert(key, value);
                }
                None => {
                    assert!(raw.rows.remove(&key).is_some());
                }
            }
            let backing = raw.rows.clone();
            let req = request(&case, 3, seq);
            let quote = session.prepare(&req, &raw).unwrap();
            let result = session.invoke(&req, quote, &raw);
            assert!(result.is_err(), "accepted physical fault {key:?}");
            let error = result.unwrap_err();
            assert!(
                matches!(error, FinalChainNativeSessionError::RawIntegrity(_)),
                "{warm} {key:?} {error:?}"
            );
            assert_aborted(&session, &before, seq);
            assert_eq!(raw.rows, backing);
            assert!(session.prepare(&req, &raw).is_err());
            assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
            assert_eq!(chain.last_block_number_typed().unwrap(), 0.into());
            let (mut retry, mut retry_raw, seq) = setup(&chain, &case, 3, warm);
            let out = invoke(&mut retry, &request(&case, 3, seq), &retry_raw);
            assert_eq!(out.status, FinalChainNativeStatus::Success);
            assert_eq!(
                write_json(&out.raw_mutations),
                case["attempts"][3]["ordered_raw_writes"]
            );
            assert_eq!(log_json(&out.logs), case["attempts"][3]["logs"]);
            retry_raw.apply(&out.raw_mutations);
            assert_raw(&retry_raw, &case["attempts"][3]["raw_after"]);
            assert_facts(&retry.dpos_state, &case["attempts"][3]["facts_after"]);
            assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
            failures += 1;
        }
    }
    println!("both-current-four {failures} valid physical faults and fresh retries");
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_both_current_four_exact_predicate_excludes_other_authority() {
    let case = oracle_case();
    let (mut chain, storage, path) = swap_append_chain();
    let (session, _, _) = setup(&chain, &case, 3, true);
    let original = session.dpos_state.clone();
    drop(session);
    let source = address(0x31);
    let dest = address(0x32);
    let caller = address(0xaa);
    let owner = address(0xa1);
    for shape in 0..42 {
        let mut state = original.clone();
        let mut block = 1.into();
        let change_node = |state: &mut DposSnapshot, validator, count, index| {
            let key = NodeKey {
                validator,
                block: 1,
            };
            let mut node = state.reward_reference_graph.load_node(&key).unwrap();
            node.count = count;
            node.reward_per_stake = if index {
                BigUint::from(1u8).into()
            } else {
                DposRewardIndex::zero()
            };
            state
                .reward_reference_graph
                .restore_loaded_node(key, node)
                .unwrap();
        };
        match shape {
            0 => change_node(&mut state, source, 5, false),
            1 => change_node(&mut state, dest, 5, false),
            2 => {
                state
                    .reward_reference_graph
                    .delete_cursor(&dest, &owner)
                    .unwrap();
                change_node(&mut state, dest, 3, false);
            }
            3 => {
                state
                    .reward_reference_graph
                    .bootstrap_node(
                        NodeKey {
                            validator: source,
                            block: 0,
                        },
                        Node {
                            count: 1,
                            reward_per_stake: DposRewardIndex::zero(),
                        },
                        false,
                        &[owner],
                    )
                    .unwrap();
                change_node(&mut state, source, 4, false);
            }
            4 => {
                state
                    .reward_reference_graph
                    .delete_cursor(&source, &owner)
                    .unwrap();
                change_node(&mut state, source, 4, false);
            }
            5 => state.validator_metadata.get_mut(&dest).unwrap().owner = address(0x99),
            6 => state.validator_metadata.get_mut(&source).unwrap().owner = caller,
            7 => {
                state.delegations.get_mut(&source).unwrap().insert(
                    address(0x99),
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            8 => {
                state.total_stakes.insert(
                    address(0x33),
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            9 => {
                state
                    .delegator_validators
                    .insert(caller, vec![dest, source]);
            }
            10 => {
                state.delegator_validators.insert(owner, vec![dest, source]);
            }
            11 => {
                state
                    .delegator_validators
                    .insert(owner, vec![source, dest, address(0x33)]);
            }
            12 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&source)
                    .unwrap()
                    .remove(&owner);
            }
            13 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&dest)
                    .unwrap()
                    .remove(&caller);
            }
            14 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&source)
                    .unwrap()
                    .insert(owner, vec![1].into());
            }
            15 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&dest)
                    .unwrap()
                    .insert(caller, vec![1].into());
            }
            16 => {
                state
                    .validator_reward_per_stake
                    .insert(source, vec![1].into());
            }
            17 => {
                state.validator_reward_per_stake.remove(&dest);
            }
            18 => change_node(&mut state, source, 4, true),
            19 => change_node(&mut state, dest, 4, true),
            20 => {
                state.delegator_rewards.insert(
                    source,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            21 => {
                state.commission_rewards.insert(
                    dest,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            22 => {
                state
                    .reward_reference_graph
                    .overwrite_validator_head_stale(source, 1)
                    .unwrap();
            }
            23 => {
                state
                    .reward_reference_graph
                    .overwrite_validator_head_stale(dest, 1)
                    .unwrap();
            }
            24 => block = 2.into(),
            25 => state.delegation_ledger_history_complete = false,
            26 => state.redelegate_same_validator_history_complete = false,
            27 => state.reward_reference_graph = DposRewardGraph::incomplete(),
            28 => {
                state.total_stakes.insert(
                    source,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1601)),
                );
            }
            29 => {
                state.redelegate_same_validator_corruption.insert(source);
            }
            30 => {
                state
                    .redelegate_same_validator_corruption
                    .insert(address(0x99));
            }
            31 => {
                state
                    .delegation_reward_cursors
                    .get_mut(&source)
                    .unwrap()
                    .insert(address(0x99), vec![].into());
            }
            32 => {
                state.validator_metadata.remove(&source);
            }
            33 => {
                state.vrf_keys.remove(&dest);
            }
            34 => {
                state
                    .reward_reference_graph
                    .bootstrap_node(
                        NodeKey {
                            validator: dest,
                            block: 0,
                        },
                        Node {
                            count: 1,
                            reward_per_stake: DposRewardIndex::zero(),
                        },
                        false,
                        &[],
                    )
                    .unwrap();
            }
            35 => {
                for account in [caller, owner] {
                    state
                        .reward_reference_graph
                        .delete_cursor(&source, &account)
                        .unwrap();
                }
                state
                    .reward_reference_graph
                    .delete_validator_head(&source)
                    .unwrap();
            }
            36 => state.validator_order.reverse(),
            37 => {
                state.delegator_validators.insert(address(0x99), vec![]);
            }
            38 => {
                state
                    .delegation_reward_cursors
                    .insert(address(0x33), Default::default());
            }
            39 => {
                state.commission_rewards.insert(
                    address(0x33),
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
                );
            }
            40 | 41 => {
                // Independently valid unequal3/4 and4/3 graphs, with the same
                // heads/cursors. Do not use restore_loaded_node to lower count.
                let mut graph = DposRewardGraph::new();
                graph.next_block(1).unwrap();
                for validator in [source, dest] {
                    let lowered =
                        (shape == 40 && validator == source) || (shape == 41 && validator == dest);
                    graph
                        .bootstrap_node(
                            NodeKey {
                                validator,
                                block: 1,
                            },
                            Node {
                                count: if lowered { 3 } else { 4 },
                                reward_per_stake: DposRewardIndex::zero(),
                            },
                            true,
                            &[caller, owner],
                        )
                        .unwrap();
                }
                state.reward_reference_graph = graph;
            }
            _ => unreachable!(),
        }
        let result = chain.bounded_both_current_four_loaded_source_node(
            &state,
            caller,
            source,
            dest,
            U256::from(700),
            block,
        );
        if [4, 25, 26, 27, 28].contains(&shape) {
            assert!(result.is_err(), "shape{shape}");
        } else {
            assert!(result.unwrap().is_none(), "shape{shape}");
        }
    }
    let mut zero = original.clone();
    zero.delegator_rewards.insert(
        source,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
    );
    zero.commission_rewards.insert(
        dest,
        StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
    );
    assert!(
        chain
            .bounded_both_current_four_loaded_source_node(
                &zero,
                caller,
                source,
                dest,
                U256::from(700),
                1.into()
            )
            .unwrap()
            .is_some()
    );
    let saved = chain.rewards_config.clone();
    let ficus = chain.ficus_activation_period;
    for fork in 0..4 {
        chain.rewards_config = saved.clone();
        chain.ficus_activation_period = ficus;
        match fork {
            0 => chain.rewards_config.fix_redelegate_block_num = 1.into(),
            1 => chain.rewards_config.magnolia_period = FinalChainBlockNumber::MAX,
            2 => chain.ficus_activation_period = FinalChainBlockNumber::MAX,
            3 => chain.rewards_config.aspen_part_two_period = 0.into(),
            _ => unreachable!(),
        }
        assert!(
            chain
                .bounded_both_current_four_loaded_source_node(
                    &original,
                    caller,
                    source,
                    dest,
                    U256::from(700),
                    1.into()
                )
                .unwrap()
                .is_none()
        );
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn redelegate_both_current_four_late_serialization_failure_discards_local_restore() {
    let case = oracle_case();
    let (chain, storage, path) = swap_append_chain();
    let committed = chain.dpos_snapshot(0.into()).unwrap();
    let (session, mut raw, seq) = setup(&chain, &case, 3, true);
    let before = session.dpos_state.clone();
    let mut next = before.clone();
    let mut accounts = StagedDposAccountPort::new([]).unwrap();
    chain
        .apply_dpos_redelegate(
            &mut next,
            &mut accounts,
            address(0xaa),
            address(0x31),
            address(0x32),
            U256::from(700).to_big_endian().to_vec(),
            1.into(),
        )
        .unwrap();
    raw.rows
        .insert(delegation_key(address(0x32), address(0xaa)), vec![0xff]);
    let backing = raw.rows.clone();
    let mut trace = FinalChainNativeRawTrace::new(&raw);
    session
        .serialize_undelegate_principal(address(0xaa), address(0x31), &before, &next, &mut trace)
        .unwrap();
    assert_eq!(
        trace
            .current(DPOS_CONTRACT_ADDRESS, reward_node_key(address(0x31), 1))
            .unwrap(),
        ConcreteRead::Present(vec![0xc2, 0x80, 4])
    );
    assert!(matches!(
        session.serialize_delegate(address(0xaa), address(0x32), &before, &next, &mut trace),
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
