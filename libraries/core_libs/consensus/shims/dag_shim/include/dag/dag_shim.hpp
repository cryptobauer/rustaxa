#pragma once

#include <cstdint>
#include <map>
#include <memory>
#include <string>
#include <unordered_set>
#include <vector>

#include "common/types.hpp"

namespace taraxa {

/** @addtogroup DAG
 * @{
 */
class DagManager;
class Network;

/**
 * Rust-mode DAG graph facade.
 *
 * This class preserves the public C++ `Dag` API while routing graph storage, reachability-sensitive ordering, leaves,
 * and Graphviz output to Rust. It is a standalone facade with no legacy implementation dependency.
 *
 * Invariants:
 * - the constructor creates a Rust graph containing the nonzero DAG genesis vertex
 * - public graph methods operate on canonical `blk_hash_t` values and preserve the legacy C++ return contracts
 * - copy operations throw instead of cloning the Rust graph implicitly
 */
class Dag {
 public:
  friend DagManager;

  /**
   * Creates a Rust-backed DAG graph with the supplied genesis hash.
   *
   * `node_addr` is accepted for C++ API compatibility but is not used by the Rust graph. Throws `std::invalid_argument`
   * when the genesis hash is zero and may throw if the bridge cannot allocate the Rust graph.
   */
  explicit Dag(blk_hash_t const& dag_genesis_block_hash, [[maybe_unused]] addr_t node_addr);
  virtual ~Dag();

  /**
   * Copy construction is deliberately unsupported.
   *
   * The Rust graph is owned uniquely by this facade; callers that attempt to copy receive `std::logic_error` instead of
   * silently falling back to legacy C++ graph copying.
   */
  Dag(const Dag&);

  /**
   * Moves the Rust graph holder without changing graph contents.
   */
  Dag(Dag&&) noexcept;

  /**
   * Copy assignment is deliberately unsupported.
   *
   * Self-assignment is a no-op; all other copy assignments throw `std::logic_error`.
   */
  Dag& operator=(const Dag&);

  /**
   * Moves the Rust graph holder without changing graph contents.
   */
  Dag& operator=(Dag&&) noexcept;

  /**
   * Returns the current vertex count reported by the Rust graph.
   */
  uint64_t getNumVertices() const;

  /**
   * Returns the current directed edge count reported by the Rust graph.
   */
  uint64_t getNumEdges() const;

  /**
   * Returns true when `v` is present in the Rust graph.
   */
  bool hasVertex(blk_hash_t const& v) const;

  /**
   * Adds `new_vertex` and directed edges to its pivot and tips.
   *
   * Returns false when the graph rejects the insertion, matching the legacy API. Throws `std::invalid_argument` before
   * crossing the Rust bridge when `new_vertex` is zero.
   */
  bool addVEEs(blk_hash_t const& new_vertex, blk_hash_t const& pivot, std::vector<blk_hash_t> const& tips);

  /**
   * Appends the current Rust graph leaves to `tips`.
   */
  void getLeaves(std::vector<blk_hash_t>& tips) const;

  /**
   * Writes the Rust graph in Graphviz DOT form to `filename`.
   */
  void drawGraph(std::string const& filename) const;

  /**
   * Computes deterministic DAG order for `anchor` over the supplied non-finalized block set.
   *
   * On success, clears and fills `ordered_period_vertices` and returns true. Returns false when `anchor` is missing.
   */
  bool computeOrder(const blk_hash_t& anchor, std::vector<blk_hash_t>& ordered_period_vertices,
                    const std::map<uint64_t, std::unordered_set<blk_hash_t>>& non_finalized_blks);

  /**
   * Clears all Rust graph state.
   */
  void clear();

 protected:
  struct RustDagGraphHolder;
  std::unique_ptr<RustDagGraphHolder> rust_graph_;
};

/**
 * Rust-mode pivot tree facade.
 *
 * PivotTree shares the Rust DAG graph machinery with Dag but exposes the ghost-path query used by DAG/PBFT
 * orchestration. It keeps the C++ inheritance relationship with `Dag` while using the same Rust graph holder.
 */
class PivotTree : public Dag {
 public:
  friend DagManager;
  /**
   * Creates a Rust-backed pivot tree with the supplied DAG genesis hash.
   */
  explicit PivotTree(blk_hash_t const& dag_genesis_block_hash, [[maybe_unused]] addr_t node_addr)
      : Dag(dag_genesis_block_hash, node_addr) {}
  virtual ~PivotTree() = default;

  /**
   * Copy construction delegates to `Dag` copy construction and therefore throws.
   */
  PivotTree(const PivotTree&) = default;

  /**
   * Moves the Rust graph holder without changing graph contents.
   */
  PivotTree(PivotTree&&) = default;

  /**
   * Copy assignment delegates to `Dag` copy assignment and therefore throws for non-self assignment.
   */
  PivotTree& operator=(const PivotTree&) = default;

  /**
   * Moves the Rust graph holder without changing graph contents.
   */
  PivotTree& operator=(PivotTree&&) = default;

  /**
   * Returns the Rust ghost path starting at `vertex`.
   *
   * The returned vector is empty when `vertex` is not present or has no ghost path, matching the legacy public API.
   */
  std::vector<blk_hash_t> getGhostPath(const blk_hash_t& vertex) const;
};

class DagBuffer;
class KeyManager;

/** @}*/

}  // namespace taraxa
