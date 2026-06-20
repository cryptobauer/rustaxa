# Rewrite Validation Strategy

This document is the source of truth for validating Rust rewrite work. Use it with the validation matrix in
`PLAN.md` and the repeatable Makefile targets in the repository root.

The default rule for existing and new rewrites is:

- deterministic behavior needs C++ vs Rust parity validation before production routing
- runtime-facing rewrites also need a Rust-enabled smoke test or subsystem test
- existing rewrite gaps are not retroactive blockers, but the gap must be documented and backfilled when the area is
  touched

## Validation Tiers

### Tier 1: Fast Per-Change Gate

Use this tier for narrow Rust changes and as the base for broader rewrite work.

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
git diff --check
```

### Tier 2: Subsystem Gate

Use this tier when a rewrite changes a deterministic subsystem boundary, a C++ shim route, or runtime-facing behavior.

Storage:

```bash
make rewrite-validate-storage
```

This builds and runs `rust_storage_tests`, then runs the C++ vs Rust storage conformance diff. The conformance diff is
expensive, so coordinate before running it during large refactors.

Consensus:

```bash
make rewrite-validate-consensus
```

This runs the fast Rust gate, builds targeted consensus tests, and runs the tests that are available after the build:
`dag_test`, `dag_block_test`, `pbft_chain_test`, `pbft_manager_test`, `vote_test`, `pillar_chain_test`, and
`rewards_stats_test`.

FinalChain:

```bash
make rewrite-validate-final-chain
```

This runs the fast Rust gate, builds targeted FinalChain/runtime tests, runs `final_chain_test`, `state_api_test`, and
`rpc_test` when available, then runs the Rust-enabled startup smoke target.

Runtime smoke:

```bash
make rewrite-validate-smoke
```

This builds `taraxad` and runs `taraxad --version` as the smallest non-destructive startup/link check available in the
current repository conventions.

VDF:

- Run Rust workspace tests for crate-level behavior.
- Run the C++ bridge tests when bridge payloads or C++ routing changes.
- Add parity fixtures before routing externally visible deterministic VDF behavior through Rust.

### Tier 3: Expensive Pre-Merge Gate

Use this tier for broad, shared, production-routing, or high-risk rewrite changes.

- Full CTest:

  ```bash
  cd /build/tests && ctest --output-on-failure
  ```

- Python integration tests when RPC, node sync, finalization, consensus runtime, or node startup behavior changes:

  ```bash
  cd tests/py && ./run.sh -s --tb=short
  ```

- Pure C++ validation on `cpp-reference` for upstream sync work or C++ intersection changes.

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
- a Rust-enabled smoke or subsystem validation when the code runs during node startup, sync, consensus, finalization, or
  RPC handling

C++ tests may be disabled, removed, or retargeted when they block retiring legacy C++ behavior, old object
materialization, or shim scaffolding. That is acceptable only after equivalent or stronger Rust module coverage exists for
the behavior that moved. If parity depends on the CXX bridge, bridge-level Rust tests or focused Rust-enabled shim tests
must replace the lost C++ signal. Closeout notes or tracker updates must state why the old C++ test no longer describes
target Rust-mode behavior.

Temporary Rust shim defaults must be tracked and tested as explicit temporary behavior. They should not be hidden by
delegation to legacy C++ implementation paths.

For storage-retirement work, `make rewrite-storage-boundary-guard` is a regression guard for newly added Rust-mode C++
storage routes. A passing guard means the current diff did not introduce unreviewed `DbStorage`, `db_`, C++ batch,
`rustStorage`, or `rustBatchId` usage outside allowlisted compatibility areas. It does not mean all pre-existing
`BridgeStorage` or `DbStorage` routes have been eliminated; those removals must be tracked in `PLAN.md` and verified by
targeted call-site searches.

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
- Tier 3 is required for broad runtime behavior, production routing, upstream sync, and changes that cross subsystem
  boundaries.

When in doubt, document the residual validation gap in the closeout notes and run the next broader tier.
