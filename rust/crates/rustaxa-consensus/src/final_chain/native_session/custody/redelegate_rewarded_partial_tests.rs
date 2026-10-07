//! Actual reward-kernel composition and finite rewarded-partial raw/account parity.
//! These tests do not publish P1 or claim commit/reopen or mixed-effect ordering.
use super::*;
use ethereum_types::H160;
use num_bigint::BigInt;

fn reward_chain() -> (FinalChain, Arc<Storage>, std::path::PathBuf) {
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
            yield_percentage: 20,
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    (chain, storage, path)
}

fn reward_case() -> Value {
    let public: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_reward/public.json"
    )))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/native_redelegate_reward/local.json"
    )))
    .unwrap();
    assert_eq!(public, local);
    public["cases"][0].clone()
}
struct RewardState {
    physical: Vec<Value>,
    period: u64,
    overlay: RefCell<BTreeMap<ConcreteStorageKey, ConcreteRead<Vec<u8>>>>,
    accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
    reads: RefCell<Vec<ConcreteStorageKey>>,
    account_reads: RefCell<Vec<[u8; 20]>>,
    fail_raw: Option<usize>,
    corrupt_raw: Option<ConcreteStorageKey>,
    missing_raw: Option<ConcreteStorageKey>,
    fail_account: Option<[u8; 20]>,
}
impl RewardState {
    fn new(case: &Value, period: u64) -> Self {
        let (rows, accounts) = if period == 0 {
            ("genesis_rows", "reward_accounts_before")
        } else {
            ("reward_committed_rows", "reward_committed_accounts")
        };
        Self {
            physical: case[rows].as_array().unwrap().clone(),
            period,
            overlay: RefCell::new(BTreeMap::new()),
            accounts: case[accounts]
                .as_object()
                .unwrap()
                .iter()
                .map(|(a, row)| {
                    (
                        unhex(a).try_into().unwrap(),
                        FinalChainNativeAccount {
                            exists: true,
                            nonce: row["nonce"]
                                .as_str()
                                .unwrap()
                                .parse::<u64>()
                                .unwrap()
                                .into(),
                            balance: row["balance"].as_str().unwrap().parse().unwrap(),
                        },
                    )
                })
                .collect(),
            reads: RefCell::new(Vec::new()),
            account_reads: RefCell::new(Vec::new()),
            fail_raw: None,
            corrupt_raw: None,
            missing_raw: None,
            fail_account: None,
        }
    }
    fn physical_read(&self, contract: [u8; 20], key: ConcreteStorageKey) -> ConcreteRead<Vec<u8>> {
        let prefix = hex(&rustaxa_storage::storage_version_prefix(contract, key));
        self.physical
            .iter()
            .filter(|row| {
                row["column"] == 4
                    && row["key"].as_str().unwrap().len() == 80
                    && row["key"].as_str().unwrap().starts_with(&prefix)
            })
            .filter(|row| {
                u64::from_str_radix(&row["key"].as_str().unwrap()[64..], 16).unwrap() <= self.period
            })
            .max_by_key(|row| row["key"].as_str().unwrap())
            .map_or(ConcreteRead::Absent, |row| {
                ConcreteRead::Present(unhex(row["value"].as_str().unwrap()))
            })
    }
    fn apply_raw(&self, mutations: &[FinalChainNativeRawMutation]) {
        for m in mutations {
            assert_eq!(self.raw_storage(m.address, &m.key).unwrap(), m.expected);
            self.overlay.borrow_mut().insert(
                m.key,
                match &m.operation {
                    FinalChainNativeRawOperation::Put(v) => {
                        ConcreteRead::Present(v.as_bytes().to_vec())
                    }
                    FinalChainNativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
                },
            );
        }
    }
}
impl FinalChainNativeStateRead for RewardState {
    fn raw_storage(
        &self,
        contract: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        self.reads.borrow_mut().push(*key);
        if self.fail_raw == Some(self.reads.borrow().len()) {
            return Err(FinalChainNativeStateReadError::Invariant(
                "injected reward raw failure".into(),
            ));
        }
        if self.missing_raw == Some(*key) {
            return Ok(ConcreteRead::Absent);
        }
        if self.corrupt_raw == Some(*key) {
            return Ok(ConcreteRead::Present(vec![0xff]));
        }
        Ok(self
            .overlay
            .borrow()
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.physical_read(contract, *key)))
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        self.account_reads.borrow_mut().push(address);
        if self.fail_account == Some(address) {
            return Err(FinalChainNativeStateReadError::Invariant(
                "injected reward account failure".into(),
            ));
        }
        self.accounts
            .get(&address)
            .cloned()
            .ok_or_else(|| FinalChainNativeStateReadError::Invariant("unavailable account".into()))
    }
}
fn reward_setup<'a>(
    chain: &'a FinalChain,
    case: &Value,
) -> (FinalChainNativeSession<'a>, RewardState) {
    let id = [0x81; 32];
    let mut session = chain
        .begin_native_session_bound(id, 1.into(), 0.into())
        .unwrap();
    let genesis = RewardState::new(case, 0);
    let plan = chain
        .plan_external_evm_rewards_stats(
            id,
            FinalizedRewardsPeriodFact {
                period: 1,
                block_author: H160::from(address(0x31)),
                blocks_per_year: 1,
                dpos_eligible_total_vote_count: 0,
                transactions: Vec::new(),
                dag_blocks: Vec::new(),
                cert_votes: vec![RewardCertVoteFact {
                    voter: H160::from(address(0x31)),
                    weight: 1,
                    period: 1,
                }],
            },
        )
        .unwrap();
    let reward = session.finish_rewards(&plan, &genesis).unwrap();
    assert_eq!(reward.total_reward.as_u256(), U256::from(800));
    assert_eq!(
        write_json(&reward.raw_mutations),
        case["reward_ordered_raw_writes"]
    );
    assert_eq!(reward.account_mutations.len(), 1);
    assert!(
        matches!(&reward.account_mutations[0],FinalChainNativeOrdinaryMutation::BalanceReplace{address:a,expected,replacement,..} if *a==DPOS_CONTRACT_ADDRESS && expected==&BigInt::from(4000) && replacement==&BigInt::from(4800))
    );
    genesis.apply_raw(&reward.raw_mutations);
    let state = RewardState::new(case, 1);
    for key in genesis.overlay.borrow().keys() {
        assert_eq!(
            genesis.raw_storage(DPOS_CONTRACT_ADDRESS, key).unwrap(),
            state.raw_storage(DPOS_CONTRACT_ADDRESS, key).unwrap()
        );
    }
    // Advance the real, complete unpublished reward-kernel snapshot. This grants
    // no historical/publication authority; the P2 reader is independently Go P1.
    chain
        .advance_reward_reference_graph_block(&mut session.dpos_state, 2.into())
        .unwrap();
    session.pending_period = 2.into();
    session.eligibility_period = 1.into();
    session.eligibility_state = session.dpos_state.clone();
    session.period_start_total_vote_count = session.dpos_state.total_vote_count;
    session.period_start_amount_delegated = total_staked_amount(&session.dpos_state).unwrap();
    session.request_id = None;
    session.finished_rewards = false;
    session.next_sequence = 0;
    session.prepared = None;
    session.aborted = false;
    state.reads.borrow_mut().clear();
    (session, state)
}
fn reward_request(case: &Value) -> FinalChainNativeRequest {
    let mut request = staged_request(case, 0);
    request.period = 2.into();
    request.input = unhex(case["attempts"][0]["input"].as_str().unwrap());
    request
}
fn reward_invoke(
    session: &mut FinalChainNativeSession<'_>,
    state: &RewardState,
    request: &FinalChainNativeRequest,
) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
    let quote = session.prepare(request, state)?;
    session.invoke(request, quote, state)
}
fn assert_reward_outcome(out: &FinalChainNativeOutcome, case: &Value) {
    let a = &case["attempts"][0];
    assert_eq!(out.status, FinalChainNativeStatus::Success);
    assert_eq!(out.gas_used, FinalChainGas::new(80000));
    assert!(out.output.is_empty());
    assert_eq!(write_json(&out.raw_mutations), a["ordered_raw_writes"]);
    assert_eq!(log_json(&out.logs), a["logs"]);
    assert_eq!(out.account_mutations.len(), 2);
    for (m, (account, before, after)) in out.account_mutations.iter().zip([
        (DPOS_CONTRACT_ADDRESS, 4800, 4404),
        (address(0xd1), 3000, 3396),
    ]) {
        assert!(
            matches!(m,FinalChainNativeOrdinaryMutation::BalanceReplace{address:a,expected_exists:true,expected,replacement} if *a==account&&*expected==BigInt::from(before)&&*replacement==BigInt::from(after))
        );
    }
}
#[test]
fn rewarded_partial_kernel_and_staged_vectors_match_actual_go() {
    let case = reward_case();
    let (chain, storage, path) = reward_chain();
    for _ in 0..2 {
        let (mut session, state) = reward_setup(&chain, &case);
        let before = session.dpos_state.clone();
        let out = completed(reward_invoke(&mut session, &state, &reward_request(&case)).unwrap());
        assert_reward_outcome(&out, &case);
        assert_eq!(
            *state.account_reads.borrow(),
            [DPOS_CONTRACT_ADDRESS, address(0xd1)]
        );
        assert_eq!(session.next_sequence, 1);
        assert!(!session.aborted);
        assert_eq!(
            delegation(&session.dpos_state, address(0x31), address(0xd1)),
            Some(U256::from(700))
        );
        assert_eq!(
            delegation(&session.dpos_state, address(0x32), address(0xd1)),
            Some(U256::from(1300))
        );
        for validator in [address(0x31), address(0x32)] {
            assert_eq!(
                session.dpos_state.delegations[&validator][&address(0xa1)],
                before.delegations[&validator][&address(0xa1)]
            );
            assert_eq!(
                session
                    .dpos_state
                    .reward_reference_graph
                    .read_cursor(&validator, &address(0xa1))
                    .unwrap(),
                0
            );
            assert_eq!(
                session
                    .dpos_state
                    .reward_reference_graph
                    .load_node(&NodeKey {
                        validator,
                        block: 0
                    })
                    .unwrap()
                    .count,
                1
            );
        }
        state.apply_raw(&out.raw_mutations);
        for (key, row) in case["attempts"][0]["raw_after"].as_object().unwrap() {
            let actual = state
                .raw_storage(
                    DPOS_CONTRACT_ADDRESS,
                    &ConcreteStorageKey(unhex(key).try_into().unwrap()),
                )
                .unwrap();
            let bytes = match actual {
                ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
                ConcreteRead::Present(v) => v,
            };
            assert_eq!(!bytes.is_empty(), row["present"] == true);
            assert_eq!(hex(&bytes), row["value"]);
        }
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn rewarded_partial_all_raw_and_account_faults_abort_and_fresh_retry_matches() {
    let case = reward_case();
    let (chain, storage, path) = reward_chain();
    let (mut session, state) = reward_setup(&chain, &case);
    let request = reward_request(&case);
    let out = completed(reward_invoke(&mut session, &state, &request).unwrap());
    assert_reward_outcome(&out, &case);
    let reads = state.reads.borrow().len();
    let keys = state
        .reads
        .borrow()
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let present=keys.iter().copied().filter(|key|matches!(state.physical_read(DPOS_CONTRACT_ADDRESS,*key),ConcreteRead::Present(ref v) if !v.is_empty())).collect::<Vec<_>>();
    let raw_faults = reads + keys.len() + present.len();
    let mut caller_authority_failures_after_debit = 0;
    for fault in 0..raw_faults + 4 {
        let (mut session, mut state) = reward_setup(&chain, &case);
        let before = session.dpos_state.clone();
        let accounts = state.accounts.clone();
        if fault < reads {
            state.fail_raw = Some(fault + 1)
        } else if fault < reads + keys.len() {
            state.corrupt_raw = Some(*keys.iter().nth(fault - reads).unwrap())
        } else if fault < raw_faults {
            state.missing_raw = Some(present[fault - reads - keys.len()]);
        } else {
            match fault - raw_faults {
                0 => state.fail_account = Some(DPOS_CONTRACT_ADDRESS),
                1 => state.fail_account = Some(address(0xd1)),
                2 => {
                    state
                        .accounts
                        .get_mut(&DPOS_CONTRACT_ADDRESS)
                        .unwrap()
                        .balance = BigInt::from(395)
                }
                _ => state.accounts.get_mut(&address(0xd1)).unwrap().exists = false,
            }
        }
        assert!(
            reward_invoke(&mut session, &state, &request).is_err(),
            "fault{fault}"
        );
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted && session.prepared.is_none());
        assert_eq!(session.next_sequence, 0);
        assert!(state.overlay.borrow().is_empty());
        if state.account_reads.borrow().len() == 2 {
            caller_authority_failures_after_debit += 1;
        }
        // Reader facts are never mutated; scratch balances are discarded on errors.
        if fault < raw_faults + 2 {
            assert_eq!(state.accounts, accounts);
        }
        assert!(reward_invoke(&mut session, &state, &request).is_err());
        let (mut retry, clean) = reward_setup(&chain, &case);
        assert_reward_outcome(
            &completed(reward_invoke(&mut retry, &clean, &request).unwrap()),
            &case,
        );
    }
    assert_eq!(caller_authority_failures_after_debit, 2);
    println!(
        "rewarded-partial {reads} reader faults, {} raw corruptions,{} missingrows,4accountfaults;all fresh retries;{caller_authority_failures_after_debit} caller authority failures after staged native debit",
        keys.len(),
        present.len()
    );
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn rewarded_partial_normal_preflight_failure_never_reads_accounts() {
    let case = reward_case();
    let (chain, storage, path) = reward_chain();
    for amount in [1001u64, 950] {
        let (mut session, state) = reward_setup(&chain, &case);
        let before = session.dpos_state.clone();
        let mut request = reward_request(&case);
        request.input[68..100].copy_from_slice(&U256::from(amount).to_big_endian());
        let out = completed(reward_invoke(&mut session, &state, &request).unwrap());
        assert!(matches!(
            out.status,
            FinalChainNativeStatus::ContractFailure { .. }
        ));
        assert!(
            out.raw_mutations.is_empty() && out.account_mutations.is_empty() && out.logs.is_empty()
        );
        assert!(state.account_reads.borrow().is_empty());
        assert_eq!(session.dpos_state, before);
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn rewarded_partial_excludes_unmeasured_shapes_without_accounts_or_state_advancement() {
    let case = reward_case();
    let (mut chain, storage, path) = reward_chain();
    for shape in 0..22 {
        let (mut session, state) = reward_setup(&chain, &case);
        let mut request = reward_request(&case);
        match shape {
            0 => session.dpos_state.delegation_ledger_history_complete = false,
            1 => {
                session
                    .dpos_state
                    .redelegate_same_validator_history_complete = false
            }
            2 => {
                session
                    .dpos_state
                    .redelegate_same_validator_corruption
                    .insert(address(0x31));
            }
            3 => session.pending_period = 3.into(),
            4 => session.pending_period = 1.into(),
            5 => request.input[68..100].copy_from_slice(&U256::from(1000).to_big_endian()),
            6 => request.input[68..100].fill(0),
            7 => {
                session.dpos_state.delegator_rewards.insert(
                    address(0x32),
                    StoredDposTokenAmount::canonical_after_mutation(U256::from(1).into()),
                );
            }
            8 => {
                session.dpos_state.commission_rewards.insert(
                    address(0x32),
                    StoredDposTokenAmount::canonical_after_mutation(U256::from(1).into()),
                );
            }
            9 => {
                session.dpos_state.commission_rewards.remove(&address(0x31));
            }
            10 => {
                session
                    .dpos_state
                    .validator_reward_per_stake
                    .insert(address(0x31), StoredDposRewardIndex::default());
            }
            11 => {
                session
                    .dpos_state
                    .delegation_reward_cursors
                    .get_mut(&address(0x31))
                    .unwrap()
                    .remove(&address(0xa1));
            }
            12 => {
                session
                    .dpos_state
                    .validator_metadata
                    .get_mut(&address(0x32))
                    .unwrap()
                    .owner = address(0xa2)
            }
            13 => {
                session
                    .dpos_state
                    .validator_metadata
                    .get_mut(&address(0x31))
                    .unwrap()
                    .owner = address(0xd1)
            }
            14 => session
                .dpos_state
                .delegator_validators
                .get_mut(&address(0xa1))
                .unwrap()
                .reverse(),
            15 => session.dpos_state.validator_order.reverse(),
            16 => {
                session
                    .dpos_state
                    .undelegations
                    .entry(address(0xa1))
                    .or_default();
            }
            17 => {
                session
                    .dpos_state
                    .undelegations_v2
                    .entry(address(0xa1))
                    .or_default();
            }
            18 => {
                session
                    .dpos_state
                    .undelegation_v2_last_ids
                    .insert(address(0xa1), 1);
            }
            19 => {
                session
                    .dpos_state
                    .slashing_jail_blocks
                    .insert(address(0x31), 0);
            }
            20 => {
                session
                    .dpos_state
                    .slashing_double_voting_proofs
                    .insert([0x81; 32]);
            }
            _ => {
                session.dpos_state.delegator_rewards.insert(
                    address(0x33),
                    StoredDposTokenAmount::canonical_after_mutation(U256::from(1).into()),
                );
            }
        }
        request.period = session.pending_period;
        let before = session.dpos_state.clone();
        assert!(
            reward_invoke(&mut session, &state, &request).is_err(),
            "shape{shape}"
        );
        assert_eq!(session.dpos_state, before);
        assert!(session.aborted && session.prepared.is_none());
        assert_eq!(session.next_sequence, 0);
        assert!(state.account_reads.borrow().is_empty() && state.overlay.borrow().is_empty());
    }
    for fork in 0..5 {
        let (session, state) = reward_setup(&chain, &case);
        let before = session.dpos_state.clone();
        drop(session);
        let original = chain.rewards_config.clone();
        let ficus = chain.ficus_activation_period;
        match fork {
            0 => chain.rewards_config.magnolia_period = FinalChainBlockNumber::MAX,
            1 => chain.rewards_config.aspen_part_one_period = FinalChainBlockNumber::MAX,
            2 => chain.rewards_config.aspen_part_two_period = 2.into(),
            3 => chain.rewards_config.fix_redelegate_block_num = 2.into(),
            _ => chain.ficus_activation_period = FinalChainBlockNumber::MAX,
        }
        let mut session = chain.begin_native_session(1.into(), 0.into()).unwrap();
        session.pending_period = 2.into();
        session.dpos_state = before.clone();
        let result = reward_invoke(&mut session, &state, &reward_request(&case));
        assert!(result.is_err(), "fork{fork}");
        assert_eq!(session.dpos_state, before);
        assert!(state.account_reads.borrow().is_empty());
        drop(session);
        chain.rewards_config = original;
        chain.ficus_activation_period = ficus;
    }
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}

/// A deliberately malformed working clone fails destination serialization
/// after the real kernel completed payout. This tests the codec composition,
/// not a late reader failure through invoke (all its raw keys are cached).
#[test]
fn rewarded_partial_destination_codec_error_after_completed_payout_discards_scratch() {
    let case = reward_case();
    let (chain, storage, path) = reward_chain();
    let (session, state) = reward_setup(&chain, &case);
    let before = session.dpos_state.clone();
    let original_accounts = state.accounts.clone();
    let mut next = before.clone();
    let mut accounts = StagedDposAccountPort::from_state(&state);
    let outcome = chain
        .apply_dpos_redelegate(
            &mut next,
            &mut accounts,
            address(0xd1),
            address(0x31),
            address(0x32),
            U256::from(300).to_big_endian().to_vec(),
            2.into(),
        )
        .unwrap();
    assert_eq!(outcome.status_code, 1);
    assert_eq!(accounts.into_mutations().len(), 2);
    assert_eq!(
        log_json(
            &outcome
                .logs
                .iter()
                .map(|log| FinalChainCallLog {
                    address: log.address,
                    topics: log.topics.clone(),
                    data: log.data.clone()
                })
                .collect::<Vec<_>>()
        ),
        case["attempts"][0]["logs"]
    );
    // Corrupt only the disposable kernel result, not seed/history authority.
    next.delegations
        .get_mut(&address(0x32))
        .unwrap()
        .remove(&address(0xd1));
    let mut trace = FinalChainNativeRawTrace::new(&state);
    session
        .serialize_undelegate_principal(address(0xd1), address(0x31), &before, &next, &mut trace)
        .unwrap();
    assert!(
        session
            .serialize_delegate(address(0xd1), address(0x32), &before, &next, &mut trace)
            .is_err()
    );
    let scratch = trace.finish();
    assert_eq!(
        write_json(&scratch),
        serde_json::Value::Array(
            case["attempts"][0]["ordered_raw_writes"]
                .as_array()
                .unwrap()[..8]
                .to_vec()
        )
    );
    assert_eq!(session.dpos_state, before);
    assert_eq!(session.next_sequence, 0);
    assert!(state.overlay.borrow().is_empty());
    assert_eq!(state.accounts, original_accounts);
    drop(session);
    drop(chain);
    drop(storage);
    std::fs::remove_dir_all(path).unwrap();
}
