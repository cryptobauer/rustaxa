# Persisted mixed-period evidence

Status: M1–M6 complete for the declared bounded corpus, independently reviewed on
2026-09-13. Validated implementation head: `77a8e7d95`. One RPC validation anomaly
is recorded below; only completed runs are counted as passes.
This is the bounded composition described in [09](09_mixed_period_milestone.md),
under the broader [S0–S8 plan](08_implementation_plan.md).

## Reproduction and reference boundary

The signed four-period workload, configuration, source pins and artifact hashes are in
[`mixed_workload_manifest.json`](../../experiments/evm_feasibility/fixtures/mixed_workload_manifest.json).
The initial genesis/rewards qualification remains independently reproducible.

```sh
python3 experiments/evm_feasibility/mixed_period_reference.py
python3 experiments/evm_feasibility/mixed_period_reference.py --scenario full
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test mixed_period_reference
```

The Go exporter runs unmodified and disposable instrumented source archives for both
pins. It compares public batched, local batched and local concrete-observer execution.
Raw setter calls retain operation order and repeats; refund counter and applied refund
are observed separately. All execution facts and roots agree. Individually enumerated
extra observer trie nodes in CF2/CF4 are allowed only as retained-node differences;
shared bytes and CF1/CF3/CF5 agree exactly. This is a Go TransitionState/TrieSink row
oracle, not a Go RocksDB reopen result.

The Rust test exclusively creates separate application and concrete databases. It uses
existing FinalChain ownership, signed input decoding, the EVM journal/driver, staged
native kernels, the rewards planner, ordered concrete phases and the existing atomic
publication/recovery protocol. Expected roots are assertions after computation, never
executor outputs. The original light-node snapshot is not opened by these tests.

## Reviewed integration contracts

- Native account reads and mutations retain full signed balances and exact existence.
  A zero debit ensures existence without introducing a zero-credit touch.
- Invocation contexts bind original request/projection identities, full frame facts,
  consumed reads and ordered effects. A second private staged session replays native
  kernels; surviving transaction projections own ordinary rollback results.
- The complete monotonic raw catalog is distinct from consumed reads. Explicit Go
  absence probes are checked as absent; expected nonempty rows cannot be imported to
  fill a missing write.
- Reward execution consumes the opaque application-prepared plan once. Aggregate
  eligibility/delegated-amount rows flush at the terminal EndBlock boundary.
- Ficus activation comes from existing immutable PBFT configuration through Rust
  application wiring. No CXX payload or production execution routing changes.
- Empty calldata remains a marker-bound planning hint. Native envelope validation uses
  sequential code state, so CREATE followed by an empty-data CALL executes new code.

## M1–M6 acceptance matrix

| Slice | Accepted bounded evidence |
| --- | --- |
| M1 | Actual Go native genesis, nonzero real rewards-plan witness, signed four-period manifest, exact staged account/raw/reward/context contracts, independent source review |
| M2 | CREATE/CALL/storage, deletion observed after reopen, nested revert/restored-slot flush, reverted SELFDESTRUCT, retained account/code/raw/orphan CF5 reads |
| M3 | Eleven period-two native invocations, repeated calls across/within transactions, business failure/native out-of-gas, parent rollback, stateless SHA2/MODEXP interleaving, subsequent reopened periods |
| M4 | Delegate, V2 undelegate and delayed confirmation through existing kernels, full-width custody effects, exact ordered iterable writes and retained LE32 zero counts |
| M5 | Real DAG/certificate/reward-plan inputs, nonzero fees and minted rewards, exact receipts/refunds/roots, existing FinalChain header codecs and hashes, delayed votes `100/100/200/100`, proved non-pillar inputs |
| M6 | Before-write and lost-acknowledgment interruptions in period two, repeated owner-driven recovery, all-period continuation, seven malformed-report cases and clean retry, unchanged rejected generations and exact uninterrupted output comparison |

The complete corpus contains 31 signed transactions and 18 consensus-native invocations
(`0/11/4/3` by period). Each period closes and reopens the real RocksDB owners. Exact
CF1–CF5 comparisons use the concrete-observer boundary; lifecycle metadata/catalog rows
outside those Go column families are checked separately through Rust owner observations.
Stored and materialized headers use the existing FinalChain codec and declared signed
PBFT input. Their bytes, receipts and execution counters also match an uninterrupted run;
no Go EVM header is invented as a reference.

The seven malformed reports are: stale native value, missing prior catalog entry,
incorrect invocation order, failure mislabeled as parent rollback, wrong prior descriptor,
inconsistent prepared intent, and stale reward-account read. Unrelated hashes/bindings
are recomputed so each test reaches its named semantic or lifecycle check. Missing catalog
rejection occurs in the real atomic commit approval; its retained pending intent is
reconciled before retry. Neither committed descriptor advances on a rejected report.

The zero-count fix accepts exactly four zero bytes only at known prior iterable count
keys when the semantic map becomes empty. Restart-round-trip, wrong-width, nonzero and
non-count negatives pass. A map created and completely removed within one transaction is
not covered by that count exception. The selected workload does not require this case.

Cornus, Magnolia and Ficus neighboring-height tests exercise admission, nonzero staged
rewards and conditional confirmation serialization. These are focused Rust boundary
checks; the full Go corpus retains its declared activation-at-genesis configuration.

## Validation checkpoints

The complete mixed integration binary contains five tests: the four-period corpus,
two adapter unit tests, deterministic recovery and malformed-report/retry coverage.
The strict targeted Clippy command is:

```sh
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm --test mixed_period_reference --no-deps -- -D warnings
make rewrite-validate-fast
```

The final workspace fast gate passed, including 1,380 consensus tests and all five
mixed integration tests, formatting, normal Clippy, storage/bridge boundary guards
and whitespace validation. The checked bridge surface remains 4,621 lines; final
bridge source matches the pre-milestone baseline. Existing unrelated workspace
Clippy warnings remain; the new mixed integration target passes strict Clippy.

The Rust-enabled subsystem build used:

```sh
cmake --build /build --target rust_consensus_tests state_api_test rpc_test taraxad --parallel 12
/build/bin/rust_consensus_tests
/build/bin/state_api_test
/build/bin/rpc_test
/build/bin/taraxad --version
```

`RUSTAXA_ENABLE=ON` was verified. Results: 14 consensus bridge tests, 3 enabled
StateAPI tests, 50 RPC tests and CLI smoke passed. The preexisting disabled
`dpos_integration` and `eth_mainnet_smoke` tests were not counted as passes. No original
upstream C++ file or storage module was changed by this milestone.

One refreshed RPC attempt stalled for approximately 113 seconds in
`RPCTest.eth_syncing_uses_live_status_reader` and was terminated. A bounded
`timeout 60 /build/bin/rpc_test` rerun completed all 50 tests in 3.18 seconds;
earlier full runs also passed. The stalled test constructs `NewEth` and a live-status
callback rather than the changed native/FinalChain path. This identifies the observed
scope, not a proven cause: the stall did not reproduce and its cause remains unresolved.
Debugger attachment was unavailable under the existing ptrace policy, which was not
changed. No hang fix is claimed, and the interrupted attempt is not counted as a pass.

## Scope retained after this milestone

This work does not complete S5–S8 or authorize production routing. Other native methods,
slashing, remaining precompiles, all historical activation combinations, simulation/
estimation/tracing, qualified real-data replay, snapshot import, pruning, disk-fault
campaigns and operational/performance budgets remain separate gates. The likely snapshot
producer and unknown capture command remain provenance limitations; synthetic parity
cannot resolve them. Pre-Magnolia staged rewards and broader pre-Ficus custody/reopen
parity remain unsupported. Focused fork-boundary tests are labeled as Rust component
coverage, not a multi-era Go staged parity claim.


| Acceptance area | Remaining beyond the declared mixed corpus |
| --- | --- |
| E1 engine | Complete frame/call/depth/value/code matrix and dependency upgrade guard |
| E2 envelope | Complete signed admission/error/post-state matrix and simulation |
| E3 rules | Remaining storage/stipend/memory/lifecycle cases and activation combinations |
| E4 native | Other DPoS/slashing/system methods, historical exceptions and broader raw/ordinary overlap |
| E5 state | Complete lifecycle/codec fuzzing and qualified real-state closure beyond finite synthetic history |
| E6 crypto | Remaining Falcon/P256/BLS and malformed/gas/fork matrices and dependency review |
| E7 period replay | Provenance-qualified historical/genesis/fork replay beyond this synthetic sequence |
| E8 APIs | Full query/call/estimate/trace/delayed/pruned-period coverage |
| E9 recovery | Process-kill/write-error injection at all durability boundaries beyond deterministic callbacks |
| E10 faults/pruning | Power-loss/storage-fault and interrupted import/pruning campaigns |
| E11 migration | Qualified snapshot import, reference rollback and catch-up |
| E12 operations | Sustained matching-hardware/data measurements and agreed resource budgets |
