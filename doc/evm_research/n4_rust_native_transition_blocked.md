# Bounded Rust-native transition: blocked execution

## Outcome and state boundary

Base: `896ac888608c508de8fd321bed7702245721f371` on
`feat/rust/evm-state-db`. The supplied prompt, contract, pilot and checkpoint
changes were present before this slice. They are preserved in this closeout.
No database was opened. The ignored `data/` and `local/` paths were inspected
only through Git status. No worktree was created or pruned.

**Rust transition execution is blocked.** The lifecycle exists, but its owner
cannot be supplied under this slice's no-database authority. The implemented
slice adds rejection tests to existing native raw and account helpers. It adds
no runtime API or route. This report makes no Rust EndBlock execution, parity,
zero-effect, snapshot completeness, publication or milestone acceptance claim.

`FinalChainNativeSession` in
`rust/crates/rustaxa-consensus/src/final_chain/native_session.rs` requires a
nonoptional `&FinalChain`. `FinalChain` in `final_chain.rs` requires
`Arc<Storage>`, which owns a live RocksDB handle. The bound constructors check
the stored finalized head, require pending = parent + 1 and post-Cornus policy,
load the complete finalized parent `DposSnapshot`, advance its reward graph,
and select a delayed eligibility snapshot. Sparse reads cannot supply these
maps or their authority. A synthetic complete snapshot alone cannot construct
the required owner without storage.

The existing terminal method is
`native_session/rewards.rs::FinalChainNativeSession::finish_rewards`. It checks
request/period binding, stored head and reward-runtime generation, compares
committed parent slashing state, and unconditionally validates raw DPoS vote
and delegated-amount origins (logical keys 4 and 5). It then reconstructs
distributions, checks the planner's semantic result, serializes deferred DPoS
rows, and performs slashing cleanup. Success returns ordered ordinary and raw
mutations, complete unpublished semantic state, and an optional scheduler
successor. The session marks rewards finished only on success. None of these
lifecycle calls ran in this slice.

`rustaxa-evm/src/native.rs::invoke_native` adapts individual native calls and
journal effects. It does not supply EndBlock ownership. Reward scheduler
authority is separately issued by FinalChain for a live StateAPI epoch.
Projection, root preparation and publication belong to separate FinalChain
owners. They are not operations of the raw/account helpers tested here.

## Go comparison and missing evidence

The pinned reference is Go revision
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`. The checked-in
[cold report](n4_empty_native_go_cold.json) SHA-256 is
`80c3b64e6789ce502c1c24ea44cb781ced3acf46696984d31160d0869162f076`.
Its recorded lifecycle is `Init -> BeginBlock -> EndBlock -> Close`, at
H=25,706,949. The parent period/root are fixture labels, not Rust state:
25,706,948 /
`926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`.

The Go fixture supplies only an 85-byte DPoS account and the one-byte jailed
list `c0`. Its port is unauthenticated. It recorded two reads and zero backend
Put, Commit and observed TrieSink mutation attempts, with no transactions,
reward distributions, warm constructor, PrepareCommit or root calculation.
This evidence is reused, not rerun. Rust requires additional deferred-origin
reads and complete owner state; no executed comparison is possible here.
Producer, producer configuration and complete native reconstruction remain
unqualified. Physical unavailable history, authenticated nonmembership and
zero values remain separate classifications.

A later executable slice needs authorized isolated storage containing a
complete synthetic fixture, exact parent/eligibility state and required raw
rows, or a separately reviewed database-free owner boundary. Historical replay
also needs authenticated complete parent state and qualified producer policy.
The Rust origin-read difference needs an explicit common-branch comparison
scope. Do not remove owner checks or synthesize zero rows to fit the Go port.

## Implemented rejection boundary

Sol owns only the `#[cfg(test)]` sections of:

- `rust/crates/rustaxa-consensus/src/final_chain/native_session/raw.rs`
- `rust/crates/rustaxa-consensus/src/final_chain/native_session/account.rs`

Tests call the real existing helper methods with observed rejecting readers.
They check typed unavailable-history and invariant errors, exact attempted
reads, unchanged staged maps/effects, and repeated reads after failure. Raw
tests cover current reads, nonempty puts and empty-value deletion. Account
tests cover reads and zero/nonzero balance additions and subtractions. The
test identity is synthetic and does not authenticate the historical parent.
Rejection tests do not execute a whole transition or establish its effects.

Root calculation, publication, full lifecycle and terminal-state tests are
unexecuted: the helpers have no such capability, and the required owner cannot
be constructed within scope. No fake root/publication API was added. No C++,
storage implementation, bridge, production orchestration or existing test
behavior changed.

## Validation and frozen review

Source froze after the import correction and passing worker tests. The lead
then repeated the focused tests and checked the frozen source:

| Command (all invoked through `rtk`) | Result |
| --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all --check` | Passed |
| `cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-consensus --all-targets` | Passed with existing warnings outside the changed test code |
| `cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus bounded_transition --lib` | 4 passed, 0 failed, 1444 filtered out; no database tests selected |
| `make rewrite-storage-boundary-guard rewrite-bridge-inventory-guard` | Both guards and self-tests passed; inventory emitted a missing consensus/shims directory diagnostic |
| `git diff --check` | Passed |
| `sha256sum doc/evm_research/n4_empty_native_go_cold.json` | Matched the reference pin above |

Failed attempts: the first worker compile had three import-path errors; the
worker corrected them before freezing source. The lead's initial overlapping
strict clippy check (`--all-targets -- -D warnings`) caught one of those imports
and 21 existing lint errors outside the owned paths. The standard repository
clippy command passed after the imports were corrected. No lint suppression or
unrelated code fix was added. No correction was made after source freeze.

Persistent logs and telemetry samples are under
`/home/fry/artifacts/n4-transition-2026-10-01/`. Independent Astra medium final
review found no blocking findings and required no correction batch. It checked
the real helper calls, owner authority, validation logs, frozen Rust/report
hashes, and all six pinned Go source hashes. It ran no database or Go harness.
Only review/closeout metadata was added after the review. Frozen source SHA-256 values:
`raw.rs`: `e154cc2192dcbe732a25b6acc650970095bba5faea5335cc8ffad556553f30c9`;
`account.rs`: `d4b598c3c859a8cefcb9bc43d9fc554482f8e54238e39bd31026dc41d9f2db9e`.
Tier 1 is the applicable base for test-only helper changes. The full workspace
test command in `make rewrite-validate-fast` and `.githooks/pre-commit` opens
databases, so it is outside the explicit slice scope. Record that gap; do not
report full Tier 1 as passed. The checkout has no configured `core.hooksPath`
and no default `.git/hooks/pre-commit`; the commit does not require bypassing
an installed hook. No storage-module implementation or C++ route changed, so
storage bridge, FinalChain subsystem and production differential gates are
not required for these tests. Execution/parity gates remain open.

## Routing and token pilot

Luna started and acknowledged before Astra or Sol started. Runtime metadata
from local `turn_context` confirms:

| Task | Requested model / reasoning | Observed model / reasoning | Session ID | Result |
| --- | --- | --- | --- | --- |
| Mapping | `gpt-6-luna` / medium | same | `01a0f58a-d5ee-7761-8dcf-1cef9bc47100` | Owner and sparse-state map complete |
| Contract | `gpt-6-astra` / high | same | `01a0f58b-2482-7301-b08e-2d32e227e43d` | Blocked contract approved; high for uncertain lifecycle authority |
| Implementation | `gpt-6.1-sol` / medium | same | `01a0f58c-c797-74c1-9a5c-40e51557c257` | Four tests passed; import paths corrected before freeze |
| Independent final review | `gpt-6-astra` / medium | same | `01a0f590-e908-74b1-a926-1594fb095fe4` | No blocking findings; no correction batch |

Root session: `01a0f589-7bcf-7d70-8c7a-81252386af90`. Its observed runtime is
`gpt-6.1-sol` / medium, which differs from the historical Astra lead listed
in the prompt. This session did not change its model or claim an Astra root.
Astra retained contract authority; Sol performed implementation. This routing
discrepancy is explicit and remains a limit of the pilot. No model startup,
thread-capacity or usage-limit failure occurred.

Start allowance sample: 67% used / 33% remaining at 03:37 UTC on 2026-10-01.
Before implementation: 68% used / 32% remaining at 03:40 UTC. Samples come from
local `token_count.rate_limits.primary` with a 10,080-minute window. They are
time-specific observations, not billing claims. Source counters, validation
and closeout samples will be frozen with the final review package. Standard
credit estimate: `unknown`. Deduplicated phase totals: `unknown` pending the
external audit. Prior 110M batch totals are not a per-slice comparison.

After validation the root sample was 68% used / 32% remaining at 03:43:47 UTC.
The persistent `implementation.json` and `validation.json` preserve per-thread
cumulative input/output/total counters and first/last telemetry times. These
are source measurements, not a deduplicated audit. Start was 03:37:03 UTC;
elapsed time through validation was about seven minutes. Exact closeout wall
time is measured through the closeout sample below, before the local commit.

Closeout sample: 68% used / 32% remaining at 03:46:24 UTC; captured at
03:46:44 UTC. Elapsed wall time from session start to capture: 9 minutes
41 seconds. One pre-freeze import correction batch was required; no independent
review correction was required. The bounded rejection objective passed; actual
transition execution stayed blocked. The lead-model discrepancy and missing
deduplicated phase usage limit comparison with another pilot.

The following are per-thread cumulative **source counters** as captured in
`closeout.json`, not deduplicated external-audit totals or allowance usage.
Input includes cached input. The root counter excludes later commit/final
response activity; full-session and phase totals remain `unknown` pending audit.

| Thread | Input | Output | Total |
| --- | ---: | ---: | ---: |
| Root integration | 2,567,199 | 11,413 | 2,578,612 |
| Luna mapping | 615,905 | 3,201 | 619,106 |
| Astra contract | 657,617 | 2,896 | 660,513 |
| Sol implementation | 608,936 | 4,721 | 613,657 |
| Astra independent review | 369,859 | 1,625 | 371,484 |

External log audit: **pending**. Prepared read-only request:

```text
Audit session 01a0f589-7bcf-7d70-8c7a-81252386af90 and all descendant agents
using local Codex logs. Keep it read-only and do not resume the session.
Deduplicate response usage by (thread_id, response_id); count compaction once
and reconcile cumulative counters. Report by model, agent, phase, and activity.
Separate token counts, Standard credit estimates with dated rates, and allowance
percentages. Compare the pilot's elapsed time, review corrections, and accepted
scope; identify missing evidence.
```

M10, implementation plan 08, complete native/catalog qualification, slashing
reconstruction, producer policy, reward-inclusive real replay/root proof,
publication/recovery and non-genesis adoption remain open. The synthetic M09
acceptance is unchanged.
