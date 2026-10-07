//! Actual positive-reward partial frames with public finalized P1 FinalChain
//! hydration and an independent physical Go P1 read lane. No EVM publication,
//! signed historical simulation or root parity is claimed.
use super::*;
use ethereum_types::{H160, H256};
use k256::ecdsa::SigningKey;
use rlp::RlpStream;
use rustaxa_consensus::RewardCertVoteFact;
use rustaxa_storage::Column;
use rustaxa_types::GenesisAccount;
#[allow(dead_code)]
#[path = "../support/mixed_native.rs"]
mod mixed_native;
fn addr(last: u8) -> [u8; 20] {
    let mut a = [0; 20];
    a[19] = last;
    a
}
fn seed() -> Value {
    serde_json::from_str::<Value>(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward/public.json"
    ))
    .unwrap()["cases"][0]
        .clone()
}
fn corpus() -> Vec<Value> {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_frames/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_frames/local.json")).unwrap();
    assert_eq!(public, local);
    public["cases"].as_array().unwrap().clone()
}
#[derive(Clone)]
struct RewardState(FixtureState);
impl ConcreteStateRead for RewardState {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: 1.into(),
            state_root: [0; 32],
        }
    }
    fn account(
        &self,
        a: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        self.0.account(a)
    }
    fn storage(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.storage(a, k)
    }
    fn code(&self, h: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        self.0.code(h)
    }
}
fn state(row: &Value) -> RewardState {
    let seed = seed();
    let mut rows = LogicalRows::default();
    let code = bytes(&row["code"]);
    let code_hash = keccak256(&code).0;
    rows.code.insert(code_hash, code.clone());
    for (a, fact) in row["accounts_before"].as_object().unwrap() {
        let address = address(a);
        let wrapper = address == addr(0xd1);
        rows.accounts.insert(
            address,
            LogicalAccount {
                nonce: FinalChainNonce::from_u64(fact["nonce"].as_str().unwrap().parse().unwrap()),
                balance: ConcreteAccountBalance::new(
                    fact["balance"].as_str().unwrap().parse().unwrap(),
                ),
                code_hash: wrapper.then_some(code_hash),
                code_size: if wrapper { code.len() as u64 } else { 0 },
            },
        );
    }
    assert_eq!(row["accounts_before"], seed["reward_committed_accounts"]);
    // Forward-resolve every selected raw key from full committedP1 physical rows.
    for (logical, value) in row["prior_raw"].as_object().unwrap() {
        let key = ConcreteStorageKey(hash(logical));
        let prefix = rustaxa_storage::storage_version_prefix(DPOS, key);
        let physical = seed["reward_committed_rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["column"] == 4)
            .filter_map(|r| {
                let raw = bytes(&r["key"]);
                if raw.len() != 40 || raw[..32] != prefix {
                    return None;
                }
                let p = u64::from_be_bytes(raw[32..].try_into().unwrap());
                (p <= 1).then_some((p, bytes(&r["value"])))
            })
            .max_by_key(|(p, _)| *p)
            .map(|(_, v)| v)
            .unwrap_or_default();
        assert_eq!(physical, bytes(value));
        if !physical.is_empty() {
            rows.storage.insert((DPOS, key), physical);
        }
    }
    // Rust authenticates two owner rows that Go Run does not load. Resolve them
    // from complete actual P1 inventory; keep the measured52-key map unchanged.
    for validator in [addr(0x31), addr(0x32)] {
        let key = ConcreteStorageKey(keccak256([&[0, 3][..], &validator].concat()).0);
        let prefix = rustaxa_storage::storage_version_prefix(DPOS, key);
        let value = seed["reward_committed_rows"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["column"] == 4)
            .filter_map(|r| {
                let raw = bytes(&r["key"]);
                if raw.len() != 40 || raw[..32] != prefix {
                    return None;
                }
                let p = u64::from_be_bytes(raw[32..].try_into().unwrap());
                (p <= 1).then_some((p, bytes(&r["value"])))
            })
            .max_by_key(|(p, _)| *p)
            .unwrap()
            .1;
        assert_eq!(value, addr(0xa1));
        assert!(rows.storage.insert((DPOS, key), value).is_none());
    }
    assert_eq!(row["prior_raw"].as_object().unwrap().len(), 52);
    RewardState(FixtureState {
        rows: Rc::new(RefCell::new(rows)),
    })
}
fn publish_reward_p1(chain: &FinalChain, storage: &Storage) {
    fn fields(s: &mut RlpStream) {
        for value in 10..14 {
            s.append(&H256::from_low_u64_be(value));
        }
        s.append(&1u64).append(&0u64).begin_list(0);
    }
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned);
    let (signature, recovery) = SigningKey::from_slice(&[9; 32])
        .unwrap()
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .unwrap();
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut pbft = RlpStream::new_list(8);
    fields(&mut pbft);
    pbft.append(&signature);
    let pbft = pbft.out().to_vec();
    let mut period = RlpStream::new_list(4);
    period
        .append_raw(&pbft, 1)
        .begin_list(0)
        .begin_list(0)
        .begin_list(0);
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(
            &mut batch,
            Column::PeriodData,
            &1u64.to_le_bytes(),
            &period.out(),
        )
        .unwrap();
    storage.commit_write_batch_with_sync(batch, false).unwrap();
    let (header, receipts) = chain
        .finalize_block_with_rewards_facts_at(
            pbft,
            vec![],
            vec![],
            1,
            vec![RewardCertVoteFact {
                voter: H160(addr(0x31)),
                weight: 1,
                period: 1,
            }],
            1.into(),
        )
        .unwrap();
    assert!(receipts.is_empty());
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        rlp::Rlp::new(&header)
            .val_at::<ethereum_types::U256>(11)
            .unwrap(),
        800.into()
    );
    for (a, fact) in seed()["reward_committed_accounts"].as_object().unwrap() {
        let account = chain.account(address(a)).unwrap().unwrap();
        assert_eq!(
            account.balance.as_u256().to_string(),
            fact["balance"].as_str().unwrap()
        );
    }
}
struct RecordingPort<'a> {
    inner: mixed_native::MixedNativeExecutionPort<'a>,
    prepared: Vec<(NativeInvocation, FinalChainGas)>,
    invoked: Vec<NativeInvocation>,
    outcomes: Vec<NativeInvocationResult>,
}
impl NativeExecutionPort for RecordingPort<'_> {
    fn prepare(
        &mut self,
        i: &NativeInvocation,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let q = self.inner.prepare(i, j)?;
        self.prepared.push((i.clone(), q.required_gas));
        Ok(q)
    }
    fn invoke(
        &mut self,
        i: &NativeInvocation,
        q: NativeGasQuote,
        j: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.invoked.push(i.clone());
        let r = self.inner.invoke(i, q, j)?;
        self.outcomes.push(r.clone());
        Ok(r)
    }
}
fn logical(read: ConcreteRead<Vec<u8>>) -> Option<Vec<u8>> {
    match read {
        ConcreteRead::Present(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}
fn account_effects(mutations: &[NativeOrdinaryAccountMutation]) -> Value {
    serde_json::json!(mutations.iter().map(|m|{let NativeOrdinaryAccountMutation::Balance{address,expected,replacement,..}=m else{panic!("only payout balance effects")};let delta=replacement.value()-expected.value();let (operation,amount)=if delta.sign()==num_bigint::Sign::Minus {("sub",-delta)}else{("add",delta)};serde_json::json!({"address":hex::encode(address),"operation":operation,"amount":amount.to_string(),"balance":replacement.value().to_string()})}).collect::<Vec<_>>())
}
fn chain(storage: Arc<Storage>, row: &Value, initialize: bool) -> FinalChain {
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        1_000_000.into(),
        0,
        // FinalChain account input is post-genesis custody, unlike Go configuration.
        vec![
            GenesisAccount {
                address: addr(0xd1),
                balance: rustaxa_types::FinalChainAccountBalance::new_account(
                    ethereum_types::U256::from(3000),
                ),
            },
            GenesisAccount {
                address: addr(0xa1),
                balance: rustaxa_types::FinalChainAccountBalance::new_account(
                    ethereum_types::U256::from(1000),
                ),
            },
        ],
        [0x31, 0x32]
            .into_iter()
            .map(|last| GenesisValidator {
                address: addr(last),
                vrf_key: [match last {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: ethereum_types::U256::from(2000).to_big_endian().to_vec(),
                delegations: vec![(
                    addr(0xa1),
                    ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                )]
                .into_iter()
                .chain(Some((
                    addr(0xd1),
                    ethereum_types::U256::from(1000).to_big_endian().to_vec(),
                )))
                .collect(),
                metadata: GenesisValidatorMetadata {
                    owner: addr(0xa1),
                    commission: 100,
                    ..Default::default()
                },
            })
            .collect(),
        GenesisDposConfig {
            eligibility_balance_threshold: ethereum_types::U256::from(100).into(),
            vote_eligibility_balance_step: ethereum_types::U256::from(10).into(),
            validator_maximum_stake: ethereum_types::U256::from(1_000_000).into(),
            minimum_deposit: ethereum_types::U256::from(100).into(),
            delegation_delay: 1,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: row["fix"].as_u64().unwrap().into(),
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: if row["aspen_zero"].as_bool().unwrap() {
                1.into()
            } else {
                FinalChainBlockNumber::MAX
            },
            dpos_blocks_per_year: 1,
            yield_percentage: 20,
            committee_size: 1,
            max_block_author_reward_percent: 0,
            dag_proposers_reward_percent: 0,
            aspen_max_supply: ethereum_types::U256::from(8000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    if initialize {
        publish_reward_p1(&chain, &storage);
    }
    chain
}

fn transaction(row: &Value, position: u32) -> ExecutionTransaction {
    ExecutionTransaction {
        position: position.into(),
        hash: [if position == 0 { 0x11 } else { 0x22 }; 32],
        sender: address(row["from"].as_str().unwrap()),
        receiver: Some(address(row["to"].as_str().unwrap())),
        nonce: FinalChainNonce::from_u64(row["nonce"].as_u64().unwrap()),
        gas_price: ExecutionGasPrice::default(),
        gas_limit: row["gas"].as_u64().unwrap().into(),
        value: ExecutionValue::new(row["value"].as_u64().unwrap().into()),
        input: bytes(&row["input"]),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    }
}

fn block() -> ExecutionBlockContext {
    ExecutionBlockContext {
        period: 2.into(),
        author: [0; 20],
        timestamp: 0,
        gas_limit: 1_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    }
}

fn writes_json(ordered: &[&NativeRawMutation]) -> Value {
    serde_json::json!(ordered.iter().map(|write| {
        let value = match &write.operation { NativeRawOperation::Put(value) => value.as_bytes(), NativeRawOperation::Delete => &[] };
        serde_json::json!({"address":hex::encode(write.address),"key":hex::encode(write.key.0),"value":hex::encode(value)})
    }).collect::<Vec<_>>())
}

#[test]
fn rewarded_partial_public_p1_frames_match_actual_go_account_and_raw_rollback() {
    let cases = corpus();
    assert_eq!(cases.len(), 7);
    for row in cases {
        let state = state(&row);
        let original_raw = state.0.rows.borrow().storage.clone();
        let path = temp_db_path("rewarded-partial-public-frames");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let first = chain(storage.clone(), &row, true);
        drop(first);
        let chain = chain(storage.clone(), &row, false);
        assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
        let committed_principal = chain.dpos_total_amount_delegated(1.into()).unwrap();
        let committed_stakes = chain
            .dpos_validators_total_stakes(1.into())
            .unwrap()
            .into_iter()
            .map(|r| (r.address, r.stake))
            .collect::<Vec<_>>();
        let committed_accounts = [addr(0xd1), addr(0xa1), DPOS]
            .into_iter()
            .map(|a| (a, chain.account(a).unwrap()))
            .collect::<Vec<_>>();
        let mut port = RecordingPort {
            inner: mixed_native::MixedNativeExecutionPort::new(
                chain
                    .begin_native_session_bound([0x91; 32], 2.into(), 1.into())
                    .unwrap(),
            ),
            prepared: vec![],
            invoked: vec![],
            outcomes: vec![],
        };
        let mut sequence = PeriodConsensusSequence::new(2.into());
        let mut journal = ExecutionJournal::new(state.clone());
        let execution = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &DposNative,
            &DposNative,
            &mut port,
            &mut sequence,
            &block(),
            &transaction(&row, 0),
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(false),
        )
        .unwrap();
        let TransactionExecutionResult::Executed(execution) = execution else {
            panic!("Go admitted {}", row["name"])
        };
        assert_eq!(
            execution.gas_used.as_u64(),
            row["transaction_gas_used"].as_u64().unwrap(),
            "{}",
            row["name"]
        );
        assert_eq!(execution.output, bytes(&row["parent_output"]));
        assert_eq!(execution.logs, fixture_logs(&row["logs"]));
        assert_eq!(
            execution.status,
            if row["parent_revert"] == true {
                CodeExecutionStatus::Failure(CodeExecutionError::Revert)
            } else {
                CodeExecutionStatus::Success
            }
        );
        let settled = journal.settle_transaction().unwrap();
        let calls = row["calls"].as_array().unwrap();
        assert_eq!(sequence.next_sequence(), calls.len() as u64);
        assert_eq!(port.outcomes.len(), calls.len());
        assert_eq!(settled.native_invocations.len(), calls.len());
        let mut ordered = vec![];
        let mut prefix = original_raw
            .iter()
            .map(|((_, key), v)| (*key, v.clone()))
            .collect::<BTreeMap<_, _>>();
        for (index, call) in calls.iter().enumerate() {
            let (invocation, quote) = &port.prepared[index];
            assert_eq!(invocation, &port.invoked[index]);
            assert_eq!(invocation.id.sequence, index as u64);
            assert_eq!(invocation.id.transaction, 0.into());
            assert_eq!(invocation.period, 2.into());
            assert_eq!(invocation.depth, call["depth"].as_u64().unwrap() as u16);
            assert_eq!(invocation.caller, address(call["caller"].as_str().unwrap()));
            assert_eq!(invocation.input, bytes(&call["input"]));
            assert_eq!(
                invocation.value.value(),
                &BigUint::from(row["value"].as_u64().unwrap())
            );
            assert_eq!(
                invocation.supplied_gas.as_u64(),
                call["supplied_native_gas"].as_u64().unwrap()
            );
            assert_eq!(quote.as_u64(), call["required_gas"].as_u64().unwrap());
            assert_eq!(
                invocation.kind,
                if row["static"] == true {
                    NativeCallKind::StaticCall
                } else {
                    NativeCallKind::Call
                }
            );
            assert_eq!(invocation.is_static, row["static"].as_bool().unwrap());
            let fact = &settled.native_invocations[index];
            let success = call["native_called"] == true && call["native_error"] == "";
            assert_eq!(
                fact.disposition,
                if !success {
                    ConsensusNativeDisposition::OwnFrameReverted
                } else if row["parent_revert"] == true {
                    ConsensusNativeDisposition::OuterFrameReverted
                } else {
                    ConsensusNativeDisposition::Normal
                }
            );
            assert_eq!(fact.logs, fixture_logs(&call["logs"]));
            assert_eq!(fact.output, bytes(&call["native_output"]));
            if call["native_called"] == false {
                assert!(matches!(
                    port.outcomes[index],
                    NativeInvocationResult::InsufficientGas { .. }
                ));
                assert_eq!(fact.gas_used, FinalChainGas::ZERO);
                assert_eq!(
                    fact.status,
                    CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas)
                );
                continue;
            }
            assert_eq!(fact.gas_used, *quote);
            let NativeInvocationResult::Completed(outcome) = &port.outcomes[index] else {
                panic!("funded")
            };
            assert_eq!(
                account_effects(&outcome.account_mutations),
                call["ordered_balance_effects"]
            );
            assert_eq!(outcome.logs, fixture_logs(&call["logs"]));
            assert_eq!(outcome.output, bytes(&call["native_output"]));
            if success {
                assert_eq!(fact.status, CodeExecutionStatus::Success)
            } else {
                let CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) = &fact.status
                else {
                    panic!("native failure")
                };
                assert_eq!(error.error, call["native_error"].as_str().unwrap());
            }
            let current = outcome.raw_mutations.iter().collect::<Vec<_>>();
            assert_eq!(writes_json(&current), call["ordered_raw_writes"]);
            for write in &current {
                assert_eq!(
                    logical(write.expected.clone()),
                    prefix.get(&write.key).cloned()
                );
                match &write.operation {
                    NativeRawOperation::Put(v) => {
                        prefix.insert(write.key, v.as_bytes().to_vec());
                    }
                    NativeRawOperation::Delete => {
                        prefix.remove(&write.key);
                    }
                }
            }
            ordered.extend(current);
        }
        assert_eq!(writes_json(&ordered), row["ordered_raw_writes"]);
        let changed_keys = ordered
            .iter()
            .map(|w| w.key)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(port.inner.take_contexts().len(), calls.len());
        // The public FinalChain P1 snapshot stays fixed after disposal of the P2 session.
        drop(port);
        for (a, account) in committed_accounts {
            assert_eq!(chain.account(a).unwrap(), account)
        }
        assert_eq!(
            chain.dpos_total_amount_delegated(1.into()).unwrap(),
            committed_principal
        );
        assert_eq!(
            chain
                .dpos_validators_total_stakes(1.into())
                .unwrap()
                .into_iter()
                .map(|r| (r.address, r.stake))
                .collect::<Vec<_>>(),
            committed_stakes
        );
        state.0.apply(settled.writes);
        for (a, fact) in row["accounts"].as_object().unwrap() {
            let rows = state.0.rows.borrow();
            let account = &rows.accounts[&address(a)];
            assert_eq!(
                account.balance.value().to_string(),
                fact["balance"].as_str().unwrap()
            );
            assert_eq!(
                account.nonce.as_u64(),
                Some(fact["nonce"].as_str().unwrap().parse::<u64>().unwrap())
            );
        }
        for (key, value) in row["after_raw"].as_object().unwrap() {
            assert_eq!(
                logical(state.storage(DPOS, ConcreteStorageKey(hash(key))).unwrap()),
                if bytes(value).is_empty() {
                    None
                } else {
                    Some(bytes(value))
                }
            );
        }
        for (key, value) in original_raw {
            let expected = prefix.get(&key.1).cloned();
            assert_eq!(logical(state.storage(key.0, key.1).unwrap()), expected);
            if !changed_keys.contains(&key.1) {
                assert_eq!(expected, Some(value));
            }
        }
        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
