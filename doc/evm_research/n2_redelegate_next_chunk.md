# Next chunk: staged redelegation and API composition

Completed for its bounded partial profile in the October 3 run. The active next
work is [zero-amount historical API](n3_zero_redelegate_next_round.md). The contract
below remains historical evidence, not a request to repeat the adapter.

Preparation baseline: `327d15fa1` on `feat/rust/evm-state-db`.
This advances N2/N3 within [Milestone 10](10_existing_network_milestone.md).
The [observations and settled adapter contract](n2_redelegate_observations.md#settled-bounded-adapter-contract)
are authoritative. Previous kernel/serializer and normal-failure tests are
accepted prerequisites, not a completed adapter. Do not repeat their discovery.

The prepared three-slice chunk is complete and independently accepted in the
October3 run. See the [compact checkpoint](n6_restart_checkpoint.md) and
[scorecard](../codex_slice_scorecard.md) for accepted extensions and the next
ready bounded gap. The ordered contract below is preserved as its baseline;
do not repeat its completed discovery or implementation.

## Ordered slices

1. **Staged adapter.** Own `rust/crates/rustaxa-consensus/src/final_chain/native_session.rs`
   admission, `native_session/custody.rs` dispatch and a narrow redelegation
   child module with focused tests. Reuse `DposAccountPort`, existing semantic
   kernel, authentication helpers and source/destination serializers.
2. **ABI and frames.** After adapter acceptance, map and name the narrow Rust
   dispatch/frame test owners. Add selector/admission and actual frame
   composition, including gas funding, value ownership, error precedence and
   rollback. Pin behavior from actual Go. Do not import metadata gas assumptions.
3. **API composition.** After frame acceptance, add real disposable historical
   simulation, then estimation and supported direct structured traces in
   separate accepted slices. Reuse existing API owners and algorithms. Separate
   physical reader reopen from semantic-owner restart. Any new uncertain trace,
   gas or historical behavior needs bounded contract review before implementation.

Continue later ready queue work only with fresh allowance and required authority.
There is no obligation to finish this entire chunk after reaching the reserve.

## Adapter invariants

Authenticate every invocation against a consistent complete snapshot using one
invocation-local raw trace. Do not claim Go block-cache or repeated read-count
parity. Preserve normal cold failure prefixes of 0/1/3/4/5 reads; authenticate
absence as well as presence before returning a semantic failure. Return those
authenticated normal failures before rejecting unsupported successful branches.

Allow success only for active Magnolia/Ficus, strictly post-fix distinct
validators, positive partial source principal, an existing positive destination
delegation and retained positive validator stakes. Authenticate both head,
cursor and current reward nodes, including expected current-node absence.
Require zero pools/relevant indices, agreement with semantic cursors, counts
sufficient for exact decrements and both membership positions against complete
semantic ordering. Execute the kernel on a clone with no account access/effects.
Serialize source before destination. Advance semantic state only on full success.
Reader/integrity errors abort; excluded success paths return explicit scope errors.
Malformed ABI and historical same-validator behavior are outside this adapter
contract; later ABI work must state its own exact scope before implementation.

## Evidence and acceptance

Use the actual pinned Go inputs/outputs under
`experiments/evm_feasibility/fixtures/native_redelegate_observation/` and the
existing reproduction tooling linked by the observations report. Retain source,
configuration and fixture identities. Do not replace actual outputs with
implementation-derived expectations. Repeated Rust authentication has its own
assertions; it is not an assertion that Go repeats cold reads.

Adapter tests cover pending and historical sessions, repeated authentication,
normal failure prefixes, invalid reward nodes/membership, reader failures and
excluded successful branches. Verify no partial effects or semantic advancement
on every failure. Preserve existing kernel/write parity tests.

Before each commit: affected Rust package checks/tests, applicable actual pinned
Go reproduction, focused ON consensus bridge build with `--parallel 12` and its
tests for runtime changes, and `make rewrite-validate-fast`. Storage changes also
require storage bridge checks. Freeze source/evidence, record full first-run
logs/exit codes, obtain independent review, correct findings and revalidate the
changed behavior. Tests alone do not close a production replacement boundary.

The lead owns shared files, validation and commits. Workers are not alone and
must preserve others' edits. Sol medium implements directly; Luna medium maps
bounded inputs; a fresh Astra medium reviewer checks adapter authentication and
rollback without importing previous families. Sol medium independently reviews
settled follow-ups. Escalate reasoning only for documented semantic uncertainty.
Record requested/actual routes and failures. No polling or duplicate implementer.

## Limits and handoff

No protocol change, fallback, production cutover, upstream C++ exception,
supplied-data mutation or broad replay/fault campaign is authorized here.
Missing producer facts remain unknown; do not repeat the request. N4 real-window
parity and N5/N6 acceptance remain open. Record accepted scope, commit, reference
pins, checks, review, correction batches and next dependency in the scorecard.
Keep the checkpoint short and link completed evidence.
