# S4 bounded persisted FinalChain path

Status: the first synthetic persisted-period path is implemented and independently
reviewed. This closes the bounded S4 composition milestone, not complete execution,
native/precompile coverage, historical replay, reference-binary reopen or cutover.

## Executed composition

`rustaxa-evm/tests/persisted_period_reference.rs` exclusively creates separate
application and concrete databases. The existing Rust FinalChain constructor uses
the known Go genesis root, which is then independently reproduced by the fresh Rust
concrete writer before any execution. Database identity is generated with OS entropy;
existing paths are refused. Imported markerless snapshots cannot use this constructor.

Actual signed legacy inputs pass through the existing transaction decoder and
`execute_final_chain_application_task`, using its `ConsensusExecutionPort` adapter.
The application supplies canonical block hashes and system facts. The bridge account
is proved absent and neither period is a pillar period. FinalChain's system planner
therefore selects an empty system stream.

The journal-backed REVM driver executes a transfer, CREATE whose initcode writes
slot zero and installs runtime `60003560005500`, then a CALL after reopening that
changes the slot. The storage writer derives each intermediate root from actual
mutation plans. Exact five-field account rows form the projection; the existing
StateAPI transaction/result and FinalChain receipt codecs preserve the separate
encodings. No expected post-execution root is assigned as an execution result.

The fixture has no validators, configured yield zero and transaction gas price
zero. Magnolia, Aspen part one and Cornus are active; Aspen part two, Cacti and
corrections are outside its periods. The adapter encodes the actual Rust-planned
rewards statistics and proposes a neutral rewards root. Existing FinalChain native
and reward kernels independently validate the complete proposal before commit
preparation. The empty prior native catalog is retained; these facts do not establish
nonzero reward or native invocation parity.

## Persistence and continuation

The existing concrete execution marker is synchronously staged before execution.
Prepared row contents remain in memory, including content-addressed nodes retained
at each intermediate root. The application persists its pending publication first.
`FinalChain::validate_pending_external_evm_commit` checks the exact durable intent
and prior published descriptor under the existing serialized application owner.

The concrete lifecycle then writes one synchronous RocksDB batch containing CF1–CF5
rows, descriptor, exact application-approved provenance, monotonic catalog and
pending-marker deletion. Only the existing FinalChain pipeline publishes the
application generation. A read accessor cannot expose the underlying writer;
discard invalidates older preparations, and uncertain metadata writes poison the
handle until reopen. The normal fixture validates paired recovery and continuation;
the bounded callback-interruption cases below exercise ambiguous commit recovery.

All application, lifecycle and concrete reader handles close. Both databases reopen,
`recover_final_chain_application_state` validates the pair, and the second period runs
through the same application entry point. Tests compare stored receipts before and
after reopening, published gas/root/hash and zero minted reward, exact account/code/
slot bytes, and every retained CF1–CF5 row. The finite complete-history read adapter is
private to these exclusively created fixture databases; it is not an imported-state
coverage flag or a general period accumulator.

The same signed-period fixture also runs through the
[ordered observer overlay](s4_ordered_overlay_evidence.md). Each transaction submits
only its settled delta, and the next transaction borrows the lifecycle's fixed
prepared execution view. This path bypasses the old adapter's account, slot and
code caches. Sealed phase output supplies the exact changed-account set and rows
for projection, then selects the application-approved atomic commit. The original
cumulative case and its assertions remain; both paths match the same Go intermediate
roots, receipts, physical maps and close/reopen continuation. The private fresh-state
absence knowledge remains a fixture-only wrapper around both read paths.

## Bounded interrupted publication

Two additional synthetic cases interrupt the commit callback after the application
has durably recorded its exact intent. One drops the concrete handle before writing
prepared contents; the other performs the real atomic concrete commit and loses its
acknowledgment. Immediate observation is unavailable in both cases, so the existing
application classifier retains its pending intent. The test proves that no header or
receipt is visible before publication, then closes both database owners.

`ConcreteStateLifecycle::inspect_existing` reopens concrete state read-only without
assuming the application's prior root. It validates existing canonical lifecycle
metadata, chain identity and descriptor/provenance/catalog/marker consistency; it
cannot create, repair, adopt or publish state. Mutable `open` still requires an exact
expected committed descriptor. The supplied upstream snapshot has no Rustaxa
provenance and cannot be adopted through either method.

After reopening, the existing `recover_final_chain_application_state` owner discards
the uncommitted marker and allows an explicit period retry, or publishes the exact
already-committed period. Its recovery-only leaf refuses execution, rewards and new
concrete commits. Exact CF1–CF5 maps match the Go genesis or committed-period rows at
the interruption boundary. Repeated recovery preserves header/receipts, execution
counters and all observed lifecycle metadata. Both cases continue to the same period
two history and database identity, with one concrete generation per committed period.

These are deterministic callback-interruption tests over real databases, not process
kills, disk failures, torn writes or a general fault campaign. Full S7 durability,
import, pruning and operational recovery acceptance remain open.

## Independent oracle

`experiments/evm_feasibility/s4_reference.go` executes real Go EVM, TransitionState
and TrieSink code at both existing pins. The Python driver requires exact agreement
between revisions and with the committed fixture and manifest bytes. The public
reference is `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`; the local reference is
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.

| Boundary | Gas charged | Concrete root |
| --- | ---: | --- |
| Genesis | — | `614520191ecb0ce0fdd9fb55b46567966d32c626a4c0f49f6aae6e7fe487bf03` |
| Transfer | 21,000 | `f1963b70bc612bc815e66026dac2bb2a8a0f949c3a7f4e11927c16045ba5f023` |
| CREATE / period 1 | 75,532 | `5bf1943e6139502ba9005649c7798bd1ec5b19d6fc61b8bb66b33d1cc5ea242c` |
| CALL / period 2 | 26,201 | `ddfbf988da333ba8947fb76a9b668682f889d9ffc92ea065b4fb31d7c2304f14` |

The oracle includes both the public batched lifecycle and the concrete observer's
per-transaction intermediate-root boundary. Their transaction results, receipts,
final accounts and roots agree for this fixture. Concrete mode retains one additional
CF2 node at the transfer root; shared node bytes agree. The Rust persisted fixture
matches the concrete-mode history: CF1–CF5 counts are respectively `1/3/4/1/1` after
period one and `1/4/6/2/2` after period two.

The Go row model projects the actual PendingBlockState key rule, including the
big-endian period suffix on CF3/CF5; it serializes/reloads those rows for continuation.
It does not open Go RocksDB. Latest-view CF6/CF7, operational metadata and reference
binary reopen are outside this row comparison. Neither batching-mode agreement nor
the fresh-journal adapter generalizes to native/raw cache lifetimes, nested frames or
account deletion after earlier slot writes. Separate ordered-overlay storage tests
cover sequential slot deletion and account deletion/recreation; this signed-period
fixture does not claim those execution or native-period cases.

## Reproduction and remaining gates

Both Go references reproduced the committed bytes. The two focused execution
test targets, storage's 118 tests (one separate copied-snapshot test explicitly ignored),
all four storage bridge tests and the repository fast gate passed. The changed
composition test also passed strict targeted clippy. These checks use tiny
synthetic data and do not replace the approval-gated campaigns below.

```sh
python3 experiments/evm_feasibility/s4_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test s4_driver_reference
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test persisted_period_reference
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-storage concrete_state::
cmake --build /build --target rust_storage_tests --parallel 12
/build/bin/rust_storage_tests
make rewrite-validate-fast
```

Full S3 frame coverage, S5 native/precompile completeness and S6 period/query/header
parity remain open. Snapshot coverage/provenance gaps, S7 interrupted recovery/import/
pruning and S8 operational acceptance remain separate gates. Broad replay,
differential/fault campaigns and sustained workloads still require the documented
approval. Production routing and protocol changes remain unauthorized.
