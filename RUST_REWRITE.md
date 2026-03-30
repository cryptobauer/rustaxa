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

> **One-time setup:** Ensure the `upstream` remote points to the original repository:
> ```bash
> git remote add upstream <upstream-repo-url>
> ```

### 1. Sync Baseline

Fetch the latest upstream commits onto `upstream-main` and push it so all contributors are up to date:

```bash
git checkout upstream-main
git pull upstream main
git push origin upstream-main
```

### 2. Verify C++ (The Gate)

Merge `upstream-main` into `cpp-reference` and validate the pure C++ build. **Merge conflicts are expected** — upstream may have modified the same lines that were wrapped in `#ifdef` blocks. When resolving, always preserve the `#ifdef` structure and update only the `#else` block with upstream's new logic.

```bash
git checkout cpp-reference
git merge upstream-main --no-ff
# resolve any conflicts, keeping the #ifdef structure intact
make configure  # RUSTAXA_ENABLE=OFF by default
make build
cd /build/tests && ctest --output-on-failure
git push origin cpp-reference
```

### 3. Integration Branch

Create a temporary sync branch from `main` and merge `cpp-reference` into it. Conflicts here are also possible — they arise where Rust porting work on `main` diverges from the updated C++ on `cpp-reference`. Apply the same resolution rule: preserve `#ifdef` structure, update `#else` with upstream logic.

```bash
git checkout main
git checkout -b sync/upstream-jan-2026
git merge cpp-reference --no-ff -m "chore(sync): merge cpp-reference into sync/upstream-jan-2026"
```

### 4. Rust Logic Port

Find what C++ logic changed in this upstream sync, then update the corresponding Rust code:

```bash
# Inspect what changed on the C++ side in this sync
git diff upstream-main~1..upstream-main -- libraries/ programs/ submodules/
```

For each changed `#else` block, update the matching Rust implementation in `/rust`. Then validate the Rust build:

```bash
# Reconfigure with Rust enabled, then build and test
cd /build && cmake /workspaces/rustaxa -DRUSTAXA_ENABLE=ON
cmake --build /build -j6 --target=taraxad
cargo test --manifest-path /workspaces/rustaxa/rust/Cargo.toml
cd /build/tests && ctest --output-on-failure
```

### 5. Merge to Main

Once all tests pass, merge the sync branch into `main`, then clean up:

```bash
git checkout main
git merge sync/upstream-jan-2026 --no-ff
git branch -d sync/upstream-jan-2026
git push origin main
```

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

> **When to run this:** After merging a Rust feature branch into `main`, run this to keep `cpp-reference` up to date with any C++ changes that were part of that feature. This is independent of the upstream sync workflow above — it flows in the opposite direction (`main` → `cpp-reference`).

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



