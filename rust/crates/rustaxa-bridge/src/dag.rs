use crate::ffi::rustaxa_ffi::{
    DagFrontier, DagHash, DagLevelHashes, DagManagerAnchors, DagManagerBlock,
    DagManagerNonFinalizedSize, DagManagerSnapshot, DagOrder, DagPivotTipsValidation,
    DagReferenceMetadata,
};
use crate::ffi::{BridgeDagGraph, BridgeDagManagerState};
use anyhow::Result;
use ethereum_types::H256;
use rustaxa_consensus::dag::{
    derive_frontier, validate_pivot_tips_metadata, DagGraph,
    DagManagerBlock as DomainDagManagerBlock, DagManagerSnapshot as DomainDagManagerSnapshot,
    DagManagerState, DagReferenceMetadata as ReferenceMetadata,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph> {
    Box::new(BridgeDagGraph(DagGraph::new(to_h256(genesis))))
}

/// Creates a Rust-owned DagManager state bridge rooted at `genesis`.
///
/// The returned state owns deterministic graph/index data only; C++ remains
/// responsible for DB writes, transaction side effects, events, and networking
/// until those domains are explicitly migrated.
pub fn create_dag_manager_state(
    genesis: &[u8; 32],
    dag_expiry_limit: u32,
) -> Result<Box<BridgeDagManagerState>> {
    Ok(Box::new(BridgeDagManagerState(DagManagerState::new(
        to_h256(genesis),
        dag_expiry_limit,
    )?)))
}

pub fn dag_derive_frontier(ghost_path: Vec<DagHash>, leaves: Vec<DagHash>) -> DagFrontier {
    let ghost_path = ghost_path
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    let leaves = leaves
        .into_iter()
        .map(|hash| H256::from(hash.hash))
        .collect::<Vec<_>>();
    let frontier = derive_frontier(&ghost_path, &leaves);

    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips),
    }
}

pub fn dag_validate_pivot_tips_metadata(
    block_level: u64,
    pivot: DagReferenceMetadata,
    tips: Vec<DagReferenceMetadata>,
) -> DagPivotTipsValidation {
    let pivot = ReferenceMetadata {
        hash: H256::from(pivot.hash),
        found: pivot.found,
        level: pivot.level,
    };
    let tips = tips
        .into_iter()
        .map(|tip| ReferenceMetadata {
            hash: H256::from(tip.hash),
            found: tip.found,
            level: tip.level,
        })
        .collect::<Vec<_>>();
    let validation = validate_pivot_tips_metadata(block_level, pivot, &tips);

    DagPivotTipsValidation {
        ok: validation.ok,
        expected_level: validation.expected_level,
        level_matches: validation.level_matches,
        missing_references: to_dag_hashes(validation.missing_references),
    }
}

impl BridgeDagGraph {
    pub fn dag_vertex_count(&self) -> usize {
        self.0.vertex_count()
    }

    pub fn dag_edge_count(&self) -> usize {
        self.0.edge_count()
    }

    pub fn dag_has_vertex(&self, vertex: &[u8; 32]) -> bool {
        self.0.has_vertex(to_h256(vertex))
    }

    pub fn dag_add_vertex_edges(
        &mut self,
        new_vertex: &[u8; 32],
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> bool {
        let tips = tips
            .iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        self.0
            .add_vertex_edges(to_h256(new_vertex), to_h256(pivot), &tips)
    }

    pub fn dag_leaves(&self) -> Vec<DagHash> {
        to_dag_hashes(self.0.leaves())
    }

    pub fn dag_ghost_path(&self, root: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.0.ghost_path(to_h256(root)))
    }

    pub fn dag_compute_order(
        &self,
        anchor: &[u8; 32],
        non_finalized_blocks: Vec<DagLevelHashes>,
    ) -> DagOrder {
        let non_finalized_blocks = non_finalized_blocks
            .into_iter()
            .map(|level_hashes| {
                (
                    level_hashes.level,
                    level_hashes
                        .hashes
                        .into_iter()
                        .map(|hash| H256::from(hash.hash))
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect::<BTreeMap<_, _>>();

        match self.0.compute_order(to_h256(anchor), &non_finalized_blocks) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    pub fn dag_clear(&mut self) {
        self.0.clear();
    }

    pub fn dag_graphviz_dot(&self) -> String {
        self.0.graphviz_dot()
    }
}

impl BridgeDagManagerState {
    pub fn dag_manager_rebuild(&mut self, snapshot: DagManagerSnapshot) -> Result<()> {
        self.0.rebuild_from_snapshot(to_domain_snapshot(snapshot))
    }

    pub fn dag_manager_add_block(&mut self, block: DagManagerBlock) -> Result<()> {
        self.0.add_block(to_domain_block(block))
    }

    pub fn dag_manager_validate_pivot_tips(
        &self,
        block_level: u64,
        pivot: &[u8; 32],
        tips: Vec<DagHash>,
    ) -> DagPivotTipsValidation {
        let tips = tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect::<Vec<_>>();
        let validation = self
            .0
            .validate_pivot_tips(block_level, to_h256(pivot), &tips);

        DagPivotTipsValidation {
            ok: validation.ok,
            expected_level: validation.expected_level,
            level_matches: validation.level_matches,
            missing_references: to_dag_hashes(validation.missing_references),
        }
    }

    pub fn dag_manager_compute_order(&self, anchor: &[u8; 32]) -> DagOrder {
        match self.0.compute_order(to_h256(anchor)) {
            Some(hashes) => DagOrder {
                found: true,
                hashes: to_dag_hashes(hashes),
            },
            None => DagOrder {
                found: false,
                hashes: Vec::new(),
            },
        }
    }

    pub fn dag_manager_frontier(&self) -> DagFrontier {
        to_bridge_frontier(self.0.frontier())
    }

    pub fn dag_manager_ghost_path(&self, source: &[u8; 32]) -> Vec<DagHash> {
        to_dag_hashes(self.0.ghost_path(to_h256(source)))
    }

    pub fn dag_manager_anchor_ghost_path(&self) -> Vec<DagHash> {
        to_dag_hashes(self.0.anchor_ghost_path())
    }

    pub fn dag_manager_graphviz_dot(&self, pivot_tree: bool) -> String {
        self.0.graphviz_dot(pivot_tree)
    }

    pub fn dag_manager_vertex_count(&self) -> usize {
        self.0.vertex_count()
    }

    pub fn dag_manager_edge_count(&self) -> usize {
        self.0.edge_count()
    }

    pub fn dag_manager_max_level(&self) -> u64 {
        self.0.max_level()
    }

    pub fn dag_manager_latest_period(&self) -> u64 {
        self.0.period()
    }

    pub fn dag_manager_anchors(&self) -> DagManagerAnchors {
        let (old_anchor, anchor) = self.0.anchors();
        DagManagerAnchors {
            old_anchor: old_anchor.into(),
            anchor: anchor.into(),
        }
    }

    pub fn dag_manager_dag_expiry_limit(&self) -> u32 {
        self.0.dag_expiry_limit()
    }

    pub fn dag_manager_dag_expiry_level(&self) -> u64 {
        self.0.dag_expiry_level()
    }

    pub fn dag_manager_non_finalized_blocks(&self) -> Vec<DagLevelHashes> {
        self.0
            .non_finalized_blocks()
            .iter()
            .map(|(level, hashes)| DagLevelHashes {
                level: *level,
                hashes: to_dag_hashes(hashes.iter().copied().collect()),
            })
            .collect()
    }

    pub fn dag_manager_non_finalized_blocks_size(&self) -> DagManagerNonFinalizedSize {
        let (levels, blocks) = self.0.non_finalized_blocks_size();
        DagManagerNonFinalizedSize {
            levels: levels as u64,
            blocks: blocks as u64,
        }
    }

    pub fn dag_manager_non_finalized_min_difficulty(&self) -> u32 {
        self.0.non_finalized_min_difficulty()
    }
}

fn to_h256(hash: &[u8; 32]) -> H256 {
    H256::from(*hash)
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}

fn to_bridge_frontier(frontier: &rustaxa_consensus::dag::DagFrontier) -> DagFrontier {
    DagFrontier {
        pivot: frontier.pivot.into(),
        tips: to_dag_hashes(frontier.tips.clone()),
    }
}

fn to_domain_block(block: DagManagerBlock) -> DomainDagManagerBlock {
    DomainDagManagerBlock {
        hash: H256::from(block.hash),
        pivot: H256::from(block.pivot),
        tips: block
            .tips
            .into_iter()
            .map(|tip| H256::from(tip.hash))
            .collect(),
        level: block.level,
        difficulty: block.difficulty,
    }
}

fn to_domain_snapshot(snapshot: DagManagerSnapshot) -> DomainDagManagerSnapshot {
    DomainDagManagerSnapshot {
        old_anchor: H256::from(snapshot.old_anchor),
        anchor: H256::from(snapshot.anchor),
        anchor_level: snapshot.anchor_level,
        period: snapshot.period,
        max_level: snapshot.max_level,
        dag_expiry_level: snapshot.dag_expiry_level,
        non_finalized_min_difficulty: snapshot.non_finalized_min_difficulty,
        non_finalized_blocks: snapshot
            .non_finalized_blocks
            .into_iter()
            .map(to_domain_block)
            .collect(),
    }
}
