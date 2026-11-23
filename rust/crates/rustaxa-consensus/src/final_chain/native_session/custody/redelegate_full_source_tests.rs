//! Actual full caller-row removal while another delegator retains the validator.
use super::*;

fn full_oracle() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/native_redelegate_full_source/public.json"
    )))
    .unwrap();
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
