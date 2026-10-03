//! New caller destination pairs use the existing kernel/serializer owners.
use super::*;

fn new_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../experiments/evm_feasibility/fixtures/native_redelegate_new_destination/public.json"))).unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../experiments/evm_feasibility/fixtures/native_redelegate_new_destination/local.json"))).unwrap();
    assert_eq!(public, local);
    public
}

fn assert_facts(snapshot: &DposSnapshot, facts: &Value) {
    for last in [0xd1, 0xa1] {
        let owner = address(last);
        let members = snapshot
            .delegator_validators
            .get(&owner)
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            json!(members.iter().map(|member| hex(member)).collect::<Vec<_>>()),
            facts["memberships"][hex(&owner)]
        );
        for last in [0x31, 0x32] {
            let validator = address(last);
            let row = &facts["delegations"][format!("{}/{}", hex(&owner), hex(&validator))];
            let actual = delegation(snapshot, validator, owner);
            if row.is_null() {
                assert!(actual.is_none());
            } else {
                assert_eq!(actual.unwrap().to_string(), row["stake"]);
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

#[test]
fn redelegate_new_destination_first_and_repeat_match_actual_go() {
    let oracle = new_oracle();
    for case in oracle["cases"].as_array().unwrap() {
        let (chain, storage, path) = kernel_chain_destination(1_000_000, address(0xa1));
        let committed = chain.dpos_snapshot(0.into()).unwrap();
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        assert_facts(&session.dpos_state, &case["facts_before"]);
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        for (sequence, attempt) in case["attempts"].as_array().unwrap().iter().enumerate() {
            raw.reads.borrow_mut().clear();
            let mut next = session.dpos_state.clone();
            let mut accounts = StagedDposAccountPort::new([]).unwrap();
            let input = unhex(case["input"].as_str().unwrap());
            let kernel = chain
                .apply_dpos_redelegate(
                    &mut next,
                    &mut accounts,
                    address(0xd1),
                    address(0x31),
                    address(0x32),
                    input[68..100].to_vec(),
                    1.into(),
                )
                .unwrap();
            assert_eq!(kernel.status_code, 1);
            assert!(accounts.into_mutations().is_empty());
            let request = staged_request(case, sequence as u64);
            let quote = session.prepare(&request, &raw).unwrap();
            let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
            assert_eq!(outcome.status, FinalChainNativeStatus::Success);
            assert_eq!(session.dpos_state, next);
            assert_eq!(
                write_json(&outcome.raw_mutations),
                attempt["ordered_raw_writes"]
            );
            assert_eq!(log_json(&outcome.logs), attempt["logs"]);
            assert_eq!(outcome.output, unhex(attempt["output"].as_str().unwrap()));
            assert!(outcome.account_mutations.is_empty());
            // Rust reauthenticates each invocation; this is not Go cache parity.
            let reads = raw.reads.borrow();
            assert_eq!(reads.len(), if sequence == 0 { 16 } else { 12 });
            assert_eq!(
                reads
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                reads.len()
            );
            drop(reads);
            raw.apply(&outcome.raw_mutations);
            assert_eq!(chain.dpos_snapshot(0.into()).unwrap(), committed);
        }
        assert_facts(&session.dpos_state, &case["facts_after"]);
        assert_eq!(
            session.dpos_state.total_stakes[&address(0x31)]
                .as_u256()
                .to_string(),
            case["source_stake"]
        );
        assert_eq!(
            session.dpos_state.total_stakes[&address(0x32)]
                .as_u256()
                .to_string(),
            case["destination_stake"]
        );
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

fn assert_aborted(session: &FinalChainNativeSession<'_>, before: &DposSnapshot, sequence: u64) {
    assert_eq!(&session.dpos_state, before);
    assert!(session.aborted && session.prepared.is_none());
    assert_eq!(session.next_sequence, sequence);
}

#[test]
fn redelegate_new_destination_authenticates_absence_and_append_membership() {
    let oracle = new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_destination(1_000_000, address(0xa1));
    // Derive the complete actual Rust read set once; corrupt presence and absence
    // at every boundary, including serializer-only reads, without changing Go.
    let mut successful = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    let request = staged_request(case, 0);
    let quote = successful.prepare(&request, &raw).unwrap();
    successful.invoke(&request, quote, &raw).unwrap();
    let keys = raw.reads.borrow().clone();
    assert_eq!(keys.len(), 16);
    for key in keys {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        raw.rows.insert(key, vec![0xff]);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(
            matches!(
                session.invoke(&request, quote, &raw),
                Err(FinalChainNativeSessionError::RawIntegrity(_))
            ),
            "{key:?}"
        );
        assert_aborted(&session, &before, 0);
    }
    for corruption in 0..4 {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let order = session
            .dpos_state
            .delegator_validators
            .get_mut(&address(0xd1))
            .unwrap();
        match corruption {
            0 => order.push(address(0x31)),
            1 => order.clear(),
            2 => order.push(address(0x32)),
            3 => session.dpos_state.validator_order.push(address(0x32)),
            _ => unreachable!(),
        }
        let before = session.dpos_state.clone();
        let raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert_aborted(&session, &before, 0);
    }
    drop(successful);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_new_destination_reader_errors_abort_without_advancement() {
    let oracle = new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_destination(1_000_000, address(0xa1));
    for index in 1..=16 {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let before = session.dpos_state.clone();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        raw.fail_at = Some(index);
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(
            matches!(
                session.invoke(&request, quote, &raw),
                Err(FinalChainNativeSessionError::StateRead(_))
            ),
            "{index}"
        );
        assert_aborted(&session, &before, 0);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_new_destination_repeat_reauthenticates_without_duplicate_membership() {
    let oracle = new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_destination(1_000_000, address(0xa1));
    let mut first = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    let request = staged_request(case, 0);
    let quote = first.prepare(&request, &raw).unwrap();
    raw.apply(&completed(first.invoke(&request, quote, &raw).unwrap()).raw_mutations);
    let warm = first.dpos_state.clone();
    let mut successful = chain.begin_native_session(1.into(), 0.into()).unwrap();
    successful.dpos_state = warm.clone();
    successful.next_sequence = 1;
    raw.reads.borrow_mut().clear();
    let request = staged_request(case, 1);
    let quote = successful.prepare(&request, &raw).unwrap();
    successful.invoke(&request, quote, &raw).unwrap();
    let keys = raw.reads.borrow().clone();
    assert_eq!(keys.len(), 12);
    assert_eq!(
        successful.dpos_state.delegator_validators[&address(0xd1)],
        vec![address(0x31), address(0x32)]
    );
    for key in keys {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        session.dpos_state = warm.clone();
        session.next_sequence = 1;
        let before = session.dpos_state.clone();
        let bad = AuthenticatedRaw {
            rows: raw.rows.clone(),
            reads: RefCell::new(Vec::new()),
            fail_at: None,
        };
        let mut bad = bad;
        bad.rows.insert(key, vec![0xff]);
        let quote = session.prepare(&request, &bad).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &bad),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert_aborted(&session, &before, 1);
    }
    drop(successful);
    drop(first);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_new_destination_rejects_orphan_cursors_and_bad_reward_nodes() {
    let oracle = new_oracle();
    let case = &oracle["cases"][0];
    let (chain, storage, path) = kernel_chain_destination(1_000_000, address(0xa1));
    for corruption in 0..10 {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let validator = address(0x32);
        let delegator = address(0xd1);
        match corruption {
            0 => session
                .dpos_state
                .reward_reference_graph
                .write_or_create_cursor(validator, delegator, 0)
                .unwrap(),
            1 => {
                session
                    .dpos_state
                    .delegation_reward_cursors
                    .entry(validator)
                    .or_default()
                    .insert(delegator, vec![].into());
            }
            2 => session.dpos_state.reward_reference_graph = DposRewardGraph::incomplete(),
            3 | 7 => {
                session
                    .dpos_state
                    .reward_reference_graph
                    .bootstrap_node(
                        NodeKey {
                            validator,
                            block: 1,
                        },
                        Node {
                            reward_per_stake: if corruption == 3 {
                                DposRewardIndex::from_canonical_bytes(&[1], "new destination test")
                                    .unwrap()
                            } else {
                                DposRewardIndex::zero()
                            },
                            count: if corruption == 7 { u32::MAX } else { 1 },
                        },
                        true,
                        &[],
                    )
                    .unwrap();
            }
            4 => {
                session.dpos_state.delegator_rewards.insert(
                    validator,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            5 => {
                session.dpos_state.commission_rewards.insert(
                    validator,
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::from(1)),
                );
            }
            6 => {
                session
                    .dpos_state
                    .validator_reward_per_stake
                    .insert(validator, vec![1].into());
            }
            8 => {
                session
                    .dpos_state
                    .delegations
                    .get_mut(&validator)
                    .unwrap()
                    .insert(
                        delegator,
                        StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
                    );
            }
            9 => session.dpos_state.delegation_ledger_history_complete = false,
            _ => unreachable!(),
        }
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        if corruption != 2 {
            for (key, node) in session
                .dpos_state
                .reward_reference_graph
                .live_nodes()
                .unwrap()
            {
                raw.rows.insert(
                    reward_node_key(key.validator, key.block),
                    encode_node(&node),
                );
            }
            raw.rows.insert(
                validator_key(validator),
                session
                    .encode_validator_row(&session.dpos_state, validator)
                    .unwrap(),
            );
            raw.rows.insert(
                rewards_key(validator),
                encode_rewards(&session.dpos_state, validator),
            );
        }
        let before = session.dpos_state.clone();
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        let error = session.invoke(&request, quote, &raw).unwrap_err();
        match corruption {
            0 | 1 => assert!(
                matches!(error, FinalChainNativeSessionError::RawIntegrity(_)),
                "{error:?}"
            ),
            2 => assert!(
                matches!(error, FinalChainNativeSessionError::Domain(_)),
                "{error:?}"
            ),
            7 => assert!(
                matches!(error, FinalChainNativeSessionError::Domain(ref text) if text.contains("count overflow")),
                "{error:?}"
            ),
            _ => assert!(
                matches!(error, FinalChainNativeSessionError::CustodyScopeUnsupported),
                "{corruption}: {error:?}"
            ),
        }
        assert_aborted(&session, &before, 0);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_new_destination_preserves_normal_failure_prefixes_and_scope() {
    let oracle = new_oracle();
    let case = &oracle["cases"][0];
    for (amount, maximum, error, count) in [
        (1100, 1500, "Validator's max stake exceeded", 4),
        (950, 1_000_000, "Insufficient delegation", 5),
    ] {
        let (chain, storage, path) = kernel_chain_destination(maximum, address(0xa1));
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        // Normal authenticated failure precedes incomplete successful history.
        session.dpos_state.delegation_ledger_history_complete = false;
        let before = session.dpos_state.clone();
        let raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        let mut request = staged_request(case, 0);
        request.input[68..100].copy_from_slice(&U256::from(amount).to_big_endian());
        let quote = session.prepare(&request, &raw).unwrap();
        let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
        assert_eq!(
            outcome.status,
            FinalChainNativeStatus::ContractFailure {
                error: error.to_owned()
            }
        );
        assert!(
            outcome.raw_mutations.is_empty()
                && outcome.account_mutations.is_empty()
                && outcome.logs.is_empty()
        );
        assert_eq!(session.dpos_state, before);
        assert!(!session.aborted);
        let expected = case["attempts"][0]["ordered_reads"].as_array().unwrap()[..count]
            .iter()
            .map(|read| {
                ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(*raw.reads.borrow(), expected);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
