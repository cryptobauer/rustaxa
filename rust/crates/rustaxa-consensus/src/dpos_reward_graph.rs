//! Deterministic DPoS reward-graph codec and mutation helpers.
//!
//! The module owns a compact persistence-ready snapshot of validator reward
//! bookkeeping used by reward-claim flows. All mutation helpers are explicit and
//! validate dangling refs, duplicate keys, and count bounds before mutating
//! in-memory state.

use num_bigint::BigUint;
use rlp::{DecoderError, Rlp, RlpStream};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Ethereum-style address type for validator identity.
pub type Validator = [u8; 20];

/// Ethereum-style address type for delegator identity.
pub type Delegator = [u8; 20];

/// Key used for reward nodes: `(validator, block)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct NodeKey {
    pub validator: Validator,
    pub block: u64,
}

/// Internal reward node value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Node {
    pub reward_per_stake: BigUint,
    pub count: u32,
}

const SCHEMA_VERSION: u8 = 1;
const MAX_U256_BYTES: usize = 32;

/// In-memory deterministic storage shape for DPoS reward graph state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DposRewardGraph {
    nodes: BTreeMap<NodeKey, Node>,
    validator_heads: BTreeMap<Validator, u64>,
    delegation_cursors: BTreeMap<(Validator, Delegator), u64>,
    history_complete: bool,

    stale_validator_heads: BTreeSet<Validator>,
    current_block: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Typed failures for graph decoding, arithmetic, and reference updates.
/// Public mutations stage a clone, so any error leaves the receiver unchanged.
pub enum DposRewardGraphError {
    UnsupportedSchema(u8),
    InvalidRlp(String),
    InvalidAddress(&'static str),
    DuplicateNode {
        validator: Validator,
        block: u64,
    },
    DuplicateCursor {
        validator: Validator,
        delegator: Delegator,
    },
    DuplicateHead {
        validator: Validator,
    },
    DuplicateStaleHead {
        validator: Validator,
    },
    DanglingHead {
        validator: Validator,
        block: u64,
    },
    DanglingCursor {
        validator: Validator,
        delegator: Delegator,
        block: u64,
    },
    MissingNode {
        validator: Validator,
        block: u64,
    },
    MissingCursor {
        validator: Validator,
        delegator: Delegator,
    },
    MissingHead {
        validator: Validator,
    },
    GraphHistoryIncomplete,
    CountMismatch {
        validator: Validator,
        block: u64,
    },
    CountOverflow {
        validator: Validator,
        block: u64,
    },
    CountUnderflow {
        validator: Validator,
        block: u64,
    },
    CheckpointRegression {
        current: u64,
        next: u64,
    },
    MissingStaleHead {
        validator: Validator,
    },
    StaleHeadConflict {
        validator: Validator,
    },
    ZeroDenominator,
    AmountOverU256 {
        field: &'static str,
    },
    ArithmeticUnderflow {
        left: &'static str,
        right: &'static str,
    },
}

impl fmt::Display for DposRewardGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use DposRewardGraphError::*;
        match self {
            UnsupportedSchema(version) => write!(f, "unsupported reward graph schema: {version}"),
            InvalidRlp(message) => write!(f, "invalid reward graph rlp: {message}"),
            InvalidAddress(field) => write!(f, "invalid address for {field}"),
            DuplicateNode { validator, block } => {
                write!(f, "duplicate node ({validator:?}, {block})")
            }
            DuplicateCursor {
                validator,
                delegator,
            } => write!(f, "duplicate cursor ({validator:?}, {delegator:?})"),
            DuplicateHead { validator } => write!(f, "duplicate head for validator {validator:?}"),
            DuplicateStaleHead { validator } => {
                write!(f, "duplicate stale head marker for validator {validator:?}")
            }
            DanglingHead { validator, block } => {
                write!(f, "dangling validator head {validator:?} -> {block}")
            }
            DanglingCursor {
                validator,
                delegator,
                block,
            } => write!(
                f,
                "dangling cursor ({validator:?}, {delegator:?}) -> {block}"
            ),
            MissingNode { validator, block } => {
                write!(f, "missing node ({validator:?}, {block})")
            }
            MissingCursor {
                validator,
                delegator,
            } => {
                write!(f, "missing cursor ({validator:?}, {delegator:?})")
            }
            MissingHead { validator } => {
                write!(f, "missing validator head for {validator:?}")
            }
            GraphHistoryIncomplete => write!(f, "graph history is incomplete"),
            CountMismatch { validator, block } => {
                write!(f, "cursor count mismatch at ({validator:?}, {block})")
            }
            CountOverflow { validator, block } => {
                write!(f, "count overflow at ({validator:?}, {block})")
            }
            CountUnderflow { validator, block } => {
                write!(f, "count underflow at ({validator:?}, {block})")
            }
            CheckpointRegression { current, next } => {
                write!(f, "checkpoint regressed {current} -> {next}")
            }
            MissingStaleHead { validator } => {
                write!(f, "missing stale head marker for validator {validator:?}")
            }
            StaleHeadConflict { validator } => {
                write!(
                    f,
                    "stale validator head still references a live node for {validator:?}"
                )
            }
            ZeroDenominator => write!(f, "total_stake is zero"),
            AmountOverU256 { field } => write!(f, "{field} exceeds 256-bit width"),
            ArithmeticUnderflow { left, right } => write!(f, "{left} < {right}"),
        }
    }
}

impl std::error::Error for DposRewardGraphError {}

impl Default for DposRewardGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl DposRewardGraph {
    /// Creates an empty, history-complete graph at block zero.
    /// Initial nodes and references must be added through `bootstrap_node`.
    pub fn new() -> Self {
        Self {
            nodes: BTreeMap::new(),
            validator_heads: BTreeMap::new(),
            delegation_cursors: BTreeMap::new(),
            history_complete: true,
            stale_validator_heads: BTreeSet::new(),
            current_block: 0,
        }
    }

    /// Returns the checkpoint block, or rejects incomplete legacy provenance.
    pub fn current_block(&self) -> Result<u64, DposRewardGraphError> {
        self.ensure_complete()?;
        Ok(self.current_block)
    }

    /// Constructs provenance for a legacy snapshot whose reward-reference
    /// history cannot be reconstructed locally. Incomplete graphs may be
    /// encoded and decoded, but cannot serve reads or mutations.
    pub fn incomplete() -> Self {
        Self {
            history_complete: false,
            ..Self::new()
        }
    }

    fn ensure_complete(&self) -> Result<(), DposRewardGraphError> {
        if self.history_complete {
            Ok(())
        } else {
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        }
    }

    /// Reports the explicitly permitted legacy stale-head state.
    /// Incomplete provenance cannot answer this query.
    pub fn is_stale_head(&self, validator: &Validator) -> Result<bool, DposRewardGraphError> {
        self.ensure_complete()?;
        Ok(self.stale_validator_heads.contains(validator))
    }

    /// Loads one arbitrary-width reward node by validator and block.
    /// Missing nodes and incomplete history are hard errors.
    pub fn load_node(&self, key: &NodeKey) -> Result<Node, DposRewardGraphError> {
        self.ensure_complete()?;
        self.nodes
            .get(key)
            .cloned()
            .ok_or(DposRewardGraphError::MissingNode {
                validator: key.validator,
                block: key.block,
            })
    }

    /// Atomically inserts a checkpoint node and assigns the requested references.
    ///
    /// Existing requested references are moved from their prior nodes, whose
    /// counts are decremented first. A stale validator head is retained when
    /// another cursor still references its node. `node.count` must already
    /// include every requested reference at the new checkpoint; it is not
    /// incremented here.
    /// Extra positive count is preserved as legacy orphan bookkeeping, while
    /// duplicate nodes and undercounted references fail
    /// without changing the graph.
    pub fn bootstrap_node(
        &mut self,
        key: NodeKey,
        node: Node,
        validator_head: bool,
        delegators: &[Delegator],
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            if graph.nodes.contains_key(&key) {
                return Err(DposRewardGraphError::DuplicateNode {
                    validator: key.validator,
                    block: key.block,
                });
            }
            let mut unique_delegators = BTreeSet::new();
            for delegator in delegators {
                if !unique_delegators.insert(*delegator) {
                    return Err(DposRewardGraphError::DuplicateCursor {
                        validator: key.validator,
                        delegator: *delegator,
                    });
                }
            }

            if validator_head
                && let Some(previous_block) = graph.validator_heads.get(&key.validator).copied()
                && !graph.stale_validator_heads.contains(&key.validator)
            {
                graph.decrement_count(&NodeKey {
                    validator: key.validator,
                    block: previous_block,
                })?;
            }
            for delegator in &unique_delegators {
                if let Some(previous_block) = graph
                    .delegation_cursors
                    .get(&(key.validator, *delegator))
                    .copied()
                {
                    graph.decrement_count(&NodeKey {
                        validator: key.validator,
                        block: previous_block,
                    })?;
                }
            }

            graph.nodes.insert(key, node);
            if validator_head {
                graph.validator_heads.insert(key.validator, key.block);
                graph.stale_validator_heads.remove(&key.validator);
            }
            for delegator in unique_delegators {
                graph
                    .delegation_cursors
                    .insert((key.validator, delegator), key.block);
            }
            Ok(())
        })
    }

    /// Returns a validator's explicit reward checkpoint head.
    /// Missing heads and incomplete provenance are hard errors.
    pub fn read_validator_head(&self, validator: &Validator) -> Result<u64, DposRewardGraphError> {
        self.ensure_complete()?;
        self.validator_heads
            .get(validator)
            .copied()
            .ok_or(DposRewardGraphError::MissingHead {
                validator: *validator,
            })
    }

    fn apply_mutation<F>(&mut self, mutator: F) -> Result<(), DposRewardGraphError>
    where
        F: FnOnce(&mut Self) -> Result<(), DposRewardGraphError>,
    {
        if !self.history_complete {
            return Err(DposRewardGraphError::GraphHistoryIncomplete);
        }
        let mut graph = self.clone();
        mutator(&mut graph)?;
        if graph.history_complete {
            graph.validate_count_contract()?;
        }
        *self = graph;
        Ok(())
    }

    /// Moves or creates a validator-head reference and adjusts both node counts.
    /// Missing nodes, count bounds, and stale conflicts fail atomically.
    pub fn attach_validator_head(
        &mut self,
        validator: Validator,
        head_block: u64,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            let head_node_key = NodeKey {
                validator,
                block: head_block,
            };
            if !graph.nodes.contains_key(&head_node_key) {
                return Err(DposRewardGraphError::MissingNode {
                    validator,
                    block: head_block,
                });
            }

            if let Some(previous_block) = graph.validator_heads.get(&validator).copied() {
                if previous_block == head_block {
                    graph.stale_validator_heads.remove(&validator);
                    return Ok(());
                }
                graph.decrement_count(&NodeKey {
                    validator,
                    block: previous_block,
                })?;
                graph.increment_count(&head_node_key)?;
                graph.validator_heads.insert(validator, head_block);
                graph.stale_validator_heads.remove(&validator);
                return Ok(());
            }

            graph.increment_count(&head_node_key)?;
            graph.validator_heads.insert(validator, head_block);
            graph.stale_validator_heads.remove(&validator);
            Ok(())
        })
    }

    /// Corrects a stale validator head without changing stored counts.
    /// The stale head may remain live when another delegation cursor retains
    /// it; the affected delegation cursor must identify `head_block`, and the
    /// target node must exist, preventing rebinding to an arbitrary orphan.
    pub fn rebind_stale_validator_head(
        &mut self,
        validator: Validator,
        affected_delegator: Delegator,
        head_block: u64,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            if !graph.stale_validator_heads.contains(&validator) {
                return Err(DposRewardGraphError::MissingStaleHead { validator });
            }
            let stale_block = graph
                .validator_heads
                .get(&validator)
                .copied()
                .ok_or(DposRewardGraphError::MissingStaleHead { validator })?;
            if graph.nodes.contains_key(&NodeKey {
                validator,
                block: stale_block,
            }) && !graph
                .delegation_cursors
                .iter()
                .any(|((cursor_validator, _), block)| {
                    *cursor_validator == validator && *block == stale_block
                })
            {
                return Err(DposRewardGraphError::StaleHeadConflict { validator });
            }
            if stale_block == head_block {
                return Err(DposRewardGraphError::MissingStaleHead { validator });
            }
            if !graph.nodes.contains_key(&NodeKey {
                validator,
                block: head_block,
            }) {
                return Err(DposRewardGraphError::MissingNode {
                    validator,
                    block: head_block,
                });
            }
            if graph
                .delegation_cursors
                .get(&(validator, affected_delegator))
                .copied()
                != Some(head_block)
            {
                return Err(DposRewardGraphError::DanglingCursor {
                    validator,
                    delegator: affected_delegator,
                    block: head_block,
                });
            }
            graph.validator_heads.insert(validator, head_block);
            graph.stale_validator_heads.remove(&validator);
            Ok(())
        })
    }

    /// Reproduces the pre-fix stale-head transition: decrement/delete the old
    /// node while retaining its head block and adding the stale marker.
    pub fn detach_validator_head(
        &mut self,
        validator: &Validator,
    ) -> Result<Option<u64>, DposRewardGraphError> {
        let mut detached = None;
        self.apply_mutation(|graph| {
            if graph.stale_validator_heads.contains(validator) {
                return Err(DposRewardGraphError::StaleHeadConflict {
                    validator: *validator,
                });
            }
            let Some(head_block) = graph.validator_heads.get(validator).copied() else {
                return Ok(());
            };
            graph.decrement_count(&NodeKey {
                validator: *validator,
                block: head_block,
            })?;
            graph.stale_validator_heads.insert(*validator);
            detached = Some(head_block);
            Ok(())
        })?;
        Ok(detached)
    }

    /// Removes a live validator head node and clears all references.
    /// This is used for normal validator deletion, where no stale marker should
    /// be retained after ownership transition.
    pub fn delete_validator_head(
        &mut self,
        validator: &Validator,
    ) -> Result<u64, DposRewardGraphError> {
        let mut deleted = None;
        self.apply_mutation(|graph| {
            if graph.stale_validator_heads.contains(validator) {
                return Err(DposRewardGraphError::StaleHeadConflict {
                    validator: *validator,
                });
            }
            let head_block = graph.validator_heads.remove(validator).ok_or(
                DposRewardGraphError::MissingHead {
                    validator: *validator,
                },
            )?;
            graph.decrement_count(&NodeKey {
                validator: *validator,
                block: head_block,
            })?;
            deleted = Some(head_block);
            Ok(())
        })?;
        deleted.ok_or(DposRewardGraphError::MissingHead {
            validator: *validator,
        })
    }

    /// Creates a validator graph root, replacing only a detached same-key
    /// orphan left by normal terminal deletion.
    ///
    /// Registration cannot coexist with a validator head or delegation cursor
    /// for the address. An unreferenced node at `key` is legacy storage that
    /// registration overwrites with reward-per-stake zero and fresh counts.
    pub fn register_validator(
        &mut self,
        key: NodeKey,
        node: Node,
        delegators: &[Delegator],
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            if graph.validator_heads.contains_key(&key.validator) {
                return Err(DposRewardGraphError::DuplicateHead {
                    validator: key.validator,
                });
            }
            if let Some((_, existing_delegator)) = graph
                .delegation_cursors
                .keys()
                .find(|(validator, _)| *validator == key.validator)
                .copied()
            {
                return Err(DposRewardGraphError::DuplicateCursor {
                    validator: key.validator,
                    delegator: existing_delegator,
                });
            }
            let mut unique_delegators = BTreeSet::new();
            for delegator in delegators {
                if !unique_delegators.insert(*delegator) {
                    return Err(DposRewardGraphError::DuplicateCursor {
                        validator: key.validator,
                        delegator: *delegator,
                    });
                }
            }
            graph.nodes.insert(key, node);
            graph.validator_heads.insert(key.validator, key.block);
            graph.stale_validator_heads.remove(&key.validator);
            for delegator in unique_delegators {
                graph
                    .delegation_cursors
                    .insert((key.validator, delegator), key.block);
            }
            Ok(())
        })
    }

    /// Reproduces legacy terminal deletion that removes the head checkpoint
    /// regardless of its persisted count. Dangling live cursors make the staged
    /// graph invalid instead of being silently repaired.
    pub fn force_delete_validator_head(
        &mut self,
        validator: &Validator,
    ) -> Result<u64, DposRewardGraphError> {
        let mut deleted = None;
        self.apply_mutation(|graph| {
            if graph.stale_validator_heads.contains(validator) {
                return Err(DposRewardGraphError::StaleHeadConflict {
                    validator: *validator,
                });
            }
            let head_block = graph.validator_heads.remove(validator).ok_or(
                DposRewardGraphError::MissingHead {
                    validator: *validator,
                },
            )?;
            graph.nodes.remove(&NodeKey {
                validator: *validator,
                block: head_block,
            });
            deleted = Some(head_block);
            Ok(())
        })?;
        deleted.ok_or(DposRewardGraphError::MissingHead {
            validator: *validator,
        })
    }

    /// Persists the stale validator copy written by a pre-fix same-validator
    /// redelegation without changing any node count.
    ///
    /// The stale block may still resolve when another cursor retains it. The
    /// configured correction rejects that topology, matching the legacy panic.
    pub fn overwrite_validator_head_stale(
        &mut self,
        validator: Validator,
        stale_block: u64,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            if !graph.validator_heads.contains_key(&validator) {
                return Err(DposRewardGraphError::MissingHead { validator });
            }
            graph.validator_heads.insert(validator, stale_block);
            graph.stale_validator_heads.insert(validator);
            Ok(())
        })
    }

    /// Creates or moves a delegation cursor with explicit count bookkeeping.
    /// Same-key writes preserve legacy load-copy/decrement-delete/increment/
    /// write-last ordering, including count inflation and node resurrection.
    pub fn write_cursor(
        &mut self,
        validator: Validator,
        delegator: Delegator,
        new_block: u64,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            let cursor_key = (validator, delegator);
            let new_node = NodeKey {
                validator,
                block: new_block,
            };
            if !graph.nodes.contains_key(&new_node) {
                return Err(DposRewardGraphError::MissingNode {
                    validator,
                    block: new_block,
                });
            }

            let previous = graph.delegation_cursors.get(&cursor_key).copied();
            if let Some(previous_block) = previous {
                if previous_block == new_block {
                    // Preserve Go's load-copy-write ordering when the cursor and
                    // current node alias: decrement/delete the persisted node,
                    // then increment and write the earlier loaded copy last.
                    let mut loaded = graph.load_node(&new_node)?;
                    graph.decrement_count(&new_node)?;
                    loaded.count =
                        loaded
                            .count
                            .checked_add(1)
                            .ok_or(DposRewardGraphError::CountOverflow {
                                validator,
                                block: new_block,
                            })?;
                    graph.nodes.insert(new_node, loaded);
                    return Ok(());
                }
                graph.decrement_count(&NodeKey {
                    validator,
                    block: previous_block,
                })?;
            }

            graph.increment_count(&new_node)?;
            graph.delegation_cursors.insert(cursor_key, new_block);
            Ok(())
        })
    }

    /// Writes one delegation cursor, creating it when no prior cursor exists.
    ///
    /// This mirrors legacy create-or-update cursor semantics by preserving
    /// overwrite ordering for existing cursors and introducing a new count
    /// reference when the delegation had no cursor yet.
    pub fn write_or_create_cursor(
        &mut self,
        validator: Validator,
        delegator: Delegator,
        new_block: u64,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            let cursor_key = (validator, delegator);
            if graph.delegation_cursors.contains_key(&cursor_key) {
                return graph.write_cursor(validator, delegator, new_block);
            }
            let new_node = NodeKey {
                validator,
                block: new_block,
            };
            if !graph.nodes.contains_key(&new_node) {
                return Err(DposRewardGraphError::MissingNode {
                    validator,
                    block: new_block,
                });
            }
            graph.increment_count(&new_node)?;
            graph.delegation_cursors.insert(cursor_key, new_block);
            Ok(())
        })
    }

    /// Replays a legacy write of an earlier-loaded node copy.
    ///
    /// Pre-fix repeated full same-validator redelegation loaded the current
    /// node before deleting the source cursor, then persisted that earlier
    /// copy after the deletion. The replacement must retain the exact loaded
    /// reward accumulator and count; it may only restore bookkeeping that the
    /// staged mutation reduced, never lower or invent a new node.
    pub fn restore_loaded_node(
        &mut self,
        key: NodeKey,
        loaded: Node,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            let current = graph
                .nodes
                .get(&key)
                .ok_or(DposRewardGraphError::MissingNode {
                    validator: key.validator,
                    block: key.block,
                })?;
            if loaded.count == 0 || loaded.count < current.count {
                return Err(DposRewardGraphError::CountMismatch {
                    validator: key.validator,
                    block: key.block,
                });
            }
            graph.nodes.insert(key, loaded);
            Ok(())
        })
    }

    /// Returns one delegation's block-key cursor.
    /// Missing cursors and incomplete provenance are hard errors.
    pub fn read_cursor(
        &self,
        validator: &Validator,
        delegator: &Delegator,
    ) -> Result<u64, DposRewardGraphError> {
        self.ensure_complete()?;
        self.delegation_cursors
            .get(&(*validator, *delegator))
            .copied()
            .ok_or(DposRewardGraphError::MissingCursor {
                validator: *validator,
                delegator: *delegator,
            })
    }

    /// Removes a cursor and decrements its node, deleting the node at count one.
    /// All failures leave the graph unchanged.
    pub fn delete_cursor(
        &mut self,
        validator: &Validator,
        delegator: &Delegator,
    ) -> Result<u64, DposRewardGraphError> {
        let mut removed_block = None;
        self.apply_mutation(|graph| {
            let cursor_key = (*validator, *delegator);
            let block = graph.delegation_cursors.remove(&cursor_key).ok_or(
                DposRewardGraphError::MissingCursor {
                    validator: *validator,
                    delegator: *delegator,
                },
            )?;
            graph.decrement_count(&NodeKey {
                validator: *validator,
                block,
            })?;
            removed_block = Some(block);
            Ok(())
        })?;
        removed_block.ok_or(DposRewardGraphError::MissingCursor {
            validator: *validator,
            delegator: *delegator,
        })
    }

    /// Replaces one existing node's arbitrary-width accumulator atomically.
    pub fn set_node_reward_per_stake(
        &mut self,
        validator: Validator,
        block: u64,
        reward_per_stake: BigUint,
    ) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            let key = NodeKey { validator, block };
            let node = graph
                .nodes
                .get_mut(&key)
                .ok_or(DposRewardGraphError::MissingNode { validator, block })?;
            node.reward_per_stake = reward_per_stake;
            Ok(())
        })
    }

    /// Advances the graph block monotonically without recomputing counts.
    pub fn next_block(&mut self, next_block: u64) -> Result<(), DposRewardGraphError> {
        self.apply_mutation(|graph| {
            if next_block < graph.current_block {
                return Err(DposRewardGraphError::CheckpointRegression {
                    current: graph.current_block,
                    next: next_block,
                });
            }
            graph.current_block = next_block;
            Ok(())
        })
    }

    /// Computes exact cumulative reward-per-stake from uint256 domain inputs.
    /// Intermediates remain arbitrary-width; zero stake and over-wide inputs fail.
    pub fn reward_per_stake(
        &self,
        head_rps: &BigUint,
        pool: &[u8],
        max_stake: &[u8],
        total_stake: &[u8],
    ) -> Result<BigUint, DposRewardGraphError> {
        self.ensure_complete()?;
        reward_per_stake(
            head_rps,
            decode_u256_like_bytes("pool", pool)?,
            decode_u256_like_bytes("max_stake", max_stake)?,
            decode_u256_like_bytes("total_stake", total_stake)?,
        )
    }

    /// Computes the exact floor-rounded reward from cumulative cursor values.
    /// Principal and maximum stake are checked uint256 domain inputs.
    pub fn reward_from_cursor(
        &self,
        cursor_rps: &BigUint,
        current_rps: BigUint,
        principal: &[u8],
        max_stake: &[u8],
    ) -> Result<BigUint, DposRewardGraphError> {
        self.ensure_complete()?;
        reward_from_cursor_principal(
            &current_rps,
            cursor_rps,
            decode_u256_like_bytes("principal", principal)?,
            decode_u256_like_bytes("max_stake", max_stake)?,
        )
    }

    /// Returns the low 256 bits as an ABI word, the model's sole modulo boundary.
    pub fn reward_to_u256_abi(value: &BigUint) -> [u8; 32] {
        let bytes = value.to_bytes_be();
        let mut out = [0_u8; 32];
        let take = bytes.len().min(32);
        out[32 - take..].copy_from_slice(&bytes[bytes.len() - take..]);
        out
    }

    fn increment_count(&mut self, node: &NodeKey) -> Result<(), DposRewardGraphError> {
        let node_key = *node;
        let node = self
            .nodes
            .get_mut(node)
            .ok_or(DposRewardGraphError::MissingNode {
                validator: node_key.validator,
                block: node_key.block,
            })?;
        node.count = node
            .count
            .checked_add(1)
            .ok_or(DposRewardGraphError::CountOverflow {
                validator: node_key.validator,
                block: node_key.block,
            })?;
        Ok(())
    }

    fn decrement_count(&mut self, node: &NodeKey) -> Result<(), DposRewardGraphError> {
        let node_key = *node;
        let next_count = {
            let node = self
                .nodes
                .get_mut(node)
                .ok_or(DposRewardGraphError::MissingNode {
                    validator: node_key.validator,
                    block: node_key.block,
                })?;
            let next_count =
                node.count
                    .checked_sub(1)
                    .ok_or(DposRewardGraphError::CountUnderflow {
                        validator: node_key.validator,
                        block: node_key.block,
                    })?;
            node.count = next_count;
            next_count
        };
        if next_count == 0 {
            self.nodes.remove(&node_key);
        }
        Ok(())
    }

    fn validate_count_contract(&self) -> Result<(), DposRewardGraphError> {
        if !self.history_complete {
            return Ok(());
        }

        let mut required: std::collections::BTreeMap<NodeKey, u32> = BTreeMap::new();
        for (cursor_key, block) in &self.delegation_cursors {
            let node_key = NodeKey {
                validator: cursor_key.0,
                block: *block,
            };
            if !self.nodes.contains_key(&node_key) {
                return Err(DposRewardGraphError::DanglingCursor {
                    validator: cursor_key.0,
                    delegator: cursor_key.1,
                    block: *block,
                });
            }
            let entry = required.entry(node_key).or_insert(0);
            *entry = entry
                .checked_add(1)
                .ok_or(DposRewardGraphError::CountOverflow {
                    validator: cursor_key.0,
                    block: *block,
                })?;
        }

        for (validator, block) in &self.validator_heads {
            let node_key = NodeKey {
                validator: *validator,
                block: *block,
            };
            if self.stale_validator_heads.contains(validator) {
                continue;
            }
            if !self.nodes.contains_key(&node_key) {
                return Err(DposRewardGraphError::DanglingHead {
                    validator: *validator,
                    block: *block,
                });
            }
            let entry = required.entry(node_key).or_insert(0);
            *entry = entry
                .checked_add(1)
                .ok_or(DposRewardGraphError::CountOverflow {
                    validator: *validator,
                    block: *block,
                })?;
        }

        for (node_key, node) in &self.nodes {
            let required_count = required.get(node_key).copied().unwrap_or(0);
            if node.count == 0 || node.count < required_count {
                return Err(DposRewardGraphError::CountMismatch {
                    validator: node_key.validator,
                    block: node_key.block,
                });
            }
        }

        for validator in self.stale_validator_heads.iter() {
            self.validator_heads.get(validator).copied().ok_or(
                DposRewardGraphError::MissingStaleHead {
                    validator: *validator,
                },
            )?;
        }
        Ok(())
    }
}

fn decode_u256_like_bytes(
    field: &'static str,
    raw: &[u8],
) -> Result<BigUint, DposRewardGraphError> {
    if raw.len() > MAX_U256_BYTES {
        return Err(DposRewardGraphError::AmountOverU256 { field });
    }
    if raw.len() == 1 && raw[0] == 0 {
        return Err(DposRewardGraphError::InvalidRlp(format!(
            "{field} uses non-canonical zero encoding"
        )));
    }
    Ok(BigUint::from_bytes_be(raw))
}

fn decode_u64(raw: &[u8], field: &'static str) -> Result<u64, DposRewardGraphError> {
    if raw.is_empty() {
        return Ok(0);
    }
    if raw[0] == 0_u8 {
        return Err(DposRewardGraphError::InvalidRlp(format!(
            "{field} uses non-canonical uint encoding"
        )));
    }
    if raw.len() > 8 {
        return Err(DposRewardGraphError::InvalidRlp(format!(
            "{field} out of u64 bounds"
        )));
    }
    let mut bytes = [0_u8; 8];
    bytes[8 - raw.len()..].copy_from_slice(raw);
    Ok(u64::from_be_bytes(bytes))
}

fn decode_address(raw: &[u8], field: &'static str) -> Result<[u8; 20], DposRewardGraphError> {
    if raw.len() != 20 {
        return Err(DposRewardGraphError::InvalidAddress(field));
    }
    let mut out = [0_u8; 20];
    out.copy_from_slice(raw);
    Ok(out)
}

fn decode_u64_vec(raw: &[u8], field: &'static str) -> Result<u64, DposRewardGraphError> {
    decode_u64(raw, field)
}

fn decode_count(raw: &[u8], field: &'static str) -> Result<u32, DposRewardGraphError> {
    let value = decode_u64(raw, field)?;
    u32::try_from(value)
        .map_err(|_| DposRewardGraphError::InvalidRlp(format!("{field} exceeds u32")))
}

fn decode_node_reward(raw: &[u8], field: &'static str) -> Result<BigUint, DposRewardGraphError> {
    if (raw.len() > 1 && raw[0] == 0_u8) || (raw.len() == 1 && raw[0] == 0_u8) {
        return Err(DposRewardGraphError::InvalidRlp(format!(
            "{field} uses non-canonical bigint bytes"
        )));
    }
    Ok(BigUint::from_bytes_be(raw))
}

fn encode_node_reward(value: &BigUint) -> Vec<u8> {
    if value == &BigUint::from(0_u8) {
        return Vec::new();
    }
    value.to_bytes_be()
}

/// Encode the graph into canonical RLP.
///
/// - item 0: schema version
/// - item 1: nodes as `[[validator,block,reward_per_stake,count], ...]`
/// - item 2: validator heads as `[[validator,head], ...]`
/// - item 3: delegation cursors as `[[validator,delegator,block], ...]`
/// - item 4: stale validator head markers as `[validator, ...]`
/// - item 5: history complete flag
/// - item 6: current block
pub fn encode_dpos_reward_graph(graph: &DposRewardGraph) -> Result<Vec<u8>, DposRewardGraphError> {
    graph.validate_count_contract()?;

    let mut nodes = RlpStream::new_list(graph.nodes.len());
    for (key, node) in &graph.nodes {
        let mut entry = RlpStream::new_list(4);
        entry.append(&key.validator.to_vec());
        entry.append(&key.block);
        entry.append(&encode_node_reward(&node.reward_per_stake));
        entry.append(&node.count);
        nodes.append_raw(&entry.out(), 1);
    }

    let mut heads = RlpStream::new_list(graph.validator_heads.len());
    for (validator, head) in &graph.validator_heads {
        let mut entry = RlpStream::new_list(2);
        entry.append(&validator.to_vec());
        entry.append(head);
        heads.append_raw(&entry.out(), 1);
    }

    let mut cursors = RlpStream::new_list(graph.delegation_cursors.len());
    for ((validator, delegator), head) in &graph.delegation_cursors {
        let mut entry = RlpStream::new_list(3);
        entry.append(&validator.to_vec());
        entry.append(&delegator.to_vec());
        entry.append(head);
        cursors.append_raw(&entry.out(), 1);
    }

    let mut stale = RlpStream::new_list(graph.stale_validator_heads.len());
    for validator in &graph.stale_validator_heads {
        stale.append(&validator.to_vec());
    }

    let mut stream = RlpStream::new_list(7);
    stream.append(&SCHEMA_VERSION);
    stream.append_raw(&nodes.out(), 1);
    stream.append_raw(&heads.out(), 1);
    stream.append_raw(&cursors.out(), 1);
    stream.append_raw(&stale.out(), 1);
    stream.append(&graph.history_complete);
    stream.append(&graph.current_block);
    Ok(stream.out().to_vec())
}

/// Decodes one canonical, seven-field reward-graph RLP payload.
/// Trailing bytes, non-list tables, unsorted/duplicate rows, noncanonical
/// integers, undercounted references, and unexpected dangling references fail.
pub fn decode_dpos_reward_graph(encoded: &[u8]) -> Result<DposRewardGraph, DposRewardGraphError> {
    let raw = Rlp::new(encoded);
    let payload = raw
        .payload_info()
        .map_err(decoder_error("top-level payload"))?;
    if !raw.is_list() || payload.header_len + payload.value_len != encoded.len() {
        return Err(DposRewardGraphError::InvalidRlp(
            "reward-graph must be one canonical top-level list".to_string(),
        ));
    }
    let item_count = raw
        .item_count()
        .map_err(decoder_error("top-level item count"))?;
    if item_count != 7 {
        return Err(DposRewardGraphError::InvalidRlp(
            "reward-graph requires 7 fields".to_string(),
        ));
    }

    let version: u8 = raw.val_at(0).map_err(decoder_error("schema version"))?;
    if version != SCHEMA_VERSION {
        return Err(DposRewardGraphError::UnsupportedSchema(version));
    }
    for (index, field) in [
        "nodes",
        "validator_heads",
        "delegation_cursors",
        "stale_heads",
    ]
    .into_iter()
    .enumerate()
    {
        if !raw.at(index + 1).map_err(decoder_error(field))?.is_list() {
            return Err(DposRewardGraphError::InvalidRlp(format!(
                "{field} must be an RLP list"
            )));
        }
    }

    let mut graph = DposRewardGraph::new();
    graph.history_complete = raw.val_at(5).map_err(decoder_error("history_complete"))?;
    graph.current_block = raw.val_at(6).map_err(decoder_error("current_block"))?;

    let node_list = raw.at(1).map_err(decoder_error("nodes"))?;
    let mut previous_node = None;
    for item in node_list.iter() {
        if item
            .item_count()
            .map_err(decoder_error("node.item_count"))?
            != 4
        {
            return Err(DposRewardGraphError::InvalidRlp(
                "node item must be [validator, block, reward_per_stake, count]".to_string(),
            ));
        }

        let validator = decode_address(
            item.at(0)
                .map_err(decoder_error("node.validator"))?
                .data()
                .map_err(decoder_error("node.validator"))?,
            "node.validator",
        )?;
        let block = decode_u64_vec(
            item.at(1)
                .map_err(decoder_error("node.block"))?
                .data()
                .map_err(decoder_error("node.block"))?,
            "node.block",
        )?;
        let reward_per_stake = decode_node_reward(
            item.at(2)
                .map_err(decoder_error("node.reward_per_stake"))?
                .data()
                .map_err(decoder_error("node.reward_per_stake"))?,
            "node.reward_per_stake",
        )?;
        let count = decode_count(
            item.at(3)
                .map_err(decoder_error("node.count"))?
                .data()
                .map_err(decoder_error("node.count"))?,
            "node.count",
        )?;
        let key = NodeKey { validator, block };
        if previous_node.is_some_and(|previous| previous > key) {
            return Err(DposRewardGraphError::InvalidRlp(
                "reward nodes must be strictly sorted".to_string(),
            ));
        }
        previous_node = Some(key);
        if graph
            .nodes
            .insert(
                key,
                Node {
                    reward_per_stake,
                    count,
                },
            )
            .is_some()
        {
            return Err(DposRewardGraphError::DuplicateNode { validator, block });
        }
    }

    let validator_heads = raw.at(2).map_err(decoder_error("validator_heads"))?;
    let mut previous_head = None;
    for item in validator_heads.iter() {
        if item
            .item_count()
            .map_err(decoder_error("validator_head.item_count"))?
            != 2
        {
            return Err(DposRewardGraphError::InvalidRlp(
                "validator head item must be [validator, head]".to_string(),
            ));
        }
        let validator = decode_address(
            item.at(0)
                .map_err(decoder_error("validator_head.validator"))?
                .data()
                .map_err(decoder_error("validator_head.validator"))?,
            "validator_head.validator",
        )?;
        let block = decode_u64_vec(
            item.at(1)
                .map_err(decoder_error("validator_head.block"))?
                .data()
                .map_err(decoder_error("validator_head.block"))?,
            "validator_head.block",
        )?;
        if previous_head.is_some_and(|previous| previous > validator) {
            return Err(DposRewardGraphError::InvalidRlp(
                "validator heads must be strictly sorted".to_string(),
            ));
        }
        previous_head = Some(validator);
        if graph.validator_heads.insert(validator, block).is_some() {
            return Err(DposRewardGraphError::DuplicateHead { validator });
        }
    }

    let delegation_cursors = raw.at(3).map_err(decoder_error("delegation_cursors"))?;
    let mut previous_cursor = None;
    for item in delegation_cursors.iter() {
        if item
            .item_count()
            .map_err(decoder_error("delegation_cursor.item_count"))?
            != 3
        {
            return Err(DposRewardGraphError::InvalidRlp(
                "delegation cursor item must be [validator, delegator, block]".to_string(),
            ));
        }
        let validator = decode_address(
            item.at(0)
                .map_err(decoder_error("delegation_cursor.validator"))?
                .data()
                .map_err(decoder_error("delegation_cursor.validator"))?,
            "delegation_cursor.validator",
        )?;
        let delegator = decode_address(
            item.at(1)
                .map_err(decoder_error("delegation_cursor.delegator"))?
                .data()
                .map_err(decoder_error("delegation_cursor.delegator"))?,
            "delegation_cursor.delegator",
        )?;
        let block = decode_u64_vec(
            item.at(2)
                .map_err(decoder_error("delegation_cursor.block"))?
                .data()
                .map_err(decoder_error("delegation_cursor.block"))?,
            "delegation_cursor.block",
        )?;
        let cursor_key = (validator, delegator);
        if previous_cursor.is_some_and(|previous| previous > cursor_key) {
            return Err(DposRewardGraphError::InvalidRlp(
                "delegation cursors must be strictly sorted".to_string(),
            ));
        }
        previous_cursor = Some(cursor_key);
        if graph.delegation_cursors.insert(cursor_key, block).is_some() {
            return Err(DposRewardGraphError::DuplicateCursor {
                validator,
                delegator,
            });
        }
    }

    let stale_heads = raw.at(4).map_err(decoder_error("stale_heads"))?;
    let mut previous_stale = None;
    for item in stale_heads.iter() {
        let validator_bytes = item.data().map_err(decoder_error("stale.head"))?;
        if validator_bytes.len() != 20 {
            return Err(DposRewardGraphError::InvalidAddress("stale.head"));
        }
        let validator = decode_address(validator_bytes, "stale.head")?;
        if previous_stale.is_some_and(|previous| previous > validator) {
            return Err(DposRewardGraphError::InvalidRlp(
                "stale heads must be strictly sorted".to_string(),
            ));
        }
        previous_stale = Some(validator);
        if !graph.stale_validator_heads.insert(validator) {
            return Err(DposRewardGraphError::DuplicateStaleHead { validator });
        }
    }

    graph.validate_count_contract()?;
    Ok(graph)
}

/// Computes `current_rps = head_rps + floor(pool * max_stake / total_stake)`
/// with arbitrary-width unsigned arithmetic. A zero denominator fails.
pub fn reward_per_stake(
    head_rps: &BigUint,
    pool: BigUint,
    max_stake: BigUint,
    total_stake: BigUint,
) -> Result<BigUint, DposRewardGraphError> {
    if total_stake == BigUint::from(0_u8) {
        return Err(DposRewardGraphError::ZeroDenominator);
    }
    Ok(head_rps + (pool * max_stake / total_stake))
}

fn decoder_error(context: &'static str) -> impl FnOnce(DecoderError) -> DposRewardGraphError {
    move |err| DposRewardGraphError::InvalidRlp(format!("{context}: {err}"))
}

/// Computes `floor((current_rps - cursor_rps) * principal / max_stake)` exactly.
/// A regressed cursor fails; zero maximum stake returns zero for legacy parity.
pub fn reward_from_cursor_principal(
    current_rps: &BigUint,
    cursor_rps: &BigUint,
    principal: BigUint,
    max_stake: BigUint,
) -> Result<BigUint, DposRewardGraphError> {
    if current_rps < cursor_rps {
        return Err(DposRewardGraphError::ArithmeticUnderflow {
            left: "current_rps",
            right: "cursor_rps",
        });
    }
    if max_stake == BigUint::from(0_u8) {
        return Ok(BigUint::from(0_u8));
    }
    Ok((current_rps - cursor_rps) * principal / max_stake)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(byte: u8) -> [u8; 20] {
        [byte; 20]
    }

    #[test]
    fn deterministic_codec_roundtrip() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(1),
                block: 10,
            },
            Node {
                reward_per_stake: BigUint::from(11_u64),
                count: 3,
            },
        );
        graph.nodes.insert(
            NodeKey {
                validator: addr(2),
                block: 15,
            },
            Node {
                reward_per_stake: BigUint::from(7_u64),
                count: 2,
            },
        );
        graph.validator_heads.insert(addr(1), 10);
        graph.validator_heads.insert(addr(2), 15);
        graph.delegation_cursors.insert((addr(1), addr(3)), 10);
        graph.delegation_cursors.insert((addr(1), addr(4)), 10);
        graph.current_block = 123;

        let encoded = encode_dpos_reward_graph(&graph).unwrap();
        let decoded = decode_dpos_reward_graph(&encoded).unwrap();
        assert_eq!(graph, decoded);

        let encoded_again = encode_dpos_reward_graph(&decoded).unwrap();
        assert_eq!(encoded, encoded_again);
    }

    #[test]
    fn rejects_legacy_schema_that_cannot_be_deterministically_repaired() {
        let mut stream = RlpStream::new_list(4);
        stream.append(&SCHEMA_VERSION);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        let legacy = stream.out().to_vec();
        assert!(decode_dpos_reward_graph(&legacy).is_err());
    }

    #[test]
    fn rejects_malformed_noncanonical_and_duplicates_and_dangling() {
        let mut stream = RlpStream::new_list(7);
        let mut nodes = RlpStream::new_list(1);
        let mut bad_node = RlpStream::new_list(3);
        bad_node.append(&addr(1).to_vec());
        bad_node.append(&1_u64);
        bad_node.append(&Vec::<u8>::new());
        nodes.append_raw(&bad_node.out(), 1);
        stream.append(&0xff_u8);
        stream.append_raw(&nodes.out(), 1);
        let empty = RlpStream::new_list(0).out().to_vec();
        stream.append_raw(&empty, 1);
        stream.append_raw(&empty, 1);
        stream.append_raw(&empty, 1);
        stream.append(&false);
        stream.append(&0_u8);
        assert!(decode_dpos_reward_graph(&stream.out()).is_err());

        let mut duplicate = RlpStream::new_list(7);
        let mut nodes = RlpStream::new_list(2);
        let mut node = RlpStream::new_list(4);
        node.append(&addr(2).to_vec());
        node.append(&1_u64);
        node.append(&Vec::<u8>::new());
        node.append(&Vec::<u8>::new());
        let node = node.out().to_vec();
        nodes.append_raw(&node, 1);
        nodes.append_raw(&node, 1);
        duplicate.append(&SCHEMA_VERSION);
        duplicate.append_raw(&nodes.out(), 1);
        duplicate.append_raw(&empty, 1);
        duplicate.append_raw(&empty, 1);
        duplicate.append_raw(&empty, 1);
        duplicate.append(&true);
        duplicate.append(&0_u8);
        let duplicate_err = decode_dpos_reward_graph(&duplicate.out()).unwrap_err();
        assert!(
            matches!(duplicate_err, DposRewardGraphError::DuplicateNode { .. }),
            "{:?}",
            duplicate_err
        );

        let mut dangling = RlpStream::new_list(7);
        let nodes = RlpStream::new_list(0);
        let mut heads = RlpStream::new_list(1);
        let mut head = RlpStream::new_list(2);
        head.append(&addr(3).to_vec());
        head.append(&9_u64);
        heads.append_raw(&head.out(), 1);
        dangling.append(&SCHEMA_VERSION);
        dangling.append_raw(&nodes.out(), 1);
        dangling.append_raw(&heads.out(), 1);
        dangling.append_raw(&empty, 1);
        dangling.append_raw(&empty, 1);
        dangling.append(&true);
        dangling.append(&0_u8);
        assert!(matches!(
            decode_dpos_reward_graph(&dangling.out()).unwrap_err(),
            DposRewardGraphError::DanglingHead { .. }
        ));
    }

    #[test]
    fn rejects_noncanonical_zero_encodings_and_schema_shape() {
        let mut stream = RlpStream::new_list(7);
        let mut nodes = RlpStream::new_list(1);
        let mut node = RlpStream::new_list(4);
        node.append(&addr(1).to_vec());
        node.append(&1_u64);
        node.append(&vec![0_u8]);
        node.append(&0_u8);
        nodes.append_raw(&node.out(), 1);
        stream.append(&SCHEMA_VERSION);
        stream.append_raw(&nodes.out(), 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append_raw(&[0xC0], 1);
        stream.append(&false);
        stream.append(&0_u8);
        assert!(matches!(
            decode_dpos_reward_graph(&stream.out()).unwrap_err(),
            DposRewardGraphError::InvalidRlp(msg) if msg.contains("non-canonical")
        ));

        let mut trailing = encode_dpos_reward_graph(&DposRewardGraph::new()).unwrap();
        trailing.push(0x80);
        assert!(matches!(
            decode_dpos_reward_graph(&trailing),
            Err(DposRewardGraphError::InvalidRlp(_))
        ));

        let mut non_list = RlpStream::new_list(7);
        non_list.append(&SCHEMA_VERSION);
        non_list.append(&Vec::<u8>::new());
        non_list.append_raw(&[0xC0], 1);
        non_list.append_raw(&[0xC0], 1);
        non_list.append_raw(&[0xC0], 1);
        non_list.append(&false);
        non_list.append(&0_u8);
        assert!(matches!(
            decode_dpos_reward_graph(&non_list.out()),
            Err(DposRewardGraphError::InvalidRlp(msg)) if msg.contains("must be an RLP list")
        ));

        let mut unsorted = RlpStream::new_list(7);
        let mut nodes = RlpStream::new_list(2);
        for validator in [addr(2), addr(1)] {
            let mut node = RlpStream::new_list(4);
            node.append(&validator.to_vec());
            node.append(&1_u64);
            node.append(&Vec::<u8>::new());
            node.append(&1_u32);
            nodes.append_raw(&node.out(), 1);
        }
        unsorted.append(&SCHEMA_VERSION);
        unsorted.append_raw(&nodes.out(), 1);
        unsorted.append_raw(&[0xC0], 1);
        unsorted.append_raw(&[0xC0], 1);
        unsorted.append_raw(&[0xC0], 1);
        unsorted.append(&false);
        unsorted.append(&0_u8);
        assert!(matches!(
            decode_dpos_reward_graph(&unsorted.out()),
            Err(DposRewardGraphError::InvalidRlp(msg)) if msg.contains("strictly sorted")
        ));
    }

    #[test]
    fn stale_head_and_rebind_contract_is_enforced() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(20),
                block: 1,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 0,
            },
        );
        graph.nodes.insert(
            NodeKey {
                validator: addr(20),
                block: 2,
            },
            Node {
                reward_per_stake: BigUint::from(7_u8),
                // The buggy same-validator redelegation transcript already
                // counted both the delegation and logical validator refs.
                count: 2,
            },
        );
        graph.delegation_cursors.insert((addr(20), addr(21)), 2);
        graph.attach_validator_head(addr(20), 1).unwrap();
        graph.detach_validator_head(&addr(20)).unwrap();
        assert!(graph.is_stale_head(&addr(20)).unwrap());
        assert_eq!(graph.validator_heads.get(&addr(20)).copied(), Some(1));
        assert!(!graph.nodes.contains_key(&NodeKey {
            validator: addr(20),
            block: 1,
        }));
        let stale_roundtrip =
            decode_dpos_reward_graph(&encode_dpos_reward_graph(&graph).unwrap()).unwrap();
        assert_eq!(stale_roundtrip, graph);

        graph
            .rebind_stale_validator_head(addr(20), addr(21), 2)
            .unwrap();
        assert!(!graph.is_stale_head(&addr(20)).unwrap());
        assert_eq!(graph.validator_heads.get(&addr(20)).copied(), Some(2));
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(20),
                    block: 2,
                })
                .unwrap()
                .count,
            2
        );

        let mut conflict = DposRewardGraph::new();
        conflict.nodes.insert(
            NodeKey {
                validator: addr(21),
                block: 2,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 1,
            },
        );
        conflict.stale_validator_heads.insert(addr(21));
        conflict.validator_heads.insert(addr(21), 2);
        let encoded_conflict = encode_dpos_reward_graph(&conflict).unwrap();
        assert_eq!(
            decode_dpos_reward_graph(&encoded_conflict).unwrap(),
            conflict
        );
        assert!(matches!(
            conflict.rebind_stale_validator_head(addr(21), addr(22), 2),
            Err(DposRewardGraphError::StaleHeadConflict { .. })
        ));
        conflict.delegation_cursors.insert((addr(21), addr(22)), 2);
        assert!(matches!(
            conflict.rebind_stale_validator_head(addr(21), addr(22), 2),
            Err(DposRewardGraphError::MissingStaleHead { .. })
        ));
        assert!(conflict.validator_heads.remove(&addr(21)).is_some());
        assert!(matches!(
            encode_dpos_reward_graph(&conflict),
            Err(DposRewardGraphError::MissingStaleHead { .. })
        ));
        assert!(matches!(
            graph.rebind_stale_validator_head(addr(20), addr(21), 2),
            Err(DposRewardGraphError::MissingStaleHead { .. })
        ));
    }

    #[test]
    fn count_validation_rolls_back_on_overflow_without_mutating_state() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(30),
                block: 1,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: u32::MAX,
            },
        );

        assert!(matches!(
            graph.write_cursor(addr(30), addr(31), 1),
            Err(DposRewardGraphError::CountOverflow { .. })
        ));
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(30),
                    block: 1
                })
                .unwrap()
                .count,
            u32::MAX
        );
        assert!(matches!(
            graph.read_cursor(&addr(30), &addr(31)),
            Err(DposRewardGraphError::MissingCursor { .. })
        ));
    }

    #[test]
    fn decrement_count_removes_node_when_zero() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(40),
                block: 3,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 0,
            },
        );

        graph.attach_validator_head(addr(40), 3).unwrap();
        assert!(graph.nodes.contains_key(&NodeKey {
            validator: addr(40),
            block: 3
        }));
        graph.detach_validator_head(&addr(40)).unwrap();
        assert!(!graph.nodes.contains_key(&NodeKey {
            validator: addr(40),
            block: 3
        }));
    }

    #[test]
    fn loaded_node_restore_reproduces_repeated_full_redelegation_count() {
        let validator = addr(41);
        let delegator = addr(42);
        let key = NodeKey {
            validator,
            block: 7,
        };
        let mut graph = DposRewardGraph::new();
        graph
            .bootstrap_node(
                key,
                Node {
                    reward_per_stake: BigUint::from(99_u8),
                    count: 2,
                },
                true,
                &[delegator],
            )
            .unwrap();
        let loaded = graph.load_node(&key).unwrap();

        graph.delete_cursor(&validator, &delegator).unwrap();
        assert_eq!(graph.load_node(&key).unwrap().count, 1);
        graph.restore_loaded_node(key, loaded).unwrap();
        graph
            .write_or_create_cursor(validator, delegator, key.block)
            .unwrap();

        assert_eq!(graph.load_node(&key).unwrap().count, 3);
        assert_eq!(graph.read_cursor(&validator, &delegator).unwrap(), 7);
    }

    #[test]
    fn bootstrap_live_head_keeps_stale_node_for_other_cursor() {
        let validator = addr(45);
        let owner = addr(46);
        let redelegator = addr(47);
        let mut graph = DposRewardGraph::new();
        graph
            .bootstrap_node(
                NodeKey {
                    validator,
                    block: 0,
                },
                Node {
                    reward_per_stake: BigUint::from(1_u8),
                    count: 3,
                },
                true,
                &[owner, redelegator],
            )
            .unwrap();
        graph
            .bootstrap_node(
                NodeKey {
                    validator,
                    block: 1,
                },
                Node {
                    reward_per_stake: BigUint::from(2_u8),
                    count: 2,
                },
                true,
                &[redelegator],
            )
            .unwrap();
        graph.stale_validator_heads.insert(validator);
        graph.validator_heads.insert(validator, 0);
        graph
            .nodes
            .get_mut(&NodeKey {
                validator,
                block: 0,
            })
            .unwrap()
            .count = 1;
        graph
            .nodes
            .get_mut(&NodeKey {
                validator,
                block: 1,
            })
            .unwrap()
            .count = 1;
        graph.delegation_cursors.insert((validator, owner), 0);
        graph.delegation_cursors.insert((validator, redelegator), 1);

        graph
            .bootstrap_node(
                NodeKey {
                    validator,
                    block: 2,
                },
                Node {
                    reward_per_stake: BigUint::from(3_u8),
                    count: 2,
                },
                true,
                &[redelegator],
            )
            .unwrap();

        assert!(graph.nodes.contains_key(&NodeKey {
            validator,
            block: 0
        }));
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator,
                    block: 0,
                })
                .unwrap()
                .count,
            1
        );
        assert!(!graph.nodes.contains_key(&NodeKey {
            validator,
            block: 1,
        }));
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator,
                    block: 2,
                })
                .unwrap()
                .count,
            2
        );
        assert_eq!(graph.read_cursor(&validator, &owner).unwrap(), 0);
        assert_eq!(graph.read_cursor(&validator, &redelegator).unwrap(), 2);
        assert_eq!(graph.read_validator_head(&validator).unwrap(), 2);
        assert!(!graph.is_stale_head(&validator).unwrap());
    }

    #[test]
    fn normal_and_force_validator_deletion_preserve_distinct_legacy_results() {
        let normal_validator = addr(43);
        let force_validator = addr(44);
        let mut graph = DposRewardGraph::new();
        for validator in [normal_validator, force_validator] {
            graph
                .bootstrap_node(
                    NodeKey {
                        validator,
                        block: 8,
                    },
                    Node {
                        reward_per_stake: BigUint::from(1_u8),
                        count: 2,
                    },
                    true,
                    &[],
                )
                .unwrap();
        }

        graph.delete_validator_head(&normal_validator).unwrap();
        assert_eq!(
            graph
                .load_node(&NodeKey {
                    validator: normal_validator,
                    block: 8,
                })
                .unwrap()
                .count,
            1
        );
        graph.force_delete_validator_head(&force_validator).unwrap();
        assert!(matches!(
            graph.load_node(&NodeKey {
                validator: force_validator,
                block: 8,
            }),
            Err(DposRewardGraphError::MissingNode { .. })
        ));
    }

    #[test]
    fn registration_reinitializes_same_block_orphan_but_rejects_live_references() {
        let validator = addr(45);
        let delegator = addr(46);
        let key = NodeKey {
            validator,
            block: 9,
        };
        let mut graph = DposRewardGraph::new();
        graph
            .bootstrap_node(
                key,
                Node {
                    reward_per_stake: BigUint::from(77_u8),
                    count: 2,
                },
                true,
                &[],
            )
            .unwrap();
        graph.delete_validator_head(&validator).unwrap();
        assert_eq!(graph.load_node(&key).unwrap().count, 1);

        graph
            .register_validator(
                key,
                Node {
                    reward_per_stake: BigUint::from(0_u8),
                    count: 2,
                },
                &[delegator],
            )
            .unwrap();
        assert_eq!(
            graph.load_node(&key).unwrap().reward_per_stake,
            BigUint::from(0_u8)
        );
        assert_eq!(graph.load_node(&key).unwrap().count, 2);
        assert_eq!(graph.read_validator_head(&validator).unwrap(), 9);
        assert_eq!(graph.read_cursor(&validator, &delegator).unwrap(), 9);

        let before = graph.clone();
        assert!(matches!(
            graph.register_validator(
                key,
                Node {
                    reward_per_stake: BigUint::from(1_u8),
                    count: 1
                },
                &[]
            ),
            Err(DposRewardGraphError::DuplicateHead { .. })
        ));
        assert_eq!(graph, before);
    }

    #[test]
    fn genesis_graph_encodes_and_decodes() {
        let graph = DposRewardGraph::new();
        let encoded = encode_dpos_reward_graph(&graph).unwrap();
        let decoded = decode_dpos_reward_graph(&encoded).unwrap();
        assert_eq!(decoded, graph);
        assert_eq!(decoded.current_block().unwrap(), 0);
    }

    #[test]
    fn public_bootstrap_inserts_node_and_references_atomically() {
        let mut graph = DposRewardGraph::new();
        let key = NodeKey {
            validator: addr(40),
            block: 7,
        };
        graph
            .bootstrap_node(
                key,
                Node {
                    reward_per_stake: BigUint::from(9_u8),
                    count: 4,
                },
                true,
                &[addr(41), addr(42)],
            )
            .unwrap();
        assert_eq!(graph.validator_heads.get(&addr(40)), Some(&7));
        assert_eq!(graph.read_validator_head(&addr(40)).unwrap(), 7);
        assert_eq!(
            graph.delegation_cursors.get(&(addr(40), addr(41))),
            Some(&7)
        );
        assert_eq!(graph.nodes.get(&key).unwrap().count, 4);

        let next_key = NodeKey {
            validator: addr(40),
            block: 8,
        };
        graph
            .bootstrap_node(
                next_key,
                Node {
                    reward_per_stake: BigUint::from(10_u8),
                    count: 2,
                },
                true,
                &[addr(41)],
            )
            .unwrap();
        assert_eq!(graph.read_validator_head(&addr(40)).unwrap(), 8);
        assert_eq!(graph.read_cursor(&addr(40), &addr(41)).unwrap(), 8);
        assert_eq!(graph.read_cursor(&addr(40), &addr(42)).unwrap(), 7);
        assert_eq!(graph.nodes.get(&key).unwrap().count, 2);
        assert_eq!(graph.nodes.get(&next_key).unwrap().count, 2);

        let before = graph.clone();
        assert!(matches!(
            graph.bootstrap_node(
                key,
                Node {
                    reward_per_stake: BigUint::from(10_u8),
                    count: 1,
                },
                false,
                &[],
            ),
            Err(DposRewardGraphError::DuplicateNode { .. })
        ));
        assert_eq!(graph, before);
        assert!(matches!(
            graph.read_validator_head(&addr(99)),
            Err(DposRewardGraphError::MissingHead { .. })
        ));
    }

    #[test]
    fn attach_detach_delete_paths() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(9),
                block: 2,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 0,
            },
        );
        graph.attach_validator_head(addr(9), 2).unwrap();
        graph.write_cursor(addr(9), addr(1), 2).unwrap();
        assert_eq!(graph.read_cursor(&addr(9), &addr(1)).unwrap(), 2);
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(9),
                    block: 2
                })
                .unwrap()
                .count,
            2
        );

        graph.delete_cursor(&addr(9), &addr(1)).unwrap();
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(9),
                    block: 2
                })
                .unwrap()
                .count,
            1
        );

        graph.detach_validator_head(&addr(9)).unwrap();
        assert!(graph.is_stale_head(&addr(9)).unwrap());
        assert_eq!(graph.validator_heads.get(&addr(9)).copied(), Some(2));
    }

    #[test]
    fn repeated_same_block_claims_inflate_count() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(6),
                block: 4,
            },
            Node {
                reward_per_stake: BigUint::from(2_u8),
                count: 0,
            },
        );
        graph.attach_validator_head(addr(6), 4).unwrap();
        graph.write_cursor(addr(6), addr(2), 4).unwrap();
        graph.write_cursor(addr(6), addr(2), 4).unwrap();
        graph.write_cursor(addr(6), addr(2), 4).unwrap();
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(6),
                    block: 4
                })
                .unwrap()
                .count,
            4
        );
    }

    #[test]
    fn next_block_checkpoint_advances_without_recompute() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(8),
                block: 5,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 1,
            },
        );
        graph.nodes.insert(
            NodeKey {
                validator: addr(8),
                block: 6,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                // Positive orphan counts are consensus-visible and must not be
                // recomputed merely because no live reference points here.
                count: 1,
            },
        );
        graph.write_cursor(addr(8), addr(1), 5).unwrap();
        graph.next_block(10).unwrap();
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(8),
                    block: 5
                })
                .unwrap()
                .count,
            2
        );
        assert_eq!(graph.current_block().unwrap(), 10);
        assert!(matches!(
            graph.next_block(9).unwrap_err(),
            DposRewardGraphError::CheckpointRegression { .. }
        ));
    }

    #[test]
    fn count_under_and_overflow() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(10),
                block: 1,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: u32::MAX,
            },
        );

        assert!(matches!(
            graph.write_cursor(addr(10), addr(2), 1),
            Err(DposRewardGraphError::CountOverflow { .. })
        ));

        graph
            .nodes
            .get_mut(&NodeKey {
                validator: addr(10),
                block: 1,
            })
            .unwrap()
            .count = 0;
        assert!(graph.write_cursor(addr(10), addr(2), 1).is_ok());
        assert!(matches!(
            graph.delete_cursor(&addr(10), &addr(3)),
            Err(DposRewardGraphError::MissingCursor { .. })
        ));
    }

    #[test]
    fn repeated_same_block_claims_track_cursor_reuse_as_additive() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(11),
                block: 2,
            },
            Node {
                reward_per_stake: BigUint::from(5_u8),
                count: 0,
            },
        );
        graph.attach_validator_head(addr(11), 2).unwrap();
        graph.write_cursor(addr(11), addr(22), 2).unwrap();
        for _ in 0..4 {
            graph.write_cursor(addr(11), addr(22), 2).unwrap();
        }
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(11),
                    block: 2
                })
                .unwrap()
                .count,
            6
        );

        graph.delete_cursor(&addr(11), &addr(22)).unwrap();
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(11),
                    block: 2
                })
                .unwrap()
                .count,
            5
        );

        graph
            .write_cursor(addr(11), addr(22), 2)
            .unwrap_or_else(|_| panic!("recreate cursor"));
        assert_eq!(
            graph
                .nodes
                .get(&NodeKey {
                    validator: addr(11),
                    block: 2
                })
                .unwrap()
                .count,
            6
        );
    }

    #[test]
    fn incomplete_history_is_codec_only_and_rejects_authority() {
        let graph = DposRewardGraph::incomplete();
        let decoded = decode_dpos_reward_graph(&encode_dpos_reward_graph(&graph).unwrap()).unwrap();
        assert!(!decoded.history_complete);

        let mut decoded = decoded;
        assert!(matches!(
            decoded.bootstrap_node(
                NodeKey {
                    validator: addr(12),
                    block: 2,
                },
                Node {
                    reward_per_stake: BigUint::from(3_u8),
                    count: 1,
                },
                true,
                &[],
            ),
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        ));
        assert!(matches!(
            decoded.reward_per_stake(&BigUint::from(0_u8), &[], &[1], &[1]),
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        ));
        assert!(matches!(
            decoded.reward_from_cursor(&BigUint::from(0_u8), BigUint::from(1_u8), &[1], &[1]),
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        ));
        assert!(matches!(
            decoded.current_block(),
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        ));
        assert!(matches!(
            decoded.is_stale_head(&addr(12)),
            Err(DposRewardGraphError::GraphHistoryIncomplete)
        ));
    }

    #[test]
    fn explicit_count_and_current_block_roundtrip() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(13),
                block: 42,
            },
            Node {
                reward_per_stake: BigUint::from(4_u8),
                count: 7,
            },
        );
        graph.current_block = 9000;

        let encoded = encode_dpos_reward_graph(&graph).unwrap();
        let decoded = decode_dpos_reward_graph(&encoded).unwrap();
        assert_eq!(decoded.current_block().unwrap(), 9000);
        assert_eq!(
            decoded
                .nodes
                .get(&NodeKey {
                    validator: addr(13),
                    block: 42
                })
                .unwrap()
                .count,
            7
        );
    }

    #[test]
    fn large_rps_storage_is_preserved() {
        let mut graph = DposRewardGraph::new();
        let huge: BigUint = (BigUint::from(1_u8) << 300) + BigUint::from(17_u8);
        graph.nodes.insert(
            NodeKey {
                validator: addr(1),
                block: 1,
            },
            Node {
                reward_per_stake: huge.clone(),
                count: 1,
            },
        );
        graph.attach_validator_head(addr(1), 1).unwrap();
        let encoded = encode_dpos_reward_graph(&graph).unwrap();
        let decoded = decode_dpos_reward_graph(&encoded).unwrap();
        assert_eq!(
            decoded
                .nodes
                .get(&NodeKey {
                    validator: addr(1),
                    block: 1
                })
                .unwrap()
                .reward_per_stake,
            huge
        );
    }

    #[test]
    fn exact_floor_math_for_reward_steps() {
        let head_rps = BigUint::from(10_u8);
        let pool = BigUint::from(7_u8);
        let max_stake = BigUint::from(3_u8);
        let total_stake = BigUint::from(2_u8);
        let current = reward_per_stake(&head_rps, pool, max_stake, total_stake).unwrap();
        assert_eq!(current, BigUint::from(20_u8));

        let reward = reward_from_cursor_principal(
            &current,
            &BigUint::from(8_u8),
            BigUint::from(100_u8),
            BigUint::from(7_u8),
        )
        .unwrap();
        assert_eq!(reward, BigUint::from(171_u8));
    }

    #[test]
    fn abi_conversion_mod_2_to_256() {
        let value = (BigUint::from(1_u8) << 300) + BigUint::from(0x1234_u16);
        let out = DposRewardGraph::reward_to_u256_abi(&value);
        let bytes = value.to_bytes_be();
        let tail = &bytes[bytes.len() - 32..];
        assert_eq!(&out[..], tail);
    }

    #[test]
    fn reject_principal_over_u256() {
        let mut graph = DposRewardGraph::new();
        graph.nodes.insert(
            NodeKey {
                validator: addr(1),
                block: 1,
            },
            Node {
                reward_per_stake: BigUint::from(1_u8),
                count: 0,
            },
        );

        let over = vec![0xFF_u8; 33];
        assert!(
            graph
                .reward_per_stake(&BigUint::from(1_u8), &over, &[], &[])
                .is_err()
        );
        assert!(
            graph
                .reward_from_cursor(&BigUint::from(1_u8), BigUint::from(1_u8), &over, &[])
                .is_err()
        );
    }
}
