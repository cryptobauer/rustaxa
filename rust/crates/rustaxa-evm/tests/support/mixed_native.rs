//! Finite-fixture adapter between the EVM native port and a staged FinalChain session.
//!
//! This helper is deliberately test-only. It records the exact current-journal
//! facts actually consumed by each prepared/invoked consensus-native call and by
//! the terminal rewards phase. It neither publishes FinalChain state nor grants
//! lifecycle authority. Stateless precompiles remain owned by the EVM driver.

use std::{cell::RefCell, collections::BTreeMap};

use rustaxa_consensus::{
    final_chain_execution::FinalChainPreparedExternalEvmRewardsStatsPlan,
    native_projection_context::{
        FinalChainNativeInvocationContext, FinalChainNativeRawRead, FinalChainNativeRewardsContext,
    },
    native_session::{
        FinalChainNativeAccount, FinalChainNativeCallKind, FinalChainNativeGasQuote,
        FinalChainNativeInvocationId, FinalChainNativeInvocationResult,
        FinalChainNativeOrdinaryMutation, FinalChainNativeOutcome, FinalChainNativeRawMutation,
        FinalChainNativeRawOperation, FinalChainNativeRequest, FinalChainNativeRewardsOutcome,
        FinalChainNativeSession, FinalChainNativeSessionError, FinalChainNativeStateRead,
        FinalChainNativeStateReadError, FinalChainNativeStatus, FinalChainNativeValue,
    },
};
use rustaxa_evm::contracts::{
    ExecutionBalance, ExecutionLog, NativeCallKind, NativeContractFailure, NativeExecutionPort,
    NativeGasQuote, NativeInvocation, NativeInvocationId, NativeInvocationResult,
    NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation, NativeOutcome,
    NativePortError, NativeRawMutation, NativeRawOperation, NativeRawValue, NativeStatus,
};
use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};

type RawIdentity = ([u8; 20], ConcreteStorageKey);

#[derive(Clone, Debug, Default)]
struct RecordedReads {
    accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
    raw: BTreeMap<RawIdentity, ConcreteRead<Vec<u8>>>,
}

impl RecordedReads {
    fn invocation_context(
        self,
        request: FinalChainNativeRequest,
        ordinary_mutations: &[FinalChainNativeOrdinaryMutation],
        raw_mutations: &[FinalChainNativeRawMutation],
    ) -> FinalChainNativeInvocationContext {
        FinalChainNativeInvocationContext {
            request,
            accounts: self.accounts.into_iter().collect(),
            raw_reads: self
                .raw
                .into_iter()
                .map(|((address, key), value)| FinalChainNativeRawRead {
                    address,
                    key,
                    value,
                })
                .collect(),
            ordinary_mutations: ordinary_mutations.to_vec(),
            raw_mutations: raw_mutations.to_vec(),
        }
    }

    fn rewards_context(
        self,
        outcome: &FinalChainNativeRewardsOutcome,
    ) -> FinalChainNativeRewardsContext {
        FinalChainNativeRewardsContext {
            accounts: self.accounts.into_iter().collect(),
            raw_reads: self
                .raw
                .into_iter()
                .map(|((address, key), value)| FinalChainNativeRawRead {
                    address,
                    key,
                    value,
                })
                .collect(),
            ordinary_mutations: outcome.account_mutations.clone(),
            raw_mutations: outcome.raw_mutations.clone(),
        }
    }
}

/// Read-through recorder for one prepare/invoke pair or terminal rewards phase.
///
/// Every request reaches the current journal. Repeated reads are accepted only
/// when the exact account facts or raw classification and bytes are unchanged.
/// The resulting context therefore contains unique, actually consumed reads.
struct RecordingRead<'a> {
    journal: &'a dyn NativeJournalRead,
    reads: RefCell<RecordedReads>,
}

impl<'a> RecordingRead<'a> {
    fn new(journal: &'a dyn NativeJournalRead, reads: RecordedReads) -> Self {
        Self {
            journal,
            reads: RefCell::new(reads),
        }
    }

    fn into_reads(self) -> RecordedReads {
        self.reads.into_inner()
    }
}

impl FinalChainNativeStateRead for RecordingRead<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        let account = self.journal.account(address).map_err(state_read_error)?;
        let account = FinalChainNativeAccount {
            exists: account.exists,
            nonce: account.nonce,
            balance: account.balance.value().clone(),
        };
        let mut reads = self.reads.borrow_mut();
        if let Some(previous) = reads.accounts.get(&address) {
            if previous != &account {
                return Err(FinalChainNativeStateReadError::Invariant(format!(
                    "native account read changed during one phase: {address:?}"
                )));
            }
        } else {
            reads.accounts.insert(address, account.clone());
        }
        Ok(account)
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let value = self
            .journal
            .raw_storage(address, key)
            .map_err(state_read_error)?;
        let identity = (address, *key);
        let mut reads = self.reads.borrow_mut();
        if let Some(previous) = reads.raw.get(&identity) {
            if previous != &value {
                return Err(FinalChainNativeStateReadError::Invariant(format!(
                    "native raw read changed during one phase: {address:?}/{key:?}"
                )));
            }
        } else {
            reads.raw.insert(identity, value.clone());
        }
        Ok(value)
    }
}

#[derive(Clone, Debug)]
struct PendingInvocation {
    request: FinalChainNativeRequest,
    reads: RecordedReads,
}

/// Test-only consensus-native port backed by one bound, unpublished FinalChain session.
///
/// A successful invocation appends one replay context. `take_contexts` transfers
/// those observations to the finite integration fixture. `finish` runs the
/// one-shot rewards/end-block phase and returns both the original semantic
/// outcome and its EVM-journal mutation projection; it does not apply either.
pub struct MixedNativeExecutionPort<'a> {
    session: FinalChainNativeSession<'a>,
    pending: Option<PendingInvocation>,
    contexts: Vec<FinalChainNativeInvocationContext>,
}

impl<'a> MixedNativeExecutionPort<'a> {
    /// Wraps a session whose request binding was established by the caller.
    ///
    /// This constructor cannot establish or verify that binding itself and does
    /// not acquire publication authority. An unbound session remains usable for
    /// invocation tests, but its terminal rewards phase is rejected by FinalChain.
    pub fn new(session: FinalChainNativeSession<'a>) -> Self {
        Self {
            session,
            pending: None,
            contexts: Vec::new(),
        }
    }

    /// Takes all completed invocation contexts in native sequence order.
    pub fn take_contexts(&mut self) -> Vec<FinalChainNativeInvocationContext> {
        std::mem::take(&mut self.contexts)
    }

    /// Runs the terminal rewards/end-block phase exactly once.
    ///
    /// The returned FinalChain outcome retains the total minted reward and
    /// unpublished semantic successor. The `NativeOutcome` contains only the
    /// converted ordinary/raw mutations for explicit journal application. The
    /// context records every current-journal fact consumed by this phase.
    pub fn finish(
        &mut self,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
        journal: &dyn NativeJournalRead,
    ) -> Result<
        (
            FinalChainNativeRewardsOutcome,
            NativeOutcome,
            FinalChainNativeRewardsContext,
        ),
        NativePortError,
    > {
        let read = RecordingRead::new(journal, RecordedReads::default());
        let outcome = self
            .session
            .finish_rewards(plan, &read)
            .map_err(native_port_error)?;
        let reads = read.into_reads();
        let context = reads.rewards_context(&outcome);
        let projected = rewards_outcome(&outcome)?;
        Ok((outcome, projected, context))
    }
}

impl NativeExecutionPort for MixedNativeExecutionPort<'_> {
    fn prepare(
        &mut self,
        invocation: &NativeInvocation,
        journal: &dyn NativeJournalRead,
    ) -> Result<NativeGasQuote, NativePortError> {
        let request = consensus_request(invocation);
        let read = RecordingRead::new(journal, RecordedReads::default());
        let quote = self
            .session
            .prepare(&request, &read)
            .map_err(native_port_error)?;
        self.pending = Some(PendingInvocation {
            request,
            reads: read.into_reads(),
        });
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
        let request = consensus_request(invocation);
        let consensus_quote = FinalChainNativeGasQuote {
            invocation: consensus_id(quote.invocation),
            required_gas: quote.required_gas,
        };
        let reads = self
            .pending
            .as_ref()
            .map(|pending| pending.reads.clone())
            .unwrap_or_default();
        let read = RecordingRead::new(journal, reads);
        let result = self
            .session
            .invoke(&request, consensus_quote, &read)
            .map_err(native_port_error)?;
        let pending = self.pending.take().ok_or_else(|| {
            NativePortError::Infrastructure(
                "native invocation completed without adapter preparation".to_owned(),
            )
        })?;
        if pending.request != request {
            return Err(NativePortError::Infrastructure(
                "native invocation completed against a different adapter request".to_owned(),
            ));
        }
        let reads = read.into_reads();
        match result {
            FinalChainNativeInvocationResult::InsufficientGas { required_gas } => {
                self.contexts
                    .push(reads.invocation_context(pending.request, &[], &[]));
                Ok(NativeInvocationResult::InsufficientGas { required_gas })
            }
            FinalChainNativeInvocationResult::Completed(outcome) => {
                let projected = native_outcome(&outcome)?;
                self.contexts.push(reads.invocation_context(
                    pending.request,
                    &outcome.account_mutations,
                    &outcome.raw_mutations,
                ));
                Ok(NativeInvocationResult::Completed(projected))
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

fn native_outcome(outcome: &FinalChainNativeOutcome) -> Result<NativeOutcome, NativePortError> {
    Ok(NativeOutcome {
        status: native_status(&outcome.status),
        gas_used: outcome.gas_used,
        output: outcome.output.clone(),
        account_mutations: ordinary_mutations(&outcome.account_mutations),
        raw_mutations: raw_mutations(&outcome.raw_mutations)?,
        logs: outcome
            .logs
            .iter()
            .map(|log| ExecutionLog {
                address: log.address,
                topics: log.topics.clone(),
                data: log.data.clone(),
            })
            .collect(),
        diagnostic: None,
    })
}

fn rewards_outcome(
    outcome: &FinalChainNativeRewardsOutcome,
) -> Result<NativeOutcome, NativePortError> {
    Ok(NativeOutcome {
        status: NativeStatus::Success,
        gas_used: rustaxa_types::FinalChainGas::ZERO,
        output: Vec::new(),
        account_mutations: ordinary_mutations(&outcome.account_mutations),
        raw_mutations: raw_mutations(&outcome.raw_mutations)?,
        logs: Vec::new(),
        diagnostic: None,
    })
}

fn native_status(status: &FinalChainNativeStatus) -> NativeStatus {
    match status {
        FinalChainNativeStatus::Success => NativeStatus::Success,
        FinalChainNativeStatus::ContractFailure { error } => {
            NativeStatus::ContractFailure(NativeContractFailure {
                error: error.clone(),
            })
        }
    }
}

fn ordinary_mutations(
    mutations: &[FinalChainNativeOrdinaryMutation],
) -> Vec<NativeOrdinaryAccountMutation> {
    mutations
        .iter()
        .map(|mutation| match mutation {
            FinalChainNativeOrdinaryMutation::EnsureExists {
                address,
                expected_exists,
            } => NativeOrdinaryAccountMutation::EnsureExists {
                address: *address,
                expected_exists: *expected_exists,
            },
            FinalChainNativeOrdinaryMutation::Touch {
                address,
                expected_exists,
            } => NativeOrdinaryAccountMutation::Touch {
                address: *address,
                expected_exists: *expected_exists,
            },
            FinalChainNativeOrdinaryMutation::BalanceReplace {
                address,
                expected_exists,
                expected,
                replacement,
            } => NativeOrdinaryAccountMutation::Balance {
                address: *address,
                expected_exists: *expected_exists,
                expected: ExecutionBalance::new(expected.clone()),
                replacement: ExecutionBalance::new(replacement.clone()),
            },
        })
        .collect()
}

fn raw_mutations(
    mutations: &[FinalChainNativeRawMutation],
) -> Result<Vec<NativeRawMutation>, NativePortError> {
    mutations
        .iter()
        .map(|mutation| {
            let operation = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => NativeRawOperation::Put(
                    NativeRawValue::new(value.as_bytes().to_vec())
                        .map_err(|error| NativePortError::Infrastructure(error.to_string()))?,
                ),
                FinalChainNativeRawOperation::Delete => NativeRawOperation::Delete,
            };
            Ok(NativeRawMutation {
                address: mutation.address,
                key: mutation.key,
                expected: mutation.expected.clone(),
                operation,
            })
        })
        .collect()
}

fn state_read_error(error: NativeJournalReadError) -> FinalChainNativeStateReadError {
    match error {
        NativeJournalReadError::State(error) => FinalChainNativeStateReadError::State(error),
        NativeJournalReadError::Invariant(error) => {
            FinalChainNativeStateReadError::Invariant(error)
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigInt;
    use rustaxa_types::{FinalChainGas, FinalChainNonce};

    struct ChangingJournal {
        account_reads: RefCell<Vec<rustaxa_evm::contracts::NativeJournalAccount>>,
        raw_reads: RefCell<Vec<ConcreteRead<Vec<u8>>>>,
    }

    impl NativeJournalRead for ChangingJournal {
        fn account(
            &self,
            _address: [u8; 20],
        ) -> Result<rustaxa_evm::contracts::NativeJournalAccount, NativeJournalReadError> {
            self.account_reads
                .borrow_mut()
                .pop()
                .ok_or_else(|| NativeJournalReadError::Invariant("missing account read".into()))
        }

        fn raw_storage(
            &self,
            _address: [u8; 20],
            _key: &ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
            self.raw_reads
                .borrow_mut()
                .pop()
                .ok_or_else(|| NativeJournalReadError::Invariant("missing raw read".into()))
        }
    }

    fn journal_account(balance: i64) -> rustaxa_evm::contracts::NativeJournalAccount {
        rustaxa_evm::contracts::NativeJournalAccount {
            exists: true,
            nonce: FinalChainNonce::from_u64(3),
            balance: ExecutionBalance::new(BigInt::from(balance)),
        }
    }

    #[test]
    fn recorder_rejects_changed_account_and_raw_repeats() {
        let journal = ChangingJournal {
            account_reads: RefCell::new(vec![journal_account(8), journal_account(7)]),
            raw_reads: RefCell::new(vec![
                ConcreteRead::Tombstone,
                ConcreteRead::Present(Vec::new()),
            ]),
        };
        let read = RecordingRead::new(&journal, RecordedReads::default());
        assert_eq!(read.account([1; 20]).unwrap().balance, BigInt::from(7));
        assert!(matches!(
            read.account([1; 20]),
            Err(FinalChainNativeStateReadError::Invariant(error))
                if error.contains("account read changed")
        ));
        let key = ConcreteStorageKey([2; 32]);
        assert_eq!(
            read.raw_storage([3; 20], &key).unwrap(),
            ConcreteRead::Present(Vec::new())
        );
        assert!(matches!(
            read.raw_storage([3; 20], &key),
            Err(FinalChainNativeStateReadError::Invariant(error))
                if error.contains("raw read changed")
        ));
    }

    #[test]
    fn conversion_preserves_signed_balances_and_raw_classification() {
        let wide = -(BigInt::from(1_u8) << 300_usize) + BigInt::from(5_u8);
        let outcome = FinalChainNativeOutcome {
            status: FinalChainNativeStatus::Success,
            gas_used: FinalChainGas::new(9),
            output: vec![1],
            account_mutations: vec![
                FinalChainNativeOrdinaryMutation::EnsureExists {
                    address: [3; 20],
                    expected_exists: false,
                },
                FinalChainNativeOrdinaryMutation::Touch {
                    address: [4; 20],
                    expected_exists: true,
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: [4; 20],
                    expected_exists: true,
                    expected: wide.clone(),
                    replacement: &wide + BigInt::from(11_u8),
                },
            ],
            raw_mutations: vec![
                FinalChainNativeRawMutation {
                    address: [5; 20],
                    key: ConcreteStorageKey([6; 32]),
                    expected: ConcreteRead::Tombstone,
                    operation: FinalChainNativeRawOperation::Delete,
                },
                FinalChainNativeRawMutation {
                    address: [5; 20],
                    key: ConcreteStorageKey([7; 32]),
                    expected: ConcreteRead::Present(Vec::new()),
                    operation: FinalChainNativeRawOperation::Put(
                        rustaxa_consensus::native_session::FinalChainNativeRawValue::new(vec![
                            0, 9,
                        ])
                        .unwrap(),
                    ),
                },
            ],
            logs: Vec::new(),
        };
        let converted = native_outcome(&outcome).unwrap();
        assert_eq!(converted.gas_used, FinalChainGas::new(9));
        assert!(matches!(
            &converted.account_mutations[2],
            NativeOrdinaryAccountMutation::Balance {
                expected,
                replacement,
                ..
            } if expected.value() == &wide
                && replacement.value() == &(&wide + BigInt::from(11_u8))
        ));
        assert!(matches!(
            &converted.raw_mutations[0],
            NativeRawMutation {
                expected: ConcreteRead::Tombstone,
                operation: NativeRawOperation::Delete,
                ..
            }
        ));
        assert!(matches!(
            &converted.account_mutations[..2],
            [
                NativeOrdinaryAccountMutation::EnsureExists {
                    expected_exists: false,
                    ..
                },
                NativeOrdinaryAccountMutation::Touch {
                    expected_exists: true,
                    ..
                }
            ]
        ));
        assert!(matches!(
            &converted.raw_mutations[1],
            NativeRawMutation {
                expected: ConcreteRead::Present(empty),
                operation: NativeRawOperation::Put(value),
                ..
            } if empty.is_empty() && value.as_bytes() == [0, 9]
        ));
    }
}
