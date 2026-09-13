//! Invocation-time ordinary account evidence for opt-in concrete execution.
//!
//! The canonical projection retains its legacy encoding. This Rust-only sidecar
//! supplies the frame facts and signed balances that encoding cannot represent.
//! It is bound to the exact application request and projection bytes, whose
//! lineage must already have passed ordinary concrete pair validation. These
//! are executor facts, not intermediate trie proofs or publication authority.

use crate::concrete_state_projection::{
    FinalChainConcreteStateProjection, concrete_state_bytes_digest,
    encode_concrete_state_projection,
};
use crate::native_session::{
    FinalChainNativeAccount, FinalChainNativeCallKind, FinalChainNativeOrdinaryMutation,
    FinalChainNativeRawMutation, FinalChainNativeRawOperation, FinalChainNativeRequest,
};
use anyhow::{Result, ensure};
use num_bigint::BigUint;
use std::collections::{BTreeMap, BTreeSet};

/// Current ordinary facts and ordered effects for exactly one native call.
///
/// A validator must independently execute the selected native kernel against
/// these account facts and compare both effect streams. It then discards that
/// scratch ordinary state: transaction account projections own which ordinary
/// effects survived enclosing rollback. Semantic/native state persists across
/// invocation replay independently of ordinary rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeInvocationContext {
    /// Complete prepared frame request, including inherited static mode.
    pub request: FinalChainNativeRequest,
    /// Unique invocation-time accounts actually supplied to the native kernel.
    /// Absence must have zero nonce and balance; no missing read means zero.
    pub accounts: Vec<([u8; 20], FinalChainNativeAccount)>,
    /// Unique exact raw values read from the current native lane. Read-only
    /// owner/metadata keys belong here even when no mutation targets them.
    pub raw_reads: Vec<FinalChainNativeRawRead>,
    /// Original ordinary effects before any enclosing rollback.
    pub ordinary_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Original raw effects in native operation order.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
}

/// One exact raw observation from the current execution journal.
/// Reads must be unique per phase; repeated reads observe the same pre-outcome
/// journal state. Raw effects from earlier calls remain visible across rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeRawRead {
    /// Native contract owning the logical row.
    pub address: [u8; 20],
    /// Logical, unhashed storage key.
    pub key: rustaxa_types::concrete_state::ConcreteStorageKey,
    /// Exact classified raw bytes, including present empty tombstones.
    pub value: rustaxa_types::concrete_state::ConcreteRead<Vec<u8>>,
}

/// Current-state evidence and ordered effects for the single terminal rewards phase.
/// The application supplies the opaque reward plan separately; no reward facts
/// or publication authority are invented by this executor context.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FinalChainNativeRewardsContext {
    /// Exact current ordinary accounts read after all transactions.
    pub accounts: Vec<([u8; 20], FinalChainNativeAccount)>,
    /// Exact current native raw reads before rewards/end-block effects apply.
    pub raw_reads: Vec<FinalChainNativeRawRead>,
    /// Ordered custody credits and other ordinary reward effects.
    pub ordinary_mutations: Vec<FinalChainNativeOrdinaryMutation>,
    /// Ordered rewards and end-block native raw effects.
    pub raw_mutations: Vec<FinalChainNativeRawMutation>,
}

/// Exact request/projection binding for the optional Rust execution context.
///
/// An adapter using staged native execution must supply a context even when
/// its invocation list is empty. Legacy leaves may omit this opt-in sidecar.
/// It is neither persisted separately nor used during recovery: the approved
/// publication intent and existing concrete provenance remain authoritative.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeProjectionContext {
    /// Request that constructed the staged native session.
    pub request_id: [u8; 32],
    /// Digest of the exact canonical thirteen-field projection bytes.
    pub projection_hash: [u8; 32],
    /// Exactly one entry per consensus invocation in global sequence order.
    pub invocations: Vec<FinalChainNativeInvocationContext>,
    /// The single rewards/end-block phase of the same native session.
    pub rewards: FinalChainNativeRewardsContext,
}

impl FinalChainNativeProjectionContext {
    /// Validates binding and every frame fact represented by the legacy codec.
    ///
    /// The caller first validates projection lineage, then calls this method
    /// before semantic replay or persisting a commit intent. Only the bounded
    /// DPoS CALL/STATICCALL route is admitted. This does not validate kernel
    /// results or authenticate invocation-time ordinary account facts.
    pub(crate) fn validate_binding(
        &self,
        request_id: [u8; 32],
        projection: &FinalChainConcreteStateProjection,
    ) -> Result<()> {
        ensure!(
            self.request_id == request_id,
            "FINAL_CHAIN_NATIVE_CONTEXT_REQUEST_MISMATCH"
        );
        ensure!(
            self.projection_hash
                == concrete_state_bytes_digest(&encode_concrete_state_projection(projection)),
            "FINAL_CHAIN_NATIVE_CONTEXT_PROJECTION_MISMATCH"
        );
        ensure!(
            self.invocations.len() == projection.invocations.len(),
            "FINAL_CHAIN_NATIVE_CONTEXT_COUNT_MISMATCH"
        );
        for (context, invocation) in self.invocations.iter().zip(&projection.invocations) {
            let request = &context.request;
            let call_type = match request.kind {
                FinalChainNativeCallKind::Call => 0,
                FinalChainNativeCallKind::StaticCall => 3,
                _ => anyhow::bail!("FINAL_CHAIN_NATIVE_CONTEXT_CALL_KIND_UNSUPPORTED"),
            };
            ensure!(
                request.id.transaction.as_u32() as u64 == invocation.transaction_index
                    && request.id.sequence == invocation.sequence
                    && request.period.as_u64() == projection.post_transaction_state.period
                    && request.depth == invocation.depth
                    && call_type == invocation.call_type
                    && request.caller == invocation.caller
                    && request.contract == invocation.contract
                    && request.value.value() == &BigUint::from_bytes_be(&invocation.value)
                    && request.input == invocation.input
                    && request.supplied_gas.as_u64() == invocation.supplied_gas,
                "FINAL_CHAIN_NATIVE_CONTEXT_FRAME_MISMATCH"
            );
            ensure!(
                request.contract == crate::final_chain::DPOS_CONTRACT_ADDRESS
                    && request.state_address == request.contract,
                "FINAL_CHAIN_NATIVE_CONTEXT_ADDRESS_UNSUPPORTED"
            );
            ensure!(
                request.kind != FinalChainNativeCallKind::StaticCall
                    || (request.is_static && request.value.is_zero()),
                "FINAL_CHAIN_NATIVE_CONTEXT_STATIC_MISMATCH"
            );
            ensure!(
                invocation.disposition != crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
                    || invocation.error.is_empty(),
                "FINAL_CHAIN_NATIVE_CONTEXT_DISPOSITION_MISMATCH"
            );
            validate_read_sets(&context.accounts, &context.raw_reads)?;
        }
        validate_read_sets(&self.rewards.accounts, &self.rewards.raw_reads)?;
        validate_raw_lineage(self)?;
        Ok(())
    }
}

/// Checks structural current-read evidence before any staged replay begins.
pub(crate) fn validate_read_sets(
    accounts: &[([u8; 20], FinalChainNativeAccount)],
    raw_reads: &[FinalChainNativeRawRead],
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for (address, account) in accounts {
        ensure!(
            seen.insert(*address),
            "FINAL_CHAIN_NATIVE_CONTEXT_DUPLICATE_ACCOUNT"
        );
        ensure!(
            account.exists
                || (account.nonce == rustaxa_types::FinalChainNonce::zero()
                    && account.balance == num_bigint::BigInt::default()),
            "FINAL_CHAIN_NATIVE_CONTEXT_ABSENT_ACCOUNT_NONZERO"
        );
    }
    let mut seen = BTreeSet::new();
    for read in raw_reads {
        ensure!(
            read.address == crate::final_chain::DPOS_CONTRACT_ADDRESS,
            "FINAL_CHAIN_NATIVE_CONTEXT_RAW_ADDRESS_UNSUPPORTED"
        );
        ensure!(
            seen.insert((read.address, read.key)),
            "FINAL_CHAIN_NATIVE_CONTEXT_DUPLICATE_RAW_READ"
        );
    }
    Ok(())
}

/// Checks raw read-after-setter facts while preserving boundary classifications.
/// Within a transaction the journal returns exact Present(empty) after Delete.
/// Between prepared views an empty value can be a tombstone or proved absence.
fn validate_raw_lineage(context: &FinalChainNativeProjectionContext) -> Result<()> {
    use rustaxa_types::concrete_state::ConcreteRead;
    type RawIdentity = ([u8; 20], rustaxa_types::concrete_state::ConcreteStorageKey);
    type KnownRead = (Option<u32>, ConcreteRead<Vec<u8>>);
    let mut known: BTreeMap<RawIdentity, KnownRead> = BTreeMap::new();
    let phases = context
        .invocations
        .iter()
        .map(|entry| {
            (
                Some(entry.request.id.transaction.as_u32()),
                entry.raw_reads.as_slice(),
                entry.raw_mutations.as_slice(),
            )
        })
        .chain(std::iter::once((
            None,
            context.rewards.raw_reads.as_slice(),
            context.rewards.raw_mutations.as_slice(),
        )));
    for (phase, reads, mutations) in phases {
        for read in reads {
            let key = (read.address, read.key);
            if let Some((prior_phase, prior)) = known.get(&key) {
                let consistent = if prior_phase == &phase {
                    prior == &read.value
                } else {
                    match (prior, &read.value) {
                        (ConcreteRead::Present(left), ConcreteRead::Present(right)) => {
                            left == right
                        }
                        (
                            ConcreteRead::Present(value),
                            ConcreteRead::Absent | ConcreteRead::Tombstone,
                        )
                        | (
                            ConcreteRead::Absent | ConcreteRead::Tombstone,
                            ConcreteRead::Present(value),
                        ) => value.is_empty(),
                        _ => true,
                    }
                };
                ensure!(
                    consistent,
                    "FINAL_CHAIN_NATIVE_CONTEXT_RAW_READ_LINEAGE_MISMATCH"
                );
            }
            known.insert(key, (phase, read.value.clone()));
        }
        for mutation in mutations {
            let key = (mutation.address, mutation.key);
            ensure!(
                known.get(&key).is_some_and(
                    |(read_phase, value)| read_phase == &phase && value == &mutation.expected
                ),
                "FINAL_CHAIN_NATIVE_CONTEXT_RAW_EXPECTED_MISMATCH"
            );
            let value = match &mutation.operation {
                FinalChainNativeRawOperation::Put(value) => value.as_bytes().to_vec(),
                FinalChainNativeRawOperation::Delete => Vec::new(),
            };
            known.insert(key, (phase, ConcreteRead::Present(value)));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::concrete_state_projection::FinalChainConcreteInvocation;
    use crate::native_session::{FinalChainNativeInvocationId, FinalChainNativeValue};

    fn fixture() -> (
        FinalChainNativeProjectionContext,
        FinalChainConcreteStateProjection,
    ) {
        let request = FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: 0_u32.into(),
                sequence: 0,
            },
            period: 1_u64.into(),
            depth: 1,
            kind: FinalChainNativeCallKind::Call,
            is_static: true,
            caller: [0x31; 20],
            contract: crate::final_chain::DPOS_CONTRACT_ADDRESS,
            state_address: crate::final_chain::DPOS_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::new((BigUint::from(1_u8) << 264) + BigUint::from(7_u8)),
            input: vec![1, 2, 3, 4],
            supplied_gas: 20_000_u64.into(),
        };
        let mut projection = FinalChainConcreteStateProjection::default();
        projection.post_transaction_state.period = 1;
        projection.invocations.push(FinalChainConcreteInvocation {
            depth: request.depth,
            caller: request.caller,
            contract: request.contract,
            value: request.value.value().to_bytes_be(),
            input: request.input.clone(),
            supplied_gas: request.supplied_gas.as_u64(),
            ..Default::default()
        });
        let context = FinalChainNativeProjectionContext {
            request_id: [5; 32],
            projection_hash: concrete_state_bytes_digest(&encode_concrete_state_projection(
                &projection,
            )),
            rewards: FinalChainNativeRewardsContext::default(),
            invocations: vec![FinalChainNativeInvocationContext {
                request,
                accounts: vec![(
                    [0x31; 20],
                    FinalChainNativeAccount {
                        exists: true,
                        nonce: rustaxa_types::FinalChainNonce::from_u64(3),
                        balance: -(num_bigint::BigInt::from(1_u8) << 280_usize),
                    },
                )],
                raw_reads: Vec::new(),
                ordinary_mutations: Vec::new(),
                raw_mutations: Vec::new(),
            }],
        };
        (context, projection)
    }

    #[test]
    fn context_retains_full_values_and_inherited_static_call_facts() {
        let (context, mut projection) = fixture();
        context.validate_binding([5; 32], &projection).unwrap();
        // Binding is numeric, without narrowing or rejecting leading zero bytes.
        projection.invocations[0].value.insert(0, 0);
        let mut normalized = context;
        normalized.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        normalized.validate_binding([5; 32], &projection).unwrap();
        normalized.invocations[0].request.value = FinalChainNativeValue::new(7_u8.into());
        assert!(
            normalized
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("FRAME_MISMATCH")
        );
    }

    #[test]
    fn context_rejects_binding_context_and_account_integrity_mismatches() {
        let (context, projection) = fixture();
        assert!(
            context
                .validate_binding([6; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("REQUEST_MISMATCH")
        );
        let mut changed = projection.clone();
        changed.generation += 1;
        assert!(
            context
                .validate_binding([5; 32], &changed)
                .unwrap_err()
                .to_string()
                .contains("PROJECTION_MISMATCH")
        );
        for mutation in 0..8 {
            let mut invalid = context.clone();
            let entry = &mut invalid.invocations[0];
            let expected = match mutation {
                0 => {
                    entry.request.id.sequence = 1;
                    "FRAME_MISMATCH"
                }
                1 => {
                    entry.request.period = 2_u64.into();
                    "FRAME_MISMATCH"
                }
                2 => {
                    entry.request.state_address = [0; 20];
                    "ADDRESS_UNSUPPORTED"
                }
                3 => {
                    entry.request.kind = FinalChainNativeCallKind::DelegateCall;
                    "CALL_KIND_UNSUPPORTED"
                }
                4 => {
                    entry.accounts.push(entry.accounts[0].clone());
                    "DUPLICATE_ACCOUNT"
                }
                5 => {
                    entry.accounts[0].1.exists = false;
                    "ABSENT_ACCOUNT_NONZERO"
                }
                6 => {
                    invalid.invocations.clear();
                    "COUNT_MISMATCH"
                }
                _ => {
                    invalid.invocations.push(invalid.invocations[0].clone());
                    "COUNT_MISMATCH"
                }
            };
            assert!(
                invalid
                    .validate_binding([5; 32], &projection)
                    .unwrap_err()
                    .to_string()
                    .contains(expected)
            );
        }
    }

    #[test]
    fn staticcall_requires_effective_static_mode_and_zero_value() {
        let (mut context, mut projection) = fixture();
        context.invocations[0].request.kind = FinalChainNativeCallKind::StaticCall;
        projection.invocations[0].call_type = 3;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("STATIC_MISMATCH")
        );
        context.invocations[0].request.value = FinalChainNativeValue::default();
        projection.invocations[0].value.clear();
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        context.validate_binding([5; 32], &projection).unwrap();
        context.invocations[0].request.is_static = false;
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("STATIC_MISMATCH")
        );
    }
    #[test]
    fn raw_lineage_preserves_journal_and_prepared_view_classifications() {
        use rustaxa_types::concrete_state::{ConcreteRead, ConcreteStorageKey};
        let (mut context, _) = fixture();
        let address = crate::final_chain::DPOS_CONTRACT_ADDRESS;
        let key = ConcreteStorageKey([8; 32]);
        context.invocations[0].raw_reads = vec![FinalChainNativeRawRead {
            address,
            key,
            value: ConcreteRead::Present(vec![1]),
        }];
        context.invocations[0].raw_mutations = vec![FinalChainNativeRawMutation {
            address,
            key,
            expected: ConcreteRead::Present(vec![1]),
            operation: FinalChainNativeRawOperation::Delete,
        }];
        let mut second = context.invocations[0].clone();
        second.request.id.sequence = 1;
        second.raw_reads[0].value = ConcreteRead::Present(Vec::new());
        second.raw_mutations.clear();
        context.invocations.push(second);
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Tombstone;
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("READ_LINEAGE_MISMATCH")
        );
        context.invocations[1].request.id.transaction = 1_u32.into();
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Absent;
        validate_raw_lineage(&context).unwrap();
        context.invocations[1].raw_reads[0].value = ConcreteRead::Present(vec![2]);
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("READ_LINEAGE_MISMATCH")
        );
        context.invocations.truncate(1);
        context.invocations[0].raw_mutations[0].expected = ConcreteRead::Tombstone;
        assert!(
            validate_raw_lineage(&context)
                .unwrap_err()
                .to_string()
                .contains("RAW_EXPECTED_MISMATCH")
        );
    }
    #[test]
    fn native_failure_cannot_be_reclassified_as_parent_rollback() {
        let (mut context, mut projection) = fixture();
        projection.invocations[0].error = "Method is not payable".to_owned();
        projection.invocations[0].disposition =
            crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        assert!(
            context
                .validate_binding([5; 32], &projection)
                .unwrap_err()
                .to_string()
                .contains("DISPOSITION_MISMATCH")
        );
        projection.invocations[0].disposition =
            crate::concrete_state_projection::FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED;
        context.projection_hash =
            concrete_state_bytes_digest(&encode_concrete_state_projection(&projection));
        context.validate_binding([5; 32], &projection).unwrap();
    }
}
