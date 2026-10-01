# Rewrite implementation workflow

Read this file before changing an upstream-owned C++ class or making Rust production routing depend on it. The root `AGENTS.md` contains the always-loaded invariants; this file gives the required implementation and closeout procedure.

## Upstream-owned C++ overlays

Start with a complete shim overlay, following the storage/FinalChain pattern. Add the shim include overlay at `shims/<class>_shim/include/.../<class>.hpp`, compile the legacy implementation as `<Class>Old`, and put Rust routing, shim-only helpers and explicit missing-work stubs in shim-owned files. Keep original upstream headers and sources free of new Rust-only methods, `ForRust` hooks, bridge includes and scattered guards. Use `#ifdef RUSTAXA_ENABLE` as the master integration guard; Rust production has one supported composition.

Each shim method calls Rust when that implementation exists. Otherwise use a visible local exception, stub or no-op. Never silently fall back to legacy C++ in production. A `*Old` call is allowed only as a documented temporary parity-test scaffold when throwing would prevent parity testing or proving correctness; place a TODO at every call site that names the behavior still to move to Rust. If a full shim would duplicate too much code, the task owner may approve the temporary protected-state pattern: change the original implementation state from private to protected with a migration TODO, inherit from `<Class>Old`, override Rust-owned methods, and declare/define each inherited public API method in the shim. Each unported method must explicitly forward to `*Old` with its own migration TODO. Keep all routing and bridge includes in shim-owned files.

If an approved temporary slice changes an original upstream file before the shim is ready, guard the change and track it as debt. Restore the file to upstream shape once the shim owns the routing. Do not fix rewrite-discovered defects in upstream C++ on `main` without task-owner approval. Do not remove C++ reference/fallback logic from `cpp-reference`.

In shim methods, prefer an early return from the Rust `#ifdef` branch, then let the legacy implementation continue below `#endif`; avoid `#else` when the Rust branch returns. Before closeout, run `git diff upstream-main -- <original C++ paths>`. It must be empty or the remaining guarded exception must be documented. Keep any dependency on a main-only file behind `RUSTAXA_ENABLE` so `cpp-reference` still builds with `RUSTAXA_ENABLE=0`.

## Validation details

Choose the narrowest tier in `doc/rewrite_validation_strategy.md` that proves the changed behavior. Rust production behavior needs C++/Rust parity and, for startup, sync, consensus, finalization or RPC paths, a Rust-enabled smoke or subsystem test. For each storage change, build and run:

```bash
cmake --build /build --target rust_storage_tests
/build/bin/rust_storage_tests
```

Also run affected C++ gtests or the relevant CTest subset when C++ storage behavior changes. Add focused behavior tests in `tests/rust/storage/test_storage.cpp`, `tests/storage_conformance/storage_conformance_runner.cpp`, or affected `tests/*_test.cpp` suites. Larger storage refactors require `scripts/storage_conformance_diff.sh`; ask the task owner before this expensive differential run. Ask before expensive repository-wide gates. Keep replay and fault work within the authorized slice. Prepare exact commands, dataset identity and bounds first; record skipped or failed required checks explicitly.

For routine C++ changes, prefer focused shim/bridge targets. `check-static` is repository-wide and can report pre-existing issues outside the changed files; run it for broad C++ changes, pre-merge cleanup, or after existing findings are baselined or fixed. If cppcheck is installed after `/build` was configured, rerun CMake or `make configure` to generate `cpp-check`. Run direct CMake builds with `--parallel 12`. Rust changes also need the affected crate checks/tests and the repository fast gate required by `AGENTS.md`.
