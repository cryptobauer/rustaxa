#include <array>
#include <fstream>
#include <stdexcept>
#include <string>
#include <utility>

#include "dag/dag.hpp"
#include "rustaxa-bridge/ffi.rs.h"

namespace taraxa {
namespace {

std::array<uint8_t, 32> to_bridge_hash(blk_hash_t const& hash) { return hash.asArray(); }

rustaxa::DagHash to_bridge_dag_hash(blk_hash_t const& hash) { return rustaxa::DagHash{to_bridge_hash(hash)}; }

rust::Vec<rustaxa::DagHash> to_bridge_dag_hashes(std::vector<blk_hash_t> const& hashes) {
  rust::Vec<rustaxa::DagHash> out;
  out.reserve(hashes.size());
  for (auto const& hash : hashes) {
    out.push_back(to_bridge_dag_hash(hash));
  }
  return out;
}

rust::Vec<rustaxa::DagLevelHashes> to_bridge_non_finalized_blocks(
    std::map<uint64_t, std::unordered_set<blk_hash_t>> const& non_finalized_blks) {
  rust::Vec<rustaxa::DagLevelHashes> out;
  out.reserve(non_finalized_blks.size());
  for (auto const& [level, hashes] : non_finalized_blks) {
    rustaxa::DagLevelHashes level_hashes;
    level_hashes.level = level;
    level_hashes.hashes.reserve(hashes.size());
    for (auto const& hash : hashes) {
      level_hashes.hashes.push_back(to_bridge_dag_hash(hash));
    }
    out.push_back(std::move(level_hashes));
  }
  return out;
}

blk_hash_t from_bridge_dag_hash(rustaxa::DagHash const& hash) {
  return blk_hash_t(hash.hash.data(), blk_hash_t::ConstructFromPointer);
}

void from_bridge_dag_hashes(rust::Vec<rustaxa::DagHash> const& hashes, std::vector<blk_hash_t>& out) {
  out.reserve(out.size() + hashes.size());
  for (auto const& hash : hashes) {
    out.emplace_back(from_bridge_dag_hash(hash));
  }
}

[[noreturn]] void throw_unimplemented_dag_api(const char* api_name) {
  throw std::logic_error("Dag::" + std::string(api_name) + " is not implemented in Rust shim mode");
}

}  // namespace

struct Dag::RustDagGraphHolder {
  explicit RustDagGraphHolder(blk_hash_t const& genesis) : graph(rustaxa::create_dag_graph(to_bridge_hash(genesis))) {}

  rust::Box<rustaxa::BridgeDagGraph> graph;
};

Dag::Dag(blk_hash_t const& dag_genesis_block_hash, [[maybe_unused]] addr_t node_addr) {
  if (dag_genesis_block_hash.isZero()) {
    throw std::invalid_argument("Dag requires a nonzero genesis hash");
  }
  rust_graph_ = std::make_unique<RustDagGraphHolder>(dag_genesis_block_hash);
}

Dag::~Dag() = default;

Dag::Dag(Dag const&) { throw std::logic_error("Rust-backed Dag cannot be copied"); }

Dag::Dag(Dag&&) noexcept = default;

Dag& Dag::operator=(Dag const& other) {
  if (this == &other) {
    return *this;
  }
  throw std::logic_error("Rust-backed Dag cannot be copy-assigned");
}

Dag& Dag::operator=(Dag&&) noexcept = default;

uint64_t Dag::getNumVertices() const { return rust_graph_->graph->dag_vertex_count(); }

uint64_t Dag::getNumEdges() const { return rust_graph_->graph->dag_edge_count(); }

bool Dag::hasVertex(blk_hash_t const& v) const { return rust_graph_->graph->dag_has_vertex(to_bridge_hash(v)); }

void Dag::getLeaves(std::vector<blk_hash_t>& tips) const {
  from_bridge_dag_hashes(rust_graph_->graph->dag_leaves(), tips);
}

bool Dag::addVEEs(blk_hash_t const& new_vertex, blk_hash_t const& pivot, std::vector<blk_hash_t> const& tips) {
  if (new_vertex.isZero()) {
    throw std::invalid_argument("Dag::addVEEs requires a nonzero vertex hash");
  }

  return rust_graph_->graph->dag_add_vertex_edges(to_bridge_hash(new_vertex), to_bridge_hash(pivot),
                                                  to_bridge_dag_hashes(tips));
}

void Dag::drawGraph(std::string const& filename) const {
  std::ofstream outfile(filename.c_str());
  outfile << std::string(rust_graph_->graph->dag_graphviz_dot());
  std::cout << "Dot file " << filename << " generated!" << std::endl;
  std::cout << "Use \"dot -Tpdf <dot file> -o <pdf file>\" to generate pdf file" << std::endl;
}

bool Dag::computeOrder(const blk_hash_t& anchor, std::vector<blk_hash_t>& ordered_period_vertices,
                       const std::map<uint64_t, std::unordered_set<blk_hash_t>>& non_finalized_blks) {
  auto order =
      rust_graph_->graph->dag_compute_order(to_bridge_hash(anchor), to_bridge_non_finalized_blocks(non_finalized_blks));
  if (!order.found) {
    return false;
  }
  ordered_period_vertices.clear();
  from_bridge_dag_hashes(order.hashes, ordered_period_vertices);
  return true;
}

void Dag::clear() { rust_graph_->graph->dag_clear(); }

bool Dag::reachable(vertex_t const&, vertex_t const&) const { throw_unimplemented_dag_api("reachable"); }

void Dag::collectLeafVertices(std::vector<vertex_t>&) const { throw_unimplemented_dag_api("collectLeafVertices"); }

std::vector<blk_hash_t> PivotTree::getGhostPath(const blk_hash_t& vertex) const {
  std::vector<blk_hash_t> pivot_chain;
  from_bridge_dag_hashes(rust_graph_->graph->dag_ghost_path(to_bridge_hash(vertex)), pivot_chain);
  return pivot_chain;
}

}  // namespace taraxa
