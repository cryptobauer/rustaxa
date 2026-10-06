//! Same-height simulation over actual signed-prefix committed H1, independently
//! built through public Rust finalization. Concrete Go root remains separate
//! authority; public semantic owner is not hydrated from physical native rows.
use super::*;
use rustaxa_evm::contracts::{
    NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationResult,
    NativeJournalAccount, NativeJournalRead, NativeJournalReadError, NativePortError,
};
use rustaxa_types::LegacyTransactionEnvelope;
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
        assert_eq!(quote.required_gas.as_u64(), 80000);
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
        if let NativeInvocationResult::Completed(outcome) = &result {
            // Internal effect witness uses the accepted source-last contract;
            // the Go DryRunner oracle measures external output, not raw writes.
            assert!(outcome.account_mutations.is_empty());
            assert_eq!(outcome.raw_mutations.len(), 17);
            let key = |parts: &[&[u8]]| ConcreteStorageKey(keccak256(parts.concat()).0);
            let membership = [&[2, 1][..], &invocation.caller].concat();
            for retained in [
                key(&[&membership, &[2], &1_u32.to_le_bytes()]),
                key(&[&membership, &[2], &address(0x33)]),
            ] {
                assert!(outcome.raw_mutations.iter().all(|m| m.key != retained));
            }
            let source = key(&[&[1], &address(0x31), &[1]]);
            let values = outcome
                .raw_mutations
                .iter()
                .filter(|m| m.key == source)
                .map(|m| match &m.operation {
                    rustaxa_evm::contracts::NativeRawOperation::Put(value) => {
                        hex::encode(value.as_bytes())
                    }
                    rustaxa_evm::contracts::NativeRawOperation::Delete => String::new(),
                })
                .collect::<Vec<_>>();
            assert_eq!(values, ["c28001"]);
        }
        Ok(result)
    }
}

pub(super) fn corpus() -> Value {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_source_last_absent_simulation/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_source_last_absent_simulation/local.json")).unwrap();
    assert_eq!(public, local);
    public
}
pub(super) fn caller(public: &Value) -> [u8; 20] {
    let key = SigningKey::from_slice(&[0x31; 32]).unwrap();
    let pubkey = key.verifying_key().to_encoded_point(false);
    let result: [u8; 20] = keccak256(&pubkey.as_bytes()[1..]).0[12..]
        .try_into()
        .unwrap();
    assert_eq!(hex::encode(result), public["caller"]);
    result
}
/// Canonical signed public envelope supplies all recovered finalization metadata.
fn prefix_transaction(public: &Value) -> FinalizationTransaction {
    let fixture = &public["prefix"]["signed_transaction"];
    let raw = bytes(&fixture["rlp"]);
    let tx = LegacyTransactionEnvelope::decode(&raw).unwrap();
    assert!(tx.signature_valid && tx.intrinsic_gas_covered);
    assert_eq!(tx.chain_id, 666);
    assert_eq!(tx.sender.unwrap().0, caller(public));
    assert_eq!(hex::encode(tx.hash.0), fixture["hash"]);
    assert_eq!(tx.rlp, raw);
    assert_eq!(tx.nonce, U256::zero());
    assert_eq!(tx.gas_price, U256::zero());
    assert_eq!(tx.value, U256::zero());
    assert_eq!(tx.gas, 200000);
    assert_eq!(tx.receiver.unwrap().0, caller(public));
    assert_eq!(tx.data, bytes(&fixture["input"]));
    assert!(tx.data.is_empty());
    FinalizationTransaction {
        hash: tx.hash.0,
        sender: tx.sender.unwrap().0,
        receiver: tx.receiver.map(|a| a.0),
        nonce: FinalChainNonce::zero(),
        value: tx.value.into(),
        gas_price: tx.gas_price.into(),
        gas_limit: tx.gas.into(),
        data: tx.data,
        rlp: raw,
    }
}
pub(super) fn history(path: &FixturePath, initialize: bool, public: &Value) -> FinalChain {
    let storage = Arc::new(Storage::new(Config::new(path.0.clone())).unwrap());
    let balance = U256::from(3000);
    let caller = caller(public);
    let chain = FinalChain::new_with_rewards_config_and_ficus_activation(
        storage.clone(),
        500_000.into(),
        0,
        vec![GenesisAccount {
            address: caller,
            balance: rustaxa_types::FinalChainAccountBalance::new_account(balance),
        }],
        [0x33, 0x31, 0x32]
            .into_iter()
            .map(|last| GenesisValidator {
                address: address(last),
                vrf_key: [match last {
                    0x31 => 0x44,
                    0x32 => 0x55,
                    _ => 0x66,
                }; 32],
                total_stake: U256::from(if last != 0x32 { 2000 } else { 1000 })
                    .to_big_endian()
                    .to_vec(),
                delegations: vec![(address(0xa1), U256::from(1000).to_big_endian().to_vec())]
                    .into_iter()
                    .chain(
                        (last != 0x32)
                            .then_some((caller, U256::from(1000).to_big_endian().to_vec())),
                    )
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
            commission_change_delta: 0,
            commission_change_frequency: 0,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: 0.into(),
            cornus_period: 0.into(),
            fix_redelegate_block_num: 0.into(),
            fix_claim_all_block_num: 0.into(),
            phalaenopsis_period: 0.into(),
            aspen_part_one_period: 0.into(),
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            yield_percentage: 0,
            dpos_blocks_per_year: 1,
            aspen_max_supply: U256::from(8000).into(),
            ..Default::default()
        },
        0.into(),
    )
    .unwrap();
    if initialize {
        assert_eq!(
            chain.last_block_number_typed().unwrap(),
            FinalChainBlockNumber::GENESIS
        );
        let tx = prefix_transaction(public);
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
            .begin_list(1)
            .append_raw(&tx.rlp, 1);
        let mut batch = storage.create_write_batch();
        storage
            .batch_put_raw(
                &mut batch,
                Column::PeriodData,
                &1_u64.to_le_bytes(),
                &period.out(),
            )
            .unwrap();
        storage.commit_write_batch_with_sync(batch, false).unwrap();
        let (_, receipts) = chain.finalize_block(pbft, vec![tx], vec![]).unwrap();
        assert_eq!(receipts.len(), 1);
        let receipt = rlp::Rlp::new(&receipts[0]);
        assert_eq!(receipt.val_at::<u8>(0).unwrap(), 1);
        assert_eq!(
            receipt.val_at::<u64>(1).unwrap(),
            public["prefix"]["receipt"]["gas_used"].as_u64().unwrap()
        );
        assert_eq!(receipt.at(3).unwrap().item_count().unwrap(), 0);
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x13, 0x88]
    );
    chain
}

/// Public committed reads bind exact pairs/order/rewards and balances, not roots.
pub(super) fn assert_committed(chain: &FinalChain, public: &Value) {
    let caller = caller(public);
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x13, 0x88]
    );
    let stakes = chain.dpos_validators_total_stakes(1.into()).unwrap();
    assert_eq!(stakes.len(), 3);
    for (row, (last, stake)) in stakes
        .iter()
        .zip([(0x31, 2000), (0x32, 1000), (0x33, 2000)])
    {
        assert_eq!(row.address, address(last));
        // This public stake API deliberately uses the delegation delay.
        assert_eq!(
            U256::from_big_endian(&row.stake),
            U256::from(if last == 0x32 { 1000 } else { 2000 })
        );
        let mut input = keccak256(b"getValidator(address)")[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&address(last));
        let current = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 1.into(),
                sender: caller,
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1000000.into(),
                input,
            })
            .unwrap();
        assert!(current.code_retval.len() >= 64);
        assert_eq!(
            U256::from_big_endian(&current.code_retval[32..64]),
            U256::from(stake)
        );
    }
    for (owner, members) in [
        (caller, vec![0x33, 0x31]),
        (address(0xa1), vec![0x33, 0x31, 0x32]),
    ] {
        let mut input = keccak256(b"getDelegations(address,uint32)")[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&owner);
        input.extend_from_slice(&[0; 32]);
        let result = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 1.into(),
                sender: owner,
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1000000.into(),
                input,
            })
            .unwrap();
        assert_eq!(result.code_retval.len(), 96 + 96 * members.len());
        assert_eq!(
            U256::from_big_endian(&result.code_retval[64..96]),
            U256::from(members.len())
        );
        for (row, last) in result.code_retval[96..]
            .as_chunks::<96>()
            .0
            .iter()
            .zip(members)
        {
            assert_eq!(&row[12..32], &address(last));
            let amount = 1000;
            assert_eq!(U256::from_big_endian(&row[32..64]), U256::from(amount));
            assert_eq!(U256::from_big_endian(&row[64..96]), U256::zero());
        }
    }
    let sender = chain.account_at_block(1.into(), caller).unwrap().unwrap();
    assert_eq!(sender.nonce, FinalChainNonce::from_u64(1));
    assert_eq!(*sender.balance.as_u256(), U256::from(3000));
    assert!(
        chain
            .account_at_block(1.into(), address(0xa1))
            .unwrap()
            .is_none()
    );
    let native = chain
        .account_at_block(1.into(), address(0xfe))
        .unwrap()
        .unwrap();
    assert_eq!(native.nonce, FinalChainNonce::from_u64(1));
    assert_eq!(*native.balance.as_u256(), U256::from(5000));
}
#[test]
fn signed_source_last_absent_same_height_simulation_matches_go_and_reopens() {
    let public = corpus();
    let caller = caller(&public);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(public["prefix"]["before"], public["prefix"]["after_prefix"]);
    assert_eq!(
        public["prefix"]["native_account_before"],
        public["prefix"]["native_account_after"]
    );
    assert_eq!(
        public["prefix"]["after_prefix"],
        public["prefix"]["after_end_block"]
    );
    assert_eq!(
        public["prefix"]["after_end_block"],
        public["prefix"]["after_commit"]
    );
    let concrete = FixturePath::new("signed-source-last-absent-concrete");
    let app = FixturePath::new("signed-source-last-absent-app");
    let identity = materialize(&public, &concrete);
    assert_eq!(identity.period, 1.into());
    assert_eq!(
        hex::encode(identity.state_root),
        public["state_before"]["root"]
    );
    assert_eq!(
        public["state_before"]["seed_rows"]
            .as_array()
            .unwrap()
            .len(),
        100
    );
    let before = concrete_rows(&concrete);
    let mut sessions = 0;
    for restart in 0..2 {
        let chain = history(&app, restart == 0, &public);
        assert_committed(&chain, &public);
        for _ in 0..2 {
            let reader = CompleteSeedReader::open(&concrete, &public, identity);
            assert_eq!(reader.identity(), identity);
            for expected in public["state_before"]["accounts"].as_array().unwrap() {
                let selected = bytes(&expected["address"]).try_into().unwrap();
                let actual = reader.account(selected).unwrap();
                if expected["exists"] == false {
                    assert!(matches!(actual, ConcreteRead::Absent));
                    continue;
                }
                let ConcreteRead::Present(actual) = actual else {
                    panic!("account present in Go")
                };
                assert_eq!(actual.physical_rlp, bytes(&expected["encoded"]));
                assert_eq!(
                    BigUint::from_bytes_be(&actual.account.nonce.to_bytes()),
                    number(&expected["nonce"])
                );
                assert_eq!(
                    actual.account.balance.value(),
                    &number(&expected["balance"])
                );
                if expected.get("code_hash").is_some() {
                    let hash = bytes(&expected["code_hash"]).try_into().unwrap();
                    assert_eq!(actual.account.code_hash, Some(hash));
                    assert_eq!(
                        reader.code(hash).unwrap(),
                        ConcreteRead::Present(bytes(&expected["code"]))
                    );
                }
            }
            for (key, expected) in public["state_before"]["native_raw"].as_object().unwrap() {
                let actual = reader
                    .storage(
                        address(0xfe),
                        ConcreteStorageKey(hex::decode(key).unwrap().try_into().unwrap()),
                    )
                    .unwrap();
                assert_eq!(
                    match actual {
                        ConcreteRead::Present(value) => Some(value),
                        _ => None,
                    },
                    if expected["present"] == true {
                        Some(bytes(&expected["value"]))
                    } else {
                        None
                    }
                );
            }
            let case = &public["cases"][0];
            let mut expected_reads: Option<Vec<ConcreteStorageKey>> = None;
            for attempt in 0..34 {
                let fail = (attempt >= 2 && attempt % 2 == 0).then(|| (attempt - 2) / 4 + 9);
                let reads = Rc::new(RefCell::new(Vec::new()));
                let drops = Rc::new(Cell::new(0));
                let supplied =
                    FinalChainNonce::from_bytes(&(BigUint::from(1_u8) << 512usize).to_bytes_be())
                        .unwrap();
                let request = ExecutionTransaction {
                    position: 0.into(),
                    hash: [0; 32],
                    sender: caller,
                    receiver: Some(address(0xfe)),
                    nonce: supplied.clone(),
                    gas_price: ExecutionGasPrice::default(),
                    gas_limit: 200000.into(),
                    value: ExecutionValue::default(),
                    input: bytes(&case["input"]),
                    canonical_rlp: None,
                    kind: ExecutionTransactionKind::Call,
                };
                sessions += 1;
                let simulated = simulate_with_native(
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
                            corrupt: attempt >= 2 && (attempt - 2) % 4 == 0,
                        })
                    },
                    &ExecutionBlockContext {
                        period: identity.period,
                        author: address(0x31),
                        timestamp: 1700000001,
                        gas_limit: 500000.into(),
                        chain_id: 666,
                        difficulty: BigUint::default(),
                    },
                    &request,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::for_phase(TaraxaPhase::Ficus),
                );
                assert_eq!(drops.get(), 1);
                assert_eq!(request.nonce, supplied);
                if let Some(failed) = fail {
                    let message = format!("{simulated:?}");
                    assert!(
                        message.contains(if (attempt - 2) % 4 == 0 {
                            "RawIntegrity"
                        } else {
                            "signed H1 authentication failure"
                        }),
                        "{message}"
                    );
                    assert!(simulated.is_err());
                    assert_eq!(
                        &*reads.borrow(),
                        &expected_reads.as_ref().unwrap()[..=failed]
                    );
                    assert_committed(&chain, &public);
                    assert_eq!(concrete_rows(&concrete), before);
                    continue;
                }
                let simulated = simulated.unwrap();
                let measured = reads.borrow().clone();
                assert_eq!(measured.len(), 17);
                if let Some(expected) = &expected_reads {
                    assert_eq!(&measured, expected);
                } else {
                    expected_reads = Some(measured);
                }
                assert_eq!(simulated.state, identity);
                let TransactionExecutionResult::Executed(result) = simulated.execution else {
                    panic!("Go admitted target")
                };
                assert_eq!(result.status, CodeExecutionStatus::Success);
                assert_eq!(
                    result.gas_used.as_u64(),
                    case["output"]["gas_used"].as_u64().unwrap()
                );
                assert_eq!(result.output, bytes(&case["output"]["return"]));
                let logs=result.logs.iter().map(|log|json!({"address":hex::encode(log.address),"topics":log.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&log.data)})).collect::<Vec<_>>();
                assert_eq!(logs, *case["output"]["logs"].as_array().unwrap());
                assert_eq!(case["output"]["effective_nonce"], "2");
                assert_committed(&chain, &public);
                assert_eq!(concrete_rows(&concrete), before);
            }
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
        }
    }
    assert_eq!(sessions, 136);
}
