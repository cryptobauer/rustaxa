# 🏗️ Project Architecture & Rust Rewrite Strategy

This project is a progressive rewrite of the upstream C++ repository into Rust. To ensure long-term maintainability and the ability to pull in upstream changes, we follow a **Validation Gate** architecture.

---

## 🌿 Branching Strategy

We maintain three primary tracks of development to separate upstream logic from our Rust implementation.

| Branch | Role | Description |
| :--- | :--- | :--- |
| **`upstream-main`** | **The Mirror** | A 1:1 clean replica of the upstream repository. No local commits. |
| **`cpp-reference`** | **Validation Gate** | The C++ codebase + `#ifdef` integration hooks. This branch is used to verify upstream updates in a pure C++ environment (`RUSTAXA_ENABLE=0`) before porting. |
| **`main`** | **The Rewrite** | The primary Rust codebase. This is the "Source of Truth" for the project's future. |

---

## 🛠️ The "Dual-Mode" Integration (`#ifdef`)

We use a "Strangler Fig" pattern to replace C++ modules. By wrapping the original logic in `#ifdef` blocks, we keep the original C++ code as a functional fallback and a clear blueprint for the Rust port.

```cpp
// Example of the integration pattern
void process_data() {
#ifdef RUSTAXA_ENABLE
    // Calls the Rust implementation via FFI
    rust_process_data_bridge();
#else
    // Original C++ logic (receives upstream updates)
    legacy_cpp_process_data();
#endif
}
```

## 🔄 Upstream Synchronization Workflow

When the upstream project releases updates, we follow this gated pipeline:

1.  **Sync Baseline:** `git checkout upstream-main && git pull upstream main`
2.  **Verify C++ (The Gate):** Merge `upstream-main` into `cpp-reference`. Resolve conflicts and ensure the project builds and runs with `RUSTAXA_ENABLE=0`.
3.  **Integration Branch:** Create a temporary feature branch from `main` (e.g., `sync/upstream-jan-2026`). Merge `cpp-reference` into this branch.
4.  **Rust Logic Port:** Build with `RUSTAXA_ENABLE=1`. Identify where the C++ logic changed (the `#else` blocks) and update the corresponding Rust code.
5.  **Merge to Main:** Once all tests pass, merge the feature branch into `main`.

## ✅ Storage Rewrite Validation Workflow

For changes in the storage module (C++ storage shim, Rust storage crate, or Rust bridge), test validation is required as part of the task.

### Regular storage changes (required)

1. Build and run storage-focused gtests in your active build tree.
2. At minimum, run the Rust bridge storage gtests binary:

```bash
cmake --build /build --target rust_storage_tests
/build/bin/rust_storage_tests
```

3. If your change touches C++ storage behavior, also run the affected C++ gtests (or `ctest --output-on-failure` for the relevant set).
4. If behavior changes are introduced (new logic, bug fix, serialization/update semantics), add or extend tests in:
   - `tests/rust/storage/test_storage.cpp`
   - `tests/storage_conformance/storage_conformance_runner.cpp`
   - other impacted `tests/*_test.cpp` suites as needed

### Larger storage refactors (required at end of task)

For larger refactors (multi-file storage rewrites, API remaps, batch semantics changes, migration of read/write paths), run the C++ vs Rust differential validation script after implementation:

```bash
scripts/storage_conformance_diff.sh
```

This script is expensive and should not be run implicitly. Always ask the task owner/requester if it should be run before executing it.

## 📂 Repository Structure

*   `/libraries`: The original C++ libraries.
*   `/programs`: The original C++ programs.
*   `/rust`: The new Rust codebase with modules that will replace the C++ logic.

```
/
├── CMakeLists.txt              <-- Include Corrosion
├── libraries/                  <-- Existing C++ code (use the `#ifdef` pattern here)
├── programs/                   <-- Existing C++ code (use the `#ifdef` pattern here)
└── rust/
    ├── Cargo.toml              <-- Workspace root (standalone Rust that can be built/tested independently)
    ├── bridge/
    │   ├── Cargo.toml          <-- The "Shim" crate
    │   ├── build.rs            <-- Configures cxx_build
    │   └── src/
    │       └── lib.rs          <-- Defines the #[cxx::bridge]
    └── libs/
        ├── vdf/                <-- Pure Rust logic (no C++ knowledge)
        └── ...
```

**Note to Contributors:** Do not delete C++ code in the `cpp-reference` branch. Instead, wrap it in `#ifdef RUSTAXA_ENABLE` to maintain the validation gate and ensure we can always fall back to the C++ baseline for debugging.

## 🔀 Syncing C++ Intersection to `cpp-reference`

When a feature is merged to `main` (including squash merge), we still want the C++ side touched by that feature to be present in `cpp-reference`.

Use the Makefile helpers:

1. Check out `cpp-reference`.
2. Apply only the C++ intersection from a commit range:

```bash
make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>
```

By default, the Makefile detects intersection paths dynamically from `main..cpp-reference`, excluding Rust and repo-meta paths.

You can inspect detected paths:

```bash
make cpp-intersection-list
```

You can override path detection explicitly if needed:

```bash
make cpp-reference-apply-intersection \
    FROM=<base_sha> TO=<tip_sha> \
    CPP_INTERSECTION_PATHS="CMakeLists.txt CMakeModules libraries programs submodules tests"
```

Then commit:

```bash
git commit -m "chore(cpp-reference): sync C++ intersection from <tip_sha>"
```

If you want to inspect the patch before applying, generate it first:

```bash
make cpp-intersection-patch FROM=<base_sha> TO=<tip_sha>
```

Patch output path: `/build/cpp-reference-intersection.patch`.


