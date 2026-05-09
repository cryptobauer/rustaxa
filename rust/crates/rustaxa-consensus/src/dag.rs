use anyhow::{Context, Result, bail, ensure};
use ethereum_types::H256;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// Deterministic DAG frontier derived from a ghost path and DAG leaves.
///
/// Inputs:
/// - `pivot`: last hash in the ghost path (or zero hash when the path is empty).
/// - `tips`: leaf hashes excluding `pivot`.
///
/// Output invariants:
/// - `tips` never contains `pivot`.
/// - tip order is preserved from the input `leaves`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagFrontier {
    pub pivot: H256,
    pub tips: Vec<H256>,
}

/// Per-reference metadata used for pivot/tip level validation.
///
/// Inputs:
/// - `hash`: pivot/tip hash being validated.
/// - `found`: whether the reference block metadata exists.
/// - `level`: reference block level when `found == true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagReferenceMetadata {
    pub hash: H256,
    pub found: bool,
    pub level: u64,
}

/// Result of validating block level against pivot/tip metadata availability.
///
/// Output fields:
/// - `ok`: true only when there are no missing references and level matches.
/// - `expected_level`: max(parent-level + 1) across available pivot/tips.
/// - `level_matches`: whether `block_level == expected_level`.
/// - `missing_references`: missing pivot/tip hashes in deterministic order:
///   pivot first, then tips in provided order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagPivotTipsValidation {
    pub ok: bool,
    pub expected_level: u64,
    pub level_matches: bool,
    pub missing_references: Vec<H256>,
}

/// Maximum number of tips allowed on one DAG block.
///
/// This mirrors legacy `kDagBlockMaxTips` and is used by Rust verify prechecks
/// to preserve deterministic parity.
pub const DAG_BLOCK_MAX_TIPS: usize = 16;

/// Legacy C++ `DagManager::VerifyBlockReturnType::AheadBlock` value.
///
/// The Rust precheck returns legacy-compatible numeric codes because the CXX
/// bridge exposes plain structs, while the public C++ enum remains owned by the
/// existing DagManager API.
pub const DAG_VERIFY_REJECT_AHEAD_BLOCK: u32 = 2;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FailedVdfVerification` value.
pub const DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION: u32 = 3;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FutureBlock` value.
pub const DAG_VERIFY_REJECT_FUTURE_BLOCK: u32 = 4;

/// Legacy C++ `DagManager::VerifyBlockReturnType::NotEligible` value.
pub const DAG_VERIFY_REJECT_NOT_ELIGIBLE: u32 = 5;

/// Legacy C++ `DagManager::VerifyBlockReturnType::ExpiredBlock` value.
pub const DAG_VERIFY_REJECT_EXPIRED_BLOCK: u32 = 6;

/// Legacy C++ `DagManager::VerifyBlockReturnType::IncorrectTransactionsEstimation` value.
pub const DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION: u32 = 7;

/// Legacy C++ `DagManager::VerifyBlockReturnType::BlockTooBig` value.
pub const DAG_VERIFY_REJECT_BLOCK_TOO_BIG: u32 = 8;

/// Legacy C++ `DagManager::VerifyBlockReturnType::FailedTipsVerification` value.
pub const DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION: u32 = 9;

/// Legacy C++ `DagManager::VerifyBlockReturnType::MissingTip` value.
pub const DAG_VERIFY_REJECT_MISSING_TIP: u32 = 10;

/// Legacy C++ `DagManager::VerifyBlockReturnType::MissingTransaction` value.
pub const DAG_VERIFY_REJECT_MISSING_TRANSACTION: u32 = 1;

/// Rust DAG verification reason: continue validation.
pub const DAG_VERIFY_REASON_CONTINUE: u32 = 0;

/// Rust DAG verification reason: VRF key was not available.
pub const DAG_VERIFY_REASON_MISSING_VRF_KEY: u32 = 1;

/// Rust DAG verification reason: VDF proof did not validate.
pub const DAG_VERIFY_REASON_INVALID_VDF: u32 = 2;

/// Rust DAG verification reason: DPoS state for the block is not available.
pub const DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT: u32 = 3;

/// Rust DAG verification reason: block sender is not DPoS eligible.
pub const DAG_VERIFY_REASON_NOT_ELIGIBLE: u32 = 4;

/// Inputs for deterministic `DagManager::verifyBlock` prechecks.
///
/// This struct covers only checks that do not need transaction bodies, VDF
/// execution, DPOS state, gas estimation, events, or network effects. It is
/// intentionally codec- and storage-independent so bridge/runtime code can
/// provide lookup results without moving infrastructure concerns into the
/// consensus domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyPrecheckInput {
    pub block_level: u64,
    pub pivot: H256,
    pub tips: Vec<H256>,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
    pub dag_expiry_level: u64,
}

/// Decision returned by deterministic `DagManager::verifyBlock` prechecks.
///
/// `continue_validation == true` means only this Rust precheck passed; callers
/// must continue the remaining transaction, VDF, DPOS, and gas checks before
/// returning the public C++ `Verified` result. When `continue_validation` is
/// false, `reject_code` is one of the legacy-compatible reject constants in
/// this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyPrecheck {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub proposal_period_found: bool,
    pub proposal_period: u64,
}

/// Per-tip gas metadata used by DAG block gas validation.
///
/// Missing tips are represented as data so consensus-invalid blocks return the
/// legacy `MissingTip` outcome instead of using error handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagTipGas {
    pub found: bool,
    pub gas_estimation: u64,
}

/// Inputs for deterministic transaction availability checks in
/// `DagManager::verifyBlock`.
///
/// C++ owns live transaction lookup. Rust owns the deterministic decision over
/// expected and resolved transaction counts so missing-transaction semantics
/// stay explicit and testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyTransactionAvailabilityInput {
    pub expected_transactions: u64,
    pub resolved_transactions: u64,
}

/// Decision returned by deterministic transaction availability verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyTransactionAvailability {
    pub continue_validation: bool,
    pub reject_code: u32,
}

/// Inputs for deterministic gas checks in `DagManager::verifyBlock`.
///
/// C++ still owns live transaction lookup and EVM-backed transaction gas
/// estimation. Rust owns the deterministic decision over the resulting counts,
/// weights, DAG/PBFT gas limits, and tip gas metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyGasInput {
    pub block_gas_estimation: u64,
    pub estimated_transactions_weight: u64,
    pub dag_gas_limit: u64,
    pub pbft_gas_limit: u64,
    pub tip_gas_estimations: Vec<DagTipGas>,
}

/// Decision returned by deterministic gas verification.
///
/// `continue_validation == true` means gas checks passed. When false,
/// `reject_code` is a legacy-compatible
/// `DagManager::VerifyBlockReturnType` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagVerifyGas {
    pub continue_validation: bool,
    pub reject_code: u32,
}

/// Inputs for preparing VDF verification in `DagManager::verifyBlock`.
///
/// C++ still owns live VRF-key lookup and DPoS vote-count/max-vote reads. Rust
/// owns the deterministic decision for missing VRF keys and carries the
/// supplied VDF vote counts to the remaining C++ VDF verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfPrepareInput {
    pub vrf_key_found: bool,
    pub eligible_vote_count: u64,
    pub vdf_max_vote_count: u64,
}

/// VDF verification preparation result.
///
/// When `continue_validation` is true, C++ must use `vote_count` and
/// `max_vote_count` for the C++ VDF verifier. When false, `reject_code` is a
/// legacy-compatible `VerifyBlockReturnType` value and `reason_code` explains
/// the Rust decision for tests and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyVdfPrepare {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub reason_code: u32,
    pub vote_count: u64,
    pub max_vote_count: u64,
}

/// Inputs for deterministic authorization decisions in
/// `DagManager::verifyBlock`.
///
/// C++ still performs live VDF verification and DPoS state access. Rust owns
/// the ordering that maps those outcomes to legacy `VerifyBlockReturnType`
/// values. Missing VRF-key handling belongs to `prepare_dag_verify_vdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyAuthorizationInput {
    pub vdf_valid: bool,
    pub dpos_snapshot_available: bool,
    pub dpos_eligible: bool,
}

/// Decision returned by deterministic DAG block authorization verification.
///
/// `reason_code` is not a public C++ API value. It exists so bridge and Rust
/// tests can distinguish why one legacy reject code was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DagVerifyAuthorization {
    pub continue_validation: bool,
    pub reject_code: u32,
    pub reason_code: u32,
}

/// Derives frontier from a ghost path and current leaves.
///
/// Behavior mirrors legacy DagManager frontier rules:
/// - empty ghost path returns `{ pivot: 0, tips: [] }`
/// - non-empty path sets `pivot` to the last ghost-path hash
/// - `tips` contains leaves except `pivot`
///
/// Additional deterministic guarantees:
/// - tip order is preserved from `leaves` while removing only `pivot`.
pub fn derive_frontier(ghost_path: &[H256], leaves: &[H256]) -> DagFrontier {
    let Some(pivot) = ghost_path.last().copied() else {
        return DagFrontier {
            pivot: H256::zero(),
            tips: Vec::new(),
        };
    };

    let tips = leaves
        .iter()
        .copied()
        .filter(|hash| *hash != pivot)
        .collect::<Vec<_>>();

    DagFrontier { pivot, tips }
}

/// Validates expected block level and missing pivot/tip references from metadata.
///
/// This mirrors legacy DagManager logic:
/// - `expected_level` starts at `0`
/// - each found pivot/tip updates `expected_level = max(expected_level, level + 1)`
///   with `u64` wrapping addition to mirror legacy C++ unsigned arithmetic
/// - missing references are returned for caller-driven sync requests
/// - final `ok` requires both no missing references and matching block level
pub fn validate_pivot_tips_metadata(
    block_level: u64,
    pivot: DagReferenceMetadata,
    tips: &[DagReferenceMetadata],
) -> DagPivotTipsValidation {
    let mut expected_level = 0_u64;
    let mut missing_references = Vec::new();

    if pivot.found {
        expected_level = expected_level.max(pivot.level.wrapping_add(1));
    } else {
        missing_references.push(pivot.hash);
    }

    for tip in tips {
        if tip.found {
            expected_level = expected_level.max(tip.level.wrapping_add(1));
        } else {
            missing_references.push(tip.hash);
        }
    }

    let level_matches = block_level == expected_level;
    let ok = missing_references.is_empty() && level_matches;

    DagPivotTipsValidation {
        ok,
        expected_level,
        level_matches,
        missing_references,
    }
}

/// Runs deterministic `DagManager::verifyBlock` prechecks.
///
/// The order mirrors the deterministic portion of the legacy C++ verification
/// path: tip count/uniqueness, proposal-period availability, and expiry. The
/// public `Verified` result is deliberately not produced here because successful
/// prechecks are only permission to continue the remaining verification stages.
pub fn validate_dag_verify_precheck(input: DagVerifyPrecheckInput) -> DagVerifyPrecheck {
    let reject_code = if input.tips.len() > DAG_BLOCK_MAX_TIPS {
        Some(DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION)
    } else {
        let mut unique_references = BTreeSet::from([input.pivot]);
        let has_duplicate_tip = input.tips.iter().any(|tip| !unique_references.insert(*tip));

        if has_duplicate_tip {
            Some(DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION)
        } else if !input.proposal_period_found {
            Some(DAG_VERIFY_REJECT_AHEAD_BLOCK)
        } else if input.block_level < input.dag_expiry_level {
            Some(DAG_VERIFY_REJECT_EXPIRED_BLOCK)
        } else {
            None
        }
    };

    DagVerifyPrecheck {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
        proposal_period_found: input.proposal_period_found,
        proposal_period: input.proposal_period,
    }
}

/// Runs deterministic transaction availability checks for
/// `DagManager::verifyBlock`.
///
/// The helper returns only `MissingTransaction` or continue; VDF/DPOS checks
/// still run before gas validation to preserve legacy return ordering.
pub fn validate_dag_verify_transaction_availability(
    input: DagVerifyTransactionAvailabilityInput,
) -> DagVerifyTransactionAvailability {
    let reject_code = (input.resolved_transactions < input.expected_transactions)
        .then_some(DAG_VERIFY_REJECT_MISSING_TRANSACTION);

    DagVerifyTransactionAvailability {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
    }
}

/// Prepares deterministic VDF inputs for `DagManager::verifyBlock`.
///
/// Missing VRF key is a consensus reject. On success, this returns the vote
/// count and max-vote count supplied by the current DPoS data source.
pub fn prepare_dag_verify_vdf(input: DagVerifyVdfPrepareInput) -> DagVerifyVdfPrepare {
    let reject_code = if !input.vrf_key_found {
        DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
    } else {
        0
    };

    DagVerifyVdfPrepare {
        continue_validation: reject_code == 0,
        reject_code,
        reason_code: if reject_code == 0 {
            DAG_VERIFY_REASON_CONTINUE
        } else {
            DAG_VERIFY_REASON_MISSING_VRF_KEY
        },
        vote_count: input.eligible_vote_count,
        max_vote_count: input.vdf_max_vote_count,
    }
}

/// Runs deterministic gas checks for `DagManager::verifyBlock`.
///
/// This must be called after transaction availability, VDF, and DPOS checks to
/// preserve legacy `verifyBlock` return ordering. Tip count is derived from the
/// provided tip metadata so callers cannot accidentally bypass missing-tip or
/// aggregate-gas checks by passing inconsistent counts.
pub fn validate_dag_verify_gas(input: DagVerifyGasInput) -> DagVerifyGas {
    let reject_code = if input.block_gas_estimation > input.dag_gas_limit {
        Some(DAG_VERIFY_REJECT_BLOCK_TOO_BIG)
    } else if input.estimated_transactions_weight != input.block_gas_estimation {
        Some(DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION)
    } else if exceeds_pbft_dag_count(
        input.tip_gas_estimations.len() as u64,
        input.dag_gas_limit,
        input.pbft_gas_limit,
    ) {
        let mut total_gas = input.block_gas_estimation;
        for tip in input.tip_gas_estimations {
            if !tip.found {
                return DagVerifyGas {
                    continue_validation: false,
                    reject_code: DAG_VERIFY_REJECT_MISSING_TIP,
                };
            }
            total_gas = total_gas.wrapping_add(tip.gas_estimation);
        }
        (total_gas > input.pbft_gas_limit).then_some(DAG_VERIFY_REJECT_BLOCK_TOO_BIG)
    } else {
        None
    };

    DagVerifyGas {
        continue_validation: reject_code.is_none(),
        reject_code: reject_code.unwrap_or(0),
    }
}

/// Runs deterministic authorization checks for `DagManager::verifyBlock`.
///
/// This must be called after transaction availability and before gas checks to
/// preserve legacy return ordering. Invalid VDF proof maps to
/// `FailedVdfVerification`; DPoS state unavailability maps to `FutureBlock`;
/// ineligible validators map to `NotEligible`.
pub fn validate_dag_verify_authorization(
    input: DagVerifyAuthorizationInput,
) -> DagVerifyAuthorization {
    let (reject_code, reason_code) = if !input.vdf_valid {
        (
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION,
            DAG_VERIFY_REASON_INVALID_VDF,
        )
    } else if !input.dpos_snapshot_available {
        (
            DAG_VERIFY_REJECT_FUTURE_BLOCK,
            DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT,
        )
    } else if !input.dpos_eligible {
        (
            DAG_VERIFY_REJECT_NOT_ELIGIBLE,
            DAG_VERIFY_REASON_NOT_ELIGIBLE,
        )
    } else {
        (0, DAG_VERIFY_REASON_CONTINUE)
    };

    DagVerifyAuthorization {
        continue_validation: reject_code == 0,
        reject_code,
        reason_code,
    }
}

fn exceeds_pbft_dag_count(tips_count: u64, dag_gas_limit: u64, pbft_gas_limit: u64) -> bool {
    let Some(max_dag_blocks) = pbft_gas_limit.checked_div(dag_gas_limit) else {
        return true;
    };
    tips_count.saturating_add(1) > max_dag_blocks
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagGraph {
    vertices: BTreeMap<H256, BTreeSet<H256>>,
}

impl DagGraph {
    pub fn new(genesis: H256) -> Self {
        assert_ne!(genesis, H256::zero(), "DAG genesis hash must not be zero");

        let mut graph = Self {
            vertices: BTreeMap::new(),
        };
        graph.add_vertex_edges(genesis, H256::zero(), &[]);
        graph
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn edge_count(&self) -> usize {
        self.vertices.values().map(BTreeSet::len).sum()
    }

    pub fn has_vertex(&self, vertex: H256) -> bool {
        self.vertices.contains_key(&vertex)
    }

    pub fn add_vertex_edges(&mut self, new_vertex: H256, pivot: H256, tips: &[H256]) -> bool {
        assert_ne!(new_vertex, H256::zero(), "DAG vertex hash must not be zero");

        self.vertices.entry(new_vertex).or_default();

        let mut inserted_all_edges = true;
        if pivot != H256::zero() && self.has_vertex(pivot) {
            inserted_all_edges &= self.add_edge(pivot, new_vertex);
        }

        for tip in tips {
            if self.has_vertex(*tip) {
                inserted_all_edges &= self.add_edge(*tip, new_vertex);
            }
        }

        inserted_all_edges
    }

    pub fn leaves(&self) -> Vec<H256> {
        self.vertices
            .iter()
            .filter_map(|(vertex, children)| children.is_empty().then_some(*vertex))
            .collect()
    }

    pub fn reachable(&self, from: H256, to: H256) -> bool {
        if from == to {
            return self.has_vertex(from);
        }
        if !self.has_vertex(from) || !self.has_vertex(to) {
            return false;
        }

        let mut stack = vec![from];
        let mut visited = BTreeSet::from([from]);

        while let Some(current) = stack.pop() {
            let Some(children) = self.vertices.get(&current) else {
                continue;
            };
            for child in children {
                if *child == to {
                    return true;
                }
                if visited.insert(*child) {
                    stack.push(*child);
                }
            }
        }

        false
    }

    pub fn ghost_path(&self, root: H256) -> Vec<H256> {
        if !self.has_vertex(root) {
            return Vec::new();
        }

        let weights = self.descendant_weights(root);
        let mut path = Vec::new();
        let mut current = root;

        loop {
            path.push(current);

            let Some(children) = self.vertices.get(&current) else {
                break;
            };
            let next = children
                .iter()
                .filter_map(|child| weights.get(child).map(|weight| (*child, *weight)))
                .max_by(|(left_hash, left_weight), (right_hash, right_weight)| {
                    left_weight
                        .cmp(right_weight)
                        .then_with(|| right_hash.cmp(left_hash))
                });

            let Some((next_hash, next_weight)) = next else {
                break;
            };
            if next_weight == 0 {
                break;
            }
            current = next_hash;
        }

        path
    }

    pub fn compute_order(
        &self,
        anchor: H256,
        non_finalized_blocks: &BTreeMap<u64, BTreeSet<H256>>,
    ) -> Option<Vec<H256>> {
        if !self.has_vertex(anchor) {
            return None;
        }

        let mut epoch_vertices = BTreeSet::from([anchor]);
        for block in non_finalized_blocks.values().flatten() {
            if self.reachable(*block, anchor) {
                epoch_vertices.insert(*block);
            }
        }

        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();

        for vertex in &epoch_vertices {
            if !visited.insert(*vertex) {
                continue;
            }

            let mut dfs = vec![(*vertex, false)];
            while let Some((current, post_order)) = dfs.pop() {
                if post_order {
                    ordered.push(current);
                    continue;
                }

                dfs.push((current, true));

                let mut neighbors: Vec<H256> = self
                    .vertices
                    .get(&current)
                    .into_iter()
                    .flatten()
                    .filter(|child| epoch_vertices.contains(child))
                    .filter(|child| visited.insert(**child))
                    .copied()
                    .collect();
                neighbors.sort();

                for neighbor in neighbors {
                    dfs.push((neighbor, false));
                }
            }
        }

        ordered.reverse();
        Some(ordered)
    }

    pub fn clear(&mut self) {
        self.vertices.clear();
    }

    pub fn graphviz_dot(&self) -> String {
        let mut dot = String::from("digraph G {\n");
        for vertex in self.vertices.keys() {
            let _ = writeln!(
                dot,
                "  \"{}\" [label=\"{} \"];",
                hex_hash(vertex),
                hex_prefix(vertex)
            );
        }
        for (from, children) in &self.vertices {
            for child in children {
                let _ = writeln!(dot, "  \"{}\" -> \"{}\";", hex_hash(from), hex_hash(child));
            }
        }
        dot.push_str("}\n");
        dot
    }

    fn add_edge(&mut self, from: H256, to: H256) -> bool {
        match self.vertices.get_mut(&from) {
            Some(children) => children.insert(to),
            None => false,
        }
    }

    fn descendant_weights(&self, root: H256) -> BTreeMap<H256, usize> {
        let mut post_order = Vec::new();
        let mut stack = vec![root];

        while let Some(current) = stack.pop() {
            post_order.push(current);
            if let Some(children) = self.vertices.get(&current) {
                for child in children {
                    stack.push(*child);
                }
            }
        }
        post_order.reverse();

        let mut weights = BTreeMap::new();
        for vertex in post_order {
            let total_children_weight = self
                .vertices
                .get(&vertex)
                .into_iter()
                .flatten()
                .filter_map(|child| weights.get(child))
                .sum::<usize>();
            weights.insert(vertex, total_children_weight + 1);
        }

        weights
    }
}

/// Immutable block metadata used to update Rust-owned `DagManager` state.
///
/// Inputs:
/// - `hash`: DAG block hash. It must be nonzero.
/// - `pivot`: pivot parent hash, or zero for the current anchor root.
/// - `tips`: non-pivot parent hashes in block order.
/// - `level`: DAG level persisted on the block.
/// - `difficulty`: VDF difficulty used for non-finalized minimum-difficulty tracking.
///
/// Invariants:
/// - A block can be applied repeatedly without duplicating graph vertices or
///   non-finalized indexes.
/// - Missing parent hashes do not create edges, matching legacy graph behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerBlock {
    pub hash: H256,
    pub pivot: H256,
    pub tips: Vec<H256>,
    pub level: u64,
    pub difficulty: u32,
}

/// Complete snapshot used to rebuild Rust-owned `DagManager` state from the
/// C++ side while DB, transaction, event, and network ownership still lives in
/// C++.
///
/// Inputs:
/// - anchors and period mirror the legacy manager state at one point in time.
/// - `anchor_level`, `max_level`, and `dag_expiry_level` preserve legacy
///   counters that are still affected by storage and finalization side effects.
/// - `non_finalized_min_difficulty` is accepted from C++ for exact parity during
///   transitional rebuilds; subsequent Rust `add_block` calls maintain it.
/// - `non_finalized_blocks` is the ordered set of currently live blocks that
///   should be present in the in-memory DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerSnapshot {
    pub old_anchor: H256,
    pub anchor: H256,
    pub anchor_level: u64,
    pub period: u64,
    pub max_level: u64,
    pub dag_expiry_level: u64,
    pub non_finalized_min_difficulty: u32,
    pub non_finalized_blocks: Vec<DagManagerBlock>,
}

/// Rust-owned in-memory state for deterministic `DagManager` behavior.
///
/// This type owns the total DAG graph, pivot tree, non-finalized block index,
/// block levels, frontier, anchors, period, max level, expiry level, and
/// non-finalized minimum difficulty. It deliberately does not own storage,
/// transaction pool effects, verified-block events, or network gossip yet; the
/// C++ shim still performs those side effects and feeds successful state changes
/// into this object.
///
/// Output guarantees:
/// - Graph reads, frontier derivation, ghost path, block ordering, counters, and
///   pivot/tip metadata are derived from one Rust state object.
/// - Non-finalized block snapshots are returned in deterministic level/hash
///   order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DagManagerState {
    total_dag: DagGraph,
    pivot_tree: DagGraph,
    block_levels: BTreeMap<H256, u64>,
    blocks: BTreeMap<H256, DagManagerBlock>,
    non_finalized_blocks: BTreeMap<u64, BTreeSet<H256>>,
    old_anchor: H256,
    anchor: H256,
    period: u64,
    max_level: u64,
    dag_expiry_limit: u32,
    dag_expiry_level: u64,
    non_finalized_min_difficulty: u32,
    frontier: DagFrontier,
}

impl DagManagerState {
    /// Creates a Rust-owned manager state rooted at the genesis DAG block.
    ///
    /// `genesis` must be nonzero. The initial state has period `0`, no
    /// non-finalized blocks, zero expiry level, and a frontier derived from the
    /// single genesis root.
    pub fn new(genesis: H256, dag_expiry_limit: u32) -> Result<Self> {
        if genesis == H256::zero() {
            bail!("DagManagerState requires a nonzero genesis hash");
        }

        let total_dag = DagGraph::new(genesis);
        let pivot_tree = DagGraph::new(genesis);
        let frontier = derive_frontier(&pivot_tree.ghost_path(genesis), &total_dag.leaves());
        let mut block_levels = BTreeMap::new();
        block_levels.insert(genesis, 0);

        Ok(Self {
            total_dag,
            pivot_tree,
            block_levels,
            blocks: BTreeMap::new(),
            non_finalized_blocks: BTreeMap::new(),
            old_anchor: H256::zero(),
            anchor: genesis,
            period: 0,
            max_level: 0,
            dag_expiry_limit,
            dag_expiry_level: 0,
            non_finalized_min_difficulty: u32::MAX,
            frontier,
        })
    }

    /// Replaces the current Rust state with a full snapshot from the C++ side.
    ///
    /// This is the transitional synchronization point after startup recovery and
    /// finalization, where storage cleanup and transaction side effects are
    /// still owned by C++.
    pub fn rebuild_from_snapshot(&mut self, snapshot: DagManagerSnapshot) -> Result<()> {
        if snapshot.anchor == H256::zero() {
            bail!("DagManagerState snapshot anchor must be nonzero");
        }

        self.total_dag.clear();
        self.pivot_tree.clear();
        self.block_levels.clear();
        self.blocks.clear();
        self.non_finalized_blocks.clear();

        self.old_anchor = snapshot.old_anchor;
        self.anchor = snapshot.anchor;
        self.period = snapshot.period;
        self.max_level = snapshot.max_level;
        self.dag_expiry_level = snapshot.dag_expiry_level;
        self.non_finalized_min_difficulty = snapshot.non_finalized_min_difficulty;

        self.block_levels
            .insert(snapshot.anchor, snapshot.anchor_level);
        self.total_dag
            .add_vertex_edges(snapshot.anchor, H256::zero(), &[]);
        self.pivot_tree
            .add_vertex_edges(snapshot.anchor, H256::zero(), &[]);

        for block in snapshot.non_finalized_blocks {
            self.add_non_finalized_block(block)?;
        }
        self.frontier = self.compute_frontier();

        Ok(())
    }

    /// Builds a fresh Rust DAG manager state from one snapshot.
    ///
    /// This is a convenience constructor for callers that create state from a
    /// persisted snapshot rather than mutating an existing instance.
    pub fn from_snapshot(snapshot: DagManagerSnapshot, dag_expiry_limit: u32) -> Result<Self> {
        let mut state = Self::new(snapshot.anchor, dag_expiry_limit)?;
        state.rebuild_from_snapshot(snapshot)?;
        Ok(state)
    }

    /// Adds one non-finalized block to the Rust-owned in-memory DAG state.
    ///
    /// The caller must invoke this only after C++ side validation, persistence,
    /// and transaction handling have succeeded. The method updates graph edges,
    /// block level metadata, non-finalized indexes, max level, min difficulty,
    /// and frontier.
    pub fn add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.add_non_finalized_block(block)?;
        self.frontier = self.compute_frontier();
        Ok(())
    }

    /// Applies one finalized DAG order update and transitions to the next
    /// period/anchor.
    ///
    /// Inputs:
    /// - `new_anchor`: anchor hash for the new period (must be nonzero).
    /// - `new_period`: expected to be exactly `period + 1`.
    /// - `finalized_order`: hashes finalized by this period.
    ///
    /// Output:
    /// - number of finalized non-finalized hashes removed from Rust state.
    ///
    /// Behavior:
    /// - updates `old_anchor`, `anchor`, and `period`
    /// - removes finalized blocks from level indexes and block metadata
    /// - rebuilds DAG graphs and frontier from remaining non-finalized blocks
    pub fn set_finalized_order(
        &mut self,
        new_anchor: H256,
        new_period: u64,
        finalized_order: &[H256],
    ) -> Result<usize> {
        ensure!(new_anchor != H256::zero(), "new anchor must be nonzero");
        ensure!(
            new_period == self.period.saturating_add(1),
            "invalid period transition: expected {}, got {}",
            self.period.saturating_add(1),
            new_period
        );

        let anchor_level = self.block_levels.get(&new_anchor).copied().unwrap_or(0);
        let finalized = finalized_order.iter().copied().collect::<BTreeSet<_>>();
        let mut removed = 0usize;
        for hash in &finalized {
            if self.blocks.remove(hash).is_some() {
                removed += 1;
            }
            if let Some(level) = self.block_levels.remove(hash) {
                let remove_level_entry = self
                    .non_finalized_blocks
                    .get_mut(&level)
                    .map(|hashes| {
                        hashes.remove(hash);
                        hashes.is_empty()
                    })
                    .unwrap_or(false);
                if remove_level_entry {
                    self.non_finalized_blocks.remove(&level);
                }
            }
        }

        self.old_anchor = self.anchor;
        self.anchor = new_anchor;
        self.period = new_period;
        self.block_levels.clear();
        self.block_levels.insert(self.anchor, anchor_level);
        for block in self.blocks.values() {
            self.block_levels.insert(block.hash, block.level);
        }
        self.rebuild_graphs_from_records()?;
        self.refresh_non_finalized_min_difficulty();
        self.frontier = self.compute_frontier();

        Ok(removed)
    }

    /// Returns true when the total DAG mirror contains `hash`.
    pub fn has_vertex(&self, hash: H256) -> bool {
        self.total_dag.has_vertex(hash)
    }

    /// Returns reference metadata for pivot/tip validation from Rust state.
    pub fn reference_metadata(&self, hash: H256) -> DagReferenceMetadata {
        match self.block_levels.get(&hash).copied() {
            Some(level) if self.total_dag.has_vertex(hash) => DagReferenceMetadata {
                hash,
                found: true,
                level,
            },
            _ => DagReferenceMetadata {
                hash,
                found: false,
                level: 0,
            },
        }
    }

    /// Validates pivot/tip availability and level for a block using Rust state.
    pub fn validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: H256,
        tips: &[H256],
    ) -> DagPivotTipsValidation {
        let pivot = self.reference_metadata(pivot);
        let tips = tips
            .iter()
            .map(|tip| self.reference_metadata(*tip))
            .collect::<Vec<_>>();
        validate_pivot_tips_metadata(block_level, pivot, &tips)
    }

    /// Computes DAG order for `anchor` from the Rust non-finalized index.
    pub fn compute_order(&self, anchor: H256) -> Option<Vec<H256>> {
        self.total_dag
            .compute_order(anchor, &self.non_finalized_blocks)
    }

    /// Returns the pivot ghost path from an explicit source.
    pub fn ghost_path(&self, source: H256) -> Vec<H256> {
        self.pivot_tree.ghost_path(source)
    }

    /// Returns the pivot ghost path from the current anchor.
    pub fn anchor_ghost_path(&self) -> Vec<H256> {
        self.pivot_tree.ghost_path(self.anchor)
    }

    /// Returns the cached frontier derived from current Rust graph state.
    pub fn frontier(&self) -> &DagFrontier {
        &self.frontier
    }

    /// Returns graphviz output for the total DAG when `pivot_tree == false`,
    /// otherwise for the pivot tree.
    pub fn graphviz_dot(&self, pivot_tree: bool) -> String {
        if pivot_tree {
            self.pivot_tree.graphviz_dot()
        } else {
            self.total_dag.graphviz_dot()
        }
    }

    /// Returns the persisted old/current anchors mirrored in Rust state.
    pub fn anchors(&self) -> (H256, H256) {
        (self.old_anchor, self.anchor)
    }

    /// Returns the current anchor hash.
    pub fn anchor(&self) -> H256 {
        self.anchor
    }

    /// Returns the previous anchor hash.
    pub fn old_anchor(&self) -> H256 {
        self.old_anchor
    }

    /// Returns the latest finalized PBFT period mirrored in Rust state.
    pub fn period(&self) -> u64 {
        self.period
    }

    /// Returns the max non-finalized DAG level mirrored in Rust state.
    pub fn max_level(&self) -> u64 {
        self.max_level
    }

    /// Returns the configured DAG expiry limit.
    pub fn dag_expiry_limit(&self) -> u32 {
        self.dag_expiry_limit
    }

    /// Returns the currently active DAG expiry level.
    pub fn dag_expiry_level(&self) -> u64 {
        self.dag_expiry_level
    }

    /// Alias accessor for current DAG expiry level.
    pub fn expiry_level(&self) -> u64 {
        self.dag_expiry_level
    }

    /// Returns the current non-finalized minimum difficulty.
    pub fn non_finalized_min_difficulty(&self) -> u32 {
        self.non_finalized_min_difficulty
    }

    /// Optional minimum difficulty for non-finalized blocks.
    pub fn min_difficulty(&self) -> Option<u32> {
        (self.non_finalized_min_difficulty != u32::MAX).then_some(self.non_finalized_min_difficulty)
    }

    /// Returns total graph vertex count.
    pub fn vertex_count(&self) -> usize {
        self.total_dag.vertex_count()
    }

    /// Returns total graph edge count.
    pub fn edge_count(&self) -> usize {
        self.total_dag.edge_count()
    }

    /// Returns non-finalized levels and hashes in deterministic order.
    pub fn non_finalized_blocks(&self) -> &BTreeMap<u64, BTreeSet<H256>> {
        &self.non_finalized_blocks
    }

    /// Per-block level lookup map for current anchor and non-finalized blocks.
    pub fn block_levels(&self) -> &BTreeMap<H256, u64> {
        &self.block_levels
    }

    /// Read-only access to total DAG mirror.
    pub fn total_dag(&self) -> &DagGraph {
        &self.total_dag
    }

    /// Read-only access to pivot-tree DAG mirror.
    pub fn pivot_tree(&self) -> &DagGraph {
        &self.pivot_tree
    }

    /// Returns `(number of levels, number of blocks)` for non-finalized state.
    pub fn non_finalized_blocks_size(&self) -> (usize, usize) {
        (
            self.non_finalized_blocks.len(),
            self.non_finalized_blocks.values().map(BTreeSet::len).sum(),
        )
    }

    fn add_non_finalized_block(&mut self, block: DagManagerBlock) -> Result<()> {
        if block.hash == H256::zero() {
            bail!("DagManagerState cannot add a zero DAG block hash");
        }

        if let Some(existing) = self.blocks.get(&block.hash) {
            ensure!(
                existing == &block,
                "DagManagerState cannot add conflicting metadata for hash {:?}",
                block.hash
            );
            return Ok(());
        }

        self.blocks.insert(block.hash, block.clone());

        self.block_levels.insert(block.hash, block.level);
        self.max_level = self.max_level.max(block.level);
        self.non_finalized_blocks
            .entry(block.level)
            .or_default()
            .insert(block.hash);
        self.non_finalized_min_difficulty = self.non_finalized_min_difficulty.min(block.difficulty);

        self.total_dag
            .add_vertex_edges(block.hash, block.pivot, &block.tips);
        self.pivot_tree
            .add_vertex_edges(block.hash, block.pivot, &[]);

        Ok(())
    }

    fn rebuild_graphs_from_records(&mut self) -> Result<()> {
        self.total_dag.clear();
        self.pivot_tree.clear();
        self.total_dag
            .add_vertex_edges(self.anchor, H256::zero(), &[]);
        self.pivot_tree
            .add_vertex_edges(self.anchor, H256::zero(), &[]);

        for hash in self.non_finalized_blocks.values().flatten() {
            let block = self.blocks.get(hash).with_context(|| {
                format!("missing non-finalized block metadata for hash {hash:?}")
            })?;
            self.total_dag
                .add_vertex_edges(block.hash, block.pivot, &block.tips);
            self.pivot_tree
                .add_vertex_edges(block.hash, block.pivot, &[]);
        }
        Ok(())
    }

    fn refresh_non_finalized_min_difficulty(&mut self) {
        self.non_finalized_min_difficulty = self
            .blocks
            .values()
            .map(|block| block.difficulty)
            .min()
            .unwrap_or(u32::MAX);
    }

    fn compute_frontier(&self) -> DagFrontier {
        derive_frontier(
            &self.pivot_tree.ghost_path(self.anchor),
            &self.total_dag.leaves(),
        )
    }
}

fn hex_hash(hash: &H256) -> String {
    hash.as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hex_prefix(hash: &H256) -> String {
    hash.as_bytes()
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(value: u64) -> H256 {
        H256::from_low_u64_be(value)
    }

    fn set(values: impl IntoIterator<Item = H256>) -> BTreeSet<H256> {
        values.into_iter().collect()
    }

    #[test]
    fn genesis_graph_has_one_vertex_and_no_edges() {
        let graph = DagGraph::new(h(1));

        assert_eq!(graph.vertex_count(), 1);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.has_vertex(h(1)));
        assert_eq!(graph.leaves(), vec![h(1)]);
    }

    #[test]
    fn repeated_vertex_insertion_does_not_duplicate_vertices_or_edges() {
        let mut graph = DagGraph::new(h(1));

        assert!(graph.add_vertex_edges(h(2), h(1), &[]));
        assert!(!graph.add_vertex_edges(h(2), h(1), &[]));

        assert_eq!(graph.vertex_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn missing_pivot_and_tips_do_not_create_edges() {
        let mut graph = DagGraph::new(h(1));

        assert!(graph.add_vertex_edges(h(2), h(99), &[h(98)]));

        assert_eq!(graph.vertex_count(), 2);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.leaves(), vec![h(1), h(2)]);
    }

    #[test]
    fn leaf_collection_includes_isolated_vertices_and_is_hash_ordered() {
        let mut graph = DagGraph::new(h(10));

        graph.add_vertex_edges(h(3), H256::zero(), &[]);
        graph.add_vertex_edges(h(2), h(10), &[]);
        graph.add_vertex_edges(h(1), h(10), &[]);

        assert_eq!(graph.leaves(), vec![h(1), h(2), h(3)]);
    }

    #[test]
    fn reachability_handles_self_descendants_missing_and_disconnected_vertices() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(2), &[]);
        graph.add_vertex_edges(h(4), H256::zero(), &[]);

        assert!(graph.reachable(h(1), h(1)));
        assert!(graph.reachable(h(1), h(3)));
        assert!(!graph.reachable(h(3), h(1)));
        assert!(!graph.reachable(h(4), h(3)));
        assert!(!graph.reachable(h(99), h(3)));
        assert!(!graph.reachable(h(3), h(99)));
    }

    #[test]
    fn ghost_path_prefers_heaviest_subtree_and_ties_by_smallest_hash() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(3), h(1), &[]);
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(4), h(3), &[]);
        graph.add_vertex_edges(h(5), h(3), &[]);

        assert_eq!(graph.ghost_path(h(1)), vec![h(1), h(3), h(4)]);
        assert_eq!(graph.ghost_path(h(99)), Vec::<H256>::new());
    }

    #[test]
    fn compute_order_returns_none_for_missing_anchor() {
        let graph = DagGraph::new(h(1));

        assert_eq!(graph.compute_order(h(99), &BTreeMap::new()), None);
    }

    #[test]
    fn compute_order_keeps_only_blocks_that_reach_anchor() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(2), &[]);
        graph.add_vertex_edges(h(4), H256::zero(), &[]);

        let non_finalized = BTreeMap::from([(1, set([h(1), h(2), h(3), h(4)]))]);

        assert_eq!(
            graph.compute_order(h(3), &non_finalized),
            Some(vec![h(1), h(2), h(3)])
        );
    }

    #[test]
    fn compute_order_is_deterministic_for_conflux_fixture() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);
        graph.add_vertex_edges(h(3), h(1), &[]);
        graph.add_vertex_edges(h(4), h(2), &[h(3)]);
        graph.add_vertex_edges(h(5), h(2), &[]);
        graph.add_vertex_edges(h(7), h(3), &[]);
        graph.add_vertex_edges(h(6), h(4), &[h(5), h(7)]);
        graph.add_vertex_edges(h(8), h(2), &[]);
        graph.add_vertex_edges(h(11), h(7), &[]);
        graph.add_vertex_edges(h(10), h(11), &[h(4)]);
        graph.add_vertex_edges(h(9), h(6), &[h(8), h(10)]);
        graph.add_vertex_edges(h(12), h(9), &[]);

        let non_finalized = BTreeMap::from([(1, set([h(8), h(9), h(10), h(11)]))]);

        assert_eq!(
            graph.compute_order(h(9), &non_finalized),
            Some(vec![h(11), h(10), h(8), h(9)])
        );
    }

    #[test]
    fn compute_order_is_stable_across_insertion_order() {
        let mut left = DagGraph::new(h(1));
        left.add_vertex_edges(h(2), h(1), &[]);
        left.add_vertex_edges(h(3), h(1), &[]);
        left.add_vertex_edges(h(4), h(2), &[h(3)]);

        let mut right = DagGraph::new(h(1));
        right.add_vertex_edges(h(3), h(1), &[]);
        right.add_vertex_edges(h(2), h(1), &[]);
        right.add_vertex_edges(h(4), h(2), &[h(3)]);

        let non_finalized = BTreeMap::from([(1, set([h(2), h(3), h(4)]))]);

        assert_eq!(
            left.compute_order(h(4), &non_finalized),
            right.compute_order(h(4), &non_finalized)
        );
    }

    #[test]
    fn clear_empties_graph_and_allows_rebuild() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);

        graph.clear();
        assert_eq!(graph.vertex_count(), 0);
        assert_eq!(graph.edge_count(), 0);
        assert!(graph.leaves().is_empty());

        graph.add_vertex_edges(h(3), H256::zero(), &[]);
        assert_eq!(graph.vertex_count(), 1);
        assert_eq!(graph.leaves(), vec![h(3)]);
    }

    #[test]
    fn graphviz_dot_uses_current_graph_edges() {
        let mut graph = DagGraph::new(h(1));
        graph.add_vertex_edges(h(2), h(1), &[]);

        let dot = graph.graphviz_dot();

        assert!(
            dot.contains("\"0000000000000000000000000000000000000000000000000000000000000001\"")
        );
        assert!(dot.contains("\"0000000000000000000000000000000000000000000000000000000000000001\" -> \"0000000000000000000000000000000000000000000000000000000000000002\""));
    }

    #[test]
    fn frontier_derivation_returns_empty_when_ghost_path_is_empty() {
        let frontier = derive_frontier(&[], &[h(1), h(2)]);

        assert_eq!(frontier.pivot, H256::zero());
        assert_eq!(frontier.tips, Vec::<H256>::new());
    }

    #[test]
    fn frontier_derivation_removes_pivot_and_preserves_leaf_order() {
        let frontier = derive_frontier(&[h(10), h(20)], &[h(30), h(20), h(10), h(30), h(2)]);

        assert_eq!(frontier.pivot, h(20));
        assert_eq!(frontier.tips, vec![h(30), h(10), h(30), h(2)]);
    }

    #[test]
    fn pivot_tips_validation_reports_missing_references_and_expected_level() {
        let result = validate_pivot_tips_metadata(
            11,
            DagReferenceMetadata {
                hash: h(100),
                found: false,
                level: 0,
            },
            &[
                DagReferenceMetadata {
                    hash: h(101),
                    found: true,
                    level: 4,
                },
                DagReferenceMetadata {
                    hash: h(102),
                    found: false,
                    level: 0,
                },
                DagReferenceMetadata {
                    hash: h(103),
                    found: true,
                    level: 9,
                },
            ],
        );

        assert!(!result.ok);
        assert_eq!(result.expected_level, 10);
        assert!(!result.level_matches);
        assert_eq!(result.missing_references, vec![h(100), h(102)]);
    }

    #[test]
    fn pivot_tips_validation_succeeds_when_level_matches_and_no_missing() {
        let result = validate_pivot_tips_metadata(
            8,
            DagReferenceMetadata {
                hash: h(200),
                found: true,
                level: 5,
            },
            &[
                DagReferenceMetadata {
                    hash: h(201),
                    found: true,
                    level: 7,
                },
                DagReferenceMetadata {
                    hash: h(202),
                    found: true,
                    level: 6,
                },
            ],
        );

        assert!(result.ok);
        assert_eq!(result.expected_level, 8);
        assert!(result.level_matches);
        assert!(result.missing_references.is_empty());
    }

    #[test]
    fn pivot_tips_validation_wraps_level_like_cpp_unsigned_arithmetic() {
        let result = validate_pivot_tips_metadata(
            0,
            DagReferenceMetadata {
                hash: h(300),
                found: true,
                level: u64::MAX,
            },
            &[],
        );

        assert!(result.ok);
        assert_eq!(result.expected_level, 0);
        assert!(result.level_matches);
        assert!(result.missing_references.is_empty());
    }

    #[test]
    fn verify_precheck_rejects_tip_count_over_limit() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: (2..=(DAG_BLOCK_MAX_TIPS as u64 + 2)).map(h).collect(),
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
    }

    #[test]
    fn verify_precheck_rejects_duplicate_pivot_or_tip_reference() {
        let duplicate_pivot = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: vec![h(2), h(1)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });
        let duplicate_tip = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 10,
            pivot: h(1),
            tips: vec![h(2), h(2)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 0,
        });

        assert_eq!(
            duplicate_pivot.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
        assert_eq!(
            duplicate_tip.reject_code,
            DAG_VERIFY_REJECT_FAILED_TIPS_VERIFICATION
        );
    }

    #[test]
    fn verify_precheck_rejects_missing_proposal_period_before_expiry() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 1,
            pivot: h(1),
            tips: vec![],
            proposal_period_found: false,
            proposal_period: 0,
            dag_expiry_level: 2,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_AHEAD_BLOCK);
    }

    #[test]
    fn verify_precheck_rejects_expired_block() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 4,
            pivot: h(1),
            tips: vec![],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 5,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_EXPIRED_BLOCK);
    }

    #[test]
    fn verify_precheck_continues_for_remaining_validation() {
        let result = validate_dag_verify_precheck(DagVerifyPrecheckInput {
            block_level: 5,
            pivot: h(1),
            tips: vec![h(2), h(3)],
            proposal_period_found: true,
            proposal_period: 7,
            dag_expiry_level: 5,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.proposal_period, 7);
    }

    #[test]
    fn verify_transaction_availability_rejects_missing_transactions() {
        let result =
            validate_dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
                expected_transactions: 3,
                resolved_transactions: 2,
            });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_MISSING_TRANSACTION);
    }

    #[test]
    fn verify_transaction_availability_continues_when_all_transactions_are_present() {
        let result =
            validate_dag_verify_transaction_availability(DagVerifyTransactionAvailabilityInput {
                expected_transactions: 3,
                resolved_transactions: 3,
            });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
    }

    #[test]
    fn verify_vdf_prepare_rejects_when_vrf_key_is_missing() {
        let result = prepare_dag_verify_vdf(DagVerifyVdfPrepareInput {
            vrf_key_found: false,
            eligible_vote_count: 12,
            vdf_max_vote_count: 42,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_MISSING_VRF_KEY);
    }

    #[test]
    fn verify_vdf_prepare_uses_supplied_max_vote_count() {
        let result = prepare_dag_verify_vdf(DagVerifyVdfPrepareInput {
            vrf_key_found: true,
            eligible_vote_count: 12,
            vdf_max_vote_count: 42,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_CONTINUE);
        assert_eq!(result.vote_count, 12);
        assert_eq!(result.max_vote_count, 42);
    }

    #[test]
    fn verify_authorization_rejects_when_vdf_is_invalid() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: false,
            dpos_snapshot_available: true,
            dpos_eligible: true,
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_FAILED_VDF_VERIFICATION
        );
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_INVALID_VDF);
    }

    #[test]
    fn verify_authorization_rejects_future_snapshot_before_not_eligible() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: false,
            dpos_eligible: false,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_FUTURE_BLOCK);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_FUTURE_DPOS_SNAPSHOT);
    }

    #[test]
    fn verify_authorization_rejects_not_eligible() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: false,
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_NOT_ELIGIBLE);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_NOT_ELIGIBLE);
    }

    #[test]
    fn verify_authorization_continues_when_all_checks_pass() {
        let result = validate_dag_verify_authorization(DagVerifyAuthorizationInput {
            vdf_valid: true,
            dpos_snapshot_available: true,
            dpos_eligible: true,
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
        assert_eq!(result.reason_code, DAG_VERIFY_REASON_CONTINUE);
    }

    #[test]
    fn verify_gas_rejects_block_over_dag_limit() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 101,
            estimated_transactions_weight: 101,
            dag_gas_limit: 100,
            pbft_gas_limit: 500,
            tip_gas_estimations: vec![],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_BLOCK_TOO_BIG);
    }

    #[test]
    fn verify_gas_rejects_weight_mismatch() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 91,
            dag_gas_limit: 100,
            pbft_gas_limit: 500,
            tip_gas_estimations: vec![],
        });

        assert!(!result.continue_validation);
        assert_eq!(
            result.reject_code,
            DAG_VERIFY_REJECT_INCORRECT_TRANSACTIONS_ESTIMATION
        );
    }

    #[test]
    fn verify_gas_rejects_missing_tip_when_pbft_aggregation_is_needed() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 200,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
                DagTipGas {
                    found: false,
                    gas_estimation: 0,
                },
            ],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_MISSING_TIP);
    }

    #[test]
    fn verify_gas_rejects_tips_over_pbft_limit() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 200,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
                DagTipGas {
                    found: true,
                    gas_estimation: 50,
                },
            ],
        });

        assert!(!result.continue_validation);
        assert_eq!(result.reject_code, DAG_VERIFY_REJECT_BLOCK_TOO_BIG);
    }

    #[test]
    fn verify_gas_continues_when_all_checks_pass() {
        let result = validate_dag_verify_gas(DagVerifyGasInput {
            block_gas_estimation: 90,
            estimated_transactions_weight: 90,
            dag_gas_limit: 100,
            pbft_gas_limit: 300,
            tip_gas_estimations: vec![
                DagTipGas {
                    found: true,
                    gas_estimation: 80,
                },
                DagTipGas {
                    found: true,
                    gas_estimation: 70,
                },
            ],
        });

        assert!(result.continue_validation);
        assert_eq!(result.reject_code, 0);
    }

    fn record(hash: u64, pivot: u64, tips: &[u64], level: u64, difficulty: u64) -> DagManagerBlock {
        DagManagerBlock {
            hash: h(hash),
            pivot: h(pivot),
            tips: tips.iter().copied().map(h).collect(),
            level,
            difficulty: difficulty as u32,
        }
    }

    #[test]
    fn dag_manager_state_add_block_updates_indexes_and_frontier() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");

        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[1], 3, 50)).expect("add");

        assert_eq!(state.max_level(), 3);
        assert_eq!(state.min_difficulty(), Some(50));
        assert_eq!(state.frontier().pivot, h(3));
        assert!(state.frontier().tips.is_empty());
        assert_eq!(state.block_levels().get(&h(2)), Some(&2));
        assert_eq!(state.block_levels().get(&h(3)), Some(&3));
    }

    #[test]
    fn dag_manager_state_rebuild_from_snapshot_restores_state() {
        let snapshot = DagManagerSnapshot {
            anchor: h(1),
            old_anchor: h(1),
            anchor_level: 0,
            period: 5,
            max_level: 9,
            dag_expiry_level: 4,
            non_finalized_min_difficulty: 60,
            non_finalized_blocks: vec![
                record(2, 1, &[], 2, 100),
                record(3, 2, &[1], 3, 80),
                record(4, 3, &[2], 4, 60),
            ],
        };

        let state = DagManagerState::from_snapshot(snapshot, 77).expect("snapshot");
        assert_eq!(state.anchor(), h(1));
        assert_eq!(state.old_anchor(), h(1));
        assert_eq!(state.period(), 5);
        assert_eq!(state.max_level(), 9);
        assert_eq!(state.expiry_level(), 4);
        assert_eq!(state.min_difficulty(), Some(60_u32));
        assert_eq!(state.frontier().pivot, h(4));
        assert!(state.frontier().tips.is_empty());
        assert!(state.total_dag().has_vertex(h(4)));
        assert!(state.pivot_tree().has_vertex(h(4)));
    }

    #[test]
    fn dag_manager_state_set_finalized_order_updates_anchor_and_rebuilds_graphs() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        state.add_block(record(2, 1, &[], 2, 100)).expect("add");
        state.add_block(record(3, 2, &[], 3, 90)).expect("add");
        state.add_block(record(4, 2, &[3], 4, 80)).expect("add");

        let removed = state
            .set_finalized_order(h(4), 1, &[h(2), h(3), h(4)])
            .expect("finalize");
        assert_eq!(removed, 3);
        assert_eq!(state.old_anchor(), h(1));
        assert_eq!(state.anchor(), h(4));
        assert_eq!(state.period(), 1);
        assert!(state.non_finalized_blocks().is_empty());
        assert_eq!(state.block_levels().len(), 1);
        assert_eq!(state.block_levels().get(&h(4)), Some(&4));
        assert_eq!(state.min_difficulty(), None);
        assert_eq!(state.frontier().pivot, h(4));
        assert!(state.frontier().tips.is_empty());
    }

    #[test]
    fn dag_manager_state_set_finalized_order_rejects_invalid_period_transition() {
        let mut state = DagManagerState::new(h(1), 0).expect("state");
        let snapshot = DagManagerSnapshot {
            anchor: h(1),
            old_anchor: h(1),
            anchor_level: 0,
            period: 2,
            max_level: 0,
            dag_expiry_level: 0,
            non_finalized_min_difficulty: u32::MAX,
            non_finalized_blocks: vec![],
        };
        state.rebuild_from_snapshot(snapshot).expect("snapshot");
        let err = state
            .set_finalized_order(h(2), 4, &[])
            .expect_err("period transition must fail");
        assert!(format!("{err:#}").contains("invalid period transition"));
    }
}
