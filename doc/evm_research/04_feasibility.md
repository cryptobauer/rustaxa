# Bounded execution feasibility checkpoint

Date: 2026-09-12. Research baseline: `6c33faaaf`. Implementation is isolated in
[`experiments/evm_feasibility`](../../experiments/evm_feasibility/README.md) on
`feat/rust/evm-state-db`. No production routing, protocol, original C++ file,
submodule revision, storage module or production Cargo graph changed. The original
three reports remain source assessments; this checkpoint adds executable evidence.

## Recommendation and stop/go decision

**Proceed with REVM interpreter plus a Taraxa-owned host/frame prototype. Do not
start a complete backend or select a production engine yet.** The unmodified
framework demonstrably rejects nonce skipping and exposes insufficient integer
widths. The interpreter exposes a usable CREATE/CREATE2 handoff without consulting
bounded account state and executes GASPRICE at full U256 width. Existing
`FinalChainNonce` supplies the correct wide state value without a shadow nonce.

This recommendation is conditional: we have not built a custom framework, a full
Taraxa frame stack or an EVM-to-native-kernel adapter. A custom framework could
still win if its replacement surface is smaller and every bounded consumer is
excluded. Current evidence favors explicit host ownership, not a proof that the
framework cannot be customized.

## Reproducible reference acquisition

The exporter executes immutable public node-v1.14.1 EVM pin
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local executor
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418` in disposable source trees. Both produce
identical JSON for this corpus; a fresh second run compares all bytes and checksums.
The [manifest](../../experiments/evm_feasibility/fixtures/manifest.json) records the
source identities, exporter hash, capture toolchain and complete synthetic-state
conventions. The Go module files come from each exact source tree. The exporter
contains transaction inputs, bytecode, fork flags and native operation order.
Unexpected code/state reads fail instead of supplying fabricated state.

Acquired corpus: 16 envelope cases, seven opcode cases, four state snapshot/flush
cases, six ordered native-map mutation stages and 14 account/slot commitment cases.
These are synthetic library fixtures, not signed block/transaction archives or a
network checkpoint. Genesis/config provenance, system/reward inputs and paired
state generations are not applicable to these direct library cases. No public
snapshot was downloaded and historical archive availability remains unresolved.
Local/public agreement here does not classify all effects of the two local EVM
commits, particularly staged persistence and malformed slashing inputs.

## E1–E5 evidence and limits

| Group | Executed result | Still open |
| --- | --- | --- |
| E1 engine depth | REVM v117 graph compiles with existing Rust domain crate. Framework actually rejects nonce 7 on a nonce-zero account; interface probes establish u64 nonce/account nonce and u128 price. Interpreter preserves price 2^128 and yields CREATE/CREATE2 without account loading. Native nonce + independent RLP reproduces Go wide-CREATE address. | Custom framework comparison, full nested creation, collision, child result insertion, failed creation nonce effects and frame rollback. Handoff is not completed execution. |
| E2 envelope | Both references agree on skips above u64, successor above U256, wide price, stale nonce charging, affordability, intrinsic failure and call/create cases before/after Cornus. Exact gas, balance, nonce, errors and return bytes captured. | Rust envelope implementation, complete post-state/receipt roots, zero sender, simulation, value-failure matrix and signed-wire admission. |
| E3 instructions | PUSH0 costs 2 in base; SLOAD case consumes 21,803 including intrinsic/push; set/clear SSTORE gives refund 19,800 and net gas 21,412. Old transient aliases work before Cacti; 0x5d fails before Cacti and works at Cacti. Nested CALL child TSTORE survives child REVERT. | Taraxa instruction table in Rust, full SSTORE matrix, stipends, memory, SELFDESTRUCT, all fork combinations and continued transactions. No general Ethereum SpecId selected. |
| E4 native writes | Existing-account raw write survives snapshot revert, ordinary value/logs revert; new-account lifecycle can erase raw writes. Native IterableMap insert/middle/last/final removal exports ordered exact writes and tombstones. Rust applies captured writes and independently matches all six storage roots. | Actual DPoS/slashing ABI-to-Rust-kernel adapter, native calls inside all call types, static/payability semantics, validator historical encodings, reward graph and exact account/storage combined roots. |
| E5 commitments | Independent Rust RLP + triehash matches 14 Go roots and exact leaf/disk account encodings, using native arbitrary-width nonce. Slot fixtures span embedded-node and short/long RLP boundaries. Persisted Go nodes are retained as separate evidence. | Physical node decoding/reopening, incremental updates/deletes, versioned RocksDB reads, code-bearing accounts, complete state roots, history and pruning. |

A returned zero word, absent raw row and a stored all-zero byte string must remain
different representations. The native map's final removal retains count
`00000000`; the root therefore remains nonempty. Root equality after replaying
captured writes proves a commitment path, not that Rust independently generated
the right native writes.

## Integration replacement map

The pinned source contracts are catalogued in [report 2](02_engines.md); the
[transaction trait](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/context/interface/src/transaction.rs)
was also checked against the compiled pin.

| Surface | Framework integration A | Interpreter/host B |
| --- | --- | --- |
| Envelope | Replace validation, pre/post execution; exclude bounded nonce/price consumers | Own envelope using existing canonical transaction/domain types |
| CREATE/account identity | Replace default creation frame, nonce bump, collision and emptiness consumers | Handle yielded create request with authoritative native nonce and Taraxa frame rules |
| Journals | Replace/restrict default account, checkpoint and raw/transient handling | Own explicit account lifecycle, ordinary, raw, transient and log views |
| Opcode behavior | Custom instruction/gas provider plus mixed dynamic-operation changes | Same selective changes; reuse common interpreter operations |
| Native execution | Custom provider and frame failure conversion | Narrow native-call port implemented around existing staged Rust kernels |
| Commitment/persistence | Separate Taraxa backend in either design | Same; retain exact raw leaves and existing database boundary initially |

No upstream source patch was required for the compiled probes. No claim of a
zero-patch complete integration follows. Prefer local extension implementations;
record any future unavoidable patch by upstream function/invariant, pinned revision,
regression fixture and upgrade review. A growing collection of replacements that
amounts to another interpreter is a stop condition. Keep Reth and Ethereum block
orchestration out of this experiment.

## Canonical native mutation design

Reuse `FinalChain::apply_dpos_mutation_transaction`, the staged DPoS snapshots,
`dpos_reward_graph`, existing slashing logic and concrete projection validation.
Keep application ordering, rewards planning, publication and recovery with their
current Rust owners. This experiment imports `rustaxa-types` directly and runs the
existing native tests; it does not duplicate consensus business transitions.

The existing projection map accepts alternative historical encodings and is not a
writer. Introduce a narrow staged native port or extract the existing kernel into
a domain unit when needed; do not make the interpreter depend on the application
manager or reenter a held FinalChain lock. The adapter needs operation-local access
to exact prior raw bytes and native semantic state, rather than regenerating the
whole concrete database from a compact snapshot.

Proposed transcript fields: operation/frame identity, account address, raw key,
exact put bytes or explicit delete, original order and account-lifecycle scope.
Keep ordinary writes and native raw writes separate until the reference flush
boundary; ordinary reads must not see the raw dirty map. TrieSink applies ordinary
writes before raw writes at an overlapping key. Preserve untouched historical
bytes, including removed map prefixes; do not select the first projection candidate
as a canonical rewrite. Native map removal must emit its swap, reverse-index,
tombstone and count operations, including the nonempty zero count.

Raw-write survival is conditional on account lifetime: a raw setter alone does not
make newly created account state irreversible. Logs retain normal frame rollback.
Transient storage needs its observed transaction-scoped behavior despite the
reference comment about journaling. All these observations are compatibility
requirements, not authorization to repair historical semantics.

## Next decisive experiments and blockers

1. Implement a small isolated frame driver around the yielded CREATE/CREATE2/CALL
   actions. Compare nested collision/revert and max-nonce growth against both Go
   references, preserving caller increment timing and child account lifetime.
   Do this before expanding opcode coverage or adding a production crate.
2. Expose/reuse the existing staged native kernel through a narrow test composition;
   capture one real DPoS mutation inside a reverting parent and compare exact raw
   writes, logs, gas and account/storage roots. Private FinalChain coupling and
   operation-specific serializer completeness are the current adapter blockers.
3. Audit historical kernel exceptions before replay claims. PLAN.md describes a
   pre-Magnolia pending-count correction and incomplete historical snapshot
   restrictions. This checkpoint did not execute those cases and cannot authorize
   treating corrected/current native behavior as historical reference parity.
4. Extend the codec probe to reopen captured physical nodes and apply deletions,
   then verify a complete account-with-native-storage root. Keep database-version
   compatibility and durability separate from root computation.
5. Acquire a provenance-checked archive or paired checkpoint before fork replay.
   Crypto E6 and operational E7–E12 remain outside this checkpoint. No performance
   estimate, migration guarantee or production cutover is supported yet.

## Validation and phase history

- `30671f4e8`: reproducible dual-reference synthetic corpus.
- `1c9efb917`: compiled framework/interpreter probes and independent commitments.
- Subsequent checkpoint adds opcode/native mutation evidence and this decision.
  All phases remain isolated on the same feature branch and are pushed; no
  alternate experimental branch is necessary for unlinked test-only code.
- Final isolated suite: seven tests pass; locked Clippy with warnings denied and
  format check pass. Fixture exporter reproduces both public/local files exactly.
- `make rewrite-validate-fast` passes, including existing native kernel/projection
  tests, workspace Clippy/tests, storage-boundary and bridge-inventory guards.
  The inventory script prints a missing retired `shims` directory diagnostic but
  completes successfully with zero shim directories; no guard was changed.
- Tier 1 plus isolated reference execution applies: no production storage,
  CXX/shim or runtime boundary changed. Storage bridge, full CTest, full-node and
  expensive differential gates were not invoked; no approval for them is inferred.
