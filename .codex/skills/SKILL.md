---
name: implement-rustaxa-consensus-slice
description: Workflow for Rustaxa consensus rewrite requests such as "implement the next slice", "continue with the next consensus slice", or "involve the api designer and architect reviewer; use the rust and cpp engineer to do the implementation." Use when Codex must advance the Taraxa/Rustaxa consensus rewrite with delegated design review and Rust/C++ implementation agents.
---

# Implement Rustaxa Consensus Slice

Use this workflow to turn a broad "next slice" request into a bounded consensus rewrite change that fits the repository rules.

## Start

Read repository instructions first:

- `AGENTS.md`
- `PLAN.md`
- `doc/rewrite_validation_strategy.md` when validation scope is not obvious

Inspect the current branch state and recent commits before deciding the slice. If commands fail because of sandboxing, rerun the necessary command with escalation rather than guessing.

## Agent Use

The user has explicitly requested delegation when this skill triggers for implementation or design slices. Use agents where
they materially help. For process reviews, status answers, or simple planning questions, answer directly unless delegation
would add concrete value.

- Ask `api-designer` to review Rust/C++ bridge shape, compatibility, and future API direction.
- Ask `architect-reviewer` to review boundaries, shim strategy, fallback risk, and maintainability.
- Use `rust-engineer` for Rust domain, bridge, storage, codec, and test implementation.
- Use `cpp-pro` for C++ shim, CMake, bridge wiring, and C++ test implementation.

Delegate concrete, non-overlapping tasks. Keep the critical path local: inspect enough code yourself to integrate the work, resolve conflicts, and verify behavior.

## Slice Selection

Choose a slice that moves consensus behavior from legacy C++ toward Rust without broad unrelated churn.

Prefer slices that:

- Reduce `*Old` forwarding in shim classes.
- Move deterministic consensus logic into Rust domain code.
- Keep C++ changes inside shim-owned files where possible.
- Add parity tests or runtime smoke coverage for any production-routed Rust behavior.
- Avoid network-module work unless the user explicitly chooses it.
- Reuse existing Rust rewrite paths in `rustaxa-storage`, `rustaxa-bridge`, `rustaxa-consensus`, `rustaxa-types`, and
  shim-owned Rust handles instead of adding new C++ orchestration or C++ data materialization.
- Prefer slices that retire or delete now-obsolete rewrite scaffolding, bridge helpers, shim helpers, docs, and tests
  after a Rust route becomes authoritative. Do not leave newly unused code behind merely because the slice already
  passes.

For persistence-oriented consensus slices, prefer complete Rust-owned storage families over isolated helper migration. A
valid storage slice should define the read/write/reload boundary, batch ownership, restart behavior, and C++ sidecar
materialization that remains temporary. Examples of good storage-family boundaries include PBFT vote persistence,
proposed-block persistence, period-data/finalization persistence, and transaction finalized-status persistence.

For consensus pipeline or manager slices, prefer canonical bytes, compact facts, side-effect-free transitions/planners,
and collected side-effect intents. Avoid designs that require eager C++ object materialization when Rust can operate on
canonical RLP, stable hashes, scalar facts, or already-migrated Rust sidecars.

Do not fix original upstream C++ bugs on `main`. Track divergences and implement the corrected behavior in the rewrite path.

## Implementation Rules

Follow the repository rewrite rules strictly:

- Before settling on a design, inspect nearby Rust crates, bridge APIs, shim-owned handles, and already migrated
  storage/FinalChain/DAG/transaction/vote functionality. Prefer extending those Rust implementations, even when it makes
  the slice moderately larger, if it reduces C++ ownership.
- Use full shim classes for upstream-owned C++ classes.
- Call Rust when implemented; otherwise prefer explicit shim-local throws/stubs.
- Forward to `*Old` only as documented temporary parity scaffolding, with a TODO at every call site.
- Never silently route Rust-enabled production behavior through legacy C++.
- Do not weaken or retarget existing tests to make Rust mode pass.
- Prefer Rust implementation over adding new C++ logic when missing behavior can live in Rust.
- Use `anyhow` for Rust error handling.
- Document new or changed modules, public types, and public functions as complete units.
- Actively delete or simplify code made unused by the slice. After routing behavior to Rust, search for obsolete bridge
  structs/functions, shim-local helpers, temporary C++ payload materialization, stale tests, and roadmap text introduced
  by earlier rewrite slices. Remove them in the same slice when they are no longer needed for parity, restart/reload, or
  public API compatibility. If compatibility code must remain, document the temporary debt at the call site or tracker.

For storage work, Rust must own the full atomic write group for the migrated operation. Do not split one logical commit
across unrelated C++ and Rust batches unless the split is explicitly documented as temporary debt. Keep `state_db/`
distinct from `DbStorage`; it is a sibling database used by FinalChain state execution, not a `DbStorage` column family.

## Verification

Run the narrowest validation tier that covers the change. At minimum for Rust consensus/bridge work, consider:

```bash
cargo fmt --manifest-path rust/Cargo.toml --all --check
cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge
cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge
cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus
git diff --check
```

For C++ shim changes, build and run the affected targets under `/build`, such as `dag_test`, `dag_shim_test`, `rust_consensus_tests`, or more specific targets named by the touched subsystem.

For storage changes, always run:

```bash
cmake --build /build --target rust_storage_tests
/build/bin/rust_storage_tests
```

Also run affected C++ tests. For larger storage refactors or serialization/key-layout changes, ask the task owner before
running:

```bash
scripts/storage_conformance_diff.sh
```

If `check-static` is relevant, remember it is repo-wide and may fail on pre-existing findings. Prefer targeted validation for routine slices unless the change is broad enough to justify the full gate.

## Closeout

Before final response:

- Integrate and review any agent changes.
- Confirm no accidental original C++ edits were made outside allowed shim/guard patterns.
- Confirm docs or trackers are updated when the slice changes roadmap status.
- Confirm newly obsolete rewrite-owned code was removed or explicitly documented as temporary compatibility debt. Use
  targeted searches for replaced bridge APIs, shim helpers, payload builders, duplicated tests, and TODO scaffolding
  before closeout.
- Commit when the user asked for a commit, keeping docs and implementation separate if requested.
- Report what changed, what passed, and any remaining consensus rewrite gap that should be the next slice.
- For storage slices, report which storage family moved, which Rust batch owns the commit, what restart/reload path was
  validated, and which C++ sidecars still remain.
