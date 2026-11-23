//! Staged validator registration using the existing FinalChain business kernel.
//!
//! Admission precedes ABI unpack and proof validation. Successful serialization
//! retains Go's initial zero-stake validator write and later principal update.
//! The frame owns value transfer; registration emits no ordinary account effect.

use super::custody::*;
use super::raw::FinalChainNativeRawTrace;
use super::*;

impl FinalChainNativeSession<'_> {
    pub(super) fn prepare_registration(
        &mut self,
        request: &FinalChainNativeRequest,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        let admission = self
            .final_chain
            .native_invocation_admission(
                &DposTransaction::MalformedMutation {
                    selector: DPOS_REGISTER_VALIDATOR_SELECTOR,
                },
                request.period,
                request.depth,
                request.value.value(),
                request.supplied_gas,
                None,
            )
            .map_err(|error| {
                self.aborted = true;
                map_kernel_error(error)
            })?;
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas: admission.required_gas,
        };
        let kind = match admission.failure {
            Some(super::super::native_admission::NativeAdmissionFailure::InsufficientGas) => {
                PreparedKind::InsufficientGas
            }
            Some(super::super::native_admission::NativeAdmissionFailure::NestedBeforeFix) => {
                PreparedKind::NestedCallRejected
            }
            Some(super::super::native_admission::NativeAdmissionFailure::NonPayable) => {
                PreparedKind::NonPayable
            }
            None => match decode_registration(request) {
                Ok(registration) => {
                    PreparedKind::SelectedCustody(DposTransaction::Register(registration))
                }
                Err(error) => PreparedKind::AbiFailure(error),
            },
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind,
        });
        Ok(quote)
    }

    pub(super) fn invoke_registration(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
        request: &FinalChainNativeRequest,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let DposTransaction::Register(mut registration) = transaction else {
            return Err(FinalChainNativeSessionError::UnsupportedOperation);
        };
        if registration.proof.len() == 65 {
            let dynamic = usize::try_from(&BigUint::from_bytes_be(&request.input[36..68]))
                .map_err(|_| {
                    FinalChainNativeSessionError::RawIntegrity("registration proof offset".into())
                })?;
            let offset = dynamic.checked_add(4 + 32 + 64).ok_or_else(|| {
                FinalChainNativeSessionError::RawIntegrity(
                    "registration proof offset overflow".into(),
                )
            })?;
            let expected = *request.input.get(offset).ok_or_else(|| {
                FinalChainNativeSessionError::RawIntegrity(
                    "registration borrowed proof range".into(),
                )
            })?;
            self.input_mutations.push(FinalChainNativeInputMutation {
                offset,
                expected,
                replacement: expected.wrapping_sub(27),
            });
        }
        let vrf_offset = usize::try_from(&BigUint::from_bytes_be(&request.input[68..100]))
            .map_err(|_| raw_integrity("registration VRF offset", "overflow"))?
            .checked_add(36)
            .ok_or_else(|| raw_integrity("registration VRF offset", "overflow"))?;
        for mutation in &self.input_mutations {
            if let Some(offset) = mutation.offset.checked_sub(vrf_offset)
                && let Some(value) = registration.vrf_key.get_mut(offset)
            {
                *value = mutation.replacement;
            }
        }
        let validator = registration.validator;
        let owner = registration.metadata.owner;
        let mut next = self.dpos_state.clone();
        let outcome = self
            .final_chain
            .apply_dpos_registration(&mut next, registration.clone(), self.pending_period)
            .map_err(map_kernel_error)?;
        let mut trace = FinalChainNativeRawTrace::new(state);
        // These business errors precede the first Go membership lookup.
        let before_lookup = matches!(
            outcome.contract_error,
            Some(
                DposContractError::WrongProof
                    | DposContractError::LegacyMessage(_)
                    | DposContractError::InsufficientDelegation
                    | DposContractError::MaxEndpointLengthExceeded
                    | DposContractError::MaxDescriptionLengthExceeded
                    | DposContractError::WrongVrfKey
                    | DposContractError::CommissionOverflow
            )
        );
        if !before_lookup {
            let expected = self
                .dpos_state
                .validator_order
                .iter()
                .position(|address| *address == validator)
                .map(|index| ((index + 1) as u32).to_le_bytes().to_vec())
                .unwrap_or_default();
            authenticate(
                &mut trace,
                iterable_position_key(&[0, 5], &validator),
                &expected,
                "registration validator membership",
            )?;
            if outcome.contract_error != Some(DposContractError::ExistentValidator) {
                authenticate(
                    &mut trace,
                    delegation_key(validator, owner),
                    &[],
                    "registration owner delegation",
                )?;
                authenticate(
                    &mut trace,
                    reward_node_key(validator, self.pending_period.as_u64()),
                    &[],
                    "registration reward node",
                )?;
            }
        }
        let success = outcome.status_code == 1;
        if success {
            let mut initial = next.clone();
            initial
                .total_stakes
                .insert(validator, StoredDposTokenAmount::default());
            checked_put(
                &mut trace,
                validator_key(validator),
                ExpectedRaw::Empty,
                self.encode_validator_row(&initial, validator)?,
                "registration initial validator",
            )?;
            checked_put(
                &mut trace,
                validator_info_key(validator),
                ExpectedRaw::Empty,
                encode_validator_info(&next.validator_metadata[&validator]),
                "registration info",
            )?;
            checked_put(
                &mut trace,
                rewards_key(validator),
                ExpectedRaw::Empty,
                encode_rewards(&next, validator),
                "registration rewards",
            )?;
            checked_put(
                &mut trace,
                validator_owner_key(validator),
                ExpectedRaw::Empty,
                owner.to_vec(),
                "registration owner",
            )?;
            let offset = vrf_offset;
            self.input_aliases.push(FinalChainNativeInputAlias {
                scope: if self.origin == NativeSessionOrigin::PendingPeriod {
                    FinalChainNativeInputAliasScope::OwnedInputOnly
                } else {
                    FinalChainNativeInputAliasScope::SharedJournal
                },
                address: DPOS_CONTRACT_ADDRESS,
                key: validator_vrf_key(validator),
                offset,
                length: registration.vrf_key.len(),
            });
            self.borrowed_vrf_keys
                .insert(validator, validator_vrf_key(validator));
            checked_put(
                &mut trace,
                validator_vrf_key(validator),
                ExpectedRaw::Empty,
                registration.vrf_key,
                "registration VRF",
            )?;
            create_iterable(
                &mut trace,
                &[0, 5],
                &self
                    .dpos_state
                    .validator_order
                    .iter()
                    .map(|address| address.to_vec())
                    .collect::<Vec<_>>(),
                validator.to_vec(),
            )?;
            if BigUint::from_bytes_be(&registration.stake) != BigUint::default() {
                checked_put(
                    &mut trace,
                    delegation_key(validator, owner),
                    ExpectedRaw::Empty,
                    encode_delegation(&next, validator, owner)?,
                    "registration delegation",
                )?;
                let memberships = self
                    .dpos_state
                    .delegator_validators
                    .get(&owner)
                    .into_iter()
                    .flatten()
                    .map(|address| address.to_vec())
                    .collect::<Vec<_>>();
                create_iterable(
                    &mut trace,
                    &delegator_validators_prefix(owner),
                    &memberships,
                    validator.to_vec(),
                )?;
                trace.put(
                    DPOS_CONTRACT_ADDRESS,
                    validator_key(validator),
                    self.encode_validator_row(&next, validator)?,
                )?;
            }
            let node = next
                .reward_reference_graph
                .load_node(&crate::dpos_reward_graph::NodeKey {
                    validator,
                    block: self.pending_period.as_u64(),
                })
                .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
            checked_put(
                &mut trace,
                reward_node_key(validator, self.pending_period.as_u64()),
                ExpectedRaw::Empty,
                encode_node(&node),
                "registration reward reference",
            )?;
            self.dpos_state = next;
        }
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status: if success {
                    FinalChainNativeStatus::Success
                } else {
                    FinalChainNativeStatus::ContractFailure {
                        error: outcome
                            .contract_error
                            .as_ref()
                            .map(DposContractError::legacy_message)
                            .unwrap_or_default(),
                    }
                },
                gas_used: quote.required_gas,
                output: outcome.code_retval,
                account_mutations: Vec::new(),
                raw_mutations: trace.finish(),
                logs: outcome
                    .logs
                    .into_iter()
                    .map(|log| FinalChainCallLog {
                        address: log.address,
                        topics: log.topics,
                        data: log.data,
                    })
                    .collect(),
            },
        ))
    }
}

fn authenticate(
    trace: &mut FinalChainNativeRawTrace<'_>,
    key: ConcreteStorageKey,
    expected: &[u8],
    name: &str,
) -> Result<(), FinalChainNativeSessionError> {
    let current = trace.current(DPOS_CONTRACT_ADDRESS, key)?;
    let valid = match current {
        ConcreteRead::Present(bytes) => bytes == expected,
        ConcreteRead::Absent | ConcreteRead::Tombstone => expected.is_empty(),
    };
    if !valid {
        return Err(FinalChainNativeSessionError::RawIntegrity(name.to_owned()));
    }
    Ok(())
}

fn decode_registration(request: &FinalChainNativeRequest) -> Result<DposRegistration, String> {
    let data = &request.input[4..];
    let word = |index: usize| {
        data.get(index..index + 32).ok_or_else(|| {
            format!(
                "abi: cannot marshal in to go type: length insufficient {} require {}",
                data.len(),
                index + 32
            )
        })
    };
    let dynamic = |index: usize| -> Result<Vec<u8>, String> {
        let end = BigUint::from_bytes_be(word(index)?) + BigUint::from(32_u8);
        let available = BigUint::from(data.len());
        if end > available {
            return Err(format!(
                "abi: cannot marshal in to go slice: offset {end} would go over slice boundary (len={available})"
            ));
        }
        if end.bits() > 63 {
            return Err(format!("abi offset larger than int64: {end}"));
        }
        let offset = usize::try_from(&end).expect("bounded ABI offset");
        let total = &end + BigUint::from_bytes_be(&data[offset - 32..offset]);
        if total.bits() > 63 {
            return Err(format!("abi length larger than int64: {total}"));
        }
        if total > available {
            return Err(format!(
                "abi: cannot marshal in to go type: length insufficient {available} require {total}"
            ));
        }
        Ok(data[offset..usize::try_from(&total).expect("bounded ABI length")].to_vec())
    };
    let validator = word(0)?[12..].try_into().expect("ABI address width");
    let proof = dynamic(32)?;
    let vrf_key = dynamic(64)?;
    let commission = u16::from_be_bytes(word(96)?[30..].try_into().expect("ABI uint16 width"));
    let description = dynamic(128)?;
    let endpoint = dynamic(160)?;
    let value = request.value.value().to_bytes_be();
    let stake = if request.value.value().bits() <= 256 {
        let mut fixed = vec![0; 32];
        fixed[32 - value.len()..].copy_from_slice(&value);
        fixed
    } else {
        value
    };
    Ok(DposRegistration {
        validator,
        stake,
        vrf_key,
        proof,
        metadata: DposValidatorMetadata {
            owner: request.caller,
            commission,
            last_commission_change: 0,
            description,
            endpoint,
        },
    })
}
