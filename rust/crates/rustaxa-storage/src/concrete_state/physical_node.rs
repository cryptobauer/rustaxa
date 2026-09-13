//! Verification of one key path through Taraxa's persisted trie-node encoding.
//!
//! Taraxa stores 16-child branches and may replace a leaf's physical value with
//! a hash hint. Verification reconstructs the canonical 17-child/hash-value
//! encoding along one requested path and authenticates it against the pinned
//! root. Hashed sibling subtrees remain unopened; embedded siblings are decoded
//! because their canonical bytes contribute directly to an ancestor hash.

use rlp::{Rlp, RlpStream};
use rustaxa_types::FinalChainBlockNumber;
use rustaxa_types::concrete_state::{ConcreteReadError, ConcreteStateIdentity};

use super::codec::{account_commitment_rlp, corrupt, keccak256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InventoryResource {
    Nodes,
    Leaves,
    ValueBytes,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum InventoryError {
    Read(ConcreteReadError),
    LimitExceeded {
        resource: InventoryResource,
        limit: u64,
        required: u64,
    },
}

impl From<ConcreteReadError> for InventoryError {
    fn from(error: ConcreteReadError) -> Self {
        Self::Read(error)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct InventoryLimits {
    pub(crate) max_nodes: u64,
    pub(crate) max_leaves: u64,
    pub(crate) max_value_bytes: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Inventory {
    pub(crate) nodes_visited: u64,
    pub(crate) value_bytes: u64,
    pub(crate) entries: Vec<([u8; 32], Vec<u8>)>,
}

#[derive(Clone, Copy)]
pub(crate) enum TrieSchema {
    Account,
    Storage,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PathProof {
    Member(Vec<u8>),
    NonMember,
}

pub(crate) struct SelectedVersion {
    pub(crate) value: Vec<u8>,
}

pub(crate) trait PhysicalTrieStore {
    fn identity(&self) -> ConcreteStateIdentity;
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError>;
    fn value(
        &self,
        column: &str,
        prefix: [u8; 32],
        period: FinalChainBlockNumber,
    ) -> Result<Option<SelectedVersion>, ConcreteReadError>;
}

/// Authenticates membership or non-membership for one already-hashed trie key.
pub(crate) fn verify_path<S: PhysicalTrieStore>(
    store: &S,
    root: [u8; 32],
    path: [u8; 32],
    node_column: &str,
    value_column: &str,
    value_prefix: impl Fn([u8; 32]) -> [u8; 32] + Copy,
    schema: TrieSchema,
) -> Result<PathProof, ConcreteReadError> {
    if root == super::codec::empty_trie_root() {
        return Ok(PathProof::NonMember);
    }
    let raw = store
        .node(node_column, root)?
        .ok_or_else(|| ConcreteReadError::HistoryUnavailable(store.identity()))?;
    let context = Context {
        store,
        target: bytes_to_nibbles(path),
        node_column,
        value_column,
        value_prefix,
        schema,
    };
    let verified = context.node(&raw, &[], true, 0)?;
    if keccak256(&verified.canonical) != root {
        return Err(corrupt("physical trie root hash mismatch"));
    }
    verified
        .proof
        .ok_or_else(|| corrupt("physical trie path produced no proof outcome"))
}

/// Traverses and authenticates every live leaf under one storage root.
///
/// A successful result is complete for that root. Resource exhaustion and any missing or
/// malformed dependency return an error and never expose the partially accumulated entries.
pub(crate) fn inventory_storage_trie<S: PhysicalTrieStore>(
    store: &S,
    root: [u8; 32],
    value_prefix: impl Fn([u8; 32]) -> [u8; 32] + Copy,
    limits: InventoryLimits,
) -> Result<Inventory, InventoryError> {
    if root == super::codec::empty_trie_root() {
        return Ok(Inventory {
            nodes_visited: 0,
            value_bytes: 0,
            entries: Vec::new(),
        });
    }
    let raw = store
        .node("4", root)?
        .ok_or_else(|| ConcreteReadError::HistoryUnavailable(store.identity()))?;
    let mut context = InventoryContext {
        store,
        value_prefix,
        limits,
        nodes_visited: 0,
        value_bytes: 0,
        entries: std::collections::BTreeMap::new(),
    };
    let canonical = context.node(&raw, &[], 0)?;
    if keccak256(&canonical) != root {
        return Err(
            ConcreteReadError::Corrupt("physical storage trie root hash mismatch".into()).into(),
        );
    }
    Ok(Inventory {
        nodes_visited: context.nodes_visited,
        value_bytes: context.value_bytes,
        entries: context.entries.into_iter().collect(),
    })
}

struct InventoryContext<'a, S, F> {
    store: &'a S,
    value_prefix: F,
    limits: InventoryLimits,
    nodes_visited: u64,
    value_bytes: u64,
    entries: std::collections::BTreeMap<[u8; 32], Vec<u8>>,
}

impl<S: PhysicalTrieStore, F: Fn([u8; 32]) -> [u8; 32] + Copy> InventoryContext<'_, S, F> {
    fn child(
        &mut self,
        raw: &[u8],
        prefix: &[u8],
        depth: usize,
    ) -> Result<Vec<u8>, InventoryError> {
        let physical_is_list = exact_rlp(raw, "physical trie child")?.is_list();
        let canonical = self.node(raw, prefix, depth)?;
        if physical_is_list && canonical.len() >= 32 {
            Ok(rlp::encode(&keccak256(&canonical).as_slice()).to_vec())
        } else {
            Ok(canonical)
        }
    }

    fn node(&mut self, raw: &[u8], prefix: &[u8], depth: usize) -> Result<Vec<u8>, InventoryError> {
        if depth > 128 || prefix.len() > 64 {
            return Err(corrupt("physical trie inventory exceeds its depth bound").into());
        }
        let rlp = exact_rlp(raw, "physical trie node")?;
        if !rlp.is_list() {
            return self.reference(&rlp, raw, prefix, depth);
        }
        self.nodes_visited = self.nodes_visited.checked_add(1).ok_or_else(|| {
            ConcreteReadError::Corrupt("physical trie inventory node count overflow".into())
        })?;
        self.enforce_limit(
            InventoryResource::Nodes,
            self.limits.max_nodes,
            self.nodes_visited,
        )?;
        match rlp.item_count().map_err(corrupt)? {
            16 => self.branch(&rlp, prefix, depth),
            1 | 2 => self.short(&rlp, prefix, depth),
            _ => Err(corrupt("physical trie node has invalid list arity").into()),
        }
    }

    fn reference(
        &mut self,
        rlp: &Rlp<'_>,
        raw: &[u8],
        prefix: &[u8],
        depth: usize,
    ) -> Result<Vec<u8>, InventoryError> {
        let reference = rlp.data().map_err(corrupt)?;
        if reference.is_empty() {
            return Ok(vec![0x80]);
        }
        let hash: [u8; 32] = reference
            .try_into()
            .map_err(|_| corrupt("physical trie reference must be empty or 32 bytes"))?;
        let child = self
            .store
            .node("4", hash)?
            .ok_or_else(|| ConcreteReadError::HistoryUnavailable(self.store.identity()))?;
        let canonical = self.node(&child, prefix, depth + 1)?;
        let computed = keccak256(&canonical);
        if computed != hash {
            return Err(corrupt(format!(
                "physical trie child hash mismatch at nibble depth {}: expected {}, computed {}",
                prefix.len(),
                hex_hash(hash),
                hex_hash(computed)
            ))
            .into());
        }
        Ok(raw.to_vec())
    }

    fn branch(
        &mut self,
        rlp: &Rlp<'_>,
        prefix: &[u8],
        depth: usize,
    ) -> Result<Vec<u8>, InventoryError> {
        if prefix.len() >= 64 {
            return Err(corrupt("physical branch extends past a complete key").into());
        }
        let mut stream = RlpStream::new_list(17);
        for index in 0..16 {
            let mut child_prefix = prefix.to_vec();
            child_prefix.push(index as u8);
            let child = self.child(
                rlp.at(index).map_err(corrupt)?.as_raw(),
                &child_prefix,
                depth + 1,
            )?;
            stream.append_raw(&child, 1);
        }
        stream.append_empty_data();
        Ok(stream.out().to_vec())
    }

    fn short(
        &mut self,
        rlp: &Rlp<'_>,
        prefix: &[u8],
        depth: usize,
    ) -> Result<Vec<u8>, InventoryError> {
        let count = rlp.item_count().map_err(corrupt)?;
        let compact = rlp.at(0).map_err(corrupt)?.data().map_err(corrupt)?;
        let (terminal, part) = decode_compact(compact)?;
        let mut leaf_path = prefix.to_vec();
        leaf_path.extend_from_slice(&part);
        if leaf_path.len() > 64 {
            return Err(corrupt("physical short-node key exceeds 32 bytes").into());
        }
        if terminal {
            if leaf_path.len() != 64 {
                return Err(corrupt("physical leaf key is not 32 bytes").into());
            }
            let leaf_key = nibbles_to_bytes(&leaf_path)?;
            let selected = self
                .store
                .value(
                    "5",
                    (self.value_prefix)(leaf_key),
                    self.store.identity().period,
                )?
                .ok_or_else(|| ConcreteReadError::HistoryUnavailable(self.store.identity()))?;
            if selected.value.is_empty() {
                return Err(corrupt("physical trie references a tombstoned value").into());
            }
            let content = if count == 2 {
                rlp.at(1).map_err(corrupt)?.data().map_err(corrupt)?
            } else {
                &[]
            };
            if !content.is_empty() && content.len() != 32 && content.len() > 8 {
                return Err(corrupt("physical leaf has invalid inline value width").into());
            }
            if !content.is_empty() && content.len() <= 8 && content != selected.value {
                return Err(corrupt("physical inline leaf and versioned value differ").into());
            }
            let commitment = rlp::encode(&selected.value).to_vec();
            let mut stream = RlpStream::new_list(2);
            stream.append(&compact);
            stream.append(&commitment);
            let canonical = stream.out().to_vec();
            if content.len() == 32 && keccak256(&canonical).as_slice() != content {
                return Err(corrupt("physical leaf hash hint does not match its value").into());
            }

            let required_leaves = (self.entries.len() as u64).checked_add(1).ok_or_else(|| {
                ConcreteReadError::Corrupt("physical trie inventory leaf count overflow".into())
            })?;
            self.enforce_limit(
                InventoryResource::Leaves,
                self.limits.max_leaves,
                required_leaves,
            )?;
            let value_len = u64::try_from(selected.value.len()).map_err(|_| {
                ConcreteReadError::Corrupt("physical trie inventory value width overflow".into())
            })?;
            let required_bytes = self.value_bytes.checked_add(value_len).ok_or_else(|| {
                ConcreteReadError::Corrupt("physical trie inventory value-byte overflow".into())
            })?;
            self.enforce_limit(
                InventoryResource::ValueBytes,
                self.limits.max_value_bytes,
                required_bytes,
            )?;
            if self.entries.insert(leaf_key, selected.value).is_some() {
                return Err(
                    corrupt("physical trie inventory contains a duplicate leaf path").into(),
                );
            }
            self.value_bytes = required_bytes;
            return Ok(canonical);
        }
        if count != 2 {
            return Err(corrupt("physical extension node has no child").into());
        }
        let child = self.child(rlp.at(1).map_err(corrupt)?.as_raw(), &leaf_path, depth + 1)?;
        let mut stream = RlpStream::new_list(2);
        stream.append(&compact);
        stream.append_raw(&child, 1);
        Ok(stream.out().to_vec())
    }

    fn enforce_limit(
        &self,
        resource: InventoryResource,
        limit: u64,
        required: u64,
    ) -> Result<(), InventoryError> {
        if required > limit {
            return Err(InventoryError::LimitExceeded {
                resource,
                limit,
                required,
            });
        }
        Ok(())
    }
}

struct Context<'a, S, F> {
    store: &'a S,
    target: [u8; 64],
    node_column: &'a str,
    value_column: &'a str,
    value_prefix: F,
    schema: TrieSchema,
}

struct VerifiedNode {
    canonical: Vec<u8>,
    proof: Option<PathProof>,
}

impl<S: PhysicalTrieStore, F: Fn([u8; 32]) -> [u8; 32] + Copy> Context<'_, S, F> {
    fn child(
        &self,
        raw: &[u8],
        prefix: &[u8],
        follows_target: bool,
        depth: usize,
    ) -> Result<VerifiedNode, ConcreteReadError> {
        let physical_is_list = exact_rlp(raw, "physical trie child")?.is_list();
        let mut verified = self.node(raw, prefix, follows_target, depth)?;
        if physical_is_list && verified.canonical.len() >= 32 {
            verified.canonical = rlp::encode(&keccak256(&verified.canonical).as_slice()).to_vec();
        }
        Ok(verified)
    }

    fn node(
        &self,
        raw: &[u8],
        prefix: &[u8],
        follows_target: bool,
        depth: usize,
    ) -> Result<VerifiedNode, ConcreteReadError> {
        if depth > 128 || prefix.len() > 64 {
            return Err(corrupt("physical trie path exceeds its depth bound"));
        }
        let rlp = exact_rlp(raw, "physical trie node")?;
        if !rlp.is_list() {
            return self.reference(&rlp, raw, prefix, follows_target, depth);
        }
        match rlp.item_count().map_err(corrupt)? {
            16 => self.branch(&rlp, prefix, follows_target, depth),
            1 | 2 => self.short(&rlp, prefix, follows_target, depth),
            _ => Err(corrupt("physical trie node has invalid list arity")),
        }
    }

    fn reference(
        &self,
        rlp: &Rlp<'_>,
        raw: &[u8],
        prefix: &[u8],
        follows_target: bool,
        depth: usize,
    ) -> Result<VerifiedNode, ConcreteReadError> {
        let reference = rlp.data().map_err(corrupt)?;
        if reference.is_empty() {
            return Ok(VerifiedNode {
                canonical: vec![0x80],
                proof: follows_target.then_some(PathProof::NonMember),
            });
        }
        let hash: [u8; 32] = reference
            .try_into()
            .map_err(|_| corrupt("physical trie reference must be empty or 32 bytes"))?;
        if !follows_target {
            return Ok(VerifiedNode {
                canonical: raw.to_vec(),
                proof: None,
            });
        }
        let child = self
            .store
            .node(self.node_column, hash)?
            .ok_or_else(|| ConcreteReadError::HistoryUnavailable(self.store.identity()))?;
        let verified = self.node(&child, prefix, true, depth + 1)?;
        let computed = keccak256(&verified.canonical);
        if computed != hash {
            return Err(corrupt(format!(
                "physical trie child hash mismatch at nibble depth {}: expected {}, computed {}",
                prefix.len(),
                hex_hash(hash),
                hex_hash(computed)
            )));
        }
        Ok(VerifiedNode {
            canonical: raw.to_vec(),
            proof: verified.proof,
        })
    }

    fn branch(
        &self,
        rlp: &Rlp<'_>,
        prefix: &[u8],
        follows_target: bool,
        depth: usize,
    ) -> Result<VerifiedNode, ConcreteReadError> {
        if follows_target && prefix.len() >= self.target.len() {
            return Err(corrupt("physical branch extends past a complete key"));
        }
        let selected = follows_target.then(|| self.target[prefix.len()] as usize);
        let mut stream = RlpStream::new_list(17);
        let mut proof = None;
        for index in 0..16 {
            let child_follows = selected == Some(index);
            let mut child_prefix = prefix.to_vec();
            child_prefix.push(index as u8);
            let child = self.child(
                rlp.at(index).map_err(corrupt)?.as_raw(),
                &child_prefix,
                child_follows,
                depth + 1,
            )?;
            if child_follows {
                proof = child.proof;
            }
            stream.append_raw(&child.canonical, 1);
        }
        stream.append_empty_data();
        Ok(VerifiedNode {
            canonical: stream.out().to_vec(),
            proof,
        })
    }

    fn short(
        &self,
        rlp: &Rlp<'_>,
        prefix: &[u8],
        follows_target: bool,
        depth: usize,
    ) -> Result<VerifiedNode, ConcreteReadError> {
        let count = rlp.item_count().map_err(corrupt)?;
        let compact = rlp.at(0).map_err(corrupt)?.data().map_err(corrupt)?;
        let (terminal, part) = decode_compact(compact)?;
        let mut leaf_path = prefix.to_vec();
        leaf_path.extend_from_slice(&part);
        if leaf_path.len() > 64 {
            return Err(corrupt("physical short-node key exceeds 32 bytes"));
        }
        let matches = self.target[prefix.len()..].starts_with(&part);
        if terminal {
            if leaf_path.len() != 64 {
                return Err(corrupt("physical leaf key is not 32 bytes"));
            }
            let leaf_key = nibbles_to_bytes(&leaf_path)?;
            let selected = self
                .store
                .value(
                    self.value_column,
                    (self.value_prefix)(leaf_key),
                    self.store.identity().period,
                )?
                .ok_or_else(|| ConcreteReadError::HistoryUnavailable(self.store.identity()))?;
            if selected.value.is_empty() {
                return Err(corrupt("physical trie references a tombstoned value"));
            }
            let content = if count == 2 {
                rlp.at(1).map_err(corrupt)?.data().map_err(corrupt)?
            } else {
                &[]
            };
            if !content.is_empty() && content.len() != 32 && content.len() > 8 {
                return Err(corrupt("physical leaf has invalid inline value width"));
            }
            if !content.is_empty() && content.len() <= 8 && content != selected.value {
                return Err(corrupt("physical inline leaf and versioned value differ"));
            }
            let commitment = match self.schema {
                TrieSchema::Account => account_commitment_rlp(
                    &super::codec::decode_physical_account(&selected.value)?,
                )?,
                TrieSchema::Storage => rlp::encode(&selected.value).to_vec(),
            };
            let mut stream = RlpStream::new_list(2);
            stream.append(&compact);
            stream.append(&commitment);
            let canonical = stream.out().to_vec();
            if content.len() == 32 && keccak256(&canonical).as_slice() != content {
                return Err(corrupt("physical leaf hash hint does not match its value"));
            }
            let exact_target = follows_target && matches && leaf_path.as_slice() == self.target;
            return Ok(VerifiedNode {
                canonical,
                proof: follows_target.then_some({
                    if exact_target {
                        PathProof::Member(selected.value)
                    } else {
                        PathProof::NonMember
                    }
                }),
            });
        }
        if count != 2 {
            return Err(corrupt("physical extension node has no child"));
        }
        let child_follows = follows_target && matches;
        let child = self.child(
            rlp.at(1).map_err(corrupt)?.as_raw(),
            &leaf_path,
            child_follows,
            depth + 1,
        )?;
        let mut stream = RlpStream::new_list(2);
        stream.append(&compact);
        stream.append_raw(&child.canonical, 1);
        Ok(VerifiedNode {
            canonical: stream.out().to_vec(),
            proof: if follows_target && !matches {
                Some(PathProof::NonMember)
            } else {
                child.proof
            },
        })
    }
}

fn decode_compact(bytes: &[u8]) -> Result<(bool, Vec<u8>), ConcreteReadError> {
    let Some(first) = bytes.first().copied() else {
        return Err(corrupt("physical short-node compact key is empty"));
    };
    let flag = first >> 4;
    if flag > 3 || flag & 1 == 0 && first & 0x0f != 0 {
        return Err(corrupt("physical short-node compact key has invalid flags"));
    }
    let mut output = Vec::with_capacity(bytes.len() * 2);
    if flag & 1 != 0 {
        output.push(first & 0x0f);
    }
    for byte in &bytes[1..] {
        output.extend([byte >> 4, byte & 0x0f]);
    }
    Ok((flag & 2 != 0, output))
}

fn bytes_to_nibbles(bytes: [u8; 32]) -> [u8; 64] {
    let mut output = [0_u8; 64];
    for (index, byte) in bytes.into_iter().enumerate() {
        output[index * 2] = byte >> 4;
        output[index * 2 + 1] = byte & 0x0f;
    }
    output
}

fn nibbles_to_bytes(nibbles: &[u8]) -> Result<[u8; 32], ConcreteReadError> {
    if nibbles.len() != 64 || nibbles.iter().any(|nibble| *nibble > 0x0f) {
        return Err(corrupt("physical trie key has invalid nibbles"));
    }
    let mut output = [0_u8; 32];
    for index in 0..32 {
        output[index] = nibbles[index * 2] << 4 | nibbles[index * 2 + 1];
    }
    Ok(output)
}

fn exact_rlp<'a>(bytes: &'a [u8], label: &str) -> Result<Rlp<'a>, ConcreteReadError> {
    let rlp = Rlp::new(bytes);
    if rlp.payload_info().map_err(corrupt)?.total() != bytes.len() {
        return Err(corrupt(format!("{label} has trailing bytes")));
    }
    Ok(rlp)
}

fn hex_hash(hash: [u8; 32]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(64);
    for byte in hash {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use rustaxa_types::concrete_state::ConcreteStateIdentity;

    use super::*;

    #[derive(Clone)]
    struct MemoryStore {
        identity: ConcreteStateIdentity,
        nodes: BTreeMap<[u8; 32], Vec<u8>>,
        values: BTreeMap<[u8; 32], Vec<u8>>,
    }

    impl PhysicalTrieStore for MemoryStore {
        fn identity(&self) -> ConcreteStateIdentity {
            self.identity
        }

        fn node(
            &self,
            _column: &str,
            hash: [u8; 32],
        ) -> Result<Option<Vec<u8>>, ConcreteReadError> {
            Ok(self.nodes.get(&hash).cloned())
        }

        fn value(
            &self,
            _column: &str,
            prefix: [u8; 32],
            _period: FinalChainBlockNumber,
        ) -> Result<Option<SelectedVersion>, ConcreteReadError> {
            Ok(self
                .values
                .get(&prefix)
                .cloned()
                .map(|value| SelectedVersion { value }))
        }
    }

    #[test]
    fn verifies_pinned_go_branch_extension_and_nonmembership_fixture() {
        // public.json node_history[2], emitted by both pinned Go references.
        let root =
            decode_hex_32("acde2a8675590a1f104ae5db7c4ab5ef0be55ebdd63b6b63f0aa5474fd5620dc");
        let mut store = MemoryStore {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(1),
                state_root: root,
            },
            nodes: BTreeMap::new(),
            values: BTreeMap::new(),
        };
        for (hash, node) in [
            (
                "11eec08482c5316b7562422fade7059df9794c5b5fc0eddfe52d985f6d00b146",
                "f843a1200100000000000000000000000000000000000000000000000000000000000001a011eec08482c5316b7562422fade7059df9794c5b5fc0eddfe52d985f6d00b146",
            ),
            (
                "1b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15",
                "f89680f842a02000000000000000000000000000000000000000000000000000000000000001a0f9aa104e4c11dab9f96b33feb13e7ef0d37e8bcf2d6920172bc6d1c60392167bf842a02000000000000000000000000000000000000000000000000000000000000002a0a9d2c714abcf26a6371624afc6baf84e67dd9287f8f1408b07b99a30d5ed2b6e80808080808080808080808080",
            ),
            (
                "5469ad332e5de42c6030829cfaad5080425441be8066edfaaf63d6bf90d06150",
                "e210a01b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15",
            ),
            (
                "acde2a8675590a1f104ae5db7c4ab5ef0be55ebdd63b6b63f0aa5474fd5620dc",
                "f873a01b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf158080808080808080808080808080f842a031000000000000000000000000000000000000000000000000000000000000f1a0593d9d0f1d5cc62870cc75287b06bc5c1cb3b78cc91bd783234ee39d9f6ede14",
            ),
        ] {
            store.nodes.insert(decode_hex_32(hash), decode_hex(node));
        }
        for (key, value) in [
            (
                "0100000000000000000000000000000000000000000000000000000000000001",
                "01",
            ),
            (
                "0200000000000000000000000000000000000000000000000000000000000002",
                "0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000",
            ),
            (
                "f1000000000000000000000000000000000000000000000000000000000000f1",
                "03",
            ),
        ] {
            store.values.insert(decode_hex_32(key), decode_hex(value));
        }

        for key in store.values.keys().copied() {
            assert!(matches!(
                verify_path(
                    &store,
                    root,
                    key,
                    "nodes",
                    "values",
                    |path| path,
                    TrieSchema::Storage
                ),
                Ok(PathProof::Member(_))
            ));
        }
        let missing =
            decode_hex_32("0300000000000000000000000000000000000000000000000000000000000003");
        assert_eq!(
            verify_path(
                &store,
                root,
                missing,
                "nodes",
                "values",
                |path| path,
                TrieSchema::Storage,
            )
            .unwrap(),
            PathProof::NonMember
        );

        let expected_entries = store
            .values
            .iter()
            .map(|(key, value)| (*key, value.clone()))
            .collect::<Vec<_>>();
        let expected_value_bytes = expected_entries
            .iter()
            .map(|(_, value)| value.len() as u64)
            .sum();
        let inventory = inventory_storage_trie(
            &store,
            root,
            |path| path,
            InventoryLimits {
                max_nodes: 32,
                max_leaves: 3,
                max_value_bytes: expected_value_bytes,
            },
        )
        .unwrap();
        assert_eq!(inventory.entries, expected_entries);
        assert_eq!(inventory.value_bytes, expected_value_bytes);
        assert!(inventory.nodes_visited > inventory.entries.len() as u64);
        assert_eq!(
            inventory_storage_trie(
                &store,
                root,
                |path| path,
                InventoryLimits {
                    max_nodes: 0,
                    max_leaves: 3,
                    max_value_bytes: expected_value_bytes,
                },
            ),
            Err(InventoryError::LimitExceeded {
                resource: InventoryResource::Nodes,
                limit: 0,
                required: 1,
            })
        );
        assert!(matches!(
            inventory_storage_trie(
                &store,
                root,
                |path| path,
                InventoryLimits {
                    max_nodes: 32,
                    max_leaves: 2,
                    max_value_bytes: expected_value_bytes,
                },
            ),
            Err(InventoryError::LimitExceeded {
                resource: InventoryResource::Leaves,
                limit: 2,
                required: 3,
            })
        ));
        assert!(matches!(
            inventory_storage_trie(
                &store,
                root,
                |path| path,
                InventoryLimits {
                    max_nodes: 32,
                    max_leaves: 3,
                    max_value_bytes: expected_value_bytes - 1,
                },
            ),
            Err(InventoryError::LimitExceeded {
                resource: InventoryResource::ValueBytes,
                limit,
                required,
            }) if limit == expected_value_bytes - 1 && required > limit
        ));

        let mut incomplete = store.clone();
        incomplete.nodes.remove(&decode_hex_32(
            "1b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15",
        ));
        assert_eq!(
            verify_path(
                &incomplete,
                root,
                decode_hex_32("0100000000000000000000000000000000000000000000000000000000000001"),
                "nodes",
                "values",
                |path| path,
                TrieSchema::Storage,
            ),
            Err(ConcreteReadError::HistoryUnavailable(store.identity))
        );
        assert_eq!(
            inventory_storage_trie(
                &incomplete,
                root,
                |path| path,
                InventoryLimits {
                    max_nodes: 32,
                    max_leaves: 3,
                    max_value_bytes: expected_value_bytes,
                },
            ),
            Err(InventoryError::Read(ConcreteReadError::HistoryUnavailable(
                store.identity
            )))
        );
    }

    fn decode_hex(input: &str) -> Vec<u8> {
        assert_eq!(input.len() % 2, 0);
        (0..input.len())
            .step_by(2)
            .map(|index| {
                let digit = |value: u8| match value {
                    b'0'..=b'9' => value - b'0',
                    b'a'..=b'f' => value - b'a' + 10,
                    _ => panic!("invalid fixture hex"),
                };
                digit(input.as_bytes()[index]) << 4 | digit(input.as_bytes()[index + 1])
            })
            .collect()
    }

    fn decode_hex_32(input: &str) -> [u8; 32] {
        decode_hex(input).try_into().unwrap()
    }
}
