//! Same-height simulation over actual two-signer count4 committed H1, independently
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
        self.inner.invoke(
            invocation,
            quote,
            &AuthJournal {
                inner: journal,
                reads: self.reads.clone(),
                fail: self.fail,
                corrupt: self.corrupt,
            },
        )
    }
}

pub(super) fn corpus() -> Value {
    let public:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_four_simulation/public.json")).unwrap();
    let local:Value=serde_json::from_str(include_str!("../../../../../experiments/evm_feasibility/fixtures/native_redelegate_both_current_four_simulation/local.json")).unwrap();
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
pub(super) fn owner(public: &Value) -> [u8; 20] {
    let key = SigningKey::from_slice(&[0xa1; 32]).unwrap();
    let pubkey = key.verifying_key().to_encoded_point(false);
    let result: [u8; 20] = keccak256(&pubkey.as_bytes()[1..]).0[12..]
        .try_into()
        .unwrap();
    assert_eq!(hex::encode(result), public["owner"]);
    assert_ne!(result, caller(public));
    assert!(![address(0x31), address(0x32)].contains(&result));
    result
}
/// Canonical signed public envelope supplies all recovered finalization metadata.
fn prefix_transaction(public: &Value, index: usize) -> FinalizationTransaction {
    let fixture = &public["prefix"]["prefixes"][index]["signed_transaction"];
    let raw = bytes(&fixture["rlp"]);
    let tx = LegacyTransactionEnvelope::decode(&raw).unwrap();
    assert!(tx.signature_valid && tx.intrinsic_gas_covered);
    assert_eq!(tx.chain_id, 666);
    assert_eq!(
        tx.sender.unwrap().0,
        if index == 0 {
            caller(public)
        } else {
            owner(public)
        }
    );
    assert_eq!(hex::encode(tx.hash.0), fixture["hash"]);
    assert_eq!(tx.rlp, raw);
    assert_eq!(tx.nonce, U256::from(if index == 2 { 1 } else { 0 }));
    assert_eq!(tx.gas_price, U256::zero());
    assert_eq!(tx.value, U256::zero());
    assert_eq!(tx.gas, 200000);
    assert_eq!(tx.receiver.unwrap().0, address(0xfe));
    assert_eq!(tx.data, bytes(&fixture["input"]));
    assert_eq!(&tx.data[16..36], &address(0x31));
    assert_eq!(&tx.data[48..68], &address(0x32));
    assert_eq!(
        U256::from_big_endian(&tx.data[68..]),
        U256::from(if index == 0 { 300 } else { 100 })
    );
    FinalizationTransaction {
        hash: tx.hash.0,
        sender: tx.sender.unwrap().0,
        receiver: tx.receiver.map(|a| a.0),
        nonce: FinalChainNonce::from_u64(if index == 2 { 1 } else { 0 }),
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
        vec![
            GenesisAccount {
                address: caller,
                balance: rustaxa_types::FinalChainAccountBalance::new_account(balance),
            },
            GenesisAccount {
                address: owner(public),
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
                delegations: vec![(owner(public), U256::from(1000).to_big_endian().to_vec())]
                    .into_iter()
                    .chain(Some((caller, U256::from(1000).to_big_endian().to_vec())))
                    .collect(),
                metadata: GenesisValidatorMetadata {
                    owner: owner(public),
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
        let transactions = (0..3)
            .map(|index| prefix_transaction(public, index))
            .collect::<Vec<_>>();
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
            .begin_list(3);
        for tx in &transactions {
            period.append_raw(&tx.rlp, 1);
        }
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
        let (_, receipts) = chain.finalize_block(pbft, transactions, vec![]).unwrap();
        assert_eq!(receipts.len(), 3);
        for (index, bytes) in receipts.iter().enumerate() {
            let receipt = rlp::Rlp::new(bytes);
            assert_eq!(receipt.val_at::<u8>(0).unwrap(), 1);
            assert_eq!(
                receipt.val_at::<u64>(1).unwrap(),
                public["prefix"]["prefixes"][index]["receipt"]["gas_used"]
                    .as_u64()
                    .unwrap()
            );
            let logs = receipt.at(3).unwrap();
            let observed = (0..logs.item_count().unwrap()).map(|index| {
            let log = logs.at(index).unwrap();
            let topics = log.at(1).unwrap();
            json!({"address":hex::encode(log.at(0).unwrap().data().unwrap()), "topics": (0..topics.item_count().unwrap()).map(|n|hex::encode(topics.at(n).unwrap().data().unwrap())).collect::<Vec<_>>(), "data":hex::encode(log.at(2).unwrap().data().unwrap())})
        }).collect::<Vec<_>>();
            assert_eq!(
                observed,
                *public["prefix"]["prefixes"][index]["receipt"]["logs"]
                    .as_array()
                    .unwrap()
            );
        }
    }
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x0f, 0xa0]
    );
    chain
}

/// Public committed reads bind exact pairs/order/rewards and balances, not roots.
pub(super) fn assert_committed(chain: &FinalChain, public: &Value) {
    let caller = caller(public);
    assert_eq!(chain.last_block_number_typed().unwrap(), 1.into());
    assert_eq!(
        chain.dpos_total_amount_delegated(1.into()).unwrap(),
        vec![0x0f, 0xa0]
    );
    let stakes = chain.dpos_validators_total_stakes(1.into()).unwrap();
    assert_eq!(stakes.len(), 2);
    for (row, (last, stake)) in stakes.iter().zip([(0x31, 1500), (0x32, 2500)]) {
        assert_eq!(row.address, address(last));
        // This public stake API deliberately uses the delegation delay.
        assert_eq!(U256::from_big_endian(&row.stake), U256::from(2000));
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
        (caller, vec![0x31, 0x32]),
        (owner(public), vec![0x31, 0x32]),
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
            let amount = if owner != caller {
                if last == 0x31 { 800 } else { 1200 }
            } else if last == 0x31 {
                700
            } else {
                1300
            };
            assert_eq!(U256::from_big_endian(&row[32..64]), U256::from(amount));
            assert_eq!(U256::from_big_endian(&row[64..96]), U256::zero());
        }
    }
    let sender = chain.account_at_block(1.into(), caller).unwrap().unwrap();
    assert_eq!(sender.nonce, FinalChainNonce::from_u64(1));
    assert_eq!(*sender.balance.as_u256(), U256::from(3000));
    let owner = chain
        .account_at_block(1.into(), owner(public))
        .unwrap()
        .unwrap();
    assert_eq!(owner.nonce, FinalChainNonce::from_u64(2));
    assert_eq!(*owner.balance.as_u256(), U256::from(1000));
    let native = chain
        .account_at_block(1.into(), address(0xfe))
        .unwrap()
        .unwrap();
    assert_eq!(native.nonce, FinalChainNonce::from_u64(1));
    assert_eq!(*native.balance.as_u256(), U256::from(4000));
}
/// Reads every physical public-owner column, using its existing comparators.
/// Config locates the owner database under db; this is separate from Go's
/// numeric-column concrete seed. Read-only opens never modify owner state.
fn public_owner_rows(path: &FixturePath) -> Vec<(String, Vec<u8>, Vec<u8>)> {
    let config = Config::new(path.0.clone());
    let options = rocksdb::Options::default();
    let descriptors = Column::all()
        .iter()
        .map(|column| column.descriptor(&options));
    let db =
        rocksdb::DB::open_cf_descriptors_read_only(&options, &config.db_path, descriptors, false)
            .unwrap();
    let mut rows = Vec::new();
    for column in Column::all() {
        for row in db.iterator_cf(
            &db.cf_handle(column.name()).unwrap(),
            rocksdb::IteratorMode::Start,
        ) {
            let (key, value) = row.unwrap();
            rows.push((column.name().into(), key.to_vec(), value.to_vec()));
        }
    }
    rows
}

/// Persisted receipts and complete public validator metadata remain immutable.
fn public_identity(chain: &FinalChain, public: &Value) -> Vec<Vec<u8>> {
    let mut result = (0..3_u32)
        .map(|position| {
            chain
                .transaction_receipt_rlp(1.into(), position.into())
                .unwrap()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for last in [0x31, 0x32] {
        let mut input = keccak256(b"getValidator(address)")[..4].to_vec();
        input.extend_from_slice(&[0; 12]);
        input.extend_from_slice(&address(last));
        let row = chain
            .call(rustaxa_types::FinalChainCallRequest {
                block_number: 1.into(),
                sender: caller(public),
                receiver: Some(address(0xfe)),
                value: U256::zero().into(),
                gas_price: U256::zero().into(),
                gas_limit: 1000000.into(),
                input,
            })
            .unwrap();
        assert!(row.code_retval.len() >= 64);
        result.push(row.code_retval);
    }
    result
}
#[test]
fn signed_both_current_four_same_height_simulation_matches_go_and_reopens() {
    let public = corpus();
    let caller = caller(&public);
    assert_eq!(public["state_before"], public["state_after"]);
    assert_eq!(
        public["prefix"]["after_prefix"],
        public["prefix"]["after_end_block"]
    );
    assert_eq!(
        public["prefix"]["after_end_block"],
        public["prefix"]["after_commit"]
    );
    let concrete = FixturePath::new("signed-both-current-four-concrete");
    let app = FixturePath::new("signed-both-current-four-app");
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
        108
    );
    let before = concrete_rows(&concrete);
    let mut sessions = 0;
    let mut expected_owner_identity = None;
    let mut expected_owner_rows = None;
    for restart in 0..2 {
        let chain = history(&app, restart == 0, &public);
        assert_committed(&chain, &public);
        let owner_header = chain.block_header(1.into()).unwrap().unwrap();
        let owner_identity = public_identity(&chain, &public);
        let owner_rows = public_owner_rows(&app);
        if let Some(expected) = &expected_owner_rows {
            assert_eq!(&owner_rows, expected);
        } else {
            expected_owner_rows = Some(owner_rows.clone());
        }
        if let Some(expected) = &expected_owner_identity {
            assert_eq!(&owner_identity, expected);
        } else {
            expected_owner_identity = Some(owner_identity.clone());
        }
        for _ in 0..2 {
            let reader = CompleteSeedReader::open(&concrete, &public, identity);
            assert_eq!(reader.identity(), identity);
            for expected in public["state_before"]["accounts"].as_array().unwrap() {
                let selected = hex::decode(expected["address"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap();
                let actual = reader.account(selected).unwrap();
                if expected["exists"] == false {
                    assert!(matches!(actual, ConcreteRead::Absent));
                    continue;
                }
                let ConcreteRead::Present(actual) = actual else {
                    panic!("Go account is present")
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
                    let code_hash = bytes(&expected["code_hash"]).try_into().unwrap();
                    assert_eq!(actual.account.code_hash, Some(code_hash));
                    assert_eq!(
                        reader.code(code_hash).unwrap(),
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
            // First measure the successful sequence; then fail EVERY key both
            // ways, each immediately followed by a fresh successful retry.
            let mut plan = vec![(None, false), (None, false)];
            let mut attempt = 0;
            while attempt < plan.len() {
                let (fail, corrupt) = plan[attempt];
                attempt += 1;
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
                            corrupt,
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
                        message.contains(if corrupt {
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
                    assert_eq!(chain.block_header(1.into()).unwrap().unwrap(), owner_header);
                    assert_eq!(public_identity(&chain, &public), owner_identity);
                    assert_eq!(concrete_rows(&concrete), before);
                    assert_eq!(public_owner_rows(&app), owner_rows);
                    continue;
                }
                let simulated = simulated.unwrap();
                let measured = reads.borrow().clone();
                assert_eq!(
                    measured
                        .iter()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    measured.len()
                );
                assert_eq!(measured.len(), 29);
                if attempt == 1 {
                    println!(
                        "both-current-four H1: {} unique authentication keys",
                        measured.len()
                    );
                    for index in 0..measured.len() {
                        plan.extend([
                            (Some(index), true),
                            (None, false),
                            (Some(index), false),
                            (None, false),
                        ]);
                    }
                }
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
                assert_eq!(chain.block_header(1.into()).unwrap().unwrap(), owner_header);
                assert_eq!(public_identity(&chain, &public), owner_identity);
                assert_eq!(concrete_rows(&concrete), before);
                assert_eq!(public_owner_rows(&app), owner_rows);
            }
            drop(reader);
            assert_eq!(concrete_rows(&concrete), before);
            assert_eq!(public_owner_rows(&app), owner_rows);
        }
    }
    assert_eq!(sessions, 472);
}
