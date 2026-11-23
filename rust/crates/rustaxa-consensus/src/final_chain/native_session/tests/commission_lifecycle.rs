//! Native lifecycle parity and every consumed deletion-read failure.
use super::super::*;
use crate as lifecycle_consensus;
#[path = "commission_lifecycle_fault_support.rs"]
mod faults;
#[path = "commission_lifecycle_support.rs"]
mod support;
use num_bigint::{BigInt, BigUint};
use serde_json::Value;
use std::cell::RefCell;
fn lifecycle_keccak(data: &[u8]) -> [u8; 32] {
    use tiny_keccak::{Hasher, Keccak};
    let mut h = Keccak::v256();
    h.update(data);
    let mut out = [0; 32];
    h.finalize(&mut out);
    out
}
struct State {
    snapshot: Value,
    reads: RefCell<Vec<ConcreteStorageKey>>,
    fault: Option<(usize, bool)>,
}
impl FinalChainNativeStateRead for State {
    fn account(
        &self,
        a: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        let hex = hex_string(&a);
        let row = self.snapshot["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["address"] == hex)
            .unwrap();
        Ok(FinalChainNativeAccount {
            exists: row["exists"].as_bool().unwrap(),
            nonce: FinalChainNonce::from_u64(row["nonce"].as_str().unwrap().parse().unwrap()),
            balance: row["balance"].as_str().unwrap().parse::<BigInt>().unwrap(),
        })
    }
    fn raw_storage(
        &self,
        _a: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push(*key);
        if let Some((at, malformed)) = self.fault
            && index == at
        {
            if malformed {
                return Ok(ConcreteRead::Present(vec![0xff]));
            }
            return Err(FinalChainNativeStateReadError::Invariant(format!(
                "commission lifecycle read {at}"
            )));
        }
        Ok(self.snapshot["native_raw"]
            .get(hex_string(&key.0))
            .map_or(ConcreteRead::Absent, |v| {
                ConcreteRead::Present(support::decode(v.as_str().unwrap()))
            }))
    }
}
fn hex_string(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn request(row: &Value) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: 0.into(),
            sequence: 0,
        },
        period: row["period"].as_u64().unwrap().into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: support::decode(row["sender"].as_str().unwrap())
            .try_into()
            .unwrap(),
        contract: DPOS_CONTRACT_ADDRESS,
        state_address: DPOS_CONTRACT_ADDRESS,
        value: FinalChainNativeValue::new(BigUint::default()),
        input: support::decode(row["input"].as_str().unwrap()),
        supplied_gas: 200000.into(),
    }
}
fn assert_semantics(snapshot: &DposSnapshot, expected: &Value) {
    let order = snapshot
        .validator_order
        .iter()
        .map(|a| format!("0x{}", hex_string(a)))
        .collect::<Vec<_>>();
    assert_eq!(
        serde_json::to_value(order).unwrap(),
        expected["validator_order"]
    );
    let mut membership = 0;
    for row in expected["validators"].as_array().unwrap() {
        let address: [u8; 20] = support::decode(row["address"].as_str().unwrap())
            .try_into()
            .unwrap();
        if row["validator"].is_null() {
            assert!(!snapshot.total_stakes.contains_key(&address));
            assert!(!snapshot.validator_metadata.contains_key(&address));
            assert!(!snapshot.vrf_keys.contains_key(&address));
        } else {
            membership += 1;
            assert_eq!(
                snapshot.total_stakes[&address].as_u256().to_string(),
                row["validator"]["TotalStake"].to_string()
            );
            assert_eq!(
                checked_undelegations_count(snapshot, address).unwrap() as u64,
                row["validator"]["UndelegationsCount"].as_u64().unwrap()
            );
            assert_eq!(
                snapshot
                    .commission_rewards
                    .get(&address)
                    .map(|v| v.as_u256())
                    .unwrap_or_default()
                    .to_string(),
                row["rewards"]["CommissionRewardsPool"].to_string()
            );
        }
    }
    assert_eq!(snapshot.total_stakes.len(), membership);
    let canonical = canonical_concrete_precompile_storage(snapshot, true).unwrap();
    for (key, value) in expected["native_raw"].as_object().unwrap() {
        let k = (
            DPOS_CONTRACT_ADDRESS,
            support::decode(key).try_into().unwrap(),
        );
        let b = support::decode(value.as_str().unwrap());
        assert!(
            canonical.get(&k).is_some_and(|v| v.contains(&b)) || b.is_empty() || b == vec![0; 4],
            "semantic native row {key}"
        );
    }
}
#[test]
fn commission_lifecycle_native_matrix_and_consumed_read_faults() {
    let corpus = support::corpus();
    let mut cases = 0;
    let mut faults = 0;
    for s in corpus["scenarios"].as_array().unwrap() {
        let root = crate::test_paths::new_path("commission-lifecycle-native");
        for row in s["cases"].as_array().unwrap() {
            let selected = row["state_before"]["period"].as_u64().unwrap();
            let (chain, _storage) = support::history(&root, true, s, selected);
            let req = request(row);
            let state = State {
                snapshot: support::expand(&corpus, &row["state_before"]),
                reads: RefCell::default(),
                fault: None,
            };
            let mut session = chain
                .begin_native_session(req.period, selected.into())
                .unwrap();
            let before = session.dpos_state.clone();
            let quote = session.prepare(&req, &state).unwrap();
            let FinalChainNativeInvocationResult::Completed(out) =
                session.invoke(&req, quote, &state).unwrap()
            else {
                panic!("completed lifecycle")
            };
            assert_semantics(&before, &row["state_before"]);
            let canonical = canonical_concrete_precompile_storage(&before, true).unwrap();
            for (key, value) in state.snapshot["native_raw"].as_object().unwrap() {
                let k: ([u8; 20], [u8; 32]) = (
                    DPOS_CONTRACT_ADDRESS,
                    support::decode(key).try_into().unwrap(),
                );
                let b = support::decode(value.as_str().unwrap());
                assert!(
                    canonical.get(&k).is_some_and(|v| v.contains(&b))
                        || b.is_empty()
                        || b == vec![0; 4],
                    "{} raw {key}",
                    s["name"]
                );
            }
            let reads = state.reads.borrow().clone();
            for (index, _) in reads.iter().enumerate() {
                for malformed in [false, true] {
                    let state = State {
                        snapshot: support::expand(&corpus, &row["state_before"]),
                        reads: RefCell::default(),
                        fault: Some((index, malformed)),
                    };
                    let mut session = chain
                        .begin_native_session(req.period, selected.into())
                        .unwrap();
                    let quote = session.prepare(&req, &state).unwrap();
                    let error = session.invoke(&req, quote, &state).unwrap_err();
                    if !malformed {
                        assert_eq!(
                            error,
                            FinalChainNativeSessionError::StateRead(
                                FinalChainNativeStateReadError::Invariant(format!(
                                    "commission lifecycle read {index}"
                                ))
                            )
                        );
                    } else {
                        assert_eq!(
                            error,
                            FinalChainNativeSessionError::RawIntegrity(
                                faults::malformed_raw_error(row, &reads[index].0)
                            )
                        );
                    }
                    assert_eq!(session.dpos_state, before);
                    assert_eq!(
                        session.prepare(&req, &state),
                        Err(FinalChainNativeSessionError::Aborted)
                    );
                    assert_eq!(state.reads.borrow().len(), index + 1);
                    faults += 1;
                }
            }
            let state = State {
                snapshot: support::expand(&corpus, &row["state_before"]),
                reads: RefCell::default(),
                fault: None,
            };
            let mut retry = chain
                .begin_native_session(req.period, selected.into())
                .unwrap();
            let quote = retry.prepare(&req, &state).unwrap();
            assert_eq!(
                retry.invoke(&req, quote, &state).unwrap(),
                FinalChainNativeInvocationResult::Completed(out)
            );
            cases += 1;
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    assert!(cases > 168 && faults > 0);
    println!("commission_lifecycle native cases={cases} consumed-read faults={faults}");
}
