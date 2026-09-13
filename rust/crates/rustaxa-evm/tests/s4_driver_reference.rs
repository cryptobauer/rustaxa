//! Direct driver comparison against the pinned two-period Go S4 oracle.
//!
//! This test applies each journal plan synchronously to a complete tiny state
//! and continues period two from those same in-memory logical rows. It compares
//! execution and complete logical mutation key sets only; serialization,
//! generation identity and physical trie roots belong to the writer composition.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
        NativeRawOperation, TransactionExecutionResult,
    },
    driver::{NativeAddressClassifier, execute_top_level_call, execute_top_level_create},
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
    profile::TaraxaProfile,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainTransactionPosition,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

#[derive(Clone)]
struct LogicalAccount {
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    storage_root: Option<[u8; 32]>,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
}

#[derive(Default)]
struct LogicalRows {
    accounts: BTreeMap<[u8; 20], LogicalAccount>,
    storage: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
    code: BTreeMap<[u8; 32], Vec<u8>>,
}

#[derive(Clone)]
struct TinyState {
    rows: Rc<RefCell<LogicalRows>>,
    period: FinalChainBlockNumber,
}

impl TinyState {
    fn genesis(fixture: &Value) -> Self {
        let sender = address(fixture["inputs"]["sender"].as_str().unwrap());
        let mut rows = LogicalRows::default();
        rows.accounts.insert(
            sender,
            LogicalAccount {
                nonce: FinalChainNonce::zero(),
                balance: ConcreteAccountBalance::new(number(
                    fixture["inputs"]["sender_genesis_balance"]
                        .as_str()
                        .unwrap(),
                )),
                storage_root: None,
                code_hash: None,
                code_size: 0,
            },
        );
        Self {
            rows: Rc::new(RefCell::new(rows)),
            period: FinalChainBlockNumber::GENESIS,
        }
    }

    fn at_period(&self, period: u64) -> Self {
        Self {
            rows: Rc::clone(&self.rows),
            period: FinalChainBlockNumber::new(period),
        }
    }

    fn apply(&self, plan: JournalWritePlan) {
        let mut rows = self.rows.borrow_mut();
        for write in plan.accounts {
            match write.operation {
                JournalAccountOperation::Upsert {
                    nonce,
                    balance,
                    code_hash,
                    code_size,
                } => {
                    let prior_root = rows
                        .accounts
                        .get(&write.address)
                        .and_then(|account| account.storage_root);
                    rows.accounts.insert(
                        write.address,
                        LogicalAccount {
                            nonce,
                            balance,
                            storage_root: prior_root,
                            code_hash,
                            code_size,
                        },
                    );
                }
                JournalAccountOperation::Delete => {
                    rows.accounts.remove(&write.address);
                    rows.storage
                        .retain(|(address, _), _| address != &write.address);
                }
            }
        }
        for write in plan.code {
            rows.code.insert(write.code_hash, write.code);
        }
        for write in plan.ordinary_storage {
            if write.value == BigUint::default() {
                rows.storage.remove(&(write.address, write.key));
            } else {
                rows.storage
                    .insert((write.address, write.key), write.value.to_bytes_be());
            }
        }
        for write in plan.raw_storage {
            match write.operation {
                NativeRawOperation::Put(value) => {
                    rows.storage
                        .insert((write.address, write.key), value.into_bytes());
                }
                NativeRawOperation::Delete => {
                    rows.storage.remove(&(write.address, write.key));
                }
            }
        }
        let addresses: Vec<_> = rows.accounts.keys().copied().collect();
        for address in addresses {
            let has_storage = rows.storage.keys().any(|(owner, _)| owner == &address);
            rows.accounts.get_mut(&address).unwrap().storage_root =
                has_storage.then_some([0x77; 32]);
        }
    }

    fn assert_account(&self, expected: &Value) {
        let address = address(expected["address"].as_str().unwrap());
        let rows = self.rows.borrow();
        let account = rows.accounts.get(&address).expect("expected account");
        assert_eq!(account.nonce, nonce(expected["nonce"].as_str().unwrap()));
        assert_eq!(
            account.balance.value(),
            &number(expected["balance"].as_str().unwrap())
        );
        assert_eq!(account.code_size, expected["code_size"].as_u64().unwrap());
        assert_eq!(account.code_hash, optional_hash(&expected["code_hash"]));
        let slot = rows.storage.get(&(address, ConcreteStorageKey([0_u8; 32])));
        if expected["slot_zero_present"].as_bool().unwrap() {
            assert_eq!(
                slot.map(hex::encode).as_deref(),
                expected["slot_zero"].as_str()
            );
        } else {
            assert!(slot.is_none());
        }
        if account.code_size != 0 {
            let hash = account.code_hash.unwrap();
            assert_eq!(
                rows.code.get(&hash).map(hex::encode).as_deref(),
                expected["code"].as_str()
            );
        }
    }

    fn assert_complete_keys(&self, expected_accounts: &[Value]) {
        let expected_account_keys: BTreeSet<_> = expected_accounts
            .iter()
            .map(|account| address(account["address"].as_str().unwrap()))
            .collect();
        let expected_storage_keys: BTreeSet<_> = expected_accounts
            .iter()
            .filter(|account| account["slot_zero_present"].as_bool().unwrap())
            .map(|account| {
                (
                    address(account["address"].as_str().unwrap()),
                    ConcreteStorageKey([0_u8; 32]),
                )
            })
            .collect();
        let expected_code_keys: BTreeSet<_> = expected_accounts
            .iter()
            .filter_map(|account| optional_hash(&account["code_hash"]))
            .collect();
        let rows = self.rows.borrow();
        assert_eq!(
            rows.accounts.keys().copied().collect::<BTreeSet<_>>(),
            expected_account_keys
        );
        assert_eq!(
            rows.storage.keys().copied().collect::<BTreeSet<_>>(),
            expected_storage_keys
        );
        assert_eq!(
            rows.code.keys().copied().collect::<BTreeSet<_>>(),
            expected_code_keys
        );
    }
}

impl ConcreteStateRead for TinyState {
    fn identity(&self) -> ConcreteStateIdentity {
        // This test adapter has no persisted generation. Its fixed root is a
        // trait placeholder and is never compared with the oracle's trie roots.
        ConcreteStateIdentity {
            period: self.period,
            state_root: [0_u8; 32],
        }
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        let rows = self.rows.borrow();
        let Some(account) = rows.accounts.get(&address) else {
            return Ok(ConcreteRead::Absent);
        };
        Ok(ConcreteRead::Present(ConcreteAccountRecord {
            account: ConcreteAccount {
                nonce: account.nonce.clone(),
                balance: account.balance.clone(),
                storage_root: account.storage_root,
                code_hash: account.code_hash,
                code_size: account.code_size,
            },
            physical_rlp: Vec::new(),
        }))
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .rows
            .borrow()
            .storage
            .get(&(address, key))
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }

    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .rows
            .borrow()
            .code
            .get(&hash)
            .cloned()
            .map_or(ConcreteRead::Absent, ConcreteRead::Present))
    }
}

struct NoHistory;

impl BlockHashRead for NoHistory {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        Err(BlockHashReadError::HistoryUnavailable(number))
    }
}

struct NoNative;

impl NativeAddressClassifier for NoNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, _address: [u8; 20]) -> bool {
        false
    }
}

#[test]
fn signed_transfer_create_and_logical_continuation_call_match_go_results_and_mutations() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/s4_public.json"
    ))
    .unwrap();
    let state = TinyState::genesis(&fixture);

    for period in fixture["periods"].as_array().unwrap() {
        let period_number = period["period"].as_u64().unwrap();
        let period_state = state.at_period(period_number.saturating_sub(1));
        for row in period["transactions"].as_array().unwrap() {
            let position =
                FinalChainTransactionPosition::from(row["index"].as_u64().unwrap() as u32);
            let signed = hex::decode(row["signed_rlp"].as_str().unwrap()).unwrap();
            let transaction =
                decode_legacy_input(position, &signed, LegacyInputKind::Signed).unwrap();
            assert_eq!(hex::encode(transaction.hash), row["hash"].as_str().unwrap());
            assert_eq!(
                transaction.canonical_rlp.as_deref(),
                Some(signed.as_slice())
            );

            let mut journal = ExecutionJournal::new(period_state.clone());
            let block = ExecutionBlockContext {
                period: FinalChainBlockNumber::new(period_number),
                author: [0_u8; 20],
                timestamp: 0,
                gas_limit: FinalChainGas::new(
                    fixture["configuration"]["block_gas_limit"]
                        .as_u64()
                        .unwrap(),
                ),
                chain_id: fixture["configuration"]["chain_id"].as_u64().unwrap(),
                difficulty: BigUint::default(),
            };
            let result = if transaction.receiver.is_some() {
                execute_top_level_call(
                    &mut journal,
                    &NoHistory,
                    &NoNative,
                    &block,
                    &transaction,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )
            } else {
                execute_top_level_create(
                    &mut journal,
                    &NoHistory,
                    &NoNative,
                    &block,
                    &transaction,
                    EnvelopeRules { cornus: true },
                    TaraxaProfile::new(false),
                )
            }
            .unwrap();
            let TransactionExecutionResult::Executed(result) = result else {
                panic!("oracle rows execute")
            };
            assert_eq!(result.status, CodeExecutionStatus::Success);
            assert_eq!(result.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
            assert_eq!(hex::encode(&result.output), row["output"].as_str().unwrap());
            assert_eq!(result.logs.len(), row["logs"].as_array().unwrap().len());
            let expected_created = address(row["created"].as_str().unwrap());
            assert_eq!(
                result.attempted_contract_address.unwrap_or([0_u8; 20]),
                expected_created
            );
            assert_eq!(
                journal.refund(),
                row["refund_before_transaction_commit"].as_u64().unwrap()
            );
            let settled = journal.settle_transaction().unwrap();
            period_state.apply(settled.writes);
        }
        let expected_accounts = period["accounts"].as_array().unwrap();
        for expected in expected_accounts {
            period_state.assert_account(expected);
        }
        period_state.assert_complete_keys(expected_accounts);
    }
}

fn address(value: &str) -> [u8; 20] {
    hex::decode(value).unwrap().try_into().unwrap()
}

fn optional_hash(value: &Value) -> Option<[u8; 32]> {
    value
        .as_str()
        .map(|value| hex::decode(value).unwrap().try_into().unwrap())
}

fn number(value: &str) -> BigUint {
    BigUint::parse_bytes(value.as_bytes(), 10).unwrap()
}

fn nonce(value: &str) -> FinalChainNonce {
    let value = number(value);
    if value == BigUint::default() {
        FinalChainNonce::zero()
    } else {
        FinalChainNonce::from_bytes(&value.to_bytes_be()).unwrap()
    }
}
