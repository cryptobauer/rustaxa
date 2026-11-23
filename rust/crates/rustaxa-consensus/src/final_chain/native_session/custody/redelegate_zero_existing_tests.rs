//! Pre-Aspen-two zero amount with positive existing caller pairs.
use super::*;

fn zero_case() -> Value {
    let oracle = oracle();
    oracle["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "zero_before_aspen_two")
        .unwrap()
        .clone()
}

#[test]
fn redelegate_zero_existing_pairs_authentication_failures_do_not_advance() {
    let case = zero_case();
    let (chain, storage, path) = kernel_chain(1_000_000);
    let mut probe = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    let request = staged_request(&case, 0);
    let quote = probe.prepare(&request, &raw).unwrap();
    let outcome = completed(probe.invoke(&request, quote, &raw).unwrap());
    let keys = raw.reads.borrow().clone();
    assert_eq!(keys.len(), 14);
    for (index, key) in keys.iter().enumerate() {
        for reader_failure in [false, true] {
            let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
            let before = session.dpos_state.clone();
            let mut broken = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
            if reader_failure {
                broken.fail_at = Some(index + 1);
            } else {
                broken.rows.insert(*key, vec![0xff]);
            }
            let quote = session.prepare(&request, &broken).unwrap();
            let error = session.invoke(&request, quote, &broken).unwrap_err();
            assert!(
                if reader_failure {
                    matches!(error, FinalChainNativeSessionError::StateRead(_))
                } else {
                    matches!(error, FinalChainNativeSessionError::RawIntegrity(_))
                },
                "{index}: {error:?}"
            );
            assert_eq!(session.dpos_state, before);
            assert!(session.aborted && session.prepared.is_none());
            assert_eq!(session.next_sequence, 0);
        }
    }
    raw.apply(&outcome.raw_mutations);
    let warm = probe.dpos_state.clone();
    let mut reader = AuthenticatedRaw {
        rows: raw.rows.clone(),
        reads: RefCell::new(Vec::new()),
        fail_at: None,
    };
    let next_request = staged_request(&case, 1);
    let mut discovery = chain.begin_native_session(1.into(), 0.into()).unwrap();
    discovery.dpos_state = warm.clone();
    discovery.next_sequence = 1;
    let quote = discovery.prepare(&next_request, &reader).unwrap();
    discovery.invoke(&next_request, quote, &reader).unwrap();
    let warm_keys = reader.reads.borrow().clone();
    assert!(!warm_keys.is_empty());
    for key in warm_keys {
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        session.dpos_state = warm.clone();
        session.next_sequence = 1;
        reader.rows = raw.rows.clone();
        reader.rows.insert(key, vec![0xff]);
        reader.reads.borrow_mut().clear();
        let quote = session.prepare(&next_request, &reader).unwrap();
        assert!(matches!(
            session.invoke(&next_request, quote, &reader),
            Err(FinalChainNativeSessionError::RawIntegrity(_))
        ));
        assert_eq!(session.dpos_state, warm);
        assert_eq!(session.next_sequence, 1);
        assert!(session.aborted);
    }
    drop(probe);
    drop(discovery);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn redelegate_zero_existing_pairs_preserves_scope_and_aspen_precedence() {
    let case = zero_case();
    for excluded in 0..3 {
        let (chain, storage, path) = if excluded == 0 {
            kernel_chain_destination(1_000_000, address(0xa1))
        } else {
            kernel_chain(1_000_000)
        };
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        let mut raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
        if excluded > 0 {
            let validator = address(if excluded == 1 { 0x31 } else { 0x32 });
            session
                .dpos_state
                .delegations
                .get_mut(&validator)
                .unwrap()
                .insert(
                    address(0xd1),
                    StoredDposTokenAmount::canonical_u256_after_mutation(U256::zero()),
                );
            raw.rows.insert(
                delegation_key(validator, address(0xd1)),
                encode_delegation(&session.dpos_state, validator, address(0xd1)).unwrap(),
            );
        }
        let before = session.dpos_state.clone();
        let request = staged_request(&case, 0);
        let quote = session.prepare(&request, &raw).unwrap();
        assert!(matches!(
            session.invoke(&request, quote, &raw),
            Err(FinalChainNativeSessionError::CustodyScopeUnsupported)
        ));
        assert_eq!(session.dpos_state, before);
        assert_eq!(session.next_sequence, 0);
        assert!(session.aborted);
        drop(session);
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
    let (mut chain, storage, path) = kernel_chain(1_000_000);
    chain.rewards_config.aspen_part_two_period = 1.into();
    let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
    let before = session.dpos_state.clone();
    let raw = AuthenticatedRaw::from_attempt(&case["attempts"][0]);
    let request = staged_request(&case, 0);
    let quote = session.prepare(&request, &raw).unwrap();
    let outcome = completed(session.invoke(&request, quote, &raw).unwrap());
    assert!(
        matches!(outcome.status, FinalChainNativeStatus::ContractFailure { ref error } if error == "Redelegation has to be more than 0")
    );
    assert!(raw.reads.borrow().is_empty());
    assert!(
        outcome.raw_mutations.is_empty()
            && outcome.account_mutations.is_empty()
            && outcome.logs.is_empty()
    );
    assert_eq!(session.dpos_state, before);
    assert_eq!(session.next_sequence, 1);
    assert!(!session.aborted);
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
