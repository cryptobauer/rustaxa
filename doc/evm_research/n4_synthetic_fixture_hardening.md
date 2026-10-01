# N4 synthetic fixture hardening — 2026-10-01

## Result and boundary

This slice closes the two fixture regression gaps recorded in the
[historical lifecycle report](n4_synthetic_native_transition.md). The runner
requires `witness.ordered_raw_writes == []` and rejects a missing or invalid list.
Backend Put, Commit and TrieSink counters must also be present integer zeroes.
The Rust parity test independently requires the observed Go write list to be empty.

Each engine has a separate literal complete input contract. Each contract is
compared with the whole parsed shared manifest before setup or lifecycle work.
This binds schema, synthetic flags, lifecycle identity/time/gas, the entire EVM,
DPoS, validator and hardfork configuration, reward facts, post-delegation Rust
balances and setup description. Missing keys, extra keys at any object level,
wrong types, changed values and changed array sizes fail closed. Object key order
and whitespace are not inputs. This is a fixed fixture, not a configurable
execution API: any future input change requires an explicit contract update,
source review and fresh execution in both engines.

The contracts contain only inputs. They do not supply expected effects, raw rows,
reads, rewards or executor outcomes. Go still constructs genesis independently
from config and runs actual Init/BeginBlock/EndBlock/Close. Rust still runs the
complete FinalChain fixture, real planner, bound session and `finish_rewards`.
The Go manifest author and period now also supply BeginBlock and witness labels.
The existing observer, source pins, oracle behavior, lifecycle controls, raw
origin negatives and request/period/authority negatives remain intact.

Both engines reject 306 drift cases: null/type changes at each node, removal of
every object field, unknown fields at every object, additions to every array and
value changes to every scalar. Go controls run the compiled harness with
`--validate-only`, which returns before genesis setup. Every rejection must have
a nonzero exit status and the input-contract error; complete stdout/stderr and
case paths are retained in the new Go evidence. Python also tests a nonempty raw
write list with all other counters zero, malformed or absent write evidence,
missing witness fields and nonzero effect counters.

Fresh output is retained separately as
[Go evidence](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_go_hardened.json)
and [Rust evidence](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_rust_hardened.json).
Both have zero measured effects. Rust returns zero reward and unchanged semantic
state. All 27 Go genesis raw rows still match Rust; the two Rust-only empty rows
and different read sets remain as described in the historical report. The old
manifest, frozen Go/Rust outputs and prior source-pin evidence are unchanged.

No production code, bridge API, storage module, routing, replay, adoption,
publication or root derivation changed. No supplied `data/` or historical snapshot
copy was accessed. Producer qualification and the real-window gate remain open.
This evidence establishes only the bounded synthetic cold nonboundary case.

## First-run validation evidence

Tier 1 plus focused consensus/storage bridge checks covers this test-only slice.
Complete first-run logs and command/exit records are under
`/home/fry/artifacts/n4-fixture-hardening-2026-10-01/`. The capture wrapper uses
exclusive creation for each log and record. Failures are retained.

| Log | Exit | Command |
| --- | --- | --- |
| `bridge-build.log` | 0 | `cmake --build /build --target rust_consensus_tests rust_storage_tests --parallel 12` |
| `consensus-bridge.log` | 0 | `/build/bin/rust_consensus_tests` |
| `go-fresh-contract-corrected.log` | 0 | `python3 experiments/evm_feasibility/synthetic_native_transition_reference.py /home/fry/artifacts/n4-fixture-hardening-2026-10-01/go.json` |
| `go-fresh-corrected.log` | 1 | `python3 experiments/evm_feasibility/synthetic_native_transition_reference.py /home/fry/artifacts/n4-fixture-hardening-2026-10-01/go.json` |
| `go-fresh.log` | 1 | `python3 experiments/evm_feasibility/synthetic_native_transition_reference.py /home/fry/artifacts/n4-fixture-hardening-2026-10-01/go.json` |
| `harness-unit.log` | 0 | `python3 -m unittest discover -s experiments/evm_feasibility -p test_synthetic_native_transition_reference.py -v` |
| `rewrite-fast.log` | 0 | `make rewrite-validate-fast` |
| `rust-nonboundary.log` | 0 | `env RUSTAXA_SYNTHETIC_REWARDS_EVIDENCE=/home/fry/artifacts/n4-fixture-hardening-2026-10-01/rust.json cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus nonboundary -- --nocapture` |
| `rust-rewards.log` | 0 | `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus final_chain::native_session::rewards::tests -- --nocapture` |
| `storage-bridge.log` | 0 | `/build/bin/rust_storage_tests` |

The first Go build failed with a uint64/uint32 modulo mismatch. After the explicit
frequency conversion, the next run rejected the positive manifest because a
mechanical edit had also changed the embedded JSON period literals. Those two
literals were restored to 1; the next full Go run passed. Neither failure changed
the pinned oracle or relaxed an expected result. Failed archives and full logs
remain under the persistent artifact root.

Focused nonboundary tests pass 4/4, including 306 Rust drift controls; the rewards
module passes 17/17. Consensus bridge tests pass 15/15 and storage bridge tests
pass 4/4. Both bridge targets built with 12 jobs. The inspected CMake cache has
`RUSTAXA_ENABLE:BOOL=ON`. No expensive differential or broad replay gate ran.

The fast workspace gate passed formatting, clippy, workspace tests, both
structural guards and whitespace validation. Existing lint warnings and the
existing ignored test remain. See `rewrite-fast.log` for complete results.

## Source freeze and independent review

Baseline: `214c87b583263333ac95561d9c6bf4b13f526bdd`, branch
`feat/rust/evm-state-db`. The lead implements directly. Requested lead route is
Sol medium; actual runtime metadata is not exposed through this tool interface.
One startup inventory found no callable account/quota telemetry tool. Current
quota is unknown; no historical sample is treated as a current balance. The
owner's 25% reserve and no-start rule at 30% or less remain in force.

Capacity was checked before the bounded Luna-medium read-only map. That route
started and completed with no edits or routing failure. Runtime metadata and
billing/usage remain unknown. No same-model implementation lead was spawned.
An independent Sol-medium reviewer reviewed frozen source, outputs and
validation records. No unresolved semantic/authority issue required Astra.

The [freeze record](../../experiments/evm_feasibility/fixtures/synthetic_native_transition_hardening_freeze.json)
binds changed source, fresh evidence, unchanged historical files and complete
validation logs. The source freeze time is `2026-10-01T18:14:34.463879+00:00`.
Pre-existing preparation in plan 08, the checkpoint, usage records and the task
prompt is preserved. Only this slice's checkpoint addition will be committed.


## Independent review and local closeout

The separate `gpt-6.1-sol`, medium reviewer started and completed after freeze,
with no implementation involvement and no blocking findings. The
[review record](n4_synthetic_fixture_hardening_review.md) confirms all 36 freeze
hashes, all five unchanged-history files against baseline, both full input
contracts, all 306 Go negative results, the Rust count and all final validation
logs. Requested routing succeeded; actual runtime metadata remains unknown.
The review record initially named `gpt-6-sol` as the requested route; the
reviewer corrected this documentation error against the explicit spawn request.
No routing failure occurred. No billing or quota usage is inferred.

The reviewer found one duplicate validation table row in the draft report. It
was removed after review. The original report-at-freeze artifact remains intact.
No frozen source, fixture, observed output or validation result changed after
review. Root verified every frozen hash again before local staging.

The historical report has an appended follow-up link; its old evidence and
limits remain as history. The checkpoint receives only this slice's new handoff
section. Pre-existing checkpoint preparation remains unstaged, along with the
other unrelated preparation files. This slice is closed with a local
`fix(evm)` Conventional Commit. No push is made. Producer qualification,
historical/mainnet completeness and the real-window acceptance gate remain open.
