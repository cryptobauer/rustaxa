//! Staged slashing with frozen delayed reads and authenticated live writes.
//!
//! The FinalChain proof owner verifies signatures and applies jail economics.
//! Proof/list rows are irreversible; first-use account nonce initialization is
//! an ordinary frame effect. Terminal rewards accept only this verified state.

use super::raw::FinalChainNativeRawTrace;
use super::*;
use rlp::RlpStream;

pub(super) struct VerifiedSlashingState {
    jail_blocks: BTreeMap<[u8; 20], u64>,
    jailed: Vec<[u8; 20]>,
    proofs: BTreeSet<[u8; 32]>,
}

impl VerifiedSlashingState {
    fn from_snapshot(snapshot: &DposSnapshot) -> Self {
        Self {
            jail_blocks: snapshot.slashing_jail_blocks.clone(),
            jailed: snapshot.slashing_jailed_validators.clone(),
            proofs: snapshot.slashing_double_voting_proofs.clone(),
        }
    }

    pub(super) fn matches(&self, snapshot: &DposSnapshot) -> bool {
        self.jail_blocks == snapshot.slashing_jail_blocks
            && self.jailed == snapshot.slashing_jailed_validators
            && self.proofs == snapshot.slashing_double_voting_proofs
    }
}

impl FinalChainNativeSession<'_> {
    pub(super) fn prepare_slashing(
        &mut self,
        request: &FinalChainNativeRequest,
    ) -> Result<FinalChainNativeGasQuote, FinalChainNativeSessionError> {
        let required_gas = match request.input.get(..4) {
            Some(selector) if selector == SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR => {
                SLASHING_COMMIT_DOUBLE_VOTING_PROOF_GAS
            }
            Some(selector)
                if selector == SLASHING_GET_JAIL_BLOCK_SELECTOR
                    || selector == SLASHING_GET_JAILED_VALIDATORS_SELECTOR =>
            {
                SLASHING_GET_METHOD_GAS
            }
            _ => 0,
        }
        .into();
        let quote = FinalChainNativeGasQuote {
            invocation: request.id,
            required_gas,
        };
        self.prepared = Some(PreparedCall {
            request: request.clone(),
            quote,
            kind: if request.supplied_gas < required_gas {
                PreparedKind::InsufficientGas
            } else {
                PreparedKind::Slashing(request.input.clone())
            },
        });
        Ok(quote)
    }

    pub(super) fn invoke_slashing(
        &mut self,
        input: &[u8],
        quote: FinalChainNativeGasQuote,
        state: &dyn FinalChainNativeStateRead,
    ) -> Result<FinalChainNativeInvocationResult, FinalChainNativeSessionError> {
        let selector = input.get(..4).unwrap_or_default();
        if selector == SLASHING_GET_JAILED_VALIDATORS_SELECTOR {
            let output = if self.origin == NativeSessionOrigin::CurrentHistoricalTrace {
                encode_abi_address_array(&self.trace_jailed(trace_reads::Reader::Slashing, state)?)
            } else {
                encode_slashing_jailed_validators(&self.eligibility_state)
            };
            return Ok(completed(quote, None, output, vec![], vec![], vec![]));
        }
        if selector == SLASHING_GET_JAIL_BLOCK_SELECTOR {
            let Some(word) = input.get(4..36) else {
                return Ok(failed(
                    quote,
                    format!(
                        "abi: cannot marshal in to go type: length insufficient {} require 32",
                        input.len().saturating_sub(4)
                    ),
                ));
            };
            let validator = word[12..].try_into().expect("ABI address width");
            let output = if self.origin == NativeSessionOrigin::CurrentHistoricalTrace {
                abi_word_from_u64(
                    self.trace_jail(trace_reads::Reader::Slashing, validator, state)?
                        .unwrap_or_default(),
                )
                .to_vec()
            } else {
                encode_slashing_jail_block(&self.eligibility_state, validator)
            };
            return Ok(completed(quote, None, output, vec![], vec![], vec![]));
        }
        if selector != SLASHING_COMMIT_DOUBLE_VOTING_PROOF_SELECTOR {
            return Ok(failed(
                quote,
                format!("no method with id: 0x{}", selector_hex_bytes(selector)),
            ));
        }
        if input.len() == 4 {
            return Ok(failed(
                quote,
                "abi: cannot marshal in to go type: length insufficient 0 require 32".into(),
            ));
        }
        let proof_key = match crate::slashing::decoded_double_voting_proof_storage_key(input) {
            Ok(key) => key.0,
            Err(error) => return Ok(failed(quote, legacy_error(&error))),
        };
        let mut trace = FinalChainNativeRawTrace::new(state);
        authenticate(
            &mut trace,
            ConcreteStorageKey(proof_key),
            if self
                .dpos_state
                .slashing_double_voting_proofs
                .contains(&proof_key)
            {
                concrete_rlp_u64(1)
            } else {
                Vec::new()
            },
            "double-voting proof",
        )?;
        if self
            .dpos_state
            .slashing_double_voting_proofs
            .contains(&proof_key)
        {
            return Ok(failed(quote, "Existing double voting proof".into()));
        }
        let proof = match verify_legacy_double_voting_proof_call_data(input) {
            Ok(proof) => proof,
            Err(error) => return Ok(failed(quote, legacy_error(&error))),
        };
        let validator: [u8; 20] = proof.offender.0;
        let member = if self.origin == NativeSessionOrigin::CurrentHistoricalTrace {
            self.trace_validator_member(validator, state)?
        } else {
            self.eligibility_state
                .total_stakes
                .get(&validator)
                .map(StoredDposTokenAmount::as_u256)
                .unwrap_or_default()
                >= self
                    .final_chain
                    .dpos_eligibility_balance_threshold
                    .as_u256()
        };
        if !member {
            return Ok(failed(quote, "Votes validators is not a validator".into()));
        }
        let jail_key = ConcreteStorageKey(concrete_storage_key(&[&[0], &validator]));
        authenticate(
            &mut trace,
            jail_key,
            self.dpos_state
                .slashing_jail_blocks
                .get(&validator)
                .copied()
                .map(concrete_rlp_u64)
                .unwrap_or_default(),
            "validator jail block",
        )?;
        let delayed_jailed = if self.origin == NativeSessionOrigin::CurrentHistoricalTrace {
            self.trace_jailed(trace_reads::Reader::Slashing, state)?
        } else {
            self.eligibility_state.slashing_jailed_validators.clone()
        };
        let mut next = self.dpos_state.clone();
        let outcome = self
            .final_chain
            .apply_slashing_with_read_facts(
                &mut next,
                self.pending_period,
                proof,
                member,
                &delayed_jailed,
            )
            .map_err(custody::map_kernel_error)?;
        if outcome.status_code != 1 {
            return Ok(failed(quote, String::new()));
        }
        trace.put(
            SLASHING_CONTRACT_ADDRESS,
            jail_key,
            concrete_rlp_u64(next.slashing_jail_blocks[&validator]),
        )?;
        if !delayed_jailed.contains(&validator) {
            let mut list_key = [0; 32];
            list_key[31] = 2;
            let list_key = ConcreteStorageKey(list_key);
            let expected = encode_jailed(&self.dpos_state.slashing_jailed_validators);
            let current = trace.current(SLASHING_CONTRACT_ADDRESS, list_key)?;
            let valid = match current {
                ConcreteRead::Present(bytes) => {
                    bytes == expected
                        || (bytes.is_empty()
                            && self.dpos_state.slashing_jailed_validators.is_empty())
                }
                ConcreteRead::Absent | ConcreteRead::Tombstone => {
                    self.dpos_state.slashing_jailed_validators.is_empty()
                }
            };
            if !valid {
                return Err(FinalChainNativeSessionError::RawIntegrity(
                    "slashing jailed membership".into(),
                ));
            }
            trace.put(
                SLASHING_CONTRACT_ADDRESS,
                list_key,
                encode_jailed(&next.slashing_jailed_validators),
            )?;
        }
        let account = state.account(SLASHING_CONTRACT_ADDRESS)?;
        let ordinary = if account.nonce.is_zero() {
            vec![FinalChainNativeOrdinaryMutation::NonceReplace {
                address: SLASHING_CONTRACT_ADDRESS,
                expected_exists: account.exists,
                expected: account.nonce,
                replacement: FinalChainNonce::from_u64(1),
            }]
        } else {
            Vec::new()
        };
        trace.put(
            SLASHING_CONTRACT_ADDRESS,
            ConcreteStorageKey(proof_key),
            concrete_rlp_u64(1),
        )?;
        self.verified_slashing = Some(VerifiedSlashingState::from_snapshot(&next));
        self.dpos_state = next;
        Ok(completed(
            quote,
            None,
            outcome.code_retval,
            ordinary,
            trace.finish(),
            outcome
                .logs
                .into_iter()
                .map(|log| FinalChainCallLog {
                    address: log.address,
                    topics: log.topics,
                    data: log.data,
                })
                .collect(),
        ))
    }
}

fn authenticate(
    trace: &mut FinalChainNativeRawTrace<'_>,
    key: ConcreteStorageKey,
    expected: Vec<u8>,
    name: &str,
) -> Result<(), FinalChainNativeSessionError> {
    let valid = match trace.current(SLASHING_CONTRACT_ADDRESS, key)? {
        ConcreteRead::Present(bytes) => bytes == expected,
        ConcreteRead::Absent | ConcreteRead::Tombstone => expected.is_empty(),
    };
    if !valid {
        return Err(FinalChainNativeSessionError::RawIntegrity(name.to_owned()));
    }
    Ok(())
}

fn encode_jailed(validators: &[[u8; 20]]) -> Vec<u8> {
    let mut list = RlpStream::new_list(validators.len());
    for validator in validators {
        list.append(&validator.as_slice());
    }
    list.out().to_vec()
}

fn failed(quote: FinalChainNativeGasQuote, error: String) -> FinalChainNativeInvocationResult {
    completed(quote, Some(error), vec![], vec![], vec![], vec![])
}

fn completed(
    quote: FinalChainNativeGasQuote,
    error: Option<String>,
    output: Vec<u8>,
    account_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    raw_mutations: Vec<FinalChainNativeRawMutation>,
    logs: Vec<FinalChainCallLog>,
) -> FinalChainNativeInvocationResult {
    FinalChainNativeInvocationResult::Completed(FinalChainNativeOutcome {
        status: error.map_or(FinalChainNativeStatus::Success, |error| {
            FinalChainNativeStatus::ContractFailure { error }
        }),
        gas_used: quote.required_gas,
        output,
        account_mutations,
        raw_mutations,
        logs,
    })
}

fn selector_hex_bytes(selector: &[u8]) -> String {
    selector.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn legacy_error(error: &LegacyDoubleVotingProofDecodeError) -> String {
    let message = error.to_string();
    match message.as_str() {
        "votes are identical" => "Votes are identical".into(),
        "invalid votes period/round/step" => "Votes period/round/step differ".into(),
        "invalid votes block hash" | "invalid mixed zero/non-zero next-vote block hashes" => {
            "Invalid votes block hash".into()
        }
        "invalid vote signature" => "Invalid vote signature".into(),
        "invalid votes validator" => "Votes validators differ".into(),
        _ => message,
    }
}
