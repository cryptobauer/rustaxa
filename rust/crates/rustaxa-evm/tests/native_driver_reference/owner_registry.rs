//! Ordinary slashing-address calls before registration and native precedence.
use super::*;
use rustaxa_consensus::FinalChain;
use rustaxa_storage::Storage;
struct OwnerRegistry<'a>(&'a FinalChain);
impl NativeAddressClassifier for OwnerRegistry<'_> {
    fn is_native_address(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.0.native_contract_kind(period, address).is_some()
    }
}
#[test]
fn owner_registry_routes_slashing_by_period_with_ordinary_account_semantics() {
    let path = crate::closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!("rustaxa-registry-evm-{}", std::process::id())),
        true,
    );
    let slash = primitive_address(0xee);
    let ordinary_code = hex::decode("602a60005360016000f3").unwrap();
    for activation in [7_u64, 8] {
        let chain = FinalChain::new_with_rewards_config(
            Arc::new(
                Storage::new(rustaxa_storage::Config::new(
                    path.join(activation.to_string()),
                ))
                .unwrap(),
            ),
            10_000_000.into(),
            0,
            Vec::new(),
            Vec::new(),
            Default::default(),
            rustaxa_types::FinalChainRewardsConfig {
                magnolia_period: activation.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let registry = OwnerRegistry(&chain);
        let owner_before = chain.account(slash).unwrap();
        for mode in 0..3 {
            let reads = Arc::new(AtomicUsize::new(0));
            let mut reader = Reader {
                accounts: BTreeMap::from([(
                    SENDER,
                    ConcreteAccount {
                        nonce: FinalChainNonce::from_u64(1),
                        balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                        storage_root: None,
                        code_hash: None,
                        code_size: 0,
                    },
                )]),
                codes: BTreeMap::new(),
                code_reads: Arc::clone(&reads),
            };
            if mode > 0 {
                let code = if mode == 2 {
                    ordinary_code.clone()
                } else {
                    Vec::new()
                };
                let hash = (!code.is_empty()).then(|| keccak256(&code).0);
                if let Some(hash) = hash {
                    reader.codes.insert(hash, code.clone());
                }
                reader.accounts.insert(
                    slash,
                    ConcreteAccount {
                        nonce: FinalChainNonce::from_u64(9),
                        balance: ConcreteAccountBalance::new(BigUint::from(11_u8)),
                        storage_root: None,
                        code_hash: hash,
                        code_size: code.len() as u64,
                    },
                );
            }
            let mut journal = ExecutionJournal::new(reader);
            let before_metadata = journal.account_metadata(slash).unwrap();
            let value = if mode == 1 { 5_u8 } else { 0 };
            let mut port = ScriptedPort::completed(NativeStatus::Success, vec![0x99]);
            let mut sequence = PeriodConsensusSequence::new(block().period);
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHistory,
                &registry,
                &registry,
                &mut port,
                &mut sequence,
                &block(),
                &transaction(slash, ExecutionValue::new(BigUint::from(value))),
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .unwrap();
            let TransactionExecutionResult::Executed(result) = result else {
                panic!("must execute")
            };
            assert_eq!(result.status, CodeExecutionStatus::Success);
            if activation == 7 {
                assert_eq!(result.gas_used, FinalChainGas::new(21_020));
                assert_eq!(result.output, vec![0x99]);
                assert_eq!(sequence.next_sequence(), 1);
                assert_eq!(port.invocations.lock().unwrap().len(), 1);
                assert_eq!(
                    reads.load(Ordering::Relaxed),
                    0,
                    "native registry takes precedence over code"
                );
            } else {
                assert_eq!(
                    result.gas_used,
                    FinalChainGas::new(if mode == 2 { 21_018 } else { 21_000 })
                );
                assert_eq!(
                    result.output,
                    if mode == 2 { vec![0x2a] } else { Vec::new() }
                );
                assert_eq!(sequence.next_sequence(), 0);
                assert!(port.invocations.lock().unwrap().is_empty());
                assert_eq!(reads.load(Ordering::Relaxed), usize::from(mode == 2));
            }
            if mode > 0 {
                let after = journal.account_metadata(slash).unwrap();
                assert_eq!(after.exists, before_metadata.exists);
                assert_eq!(after.nonce, before_metadata.nonce);
                assert_eq!(after.code_hash, before_metadata.code_hash);
                assert_eq!(after.code_size, before_metadata.code_size);
            }
            if mode == 1 {
                assert_eq!(
                    journal.account(slash).unwrap().balance.value(),
                    &num_bigint::BigInt::from(16_u8)
                );
            }
        }
        assert_eq!(
            chain.account(slash).unwrap(),
            owner_before,
            "registry classification must not mutate owner accounts"
        );
        drop(chain);
    }
    crate::closeout_test_scratch::dispose(&path).unwrap();
}
