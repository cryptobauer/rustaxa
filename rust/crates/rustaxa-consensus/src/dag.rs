use ethereum_types::H256;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

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
}
