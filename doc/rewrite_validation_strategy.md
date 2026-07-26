# Rewrite Validation Strategy

This document is the source of truth for validating Rust rewrite work. Use it with the subsystem-specific
**Validation Matrix** in `doc/consensus_rewrite_tracker.md` and the repeatable Makefile targets in the repository root.

The default rule for existing and new rewrites is:

- deterministic behavior needs C++ vs Rust parity validation before production routing
- runtime-facing rewrites also need a Rust-enabled smoke test or subsystem test
- existing rewrite gaps are not retroactive blockers, but the gap must be documented and backfilled when the area is
  touched

## Validation Tiers

### Tier 1: Fast Per-Change Gate

Run this tier for every implementation slice. It is sufficient by itself only for narrow Rust changes that do not alter
production authority, subsystem state, a bridge/shim route, or runtime-facing behavior; use it as the base for every
broader tier.

- Rust formatting, linting, and tests for Rust changes:

  ```bash
  make rewrite-validate-fast
  ```

- `git diff --check` for whitespace and conflict-marker mistakes.
- Affected C++ unit tests when touched C++ behavior has user-visible or deterministic effects.

`make rewrite-validate-fast` runs:

```bash
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo clippy --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml
make rewrite-storage-boundary-guard
make rewrite-bridge-inventory-guard
git diff --check
```

The bridge inventory guard checks live CXX handles, bridge modules, shim directories, and their audit rows. Like the
storage boundary guard, it is a structural regression check rather than proof of behavioral parity or successful
production routing.

### Tier 2: Subsystem Gate

Use this tier when a rewrite changes a deterministic subsystem boundary, a C++ shim route, or runtime-facing behavior.

Before reporting a Rust-enabled CMake gate, inspect `/build/CMakeCache.txt` (or the selected `BUILD_OUTPUT_DIR`) and
verify `RUSTAXA_ENABLE:BOOL=ON` plus every module option traversed by the tested route. Prefer `make configure` for the
supported production feature bundle. The current Make targets only warn when the master option is disabled and do not
enforce module-specific options, so target success alone does not prove that the intended Rust implementation ran.
Record the relevant cache options with the validation evidence.

Storage:

```bash
make rewrite-validate-storage
```

This builds and runs `rust_storage_tests`. Escalate larger or broad storage changes to the Tier 3 storage conformance
target below.

Consensus:

```bash
make rewrite-validate-consensus
```

This runs the fast Rust gate and the current `REWRITE_CONSENSUS_TESTS` inventory from the Makefile:
`rust_consensus_tests`, `dag_test`, `dag_block_test`, `pbft_chain_test`, `pbft_chain_shim_test`,
`proposed_blocks_shim_test`, `pbft_manager_test`, `vote_test`, `pillar_chain_test`, and `rewards_stats_test`.

The target tolerates CMake targets that are unavailable in the configured build. A skipped test that the tracker
Validation Matrix or the changed boundary requires is a validation gap and does not satisfy Tier 2. Reconfigure the
build, run the required test directly, or report the unresolved gap; do not treat the aggregate target's zero exit status
as sufficient by itself.

FinalChain:

```bash
make rewrite-validate-final-chain
```

This runs the fast Rust gate, builds targeted FinalChain/runtime tests, runs `final_chain_test`, `state_api_test`, and
`rpc_test` when available, then runs the Rust-enabled binary link/CLI smoke target. Before treating this as Rust
FinalChain evidence, require at minimum `RUSTAXA_ENABLE`, `RUSTAXA_ENABLE_STORAGE`, and
`RUSTAXA_ENABLE_FINAL_CHAIN` to be `ON`, plus any other module option exercised by the changed route.

As with the consensus target, a skipped test required by the changed boundary is a validation gap. The target must not
be reported as complete Tier 2 evidence unless all required tests actually ran.

Binary link/CLI smoke:

```bash
make rewrite-validate-smoke
```

This builds `taraxad` and runs `taraxad --version`. It proves binary loading, linkage, and basic CLI handling only. It
does not initialize storage, initialize or start `App`, restore native services, or prove node startup. For
startup-facing changes, require an actual bounded startup/subsystem test or the applicable full-node/Python integration
coverage in addition to this smoke.

VDF:

- Run Rust workspace tests for crate-level behavior.
- Run the C++ bridge tests when bridge payloads or C++ routing changes.
- Add parity fixtures before routing externally visible deterministic VDF behavior through Rust.

Network and transport:

- For message inspection, admission, dispatch, queueing, peer decisions, or typed egress effects, run Rust unit tests for
  every affected packet kind plus canonical-byte/golden-vector or focused C++ boundary parity.
- Run `make rewrite-validate-consensus` when a network route feeds consensus state or effects.
- Exercise the affected tarcap handler/transport boundary and effect ordering without moving physical socket, peer, or
  packet-wrapping mechanics into consensus tests.
- Treat application-owned network pipeline routing as production authority: require Tier 3 full-node/CTest and Python
  integration coverage for affected sync, gossip, disconnect, or finalization behavior.

Execution orchestration:

- For FinalChain/EVM request planning, canonical payloads, result validation, commit ordering, recovery, or publication,
  run Rust unit tests, `make rewrite-validate-final-chain`, and focused `state_api_test`/`final_chain_test` coverage.
- Preserve concrete EVM and `state_db/` execution as a leaf boundary unless the roadmap explicitly changes it. Validate
  CXX request/result conversion and error mapping separately from native orchestration behavior.
- Treat an execution-authority cutover as Tier 3. Use current-source parity where deterministic legacy behavior is
  claimed, plus full-node or Python integration coverage when finalization, receipts, recovery, or RPC behavior changes.

Bridge and shim contraction:

- Run the fast gate so both structural guards execute, then run every subsystem gate affected by migrated callers.
- Prove the last production caller moved and that every retained adapter has a named client, classification, and deletion
  condition in `doc/consensus_bridge_shim_audit.md`.
- Record measured changes to bridge/shim lines, CXX functions and carriers, handles, shim directories, flags, partial
  factories, compatibility constructors, and production/test callers. A passing inventory guard proves inventory
  consistency only; it does not prove the removed facade's behavior was replaced.
- When compatibility-only tests are removed, identify the native behavior tests and focused ABI/conversion tests that
  replace them.
- When module flags, source selection, overlays, constructors, or upstream-owned C++ intersections change, validate both
  the Rust-enabled route and the all-Rust-disabled pure-C++ route.

### Tier 3: Expensive Production-Authority and Pre-Merge Gate

Use this tier for every production-authority cutover and for broad, shared, cross-subsystem, upstream-sync, C++
intersection, or otherwise high-risk rewrite changes.

- Full CTest:

  ```bash
  cd /build/tests && ctest --output-on-failure
  ```

- Python integration tests when RPC, node sync, finalization, consensus runtime, or node startup behavior changes:

  ```bash
  cd tests/py && ./run.sh -s --tb=short
  ```

- C++/Rust storage differential when storage behavior changes broadly or a larger storage refactor requires full
  conformance coverage:

  ```bash
  make rewrite-validate-storage-conformance
  ```

  This target runs the Tier 2 storage bridge tests first, then `scripts/storage_conformance_diff.sh`. The differential is
  a regular Tier 3 gate. A task owner's standing authorization to run warranted Tier 3 validation includes this script;
  no separate script-specific approval is required. Without standing Tier 3 authorization, coordinate before running
  it as required by the repository guidelines.

- Current-source FinalChain C++/Rust parity for any DPoS, slashing, receipt, or persisted-state method family that claims
  legacy parity:

  ```bash
  make rewrite-validate-final-chain-parity
  ```

  This composite gate runs the Tier 2 FinalChain target first, then configures the same source tree in an isolated
  all-Rustaxa-disabled pure-C++ build, builds `final_chain_test` with 12 jobs, and runs all focused
  `FinalChainTest.native_dpos_*` fixtures followed by the complete suite. The reusable pure-C++ build defaults to
  `/tmp/rustaxa-final-chain-pure-cpp`; override `FINAL_CHAIN_CPP_BUILD_ROOT` only with another isolated absolute path.
  This is the regular Tier 3 differential for current-source FinalChain method, receipt, and persisted-state parity
  claims. Standing Tier 3 authorization includes the underlying script, so do not request separate approval when this
  gate is required. The script enforces the pure-C++ leg, but the caller must first verify that the Tier 2 build has
  `RUSTAXA_ENABLE`, `RUSTAXA_ENABLE_STORAGE`, and `RUSTAXA_ENABLE_FINAL_CHAIN` set to `ON`; otherwise the result may be
  C++ compared with C++ and must not be reported as Rust/C++ parity.

- Pure C++ validation on `cpp-reference` for upstream sync work or C++ intersection changes. Use the repository
  intersection helpers to identify and carry the smallest applicable upstream-owned change, verify the staged patch,
  and run the required all-Rust-disabled build and tests on `cpp-reference`. A Rust-enabled result on `main` does not
  substitute for this gate.

## Correctness Rules

Differential parity is the default for deterministic rewrite surfaces. This includes:

- storage reads and writes
- codecs and canonical byte encodings
- DAG ordering
- vote aggregation
- PBFT chain state
- transaction ordering
- rewards calculations
- pillar encoding

New Rust production routing requires:

- Rust unit coverage for the moved logic
- a C++ vs Rust parity fixture, transcript, or conformance check for externally visible deterministic behavior
- a Rust-enabled subsystem test plus an actual bounded startup/full-node or Python integration test when the code runs
  during node startup, sync, consensus, finalization, or RPC handling; the `taraxad --version` link/CLI smoke is not
  startup evidence

C++ tests may be disabled, removed, or retargeted when they block retiring legacy C++ behavior, old object
materialization, or shim scaffolding. That is acceptable only after equivalent or stronger Rust module coverage exists for
the behavior that moved. If parity depends on the CXX bridge, bridge-level Rust tests or focused Rust-enabled shim tests
must replace the lost C++ signal. Closeout notes or tracker updates must state why the old C++ test no longer describes
target Rust-mode behavior.

Tests do not justify retaining a production CXX export, compatibility constructor, partial-service factory, or
manager-shaped shim. Move behavioral coverage to the native owner and retain only focused ABI, conversion, lifetime,
error-mapping, or explicitly allowlisted conformance coverage at the bridge. Validate deletion by combining last-caller
searches, bridge inventory checks, feature-on subsystem tests, and the pure-C++ route when source selection changes.

For consensus work, unresolved work and parity gaps live in the **Remaining Consensus Work Queue**, while
subsystem-specific minimum suites live in the **Validation Matrix** of `doc/consensus_rewrite_tracker.md`. Detailed
edge cases should live beside the owning Rust
module tests, bridge fixtures, or focused C++ suite. Do not preserve a separate pre-routing checklist after a route has
landed; when a touched behavior still lacks a reusable parity fixture, record that concrete gap in the tracker and add
the fixture before changing production authority.

Temporary Rust shim defaults must be tracked and tested as explicit temporary behavior. They should not be hidden by
delegation to legacy C++ implementation paths.

For storage-retirement work, `make rewrite-storage-boundary-guard` is a regression guard for newly added Rust-mode C++
storage routes. A passing guard means the current diff did not introduce unreviewed `DbStorage`, `db_`, C++ batch,
`rustStorage`, or `rustBatchId` usage outside allowlisted compatibility areas. It does not mean all pre-existing
`BridgeStorage` or `DbStorage` routes have been eliminated; actionable removal work and debt belong in
`doc/consensus_rewrite_tracker.md`, live classifications and deletion conditions belong in
`doc/consensus_bridge_shim_audit.md`, and elimination must be verified by targeted call-site searches.

For post-migration consensus storage cleanup, use the guard plus targeted audits for `DbStorage`, `db_->`, `getDB()`,
`rustStorage`, `createWriteBatch`, `commitWriteBatch`, `rustBatchId`, bridge-batch appenders, and direct FinalChain
DPoS/account fact reads from consensus consumers. Remaining Rust-mode C++ storage references must be classified as one
of: storage-shim internals and tests, marked query compatibility, marked network/tarcap compatibility, FinalChain/EVM
boundary work, temporary sidecar/API materialization, or app/admin lifecycle behavior. Unclassified production consensus
references are rewrite blockers and should move to Rust-owned storage runtimes before closeout.

## Choosing The Narrowest Tier

Before closing rewrite work, choose the narrowest validation tier that covers the behavior changed:

- Tier 1 is enough for local Rust-only model, helper, or codec changes that do not route production behavior.
- Tier 2 is required for subsystem state changes, bridge/shim routing changes, and deterministic behavior that can be
  compared to C++.
- Tier 3 is required for every production-authority cutover, broad runtime behavior, upstream sync, C++ intersection
  changes, and changes that cross subsystem boundaries.

When in doubt, document the residual validation gap in the closeout notes and run the next broader tier.
