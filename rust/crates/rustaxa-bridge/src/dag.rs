use crate::ffi::rustaxa_ffi::{
    DagFrontier, DagHash, DagLevelHashes, DagOrder, DagPivotTipsValidation, DagReferenceMetadata,
};
use crate::ffi::BridgeDagGraph;
use ethereum_types::H256;
use rustaxa_consensus::dag::{
    derive_frontier, validate_pivot_tips_metadata, DagGraph,
    DagReferenceMetadata as ReferenceMetadata,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph> {
    Box::new(BridgeDagGraph(DagGraph::new(to_h256(genesis))))
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

fn to_h256(hash: &[u8; 32]) -> H256 {
    H256::from(*hash)
}

fn to_dag_hashes(hashes: Vec<H256>) -> Vec<DagHash> {
    hashes
        .into_iter()
        .map(|hash| DagHash { hash: hash.0 })
        .collect()
}
