# Restart checkpoint: escrow DryRunner complete

## Current state

Branch: `feat/rust/evm-state-db`. Preparation is committed at `4148a1a` after
separate Sol review and quota-reader corrections. Decoded metadata is committed
at `86e74ff`. The latest implementation commit includes the completed
[ABI/admission report](n2_validator_info_abi.md), at `8945453`. The latest local
commit adds [staged escrow entry](n2_staged_escrow_transfer.md); use `git log`
for its exact ID.
No push or production routing is authorized.

The [metadata report](n2_staged_validator_info.md) records the shared pending and
historical session adapter, actual dual-pin Go corpus, complete logs and frozen
independent Astra review. Five targeted tests, workspace fast checks, an ON
consensus bridge build with 12 jobs, and all 15 bridge tests passed. The first
owner-fixture failure and independent-review gas-funding correction are preserved
in `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

The subsequent ABI/admission slice passed seven targeted Rust tests, both pinned
21-case Cacti oracles plus diagnostic reproduction, workspace fast checks and a
rebuilt ON consensus bridge with all 15 tests. Frozen independent Astra review
accepted it without findings or corrections. Scope is staged metadata over
consistent snapshots, with exact ABI/admission and synthetic active-Cacti
evidence. Sparse owner/info distinctions, pre-Cornus session admission, delegate,
call-code and delayed/nested/OpenEthereum native traces remain open. Historical
simulation here is synthetic and does not certify real inputs.
Milestone 10 and N1–N6 remain open. Supplied data and historical copies were not
opened or changed. The earlier N4 hardening remains complete at `5bfdf494c`;
its reports and [checkpoint history](n6_restart_checkpoint_history_2026_10_01_post_hardening.md)
are retained.

## Next action

Continue the active [root prompt](../../next_executable_slice_prompt.md) and
[remaining queue](n6_remaining_branch_queue.md) under the
[slice workflow](../codex_slice_workflow.md). The frame/API slice passed all five
integration tests, dual-pin fixture reproduction and workspace fast checks.
Independent frozen Astra review accepted it without findings. It covers eight
actual frames and sixteen historical probes over synthetic in-memory readers.
The subsequent metadata DryRunner slice passed all six persisted simulation
tests, reproduced nine actual dual-pin cases and passed workspace fast checks.
Independent frozen Astra review accepted it without findings. Thirty-six Rust
metadata simulations cover two physical reader reopens and fresh sessions for
each probe. Matching semantic history remains fixed; no network adoption or
semantic-owner reopen is claimed. The first intrinsic-gas fixture discovery and
its retained/additional cases are in the report and complete logs.

Native metadata estimation also passed targeted tests, actual dual-pin/C++
reproduction and workspace fast checks; independent frozen Astra review accepted
it without findings. The unchanged C++ search consumes 24 exact actual-Go probes,
and Rust runs 96 fresh simulations across reader reopen with matching results.
The direct native structured facade passed 30 focused tests, dual-pin actual
TraceRunner reproduction and workspace fast checks. Independent Astra contract
and frozen final review accepted it without findings. One private real pending
port, journal and sequence retain live current metadata across prefix/targets;
28 repeated trace runs span two physical reader reopens. The API explicitly
requires method-specific live native reader compatibility. Do not infer full
delayed-query, nested, period-zero or OpenEthereum parity from this coverage.

The exact active escrow-entry slice also passed five targeted consensus tests,
four frame/support tests, 12-case dual-pin reproduction, workspace fast checks
and an ON consensus bridge build with 12 jobs plus all 15 tests. Independent
Astra contract and frozen final reviews accepted it without findings. Native
execution is a shared pure helper; all value transfer/rollback stays in the EVM
frame. Inactive/trailing input and pre-Cornus session limits remain explicit.

Continue other ready native-method coverage from the remaining queue. Luna mapped
`reDelegate(address,address,uint256)` as the next bounded gap: post-fix distinct
validators, partial principal, zero rewards. Reuse `apply_dpos_redelegate` and
its destination helper; their account parameters still use `HashMap` while the
underlying reward/removal kernels already accept `DposAccountPort`. A narrow port
conversion is needed before session wiring. Existing custody serializers have
source-removal and destination-delegate primitives, but exact two-validator
operation ordering needs a new actual-Go dual-pin corpus and semantic review.
Do not claim same-validator historical corrections or reward-bearing scope.
Read selected kernel ranges 9072–9330 and the custody serializers only.

Current root log confirmed `gpt-6.1-sol` medium. Luna startup map ran on
`gpt-6-luna` medium; independent metadata review ran on `gpt-6-astra` medium.
There were no routing failures. Compatible reviewer/helper threads can be reused
in this session; fresh startup must confirm routing again. Account allowance was
25% after escrow validation and review. Read fresh telemetry at startup and
before new work; do not reuse this observation. At 20% remaining, stop new work
and finish only the in-flight atomic closeout. Unknown telemetry after one
bounded refresh also stops new work.

The [redelegation reference corpus](n2_redelegate_observations.md) now captures
12 actual dual-pin cases, 13 attempts, failure read prefixes and repeated exact
raw writes. Both uninstrumented controls and fixture reproduction passed,
workspace fast checks passed, and frozen independent Astra medium review accepted
the slice without findings. The first read-count assertion was corrected after
actual Go showed zero storage reads on the cached repeated call. Storage vectors
do not prove all API lookups or read/write interleaving. A narrow generic account
port conversion is complete: both helper parameters now use `DposAccountPort`.
Ten existing redelegation tests, workspace fast checks, ON consensus bridge build
with 12 jobs and all 15 bridge tests passed. Independent Sol medium review
accepted the corrected method documentation. Staged redelegation remains
unsupported; the next adapter must settle authenticated reads and excluded
success branches before wiring. The independent N3 escrow DryRunner gap is now
complete within its bounded synthetic scope: five actual dual-pin cases,
20 fresh Rust simulations across two physical reopens, all ten integration tests,
workspace fast and frozen independent Astra review passed without corrections.
The semantic owner remains fixed. The next ready N3 dependency is escrow gas
estimation through fresh historical sessions and the unchanged C++ search.
Root telemetry remained 23% after this review. Read a fresh value on restart.

The user reports that exact producer facts might not be recoverable. Record
executable identity, runtime overrides and capture command as unknown unless
evidence is found; do not invent them or repeat the request. The likely release
commit and checked-in reference configuration remain available for bounded
checks, but do not prove the producer setup. Qualified real-window acceptance
remains limited by those unknowns and completeness; other ready N2/N3
work can continue. Broad replay/differential/fault gates still need the authority
required by repository policy. Nothing has been pushed.
