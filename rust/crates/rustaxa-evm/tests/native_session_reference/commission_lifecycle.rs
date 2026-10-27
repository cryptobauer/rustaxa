//! Successful native children under real reverting parents for every lifecycle seed.
use super::*;
use rustaxa_consensus as lifecycle_consensus;
#[path = "../../../rustaxa-consensus/src/final_chain/native_session/tests/commission_lifecycle_support.rs"]
mod support;
fn lifecycle_keccak(data: &[u8]) -> [u8; 32] {
    keccak256(data).0
}
struct RecordingPort<'a> {
    inner: mixed_native::MixedNativeExecutionPort<'a>,
    outcomes: Vec<NativeInvocationResult>,
}
impl NativeExecutionPort for RecordingPort<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.inner.prepare(i, j)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        let result = self.inner.invoke(i, q, j)?;
        self.outcomes.push(result.clone());
        Ok(result)
    }
}
#[test]
fn commission_lifecycle_real_children_parent_revert_retain_and_delete() {
    let corpus = support::corpus();
    let mut retained = 0;
    let mut deleted = 0;
    for s in corpus["scenarios"].as_array().unwrap() {
        let row = &s["frame"];
        let expanded = support::expand(&corpus, &s["state_before"]);
        let before = &expanded;
        let selected = before["period"].as_u64().unwrap();
        let period = selected + 1;
        let owner = address(row["receiver"].as_str().unwrap());
        let sender = address(row["sender"].as_str().unwrap());
        let code = bytes(&row["code_overlay"]);
        let hash = keccak256(&code).0;
        let state = FixtureState::from_case(
            &serde_json::json!({"code":row["code_overlay"],"prior_raw":row["raw_before"]}),
        );
        {
            let mut rows = state.rows.borrow_mut();
            rows.accounts.clear();
            for account in before["accounts"].as_array().unwrap() {
                let a = address(account["address"].as_str().unwrap());
                rows.accounts.insert(
                    a,
                    LogicalAccount {
                        nonce: nonce(account["nonce"].as_str().unwrap()),
                        balance: ConcreteAccountBalance::new(number(
                            account["balance"].as_str().unwrap(),
                        )),
                        code_hash: if a == owner { Some(hash) } else { None },
                        code_size: if a == owner { code.len() as u64 } else { 0 },
                    },
                );
            }
        }
        let path = FixturePath(temp_db_path("commission-lifecycle-frame"));
        let (chain, _storage) = support::history(&path.0, true, s, selected);
        let mut port = RecordingPort {
            inner: mixed_native::MixedNativeExecutionPort::new(
                chain
                    .begin_native_session(period.into(), selected.into())
                    .unwrap(),
            ),
            outcomes: vec![],
        };
        let mut journal = ExecutionJournal::new(state.clone());
        let mut sequence = PeriodConsensusSequence::new(period.into());
        let execution = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &DposNative,
            &DposNative,
            &mut port,
            &mut sequence,
            &ExecutionBlockContext {
                period: period.into(),
                author: VALIDATOR,
                timestamp: 1700000000 + period,
                gas_limit: 1000000.into(),
                chain_id: 666,
                difficulty: BigUint::default(),
            },
            &ExecutionTransaction {
                position: 0.into(),
                hash: [0; 32],
                sender,
                receiver: Some(owner),
                nonce: nonce(
                    row["accounts_before"][hex::encode(sender)]["nonce"]
                        .as_str()
                        .unwrap(),
                ),
                gas_price: ExecutionGasPrice::default(),
                gas_limit: 200000.into(),
                value: ExecutionValue::default(),
                input: bytes(&row["input"]),
                canonical_rlp: None,
                kind: ExecutionTransactionKind::Call,
            },
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(execution) = execution else {
            panic!("parent executed")
        };
        assert_eq!(
            execution.status,
            CodeExecutionStatus::Failure(CodeExecutionError::Revert)
        );
        assert_eq!(
            execution.gas_used.as_u64(),
            row["gas_used"].as_u64().unwrap()
        );
        assert_eq!(execution.output, bytes(&row["output"]));
        assert!(execution.logs.is_empty());
        assert_eq!(port.outcomes.len(), 1);
        let NativeInvocationResult::Completed(child) = &port.outcomes[0] else {
            panic!("child completed")
        };
        assert_eq!(child.status, NativeStatus::Success);
        assert_eq!(child.logs, fixture_logs(&row["native_observation"]["logs"]));
        let raw=child.raw_mutations.iter().map(|w|serde_json::json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
        assert_eq!(raw, *row["ordered_raw_writes"].as_array().unwrap());
        let effects=child.account_mutations.iter().filter_map(|m| match m {
      NativeOrdinaryAccountMutation::Balance {address,expected_exists,expected,replacement} => {assert!(*expected_exists);let delta=replacement.value()-expected.value();assert_ne!(delta,num_bigint::BigInt::default());Some(serde_json::json!({"address":hex::encode(address),"operation":if delta<num_bigint::BigInt::default(){"sub"}else{"add"},"amount":if delta<num_bigint::BigInt::default(){(-delta).to_string()}else{delta.to_string()},"balance":replacement.value().to_string()}))},
      NativeOrdinaryAccountMutation::EnsureExists{expected_exists,..}|NativeOrdinaryAccountMutation::Touch{expected_exists,..}=>{assert!(*expected_exists);None},
 NativeOrdinaryAccountMutation::Nonce{..}=>panic!("native lifecycle must not emit an account nonce mutation")
    }).collect::<Vec<_>>();
        assert_eq!(effects, *row["ordered_balance_effects"].as_array().unwrap());

        let settled = journal.settle_transaction().unwrap();
        assert_eq!(settled.native_invocations.len(), 1);
        assert_eq!(
            settled.native_invocations[0].disposition,
            ConsensusNativeDisposition::OuterFrameReverted
        );
        assert!(settled.logs.is_empty());
        state.apply(settled.writes);
        let rows = state.rows.borrow();
        for (a, expected) in row["accounts_after"].as_object().unwrap() {
            let account = &rows.accounts[&address(a)];
            assert_eq!(account.balance.value().to_string(), expected["balance"]);
            assert_eq!(
                BigUint::from_bytes_be(&account.nonce.to_bytes()).to_string(),
                expected["nonce"]
            );
        }
        let actual = rows
            .storage
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|((a, k), v)| {
                assert_eq!(*a, DPOS);
                (hex::encode(k.0), serde_json::json!(hex::encode(v)))
            })
            .collect::<serde_json::Map<_, _>>();
        assert_eq!(actual, *row["raw_after"].as_object().unwrap());
        if child
            .raw_mutations
            .iter()
            .any(|w| matches!(w.operation, NativeRawOperation::Delete))
        {
            deleted += 1
        } else {
            retained += 1
        }
    }
    assert!(retained > 0 && deleted > 0);
    println!("commission_lifecycle parent frames retain={retained} delete={deleted}");
}
struct FixturePath(PathBuf);
impl Drop for FixturePath {
    fn drop(&mut self) {
        crate::closeout_test_scratch::dispose(&self.0).unwrap();
    }
}
