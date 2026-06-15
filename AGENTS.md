# Repository Guidelines

This repository is the Rust rewrite track for Taraxa. Keep day-to-day work aligned with the rewrite plan in `PLAN.md`.

## Project Structure

- `libraries/`: C++ modules by domain, including `core_libs/{consensus,network,storage}`, `types/*`, `app`, `cli`, and `metrics`.
- `programs/`: node binaries such as `taraxad` and `taraxa-bootnode`.
- `tests/`: C++/CTest suites, Rust bridge tests under `tests/rust`, storage conformance tests, and Python integration tests.
- `rust/crates/`: Rust workspace crates, including `rustaxa-bridge`, `rustaxa-consensus`, `rustaxa-storage`, `rustaxa-types`, and `rustaxa-vdf`.
- `scripts/`: build, configuration, sync, and validation helpers.
- `PLAN.md`: consolidated Rust rewrite roadmap, architecture, scope, and validation plan.

## Rewrite Branch Model

- Strategic goal: progressively replace C++ internals with Rust while preserving upstream sync ability.
- `upstream-main`: clean mirror of upstream C++; no local commits.
- `cpp-reference`: C++ validation gate with integration hooks; must build and test in pure C++ mode (`RUSTAXA_ENABLE=0`).
- `main`: primary Rust rewrite branch and source of truth for future implementation.
- Hard rule: when a dependency or subsystem already has a Rust rewrite path, new rewrite work should leverage that Rust
  implementation directly instead of re-centering behavior in C++. Prefer extending Rust crates, bridges, and shim-owned
  Rust handles over adding C++ orchestration or C++ data materialization, unless a concrete blocker is documented and
  accepted by the task owner.
- Hard rule: slice planning and implementation must proactively look for opportunities to bridge into existing Rust
  rewrite code. Before settling on a design, inspect nearby Rust crates, bridge APIs, shim-owned handles, and already
  migrated storage/FinalChain/DAG/transaction/vote functionality. Prefer routing through those Rust implementations,
  even when it makes the slice moderately larger, if it reduces C++ ownership and advances the long-term goal of a
  complete Rust rewrite.
- Hard rule: logging is not an architectural blocker for moving consensus behavior to Rust. Do not keep deterministic
  logic in C++ merely because the legacy path logs there. Rust may return typed statuses, telemetry facts, or executor
  reports that C++ logs temporarily; logging can be moved, changed, or deleted later without affecting ownership.
- PBFT manager breakthrough boundary: move the protocol brain to Rust while keeping network/tarcap transport and
  EVM/FinalChain execution outside the manager migration for now. Rust should return typed effects for those boundaries
  instead of owning peer transport, packet wrapping, gossip fanout, gas/state execution, receipts, or contract execution.
- In C++ files, prefer additive per-module guards (`#ifdef RUSTAXA_ENABLE_VDF`, `#ifdef RUSTAXA_ENABLE_STORAGE`, `#ifdef RUSTAXA_ENABLE_FINAL_CHAIN`) or the master `#ifdef RUSTAXA_ENABLE` for shim-overlay integration over deleting legacy logic.
- For upstream-owned C++ classes, use the storage/final-chain overlay shim strategy instead of editing legacy files inline:
  add a shim include overlay, compile legacy implementation as `*Old`, and provide a shim class in shim-owned files. The
  shim class is the Rust-mode surface. Each method should call into Rust when the Rust implementation exists; otherwise,
  prefer an explicit shim-local exception/stub so missing rewrite work is visible. Forward to the `*Old` class only when
  throwing would prevent parity testing or proving correctness, and add a TODO comment at every `*Old` call site stating
  what must move to Rust. This is the default to minimize upstream merge conflicts while keeping temporary legacy use
  auditable.
- Hard rule: treat the full overlay shim as the first design step, not a later cleanup. Before adding Rust-mode behavior
  to an upstream-owned C++ class, create or extend the class overlay (`shims/<class>_shim/include/.../<class>.hpp`),
  compile the legacy implementation as `<Class>Old`, and put Rust routing, temporary stubs, and shim-only helper methods
  in shim-owned files. Do not add new Rust-only methods, `ForRust` hooks, bridge includes, or scattered `#ifdef`
  branches to original upstream headers/sources unless the task owner explicitly approves a temporary guarded hook.
  Before closeout, run `git diff upstream-main -- <original C++ paths>` for any upstream-owned files you touched; the
  expected result is empty or an explicitly documented temporary exception.
- Accepted temporary hook pattern: when a full shim copy would duplicate too much legacy implementation, the task owner
  may approve changing an upstream-owned class's implementation-state section from `private` to `protected` with a TODO
  that names the Rust overlay migration debt. The shim may then inherit from `<Class>Old`, let inherited methods delegate
  to the legacy state machine, and override only the Rust-owned methods that need direct state access. When using this
  pattern, still declare and define shim-owned public methods for the inherited public API: each unported method should
  explicitly forward to `<Class>Old::<method>` and include a TODO at the forwarding call site naming the Rust migration
  work that remains. This keeps temporary legacy delegation auditable instead of implicit. This is still temporary debt:
  do not add Rust bridge includes or Rust-only methods to the original class, keep all Rust routing in shim-owned files,
  and document the upstream-owned header diff before closeout.
- If a rewrite slice temporarily touches an upstream-owned C++ file because a complete class shim is not ready yet, keep
  the change guarded and track it as temporary debt. Revert that file back to its upstream shape as soon as a complete
  shim can own the Rust-mode routing.
- Hard rule: do not fix rewrite-discovered bugs in original upstream C++ code on `main`. Track the issue and implement the fix in the Rust rewrite path (Rust modules, bridge, or shim overlay). Touch original C++ only when explicitly approved by the task owner.
- Hard rule: in Rust-enabled production routing, never silently forward/delegate/inherit behavior from legacy C++
  implementations. Gaps must be explicit shim-local stubs/no-ops/throws until Rust parity is implemented, except for
  documented `*Old` forwarding used as a temporary parity-test scaffold as described above. If fallback is being
  considered for production behavior rather than parity scaffolding, stop and get explicit task-owner approval first.
- In C++ shim functions, prefer an early `return` inside the Rust `#ifdef` branch, then close with `#endif` and let the legacy C++ implementation continue below. Avoid `#ifdef` / `#else` / `#endif` when the Rust branch already returns.
- Hard rule: do not weaken, retarget, or otherwise tamper with existing tests to make rewrite mode pass. If Rust-mode behavior diverges from test expectations, fix implementation or parity wiring first, then update tests only when the intended product behavior has actually changed.
- Documentation rule: whenever adding or changing Rust/C++ rewrite code, document modules, public types, and public functions as complete units (purpose, inputs, outputs, invariants, and error/edge behavior), not just isolated lines.
- Do not remove C++ fallback/reference logic from `cpp-reference`.
- Any original upstream file that includes, links, or otherwise depends on a file that exists only on `main` must guard that dependency behind `RUSTAXA_ENABLE=1` / `#ifdef RUSTAXA_ENABLE` so pure C++ mode on `cpp-reference` does not require main-only files.

## Build and Test Commands

- `make help`: list local development targets.
- `make configure && make build`: configure Conan/CMake and compile `taraxad` using the default `/build` tree.
- For direct CMake builds, always compile with 12 jobs, for example
  `cmake --build /build --target pbft_manager_test --parallel 12`.
- `cd /build/tests && ctest --output-on-failure`: run registered C++ and `go_test` suites.
- `cd tests/py && ./run.sh -s --tb=short`: run Python integration tests.
- `make rewrite-validate-fast`: run the Rust pre-commit checks (`cargo fmt --check`, `cargo clippy`, and `cargo test`) plus whitespace validation.
- `cmake --build /build --target check-static`: run configured static/style checks before closeout when C++ changed.

Rust code changes are validated by the repository pre-commit hook at `.githooks/pre-commit`; address any problems it finds before closeout.
Always run the narrowest relevant targeted validation before closing a rewrite slice. For routine Rust rewrite work, this
means the affected Rust package checks/tests plus the focused C++ shim/bridge build or test targets that exercise changed
behavior. Do not skip validation merely because it is time-consuming; ask the task owner only before expensive repo-wide
or differential gates.

`check-static` is a repo-wide cppcheck/format gate and can currently fail on pre-existing findings outside the files touched
by a rewrite slice. For routine Rust rewrite work, prefer targeted validation for the changed C++ shim/bridge files plus the
relevant Rust and C++ tests. Run full `check-static` when broad C++ changes justify a repo-wide pass, before pre-merge
cleanup, or after existing cppcheck findings have been baselined/fixed. If `cppcheck` is installed after `/build` was
configured, rerun CMake or `make configure` before invoking `check-static` so the `cpp-check` target is generated.

## Storage Rewrite Validation

Before closing rewrite work, choose the narrowest validation tier in `doc/rewrite_validation_strategy.md` that covers the
changed behavior. Do not route Rust production behavior without C++ vs Rust parity validation, plus a Rust-enabled smoke
or subsystem test when the code runs during node startup, sync, consensus, finalization, or RPC handling.

For every storage-module change, validate the Rust storage bridge tests:

```bash
cmake --build /build --target rust_storage_tests
/build/bin/rust_storage_tests
```

If C++ storage behavior changes, also run the affected C++ gtests or the relevant `ctest` subset.

Add or update targeted tests when behavior changes:

- `tests/rust/storage/test_storage.cpp`
- `tests/storage_conformance/storage_conformance_runner.cpp`
- affected `tests/*_test.cpp` suites

For larger storage refactors, run the C++ vs Rust differential validation script before closeout:

```bash
scripts/storage_conformance_diff.sh
```

This script is expensive; ask the task owner before running it.

## Coding Style

- C++ follows `.clang-format` with Google base style and `ColumnLimit: 120`.
- Use C++20-compatible code and existing module boundaries in `libraries/*`.
- Prefer snake_case file names; keep new C++ tests named `*_test.cpp`.
- Rust code should preserve crate-local style, use explicit domain types at module boundaries, and keep codec/FFI wrappers separate from domain models.
- Keep hot-path storage and FinalChain code straightforward: avoid broad context structs, large traits, eager decoding, and unnecessary heap-backed abstractions.

## Rust Rewrite Design Rules

- Keep public C++ APIs stable during migration unless a task explicitly changes the API.
- Prefer small C++ shims over invasive call-site rewrites.
- In Rust, use explicit struct composition plus narrow trait-based ports for domain dependencies.
- Define dependencies in domain crates as task-oriented traits; implement them in infrastructure crates.
- Prefer static dispatch for throughput-critical paths and trait objects only at wiring boundaries.
- Use `#[repr(transparent)]` newtypes for semantically distinct scalar values such as periods, block numbers, transaction positions, gas, and hashes.
- Keep domain models independent of wire/storage codecs. Put RLP and compatibility encoding under codec modules.
- Decode late, encode early, and preserve canonical bytes when repeated hashing or persistence would otherwise re-encode data.
- Keep CXX bridge payloads plain and stable, then convert to domain types inside Rust logic.

## Upstream Sync Workflow

1. Sync `upstream-main` from upstream.
2. Merge `upstream-main` into `cpp-reference`.
3. Resolve conflicts by preserving integration hooks and updating the legacy C++ branch of dual-mode code.
4. Build and test `cpp-reference` in pure C++ mode.
5. Create a temporary sync branch from `main` and merge `cpp-reference`.
6. Port changed C++ behavior into the corresponding Rust implementation.
7. Build and test Rust-enabled mode, then merge the sync branch to `main`.

Useful helpers:

```bash
make cpp-intersection-list
make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>
make cpp-intersection-patch FROM=<base_sha> TO=<tip_sha>
```

The intersection helpers are intentionally narrow. `make cpp-intersection-list` defaults to `upstream-main..main`. Patch/apply targets use the explicit `FROM..TO` range. In both cases, helpers select only modified paths that already exist in upstream-owned code, excluding Rust-only, devcontainer, GitHub workflow, docs, Makefile, and `.gitignore` changes. New files that exist only on `main` are not included in the intersection patch.

Use `make cpp-intersection-list` before applying a carry-back patch and verify it is the smallest set of upstream-owned files touched by `main`. Use `make cpp-reference-apply-intersection` after Rust feature work lands on `main` when those original-file changes need to be carried back to `cpp-reference`.

## Commit and PR Guidelines

- Use Conventional Commits: `<type>(<scope>)!: <subject>`.
- Allowed types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `build`, `perf`, `style`, `ci`.
- Mark breaking changes with `!` in the header or a `BREAKING CHANGE:` footer.
- Keep subjects imperative, lowercase-first, and without a trailing period.
- Standard work branches from `develop`; reserve `hotfix/*` and `release/*` for release workflow.
- PRs should fill template sections: `Purpose`, `How does the solution address the problem`, and `Changes made`.
- Link related issues when possible and ensure required CI/review gates pass before merge.
