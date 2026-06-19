# Storage Compatibility Collapse Plan

This plan tracks the cleanup phase after migrated consensus storage ownership moved to Rust. It complements `PLAN.md`,
`doc/consensus_rewrite_tracker.md`, and `doc/rewrite_validation_strategy.md`; keep those files as the higher-level source
of truth.

Use `$implement-rustaxa-consensus-slice` for implementation slices from this plan when they touch consensus storage
shims, Rust consensus runtimes, or CXX bridge surfaces.

## Goal

Substantially reduce storage shim and bridge code now that production Rust-mode consensus storage routes no longer depend
on `DbStorage`, public `rustBatchId`, C++ batch assembly, or bridge-batch appenders as storage authority.

The target is not to delete every C++ storage API. `DbStorage` may remain as the legacy/reference, app lifecycle,
admin/query, conformance, and public-object materialization shell until those owning tracks move. The target is to delete
compatibility APIs that are no longer needed by production Rust-mode consensus or by required compatibility callers.

## Current Starting Point

Already Rust-owned:

- `rustaxa-storage` is the durable backend for migrated production consensus storage rows.
- PBFT finalization, VoteManager persistence, TransactionManager consensus storage, DAG/proposed-block storage, rewards
  stats, pillar storage, PBFT-manager residual/scalar storage, gas-pricer storage, and consensus FinalChain fact ports
  route through Rust-owned runtimes or storage sessions.
- The post-migration audit found no remaining unclassified production consensus route that depends on `DbStorage`,
  `getDB()`, public `rustBatchId`, or bridge-batch appenders as storage authority.
- `scripts/rewrite_storage_boundary_guard.sh` rejects newly added unclassified C++ storage authority in Rust mode.

Remaining compatibility surfaces:

- `libraries/core_libs/consensus/shims/storage_shim/*` is 1,400 lines and exposes a broad `DbStorage` compatibility API.
- `rust/crates/rustaxa-bridge/src/storage.rs` is 1,268 lines and still hosts `BridgeStorage`, query/materialization
  helpers, and compatibility batch registry behavior.
- `rust/crates/rustaxa-bridge/src/ffi.rs` exposes a large `BridgeStorage` CXX surface for read/write compatibility.
- C++ tests, storage conformance fixtures, network/API materialization, admin/lifecycle code, and some shim constructors
  still call `DbStorage::rustStorage()`, `createWriteBatch()`, or direct `BridgeStorage` helpers.

## Target Boundary

Rust consensus runtimes should receive or own explicit Rust storage handles and call Rust repositories directly. C++ may
execute external effects and materialize legacy public objects, but it should not use `DbStorage` or a generic
`BridgeStorage` facade as a consensus storage API.

Allowed to remain until their owning tracks move:

- legacy/reference C++ storage implementation
- storage conformance and test fixture routes that intentionally compare C++ and Rust behavior
- RPC, GraphQL, debug, network/tarcap, and public API materialization reads with explicit compatibility classification
- app lifecycle, migration, snapshot, compaction, iterator, plugin/light, and admin operations
- FinalChain external-EVM/state/account/code/storage boundaries outside consensus storage ownership

Not allowed to remain as production Rust-mode consensus authority:

- generic C++ batch creation/commit for migrated consensus writes
- bridge-side batch appenders used by production consensus write paths
- unclassified `DbStorage` reads that collect consensus facts for Rust planners
- new direct `BridgeStorage` dependencies in subsystem runtimes where a typed Rust runtime/storage session can own the
  storage handle

## Slice 1: Compatibility Surface Inventory and Guard Tightening

Status: ready to implement deletion slices from the initial inventory.

Classify every remaining `DbStorage::rustStorage()`, `BridgeStorage`, `createWriteBatch()`, `commitWriteBatch()`, and
bridge-batch call site.

Initial inventory:

| Owner class | Current evidence | Classification | Deletion direction |
| --- | --- | --- | --- |
| Production shim runtime seeds | `dag_manager_shim`, `gas_pricer_shim`, `transaction_manager_shim`, `proposed_blocks_shim`, `final_chain_shim`, `pbft_manager_shim`, `sortition_params_manager_shim`, `vote_manager_shim`, `pillar_chain_manager_shim`, `rewards_stats_shim`, and `pbft_chain_shim` still call `DbStorage::rustStorage()` or retain `BridgeStorage*` fields. | Transitional production runtime wiring. | Slice 2 should replace these with typed Rust runtime constructors or typed storage handles that clone/own Rust storage internally, then delete the retained generic `BridgeStorage*` members. |
| Rust bridge runtime APIs | `transaction_manager.rs`, `sortition.rs`, `rewards_stats.rs`, `pbft_finalize.rs`, `final_chain.rs`, `dag.rs`, `pillar_chain.rs`, `proposed_blocks.rs`, `pbft_manager.rs`, `pbft_chain.rs`, and `gas_pricer.rs` accept `&BridgeStorage` for runtime setup, storage restore, or direct storage operations. | Transitional bridge facade. | Slice 2 should move production runtime setup to typed constructor APIs; Slice 4 should leave only typed public/query compatibility APIs where C++ materialization still owns the caller. |
| Generic bridge batch registry | `storage_shim.hpp/cpp` maps C++ `Batch` objects to bridge batch ids; `storage.rs` exposes `create_write_batch`, `batch_put`, `batch_delete`, `commit_write_batch`, and `drop_write_batch`; `ffi.rs` exports those methods. | Compatibility write scaffold. | Slice 3 should delete this from production paths first, then either remove it from `BridgeStorage` or quarantine it behind explicit test/conformance APIs. |
| Test and conformance batches | `tests/rust/storage/test_storage.cpp`, `tests/storage_conformance/storage_conformance_runner.cpp`, `tests/rust/consensus/test_pbft_sync.cpp`, and legacy C++ gtests seed storage through bridge or `DbStorage` batches. | Required validation compatibility. | Keep until Slice 3 migrates Rust-mode fixtures to direct Rust storage helpers; do not remove conformance coverage. |
| Legacy/reference C++ storage batches | `libraries/core_libs/storage/*`, `libraries/core_libs/consensus/src/*`, migrations, and pure C++ tests still use `DbStorage::createWriteBatch()` / `commitWriteBatch()`. | Legacy/reference and lifecycle behavior. | Keep for `cpp-reference` and legacy builds; do not use these as Rust-mode production authority. |
| App lifecycle and light/admin routes | `libraries/plugin/light/src/light.cpp`, migration helpers, snapshot/compaction/iterator/admin-style storage surfaces. | Lifecycle/admin compatibility. | Keep until those owners move; mark unsupported Rust-mode routes rather than silently routing consensus through them. |
| Public query and network materialization | Broad `BridgeStorage` getters in `ffi.rs` and `storage.rs` support DAG, PBFT, vote, transaction, pillar, FinalChain, rewards, RPC/debug, and tarcap materialization. | Query/materialization compatibility. | Slice 4 should split these into typed read-only APIs and retain C++ object construction only at public/network boundaries. |

Guard status:

- `scripts/rewrite_storage_boundary_guard.sh` already rejects newly added unclassified C++ storage authority in
  non-allowlisted Rust-mode paths, including `DbStorage`, `db_->`, `getDB()`, `rustStorage()`, `createWriteBatch()`,
  `commitWriteBatch()`, `rustBatchId()`, and direct FinalChain DPoS fact reads from non-provider paths.
- Current allowlists intentionally cover the existing storage shim, storage implementation, and tests. RPC/GraphQL reads
  require `RUSTAXA_QUERY_COMPAT_READ`; network/tarcap compatibility requires `RUSTAXA_NETWORK_COMPAT_LEGACY_ONLY`.
- No guard change is required for the inventory slice. Tighten it in later slices only when a newly discovered
  production consensus pattern is not rejected.

Concrete deletion candidates for the next slices:

- Production shim `BridgeStorage*` fields and constructor-time `DbStorage::rustStorage()` calls that only seed Rust
  runtimes.
- Bridge runtime functions that accept `&BridgeStorage` even though they immediately operate on subsystem-specific Rust
  repositories.
- `BridgeStorage` generic batch registry functions and the `DbStorage` shim batch-id mapping layer.
- Broad `BridgeStorage` query methods that survive only because C++ public/network materializers still need legacy
  object construction.

Scope:

- Build an inventory grouped by owner: production shim constructor, production runtime storage call, public query/API,
  network sync/materialization, admin/lifecycle, test/conformance, legacy/reference, or unsupported Rust-mode behavior.
- Add or update compatibility markers where a C++ call site intentionally remains.
- Tighten `scripts/rewrite_storage_boundary_guard.sh` if it misses any unclassified production consensus pattern found
  during the inventory.
- Update `PLAN.md` or `doc/consensus_rewrite_tracker.md` only if the current storage-boundary status is inaccurate.

Acceptance:

- Every remaining storage compatibility call site is classified.
- New unclassified production consensus storage routes are still rejected by the guard.
- The inventory identifies concrete deletion candidates for Slices 2 and 3.

Validation:

- `scripts/rewrite_storage_boundary_guard.sh`
- `make rewrite-validate-fast` when guard logic changes.
- `git diff --check`

## Slice 2: Runtime Handle Collapse

Status: in progress.

Replace production shim constructor uses of `DbStorage::rustStorage()` / `BridgeStorage&` with typed Rust runtime
constructors or typed bridge handles that clone/own `Arc<rustaxa_storage::Storage>` on the Rust side.

Landed sub-slices:

- Proposed blocks: `BridgeProposedBlocks` now owns an optional cloned Rust storage handle, the C++ proposed-block shim no
  longer retains a `BridgeStorage*` sidecar, and proposed-block restore/persist/cleanup methods no longer take generic
  storage as a per-call argument. Rust still owns the storage write batch for proposed-block save and cleanup. The C++
  shim keeps `DbStorage` only as the lifetime owner and constructor seed until a broader lifecycle cleanup can remove it.
- PBFT chain startup: the C++ shim now uses the typed `create_pbft_chain_from_storage(...)` constructor instead of
  separate generic restore-plus-create storage calls. The Rust runtime carries the default-initialization flag needed for
  the legacy startup log branch.
- Pillar chain manager: production pillar-chain storage reads and writes now use a typed `BridgePillarChainStorage`
  handle that owns a cloned Rust storage handle. The C++ manager no longer retains or passes a generic `BridgeStorage*`
  for own-vote, current-block, finalized-block, latest-block, or period-data pillar storage calls.
- TransactionManager: the Rust runtime now owns an optional cloned Rust storage handle, and the production C++ shim uses
  `create_transaction_manager_runtime_from_storage(...)` instead of retaining a `BridgeStorage*`. DAG transaction
  persistence, transaction-view lookup, finalized-status updates, finalized filtering, finalized verification, and
  recovery now use runtime-owned storage in production calls; older byte-oriented bridge helpers remain for tests and
  compatibility materialization.
- VoteManager: production PBFT vote persistence now uses the storage-attached `VerifiedVotes` / `BridgeVerifiedVotes`
  runtime. The C++ manager no longer retains a generic `BridgeStorage*`; own-vote save/clear, vote-progress
  persistence, and reward-vote finalization reset route through the typed verified-votes runtime.

Next target:

- Collapse one of the remaining constructor-time `BridgeStorage` seeds with retained generic storage fields, likely DAG
  manager or PBFT manager startup/runtime handles.

Scope:

- Start with shims that only need a durable Rust storage handle to seed a runtime, such as DAG manager, PBFT chain,
  gas pricer, rewards stats, proposed blocks, sortition params, TransactionManager, VoteManager, and PBFT manager
  startup/runtime handles.
- Prefer typed constructors like `create_<subsystem>_runtime_from_storage(...)` that hide generic storage access behind
  subsystem bridge APIs.
- Avoid adding new C++ storage fact collection. C++ may still pass config and executor facts.
- Remove `BridgeStorage` fields from shim classes once the typed runtime owns the needed storage access.

Acceptance:

- Production shims no longer retain generic `BridgeStorage*` fields when a typed Rust runtime can hold the storage
  handle.
- `DbStorage::rustStorage()` use is reduced to lifecycle/query/test/conformance compatibility and explicitly remaining
  transition points.
- No migrated consensus write path uses generic bridge-batch authority.

Validation:

- Affected C++ shim targets and tests, for example `dag_block_test`, `rust_consensus_tests`, transaction/gas/rewards
  shim tests as applicable.
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge <affected module>`
- `git diff --check`

## Slice 3: Bridge Batch Registry Deletion

Status: in progress.

Remove compatibility batch helpers from `BridgeStorage` after production and required compatibility callers no longer
need C++ `Batch` objects backed by Rust bridge batch ids.

Landed sub-slices:

- PBFT manager storage bypasses: deleted standalone CXX bridge helpers that accepted generic `BridgeStorage` for
  transition writes and startup replay reads outside the long-lived runtime handle. The remaining C++ storage coverage
  now constructs `BridgePbftManagerRuntime` from storage and uses runtime-owned transition/replay methods, so production
  and validation both exercise the typed runtime storage handle after construction.
- Generic bridge batch registry: renamed the CXX-exposed `BridgeStorage` batch methods to
  `compat_create_write_batch`, `compat_batch_put`, `compat_batch_delete`, `compat_commit_write_batch`, and
  `compat_drop_write_batch`. This was an intermediate quarantine step before the registry was deleted.
- PBFT finalization fixture seeding: `rust_consensus_tests` no longer seeds DAG/transaction rows through compatibility
  bridge batches. The PBFT finalization storage apply tests now use typed storage helpers for the prerequisite DAG block
  and transaction rows.
- Native storage batch coverage: moved raw batch commit/drop/delete semantics into `rustaxa-storage` unit tests and
  removed the dedicated CXX bridge-registry batch tests from `rust_storage_tests`.
- Storage conformance setup: removed `compat_*` bridge-batch use from the Rust-mode conformance runner. Period-data
  setup now uses the typed `save_period_data` helper, final-chain lookup fixture rows are committed by a narrow native
  `rustaxa-storage` final-chain conformance writer, and generic bridge-batch lifecycle transcript entries were removed
  because raw batch semantics are covered directly in `rustaxa-storage` unit tests.
- Storage shim batch ownership: deleted the `BridgeStorage` integer batch registry and the `compat_*` CXX methods.
  The C++ storage shim now maps legacy `Batch*` values to opaque `BridgeStorageBatch` boxes whose live
  `StorageWriteBatch` is owned by `rustaxa-storage`; commit consumes the Rust batch object, while dropped C++ batches
  discard their Rust-owned writes without a bridge-side registry.
- Metadata/rewards shim batch appends: routed storage-shim status fields, sortition parameter changes, period lambda,
  dynamic-lambda rounds count, and block-rewards stats writes through typed `rustaxa-storage` metadata batch methods
  instead of broad `insert(Batch&, Column, ...)` appends. These methods still preserve the legacy C++ `Batch&` commit
  boundary by appending to the same opaque `BridgeStorageBatch`.
- PBFT manager/vote shim batch appends: routed PBFT manager field/status writes, cert-voted block cleanup, PBFT head
  writes, own verified vote cleanup, latest-round 2t+1 vote replacement, and extra reward vote cleanup through typed
  `rustaxa-storage` PBFT batch methods. The C++ shim still builds legacy PBFT object RLP where required, but column
  selection, key encoding, delete/put ordering, and commit/drop ownership are now Rust-owned.
- Period-index shim batch appends: routed finalized PBFT block hash-to-period and DAG block hash-to-period/position
  writes through typed `rustaxa-storage` period/DAG batch methods. Rust now owns the `pbft_block_period` little-endian
  value encoding and `dag_block_period` legacy RLP payload while preserving the caller's legacy `Batch&` commit boundary.

Next target:

- Reduce the remaining storage-shim raw append surface in the DAG block/counter, transaction, proposed-block cleanup, and
  proposal-level mapping families by replacing broad `insert(Batch&, Column, ...)` and `remove(Batch&, Column, ...)`
  callers with typed Rust storage helpers where active tests or public compatibility paths still need them.

Scope:

- Replace remaining migrated consensus batch appenders with Rust-owned storage apply functions or operation-specific
  runtime reports.
- Move test/conformance fixture batch setup to direct Rust storage test APIs where possible.
- Keep legacy C++ `DbStorageOld` batch behavior for pure C++ reference builds.
- Delete the remaining storage-shim raw append methods once no required compatibility caller remains.

Acceptance:

- No Rust-mode production consensus path creates or commits a generic C++/bridge storage batch.
- Batch registry state is removed from `BridgeStorage`.
- Storage conformance still covers Rust storage batch semantics through direct Rust storage APIs.

Validation:

- `cmake --build /build --target rust_storage_tests --parallel 12`
- `/build/bin/rust_storage_tests`
- affected C++ storage/conformance tests
- `scripts/storage_conformance_diff.sh` after owner confirmation if key layout or batch semantics changed
- `git diff --check`

## Slice 4: Query and Materialization API Split

Split the broad `BridgeStorage` query surface into typed read-only Rust query APIs for active Rust-mode callers, and
delete unused broad storage-shim methods.

Scope:

- Group current query/materialization callers by API family: DAG sync/public DAG reads, transaction public reads,
  PBFT/vote query reads, pillar query reads, FinalChain publication/status reads, rewards stats reads, and admin/debug.
- For active Rust-mode surfaces, introduce typed read APIs that return canonical bytes, compact facts, or explicit
  missing/error statuses instead of generic `DbStorage` methods.
- Mark or remove unsupported Rust-mode admin/snapshot/migration paths instead of silently falling back.
- Delete `DbStorage` shim methods once their callers use typed APIs or are explicitly unsupported in Rust mode.

Acceptance:

- The storage shim no longer exposes broad methods solely because an old public API caller existed.
- Remaining query methods are grouped and documented as public/query/network/admin compatibility.
- Public materialization continues to construct legacy C++ objects only at the API/network boundary.

Validation:

- affected RPC/GraphQL/network/gtest coverage
- `rust_storage_tests` when query behavior touches storage rows
- targeted startup smoke if app lifecycle paths change
- `git diff --check`

## Slice 5: Storage Shim Header and FFI Surface Pruning

After runtime, batch, and query callers have moved, delete unused declarations and CXX FFI entries.

Scope:

- Remove unused `DbStorage` shim declarations from `storage_shim.hpp`.
- Remove corresponding `storage_shim.cpp` methods.
- Remove unused `BridgeStorage` methods from `rust/crates/rustaxa-bridge/src/storage.rs`.
- Remove unused CXX declarations from `rust/crates/rustaxa-bridge/src/ffi.rs`.
- Remove obsolete tests that only exercised deleted compatibility routes, or convert them to typed Rust API tests.

Acceptance:

- `BridgeStorage` is either deleted or reduced to a narrow constructor/handle adapter with no generic consensus storage
  authority.
- `storage_shim` is either deleted or reduced to lifecycle/query/admin compatibility that cannot be mistaken for a
  production consensus storage API.
- Targeted searches show no stale references to deleted methods.

Validation:

- `cargo check --manifest-path rust/Cargo.toml -p rustaxa-bridge`
- `cargo test --manifest-path rust/Cargo.toml -p rustaxa-bridge storage`
- `cmake --build /build --target rust_storage_tests --parallel 12`
- `/build/bin/rust_storage_tests`
- affected C++ tests
- `git diff --check`

## Tracking Notes

- This is a deletion track. Prefer removing compatibility entry points over adding new adapters.
- Do not move network/tarcap transport, arbitrary EVM execution, receipt/contract execution, or public API object
  ownership into this storage cleanup.
- Do not remove pure C++ reference behavior from `cpp-reference`.
- Keep compatibility markers on any remaining C++ storage reads so future guard failures are actionable.
- Commit each landed slice separately.
