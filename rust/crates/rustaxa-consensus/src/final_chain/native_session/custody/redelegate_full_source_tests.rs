//! Actual full caller-row removal while another delegator retains the validator.
use super::*;

fn full_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_full_source/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_full_source/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    public
}

fn full_raw(case: &Value) -> AuthenticatedRaw {
    let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    for read in case["seed_raw"].as_array().unwrap() {
        if read["present"].as_bool().unwrap() {
            raw.rows.insert(
                ConcreteStorageKey(unhex(read["key"].as_str().unwrap()).try_into().unwrap()),
                unhex(read["value"].as_str().unwrap()),
            );
        }
    }
    raw
}

fn source(case: &Value) -> u8 {
    unhex(case["from"].as_str().unwrap())[19]
}
fn destination(case: &Value) -> u8 {
    unhex(case["to"].as_str().unwrap())[19]
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
fn redelegate_full_source_pending_and_historical_match_both_actual_removal_shapes() {
    for case in full_oracle()["cases"].as_array().unwrap() {
        for historical in [false, true] {
            let (chain, storage, path) =
                kernel_chain_profile(1_000_000, address(0xd1), Some(source(case)));
            if historical {
                install_historical(&chain);
            }
            let committed_period = if historical { 1.into() } else { 0.into() };
            let committed = chain.dpos_snapshot(committed_period).unwrap();
            let mut pending;
            let mut simulation;
            let session = if historical {
                simulation = chain.begin_native_simulation(1.into()).unwrap();
                &mut simulation.session
            } else {
                pending = chain.begin_native_session(1.into(), 0.into()).unwrap();
                &mut pending
            };
            assert_facts(&session.dpos_state, &case["facts_before"]);
            let before = session.dpos_state.clone();
            let mut next = before.clone();
            let mut accounts = StagedDposAccountPort::new([]).unwrap();
            let input = unhex(case["input"].as_str().unwrap());
            chain
                .apply_dpos_redelegate(
                    &mut next,
                    &mut accounts,
                    address(0xd1),
                    address(source(case)),
                    address(destination(case)),
                    input[68..100].to_vec(),
                    1.into(),
                )
                .unwrap();
            assert!(accounts.into_mutations().is_empty());
            let mut raw = full_raw(case);
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
                session.dpos_state.total_stakes[&address(source(case))].as_u256(),
                U256::from(1000)
            );
            assert_eq!(
                session.dpos_state.total_stakes[&address(destination(case))].as_u256(),
                U256::from(2000)
            );
            assert!(matches!(
                session
                    .dpos_state
                    .reward_reference_graph
                    .read_cursor(&address(source(case)), &address(0xd1)),
                Err(DposRewardGraphError::MissingCursor { .. })
            ));
            assert!(
                session
                    .dpos_state
                    .delegation_reward_cursors
                    .get(&address(source(case)))
                    .is_none_or(|rows| !rows.contains_key(&address(0xd1)))
            );
            let reads = raw.reads.borrow();
            assert_eq!(
                reads
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                reads.len()
            );
            drop(reads);
            raw.apply(&outcome.raw_mutations);
            // Compare committed Go presence, including physical deletes. The shared
            // helper retains empty deletion markers, so remove those markers here.
            for mutation in &outcome.raw_mutations {
                if matches!(mutation.operation, FinalChainNativeRawOperation::Delete) {
                    raw.rows.remove(&mutation.key);
                }
            }
            for (key, row) in case["final_raw"].as_object().unwrap() {
                let key = ConcreteStorageKey(unhex(key).try_into().unwrap());
                assert_eq!(
                    raw.rows.contains_key(&key),
                    row["present"].as_bool().unwrap()
                );
                assert_eq!(
                    raw.rows.get(&key).cloned().unwrap_or_default(),
                    unhex(row["value"].as_str().unwrap())
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
            assert_eq!(chain.dpos_snapshot(committed_period).unwrap(), committed);
            // Following same-direction transfer authenticates the now absent row
            // and returns a normal failure without resetting retained membership.
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
            assert_eq!(raw.reads.borrow().len(), 5);
            assert_eq!(session.dpos_state, warm);
            assert!(!session.aborted);
            drop(chain);
            drop(storage);
            std::fs::remove_dir_all(path).unwrap();
        }
    }
}

#[test]
fn redelegate_full_source_all_authentication_and_reader_failures_do_not_advance() {
    for case in full_oracle()["cases"].as_array().unwrap() {
        let (chain, storage, path) =
            kernel_chain_profile(1_000_000, address(0xd1), Some(source(case)));
        let raw = full_raw(case);
        let request = staged_request(case, 0);
        let mut successful = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let quote = successful.prepare(&request, &raw).unwrap();
        successful.invoke(&request, quote, &raw).unwrap();
        let keys = raw.reads.borrow().clone();
        println!(
            "{}: {} fresh Rust authentication reads",
            case["name"],
            keys.len()
        );
        assert!(
            keys.contains(&iterable_count_key(&delegator_validators_prefix(address(
                0xd1
            ))))
        );
        for (index, key) in keys.iter().enumerate() {
            for reader_failure in [false, true] {
                let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
                let before = session.dpos_state.clone();
                let mut bad = full_raw(case);
                if reader_failure {
                    bad.fail_at = Some(index + 1);
                } else {
                    bad.rows.insert(*key, vec![0xff]);
                }
                let quote = session.prepare(&request, &bad).unwrap();
                let error = session.invoke(&request, quote, &bad).unwrap_err();
                assert!(
                    if reader_failure {
                        matches!(error, FinalChainNativeSessionError::StateRead(_))
                    } else {
                        matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                    },
                    "{key:?}: {error:?}"
                );
                assert_eq!(session.dpos_state, before);
                assert!(session.aborted && session.prepared.is_none());
                assert_eq!(session.next_sequence, 0);
            }
        }
        drop(successful);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}

#[test]
fn redelegate_full_source_rejects_validator_deletion_and_inconsistent_new_destination_seed() {
    let oracle = full_oracle();
    let case = &oracle["cases"][0];
    for retained in [false, true] {
        let (chain, storage, path) = if retained {
            kernel_chain_profile(1_000_000, address(0xa1), Some(0x31))
        } else {
            kernel_chain(1_000_000)
        };
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut raw = full_raw(case);
        for validator in [0x31, 0x32] {
            raw.rows.insert(
                validator_key(address(validator)),
                session
                    .encode_validator_row(&session.dpos_state, address(validator))
                    .unwrap(),
            );
        }
        let before = session.dpos_state.clone();
        let request = staged_request(case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        let error = session.invoke(&request, quote, &raw).unwrap_err();
        if retained {
            // This one-member semantic full+new shape is now admitted, but the
            // old physical fixture still contains the caller destination pair
            // and membership. It must fail exact raw authentication.
            assert!(matches!(
                error,
                FinalChainNativeSessionError::RawIntegrity(_)
            ));
        } else {
            assert_eq!(error, FinalChainNativeSessionError::CustodyScopeUnsupported);
        }
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted);
        assert_eq!(session.next_sequence, 0);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
