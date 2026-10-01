//! Staged metadata replacement for a consistent, complete validator snapshot.
//!
//! The existing FinalChain kernel owns length, ownership and log rules. This
//! adapter authenticates the owner, metadata and iterable position in Go read
//! order, then emits one irreversible metadata put. Failed reads or mismatched
//! raw/domain state poison the session without advancing semantic state.
//! Malformed ABI and inconsistent owner/info snapshots remain outside this
//! bounded adapter; ordinary frames, fees and publication remain caller-owned.

use super::custody::{encode_validator_info, validator_info_key, validator_owner_key};
use super::raw::FinalChainNativeRawTrace;
use super::*;

impl FinalChainNativeSession<'_> {
    /// Executes a decoded metadata update using the existing business kernel.
    pub(super) fn invoke_validator_info(
        &mut self,
        transaction: DposTransaction,
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let DposTransaction::SetValidatorInfo {
            owner,
            validator,
            description,
            endpoint,
        } = transaction
        else {
            return Err(FinalChainNativeSessionError::UnsupportedOperation);
        };
        let mut next = self.dpos_state.clone();
        let mut trace = FinalChainNativeRawTrace::new(state);
        // Business length failures precede all storage reads in the reference.
        if endpoint.len() <= DPOS_MAX_ENDPOINT_LENGTH
            && description.len() <= DPOS_MAX_DESCRIPTION_LENGTH
        {
            let metadata = self.dpos_state.validator_metadata.get(&validator);
            let expected_owner = metadata.map(|metadata| metadata.owner.to_vec());
            let observed_owner =
                trace.current(DPOS_CONTRACT_ADDRESS, validator_owner_key(validator))?;
            let matches_owner = match (&observed_owner, &expected_owner) {
                (ConcreteRead::Present(bytes), Some(expected)) => bytes == expected,
                (ConcreteRead::Absent, None) => true,
                (ConcreteRead::Present(bytes), None) => bytes.is_empty(),
                _ => false,
            };
            if !matches_owner {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "validator info owner raw/domain mismatch".to_owned(),
                ));
            }
            // Go treats a missing owner as the zero address. The semantic owner
            // map cannot represent its subsequent independent info lookup.
            if metadata.is_none() && owner == [0; 20] {
                return Err(FinalChainNativeSessionError::UnsupportedOperation);
            }
            if let Some(metadata) = metadata.filter(|metadata| metadata.owner == owner) {
                let observed_info =
                    trace.current(DPOS_CONTRACT_ADDRESS, validator_info_key(validator))?;
                if observed_info != ConcreteRead::Present(encode_validator_info(metadata)) {
                    return Err(FinalChainNativeSessionError::RawIntegrity(
                        "validator info row raw/domain mismatch".to_owned(),
                    ));
                }
                let position = self
                    .dpos_state
                    .validator_order
                    .iter()
                    .position(|address| *address == validator)
                    .ok_or_else(|| {
                        FinalChainNativeSessionError::RawIntegrity(
                            "validator info membership is absent".to_owned(),
                        )
                    })?;
                let key = ConcreteStorageKey(concrete_storage_key(&[&[0, 5, 2], &validator]));
                let position = u32::try_from(position + 1).map_err(|_| {
                    FinalChainNativeSessionError::RawIntegrity(
                        "validator info membership exceeds uint32".to_owned(),
                    )
                })?;
                if trace.current(DPOS_CONTRACT_ADDRESS, key)?
                    != ConcreteRead::Present(position.to_le_bytes().to_vec())
                {
                    return Err(FinalChainNativeSessionError::RawIntegrity(
                        "validator info membership raw/domain mismatch".to_owned(),
                    ));
                }
            }
        }
        let outcome = self
            .final_chain
            .apply_dpos_validator_info_update(&mut next, owner, validator, description, endpoint)
            .map_err(|error| FinalChainNativeSessionError::Domain(error.to_string()))?;
        let status = if outcome.status_code == 1 {
            let metadata = next
                .validator_metadata
                .get(&validator)
                .expect("successful kernel retains metadata");
            trace.put(
                DPOS_CONTRACT_ADDRESS,
                validator_info_key(validator),
                encode_validator_info(metadata),
            )?;
            self.dpos_state = next;
            FinalChainNativeStatus::Success
        } else {
            FinalChainNativeStatus::ContractFailure {
                error: outcome
                    .contract_error
                    .as_ref()
                    .map(DposContractError::legacy_message)
                    .unwrap_or_default(),
            }
        };
        Ok(FinalChainNativeInvocationResult::Completed(
            FinalChainNativeOutcome {
                status,
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
