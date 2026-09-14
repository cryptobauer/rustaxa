# Restart checkpoint: existing-network milestone

Milestone 10 remains in progress; 08 remains open and 09 remains complete only
within its declared synthetic scope. The Luna-first continuation integrated and
pushed the reviewed epoch, default TraceRunner, staged claims and shared
setCommission slices. Implementation is accepted through `02281c89d`, with
supporting documentation pushed through `bb2034611`. Scheduler work below remains
unaccepted. No production backend switch or milestone acceptance is implied.

## Start the next session

Read `AGENTS.md`, `PLAN.md`, `doc/evm_research/README.md`, milestone 10 and this
file. Inspect repository/worktree state before edits. Continue on
`feat/rust/evm-state-db`.

Launch Luna first, explicitly requesting `gpt-5.6-luna` with medium reasoning and
an independent task context, and confirm successful startup before allocating
other slots. Reserve that thread for bounded helper work. Retain Sol high for
implementation and independent Astra/Sol review for semantic acceptance.

The fresh continuation successfully ran Luna first. Its recorded runtime
`turn_context` confirms `gpt-5.6-luna`, medium reasoning. Reused helper tasks mapped
epoch fixtures, C++ checks, semantic-port registration, bridge checks and remaining
N4 qualification. Independent reviews corrected overbroad helper suggestions;
helper maps never approved their own semantics. All six workers/reviewers started
successfully; no routing or capacity failure occurred. The previous session's
`agent thread limit reached` failure remains historical, not a current quota or
billing conclusion. Exact assignments are in [the ledger](n6_agent_handoffs.md).

## Integrated continuation

| Slice | Accepted feature commits | Evidence and limits |
| --- | --- | --- |
| StateAPI epochs | `6b73513d6`, `7e317fb7a`, `da32ce9fc`, `89a361075`, `1d151e699`, `7b2c29ac3` | Nonwrapping owner epochs, poison checks, truthful fixture discard/reopen identities, durable-intent comparison independent of unpersisted epochs; actual post-reconstruction Go fault injection remains unproved |
| Default structured TraceRunner | `b936558ab` | Independent dual-pin trace/refund reproduction; one disposable journal, supplied nonces, fresh target collectors; native/nested/OpenEthereum/RPC scope remains open |
| Staged claims | `d620a5711`, `12fa2f48f` | Actual-Go corpus assertions, full-width and typed account errors, exact zero effects; unpublished semantic sessions only, zero-stake commission unsupported |
| Shared setCommission kernel | `c3f53a0e2`, `02281c89d` | One kernel via complete-snapshot and authenticated checkpoint-row adapters; no sparse-snapshot or publication authority |

The epoch fast gate exposed two additional issues beyond the original three
fixture files: rejected descriptor-only preflight in `tests/contracts.rs` needed
an explicit zero epoch, and durable pending-intent comparison incorrectly compared
a decoded zero sentinel to a live epoch. Both are corrected and independently
reviewed. Epoch identity is never serialized as durable authority.

The bridge ratchet also exposed added surface. Reviewed contraction now borrows
discard marker bytes plus an epoch scalar, centralizes epoch/descriptor checks in
the native owner, preserves the essential pre-zip result-count check, and moves
raw period-envelope assembly into the existing Rust types codec. Certificate
bytes and weights move together without cloning. The bridge has 4,619 lines,
below the previous 4,621, with unchanged carrier/function/handle counts. No
inventory guard or test expectation was weakened to make the integration pass.

The checked-in pre-commit gate passes on the integrated feature branch. Focused
lead evidence includes 38 execution-domain tests, two codec byte tests, persisted
and mixed fixtures (3/5), 64 native-session tests, three commission rule-order
tests and the six-case dual-pin driver/journal corpus. The epoch boundary rebuilt
and passed eight C++ leaf/FinalChain tests with `RUSTAXA_ENABLE:BOOL=ON` and CMake
`--parallel 12`; targeted C++ formatting passed. All touched C++ paths are
main-only, with no original upstream C++ modification.

See [trace evidence](n3_api_parity.md), [claims evidence](n2_claims_evidence.md),
and [commission evidence](n2_commission_semantic_port.md) for exact scope.

## Scheduler work still under review

The original saved checkpoint is `checkpoint/evm-reward-scheduler` at
`621241a863e721b3facba41c02b6dc1295e01818`. Current corrections are in
`/tmp/rustaxa-evm-current-rewards` on `task/evm-current-rewards`; inspect its dirty
state before integrating. Do not cherry-pick overlapping older integration
history. Lead alone owns shared `/build`.

Sol is correcting explicit joint-startup epoch authority, actual native-session
basis wiring, publication-marker retention through runtime installation,
idempotent recovery and verified-discard transitions. Independent review still
requires fixes for old-publication cache rewind, rejected-report completion,
pre-marker discard cleanup, exact live pending-intent binding and same-process
retry after a successful discard followed by a failed verification read.

A second Sol worker owns a new joined runtime/native-serializer test file,
coordinated with the implementation owner. It must compare actual Rust scheduler
and ordered slashing writes to both regenerated Go constructor/reopen witnesses;
fixture self-comparison is not parity. The exporter reconstructs actual Go
StateTransition over committed in-memory state. It does not prove actual
StateAPI discard or interrupted durable recovery by itself. Final independent
review and fresh integrated gates are required before accepting the slice.

## Other saved worktrees

- Trace: `/tmp/rustaxa-evm-trace`, saved `b844c1896`, integrated.
- Claims: `/tmp/rustaxa-evm-native-simulation`, completion `03f3598d3`, integrated.
- Semantic port: `/tmp/rustaxa-evm-bootstrap-recovery`, completion `dd64b1125`, integrated.
- Epoch checkpoint: `checkpoint/evm-epoch-integration`, saved `db8ec5e4d`, integrated
  with the reviewed fixture/runtime and bridge-contraction corrections above.

Normal commits do not establish that `.githooks/pre-commit` ran when
`core.hooksPath` is unset. The lead explicitly invoked the checked-in hook for
integrated validation. Saved WIP and a clean worktree never imply acceptance.

## Existing-network and authority constraints

Original `/tmp/snapshot-litenode` is untouched. Only the independent qualified copy
`/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy` may be opened for bounded
read validation. Mainnet H is 25,706,949. Owner-reported likely build, suspected
validator identity and insufficient stake remain provenance, not verified
production facts. No private keys are required for semantic reconstruction.

Current authenticated DPoS inventory has 23,278 live rows. Strict inverse decoding
plus 290 known validator/owner candidates explains 2,065; 21,213 remain unexplained.
These are unresolved reconstruction/qualification inputs, not proof that data is
unavailable externally. Missing physical rows are never silently promoted to
absence. Global delegator completeness, historical/deleted-key coverage and
slashing reconstruction remain unqualified.

Lazy point reads and localized semantic kernels do not authorize complete state
publication. Current publication/recovery still requires complete DposSnapshot
maps, global principal/reward-graph invariants and snapshot/projection hashes.
Do not serialize a sparse cache as a complete snapshot. Any future checkpoint
delta-lineage contract needs review before changing that authority boundary.
Existing Rust owners, separate databases and general layout remain fixed.

The 19 copied-head transactions have bounded receipt/gas/root preflight evidence.
Full reward/system-input closure, complete native/catalog qualification,
non-genesis paired adoption and real full-period replay/recovery remain open.
Continue bounded local reconstruction before declaring an external data blocker.
No broad replay, expensive differential/fault campaign, protocol change, original
upstream C++ modification or production backend switch is authorized. Ask before
expensive validation under AGENTS.md.
