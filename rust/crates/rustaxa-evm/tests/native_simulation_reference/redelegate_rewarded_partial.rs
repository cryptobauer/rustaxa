//! Reward-only same-H1 simulation over public finalized/reopened semantic P1.
//! The independent complete Go P1 reader supplies accounts/raw/root authority.
//! No production routing or Rust root-algorithm equality is claimed.
use super::*;
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
struct AuthPort<'a> {
    inner: mixed_native::SimulationNativeExecutionPort<'a>,
    reads: Rc<RefCell<Vec<ConcreteStorageKey>>>,
    drops: Rc<Cell<usize>>,
    fail: Option<usize>,
    corrupt: bool,
    outcomes: Rc<RefCell<Vec<NativeInvocationResult>>>,
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
    corrupt: bool,
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
            if self.corrupt {
                return Ok(ConcreteRead::Present(vec![0xff]));
            }
            return Err(NativeJournalReadError::Invariant(
                "signed H1 authentication failure".into(),
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
        self.inner.prepare(invocation, journal)
    }
    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        assert_eq!(invocation.period, 1.into());
        let result = self.inner.invoke(
            invocation,
            quote,
            &AuthJournal {
                inner: journal,
                reads: self.reads.clone(),
                fail: self.fail,
                corrupt: self.corrupt,
            },
        )?;
        self.outcomes.borrow_mut().push(result.clone());
        Ok(result)
    }
}

fn corpus() -> Value {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1_raw/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_reward_h1_raw/local.json")).unwrap();
    assert_eq!(public, local);
    public
}
pub(super) fn history(path: &FixturePath, initialize: bool, public: &Value) -> FinalChain {
    let storage = Arc::new(Storage::new(Config::new(path.0.clone())).unwrap());
    let balance = U256::from(3000);
    let caller: [u8; 20] = bytes(&public["caller"]).try_into().unwrap();
    let owner: [u8; 20] = bytes(&public["owner"]).try_into().unwrap();
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        500_000.into(),
        0,
        vec![
            GenesisAccount {
                address: caller,
                balance: rustaxa_types::FinalChainAccountBalance::new_account(balance),
            },
            GenesisAccount {
                address: owner,
                balance: rustaxa_types::FinalChainAccountBalance::new_account(U256::from(1000)),
            },
        ],
        [0x31, 0x32]
            .into_iter()
            .map(|last| GenesisValidator {
                address: address(last),
                vrf_key: [match last {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: U256::from(2000).to_big_endian().to_vec(),
                delegations: vec![(owner, U256::from(1000).to_big_endian().to_vec())]
                    .into_iter()
                    .chain(Some((caller, U256::from(1000).to_big_endian().to_vec())))
                    .collect(),
                metadata: GenesisValidatorMetadata {
                    owner: owner,
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
            aspen_max_supply: U256::from(8000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    if initialize {
        fn fields(stream: &mut RlpStream) {
            for value in 10..14 {
                stream.append(&H256::from_low_u64_be(value));
            }
            stream
                .append(&1_u64)
                .append(&1_700_000_001_u64)
                .begin_list(0);
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
        let (_, receipts) = chain
            .finalize_block_with_rewards_facts_at(
                pbft,
                vec![],
                vec![],
                1,
                vec![rustaxa_consensus::RewardCertVoteFact {
                    voter: ethereum_types::H160(address(0x31)),
                    weight: 1,
                    period: 1,
                }],
                1.into(),
            )
            .unwrap();
        assert!(receipts.is_empty());
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    chain
}

fn committed(chain: &FinalChain, public: &Value) -> Value {
    let accounts = public["state_before"]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let a: [u8; 20] = bytes(&row["address"]).try_into().unwrap();
            let account = chain.account(a).unwrap().unwrap();
            assert_eq!(
                account.balance.as_u256().to_string(),
                row["balance"].as_str().unwrap()
            );
            (hex::encode(a), format!("{account:?}"))
        })
        .collect::<Vec<_>>();
    let stakes = chain.dpos_validators_total_stakes(1.into()).unwrap();
    assert_eq!(stakes.len(), 2);
    for row in &stakes {
        assert_eq!(row.stake, U256::from(2000).to_big_endian().to_vec());
    }
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x0f, 0xa0]
    );
    json!({"accounts":accounts,"stakes":format!("{stakes:?}"),"header":hex::encode(chain.block_header(1.into()).unwrap().unwrap())})
}
#[test]
fn reward_only_h1_public_simulation_matches_go_and_discards_every_attempt() {
    let public = corpus();
    let concrete = FixturePath::new("reward-h1-concrete");
    let identity = materialize(&public, &concrete);
    let baseline = concrete_rows(&concrete);
    let reader = CompleteSeedReader::open(&concrete, &public, identity);
    let owner = FixturePath::new("reward-h1-owner");
    drop(history(&owner, true, &public));
    let chain = history(&owner, false, &public);
    let owner_before = committed(&chain, &public);
    let caller: [u8; 20] = bytes(&public["caller"]).try_into().unwrap();
    let block = ExecutionBlockContext {
        period: 1.into(),
        author: address(0x31),
        timestamp: 1700000001,
        gas_limit: 500000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    };
    let mut sessions = 0;
    for case in public["cases"].as_array().unwrap() {
        let mut plan = vec![(None, false), (None, false)];
        let mut attempt = 0;
        let mut expected_reads = None;
        while attempt < plan.len() {
            let (fail, corrupt) = plan[attempt];
            attempt += 1;
            sessions += 1;
            let reads = Rc::new(RefCell::new(Vec::new()));
            let drops = Rc::new(Cell::new(0));
            let outcomes = Rc::new(RefCell::new(Vec::new()));
            let supplied =
                FinalChainNonce::from_bytes(&(BigUint::from(1u8) << 512usize).to_bytes_be())
                    .unwrap();
            let request = ExecutionTransaction {
                position: 0.into(),
                hash: [0; 32],
                sender: caller,
                receiver: Some(address(0xfe)),
                nonce: supplied.clone(),
                gas_price: ExecutionGasPrice::default(),
                gas_limit: case["gas"].as_u64().unwrap().into(),
                value: ExecutionValue::new(number(&case["value"])),
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
                        fail,
                        corrupt,
                        outcomes: outcomes.clone(),
                    })
                },
                &block,
                &request,
                EnvelopeRules { cornus: true },
                TaraxaProfile::for_phase(TaraxaPhase::Ficus),
            );
            assert_eq!(drops.get(), 1);
            assert_eq!(request.nonce, supplied);
            assert_eq!(committed(&chain, &public), owner_before);
            assert_eq!(concrete_rows(&concrete), baseline);
            if fail.is_some() {
                assert!(result.is_err(), "fault {fail:?}");
                assert!(outcomes.borrow().is_empty());
                continue;
            }
            let simulated = result.unwrap();
            assert_eq!(simulated.state, identity);
            if let TransactionExecutionResult::ConsensusFailure(result) = &simulated.execution {
                assert_eq!(
                    result.error,
                    rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
                );
                assert_eq!(case["output"]["consensus_error"], "intrinsic gas too low");
                assert_eq!(
                    result.gas_used.as_u64(),
                    case["output"]["gas_used"].as_u64().unwrap()
                );
                assert!(result.output.is_empty());
                assert!(outcomes.borrow().is_empty());
                continue;
            }
            let TransactionExecutionResult::Executed(result) = simulated.execution else {
                panic!("admitted case")
            };
            let error = match &result.status {
                CodeExecutionStatus::Success => "",
                CodeExecutionStatus::Failure(CodeExecutionError::Native(e)) => e.error.as_str(),
                CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas",
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(error, case["output"]["execution_error"].as_str().unwrap());
            assert_eq!(
                result.gas_used.as_u64(),
                case["output"]["gas_used"].as_u64().unwrap()
            );
            assert_eq!(result.output, bytes(&case["output"]["return"]));
            let logs=result.logs.iter().map(|l|json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
            assert_eq!(logs, *case["output"]["logs"].as_array().unwrap());
            assert_eq!(case["output"]["effective_nonce"], "1");
            if case["name"] == "partial" {
                let keys = reads.borrow().clone();
                assert_eq!(keys.len(), 29);
                assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 29);
                if attempt == 1 {
                    for i in 0..keys.len() {
                        plan.extend([
                            (Some(i), false),
                            (None, false),
                            (Some(i), true),
                            (None, false),
                        ]);
                    }
                    expected_reads = Some(keys.clone());
                }
                assert_eq!(expected_reads.as_ref().unwrap(), &keys);
                let outcomes = outcomes.borrow();
                assert_eq!(outcomes.len(), 1);
                let NativeInvocationResult::Completed(out) = &outcomes[0] else {
                    panic!("funded")
                };
                let raw=out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
                assert_eq!(raw, *case["ordered_raw_writes"].as_array().unwrap());
                assert_eq!(out.account_mutations.len(), 2);
                for (mutation, (a, before, after)) in out
                    .account_mutations
                    .iter()
                    .zip([(address(0xfe), 4800, 4404), (caller, 3000, 3396)])
                {
                    assert!(
                        matches!(mutation,NativeOrdinaryAccountMutation::Balance{address,expected_exists:true,expected,replacement} if *address==a&&expected.value()==&num_bigint::BigInt::from(before)&&replacement.value()==&num_bigint::BigInt::from(after))
                    );
                }
            }
        }
    }
    assert_eq!(sessions, 136);
    drop(chain);
    let reopened = history(&owner, false, &public);
    assert_eq!(committed(&reopened, &public), owner_before);
    assert_eq!(concrete_rows(&concrete), baseline);
}
