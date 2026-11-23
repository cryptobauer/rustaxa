//! Independent invocation replay using the existing staged native session.
//!
//! The sidecar preserves current executor reads that the legacy projection
//! cannot encode. Each call gets a fresh read witness and ordinary scratch
//! overlay while this session retains its semantic/native state. Kernel and
//! serializer code are reused; the pinned Go oracle remains the independent
//! serialization source. Nothing in this module publishes either database.

use super::*;
use crate::native_projection_context::{
    FinalChainNativeInvocationContext, FinalChainNativeProjectionContext, FinalChainNativeRawRead,
    FinalChainNativeRewardsContext,
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

/// Checks the surviving ordered raw effects for one transaction.
pub(in crate::final_chain) fn validate_native_context_transaction_storage(
    context: &crate::native_projection_context::FinalChainNativeProjectionContext,
    effect: &crate::concrete_state_projection::FinalChainConcreteTransactionEffect,
) -> anyhow::Result<BTreeSet<([u8; 20], [u8; 32])>> {
    let fold = crate::native_projection_context::fold_lifecycle_raw(context)?;
    let transaction = u32::try_from(effect.index)?;
    let delta = fold
        .transactions
        .get(&transaction)
        .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_NATIVE_LIFECYCLE_TRANSACTION_MISSING"))?;
    let expected = delta
        .values
        .iter()
        .map(|((address, key), value)| ((*address, key.0), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let actual = effect
        .storage
        .iter()
        .map(|row| ((row.contract, row.key), row.value.clone()))
        .collect::<BTreeMap<_, _>>();
    anyhow::ensure!(
        expected == actual,
        "FINAL_CHAIN_NATIVE_CONTEXT_TRANSACTION_STORAGE_MISMATCH"
    );
    Ok(delta
        .initially_absent
        .iter()
        .filter(|key| {
            delta.created.contains(key) && delta.values.get(key).is_some_and(Vec::is_empty)
        })
        .map(|(address, key)| (*address, key.0))
        .collect())
}

/// Checks final values and the absence baseline of each reset account.
pub(in crate::final_chain) fn validate_native_context_final_storage(
    context: &crate::native_projection_context::FinalChainNativeProjectionContext,
    rows: &[FinalChainConcreteStorageProjection],
) -> anyhow::Result<()> {
    let fold = crate::native_projection_context::fold_lifecycle_raw(context)?;
    let actual = rows
        .iter()
        .map(|row| ((row.contract, ConcreteStorageKey(row.key)), &row.value))
        .collect::<BTreeMap<_, _>>();
    for (key, value) in &fold.final_values {
        anyhow::ensure!(
            actual.get(key).is_some_and(|actual| *actual == value),
            "FINAL_CHAIN_NATIVE_CONTEXT_FINAL_STORAGE_MISMATCH"
        );
    }
    for (key, value) in &actual {
        anyhow::ensure!(
            !fold.absent_accounts.contains(&key.0)
                || fold.final_values.contains_key(key)
                || value.is_empty(),
            "FINAL_CHAIN_NATIVE_CONTEXT_FINAL_STORAGE_RESET_MISMATCH"
        );
    }
    Ok(())
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

/// Evidence for the non-batched empty-list scheduler input.
pub(in crate::final_chain) struct AbsentSlashingCleanupInput;

pub(in crate::final_chain) fn absent_slashing_cleanup_input(
    accounts: &HashMap<[u8; 20], Account>,
    context: &FinalChainNativeProjectionContext,
) -> anyhow::Result<Option<AbsentSlashingCleanupInput>> {
    validate_native_context_reward_accounts(accounts, &context.rewards.accounts)?;
    crate::native_projection_context::fold_lifecycle_raw(context)?;
    if accounts.contains_key(&SLASHING_CONTRACT_ADDRESS) {
        return Ok(None);
    }
    let mut account_reads = context
        .rewards
        .accounts
        .iter()
        .filter(|(address, _)| *address == SLASHING_CONTRACT_ADDRESS);
    let Some((_, account)) = account_reads.next() else {
        return Ok(None);
    };
    if account_reads.next().is_some()
        || account.exists
        || account.nonce != rustaxa_types::FinalChainNonce::default()
        || account.balance != Default::default()
    {
        return Ok(None);
    }
    let mut list_key = [0_u8; 32];
    list_key[31] = 2;
    let list_key = ConcreteStorageKey(list_key);
    let mut reads = context
        .rewards
        .raw_reads
        .iter()
        .filter(|read| read.address == SLASHING_CONTRACT_ADDRESS && read.key == list_key);
    let Some(read) = reads.next() else {
        return Ok(None);
    };
    if reads.next().is_some() || !matches!(read.value, ConcreteRead::Absent) {
        return Ok(None);
    }
    if context
        .rewards
        .raw_mutations
        .iter()
        .any(|mutation| mutation.address == SLASHING_CONTRACT_ADDRESS && mutation.key == list_key)
    {
        return Ok(None);
    }
    Ok(Some(AbsentSlashingCleanupInput))
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
            lifecycle: vec![crate::native_projection_context::FinalChainLifecycleReceipt {
                transaction_position: 0, transaction_hash: [0; 32], token: 1, event_count: 3,
                events: vec![crate::native_projection_context::FinalChainLifecycleEvent::NativeStarted { transaction: 0, sequence: 0 }, crate::native_projection_context::FinalChainLifecycleEvent::NativeObserved { transaction: 0, sequence: 0 }, crate::native_projection_context::FinalChainLifecycleEvent::TransactionEnd { observations: 1 }],
            }],
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
                raw_reads: vec![FinalChainNativeRawRead { address: DPOS_CONTRACT_ADDRESS, key, value: ConcreteRead::Absent }],
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
        context.invocations[0].raw_reads[0].value = ConcreteRead::Present(vec![9]);
        context.invocations[0].raw_mutations = vec![delete.clone()];
        assert!(
            validate_native_context_transaction_storage(&context, &effect)
                .unwrap()
                .is_empty()
        );
        context.invocations[0].raw_mutations = vec![put, delete];
        context.invocations[0].raw_mutations[0].expected = ConcreteRead::Present(vec![8]);
        context.invocations[0].raw_reads[0].value = ConcreteRead::Present(vec![8]);
        assert!(
            validate_native_context_transaction_storage(&context, &effect)
                .unwrap()
                .is_empty()
        );
        let mut wrong = effect;
        wrong.storage[0].value = vec![9];
        assert!(validate_native_context_transaction_storage(&context, &wrong).is_err());
    }
    #[test]
    fn reset_baseline_rejects_old_catalog_rows() {
        use crate::native_projection_context::{
            FinalChainLifecycleEvent as E, FinalChainLifecycleReceipt,
        };
        let address = DPOS_CONTRACT_ADDRESS;
        let context = FinalChainNativeProjectionContext {
            request_id: [0; 32],
            projection_hash: [0; 32],
            invocations: Vec::new(),
            rewards: Default::default(),
            lifecycle: vec![FinalChainLifecycleReceipt {
                transaction_position: 0,
                transaction_hash: [0; 32],
                token: 1,
                event_count: 4,
                events: vec![
                    E::FrameOpened { parent: None },
                    E::AccountCreated {
                        address,
                        frame: Some(0),
                        saved_absent: true,
                    },
                    E::AccountReset {
                        address,
                        creation: 1,
                        rollback_frame: 0,
                    },
                    E::TransactionEnd { observations: 0 },
                ],
            }],
        };
        assert!(validate_native_context_final_storage(&context, &[]).is_ok());
        let row = FinalChainConcreteStorageProjection {
            contract: address,
            key: [2; 32],
            value: vec![9],
        };
        assert!(validate_native_context_final_storage(&context, &[row]).is_err());
    }
}
