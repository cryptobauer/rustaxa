//! Strict codecs for StateAPI concrete projection, provenance, and markers.
//!
//! StateAPI owns a concrete EVM/state-db leaf and returns these values only as
//! canonical RLP bytes. Rust verifies their exact database/chain/generation
//! identity, digests, roots, ordering, and duplicates. Consensus precompile
//! invocations and rewards inputs are then replayed independently; raw account
//! and storage rows are evidence and account synchronization inputs, not a
//! substitute for Rust DPoS/slashing semantics.

use anyhow::ensure;
use rlp::{Rlp, RlpStream};

use rustaxa_types::codec::rlp::concrete_lifecycle::{
    append_identity, append_state, decode_identity, decode_state,
};
pub use rustaxa_types::codec::rlp::concrete_lifecycle::{
    concrete_state_bytes_digest, decode_concrete_execution_marker,
    decode_concrete_state_provenance, encode_concrete_execution_marker,
    encode_concrete_state_provenance,
};
pub use rustaxa_types::concrete_lifecycle::{
    FINAL_CHAIN_CONCRETE_PROJECTION_VERSION, FinalChainConcreteExecutionMarker,
    FinalChainConcreteIdentity, FinalChainConcreteState, FinalChainConcreteStateProvenance,
};

use crate::rewards_stats::RewardsStatsPeriodRlp;
use crate::{
    FinalChainEvmLog, FinalChainEvmLogTopic, FinalChainEvmTransactionInput,
    FinalChainEvmTransactionResult,
};

/// Invocation completed without a reverted containing frame.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL: u8 = 0;
/// Invocation's own frame reverted, while legacy precompile writes survived.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED: u8 = 1;
/// An enclosing frame reverted, while legacy precompile writes survived.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED: u8 = 2;

/// One exact account value at the projection's post-rewards root.
///
/// Rows are strictly address-ordered. An empty raw account RLP is a deletion
/// tombstone; a non-empty payload uses the canonical five-field concrete
/// main-trie account shape before it may update the native snapshot. This is
/// the physical account RLP returned by StateAPI, not the separate FinalChain
/// snapshot account encoding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteAccountProjection {
    pub address: [u8; 20],
    pub raw_account_rlp: Vec<u8>,
}

/// One slot in the complete monotonic DPoS/slashing raw storage catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteStorageProjection {
    pub contract: [u8; 20],
    pub key: [u8; 32],
    /// Empty bytes encode absence; otherwise this is the raw stored value.
    pub value: Vec<u8>,
}

/// One ordered, revert-aware concrete DPoS/slashing invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteInvocation {
    pub transaction_index: u64,
    pub sequence: u64,
    pub depth: u16,
    pub call_type: u8,
    pub caller: [u8; 20],
    pub contract: [u8; 20],
    pub value: Vec<u8>,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
    pub supplied_gas: u64,
    pub required_gas: u64,
    pub gas_used: u64,
    pub error: String,
    pub logs: Vec<FinalChainEvmLog>,
    pub disposition: u8,
}

/// Per-transaction concrete effects at an independently prepared intermediate root.
///
/// Effects are contiguous from index zero. Account and storage rows are sorted,
/// invocation order is strict, and the concatenated invocation lists must equal
/// the projection-wide transcript byte-for-byte.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteTransactionEffect {
    pub index: u64,
    pub transaction_rlp: Vec<u8>,
    pub execution_result_rlp: Vec<u8>,
    pub intermediate_state: FinalChainConcreteState,
    pub accounts: Vec<FinalChainConcreteAccountProjection>,
    pub storage: Vec<FinalChainConcreteStorageProjection>,
    pub invocations: Vec<FinalChainConcreteInvocation>,
}

/// Root-bound concrete result spanning transactions and rewards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteStateProjection {
    pub identity: FinalChainConcreteIdentity,
    pub generation: u64,
    pub plan_hash: [u8; 32],
    pub prior_state: FinalChainConcreteState,
    pub post_transaction_state: FinalChainConcreteState,
    pub post_rewards_state: FinalChainConcreteState,
    pub transaction_effects: Vec<FinalChainConcreteTransactionEffect>,
    pub accounts: Vec<FinalChainConcreteAccountProjection>,
    pub storage: Vec<FinalChainConcreteStorageProjection>,
    pub invocations: Vec<FinalChainConcreteInvocation>,
    pub rewards_input: Vec<u8>,
    pub catalog_hash: [u8; 32],
}

/// Decodes a canonical projection and rejects unknown versions, malformed
/// fields, non-DPoS storage, duplicate/unordered rows or calls, invalid
/// disposition, a mismatched catalog digest, and noncanonical RLP.
pub fn decode_concrete_state_projection(
    bytes: &[u8],
) -> anyhow::Result<FinalChainConcreteStateProjection> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 13,
        "concrete projection must contain thirteen fields"
    );
    let version: u64 = rlp.val_at(0)?;
    ensure!(
        version == FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
        "unsupported concrete projection version {version}"
    );
    let identity = decode_identity(&rlp.at(1)?)?;
    let generation = rlp.val_at(2)?;
    let plan_hash = fixed::<32>(&rlp.at(3)?, "projection plan hash")?;
    let prior_state = decode_state(&rlp.at(4)?)?;
    let post_transaction_state = decode_state(&rlp.at(5)?)?;
    let post_rewards_state = decode_state(&rlp.at(6)?)?;
    ensure!(
        prior_state.period.checked_add(1) == Some(post_transaction_state.period)
            && post_transaction_state.period == post_rewards_state.period,
        "concrete projection period lineage mismatch"
    );
    ensure!(
        prior_state.root != [0; 32]
            && post_transaction_state.root != [0; 32]
            && post_rewards_state.root != [0; 32],
        "concrete projection root missing"
    );

    let mut transaction_effects = Vec::new();
    for effect in rlp.at(7)?.iter() {
        ensure!(
            effect.item_count()? == 7,
            "concrete transaction effect must contain seven fields"
        );
        let index = effect.val_at(0)?;
        ensure!(
            index == transaction_effects.len() as u64,
            "concrete transaction effects are missing, duplicated, or unordered"
        );
        let intermediate_state = decode_state(&effect.at(3)?)?;
        ensure!(
            intermediate_state.period == post_transaction_state.period,
            "concrete transaction effect period mismatch"
        );
        let transaction_effect = FinalChainConcreteTransactionEffect {
            index,
            transaction_rlp: effect.val_at(1)?,
            execution_result_rlp: effect.val_at(2)?,
            intermediate_state,
            accounts: decode_accounts(&effect.at(4)?)?,
            storage: decode_storage(&effect.at(5)?)?,
            invocations: decode_invocations(&effect.at(6)?)?,
        };
        ensure!(
            !transaction_effect.transaction_rlp.is_empty(),
            "concrete transaction effect payload is empty"
        );
        ensure!(
            transaction_effect
                .invocations
                .iter()
                .all(|invocation| invocation.transaction_index == index),
            "concrete invocation transaction index does not match containing effect"
        );
        transaction_effects.push(transaction_effect);
    }

    let accounts = decode_accounts(&rlp.at(8)?)?;
    let storage = decode_storage(&rlp.at(9)?)?;
    let invocations = decode_invocations(&rlp.at(10)?)?;
    let concatenated = transaction_effects
        .iter()
        .flat_map(|effect| effect.invocations.iter().cloned())
        .collect::<Vec<_>>();
    ensure!(
        concatenated == invocations,
        "concrete invocation transcript does not match transaction effects"
    );
    for (sequence, invocation) in invocations.iter().enumerate() {
        ensure!(
            invocation.sequence == sequence as u64,
            "concrete invocation sequence is not contiguous from zero"
        );
    }
    let rewards_input = rlp.val_at(11)?;
    let catalog_hash = fixed::<32>(&rlp.at(12)?, "projection catalog hash")?;
    ensure!(
        catalog_hash == concrete_storage_catalog_hash(&storage),
        "concrete projection catalog hash mismatch"
    );

    let projection = FinalChainConcreteStateProjection {
        identity,
        generation,
        plan_hash,
        prior_state,
        post_transaction_state,
        post_rewards_state,
        transaction_effects,
        accounts,
        storage,
        invocations,
        rewards_input,
        catalog_hash,
    };
    ensure!(
        encode_concrete_state_projection(&projection) == bytes,
        "concrete projection is not canonical RLP"
    );
    Ok(projection)
}

/// Validates a projection/provenance pair against expected execution facts.
///
/// The pair must share one identity/generation/plan/catalog, provenance must
/// digest the exact projection bytes, and all state roots must match the
/// session-owned concrete lineage. Transaction/reward hashes are caller-owned
/// identities and are checked independently rather than inferred from roots.
#[allow(clippy::too_many_arguments)]
pub fn validate_concrete_state_pair(
    projection_rlp: &[u8],
    provenance_rlp: &[u8],
    expected_plan_hash: [u8; 32],
    expected_period: u64,
    expected_prior_root: [u8; 32],
    expected_post_transaction_root: [u8; 32],
    expected_post_rewards_root: [u8; 32],
    expected_transactions_hash: [u8; 32],
    expected_rewards_hash: [u8; 32],
) -> anyhow::Result<(
    FinalChainConcreteStateProjection,
    FinalChainConcreteStateProvenance,
)> {
    let projection = decode_concrete_state_projection(projection_rlp)?;
    let provenance = decode_concrete_state_provenance(provenance_rlp)?;
    ensure!(
        projection.identity == provenance.identity,
        "concrete pair identity mismatch"
    );
    ensure!(
        projection.generation == provenance.generation,
        "concrete pair generation mismatch"
    );
    ensure!(
        projection.plan_hash == expected_plan_hash && provenance.plan_hash == expected_plan_hash,
        "concrete pair plan hash mismatch"
    );
    ensure!(
        projection.prior_state.period.checked_add(1) == Some(expected_period),
        "concrete pair prior period mismatch"
    );
    ensure!(
        projection.prior_state.root == expected_prior_root,
        "concrete pair prior root mismatch"
    );
    ensure!(
        projection.post_transaction_state.period == expected_period
            && projection.post_transaction_state.root == expected_post_transaction_root,
        "concrete pair post-transaction state mismatch"
    );
    ensure!(
        projection.post_rewards_state.period == expected_period
            && projection.post_rewards_state.root == expected_post_rewards_root,
        "concrete pair post-rewards state mismatch"
    );
    ensure!(
        provenance.committed_state.period == expected_period
            && provenance.committed_state.root == expected_post_rewards_root,
        "concrete pair committed state mismatch"
    );
    ensure!(
        provenance.transactions_hash == expected_transactions_hash,
        "concrete pair transactions hash mismatch"
    );
    ensure!(
        provenance.rewards_hash == expected_rewards_hash,
        "concrete pair rewards hash mismatch"
    );
    ensure!(
        provenance.projection_hash == concrete_state_bytes_digest(projection_rlp),
        "concrete pair projection hash mismatch"
    );
    ensure!(
        provenance.catalog_hash == projection.catalog_hash,
        "concrete pair catalog hash mismatch"
    );
    Ok((projection, provenance))
}

/// Canonically encodes a projection. Public callers normally receive bytes
/// from StateAPI; this encoder exists for fake-port and differential fixtures.
pub fn encode_concrete_state_projection(projection: &FinalChainConcreteStateProjection) -> Vec<u8> {
    let mut stream = RlpStream::new_list(13);
    stream.append(&FINAL_CHAIN_CONCRETE_PROJECTION_VERSION);
    append_identity(&mut stream, projection.identity);
    stream.append(&projection.generation);
    stream.append(&projection.plan_hash.as_slice());
    append_state(&mut stream, projection.prior_state);
    append_state(&mut stream, projection.post_transaction_state);
    append_state(&mut stream, projection.post_rewards_state);
    stream.begin_list(projection.transaction_effects.len());
    for effect in &projection.transaction_effects {
        stream.begin_list(7);
        stream.append(&effect.index);
        stream.append(&effect.transaction_rlp);
        stream.append(&effect.execution_result_rlp);
        append_state(&mut stream, effect.intermediate_state);
        append_accounts(&mut stream, &effect.accounts);
        append_storage(&mut stream, &effect.storage);
        append_invocations(&mut stream, &effect.invocations);
    }
    append_accounts(&mut stream, &projection.accounts);
    append_storage(&mut stream, &projection.storage);
    append_invocations(&mut stream, &projection.invocations);
    stream.append(&projection.rewards_input);
    stream.append(&projection.catalog_hash.as_slice());
    stream.out().to_vec()
}

fn append_accounts(stream: &mut RlpStream, accounts: &[FinalChainConcreteAccountProjection]) {
    stream.begin_list(accounts.len());
    for account in accounts {
        stream.begin_list(2);
        stream.append(&account.address.as_slice());
        stream.append(&account.raw_account_rlp);
    }
}

fn append_storage(stream: &mut RlpStream, storage: &[FinalChainConcreteStorageProjection]) {
    stream.begin_list(storage.len());
    for row in storage {
        stream.begin_list(3);
        stream.append(&row.contract.as_slice());
        stream.append(&row.key.as_slice());
        stream.append(&row.value);
    }
}

fn append_invocations(stream: &mut RlpStream, invocations: &[FinalChainConcreteInvocation]) {
    stream.begin_list(invocations.len());
    for call in invocations {
        stream.begin_list(15);
        stream.append(&call.transaction_index);
        stream.append(&call.sequence);
        stream.append(&call.depth);
        stream.append(&call.call_type);
        stream.append(&call.caller.as_slice());
        stream.append(&call.contract.as_slice());
        stream.append(&call.value);
        stream.append(&call.input);
        stream.append(&call.output);
        stream.append(&call.supplied_gas);
        stream.append(&call.required_gas);
        stream.append(&call.gas_used);
        stream.append(&call.error);
        stream.begin_list(call.logs.len());
        for log in &call.logs {
            stream.begin_list(3);
            stream.append(&log.address.as_slice());
            stream.begin_list(log.topics.len());
            for topic in &log.topics {
                stream.append(&topic.topic.as_slice());
            }
            stream.append(&log.data);
        }
        stream.append(&call.disposition);
    }
}

/// Derives the catalog hash from sorted `(address,key)` identities exactly as
/// StateAPI does; values are intentionally excluded from this inventory hash.
pub fn concrete_storage_catalog_hash(storage: &[FinalChainConcreteStorageProjection]) -> [u8; 32] {
    rustaxa_types::codec::rlp::concrete_lifecycle::concrete_storage_slot_catalog_hash(
        storage.iter().map(
            |row| rustaxa_types::concrete_lifecycle::ConcreteStorageSlot {
                address: row.contract,
                key: row.key,
            },
        ),
    )
}

fn decode_accounts(rlp: &Rlp<'_>) -> anyhow::Result<Vec<FinalChainConcreteAccountProjection>> {
    let mut accounts: Vec<FinalChainConcreteAccountProjection> = Vec::new();
    for entry in rlp.iter() {
        ensure!(
            entry.item_count()? == 2,
            "concrete account projection must contain two fields"
        );
        let account = FinalChainConcreteAccountProjection {
            address: fixed::<20>(&entry.at(0)?, "projection account address")?,
            raw_account_rlp: entry.val_at(1)?,
        };
        if let Some(previous) = accounts.last() {
            ensure!(
                previous.address < account.address,
                "concrete account projection is unordered or duplicated"
            );
        }
        accounts.push(account);
    }
    Ok(accounts)
}

fn decode_storage(rlp: &Rlp<'_>) -> anyhow::Result<Vec<FinalChainConcreteStorageProjection>> {
    let mut storage: Vec<FinalChainConcreteStorageProjection> = Vec::new();
    for entry in rlp.iter() {
        ensure!(
            entry.item_count()? == 3,
            "concrete storage projection must contain three fields"
        );
        let row = FinalChainConcreteStorageProjection {
            contract: fixed::<20>(&entry.at(0)?, "projection storage contract")?,
            key: fixed::<32>(&entry.at(1)?, "projection storage key")?,
            value: entry.val_at(2)?,
        };
        ensure!(
            row.contract == crate::final_chain::DPOS_CONTRACT_ADDRESS
                || row.contract == crate::final_chain::SLASHING_CONTRACT_ADDRESS,
            "concrete storage projection contains non-consensus contract"
        );
        if let Some(previous) = storage.last() {
            ensure!(
                (previous.contract, previous.key) < (row.contract, row.key),
                "concrete storage projection is unordered or duplicated"
            );
        }
        storage.push(row);
    }
    Ok(storage)
}

fn decode_invocations(rlp: &Rlp<'_>) -> anyhow::Result<Vec<FinalChainConcreteInvocation>> {
    let mut invocations: Vec<FinalChainConcreteInvocation> = Vec::new();
    for call in rlp.iter() {
        ensure!(
            call.item_count()? == 15,
            "concrete invocation must contain fifteen fields"
        );
        let disposition = call.val_at(14)?;
        ensure!(
            disposition <= FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED,
            "concrete invocation disposition is invalid"
        );
        let mut logs = Vec::new();
        for log in call.at(13)?.iter() {
            ensure!(
                log.item_count()? == 3,
                "concrete invocation log must contain three fields"
            );
            let topics = log
                .at(1)?
                .iter()
                .map(|topic| {
                    Ok(FinalChainEvmLogTopic {
                        topic: fixed::<32>(&topic, "invocation log topic")?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            logs.push(FinalChainEvmLog {
                address: fixed::<20>(&log.at(0)?, "invocation log address")?,
                topics,
                data: log.val_at(2)?,
            });
        }
        let invocation = FinalChainConcreteInvocation {
            transaction_index: call.val_at(0)?,
            sequence: call.val_at(1)?,
            depth: call.val_at(2)?,
            call_type: call.val_at(3)?,
            caller: fixed::<20>(&call.at(4)?, "invocation caller")?,
            contract: fixed::<20>(&call.at(5)?, "invocation contract")?,
            value: call.val_at(6)?,
            input: call.val_at(7)?,
            output: call.val_at(8)?,
            supplied_gas: call.val_at(9)?,
            required_gas: call.val_at(10)?,
            gas_used: call.val_at(11)?,
            error: call.val_at(12)?,
            logs,
            disposition,
        };
        ensure!(
            invocation.contract == crate::final_chain::DPOS_CONTRACT_ADDRESS
                || invocation.contract == crate::final_chain::SLASHING_CONTRACT_ADDRESS,
            "concrete provenance contains non-consensus invocation"
        );
        if let Some(previous) = invocations.last() {
            ensure!(
                (previous.transaction_index, previous.sequence)
                    < (invocation.transaction_index, invocation.sequence),
                "concrete invocations are unordered or duplicated"
            );
        }
        invocations.push(invocation);
    }
    Ok(invocations)
}

fn fixed<const N: usize>(rlp: &Rlp<'_>, field: &str) -> anyhow::Result<[u8; N]> {
    let bytes = rlp.data()?;
    ensure!(bytes.len() == N, "{field} must contain {N} bytes");
    let mut result = [0; N];
    result.copy_from_slice(bytes);
    Ok(result)
}

/// Encodes the StateAPI `vm.Transaction` value independently from the signed
/// canonical transaction envelope. StateAPI records this seven-field execution
/// input in each concrete per-transaction effect. The caller supplies an already
/// selected transaction; this codec performs no admission or execution checks.
/// The full nonce is preserved. An absent receiver is empty RLP data. Hash,
/// signature, position and system classification are not StateAPI fields.
pub fn encode_concrete_evm_transaction(transaction: &FinalChainEvmTransactionInput) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(7);
    stream.append(&transaction.sender.as_slice());
    stream.append(&transaction.gas_price.as_u256());
    if let Some(receiver) = transaction.receiver {
        stream.append(&receiver.as_slice());
    } else {
        stream.append_empty_data();
    }
    stream.append(&transaction.nonce.to_bytes());
    stream.append(&transaction.value.as_u256());
    stream.append(&transaction.gas_limit.as_u64());
    stream.append(&transaction.data);
    stream.out().to_vec()
}

/// Encodes the exact six-field StateAPI execution result, independently of the receipt.
///
/// Preserves output, attempted creation address, ordered logs/topics, gas used and
/// both error payloads. No creation address encodes as twenty zero bytes. The
/// caller remains responsible for validating status, cumulative gas and receipt
/// consistency; none of those derived fields participates in this encoding.
pub fn encode_concrete_execution_result(result: &FinalChainEvmTransactionResult) -> Vec<u8> {
    let mut stream = RlpStream::new_list(6);
    stream.append(&result.output);
    stream.append(&result.new_contract_address.unwrap_or_default().as_slice());
    stream.begin_list(result.logs.len());
    for log in &result.logs {
        stream.begin_list(3);
        stream.append(&log.address.as_slice());
        stream.begin_list(log.topics.len());
        for topic in &log.topics {
            stream.append(&topic.topic.as_slice());
        }
        stream.append(&log.data);
    }
    stream.append(&result.gas_used.as_u64());
    stream.append(&result.code_error);
    stream.append(&result.consensus_error);
    stream.out().to_vec()
}

/// Encodes the ordered StateAPI rewards input as a list of canonical period-stat RLPs.
///
/// Inputs must already be validated `RewardsStatsPeriodRlp` values from the Rust
/// rewards planner. This encoding preserves each inner value verbatim, including
/// order; it does not decode, authorize, sort or recompute the supplied facts.
/// Empty input yields the empty RLP list, with no C API wrapper structure.
pub fn encode_concrete_rewards_input(stats: &[RewardsStatsPeriodRlp]) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(stats.len());
    for stat in stats {
        stream.append_raw(&stat.data, 1);
    }
    stream.out().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordering_projection() -> FinalChainConcreteStateProjection {
        let state = |period, root| FinalChainConcreteState {
            period,
            root: [root; 32],
        };
        let invocation = |transaction_index, sequence| FinalChainConcreteInvocation {
            transaction_index,
            sequence,
            contract: crate::final_chain::DPOS_CONTRACT_ADDRESS,
            ..Default::default()
        };
        let effects = vec![
            FinalChainConcreteTransactionEffect {
                index: 0,
                transaction_rlp: vec![1],
                execution_result_rlp: vec![2],
                intermediate_state: state(2, 4),
                invocations: vec![invocation(0, 0)],
                ..Default::default()
            },
            FinalChainConcreteTransactionEffect {
                index: 1,
                transaction_rlp: vec![3],
                execution_result_rlp: vec![4],
                intermediate_state: state(2, 5),
                invocations: Vec::new(),
                ..Default::default()
            },
            FinalChainConcreteTransactionEffect {
                index: 2,
                transaction_rlp: vec![5],
                execution_result_rlp: vec![6],
                intermediate_state: state(2, 6),
                invocations: vec![invocation(2, 1)],
                ..Default::default()
            },
        ];
        FinalChainConcreteStateProjection {
            identity: FinalChainConcreteIdentity {
                policy_version: FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
                database_id: [1; 32],
                chain_id: [2; 32],
            },
            prior_state: state(1, 3),
            post_transaction_state: state(2, 7),
            post_rewards_state: state(2, 8),
            transaction_effects: effects.clone(),
            invocations: effects
                .into_iter()
                .flat_map(|effect| effect.invocations)
                .collect(),
            catalog_hash: concrete_storage_catalog_hash(&[]),
            ..Default::default()
        }
    }

    #[test]
    fn concrete_invocation_ordering_binds_effect_and_global_sequence() {
        let projection = ordering_projection();
        let encoded = encode_concrete_state_projection(&projection);
        assert_eq!(
            decode_concrete_state_projection(&encoded).unwrap(),
            projection
        );
        let mut wrong_effect = projection.clone();
        wrong_effect.transaction_effects[0].invocations[0].transaction_index = 1;
        wrong_effect.invocations[0].transaction_index = 1;
        let error =
            decode_concrete_state_projection(&encode_concrete_state_projection(&wrong_effect))
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("transaction index does not match containing effect")
        );
        for sequence in [1, 2] {
            let mut invalid = projection.clone();
            invalid.transaction_effects[0].invocations[0].sequence = sequence;
            invalid.invocations[0].sequence = sequence;
            assert!(
                decode_concrete_state_projection(&encode_concrete_state_projection(&invalid))
                    .is_err()
            );
        }
        let mut reset = projection.clone();
        reset.transaction_effects[2].invocations[0].sequence = 0;
        reset.invocations[1].sequence = 0;
        assert!(
            decode_concrete_state_projection(&encode_concrete_state_projection(&reset)).is_err()
        );
    }

    #[test]
    fn concrete_pair_rejects_unrelated_committed_descriptor() {
        let identity = FinalChainConcreteIdentity {
            policy_version: FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
            database_id: [1; 32],
            chain_id: [2; 32],
        };
        let prior = FinalChainConcreteState {
            period: 4,
            root: [3; 32],
        };
        let post_transaction = FinalChainConcreteState {
            period: 5,
            root: [4; 32],
        };
        let post_rewards = FinalChainConcreteState {
            period: 5,
            root: [5; 32],
        };
        let catalog_hash = concrete_storage_catalog_hash(&[]);
        let projection = FinalChainConcreteStateProjection {
            identity,
            generation: 7,
            plan_hash: [6; 32],
            prior_state: prior,
            post_transaction_state: post_transaction,
            post_rewards_state: post_rewards,
            catalog_hash,
            ..Default::default()
        };
        let projection_rlp = encode_concrete_state_projection(&projection);
        let provenance = FinalChainConcreteStateProvenance {
            identity,
            generation: 7,
            plan_hash: [6; 32],
            committed_state: FinalChainConcreteState {
                period: 5,
                root: [0xff; 32],
            },
            transactions_hash: [7; 32],
            rewards_hash: [8; 32],
            projection_hash: concrete_state_bytes_digest(&projection_rlp),
            catalog_hash,
        };
        let error = validate_concrete_state_pair(
            &projection_rlp,
            &encode_concrete_state_provenance(&provenance),
            [6; 32],
            5,
            [3; 32],
            [4; 32],
            [5; 32],
            [7; 32],
            [8; 32],
        )
        .unwrap_err();
        assert!(error.to_string().contains("committed state mismatch"));
    }
}
