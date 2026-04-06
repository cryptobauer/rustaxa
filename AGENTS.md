# Repository Guidelines

## Project Structure & Module Organization
- `libraries/`: C++ modules by domain (`core_libs/{consensus,network,storage}`, `types/*`, `app`, `cli`, `metrics`).
- `programs/`: node binaries (`taraxad`, `taraxa-bootnode`).
- `tests/`: C++/CTest suites (`*_test.cpp`), Rust bridge tests (`tests/rust`), Python integration tests (`tests/py`).
- `rust/crates/`: Rust workspace crates (`rustaxa-bridge`, `rustaxa-storage`, `rustaxa-types`, `rustaxa-vdf`).
- `scripts/`: build/config helpers (`build.sh`, `config.sh`).

## Rust Rewrite Goal & Branch Model
- Strategic goal: progressively replace C++ with Rust; `main` is the rewrite branch.
- `upstream-main` mirrors upstream C++ with no local commits.
- `cpp-reference` is the validation gate: verify sync/build in pure C++ mode (`RUSTAXA_ENABLE=0`) before Rust ports.
- In C++ files, prefer `#ifdef RUSTAXA_ENABLE` dual-mode integration over deleting legacy logic.
- For upstream syncs or intersections, use Makefile helpers: `make cpp-intersection-list`, `make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>`.

## Build, Test, and Development Commands
- `make help`: list available local development targets (devcontainer-oriented).
- `make configure && make build`: configure Conan + CMake (default `/build`) and compile `taraxad`.
- `cd /build/tests && ctest --output-on-failure`: run registered C++ and `go_test` suites.
- `cd tests/py && ./run.sh -s --tb=short`: create virtualenv, install requirements, run pytest tests.
- `cargo test --manifest-path rust/Cargo.toml`: run Rust workspace tests.

## Coding Style & Naming Conventions
- C++ formatting follows `.clang-format` (Google base, `ColumnLimit: 120`).
- Use C++20-compatible code and existing module boundaries in `libraries/*`.
- Prefer snake_case file names (example: `pbft_manager_test.cpp`); keep new tests named `*_test.cpp`.
- Before submitting, run static/style checks from the build tree:
  - `cmake --build /build --target check-static`

## Testing Guidelines
- Add unit tests for new C++ behavior in `tests/*_test.cpp`.
- For Rust bridge/storage work, add coverage under `tests/rust/...`.
- For end-to-end or RPC behavior, add/update pytest cases in `tests/py/tests/`.
- No numeric coverage threshold is documented; add targeted regression tests for behavior changes.

## Commit & Pull Request Guidelines
- Use Conventional Commits: `<type>(<scope>)!: <subject>` (example: `feat(storage): add DAG repository cache`).
- Allowed types: `feat`, `fix`, `refactor`, `test`, `docs`, `chore`, `build`, `perf`, `style`, `ci`.
- Mark breaking changes with `!` in the header or a `BREAKING CHANGE:` footer.
- Keep subjects imperative, lowercase-first, no trailing period.
- Standard work branches from `develop`; reserve `hotfix/*` and `release/*` for release workflow.
- PRs should fill template sections: `Purpose`, `How does the solution address the problem`, `Changes made`.
- Link related issues when possible (for example `issue-1234/my-change`) and ensure CI/review requirements are satisfied before merge.
