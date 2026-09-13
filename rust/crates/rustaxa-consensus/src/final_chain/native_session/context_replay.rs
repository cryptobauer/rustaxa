//! Independent invocation replay using the existing staged native session.
//!
//! The sidecar preserves current executor reads that the legacy projection
//! cannot encode. Each call gets a fresh read witness and ordinary scratch
//! overlay while this session retains its semantic/native state. Kernel and
//! serializer code are reused; the pinned Go oracle remains the independent
//! serialization source. Nothing in this module publishes either database.

use super::*;
use crate::native_projection_context::{
    FinalChainNativeInvocationContext, FinalChainNativeRawRead, FinalChainNativeRewardsContext,
};
use std::cell::RefCell;
use std::collections::BTreeSet;

/// Read-only evidence for one invocation or terminal reward phase.
struct ContextRead<'a> {
    accounts: &'a [([u8; 20], FinalChainNativeAccount)],
    raw_reads: &'a [FinalChainNativeRawRead],
    consumed_accounts: RefCell<BTreeSet<[u8; 20]>>,
    consumed_raw: RefCell<BTreeSet<([u8; 20], ConcreteStorageKey)>>,
}

impl<'a> ContextRead<'a> {
    fn new(
        accounts: &'a [([u8; 20], FinalChainNativeAccount)],
        raw_reads: &'a [FinalChainNativeRawRead],
    ) -> anyhow::Result<Self> {
        crate::native_projection_context::validate_read_sets(accounts, raw_reads)?;
        Ok(Self {
            accounts,
            raw_reads,
            consumed_accounts: RefCell::new(BTreeSet::new()),
            consumed_raw: RefCell::new(BTreeSet::new()),
        })
    }

    /// Rejects unused evidence as well as the missing reads rejected by the port.
    fn validate_consumed(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.consumed_accounts.borrow().len() == self.accounts.len()
                && self.consumed_raw.borrow().len() == self.raw_reads.len(),
            "FINAL_CHAIN_NATIVE_CONTEXT_UNUSED_READ"
        );
        Ok(())
    }
}

impl FinalChainNativeStateRead for ContextRead<'_> {
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<FinalChainNativeAccount, FinalChainNativeStateReadError> {
        let account = self
            .accounts
            .iter()
            .find(|(key, _)| key == &address)
            .ok_or_else(|| {
                FinalChainNativeStateReadError::Invariant(format!(
                    "missing invocation account read: {address:?}"
                ))
            })?;
        self.consumed_accounts.borrow_mut().insert(address);
        Ok(account.1.clone())
    }

    fn raw_storage(
        &self,
        address: [u8; 20],
        key: &ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
        let read = self
            .raw_reads
            .iter()
            .find(|read| read.address == address && &read.key == key)
            .ok_or_else(|| {
                FinalChainNativeStateReadError::Invariant(format!(
                    "missing invocation raw read: {address:?}/{key:?}"
                ))
            })?;
        self.consumed_raw.borrow_mut().insert((address, *key));
        Ok(read.value.clone())
    }
}

impl FinalChainNativeSession<'_> {
    /// Replays one bound context and compares all original native outcome facts.
    ///
    /// Ordinary effects are only compared, never applied to FinalChain accounts.
    /// The caller's transaction projection owns their surviving values. A failure
    /// poisons this private replay session; it can never contaminate publication.
    pub(in crate::final_chain) fn replay_context(
        &mut self,
        context: &FinalChainNativeInvocationContext,
        invocation: &FinalChainConcreteInvocation,
    ) -> anyhow::Result<()> {
        let result = (|| {
            let read = ContextRead::new(&context.accounts, &context.raw_reads)?;
            let quote = self.prepare(&context.request, &read)?;
            anyhow::ensure!(
                quote.required_gas.as_u64() == invocation.required_gas,
                "FINAL_CHAIN_NATIVE_CONTEXT_REQUIRED_GAS_MISMATCH"
            );
            let outcome = self.invoke(&context.request, quote, &read)?;
            read.validate_consumed()?;
            let (gas_used, output, error, logs, accounts, raw) = match outcome {
                FinalChainNativeInvocationResult::InsufficientGas { required_gas } => {
                    anyhow::ensure!(
                        required_gas == quote.required_gas,
                        "FINAL_CHAIN_NATIVE_CONTEXT_REQUIRED_GAS_MISMATCH"
                    );
                    (
                        0_u64,
                        Vec::new(),
                        "out of gas".to_owned(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    )
                }
                FinalChainNativeInvocationResult::Completed(outcome) => {
                    let error = match outcome.status {
                        FinalChainNativeStatus::Success => String::new(),
                        FinalChainNativeStatus::ContractFailure { error } => error,
                    };
                    (
                        outcome.gas_used.as_u64(),
                        outcome.output,
                        error,
                        outcome.logs,
                        outcome.account_mutations,
                        outcome.raw_mutations,
                    )
                }
            };
            anyhow::ensure!(
                gas_used == invocation.gas_used
                    && output == invocation.output
                    && error == invocation.error,
                "FINAL_CHAIN_NATIVE_CONTEXT_OUTCOME_MISMATCH"
            );
            let expected_logs = invocation
                .logs
                .iter()
                .map(|log| FinalChainCallLog {
                    address: log.address,
                    topics: log.topics.iter().map(|topic| topic.topic).collect(),
                    data: log.data.clone(),
                })
                .collect::<Vec<_>>();
            anyhow::ensure!(
                logs == expected_logs,
                "FINAL_CHAIN_NATIVE_CONTEXT_LOG_MISMATCH"
            );
            anyhow::ensure!(
                accounts == context.ordinary_mutations && raw == context.raw_mutations,
                "FINAL_CHAIN_NATIVE_CONTEXT_MUTATION_MISMATCH"
            );
            anyhow::ensure!(
                (invocation.disposition == FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL && error.is_empty())
                    || (invocation.disposition == FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED
                        && !error.is_empty())
                    || (invocation.disposition == crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
                        && error.is_empty()),
                "FINAL_CHAIN_CONCRETE_INVOCATION_DISPOSITION_ERROR_MISMATCH"
            );
            Ok(())
        })();
        if result.is_err() {
            self.aborted = true;
        }
        result
    }

    /// Borrows a clone for existing semantic/projection validators only.
    pub(in crate::final_chain) fn projection_snapshot(&self) -> DposSnapshot {
        self.dpos_state.clone()
    }

    /// Consumes replay at the same opaque rewards boundary as actual execution.
    /// Exact read consumption and both ordered effect streams are checked before
    /// the caller receives any candidate semantic state for final validation.
    pub(in crate::final_chain) fn replay_rewards_context(
        mut self,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
        context: &FinalChainNativeRewardsContext,
    ) -> anyhow::Result<FinalChainNativeRewardsOutcome> {
        let read = ContextRead::new(&context.accounts, &context.raw_reads)?;
        let outcome = self.finish_rewards(plan, &read)?;
        read.validate_consumed()?;
        anyhow::ensure!(
            outcome.account_mutations == context.ordinary_mutations
                && outcome.raw_mutations == context.raw_mutations,
            "FINAL_CHAIN_NATIVE_REWARDS_CONTEXT_MUTATION_MISMATCH"
        );
        Ok(outcome)
    }
}

/// Binds each transaction's touched raw rows to the last ordered setter call.
/// This is specific to the preexisting DPoS account in the bounded CALL route.
pub(in crate::final_chain) fn validate_native_context_transaction_storage(
    context: &crate::native_projection_context::FinalChainNativeProjectionContext,
    effect: &crate::concrete_state_projection::FinalChainConcreteTransactionEffect,
) -> anyhow::Result<BTreeSet<([u8; 20], [u8; 32])>> {
    let mut expected = BTreeMap::new();
    let mut initially_absent = BTreeSet::new();
    let mut created = BTreeSet::new();
    for invocation in &context.invocations {
        if u64::from(invocation.request.id.transaction.as_u32()) == effect.index {
            for mutation in &invocation.raw_mutations {
                let key = (mutation.address, mutation.key.0);
                let absent = match &mutation.expected {
                    ConcreteRead::Present(value) => value.is_empty(),
                    ConcreteRead::Absent | ConcreteRead::Tombstone => true,
                };
                if !expected.contains_key(&key) && absent {
                    initially_absent.insert(key);
                }
                if matches!(mutation.operation, FinalChainNativeRawOperation::Put(_)) {
                    created.insert(key);
                }
                collect_raw_values(&mut expected, std::slice::from_ref(mutation));
            }
        }
    }
    let actual = effect
        .storage
        .iter()
        .map(|row| ((row.contract, row.key), row.value.clone()))
        .collect::<BTreeMap<_, _>>();
    anyhow::ensure!(
        expected == actual,
        "FINAL_CHAIN_NATIVE_CONTEXT_TRANSACTION_STORAGE_MISMATCH"
    );
    Ok(initially_absent
        .into_iter()
        .filter(|key| created.contains(key) && expected.get(key).is_some_and(Vec::is_empty))
        .collect())
}

/// Requires every native/reward setter's final value in the complete final catalog.
pub(in crate::final_chain) fn validate_native_context_final_storage(
    context: &crate::native_projection_context::FinalChainNativeProjectionContext,
    rows: &[FinalChainConcreteStorageProjection],
) -> anyhow::Result<()> {
    let mut expected = BTreeMap::new();
    for invocation in &context.invocations {
        collect_raw_values(&mut expected, &invocation.raw_mutations);
    }
    collect_raw_values(&mut expected, &context.rewards.raw_mutations);
    let actual = rows
        .iter()
        .map(|row| ((row.contract, row.key), &row.value))
        .collect::<BTreeMap<_, _>>();
    for (key, value) in &expected {
        anyhow::ensure!(
            actual.get(key).is_some_and(|actual| *actual == value),
            "FINAL_CHAIN_NATIVE_CONTEXT_FINAL_STORAGE_MISMATCH"
        );
    }
    Ok(())
}

fn collect_raw_values(
    values: &mut BTreeMap<([u8; 20], [u8; 32]), Vec<u8>>,
    mutations: &[FinalChainNativeRawMutation],
) {
    for mutation in mutations {
        let value = match &mutation.operation {
            FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
            FinalChainNativeRawOperation::Delete => Vec::new(),
        };
        values.insert((mutation.address, mutation.key.0), value);
    }
}

/// Binds terminal reward reads to the independently validated post-transaction
/// account projection. Numeric balances retain their complete width; absent
/// accounts require zero nonce and balance. No account state is changed.
pub(in crate::final_chain) fn validate_native_context_reward_accounts(
    accounts: &HashMap<[u8; 20], Account>,
    reads: &[([u8; 20], FinalChainNativeAccount)],
) -> anyhow::Result<()> {
    for (address, read) in reads {
        let expected = accounts.get(address);
        let balance = expected
            .map(|account| {
                num_bigint::BigInt::from(BigUint::from_bytes_be(
                    &account.balance.as_u256().to_big_endian(),
                ))
            })
            .unwrap_or_default();
        let nonce = expected
            .map(|account| account.nonce.clone())
            .unwrap_or_default();
        anyhow::ensure!(
            read.exists == expected.is_some() && read.nonce == nonce && read.balance == balance,
            "FINAL_CHAIN_NATIVE_REWARDS_CONTEXT_ACCOUNT_MISMATCH"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concrete_state_projection::FinalChainConcreteTransactionEffect;
    use crate::native_projection_context::FinalChainNativeProjectionContext;

    #[test]
    fn reward_reads_reject_stale_wide_and_absence_aliases() {
        let address = [7; 20];
        let mut account = empty_account();
        account.nonce = FinalChainNonce::from_u64(3);
        account.balance.replace_after_mutation(U256::from(9));
        let accounts = HashMap::from([(address, account)]);
        let fact = FinalChainNativeAccount {
            exists: true,
            nonce: FinalChainNonce::from_u64(3),
            balance: 9.into(),
        };
        validate_native_context_reward_accounts(&accounts, &[(address, fact.clone())]).unwrap();
        for change in 0..5 {
            let mut stale = fact.clone();
            match change {
                0 => stale.balance = 8.into(),
                1 => stale.balance += num_bigint::BigInt::from(1_u8) << 256_usize,
                2 => stale.nonce = FinalChainNonce::from_u64(2),
                3 => stale.exists = false,
                _ => stale.balance = (-1).into(),
            }
            assert!(
                validate_native_context_reward_accounts(&accounts, &[(address, stale)]).is_err()
            );
        }
        let absent = FinalChainNativeAccount {
            exists: false,
            nonce: Default::default(),
            balance: 0.into(),
        };
        validate_native_context_reward_accounts(&accounts, &[([8; 20], absent.clone())]).unwrap();
        assert!(validate_native_context_reward_accounts(&accounts, &[(address, absent)]).is_err());
        assert!(validate_native_context_reward_accounts(&accounts, &[([8; 20], fact)]).is_err());
    }

    #[test]
    fn transient_raw_row_requires_initial_absence_put_and_final_delete() {
        let key = ConcreteStorageKey([9; 32]);
        let put = FinalChainNativeRawMutation {
            address: DPOS_CONTRACT_ADDRESS,
            key,
            expected: ConcreteRead::Absent,
            operation: FinalChainNativeRawOperation::Put(
                FinalChainNativeRawValue::new(vec![9]).unwrap(),
            ),
        };
        let delete = FinalChainNativeRawMutation {
            address: DPOS_CONTRACT_ADDRESS,
            key,
            expected: ConcreteRead::Present(vec![9]),
            operation: FinalChainNativeRawOperation::Delete,
        };
        let mut context = FinalChainNativeProjectionContext {
            request_id: [0; 32],
            projection_hash: [0; 32],
            rewards: Default::default(),
            invocations: vec![FinalChainNativeInvocationContext {
                request: FinalChainNativeRequest {
                    id: FinalChainNativeInvocationId {
                        transaction: 0_u32.into(),
                        sequence: 0,
                    },
                    period: 1_u64.into(),
                    depth: 1,
                    kind: FinalChainNativeCallKind::Call,
                    is_static: false,
                    caller: [1; 20],
                    contract: DPOS_CONTRACT_ADDRESS,
                    state_address: DPOS_CONTRACT_ADDRESS,
                    value: FinalChainNativeValue::new(0_u8.into()),
                    input: Vec::new(),
                    supplied_gas: 100_000_u64.into(),
                },
                accounts: Vec::new(),
                raw_reads: Vec::new(),
                ordinary_mutations: Vec::new(),
                raw_mutations: vec![put.clone(), delete.clone()],
            }],
        };
        let effect = FinalChainConcreteTransactionEffect {
            storage: vec![FinalChainConcreteStorageProjection {
                contract: DPOS_CONTRACT_ADDRESS,
                key: key.0,
                value: Vec::new(),
            }],
            ..Default::default()
        };
        assert_eq!(
            validate_native_context_transaction_storage(&context, &effect).unwrap(),
            BTreeSet::from([(DPOS_CONTRACT_ADDRESS, key.0)])
        );
        // An isolated delete and deleting a preexisting row are not transient creation proofs.
        context.invocations[0].raw_mutations = vec![delete.clone()];
        assert!(
            validate_native_context_transaction_storage(&context, &effect)
                .unwrap()
                .is_empty()
        );
        context.invocations[0].raw_mutations = vec![put, delete];
        context.invocations[0].raw_mutations[0].expected = ConcreteRead::Present(vec![8]);
        assert!(
            validate_native_context_transaction_storage(&context, &effect)
                .unwrap()
                .is_empty()
        );
        let mut wrong = effect;
        wrong.storage[0].value = vec![9];
        assert!(validate_native_context_transaction_storage(&context, &wrong).is_err());
    }
}
