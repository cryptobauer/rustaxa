//! End-to-end composition of the iterative EVM driver, journal lanes, and the
//! real staged FinalChain `setCommission` kernel against both pinned Go pins.

use std::{
    cell::RefCell,
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use num_bigint::BigUint;
use revm::primitives::keccak256;
use rustaxa_consensus::{
    FinalChain,
    native_session::{
        FinalChainNativeCallKind, FinalChainNativeGasQuote, FinalChainNativeInvocationId,
        FinalChainNativeInvocationResult, FinalChainNativeOutcome, FinalChainNativeRequest,
        FinalChainNativeSession, FinalChainNativeSessionError, FinalChainNativeStateRead,
        FinalChainNativeStateReadError, FinalChainNativeStatus, FinalChainNativeValue,
    },
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ExecutionBlockContext, ExecutionGasPrice, ExecutionLog, ExecutionTransaction,
        ExecutionTransactionKind, ExecutionValue, NativeCallKind, NativeContractFailure,
        NativeExecutionPort, NativeGasQuote, NativeInvocation, NativeInvocationId,
        NativeInvocationResult, NativeJournalRead, NativeJournalReadError,
        NativeOrdinaryAccountMutation, NativeOutcome, NativePortError, NativeRawMutation,
        NativeRawOperation, NativeRawValue, NativeStatus, TransactionExecutionResult,
    },
    driver::{
        NativeAddressClassifier, PeriodConsensusSequence, execute_top_level_call_with_native,
    },
    envelope::EnvelopeRules,
    journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan},
    profile::TaraxaProfile,
};
use rustaxa_storage::{Config, Storage, account_commitment_rlp, decode_physical_account};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainNonce, FinalChainRewardsConfig,
    FinalChainTransactionPosition, GenesisDposConfig, GenesisValidator, GenesisValidatorMetadata,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteRead,
        ConcreteReadError, ConcreteStateIdentity, ConcreteStateRead, ConcreteStorageKey,
    },
};
use serde_json::Value;

const SENDER: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
];
const TARGET: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xbb,
];
const DPOS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];
const VALIDATOR: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x31,
];

#[derive(Clone)]
struct LogicalAccount {
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
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
struct FixtureState {
    rows: Rc<RefCell<LogicalRows>>,
}

impl FixtureState {
    fn from_case(row: &Value) -> Self {
        let code = bytes(&row["code"]);
        let code_hash = keccak256(&code).0;
        let mut rows = LogicalRows::default();
        rows.accounts.insert(
            SENDER,
            LogicalAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(1_000_000_u64)),
                code_hash: None,
                code_size: 0,
            },
        );
        rows.accounts.insert(
            TARGET,
            LogicalAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::default(),
                code_hash: Some(code_hash),
                code_size: code.len() as u64,
            },
        );
        rows.accounts.insert(
            DPOS,
            LogicalAccount {
                nonce: FinalChainNonce::from_u64(1),
                balance: ConcreteAccountBalance::new(BigUint::from(10_000_u64)),
                code_hash: None,
                code_size: 0,
            },
        );
        rows.code.insert(code_hash, code);
        for (key, value) in row["prior_raw"].as_object().unwrap() {
            rows.storage.insert(
                (DPOS, ConcreteStorageKey(hash(key))),
                hex::decode(value.as_str().unwrap()).unwrap(),
            );
        }
        Self {
            rows: Rc::new(RefCell::new(rows)),
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
                    rows.accounts.insert(
                        write.address,
                        LogicalAccount {
                            nonce,
                            balance,
                            code_hash,
                            code_size,
                        },
                    );
                }
                JournalAccountOperation::Delete => {
                    rows.accounts.remove(&write.address);
                    rows.storage
                        .retain(|(address, _), _| *address != write.address);
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
    }

    fn assert_accounts(&self, row: &Value) {
        let rows = self.rows.borrow();
        let expected = row["accounts"].as_object().unwrap();
        assert_eq!(rows.accounts.len(), expected.len(), "{}", row["case"]);
        for (raw_address, facts) in expected {
            let address = address(raw_address);
            let account = rows.accounts.get(&address).expect("fixture account exists");
            let expected_record = decode_physical_account(&bytes(&facts["disk"])).unwrap();
            assert_eq!(
                account.nonce, expected_record.account.nonce,
                "{} nonce {raw_address}",
                row["case"]
            );
            assert_eq!(
                account.balance.value(),
                expected_record.account.balance.value(),
                "{} balance {raw_address}",
                row["case"]
            );
            let expected_code = bytes(&facts["code"]);
            assert_eq!(
                account.code_size,
                expected_code.len() as u64,
                "{} code size {raw_address}",
                row["case"]
            );
            assert_eq!(
                account.code_hash, expected_record.account.code_hash,
                "{} code hash {raw_address}",
                row["case"]
            );
            let actual_code = account
                .code_hash
                .and_then(|code_hash| rows.code.get(&code_hash).cloned())
                .unwrap_or_default();
            assert_eq!(
                actual_code, expected_code,
                "{} code {raw_address}",
                row["case"]
            );
            assert_eq!(
                account_commitment_rlp(&expected_record).unwrap(),
                bytes(&facts["leaf"]),
                "{} reconstructed account commitment {raw_address}",
                row["case"]
            );
            assert_eq!(
                expected_record.account.nonce,
                nonce(facts["nonce"].as_str().unwrap()),
                "{} decoded nonce {raw_address}",
                row["case"]
            );
            assert_eq!(
                expected_record.account.balance.value(),
                &number(facts["balance"].as_str().unwrap()),
                "{} decoded balance {raw_address}",
                row["case"]
            );
            if address == DPOS {
                assert_eq!(
                    expected_record.account.storage_root,
                    Some(hash(row["storage_root"].as_str().unwrap())),
                    "{} reconstructed native storage root",
                    row["case"]
                );
            }
        }
    }

    fn assert_raw(&self, row: &Value) {
        let rows = self.rows.borrow();
        let actual = rows
            .storage
            .iter()
            .filter(|((address, _), _)| *address == DPOS)
            .map(|((_, key), value)| (hex::encode(key.0), hex::encode(value)))
            .collect::<BTreeMap<_, _>>();
        let expected = row["raw"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected, "{} final raw map", row["case"]);
    }
}

impl ConcreteStateRead for FixtureState {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: [0; 32],
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
                storage_root: rows
                    .storage
                    .keys()
                    .any(|(owner, _)| owner == &address)
                    .then_some([0x77; 32]),
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

    fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .rows
            .borrow()
            .code
            .get(&code_hash)
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

struct DposNative;

impl NativeAddressClassifier for DposNative {
    fn is_native_address(&self, _period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        address == DPOS
    }
}

struct SessionRead<'a>(&'a dyn NativeJournalRead);

impl FinalChainNativeStateRead for SessionRead<'_> {
    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        self.0
            .raw_storage(address, key)
            .map_err(|error| match error {
                NativeJournalReadError::State(error) => {
                    FinalChainNativeStateReadError::State(error)
                }
                NativeJournalReadError::Invariant(error) => {
                    FinalChainNativeStateReadError::Invariant(error)
                }
            })
    }
}

struct SessionPort<'a> {
    session: FinalChainNativeSession<'a>,
    prepared: Vec<(NativeInvocation, FinalChainGas)>,
    invoked: Vec<NativeInvocation>,
}

impl NativeExecutionPort for SessionPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let quote = self
            .session
            .prepare(&consensus_request(invocation), &SessionRead(journal))
            .map_err(native_port_error)?;
        self.prepared.push((invocation.clone(), quote.required_gas));
        Ok(NativeGasQuote {
            invocation: native_id(quote.invocation),
            required_gas: quote.required_gas,
        })
    }

    fn invoke(
        &mut self,
        invocation: &NativeInvocation,
        quote: NativeGasQuote,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeInvocationResult, NativePortError> {
        self.invoked.push(invocation.clone());
        let request = consensus_request(invocation);
        let quote = FinalChainNativeGasQuote {
            invocation: consensus_id(quote.invocation),
            required_gas: quote.required_gas,
        };
        match self
            .session
            .invoke(&request, quote, &SessionRead(journal))
            .map_err(native_port_error)?
        {
            FinalChainNativeInvocationResult::InsufficientGas { required_gas } => {
                Ok(NativeInvocationResult::InsufficientGas { required_gas })
            }
            FinalChainNativeInvocationResult::Completed(outcome) => {
                Ok(NativeInvocationResult::Completed(native_outcome(outcome)?))
            }
        }
    }
}

fn consensus_request(invocation: &NativeInvocation) -> FinalChainNativeRequest {
    FinalChainNativeRequest {
        id: consensus_id(invocation.id),
        period: invocation.period,
        depth: invocation.depth,
        kind: match invocation.kind {
            NativeCallKind::Call => FinalChainNativeCallKind::Call,
            NativeCallKind::CallCode => FinalChainNativeCallKind::CallCode,
            NativeCallKind::DelegateCall => FinalChainNativeCallKind::DelegateCall,
            NativeCallKind::StaticCall => FinalChainNativeCallKind::StaticCall,
        },
        is_static: invocation.is_static,
        caller: invocation.caller,
        contract: invocation.contract,
        state_address: invocation.state_address,
        value: FinalChainNativeValue::new(invocation.value.value().clone()),
        input: invocation.input.clone(),
        supplied_gas: invocation.supplied_gas,
    }
}

fn consensus_id(id: NativeInvocationId) -> FinalChainNativeInvocationId {
    FinalChainNativeInvocationId {
        transaction: id.transaction,
        sequence: id.sequence,
    }
}

fn native_id(id: FinalChainNativeInvocationId) -> NativeInvocationId {
    NativeInvocationId {
        transaction: id.transaction,
        sequence: id.sequence,
    }
}

fn native_outcome(outcome: FinalChainNativeOutcome) -> Result<NativeOutcome, NativePortError> {
    let status = match outcome.status {
        FinalChainNativeStatus::Success => NativeStatus::Success,
        FinalChainNativeStatus::ContractFailure { error } => {
            NativeStatus::ContractFailure(NativeContractFailure { error })
        }
    };
    let raw_mutations = outcome
        .raw_mutations
        .into_iter()
        .map(|mutation| {
            let value = NativeRawValue::new(mutation.replacement)
                .map_err(|error| NativePortError::Infrastructure(error.to_string()))?;
            Ok(NativeRawMutation {
                address: mutation.address,
                key: mutation.key,
                expected: mutation.expected,
                operation: NativeRawOperation::Put(value),
            })
        })
        .collect::<Result<Vec<_>, NativePortError>>()?;
    Ok(NativeOutcome {
        status,
        gas_used: outcome.gas_used,
        output: outcome.output,
        account_mutations: Vec::<NativeOrdinaryAccountMutation>::new(),
        raw_mutations,
        logs: outcome
            .logs
            .into_iter()
            .map(|log| ExecutionLog {
                address: log.address,
                topics: log.topics,
                data: log.data,
            })
            .collect(),
        diagnostic: None,
    })
}

fn native_port_error(error: FinalChainNativeSessionError) -> NativePortError {
    match error {
        FinalChainNativeSessionError::OutOfSequence { expected, actual } => {
            NativePortError::OutOfSequence { expected, actual }
        }
        FinalChainNativeSessionError::QuoteMismatch => NativePortError::QuoteMismatch,
        FinalChainNativeSessionError::StateRead(error) => NativePortError::Journal(match error {
            FinalChainNativeStateReadError::State(error) => NativeJournalReadError::State(error),
            FinalChainNativeStateReadError::Invariant(error) => {
                NativeJournalReadError::Invariant(error)
            }
        }),
        error => NativePortError::Domain(error.to_string()),
    }
}

#[test]
fn six_native_cases_match_both_go_pins_through_driver_journal_and_final_chain_kernel() {
    let public: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/public.json"
    ))
    .unwrap();
    let local: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .unwrap();
    assert_eq!(public["native_calls"], local["native_calls"]);

    for row in public["native_calls"].as_array().unwrap() {
        run_case(row);
    }
}

fn run_case(row: &Value) {
    let state = FixtureState::from_case(row);
    let owner = address(row["owner"].as_str().unwrap());
    let path = temp_db_path(row["case"].as_str().unwrap());
    let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
    let final_chain = FinalChain::new_with_rewards_config(
        storage.clone(),
        1_000_000.into(),
        0,
        Vec::new(),
        vec![GenesisValidator {
            address: VALIDATOR,
            vrf_key: [0; 32],
            total_stake: ethereum_types::U256::from(10_000).to_big_endian().to_vec(),
            delegations: vec![(
                VALIDATOR,
                ethereum_types::U256::from(10_000).to_big_endian().to_vec(),
            )],
            metadata: GenesisValidatorMetadata {
                owner,
                commission: 100,
                ..Default::default()
            },
        }],
        GenesisDposConfig {
            eligibility_balance_threshold: ethereum_types::U256::from(1_000).into(),
            vote_eligibility_balance_step: ethereum_types::U256::from(1_000).into(),
            validator_maximum_stake: ethereum_types::U256::from(30_000).into(),
            commission_change_delta: 0,
            commission_change_frequency: 0,
            ..Default::default()
        },
        FinalChainRewardsConfig {
            magnolia_period: FinalChainBlockNumber::GENESIS,
            cornus_period: FinalChainBlockNumber::GENESIS,
            fix_redelegate_block_num: if row["pre_fix"].as_bool().unwrap() {
                FinalChainBlockNumber::new(2)
            } else {
                FinalChainBlockNumber::GENESIS
            },
            aspen_part_two_period: FinalChainBlockNumber::MAX,
            cacti_period: FinalChainBlockNumber::MAX,
            ..Default::default()
        },
    )
    .unwrap();
    let session = final_chain
        .begin_native_session(1.into(), FinalChainBlockNumber::GENESIS)
        .unwrap();
    let mut port = SessionPort {
        session,
        prepared: Vec::new(),
        invoked: Vec::new(),
    };
    let mut sequence = PeriodConsensusSequence::new(1.into());
    let mut journal = ExecutionJournal::new(state.clone());
    let transaction = ExecutionTransaction {
        position: FinalChainTransactionPosition::new(0),
        hash: [0x11; 32],
        sender: SENDER,
        receiver: Some(TARGET),
        nonce: FinalChainNonce::from_u64(1),
        gas_price: ExecutionGasPrice::new(BigUint::from(1_u8)),
        gas_limit: FinalChainGas::new(100_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let result = execute_top_level_call_with_native(
        &mut journal,
        &NoHistory,
        &DposNative,
        &DposNative,
        &mut port,
        &mut sequence,
        &ExecutionBlockContext {
            period: FinalChainBlockNumber::new(1),
            author: [0; 20],
            timestamp: 0,
            gas_limit: FinalChainGas::new(1_000_000),
            chain_id: 666,
            difficulty: BigUint::default(),
        },
        &transaction,
        EnvelopeRules { cornus: true },
        TaraxaProfile::new(false),
    )
    .unwrap();
    let TransactionExecutionResult::Executed(result) = result else {
        panic!("{} was admitted by the Go fixture", row["case"])
    };
    let expected_error = row["error"].as_str().unwrap();
    if expected_error.is_empty() {
        assert_eq!(
            result.status,
            CodeExecutionStatus::Success,
            "{}",
            row["case"]
        );
    } else {
        assert_eq!(expected_error, "execution reverted");
        assert_eq!(
            result.status,
            CodeExecutionStatus::Failure(CodeExecutionError::Revert),
            "{}",
            row["case"]
        );
    }
    assert_eq!(result.gas_used.as_u64(), row["gas_used"].as_u64().unwrap());
    assert_eq!(hex::encode(&result.output), row["return"].as_str().unwrap());
    assert_eq!(result.logs, fixture_logs(&row["logs"]));
    assert_eq!(result.attempted_contract_address, None);
    assert_eq!(sequence.next_sequence(), 1, "{}", row["case"]);
    assert_native_invocation(row, &port);

    let settled = journal.settle_transaction().unwrap();
    assert_eq!(settled.refund, 0, "{}", row["case"]);
    assert!(settled.writes.ordinary_storage.is_empty());
    assert_ordered_raw_writes(row, &settled.writes);
    state.apply(settled.writes);
    state.assert_accounts(row);
    state.assert_raw(row);

    drop(port);
    drop(final_chain);
    drop(storage);
    let _ = std::fs::remove_dir_all(path);
}

fn assert_native_invocation(row: &Value, port: &SessionPort<'_>) {
    assert_eq!(port.prepared.len(), 1, "{} prepare count", row["case"]);
    assert_eq!(port.invoked.len(), 1, "{} invoke count", row["case"]);
    let (invocation, quote) = &port.prepared[0];
    assert_eq!(invocation, &port.invoked[0], "{} request", row["case"]);
    assert_eq!(*quote, FinalChainGas::new(20_000), "{} quote", row["case"]);
    assert_eq!(
        invocation.id,
        NativeInvocationId {
            transaction: FinalChainTransactionPosition::new(0),
            sequence: 0,
        },
        "{} identity",
        row["case"]
    );
    assert_eq!(invocation.period, FinalChainBlockNumber::new(1));
    assert_eq!(invocation.depth, 1);
    assert_eq!(invocation.caller, TARGET);
    assert_eq!(invocation.contract, DPOS);
    assert_eq!(invocation.state_address, DPOS);
    assert_eq!(invocation.value, ExecutionValue::default());
    assert_eq!(invocation.input, bytes(&row["abi"]));
    assert_eq!(invocation.supplied_gas, FinalChainGas::new(50_000));
    let expected_kind = if row["static"].as_bool().unwrap() {
        NativeCallKind::StaticCall
    } else {
        NativeCallKind::Call
    };
    assert_eq!(invocation.kind, expected_kind);
    assert_eq!(invocation.is_static, row["static"].as_bool().unwrap());
}

fn assert_ordered_raw_writes(row: &Value, plan: &JournalWritePlan) {
    let actual = plan
        .raw_storage
        .iter()
        .map(|write| {
            let value = match &write.operation {
                NativeRawOperation::Put(value) => hex::encode(value.as_bytes()),
                NativeRawOperation::Delete => String::new(),
            };
            serde_json::json!({"key": hex::encode(write.key.0), "value": value})
        })
        .collect::<Vec<_>>();
    let expected = row["writes"].as_array().cloned().unwrap_or_default();
    assert_eq!(actual, expected, "{} ordered raw writes", row["case"]);
}

fn fixture_logs(value: &Value) -> Vec<ExecutionLog> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|log| ExecutionLog {
            address: address(log["address"].as_str().unwrap()),
            topics: log["topics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|topic| hash(topic.as_str().unwrap()))
                .collect(),
            data: bytes(&log["data"]),
        })
        .collect()
}

fn temp_db_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rustaxa-native-session-reference-{test_name}-{}-{nanos}",
        std::process::id()
    ))
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().unwrap()).unwrap()
}

fn hash(value: &str) -> [u8; 32] {
    hex::decode(value).unwrap().try_into().unwrap()
}

fn address(value: &str) -> [u8; 20] {
    hex::decode(value).unwrap().try_into().unwrap()
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
