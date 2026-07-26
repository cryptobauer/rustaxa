---
name: implement-rustaxa-consensus-slice
description: Select, design, implement, validate, and close out a bounded Rustaxa consensus rewrite or bridge/shim contraction slice using the repository's live roadmap, ownership boundaries, deletion inventory, validation policy, specialist-agent routing, and independent review. Use for requests to implement the next consensus slice, continue the rewrite, audit a proposed slice, move application ownership into native Rust, remove C++ consensus authority or compatibility surface, or close a consensus rewrite gap.
---

# Implement Rustaxa Consensus Slice

Turn a broad consensus rewrite request into one bounded, roadmap-directed change without recreating completed work or
preserving obsolete internal C++ architecture.

## Establish Current Authority

Read these sources before selecting or designing a slice:

- `AGENTS.md`
- `PLAN.md`
- `doc/consensus_rewrite_tracker.md`
- `doc/consensus_consolidation_plan.md`
- `doc/consensus_bridge_shim_audit.md`
- `doc/rewrite_validation_strategy.md`

Use them according to their declared ownership:

- `PLAN.md` owns strategy, authorized scope, application boundaries, and retained external executors.
- The tracker is the only execution queue and status/dependency authority.
- The consolidation plan owns active bridge/shim reduction design and sequencing, not implementation history.
- The bridge/shim audit is the mechanically checked inventory of live consumers, classifications, and deletion
  conditions.
- The validation strategy owns reusable validation tiers and parity requirements.

Do not create another consensus gap, compatibility, cleanup, or slice tracker.

If the documents disagree, do not silently choose the wording that permits implementation. Inspect recent commits and
current code, reconcile the authoritative documents as part of the slice when the intended policy is clear, or stop for
a task-owner decision when the disagreement changes scope.

Inspect the current branch, worktree, recent commits, relevant callers, native Rust owners, CXX surface, shims, tests, and
build wiring. Preserve unrelated user changes. Diagnose failed inspection commands and use safe alternatives rather than
guessing.

## Route Specialist Work

Route each material workstream to the configured custom agent whose role matches it. Use only applicable roles and keep
status answers, simple audits, and process-only planning local.

- Use `code-mapper` when ownership, call paths, state flow, or compatibility boundaries are unclear.
- Use `api-designer` to review CXX/API shape, compatibility, and the intended native API.
- Use `architect-reviewer` to review ownership boundaries, adapter strategy, fallback risk, and maintainability.
- Use `rust-engineer` for Rust domain, runtime, port, storage, codec, bridge, and test implementation.
- Use `cpp-pro` for C++ adapters, shims, CMake, bridge wiring, and C++ tests.
- Use `reviewer` for independent correctness, regression, and coverage review of a non-trivial intended diff.
- Use `blockchain-engineer` only for explicitly scoped EVM, contract, signing, gas, slashing-transaction, or on-chain
  transaction-lifecycle work.

Give agents concrete, non-overlapping ownership and name the expected artifact or decision. Keep slice selection,
integration, validation, documentation, commits, and goal state with the parent. Count a route as using a configured role
only when the runtime identifies that role; do not substitute a generic worker when a required configured role cannot be
selected.

For a non-trivial slice, obtain independent review after targeted validation and before commit. Address evidence-backed
findings, rerun affected validation, and repeat review until approved.

## Select a Slice

Continue an applicable active item before selecting unrelated work. Otherwise, start from an item currently marked ready
in the tracker's remaining-work queue. Verify its status, dependencies, authorization, and completion condition against
current code before committing to it. Do not start blocked work until its named dependency or scope decision is
satisfied. Preserve the queue item's stable identifier in tracker updates, but do not encode current item identifiers or
transient statuses into this skill.

Mark an item active when implementation begins. Mark it complete only when its completion condition, required deletion
or ownership result, documentation updates, validation, and review have landed. When an item spans several deployable
slices, keep each slice independently coherent and record concise evidence without turning planning documents into an
implementation diary.

Select a slice only when it produces at least one concrete roadmap result:

- deletes a complete ownership or compatibility family;
- moves application state, construction, restoration, locking, or behavioral tests from the bridge or C++ into a native
  Rust owner;
- moves deterministic decision authority from C++ into a Rust planner, service, or typed port;
- replaces internal legacy object materialization with canonical bytes, opaque identities, borrowed native views, or a
  client-specific boundary type;
- contracts a manager-shaped shim into a named query, transport, execution, lifecycle, signing, VDF, admin, bootstrap,
  or public-client adapter;
- removes obsolete exports, carriers, handles, factories, flags, sidecars, duplicated state, tests, or documentation;
- closes a demonstrated parity or production-routing regression; or
- unblocks a named downstream deletion or ownership cutover.

Merely renaming exports, combining DTOs, moving code between bridge modules, or reclassifying retained surface is not a
sufficient slice.

Prefer work that:

- reduces measured bridge/shim surface and compatibility topology;
- removes an entire last-caller family in the same change;
- expresses deterministic consensus rules as Rust planners over explicit facts and borrowed state views;
- preserves canonical bytes, decodes late, avoids eager C++ materialization, and returns ordered typed effects;
- reuses native Rust storage, consensus, types, codecs, FinalChain, DAG, transaction, vote, and query capabilities; and
- leaves C++ only at named external or operational boundaries authorized by `PLAN.md`.

Treat network ingress/routing and execution orchestration according to the live roadmap rather than assuming they are
permanently out of scope. Physical transport mechanics and concrete EVM/`state_db` operations remain leaf executors
unless the task owner changes that boundary.

Select persistence work only for a demonstrated unclassified route or new operation. Migrate the complete operation,
including reads, writes, atomic batch ownership, reload/restart, duplicate behavior, and temporary sidecar
materialization.

## Design for Native Ownership

- Make a native Rust application/runtime owner the default for internal consensus behavior. Keep `rustaxa-bridge`
  limited to CXX declarations, plain carriers, conversions, error/lifetime mapping, and thin calls into native services.
- Move behavioral tests with their native owner. Retain bridge tests for ABI, conversion, lifetime, error mapping, and
  explicitly allowlisted conformance boundaries.
- Support one production Rust application composition unless the live plan explicitly requires another topology.
  Treat granular rewrite flags, partial-service factories, and compatibility constructors as removable migration
  scaffolding.
- Do not preserve an upstream manager class merely because internal C++ callers or tests still use it. Migrate those
  callers to native application, query, transport, or executor APIs.
- Add or extend an overlay shim only when a named external C++ client cannot use an existing narrow adapter. Record its
  owner and deletion condition. Keep Rust routing and temporary scaffolding in shim-owned files and preserve untouched
  pure-C++ source selection.
- Use an approved temporary upstream hook only with explicit task-owner authorization, named debt, guarded dependencies,
  and closeout evidence against `upstream-main`.
- Never route Rust-enabled production through legacy C++ by forwarding, delegation, or inherited behavior. A missing
  Rust route must be an explicit adapter-local throw, stub, or no-op unless the task owner approves documented parity
  scaffolding.
- Do not fix original upstream C++ bugs on the rewrite branch. Record the divergence and implement intended behavior in
  the Rust-owned path.
- Treat logging, formatting, timers, and events as adapter or observability concerns, not reasons to retain deterministic
  C++ orchestration.
- Use narrow task-oriented Rust ports and explicit domain types. Keep codecs and FFI carriers separate from domain
  models. Use `anyhow` for fallible Rust APIs unless a deliberate domain error belongs at the boundary.
- Document changed modules, public types, and public functions as complete units: purpose, inputs, outputs, invariants,
  and error or edge behavior.

For storage changes, Rust must own each complete logical atomic write group. Do not split authority across unrelated C++
and Rust batches without approved, tracked temporary debt. Keep `state_db/` distinct from `DbStorage`.

## Remove the Old Surface

After routing the authoritative path, search all consumers and delete newly obsolete items together:

- CXX declarations and carriers;
- bridge functions, handles, wrappers, constructors, and partial factories;
- shim methods or entire shim directories;
- module flags and CMake dependency branches;
- payload builders, sidecars, mirrors, compatibility mutexes, and revalidation protocols;
- compatibility-only tests superseded by native behavioral or focused boundary coverage; and
- stale audit rows and implementation-status prose.

Do not retain a surface as “public compatibility” without a named client. For each retained adapter, record its client,
classification, owner, and deletion or narrowing condition in the live inventory.

Do not weaken tests to make Rust mode pass. Remove or retarget a legacy-only test only after equivalent or stronger
native coverage exists, plus bridge/shim coverage when boundary parity depends on CXX. Document the replacement.

## Validate

Use `doc/rewrite_validation_strategy.md` to choose the narrowest tier that fully covers the change. Do not copy temporary
queue-item validation rules into this skill.

Run the fast gate for every implementation slice:

```bash
make rewrite-validate-fast
```

Run every applicable subsystem gate for deterministic, bridge/shim, or runtime-facing changes:

```bash
make rewrite-validate-consensus
make rewrite-validate-final-chain
make rewrite-validate-storage
make rewrite-validate-smoke
```

Run only the gates relevant to the touched boundaries. Escalate to the validation strategy's expensive tier for every
production-authority cutover, cross-subsystem behavior, upstream sync, C++ intersection change, or other high-risk
change. Coordinate before expensive repo-wide or differential gates when repository policy requires it, unless the task
has standing authorization.

Before routing deterministic production behavior, require:

- Rust unit coverage for the moved rule;
- C++/Rust parity through an appropriate fixture, transcript, conformance check, or focused boundary test; and
- Rust-enabled subsystem or smoke coverage for startup, sync, consensus, finalization, execution, or RPC paths.

For storage-module changes, also build and run the Rust storage bridge tests:

```bash
cmake --build /build --target rust_storage_tests --parallel 12
/build/bin/rust_storage_tests
```

Run affected C++ tests. Use the repository's composite parity and conformance targets when the validation strategy
classifies them as required. Never silently downgrade a required tier; report any residual gap.

## Close Out

- Integrate and review all changes, including agent-authored changes.
- Re-run inventory and boundary guards applicable to the touched surface.
- Confirm no accidental original C++ edits exist outside authorized overlay, source-selection, or guarded patterns.
- For every touched upstream-owned C++ path, run `git diff upstream-main -- <path>` and require an empty diff or document
  the explicitly approved exception.
- When a slice changes the C++ intersection, use the repository intersection helpers to identify and carry the relevant
  change to `cpp-reference`, then run the required all-Rust-disabled pure-C++ validation. Treat upstream sync work the
  same way.
- Update the selected tracker item when status, dependencies, completion evidence, or debt changes.
- Update `PLAN.md` only for a strategy, scope, or accepted-boundary change.
- Update the consolidation plan only when active design or sequencing changes.
- Update the bridge/shim audit in the same slice whenever consumers, classifications, deletion conditions, or live
  inventory change.
- Measure and report production callers migrated and lines, functions, carriers, handles, shims, flags, partial
  factories, and compatibility constructors removed. Report zero explicitly where a category was examined but unchanged.
- Record native behavior or parity tests replacing deleted compatibility tests and name every retained boundary client.
- Run `git diff --check`, review the complete final diff, and preserve unrelated worktree changes.
- Commit only when requested, using repository Conventional Commit rules.
- Report exact validation results and only demonstrated remaining gaps. Do not invent a next slice when the audited queue
  has no ready authorized work.

For storage work, also report the migrated operation or family, the Rust batch owner, restart/reload coverage, and any
remaining C++ sidecars.
