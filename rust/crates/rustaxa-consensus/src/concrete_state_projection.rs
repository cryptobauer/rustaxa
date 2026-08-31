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
use tiny_keccak::{Hasher, Keccak};

use crate::{FinalChainEvmLog, FinalChainEvmLogTopic};

/// Concrete-root policy and projection codec version accepted by Rust.
pub const FINAL_CHAIN_CONCRETE_PROJECTION_VERSION: u64 = 1;
/// Invocation completed without a reverted containing frame.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL: u8 = 0;
/// Invocation's own frame reverted, while legacy precompile writes survived.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED: u8 = 1;
/// An enclosing frame reverted, while legacy precompile writes survived.
pub const FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED: u8 = 2;

/// Stable identity of the concrete state database paired with one chain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteIdentity {
    pub policy_version: u64,
    pub database_id: [u8; 32],
    pub chain_id: [u8; 32],
}

/// Concrete committed or staged state descriptor encoded in StateAPI bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteState {
    pub period: u64,
    pub root: [u8; 32],
}

/// Durable exact staged-execution marker owned by StateAPI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteExecutionMarker {
    pub identity: FinalChainConcreteIdentity,
    pub generation: u64,
    pub plan_hash: [u8; 32],
    pub period: u64,
    pub prior_state: FinalChainConcreteState,
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
}

/// One exact account value at the projection's post-rewards root.
///
/// Rows are strictly address-ordered. An empty raw account RLP is a deletion
/// tombstone; a non-empty payload must be the canonical six-field FinalChain
/// account shape before it may update the native snapshot.
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

/// Concrete StateAPI provenance for one exact staged or committed plan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FinalChainConcreteStateProvenance {
    pub identity: FinalChainConcreteIdentity,
    pub generation: u64,
    pub plan_hash: [u8; 32],
    pub committed_state: FinalChainConcreteState,
    pub transactions_hash: [u8; 32],
    pub rewards_hash: [u8; 32],
    pub projection_hash: [u8; 32],
    pub catalog_hash: [u8; 32],
}

/// Computes Keccak-256 over canonical projection, provenance, or marker bytes.
pub fn concrete_state_bytes_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut digest = [0; 32];
    hasher.finalize(&mut digest);
    digest
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

/// Decodes canonical StateAPI provenance and rejects an unknown policy version
/// or noncanonical RLP.
pub fn decode_concrete_state_provenance(
    bytes: &[u8],
) -> anyhow::Result<FinalChainConcreteStateProvenance> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 8,
        "concrete provenance must contain eight fields"
    );
    let provenance = FinalChainConcreteStateProvenance {
        identity: decode_identity(&rlp.at(0)?)?,
        generation: rlp.val_at(1)?,
        plan_hash: fixed::<32>(&rlp.at(2)?, "provenance plan hash")?,
        committed_state: decode_state(&rlp.at(3)?)?,
        transactions_hash: fixed::<32>(&rlp.at(4)?, "provenance transactions hash")?,
        rewards_hash: fixed::<32>(&rlp.at(5)?, "provenance rewards hash")?,
        projection_hash: fixed::<32>(&rlp.at(6)?, "provenance projection hash")?,
        catalog_hash: fixed::<32>(&rlp.at(7)?, "provenance catalog hash")?,
    };
    ensure!(
        encode_concrete_state_provenance(&provenance) == bytes,
        "concrete provenance is not canonical RLP"
    );
    Ok(provenance)
}

/// Decodes an exact canonical StateAPI staged-execution marker.
pub fn decode_concrete_execution_marker(
    bytes: &[u8],
) -> anyhow::Result<FinalChainConcreteExecutionMarker> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.item_count()? == 7,
        "concrete execution marker must contain seven fields"
    );
    let marker = FinalChainConcreteExecutionMarker {
        identity: decode_identity(&rlp.at(0)?)?,
        generation: rlp.val_at(1)?,
        plan_hash: fixed::<32>(&rlp.at(2)?, "marker plan hash")?,
        period: rlp.val_at(3)?,
        prior_state: decode_state(&rlp.at(4)?)?,
        transactions_hash: fixed::<32>(&rlp.at(5)?, "marker transactions hash")?,
        rewards_hash: fixed::<32>(&rlp.at(6)?, "marker rewards hash")?,
    };
    ensure!(
        marker.prior_state.period.checked_add(1) == Some(marker.period),
        "concrete marker period lineage mismatch"
    );
    ensure!(
        encode_concrete_execution_marker(&marker) == bytes,
        "concrete execution marker is not canonical RLP"
    );
    Ok(marker)
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

/// Canonically encodes StateAPI provenance for fake-port/differential tests.
pub fn encode_concrete_state_provenance(provenance: &FinalChainConcreteStateProvenance) -> Vec<u8> {
    let mut stream = RlpStream::new_list(8);
    append_identity(&mut stream, provenance.identity);
    stream.append(&provenance.generation);
    stream.append(&provenance.plan_hash.as_slice());
    append_state(&mut stream, provenance.committed_state);
    stream.append(&provenance.transactions_hash.as_slice());
    stream.append(&provenance.rewards_hash.as_slice());
    stream.append(&provenance.projection_hash.as_slice());
    stream.append(&provenance.catalog_hash.as_slice());
    stream.out().to_vec()
}

/// Canonically encodes a StateAPI staged marker for lifecycle requests/tests.
pub fn encode_concrete_execution_marker(marker: &FinalChainConcreteExecutionMarker) -> Vec<u8> {
    let mut stream = RlpStream::new_list(7);
    append_identity(&mut stream, marker.identity);
    stream.append(&marker.generation);
    stream.append(&marker.plan_hash.as_slice());
    stream.append(&marker.period);
    append_state(&mut stream, marker.prior_state);
    stream.append(&marker.transactions_hash.as_slice());
    stream.append(&marker.rewards_hash.as_slice());
    stream.out().to_vec()
}

/// Derives the catalog hash from sorted `(address,key)` identities exactly as
/// StateAPI does; values are intentionally excluded from this inventory hash.
pub fn concrete_storage_catalog_hash(storage: &[FinalChainConcreteStorageProjection]) -> [u8; 32] {
    let mut stream = RlpStream::new_list(storage.len());
    for row in storage {
        stream.begin_list(2);
        stream.append(&row.contract.as_slice());
        stream.append(&row.key.as_slice());
    }
    concrete_state_bytes_digest(&stream.out())
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

fn decode_identity(rlp: &Rlp<'_>) -> anyhow::Result<FinalChainConcreteIdentity> {
    ensure!(
        rlp.item_count()? == 3,
        "concrete identity must contain three fields"
    );
    let identity = FinalChainConcreteIdentity {
        policy_version: rlp.val_at(0)?,
        database_id: fixed::<32>(&rlp.at(1)?, "concrete database identity")?,
        chain_id: fixed::<32>(&rlp.at(2)?, "concrete chain identity")?,
    };
    ensure!(
        identity.policy_version == FINAL_CHAIN_CONCRETE_PROJECTION_VERSION,
        "unsupported concrete policy version {}",
        identity.policy_version
    );
    ensure!(
        identity.database_id != [0; 32] && identity.chain_id != [0; 32],
        "concrete identity is missing"
    );
    Ok(identity)
}

fn decode_state(rlp: &Rlp<'_>) -> anyhow::Result<FinalChainConcreteState> {
    ensure!(
        rlp.item_count()? == 2,
        "concrete state must contain two fields"
    );
    Ok(FinalChainConcreteState {
        period: rlp.val_at(0)?,
        root: fixed::<32>(&rlp.at(1)?, "concrete state root")?,
    })
}

fn append_identity(stream: &mut RlpStream, identity: FinalChainConcreteIdentity) {
    stream.begin_list(3);
    stream.append(&identity.policy_version);
    stream.append(&identity.database_id.as_slice());
    stream.append(&identity.chain_id.as_slice());
}

fn append_state(stream: &mut RlpStream, state: FinalChainConcreteState) {
    stream.begin_list(2);
    stream.append(&state.period);
    stream.append(&state.root.as_slice());
}

fn fixed<const N: usize>(rlp: &Rlp<'_>, field: &str) -> anyhow::Result<[u8; N]> {
    let bytes = rlp.data()?;
    ensure!(bytes.len() == N, "{field} must contain {N} bytes");
    let mut result = [0; N];
    result.copy_from_slice(bytes);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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
