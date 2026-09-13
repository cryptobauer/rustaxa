//! Incremental updates for Taraxa's persisted Merkle-Patricia trie encoding.
//!
//! The writer resolves only nodes on touched paths. Unchanged hashed siblings
//! remain references, while changed branches and roots use the reference
//! 16-child physical encoding and 17-child canonical hash encoding. Values are
//! supplied separately because Taraxa versions them outside trie nodes.

use rlp::{Rlp, RlpStream};
use rustaxa_types::concrete_state::ConcreteReadError;

use super::codec::{account_commitment_rlp, corrupt, decode_physical_account, keccak256};
use super::physical_node::TrieSchema;

pub(crate) trait TrieWriteStore {
    fn node(&self, column: &str, hash: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError>;
    fn value(&self, key: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError>;
}

pub(crate) struct IncrementalTrie<'a, S> {
    store: &'a S,
    node_column: &'a str,
    schema: TrieSchema,
    root: Node,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrieCommit {
    pub(crate) root: [u8; 32],
    pub(crate) nodes: Vec<([u8; 32], Vec<u8>)>,
}

#[derive(Clone)]
enum Node {
    Empty,
    Hash([u8; 32]),
    Leaf { part: Vec<u8>, value: Vec<u8> },
    Extension { part: Vec<u8>, child: Box<Node> },
    Branch(Box<[Node; 16]>),
}

struct Encoded {
    canonical_field: Vec<u8>,
    physical_field: Vec<u8>,
    hash: Option<[u8; 32]>,
}

impl<'a, S: TrieWriteStore> IncrementalTrie<'a, S> {
    pub(crate) fn new(
        store: &'a S,
        node_column: &'a str,
        schema: TrieSchema,
        root: [u8; 32],
    ) -> Self {
        let root = if root == super::codec::empty_trie_root() {
            Node::Empty
        } else {
            Node::Hash(root)
        };
        Self {
            store,
            node_column,
            schema,
            root,
        }
    }

    /// Inserts or replaces one already-hashed 32-byte trie path.
    pub(crate) fn put(&mut self, key: [u8; 32], value: Vec<u8>) -> Result<(), ConcreteReadError> {
        if value.is_empty() {
            return Err(corrupt("a live trie value cannot be empty"));
        }
        let path = bytes_to_nibbles(key);
        self.root = insert(
            self.store,
            self.node_column,
            self.schema,
            std::mem::replace(&mut self.root, Node::Empty),
            &path,
            &[],
            value,
        )?;
        Ok(())
    }

    /// Deletes a path. Missing paths are a no-op, matching the reference trie,
    /// and return `false` so callers do not invent a physical tombstone.
    pub(crate) fn delete(&mut self, key: [u8; 32]) -> Result<bool, ConcreteReadError> {
        let path = bytes_to_nibbles(key);
        let (root, changed) = delete(
            self.store,
            self.node_column,
            self.schema,
            std::mem::replace(&mut self.root, Node::Empty),
            &path,
            &[],
        )?;
        self.root = root;
        Ok(changed)
    }

    /// Computes the root and the content-addressed node rows created by all
    /// preceding updates without mutating the backing store.
    pub(crate) fn commit(&self) -> Result<TrieCommit, ConcreteReadError> {
        if matches!(self.root, Node::Empty) {
            return Ok(TrieCommit {
                root: super::codec::empty_trie_root(),
                nodes: Vec::new(),
            });
        }
        if let Node::Hash(root) = self.root {
            return Ok(TrieCommit {
                root,
                nodes: Vec::new(),
            });
        }
        let mut nodes = Vec::new();
        let encoded = encode_node(self.schema, &self.root, true, &mut nodes)?;
        let root = encoded
            .hash
            .ok_or_else(|| corrupt("non-empty trie root was not hashed"))?;
        Ok(TrieCommit { root, nodes })
    }
}

fn insert<S: TrieWriteStore>(
    store: &S,
    node_column: &str,
    schema: TrieSchema,
    node: Node,
    remaining: &[u8],
    prefix: &[u8],
    value: Vec<u8>,
) -> Result<Node, ConcreteReadError> {
    match node {
        Node::Empty => Ok(Node::Leaf {
            part: remaining.to_vec(),
            value,
        }),
        Node::Hash(hash) => {
            let resolved = resolve_hash(store, node_column, schema, hash, prefix)?;
            insert(
                store,
                node_column,
                schema,
                resolved,
                remaining,
                prefix,
                value,
            )
        }
        Node::Leaf {
            part,
            value: old_value,
        } => {
            let common = common_prefix(remaining, &part);
            if common == part.len() && common == remaining.len() {
                return Ok(Node::Leaf { part, value });
            }
            let mut children = empty_children();
            let old_pivot = part[common] as usize;
            children[old_pivot] = if common + 1 == part.len() {
                Node::Leaf {
                    part: Vec::new(),
                    value: old_value,
                }
            } else {
                Node::Leaf {
                    part: part[common + 1..].to_vec(),
                    value: old_value,
                }
            };
            let new_pivot = remaining[common] as usize;
            children[new_pivot] = Node::Leaf {
                part: remaining[common + 1..].to_vec(),
                value,
            };
            let branch = Node::Branch(Box::new(children));
            if common == 0 {
                Ok(branch)
            } else {
                Ok(Node::Extension {
                    part: part[..common].to_vec(),
                    child: Box::new(branch),
                })
            }
        }
        Node::Extension { part, child } => {
            let common = common_prefix(remaining, &part);
            if common == part.len() {
                let mut child_prefix = prefix.to_vec();
                child_prefix.extend_from_slice(&part);
                let child = insert(
                    store,
                    node_column,
                    schema,
                    *child,
                    &remaining[common..],
                    &child_prefix,
                    value,
                )?;
                return Ok(Node::Extension {
                    part,
                    child: Box::new(child),
                });
            }
            let mut children = empty_children();
            let old_pivot = part[common] as usize;
            children[old_pivot] = if common + 1 == part.len() {
                *child
            } else {
                Node::Extension {
                    part: part[common + 1..].to_vec(),
                    child,
                }
            };
            let new_pivot = remaining[common] as usize;
            children[new_pivot] = Node::Leaf {
                part: remaining[common + 1..].to_vec(),
                value,
            };
            let branch = Node::Branch(Box::new(children));
            if common == 0 {
                Ok(branch)
            } else {
                Ok(Node::Extension {
                    part: part[..common].to_vec(),
                    child: Box::new(branch),
                })
            }
        }
        Node::Branch(mut children) => {
            let (&pivot, rest) = remaining
                .split_first()
                .ok_or_else(|| corrupt("branch reached the end of a fixed-width trie key"))?;
            let mut child_prefix = prefix.to_vec();
            child_prefix.push(pivot);
            children[pivot as usize] = insert(
                store,
                node_column,
                schema,
                std::mem::replace(&mut children[pivot as usize], Node::Empty),
                rest,
                &child_prefix,
                value,
            )?;
            Ok(Node::Branch(children))
        }
    }
}

fn delete<S: TrieWriteStore>(
    store: &S,
    node_column: &str,
    schema: TrieSchema,
    node: Node,
    remaining: &[u8],
    prefix: &[u8],
) -> Result<(Node, bool), ConcreteReadError> {
    match node {
        Node::Empty => Ok((Node::Empty, false)),
        Node::Hash(hash) => {
            let resolved = resolve_hash(store, node_column, schema, hash, prefix)?;
            let (updated, changed) =
                delete(store, node_column, schema, resolved, remaining, prefix)?;
            Ok((if changed { updated } else { Node::Hash(hash) }, changed))
        }
        Node::Leaf { part, value } => {
            if part == remaining {
                Ok((Node::Empty, true))
            } else {
                Ok((Node::Leaf { part, value }, false))
            }
        }
        Node::Extension { part, child } => {
            if !remaining.starts_with(&part) {
                return Ok((Node::Extension { part, child }, false));
            }
            let mut child_prefix = prefix.to_vec();
            child_prefix.extend_from_slice(&part);
            let (child, changed) = delete(
                store,
                node_column,
                schema,
                *child,
                &remaining[part.len()..],
                &child_prefix,
            )?;
            if !changed {
                return Ok((
                    Node::Extension {
                        part,
                        child: Box::new(child),
                    },
                    false,
                ));
            }
            match child {
                Node::Empty => Ok((Node::Empty, true)),
                Node::Leaf {
                    part: child_part,
                    value,
                } => Ok((
                    Node::Leaf {
                        part: [part, child_part].concat(),
                        value,
                    },
                    true,
                )),
                Node::Extension {
                    part: child_part,
                    child,
                } => Ok((
                    Node::Extension {
                        part: [part, child_part].concat(),
                        child,
                    },
                    true,
                )),
                child => Ok((
                    Node::Extension {
                        part,
                        child: Box::new(child),
                    },
                    true,
                )),
            }
        }
        Node::Branch(mut children) => {
            let Some((&pivot, rest)) = remaining.split_first() else {
                return Ok((Node::Branch(children), false));
            };
            let mut child_prefix = prefix.to_vec();
            child_prefix.push(pivot);
            let (child, changed) = delete(
                store,
                node_column,
                schema,
                std::mem::replace(&mut children[pivot as usize], Node::Empty),
                rest,
                &child_prefix,
            )?;
            children[pivot as usize] = child;
            if !changed {
                return Ok((Node::Branch(children), false));
            }
            let present = children
                .iter()
                .enumerate()
                .filter(|(_, child)| !matches!(child, Node::Empty))
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            if present.len() != 1 {
                return Ok((Node::Branch(children), true));
            }
            let only = present[0];
            let mut child = std::mem::replace(&mut children[only], Node::Empty);
            if let Node::Hash(hash) = child {
                let mut only_prefix = prefix.to_vec();
                only_prefix.push(only as u8);
                child = resolve_hash(store, node_column, schema, hash, &only_prefix)?;
            }
            let result = match child {
                Node::Leaf { part, value } => Node::Leaf {
                    part: [vec![only as u8], part].concat(),
                    value,
                },
                Node::Extension { part, child } => Node::Extension {
                    part: [vec![only as u8], part].concat(),
                    child,
                },
                child => Node::Extension {
                    part: vec![only as u8],
                    child: Box::new(child),
                },
            };
            Ok((result, true))
        }
    }
}

fn resolve_hash<S: TrieWriteStore>(
    store: &S,
    node_column: &str,
    schema: TrieSchema,
    hash: [u8; 32],
    prefix: &[u8],
) -> Result<Node, ConcreteReadError> {
    let raw = store
        .node(node_column, hash)?
        .ok_or_else(|| corrupt("missing content-addressed trie node"))?;
    if !exact_rlp(&raw)?.is_list() {
        return Err(corrupt("content-addressed trie node row must be a list"));
    }
    let node = decode_node(store, schema, &raw, prefix)?;
    let authenticated = encode_node(schema, &node, true, &mut Vec::new())?
        .hash
        .ok_or_else(|| corrupt("loaded trie node did not produce a hash"))?;
    if authenticated != hash {
        return Err(corrupt(
            "loaded trie node bytes do not match their hash key",
        ));
    }
    Ok(node)
}

fn decode_node<S: TrieWriteStore>(
    store: &S,
    schema: TrieSchema,
    raw: &[u8],
    prefix: &[u8],
) -> Result<Node, ConcreteReadError> {
    let rlp = exact_rlp(raw)?;
    if !rlp.is_list() {
        let data = rlp.data().map_err(corrupt)?;
        if data.is_empty() {
            return Ok(Node::Empty);
        }
        let hash = data
            .try_into()
            .map_err(|_| corrupt("trie reference must be empty or 32 bytes"))?;
        return Ok(Node::Hash(hash));
    }
    match rlp.item_count().map_err(corrupt)? {
        16 => {
            let mut children = empty_children();
            for (index, child) in children.iter_mut().enumerate() {
                let mut child_prefix = prefix.to_vec();
                child_prefix.push(index as u8);
                *child = decode_node(
                    store,
                    schema,
                    rlp.at(index).map_err(corrupt)?.as_raw(),
                    &child_prefix,
                )?;
            }
            Ok(Node::Branch(Box::new(children)))
        }
        1 | 2 => {
            let compact = rlp.at(0).map_err(corrupt)?.data().map_err(corrupt)?;
            let (terminal, part) = decode_compact(compact)?;
            if terminal {
                let full = [prefix, &part].concat();
                if full.len() != 64 {
                    return Err(corrupt("leaf path is not 32 bytes"));
                }
                let key = nibbles_to_bytes(&full)?;
                let value = store
                    .value(key)?
                    .ok_or_else(|| corrupt("trie leaf has no selected version value"))?;
                if value.is_empty() {
                    return Err(corrupt("trie leaf selects a tombstone"));
                }
                let count = rlp.item_count().map_err(corrupt)?;
                let content = if count == 2 {
                    rlp.at(1).map_err(corrupt)?.data().map_err(corrupt)?
                } else {
                    &[]
                };
                if !content.is_empty() && content.len() != 32 && content.len() > 8 {
                    return Err(corrupt("physical leaf has invalid inline value width"));
                }
                if !content.is_empty() && content.len() <= 8 && content != value {
                    return Err(corrupt("physical inline leaf and version value differ"));
                }
                let commitment = match schema {
                    TrieSchema::Account => {
                        account_commitment_rlp(&decode_physical_account(&value)?)?
                    }
                    TrieSchema::Storage => rlp::encode(&value.as_slice()).to_vec(),
                };
                let mut canonical = RlpStream::new_list(2);
                canonical.append(&compact);
                canonical.append(&commitment.as_slice());
                if content.len() == 32 && keccak256(&canonical.out()).as_slice() != content {
                    return Err(corrupt("physical leaf hash hint differs from its value"));
                }
                Ok(Node::Leaf { part, value })
            } else {
                if rlp.item_count().map_err(corrupt)? != 2 || part.is_empty() {
                    return Err(corrupt("invalid physical extension"));
                }
                let child_prefix = [prefix, &part].concat();
                let child = decode_node(
                    store,
                    schema,
                    rlp.at(1).map_err(corrupt)?.as_raw(),
                    &child_prefix,
                )?;
                Ok(Node::Extension {
                    part,
                    child: Box::new(child),
                })
            }
        }
        _ => Err(corrupt("invalid physical trie node arity")),
    }
}

fn encode_node(
    schema: TrieSchema,
    node: &Node,
    is_root: bool,
    writes: &mut Vec<([u8; 32], Vec<u8>)>,
) -> Result<Encoded, ConcreteReadError> {
    match node {
        Node::Empty => Ok(Encoded {
            canonical_field: vec![0x80],
            physical_field: vec![0x80],
            hash: None,
        }),
        Node::Hash(hash) => {
            let field = rlp::encode(&hash.as_slice()).to_vec();
            Ok(Encoded {
                canonical_field: field.clone(),
                physical_field: field,
                hash: Some(*hash),
            })
        }
        Node::Leaf { part, value } => {
            let compact = encode_compact(part, true);
            let commitment = match schema {
                TrieSchema::Account => account_commitment_rlp(&decode_physical_account(value)?)?,
                TrieSchema::Storage => rlp::encode(&value.as_slice()).to_vec(),
            };
            let mut canonical = RlpStream::new_list(2);
            canonical.append(&compact.as_slice());
            canonical.append(&commitment.as_slice());
            let canonical = canonical.out().to_vec();
            let hash = (is_root || canonical.len() >= 32).then(|| keccak256(&canonical));
            let mut physical = RlpStream::new_list(if hash.is_some() || value.len() <= 8 {
                2
            } else {
                1
            });
            physical.append(&compact.as_slice());
            if let Some(hash) = hash {
                physical.append(&hash.as_slice());
            } else if value.len() <= 8 {
                physical.append(&value.as_slice());
            }
            let physical = physical.out().to_vec();
            if is_root {
                writes.push((hash.expect("root is hashed"), physical.clone()));
            }
            Ok(Encoded {
                canonical_field: hash
                    .map(|hash| rlp::encode(&hash.as_slice()).to_vec())
                    .unwrap_or_else(|| canonical.clone()),
                physical_field: physical,
                hash,
            })
        }
        Node::Extension { part, child } => {
            let child = encode_node(schema, child, false, writes)?;
            let compact = encode_compact(part, false);
            let mut canonical = RlpStream::new_list(2);
            canonical.append(&compact.as_slice());
            canonical.append_raw(&child.canonical_field, 1);
            let canonical = canonical.out().to_vec();
            let hash = (is_root || canonical.len() >= 32).then(|| keccak256(&canonical));
            let mut physical = RlpStream::new_list(2);
            physical.append(&compact.as_slice());
            physical.append_raw(&child.physical_field, 1);
            let physical = physical.out().to_vec();
            if is_root {
                writes.push((hash.expect("root is hashed"), physical.clone()));
            }
            Ok(Encoded {
                canonical_field: hash
                    .map(|hash| rlp::encode(&hash.as_slice()).to_vec())
                    .unwrap_or_else(|| canonical.clone()),
                physical_field: physical,
                hash,
            })
        }
        Node::Branch(children) => {
            let mut canonical = RlpStream::new_list(17);
            let mut physical = RlpStream::new_list(16);
            for child in children.iter() {
                let child = encode_node(schema, child, false, writes)?;
                canonical.append_raw(&child.canonical_field, 1);
                physical.append_raw(&child.physical_field, 1);
            }
            canonical.append_empty_data();
            let canonical = canonical.out().to_vec();
            let physical = physical.out().to_vec();
            let hash = (is_root || canonical.len() >= 32).then(|| keccak256(&canonical));
            if let Some(hash) = hash {
                writes.push((hash, physical.clone()));
            }
            let physical_field = match (hash, is_root) {
                (Some(hash), false) => rlp::encode(&hash.as_slice()).to_vec(),
                _ => physical,
            };
            Ok(Encoded {
                canonical_field: hash
                    .map(|hash| rlp::encode(&hash.as_slice()).to_vec())
                    .unwrap_or_else(|| canonical.clone()),
                physical_field,
                hash,
            })
        }
    }
}

fn exact_rlp(bytes: &[u8]) -> Result<Rlp<'_>, ConcreteReadError> {
    let rlp = Rlp::new(bytes);
    if rlp.payload_info().map_err(corrupt)?.total() != bytes.len() {
        return Err(corrupt("physical trie node has trailing bytes"));
    }
    Ok(rlp)
}

fn empty_children() -> [Node; 16] {
    std::array::from_fn(|_| Node::Empty)
}

fn common_prefix(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn bytes_to_nibbles(bytes: [u8; 32]) -> [u8; 64] {
    let mut nibbles = [0_u8; 64];
    for (index, byte) in bytes.into_iter().enumerate() {
        nibbles[index * 2] = byte >> 4;
        nibbles[index * 2 + 1] = byte & 0x0f;
    }
    nibbles
}

fn nibbles_to_bytes(nibbles: &[u8]) -> Result<[u8; 32], ConcreteReadError> {
    if nibbles.len() != 64 || nibbles.iter().any(|nibble| *nibble > 0x0f) {
        return Err(corrupt("invalid fixed-width nibble path"));
    }
    let mut bytes = [0_u8; 32];
    for index in 0..32 {
        bytes[index] = (nibbles[index * 2] << 4) | nibbles[index * 2 + 1];
    }
    Ok(bytes)
}

fn encode_compact(part: &[u8], terminal: bool) -> Vec<u8> {
    let odd = part.len() % 2 == 1;
    let mut compact = vec![0_u8; part.len() / 2 + 1];
    compact[0] = (terminal as u8) << 5;
    let mut source = part;
    if odd {
        compact[0] |= 0x10 | part[0];
        source = &part[1..];
    }
    for (index, pair) in source.as_chunks::<2>().0.iter().enumerate() {
        compact[index + 1] = (pair[0] << 4) | pair[1];
    }
    compact
}

fn decode_compact(compact: &[u8]) -> Result<(bool, Vec<u8>), ConcreteReadError> {
    if compact.is_empty() || compact[0] & 0xc0 != 0 {
        return Err(corrupt("invalid compact trie key flags"));
    }
    let terminal = compact[0] & 0x20 != 0;
    let odd = compact[0] & 0x10 != 0;
    if !odd && compact[0] & 0x0f != 0 {
        return Err(corrupt("invalid compact trie key padding"));
    }
    let mut part = Vec::with_capacity(compact.len() * 2);
    if odd {
        part.push(compact[0] & 0x0f);
    }
    for byte in &compact[1..] {
        part.push(byte >> 4);
        part.push(byte & 0x0f);
    }
    Ok((terminal, part))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        nodes: RefCell<BTreeMap<[u8; 32], Vec<u8>>>,
        values: RefCell<BTreeMap<[u8; 32], Vec<u8>>>,
    }

    impl TrieWriteStore for MemoryStore {
        fn node(
            &self,
            _column: &str,
            hash: [u8; 32],
        ) -> Result<Option<Vec<u8>>, ConcreteReadError> {
            Ok(self.nodes.borrow().get(&hash).cloned())
        }

        fn value(&self, key: [u8; 32]) -> Result<Option<Vec<u8>>, ConcreteReadError> {
            Ok(self.values.borrow().get(&key).cloned())
        }
    }

    #[test]
    fn matches_pinned_go_incremental_node_history() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../experiments/evm_feasibility/fixtures/public.json"
        ))
        .unwrap();
        let history = fixture["node_history"].as_array().unwrap();
        let store = MemoryStore::default();
        let mut root = super::super::codec::empty_trie_root();
        for step in history {
            let key = decode_hash(step["key"].as_str().unwrap());
            let value = hex::decode(step["value"].as_str().unwrap()).unwrap();
            let mut trie = IncrementalTrie::new(&store, "nodes", TrieSchema::Storage, root);
            if value.is_empty() {
                trie.delete(key).unwrap();
            } else {
                trie.put(key, value.clone()).unwrap();
            }
            let commit = trie.commit().unwrap();
            root = commit.root;
            store.values.borrow_mut().insert(key, value);
            for (hash, node) in commit.nodes {
                store.nodes.borrow_mut().insert(hash, node);
            }
            assert_eq!(hex::encode(root), step["root"].as_str().unwrap());
            let expected = step["nodes"].as_object().unwrap();
            assert_eq!(store.nodes.borrow().len(), expected.len());
            for (hash, node) in expected {
                assert_eq!(
                    store.nodes.borrow().get(&decode_hash(hash)).unwrap(),
                    &hex::decode(node.as_str().unwrap()).unwrap()
                );
            }
        }
    }

    #[test]
    fn authenticates_surviving_hashed_sibling_before_branch_collapse() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../experiments/evm_feasibility/fixtures/public.json"
        ))
        .unwrap();
        let step = &fixture["node_history"][2];
        let store = MemoryStore::default();
        for (hash, node) in step["nodes"].as_object().unwrap() {
            store.nodes.borrow_mut().insert(
                decode_hash(hash),
                hex::decode(node.as_str().unwrap()).unwrap(),
            );
        }
        for (key, value) in step["values"].as_object().unwrap() {
            store.values.borrow_mut().insert(
                decode_hash(key),
                hex::decode(value.as_str().unwrap()).unwrap(),
            );
        }
        let sibling =
            decode_hash("1b174252b29aa58b9c5446f818d6a89f1987b3502d5ec8fc0bb0807433eaaf15");
        store
            .nodes
            .borrow_mut()
            .insert(sibling, rlp::encode(&sibling.as_slice()).to_vec());
        let root = decode_hash(step["root"].as_str().unwrap());
        let mut trie = IncrementalTrie::new(&store, "nodes", TrieSchema::Storage, root);
        let deleted = decode_hash(step["key"].as_str().unwrap());
        assert!(trie.delete(deleted).is_err());
    }

    fn decode_hash(hexadecimal: &str) -> [u8; 32] {
        hex::decode(hexadecimal).unwrap().try_into().unwrap()
    }
}
