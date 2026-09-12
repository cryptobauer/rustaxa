# Bounded execution feasibility

This directory is an isolated experiment, outside the production Rust workspace and
CMake routing. Keep it on `feat/rust/evm-state-db`; no experimental production branch
is needed while all changes remain here and in research documentation.

## Phase 1: reproducible synthetic reference fixtures

Run `python3 experiments/evm_feasibility/reference.py` from the repository. Requires
Go and both pinned EVM revisions in the submodule object database. It exports each
revision into a disposable directory, adds the exporter, executes with the pinned
Go module graph, and compares all output bytes and hashes. `--record` explicitly
refreshes evidence; inspect changes before accepting them. It never edits a source
submodule or opens a node database. Go dependency downloads may require network access.

The manifest pins both references, exporter checksum, toolchain used to capture,
and synthetic environment. The exporter is the complete input specification.
Fixtures cover 16 envelope cases, four state snapshot/flush cases and 13 account/
slot trie cases. Account leaves include nonce 2^256 and 2^264. Slot pairs use adjacent
prehashed keys to exercise embedded child nodes and RLP length transitions.
Persisted Taraxa node bytes are recorded separately from commitment leaves.

The two references give identical bytes for this corpus. This is not historical
network replay, transaction-wire admission, complete E1–E5 coverage, RocksDB
compatibility or a production parity claim. The raw-write cases directly exercise
TransitionState, not DPoS ABI dispatch or nested EVM CALL opcodes. Envelope cases do
not yet capture full post-state roots. Unexpected backing-state reads panic.

Observed: nonce skipping and successor above U256 work; pre/post Cornus failure
nonces differ; stale nonce charges full gas cap. Existing-account raw writes survive
snapshot revert while ordinary writes/logs revert. Transient writes survive revert
but clear on transaction commit. A newly created account is removed by revert,
including its raw writes; without revert an otherwise empty new account is deleted
at flush even when it has dirty storage. These are compatibility observations, not
permission to fix the reference semantics.

Validation: both pinned exporters executed and a fresh second execution reproduced
all fixture bytes. No production Rust/C++/storage module changed, so focused fixture
validation applies; expensive repository differential gates are not invoked.

## Phase 2: compiled integration and commitment probes

Run `cargo test --locked --manifest-path experiments/evm_feasibility/Cargo.toml`
and `cargo clippy --locked --manifest-path experiments/evm_feasibility/Cargo.toml --all-targets -- -D warnings`.
Use Rust 1.98.1 (capture toolchain); the lockfile pins the resolved graph and the
REVM source revision is the research pin. Five tests pass. The framework actually
rejects a nonce skip before execution; its transaction nonce, gas price and account
nonce interfaces compile as u64/u128/u64. This rejects an unmodified framework,
not every possible custom framework design.

The interpreter GASPRICE probe retains 2^128. CREATE and CREATE2 yield a frame
request without reading account state; the fail-closed host panics on any such
read. Existing `FinalChainNonce` retains 2^256 without a bounded shadow. This tests
the handoff, not a complete nested frame executor, collision or rollback parity.
ISTANBUL is used only as an explicit probe profile, never as a proposed Taraxa fork.

Independent Rust RLP + `triehash` matches all 13 Go roots and exact account/slot
leaves. Nonces use existing Rust domain types. Persisted Taraxa nodes are evidence,
not parsed by the Rust calculator. No claim of incremental storage compatibility
follows from root agreement. No upstream library patches were needed for these
probes; full transaction, frame, journal, native and mixed-gas integration remain.

## Phase 3: native mutations and opcode observations

The final corpus also includes seven opcode cases (including a real CALL child
that TSTOREs and REVERTs) and six native IterableMap stages. The native exporter
records each ordered write, including middle-item swap/removal, final removal,
and exact count bytes. Seven Rust tests now pass, including independent roots
from those raw writes and wide CREATE address comparison. The primitive map test
is not a DPoS business-kernel replacement or proof of complete native dispatch.

The [checkpoint report](../../doc/evm_research/04_feasibility.md) records measured
results, the framework/host replacement map, canonical mutation design, historical
compatibility blockers and the next bounded experiments. The repository fast gate
also passed. Full backend, production routing and protocol changes remain excluded.

The final corpus adds raw length 27 beside length 28, exercising exactly 31- and
32-byte child hash encodings. The final commitment inventory is 14 cases; phase 2
originally contained 13.

## Phase 4: bounded nested creation driver

The [creation checkpoint](../../doc/evm_research/05_creation_frames.md) adds 16 dual-
reference scenarios and a Rust frame driver. Eight isolated tests now pass, including
exact gas, return/error fields, account bytes and full account-state roots for nested
CREATE/CREATE2, wide successors, collisions, child/parent revert and code deposit.
Manifest schema two hashes both Go exporter files. Production remains unlinked.

The driver rejects unsupported host work, nonzero creation value and opcodes outside
its fixture subset. It uses clone checkpoints and a fixed valid envelope; neither is
a proposed general executor. CALL/native kernels, storage journals, full gas/fork
policy, physical persistence and historical replay remain explicit next gates.
