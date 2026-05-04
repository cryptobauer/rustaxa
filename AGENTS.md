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
- In C++ files, prefer additive per-module guards (`#ifdef RUSTAXA_ENABLE_VDF`, `#ifdef RUSTAXA_ENABLE_STORAGE`, `#ifdef RUSTAXA_ENABLE_FINAL_CHAIN`) or the master `#ifdef RUSTAXA_ENABLE` for shim-overlay integration over deleting legacy logic.
- In C++ shim functions, prefer an early `return` inside the Rust `#ifdef` branch, then close with `#endif` and let the legacy C++ implementation continue below. Avoid `#ifdef` / `#else` / `#endif` when the Rust branch already returns.
- Do not remove C++ fallback/reference logic from `cpp-reference`.
- Any original upstream file that includes, links, or otherwise depends on a file that exists only on `main` must guard that dependency behind `RUSTAXA_ENABLE=1` / `#ifdef RUSTAXA_ENABLE` so pure C++ mode on `cpp-reference` does not require main-only files.

## Build and Test Commands

- `make help`: list local development targets.
- `make configure && make build`: configure Conan/CMake and compile `taraxad` using the default `/build` tree.
- `cd /build/tests && ctest --output-on-failure`: run registered C++ and `go_test` suites.
- `cd tests/py && ./run.sh -s --tb=short`: run Python integration tests.
- `make rewrite-validate-fast`: run the Rust pre-commit checks (`cargo fmt --check`, `cargo clippy`, and `cargo test`) plus whitespace validation.
- `cmake --build /build --target check-static`: run configured static/style checks before closeout when C++ changed.

Rust code changes are validated by the repository pre-commit hook at `.githooks/pre-commit`; address any problems it finds before closeout.

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
