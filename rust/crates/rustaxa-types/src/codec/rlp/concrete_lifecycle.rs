//! Canonical existing StateAPI marker and provenance codecs.
//!
//! These preserve the historical field order and strict identity/width checks
//! formerly owned by the consensus projection codec. Decoding rejects malformed
//! or noncanonical bytes; it does not authorize bootstrap, execution or publication.

use crate::concrete_lifecycle::*;
use anyhow::ensure;
use rlp::{Rlp, RlpStream};
use tiny_keccak::{Hasher, Keccak};

/// Computes Keccak-256 over the supplied projection, provenance, or marker bytes.
/// Empty input is permitted; this does not validate the input's encoding.
pub fn concrete_state_bytes_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    hasher.update(bytes);
    let mut digest = [0; 32];
    hasher.finalize(&mut digest);
    digest
}

/// Encodes slot identities in the existing catalog's RLP list-of-pairs shape.
///
/// Input order is preserved without sorting or deduplication, matching the
/// existing consensus hash helper. Callers preparing a new catalog must sort
/// and deduplicate explicitly. Values are not part of the catalog encoding.
pub fn encode_concrete_storage_catalog(
    slots: impl ExactSizeIterator<Item = ConcreteStorageSlot>,
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(slots.len());
    for slot in slots {
        stream.begin_list(2);
        stream.append(&slot.address.as_slice());
        stream.append(&slot.key.as_slice());
    }
    stream.out().to_vec()
}

/// Hashes the exact ordered slot catalog with the existing Keccak/RLP rule.
/// Values are excluded and caller-supplied order is preserved, even if invalid;
/// strict received-catalog validation belongs to the decoder below.
pub fn concrete_storage_slot_catalog_hash(
    slots: impl ExactSizeIterator<Item = ConcreteStorageSlot>,
) -> [u8; 32] {
    concrete_state_bytes_digest(&encode_concrete_storage_catalog(slots))
}

/// Decodes an exact canonical persisted catalog into sorted unique slot facts.
///
/// Rejects malformed pair/field widths, duplicate or unordered slots and trailing
/// or noncanonical RLP. An empty catalog is the RLP empty list, not empty bytes.
/// Contract eligibility and monotonic extension require application context and
/// are deliberately not inferred from this inventory alone.
pub fn decode_concrete_storage_catalog(bytes: &[u8]) -> anyhow::Result<Vec<ConcreteStorageSlot>> {
    let list = Rlp::new(bytes);
    ensure!(list.is_list(), "concrete storage catalog must be a list");
    let mut slots: Vec<ConcreteStorageSlot> = Vec::new();
    for entry in list.iter() {
        ensure!(
            entry.item_count()? == 2,
            "concrete storage catalog slot must contain two fields"
        );
        let slot = ConcreteStorageSlot {
            address: fixed::<20>(&entry.at(0)?, "catalog slot address")?,
            key: fixed::<32>(&entry.at(1)?, "catalog slot key")?,
        };
        ensure!(
            slots.last().is_none_or(|prior| prior < &slot),
            "concrete storage catalog is unordered or duplicated"
        );
        slots.push(slot);
    }
    ensure!(
        encode_concrete_storage_catalog(slots.iter().copied()) == bytes,
        "concrete storage catalog is not canonical RLP"
    );
    Ok(slots)
}

/// Decodes eight-field StateAPI provenance and rejects malformed widths, missing
/// identities, unknown policy versions or noncanonical RLP. The caller validates
/// generation, digests and the committed descriptor against its approved plan.
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

/// Decodes an exact canonical seven-field StateAPI staged-execution marker.
/// Rejects malformed identities/widths and nonconsecutive or overflowing periods.
/// The caller validates generation and input digests against its approved plan.
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

/// Encodes StateAPI provenance in its existing eight-field canonical RLP shape.
/// Field values are preserved without validating identity or commit authority;
/// a strict decoder and the lifecycle owner validate received bytes.
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

/// Encodes a StateAPI staged marker in its existing seven-field canonical shape.
/// This preserves fields without checking lineage or authorizing execution.
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

/// Decodes a three-field identity, rejecting unsupported policy versions, missing
/// database/chain identities and malformed field widths. The enclosing codec
/// checks canonical RLP for the complete value.
pub fn decode_identity(rlp: &Rlp<'_>) -> anyhow::Result<FinalChainConcreteIdentity> {
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

/// Decodes a two-field period/root descriptor and rejects malformed field widths.
/// This validates encoding shape; the caller checks period/root lineage.
pub fn decode_state(rlp: &Rlp<'_>) -> anyhow::Result<FinalChainConcreteState> {
    ensure!(
        rlp.item_count()? == 2,
        "concrete state must contain two fields"
    );
    Ok(FinalChainConcreteState {
        period: rlp.val_at(0)?,
        root: fixed::<32>(&rlp.at(1)?, "concrete state root")?,
    })
}

/// Appends the three-field identity to an existing RLP stream without validating
/// policy or nonzero identities; strict decoders validate received bytes.
pub fn append_identity(stream: &mut RlpStream, identity: FinalChainConcreteIdentity) {
    stream.begin_list(3);
    stream.append(&identity.policy_version);
    stream.append(&identity.database_id.as_slice());
    stream.append(&identity.chain_id.as_slice());
}

/// Appends a two-field period/root descriptor to an existing RLP stream.
/// This preserves bytes and does not authorize a state transition.
pub fn append_state(stream: &mut RlpStream, state: FinalChainConcreteState) {
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
    fn catalog_keeps_existing_pair_bytes_and_rejects_ambiguous_inventory() {
        let first = ConcreteStorageSlot {
            address: [1; 20],
            key: [2; 32],
        };
        let second = ConcreteStorageSlot {
            address: [1; 20],
            key: [3; 32],
        };
        // One outer item containing a 20-byte address and 32-byte logical key.
        let mut exact = vec![0xf7, 0xf6, 0x94];
        exact.extend([1; 20]);
        exact.push(0xa0);
        exact.extend([2; 32]);
        assert_eq!(encode_concrete_storage_catalog([first].into_iter()), exact);
        assert_eq!(decode_concrete_storage_catalog(&exact).unwrap(), [first]);
        assert!(decode_concrete_storage_catalog(&[0xc0]).unwrap().is_empty());
        for malformed in [
            vec![],
            vec![0x80],
            vec![0xc0, 0xc0],
            vec![0xc2, 0x01, 0x02],
            encode_concrete_storage_catalog([first, first].into_iter()),
            encode_concrete_storage_catalog([second, first].into_iter()),
        ] {
            assert!(
                decode_concrete_storage_catalog(&malformed).is_err(),
                "{malformed:02x?}"
            );
        }
        let mut trailing = exact.clone();
        trailing.push(0);
        assert!(decode_concrete_storage_catalog(&trailing).is_err());
        let sorted = encode_concrete_storage_catalog([first, second].into_iter());
        assert_eq!(
            decode_concrete_storage_catalog(&sorted).unwrap(),
            [first, second]
        );
        // Encoding/hash helpers retain prior behavior: no implicit reordering.
        assert_ne!(
            concrete_storage_slot_catalog_hash([first, second].into_iter()),
            concrete_storage_slot_catalog_hash([second, first].into_iter()),
        );
    }
}
