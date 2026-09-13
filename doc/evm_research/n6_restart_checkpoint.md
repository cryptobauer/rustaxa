# Restart checkpoint: existing-network milestone

The owner requested an immediate checkpoint to start a fresh session with Luna
capacity. Milestone 10 remains in progress; 08 remains open and 09 remains
complete only within its declared synthetic scope. No acceptance or production
routing is implied by this checkpoint.

## Start the next session

Read `AGENTS.md`, `PLAN.md`, `doc/evm_research/README.md`, milestone 10 and this
file. Inspect repository/worktree state before edits. Continue on
`feat/rust/evm-state-db`; accepted implementation is pushed through `19e9e7e59`.
Subsequent feature-branch checkpoint commits are documentation only.

**Launch Luna first and confirm successful startup before allocating the other
agent slots.** Request `gpt-5.6-luna` explicitly, medium reasoning, for a bounded
read-only inventory of the missing epoch fields in the three EVM fixture files
listed below. Reuse that Luna thread for related settled helper work. Do not
substitute Sol/Astra if startup fails. Preserve the existing Sol implementation
and independent Astra/Sol review assignments for complex semantic work.

The old session had seven open threads including a completed historical audit.
Explicit Luna startup failed with `agent thread limit reached`. It did not run.
The exposed tools could neither close threads nor change an existing model;
completion/interruption did not establish released capacity. All workers were
asked to checkpoint and stop new work. No usage/billing conclusion follows.
Use a fresh session; merely resuming the old conversation has not been verified
to release its thread slots.

## Saved work (all pushed)

| Remote branch | Commit | State and next step |
| --- | --- | --- |
| `feat/rust/evm-state-db` | `19e9e7e59` implementation base | Integrated/reviewed bounded execution, native custody, historical policy, trace facts/serializer/refund sequences, and partial native inverse coverage; resume from here |
| `checkpoint/evm-epoch-integration` | `db8ec5e4d` | Source-reviewed epoch transport, poison handling, nonwrapping allocation and C++ fixture adaptation; integration fast gate blocked by Rust fixture epoch fields |
| `checkpoint/evm-trace-runner` | `b844c1896063eac32eab64fbd166e277ca6eca63` | Completed bounded default TraceRunner with targeted checks; final independent source/corpus review pending before cherry-pick |
| `checkpoint/evm-reward-scheduler` | `621241a863e721b3facba41c02b6dc1295e01818` | Explicit unreviewed WIP; runtime/session/publication/recovery changes and new Go reopen exporter, whose fixtures are stale |
| `checkpoint/evm-native-claims` | `f413b296af7b252f618ad0ef52d24666625196f6` | Explicit WIP claims adapters/oracle; Rust corpus comparison file is still a placeholder; final validation/review required |
| `checkpoint/evm-semantic-port` | `0e629e9830a014992dc57a747a776634c0177801` | Unregistered/uncompiled setCommission-only semantic-port draft; design approved, source not reviewed or validated |

WIP commits were preserved on separate branches, not accepted onto the feature
branch. Some worker WIP commits intentionally used no-verify; their clean worktrees
and saved commits do not establish validation. Fetch/cherry-pick only the intended
slice; worker branches contain overlapping older integration history.

Local worktrees remain available:

- Trace: `/tmp/rustaxa-evm-trace`, `task/evm-trace-runner`.
- Scheduler: `/tmp/rustaxa-evm-current-rewards`, `task/evm-current-rewards`.
- Claims: `/tmp/rustaxa-evm-native-simulation`, `task/evm-native-simulation`.
- Semantic port: `/tmp/rustaxa-evm-bootstrap-recovery`, `task/evm-checkpoint-account`.

## Immediate integration work

Epoch integration consists of commits `271dd258d`, `f60b5baa0`, `2d1f30a93`,
`d941fceed`, then C++ tests `db8ec5e4d`, all preserved on its checkpoint branch.
Its 37 Rust execution-domain tests and eight Rust-enabled C++ FinalChain/leaf
tests passed. CMake built `rust_consensus_tests` with 12 jobs. Review approved
source/contract, including permanent allocator exhaustion and poison checks.
The poison test uses malformed marker input at the fallible Go boundary; it does
not inject a failure after actual Go reconstruction.

`make rewrite-validate-fast` exposed E0063 initializer errors in:

- `rust/crates/rustaxa-evm/tests/persisted_period_reference.rs`
- `rust/crates/rustaxa-evm/tests/mixed_period_reference.rs`
- `rust/crates/rustaxa-evm/tests/support/mixed_recovery.rs`

The isolated test leaves must issue/echo nonzero current epochs and model actual
old/new epoch replacement on discard/reopen. Add the missing `state_api_epoch`
and `previous_state_api_epoch` fields without weakening identity assertions or
inventing an epoch that skips lifecycle validation. Then rerun focused fixtures,
fast gate, and relevant C++ tests before accepting this branch. Existing unrelated
consensus warnings remain; do not weaken checks to hide them.

TraceRunner worker reports eight facade tests, ten driver tests, eight simulation
tests and three serializer tests passing, plus strict focused clippy. It uses
one disposable journal without normal transaction resets, exact preceding state,
unchanged supplied nonces and a fresh collector per target. Review the final
commit before integrating. Native/nested/OpenEthereum modes remain outside scope.

Scheduler WIP requires lock-order/Fresh-vs-Live/recovery/discard/publication review,
current fixtures regenerated from both actual Go pins, and fresh targeted checks.
Earlier passing tests predate later WIP changes and do not validate its HEAD.

Claims WIP reuses kernels for claimRewards/claimCommissionRewards. Zero-stake
commission paths are explicitly unsupported pending witnesses. Finish actual-Go
corpus assertions, check typed/full-width account errors and exact zero-transfer
behavior, then validate/review. Accrued cancellation is already integrated;
three-period Go evidence is retained for future real publication/reopen tests.

Semantic-port WIP needs registration, privacy/compile fixes, refactoring only the
existing setCommission kernel through snapshot and authenticated checkpoint
adapters, exact-byte tests and review. Do not route claimCommissionRewards,
publication or sparse DposSnapshot state through this seam.

## State and architecture constraints

Original `/tmp/snapshot-litenode` is untouched. Only the independent qualified copy
`/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy` may be opened for bounded
read validation. Mainnet H is 25,706,949. Owner-reported likely build, suspected
validator identity and insufficient stake are recorded as provenance, not verified
production facts. No private keys are required for semantic reconstruction.

Current authenticated DPoS inventory has 23,278 live rows. Strict inverse decoding
plus 290 known validator/owner candidates explains 2,065; 21,213 remain unexplained.
Missing physical rows are never silently promoted to absence. Global delegator
completeness, historical/deleted-key coverage and slashing reconstruction remain
unqualified. Root independently reproduced the initial inverse report byte-for-byte;
expanded coverage has worker execution and independent source/hash review.

Lazy semantic point reads can execute localized native operations without inverting
every key. Current publication/recovery still requires complete DposSnapshot maps,
principal/reward-graph global invariants and snapshot/projection hashes. Do not
serialize a sparse cache as a complete snapshot. A later checkpoint delta-lineage
contract must be reviewed before changing that authority boundary. Existing Rust
owners, separate databases and general layout remain fixed.

No broad replay, expensive differential/fault campaign, protocol change, original
upstream C++ modification or production backend switch is authorized. Ask before
expensive validation under AGENTS.md. Lead exclusively owns shared `/build`.
Its artifacts currently reflect the epoch checkpoint branch; rebuild the desired
source before relying on subsequent C++ test runs.
