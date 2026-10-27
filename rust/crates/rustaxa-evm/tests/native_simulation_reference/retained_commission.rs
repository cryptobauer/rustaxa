//! Retained commission parity over reopened complete Go seed rows.
use super::*;

pub(super) fn history(path: &FixturePath, initialize: bool, public: &Value) -> FinalChain {
    let storage = Arc::new(Storage::new(Config::new(path.0.clone())).unwrap());
    let balance = U256::from(1000);
    let caller: [u8; 20] = bytes(&public["delegator"]).try_into().unwrap();
    let owner: [u8; 20] = bytes(&public["owner"]).try_into().unwrap();
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        500_000.into(),
        0,
        rustaxa_consensus::FinalChainGenesisAllocation {
            accounts: vec![
                GenesisAccount {
                    address: caller,
                    balance: rustaxa_types::FinalChainAccountBalance::new_account(balance),
                },
                GenesisAccount {
                    address: owner,
                    balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(1000)),
                },
            ],
            validators: [0x31]
                .into_iter()
                .map(|last| GenesisValidator {
                    address: address(last),
                    vrf_key: [match last {
                        0x31 => 0x44,
                        0x32 => 0x55,
                        _ => 0x66,
                    }; 32],
                    total_stake: U256::from(1000).to_big_endian().to_vec(),
                    delegations: vec![(caller, U256::from(1000).to_big_endian().to_vec())],
                    metadata: GenesisValidatorMetadata {
                        owner,
                        commission: 100,
                        ..Default::default()
                    },
                })
                .collect(),
        },
        GenesisDposConfig {
            eligibility_balance_threshold: U256::from(100).into(),
            vote_eligibility_balance_step: U256::from(10).into(),
            validator_maximum_stake: U256::from(1_000_000).into(),
            minimum_deposit: U256::from(1).into(),
            delegation_delay: 1,
            commission_change_delta: 0,
            commission_change_frequency: 0,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            dpos_delegation_locking_period: 2,
            cornus_period: 0.into(),
            cornus_delegation_locking_period: 3,
            cacti_delegation_locking_period: 7,
            fix_redelegate_block_num: 0.into(),
            fix_claim_all_block_num: 0.into(),
            phalaenopsis_period: 0.into(),
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            yield_percentage: 20,
            committee_size: 1,
            max_block_author_reward_percent: 0,
            dag_proposers_reward_percent: 0,
            dpos_blocks_per_year: 1,
            aspen_max_supply: U256::from(3000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    if initialize {
        commit_period(&chain, &storage, 1, vec![]);
        let tx = FinalizationTransaction {
            hash: [0x52; 32],
            sender: caller,
            receiver: Some(address(0xfe)),
            nonce: FinalChainNonce::zero(),
            value: U256::zero().into(),
            gas_price: U256::zero().into(),
            gas_limit: 200000.into(),
            data: bytes(&public["provenance"][2]["input"]),
            rlp: vec![0xc0],
        };
        commit_period(&chain, &storage, 2, vec![tx]);
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), 2.into());
    chain
}

fn commit_period(
    chain: &FinalChain,
    storage: &Storage,
    period: u64,
    txs: Vec<FinalizationTransaction>,
) {
    let fields = |s: &mut RlpStream| {
        for value in 10..14 {
            s.append(&H256::from_low_u64_be(value));
        }
        s.append(&period)
            .append(&(1700000000 + period))
            .begin_list(0);
    };
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
    let mut data = RlpStream::new_list(4);
    data.append_raw(&pbft, 1)
        .begin_list(0)
        .begin_list(0)
        .begin_list(txs.len());
    for tx in &txs {
        data.append_raw(&tx.rlp, 1);
    }
    let mut batch = storage.create_write_batch();
    storage
        .batch_put_raw(
            &mut batch,
            Column::PeriodData,
            &period.to_le_bytes(),
            &data.out(),
        )
        .unwrap();
    storage.commit_write_batch_with_sync(batch, false).unwrap();
    let (_, receipts) = if period == 1 {
        chain
            .finalize_block_with_rewards_facts_at(
                pbft,
                txs,
                vec![],
                1,
                vec![rustaxa_consensus::RewardCertVoteFact {
                    voter: ethereum_types::H160(address(0x31)),
                    weight: 1,
                    period: 1,
                }],
                1.into(),
            )
            .unwrap()
    } else {
        chain.finalize_block(pbft, txs, vec![]).unwrap()
    };
    for receipt in receipts {
        assert_eq!(rlp::Rlp::new(&receipt).val_at::<u8>(0).unwrap(), 1);
    }
}

use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalAccount, NativeJournalRead, NativeJournalReadError, NativePortError,
};
use rustaxa_evm::contracts::{NativeOrdinaryAccountMutation, NativeRawOperation};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

/// Observe actual authentication reads, then inject each failure without changing
/// the committed reader. Every attempt owns and drops a fresh semantic session.
pub(super) struct AuthPort<'a> {
    pub(super) inner: mixed_native::SimulationNativeExecutionPort<'a>,
    pub(super) reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    pub(super) drops: Rc<Cell<usize>>,
    pub(super) fail: Option<usize>,
    pub(super) mode: u8,
    pub(super) outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
    pub(super) seen: Rc<RefCell<Vec<NativeInvocation>>>,
    pub(super) fail_prepare: bool,
    pub(super) identity: ConcreteStateIdentity,
}
impl Drop for AuthPort<'_> {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}
struct AuthJournal<'a> {
    inner: &'a dyn NativeJournalRead,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    fail: Option<usize>,
    mode: u8,
    identity: ConcreteStateIdentity,
}
impl NativeJournalRead for AuthJournal<'_> {
    fn account(&self, address: [u8; 20]) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.inner.account(address)
    }
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        let index = self.reads.borrow().len();
        self.reads.borrow_mut().push(*key);
        if self.fail == Some(index) {
            if self.mode == 2 {
                return Ok(ConcreteRead::Present(vec![0xff]));
            }
            if self.mode == 3 {
                return Ok(ConcreteRead::Absent);
            }
            return Err(NativeJournalReadError::State(
                ConcreteReadError::HistoryUnavailable(self.identity),
            ));
        }
        self.inner.raw_storage(address, key)
    }
}
impl NativeExecutionPort for AuthPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        self.seen.borrow_mut().push(invocation.clone());
        if self.fail_prepare {
            return Err(NativePortError::Infrastructure(
                "injected trace prepare".into(),
            ));
        }
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        assert_eq!(invocation.period, 2.into());
        let result = self.inner.invoke(
            invocation,
            quote,
            &AuthJournal {
                inner: journal,
                reads: self.reads.clone(),
                fail: self.fail,
                mode: self.mode,
                identity: self.identity,
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}

pub(super) fn corpus() -> Value {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_retained_commission/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_retained_commission/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    public
}

/// Public native reads must retain the complete validator and pending queue.
pub(super) fn committed(chain: &FinalChain, public: &Value) -> Value {
    let mut rows = Vec::new();
    for signature in ["getValidator(address)", "getUndelegations(address,uint32)"] {
        let mut input = keccak256(signature.as_bytes())[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&if signature.starts_with("getValidator") {
            address(0x31)
        } else {
            bytes(&public["delegator"]).try_into().unwrap()
        });
        if signature.starts_with("getUndelegations") {
            input.extend_from_slice(&[0; 32]);
        }
        let out = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 2.into(),
                sender: address(0xaa),
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1000000.into(),
                input,
            })
            .unwrap();
        assert!(out.code_err.is_empty() && out.consensus_err.is_empty());
        rows.push(hex::encode(out.code_retval));
    }
    let stakes = chain.dpos_validators_total_stakes(3.into()).unwrap();
    assert_eq!(stakes.len(), 1);
    assert_eq!(U256::from_big_endian(&stakes[0].stake), U256::zero());
    let accounts = public["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let account = chain
                .account(bytes(&row["address"]).try_into().unwrap())
                .unwrap()
                .unwrap();
            assert_eq!(
                account.balance.as_u256().to_string(),
                row["balance"].as_str().unwrap()
            );
            (row["address"].clone(), format!("{account:?}"))
        })
        .collect::<Vec<_>>();
    json!({"native":rows,"header":hex::encode(chain.block_header(2.into()).unwrap().unwrap()),"accounts":accounts})
}

#[test]
fn retained_commission_historical_parity_faults_retry_and_fresh_reopen() {
    let public = corpus();
    let concrete = FixturePath::new("retained-commission-concrete");
    let identity = materialize(&public, &concrete);
    let baseline = concrete_rows(&concrete);
    let app = FixturePath::new("retained-commission-owner");
    let mut successes = 0;
    let mut failures = 0;
    let mut original_committed = None;
    for restart in 0..2 {
        let chain = history(&app, restart == 0, &public);
        let before = committed(&chain, &public);
        if let Some(original) = &original_committed {
            assert_eq!(&before, original);
        } else {
            original_committed = Some(before.clone());
        }
        for case in public["simulation"].as_array().unwrap() {
            let caller = bytes(
                &public[if case["name"] == "positive" {
                    "owner"
                } else {
                    "delegator"
                }],
            )
            .try_into()
            .unwrap();
            // Reopen the actual persisted historical reader for every attempt.
            for mode in if case["name"] == "positive" {
                vec![0, 1, 0, 2, 0, 3, 0]
            } else {
                vec![0]
            } {
                let reader = CompleteSeedReader::open(&concrete, &public, identity);
                assert_eq!(reader.identity(), identity);
                for (key, value) in public["state_before"]["native_raw"].as_object().unwrap() {
                    assert_eq!(
                        reader
                            .storage(
                                address(0xfe),
                                ConcreteStorageKey(bytes(&json!(key)).try_into().unwrap())
                            )
                            .unwrap(),
                        ConcreteRead::Present(bytes(value))
                    );
                }
                let reads = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let outcomes = Rc::new(RefCell::new(Vec::new()));
                let seen = Rc::new(RefCell::new(Vec::new()));
                let nonce =
                    FinalChainNonce::from_bytes(&(BigUint::from(1u8) << 512usize).to_bytes_be())
                        .unwrap();
                let request = ExecutionTransaction {
                    position: 0.into(),
                    hash: [0; 32],
                    sender: caller,
                    receiver: Some(address(0xfe)),
                    nonce: nonce.clone(),
                    gas_price: ExecutionGasPrice::default(),
                    gas_limit: 200000.into(),
                    value: ExecutionValue::default(),
                    input: bytes(&case["input"]),
                    canonical_rlp: None,
                    kind: ExecutionTransactionKind::Call,
                };
                let result = simulate_with_native(
                    &reader,
                    &NoHistory,
                    &Dpos,
                    &Dpos,
                    |selected| {
                        assert_eq!(selected, identity);
                        Ok(AuthPort {
                            inner: mixed_native::SimulationNativeExecutionPort::new(
                                chain.begin_native_simulation(selected.period).unwrap(),
                            ),
                            reads: reads.clone(),
                            drops: drops.clone(),
                            outcomes: outcomes.clone(),
                            seen: seen.clone(),
                            fail: if mode == 0 { None } else { Some(0) },
                            mode,
                            fail_prepare: false,
                            identity,
                        })
                    },
                    &ExecutionBlockContext {
                        period: 2.into(),
                        author: address(0x31),
                        timestamp: 1700000002,
                        gas_limit: 1000000.into(),
                        chain_id: 666,
                        difficulty: BigUint::default(),
                    },
                    &request,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                assert_eq!(drops.get(), 1);
                assert_eq!(request.nonce, nonce);
                assert_eq!(committed(&chain, &public), before);
                assert_eq!(concrete_rows(&concrete), baseline);
                if mode != 0 {
                    failures += 1;
                    assert!(result.is_err());
                    if mode == 1 {
                        assert!(format!("{result:?}").contains("HistoryUnavailable"));
                    }
                    assert!(outcomes.borrow().is_empty());
                    assert_eq!(reads.borrow().len(), 1);
                    continue;
                }
                successes += 1;
                let result = result.unwrap();
                assert_eq!(result.state, identity);
                let TransactionExecutionResult::Executed(execution) = result.execution else {
                    panic!("executed claim")
                };
                let error = match &execution.status {
                    CodeExecutionStatus::Success => "",
                    CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) => e.error.as_str(),
                    s => panic!("{s:?}"),
                };
                assert_eq!(error, case["output"]["execution_error"].as_str().unwrap());
                assert_eq!(
                    execution.gas_used.as_u64(),
                    case["output"]["gas_used"].as_u64().unwrap()
                );
                assert_eq!(execution.output, bytes(&case["output"]["return"]));
                let logs=execution.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
                assert_eq!(logs, *case["output"]["logs"].as_array().unwrap());
                let outcomes = outcomes.borrow();
                assert_eq!(outcomes.len(), 1);
                let NativeInvocationResult::Completed(out) = &outcomes[0] else {
                    panic!("completed")
                };
                if case["name"] == "positive" {
                    assert_eq!(reads.borrow().len(), 1);
                    let raw=out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
                    assert_eq!(
                        raw,
                        *public["cases"][1]["ordered_raw_writes"].as_array().unwrap()
                    );
                    assert_eq!(out.account_mutations.len(), 2);
                    for (m, (a, prior, next)) in out
                        .account_mutations
                        .iter()
                        .zip([(address(0xfe), 1002, 1000), (caller, 1000, 1002)])
                    {
                        assert!(
                            matches!(m,NativeOrdinaryAccountMutation::Balance{address,expected_exists:true,expected,replacement} if *address==a&&expected.value()==&num_bigint::BigInt::from(prior)&&replacement.value()==&num_bigint::BigInt::from(next))
                        );
                    }
                } else {
                    assert!(reads.borrow().is_empty());
                    assert!(out.raw_mutations.is_empty());
                    assert!(out.account_mutations.is_empty());
                }
            }
        }
    }
    assert_eq!((successes, failures), (10, 6));
    assert_eq!(concrete_rows(&concrete), baseline);
}
