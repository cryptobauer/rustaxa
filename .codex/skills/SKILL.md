---
name: implement-rustaxa-consensus-slice
description: Select, design, implement, validate, and close out a bounded Rustaxa consensus rewrite slice with required code mapping, boundary-appropriate API, architecture, Rust, and C++ delegation, and independent review. Use for requests such as "implement the next consensus slice", "continue the consensus rewrite", audit a proposed slice, remove remaining C++ consensus authority, or advance Rust consensus ownership.
---

# Implement Rustaxa Consensus Slice

Turn a broad consensus rewrite request into a bounded change that advances the current roadmap without recreating completed work.

## Start

Read these repository sources before selecting a slice:

- `AGENTS.md`
- `PLAN.md`
- `doc/consensus_rewrite_tracker.md`
- `doc/rewrite_validation_strategy.md`

Inspect the current branch, worktree, recent commits, and relevant implementation. Preserve unrelated user changes. If an
inspection command fails, diagnose it and use an available safe alternative rather than guessing.

## Agent Use

Delegate when the user explicitly requests agents, repository instructions require them, or independent workstreams will
materially improve a non-trivial design or implementation slice. Do not spawn agents merely because the skill triggered.
Handle process reviews, status answers, and simple planning locally unless delegation adds concrete value.

The profile TOML files are the source of truth for agent names, instructions, models, reasoning, permissions, skills, and
MCP configuration. Prefer native profile binding and verify the resolved identity. If the runtime cannot select or verify
a profile, follow the fallback-emulation policy in `AGENTS.md`: reproduce the selected profile's
`developer_instructions` verbatim, reproduce other settings wherever supported, state any gaps, and label the child
`<name> fallback emulation`. Never claim that fallback emulation is verified profile use.

- Use `task-distributor` once for a broad multi-slice goal that needs dependency-aware decomposition.
- Use `code-mapper` when ownership, call paths, state flow, or remaining compatibility boundaries are unclear.
- Ask `api-designer` to review Rust/C++ bridge shape, compatibility, and future API direction.
- Ask `architect-reviewer` to review boundaries, shim strategy, fallback risk, and maintainability.
- Use `rust-engineer` for Rust domain, bridge, storage, codec, and test implementation.
- Use `cpp-pro` for C++ shim, CMake, bridge wiring, and C++ test implementation.
- Use `reviewer` for an independent correctness, regression, and test-coverage review after a non-trivial slice and
  before committing it.
- Use `blockchain-engineer` only for explicitly scoped EVM, contract-call, signing, gas, slashing-transaction, or
  on-chain transaction-lifecycle work.

Assign concrete, non-overlapping tasks and name the expected artifact or decision. Do not delegate the same discovery or
implementation scope to multiple agents unless they are intentionally providing independent reviews. Keep the critical
path local: inspect enough code yourself to integrate the work, resolve conflicts, review every change, and verify
behavior.

## Required Orchestration

For every non-trivial implementation slice:

1. Use `code-mapper` to inspect the relevant ownership, state flow, existing Rust paths, bridge and shim boundaries,
   tests, and compatibility debt. Give it a bounded read-only assignment and require file/symbol evidence, risks, reuse
   opportunities, and unresolved questions. Wait for its report before finalizing the design. Reuse a still-current report
   during review-fix iterations unless the implementation materially changes the researched boundary.
2. Use `api-designer` when the slice changes a Rust/C++ contract or public compatibility surface, and
   `architect-reviewer` when it changes ownership, dependency direction, persistence boundaries, fallback behavior, or
   maintainability. Wait for required design reports before implementation.
3. Give the research and design reports to `rust-engineer` for bounded Rust domain, bridge, storage, codec, and Rust-test
   work. Use `cpp-pro` for bounded C++ shim, CMake, bridge-wiring, and C++-test work. They may run concurrently only when
   ownership does not overlap. Wait for their reports, inspect the changes locally, and integrate them in the parent.
4. After integration and targeted validation, use `reviewer` in a fresh independent read-only context to review the
   complete intended slice diff. Require exactly one disposition: `APPROVED`, or `CHANGES_REQUESTED` with prioritized,
   evidence-backed findings and missing validation.
5. Return accepted findings to the appropriate `rust-engineer` or `cpp-pro`, integrate the fixes, rerun affected
   validation, and request another complete review. Repeat until the reviewer returns `APPROVED`. Do not commit first.
6. Use `blockchain-engineer` in this sequence only when the task explicitly crosses an EVM, contract, signing, gas,
   slashing-transaction, or on-chain transaction-lifecycle boundary.

Use `task-distributor` once at the beginning of a broad multi-slice goal to produce dependency-aware decomposition. The
parent owns slice selection, integration, validation, commits, and goal state throughout the workflow.

Spawn every child with no inherited conversation turns, or the smallest supported context, and provide only its bounded
assignment, required source paths, prior reports, invariants, and return format. A child must not create, update, complete,
block, resume, or otherwise operate on the parent `/goal`; it must finish only the delegated assignment.

## Slice Selection

Audit the roadmap and implementation before proposing work. The non-network/non-EVM native consensus closeout and PBFT
manager protocol-runtime boundary are currently complete. Do not recreate completed ownership slices or treat accepted
executor and compatibility boundaries as unfinished consensus logic.

Choose a slice only when it does at least one of the following:

- Closes a demonstrated regression against the consensus closeout definition.
- Moves decision authority from an unclassified C++ path into an existing Rust runtime or typed port.
- Removes obsolete compatibility materialization, bridge surface, shim scaffolding, or duplicated state.
- Adds genuinely new consensus behavior in Rust while preserving established executor boundaries.
- Implements network/tarcap pipeline or EVM/FinalChain execution work explicitly put in scope by the task owner.

Prefer slices that:

- Reduce documented `*Old` parity scaffolding without moving authority back into C++.
- Express deterministic consensus rules as Rust planners over explicit facts and borrowed state views.
- Keep C++ changes inside shim-owned files.
- Reuse `rustaxa-storage`, `rustaxa-bridge`, `rustaxa-consensus`, `rustaxa-types`, typed ports, and shim-owned Rust handles.
- Preserve canonical bytes, decode late, avoid eager C++ object materialization, and return ordered typed effects.
- Delete rewrite-owned helpers, sidecars, tests, and documentation made obsolete by the authoritative Rust route.

Keep network/tarcap transport and EVM/state execution outside consensus-manager ownership unless explicitly re-scoped.
Do not fix original upstream C++ bugs on `main`; track the divergence and implement the corrected rewrite behavior in Rust.

The migrated production consensus storage families are already Rust-owned. Select persistence work only after finding a
real unclassified route or new operation. Migrate the complete operation: read/write/reload boundary, Rust-owned atomic
batch, restart and duplicate behavior, and any temporary C++ sidecar materialization.

## Implementation Rules

- Inspect adjacent Rust crates, bridge APIs, shim-owned handles, and migrated storage/FinalChain/DAG/transaction/vote
  functionality before settling on a design. Extend those paths when doing so reduces C++ ownership.
- Use a full overlay shim as the first design for upstream-owned C++ classes. Use the accepted protected-state inheritance
  hook only with explicit task-owner approval, named migration-debt TODOs, shim-owned public forwarding methods, no Rust
  bridge surface in the original class, and a documented upstream-file diff at closeout.
- Call Rust when implemented; otherwise use explicit shim-local throws, stubs, or no-ops.
- Forward to `*Old` only as temporary parity scaffolding, with a TODO naming the remaining Rust work at every call site.
- Never route Rust-enabled production behavior through legacy C++ by delegation, forwarding, or inherited behavior.
- Prefer Rust implementation over new C++ logic. Treat logging as observability, not a reason to retain C++ authority.
- Use `anyhow` for Rust fallible APIs unless a narrower domain-boundary error is intentionally required.
- Document changed modules, public types, and public functions as complete units: purpose, inputs, outputs, invariants,
  and error or edge behavior.
- Do not weaken or retarget tests to make Rust mode pass. Retire a legacy-only test only after equivalent or stronger Rust
  or bridge coverage exists and the tested compatibility surface is intentionally removed; document the replacement.
- Search for obsolete bridge structs/functions, shim helpers, payload builders, materialization, duplicated tests, and
  TODO scaffolding after routing. Remove them unless required for parity, restart/reload, or public API compatibility;
  document retained debt at the call site or tracker.

For storage work, Rust must own the complete atomic write group. Do not split one logical commit across unrelated C++ and
Rust batches unless the task owner accepts documented temporary debt. Keep `state_db/` distinct from `DbStorage`; it is a
sibling database used by FinalChain state execution, not a `DbStorage` column family.

## Verification

Use `doc/rewrite_validation_strategy.md` as the source of truth and run the narrowest tier that fully covers the change.

For every implementation slice, run Tier 1:

```bash
make rewrite-validate-fast
```

For deterministic subsystem changes, bridge/shim routing, or runtime-facing consensus behavior, run Tier 2:

```bash
make rewrite-validate-consensus
```

Before production-routing deterministic behavior, require Rust unit coverage plus C++/Rust parity through a fixture,
transcript, conformance check, or focused bridge/shim test. For startup, sync, consensus, or finalization paths, also
require Rust-enabled subsystem or smoke coverage.

Use Tier 3 for broad production routing, cross-subsystem behavior, upstream sync, or other high-risk changes. Ask the
task owner before expensive repo-wide or differential gates when repository instructions require coordination. Never
silently downgrade a required tier; report any residual validation gap.

For every storage-module change, also run:

```bash
cmake --build /build --target rust_storage_tests --parallel 12
/build/bin/rust_storage_tests
```

Run affected C++ tests. Coordinate with the task owner before the expensive storage conformance diff when required:

```bash
scripts/storage_conformance_diff.sh
```

Prefer targeted C++ validation for routine changes. Run repo-wide `check-static` only when the change scope or repository
instructions justify it, and distinguish pre-existing findings from regressions.

## Closeout

- Integrate and review all agent changes.
- Record only custom-agent profiles whose resolution was confirmed by the delegation interface or agent metadata. Keep
  intended roles, generic task agents, and local primary-agent work distinct in the closeout report.
- Confirm no accidental original C++ edits exist outside allowed shim or guarded patterns.
- For every touched upstream-owned C++ path, run `git diff upstream-main -- <path>` and require an empty diff or document
  the explicitly approved temporary exception.
- Update `PLAN.md` or `doc/consensus_rewrite_tracker.md` when roadmap status or tracked debt changes.
- Confirm newly obsolete rewrite-owned code was removed or explicitly documented as compatibility debt.
- Run `git diff --check`, review the final diff, and preserve unrelated worktree changes.
- Commit only when requested, using the repository's Conventional Commit rules.
- Report what changed, the exact validation that passed, and only demonstrated remaining gaps. Do not manufacture a next
  consensus slice when the audited boundary is already complete.
- For storage work, report the migrated operation or family, the Rust batch owner, restart/reload coverage, and remaining
  C++ sidecars.
