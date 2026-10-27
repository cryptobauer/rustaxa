//! Real parent-revert frame over the same retained commission seed.
use super::mixed_native;
use super::*;
use ethereum_types::{H256, U256};
use k256::ecdsa::SigningKey;
use rlp::RlpStream;
use rustaxa_storage::Column;
use rustaxa_types::{FinalizationTransaction, GenesisAccount};
use serde_json::json;

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
fn retained_commission_real_parent_revert_retains_raw_and_reverts_accounts_logs() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_retained_commission/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../../experiments/evm_feasibility/fixtures/native_retained_commission/local.json"
    ))
    .unwrap();
    assert_eq!(public, local);
    let row = &public["frame"];
    let code = bytes(&row["code_overlay"]);
    let code_hash = keccak256(&code).0;
    let state =
        FixtureState::from_case(&json!({"code":row["code_overlay"],"prior_raw":row["raw_before"]}));
    let owner = address(public["owner"].as_str().unwrap());
    let delegator = address(public["delegator"].as_str().unwrap());
    {
        let mut rows = state.rows.borrow_mut();
        rows.accounts.clear();
        for account in public["state_before"]["accounts"].as_array().unwrap() {
            let a = address(account["address"].as_str().unwrap());
            rows.accounts.insert(
                a,
                LogicalAccount {
                    nonce: nonce(account["nonce"].as_str().unwrap()),
                    balance: ConcreteAccountBalance::new(number(
                        account["balance"].as_str().unwrap(),
                    )),
                    code_hash: if a == owner { Some(code_hash) } else { None },
                    code_size: if a == owner { code.len() as u64 } else { 0 },
                },
            );
        }
    }
    let path = FixturePath(temp_db_path("retained-commission-frame"));
    let chain = history(&path, true, &public);
    let mut port = RecordingPort {
        inner: mixed_native::MixedNativeExecutionPort::new(
            chain.begin_native_session(3.into(), 2.into()).unwrap(),
        ),
        outcomes: vec![],
    };
    let mut sequence = PeriodConsensusSequence::new(3.into());
    for attempt in 0..2 {
        let mut journal = ExecutionJournal::new(state.clone());
        let result = execute_top_level_call_with_native(
            &mut journal,
            &NoHistory,
            &DposNative,
            &DposNative,
            &mut port,
            &mut sequence,
            &ExecutionBlockContext {
                period: 3.into(),
                author: addr(0x31),
                timestamp: 1700000003,
                gas_limit: 1000000.into(),
                chain_id: 666,
                difficulty: BigUint::default(),
            },
            &ExecutionTransaction {
                position: attempt.into(),
                hash: [0; 32],
                sender: delegator,
                receiver: Some(owner),
                nonce: FinalChainNonce::from_u64(1 + u64::from(attempt)),
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
        let TransactionExecutionResult::Executed(execution) = result else {
            panic!("frame admitted")
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
        let NativeInvocationResult::Completed(out) = &port.outcomes[attempt as usize] else {
            panic!("child completed")
        };
        assert_eq!(out.status, NativeStatus::Success);
        assert_eq!(out.gas_used.as_u64(), 20000);
        assert!(out.output.is_empty());
        let raw=out.raw_mutations.iter().map(|w|json!({"address":hex::encode(w.address),"key":hex::encode(w.key.0),"value":hex::encode(match &w.operation{NativeRawOperation::Put(v)=>v.as_bytes(),NativeRawOperation::Delete=>&[]})})).collect::<Vec<_>>();
        assert_eq!(raw, *row["ordered_raw_writes"].as_array().unwrap());
        assert_eq!(out.logs.len(), 1);
        assert_eq!(
            BigUint::from_bytes_be(&out.logs[0].data),
            BigUint::from(if attempt == 0 { 2u8 } else { 0 })
        );
        if attempt == 0 {
            assert_eq!(out.logs, fixture_logs(&row["native_observation"]["logs"]));
        }
        let settled = journal.settle_transaction().unwrap();
        assert_eq!(settled.native_invocations.len(), 1);
        assert_eq!(
            settled.native_invocations[0].disposition,
            ConsensusNativeDisposition::OuterFrameReverted
        );
        assert!(settled.logs.is_empty());
        state.apply(settled.writes);
        let rows = state.rows.borrow();
        for (a, account) in row["accounts_after"].as_object().unwrap() {
            let current = &rows.accounts[&address(a)];
            assert_eq!(current.balance.value().to_string(), account["balance"]);
            if attempt == 0 {
                assert_eq!(
                    BigUint::from_bytes_be(&current.nonce.to_bytes()).to_string(),
                    account["nonce"]
                );
            }
        }
        for (key, value) in row["raw_after"].as_object().unwrap() {
            assert_eq!(
                rows.storage
                    .get(&(DPOS, ConcreteStorageKey(hash(key))))
                    .unwrap(),
                &bytes(value)
            );
        }
        assert_eq!(
            rows.storage.len(),
            row["raw_after"].as_object().unwrap().len()
        );
    }
}
fn addr(last: u8) -> [u8; 20] {
    let mut a = [0; 20];
    a[19] = last;
    a
}
struct FixturePath(PathBuf);
impl Drop for FixturePath {
    fn drop(&mut self) {
        crate::closeout_test_scratch::dispose(&self.0).unwrap();
    }
}
fn history(path: &FixturePath, initialize: bool, public: &Value) -> FinalChain {
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
                    address: addr(last),
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
            receiver: Some(addr(0xfe)),
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
                    voter: ethereum_types::H160(addr(0x31)),
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
