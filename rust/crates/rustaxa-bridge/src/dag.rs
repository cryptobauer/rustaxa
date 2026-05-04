use crate::ffi::rustaxa_ffi::{DagHash, DagLevelHashes, DagOrder};
use crate::ffi::BridgeDagGraph;
use ethereum_types::H256;
use rustaxa_consensus::dag::DagGraph;
use std::collections::{BTreeMap, BTreeSet};

pub fn create_dag_graph(genesis: &[u8; 32]) -> Box<BridgeDagGraph> {
    Box::new(BridgeDagGraph(DagGraph::new(to_h256(genesis))))
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
