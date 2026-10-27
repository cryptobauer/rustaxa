//! Go reader identities and callback-driven caches for a private trace sequence.
//!
//! Writer mutations do not invalidate a separate reader's cache. A committed
//! miss is not cached. A first dirty-empty callback is forwarded and cached;
//! the subsequent empty cache hit suppresses the callback. Ordered native
//! mutations prove dirty callback provenance without guessing from absence.
use super::*;

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum Reader {
    Dpos,
    DposSlashing,
    Slashing,
    SlashingValidator,
}

#[derive(Default)]
pub(super) struct TraceReadCaches {
    cached: BTreeMap<(Reader, [u8; 20], ConcreteStorageKey), Vec<u8>>,
    dirty: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
}

impl TraceReadCaches {
    pub(super) fn record(&mut self, mutations: &[FinalChainNativeRawMutation]) {
        for mutation in mutations {
            self.dirty.insert(
                (mutation.address, mutation.key),
                match &mutation.operation {
                    FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
                    FinalChainNativeRawOperation::Delete => Vec::new(),
                },
            );
        }
    }

    fn read(
        &mut self,
        reader: Reader,
        address: [u8; 20],
        key: ConcreteStorageKey,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<Option<Vec<u8>>, FinalChainNativeSessionError> {
        if let Some(value) = self.cached.get(&(reader, address, key)) {
            return Ok((!value.is_empty()).then(|| value.clone()));
        }
        let observed = state.raw_storage(address, &key)?;
        let callback = if let Some(dirty) = self.dirty.get(&(address, key)) {
            let valid = match &observed {
                ConcreteRead::Present(value) => value == dirty,
                ConcreteRead::Absent | ConcreteRead::Tombstone => dirty.is_empty(),
            };
            if !valid {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "trace dirty raw provenance mismatch".into(),
                ));
            }
            Some(dirty.clone())
        } else {
            match observed {
                ConcreteRead::Present(value) if !value.is_empty() => Some(value),
                ConcreteRead::Present(_) | ConcreteRead::Absent | ConcreteRead::Tombstone => None,
            }
        };
        if let Some(value) = &callback {
            self.cached.insert((reader, address, key), value.clone());
        }
        Ok(callback)
    }
}

impl FinalChainNativeSession<'_> {
    fn trace_stake(
        &self,
        reader: Reader,
        validator: [u8; 20],
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<U256, FinalChainNativeSessionError> {
        let value = self.trace_readers.borrow_mut().read(
            reader,
            DPOS_CONTRACT_ADDRESS,
            ConcreteStorageKey(concrete_storage_key(&[&[0, 0], &validator])),
            state,
        )?;
        let Some(value) = value else {
            return Ok(U256::zero());
        };
        let row = exact_rlp(&value, "trace validator")?;
        let extended = row
            .item_count()
            .map_err(|error| raw_integrity("trace validator", error))?
            == 2;
        let base = if extended {
            row.val_at::<u16>(1)
                .map_err(|error| raw_integrity("trace validator custody count", error))?;
            row.at(0)
                .map_err(|error| raw_integrity("trace validator", error))?
        } else {
            row
        };
        if base
            .item_count()
            .map_err(|error| raw_integrity("trace validator", error))?
            != 4
        {
            return Err(FinalChainNativeSessionError::RawIntegrity(
                "trace validator shape".into(),
            ));
        }
        base.val_at::<u16>(1)
            .map_err(|error| raw_integrity("trace commission", error))?;
        base.val_at::<u64>(2)
            .map_err(|error| raw_integrity("trace commission period", error))?;
        base.val_at::<u64>(3)
            .map_err(|error| raw_integrity("trace reward head", error))?;
        base.val_at(0)
            .map_err(|error| raw_integrity("trace stake", error))
    }

    pub(super) fn trace_jail(
        &self,
        reader: Reader,
        validator: [u8; 20],
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<Option<u64>, FinalChainNativeSessionError> {
        self.trace_readers
            .borrow_mut()
            .read(
                reader,
                SLASHING_CONTRACT_ADDRESS,
                ConcreteStorageKey(concrete_storage_key(&[&[0], &validator])),
                state,
            )?
            .map(|value| {
                exact_rlp(&value, "trace jail block")?
                    .as_val::<u64>()
                    .map_err(|error| raw_integrity("trace jail block", error))
            })
            .transpose()
    }

    pub(super) fn trace_jailed(
        &self,
        reader: Reader,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<Vec<[u8; 20]>, FinalChainNativeSessionError> {
        let mut key = [0; 32];
        key[31] = 2;
        let value = self.trace_readers.borrow_mut().read(
            reader,
            SLASHING_CONTRACT_ADDRESS,
            ConcreteStorageKey(key),
            state,
        )?;
        let Some(value) = value.filter(|value| !value.is_empty()) else {
            return Ok(Vec::new());
        };
        exact_rlp(&value, "trace jailed list")?
            .as_list::<ethereum_types::H160>()
            .map(|items| items.into_iter().map(|address| address.0).collect())
            .map_err(|error| raw_integrity("trace jailed list", error))
    }

    pub(super) fn trace_validator_member(
        &self,
        validator: [u8; 20],
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<bool, FinalChainNativeSessionError> {
        Ok(
            self.trace_stake(Reader::SlashingValidator, validator, state)?
                >= self
                    .final_chain
                    .dpos_eligibility_balance_threshold
                    .as_u256(),
        )
    }

    fn trace_is_jailed(
        &self,
        validator: [u8; 20],
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<bool, FinalChainNativeSessionError> {
        if !self.final_chain.cacti_active(self.eligibility_period)
            || !self.final_chain.magnolia_active(self.eligibility_period)
        {
            return Ok(false);
        }
        Ok(self
            .trace_jail(Reader::DposSlashing, validator, state)?
            .is_some_and(|end| end >= self.eligibility_period.as_u64()))
    }

    fn trace_votes(
        &self,
        validator: [u8; 20],
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<u64, FinalChainNativeSessionError> {
        let stake = self.trace_stake(Reader::Dpos, validator, state)?;
        dpos_vote_count(
            &stake.to_big_endian(),
            self.final_chain.dpos_eligibility_balance_threshold,
            self.final_chain.dpos_vote_eligibility_balance_step,
            self.final_chain.dpos_validator_maximum_stake,
        )
        .map_err(custody::map_kernel_error)
    }

    pub(super) fn invoke_trace_eligibility(
        &self,
        transaction: DposTransaction,
        diagnostic: Option<String>,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let output = match transaction {
            DposTransaction::GetTotalEligibleVotesCount => {
                let value = self
                    .trace_readers
                    .borrow_mut()
                    .read(
                        Reader::Dpos,
                        DPOS_CONTRACT_ADDRESS,
                        ConcreteStorageKey(concrete_storage_key(&[&[4]])),
                        state,
                    )?
                    .unwrap_or_default();
                if value.len() > 8 {
                    return Err(FinalChainNativeSessionError::RawIntegrity(
                        "trace eligible vote scalar width".into(),
                    ));
                }
                let mut total = value
                    .iter()
                    .fold(0_u64, |total, byte| total << 8 | u64::from(*byte));
                for validator in self.trace_jailed(Reader::DposSlashing, state)? {
                    total = total.wrapping_sub(self.trace_votes(validator, state)?);
                }
                Some(abi_word_from_u64(total).to_vec())
            }
            DposTransaction::GetValidatorEligibleVotesCount(Ok(validator)) => {
                let votes = if self.trace_is_jailed(validator, state)? {
                    0
                } else {
                    self.trace_votes(validator, state)?
                };
                Some(abi_word_from_u64(votes).to_vec())
            }
            DposTransaction::IsValidatorEligible(Ok(validator)) => {
                let eligible = !self.trace_is_jailed(validator, state)?
                    && self.trace_stake(Reader::Dpos, validator, state)?
                        >= self
                            .final_chain
                            .dpos_eligibility_balance_threshold
                            .as_u256();
                Some(abi_word_from_bool(eligible).to_vec())
            }
            _ => None,
        };
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status: if output.is_some() {
                    FinalChainNativeStatus::Success
                } else {
                    FinalChainNativeStatus::ContractFailure {
                        error: diagnostic.unwrap_or_default(),
                    }
                },
                gas_used: quote.required_gas,
                output: output.unwrap_or_default(),
                account_mutations: Vec::new(),
                raw_mutations: Vec::new(),
                logs: Vec::new(),
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    struct State {
        value: RefCell<ConcreteRead<Vec<u8>>>,
        reads: Cell<usize>,
        fail: Cell<bool>,
    }
    impl FinalChainNativeStateRead for State {
        fn raw_storage(
            &self,
            _: [u8; 20],
            _: &ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, FinalChainNativeStateReadError> {
            self.reads.set(self.reads.get() + 1);
            if self.fail.get() {
                return Err(FinalChainNativeStateReadError::Invariant(
                    "cache read failed".into(),
                ));
            }
            Ok(self.value.borrow().clone())
        }
    }
    fn state(value: ConcreteRead<Vec<u8>>) -> State {
        State {
            value: RefCell::new(value),
            reads: Cell::new(0),
            fail: Cell::new(false),
        }
    }
    fn mutation(value: Vec<u8>) -> FinalChainNativeRawMutation {
        FinalChainNativeRawMutation {
            address: DPOS_CONTRACT_ADDRESS,
            key: ConcreteStorageKey([7; 32]),
            expected: ConcreteRead::Absent,
            operation: if value.is_empty() {
                FinalChainNativeRawOperation::Delete
            } else {
                FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(value).unwrap())
            },
        }
    }
    #[test]
    fn native_integration_reader_caches_are_independent_and_do_not_cache_misses() {
        let state = state(ConcreteRead::Absent);
        let mut cache = TraceReadCaches::default();
        let key = ConcreteStorageKey([7; 32]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            None
        );
        cache.record(&[mutation(vec![1])]);
        *state.value.borrow_mut() = ConcreteRead::Present(vec![1]);
        let mut first = cache
            .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
            .unwrap()
            .unwrap();
        first[0] = 99;
        cache.record(&[mutation(vec![2])]);
        *state.value.borrow_mut() = ConcreteRead::Present(vec![2]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            Some(vec![1])
        );
        assert_eq!(
            cache
                .read(
                    Reader::SlashingValidator,
                    DPOS_CONTRACT_ADDRESS,
                    key,
                    &state
                )
                .unwrap(),
            Some(vec![2])
        );
        state.fail.set(true);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            Some(vec![1])
        );
        assert_eq!(state.reads.get(), 3);
    }
    #[test]
    fn native_integration_dirty_empty_forwards_once_but_committed_absence_does_not() {
        let state = state(ConcreteRead::Tombstone);
        let mut cache = TraceReadCaches::default();
        let key = ConcreteStorageKey([7; 32]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            None
        );
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            None
        );
        cache.record(&[mutation(vec![])]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            Some(vec![])
        );
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            None
        );
        cache.record(&[mutation(vec![3])]);
        *state.value.borrow_mut() = ConcreteRead::Present(vec![3]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            None
        );
        assert_eq!(
            cache
                .read(
                    Reader::SlashingValidator,
                    DPOS_CONTRACT_ADDRESS,
                    key,
                    &state
                )
                .unwrap(),
            Some(vec![3])
        );
        assert_eq!(state.reads.get(), 4);
    }
    #[test]
    fn native_integration_reader_failure_and_dirty_mismatch_do_not_create_cache_entries() {
        let state = state(ConcreteRead::Present(vec![1]));
        let mut cache = TraceReadCaches::default();
        let key = ConcreteStorageKey([7; 32]);
        state.fail.set(true);
        assert!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .is_err()
        );
        state.fail.set(false);
        cache.record(&[mutation(vec![2])]);
        assert!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .is_err()
        );
        *state.value.borrow_mut() = ConcreteRead::Present(vec![2]);
        assert_eq!(
            cache
                .read(Reader::Dpos, DPOS_CONTRACT_ADDRESS, key, &state)
                .unwrap(),
            Some(vec![2])
        );
        assert_eq!(state.reads.get(), 3);
    }
}
