# N4 storage checkpoint — October 11

Branch `feat/rust/evm-state-db`. Accepted implementation HEAD
`8a4a613dbbc4c82dcf5d608efb6f65d9d288f048`. Local commits only; no push.
Resume base: implementation `47032231369aca78904c4a367830973d9e18e935`,
checkpoint `8ceb7742aafa42b8e01c0ab7a196609db62e2ea4`.
Artifacts: `/home/fry/artifacts/n4-binding-resume-20261011` (R below).
Earlier evidence remains under
`/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011`.

## Accepted work

| Commit | Result | Independent evidence in R |
| --- | --- | --- |
| `7b512956ceb1ce10819e73bc29577ca13e155c10` | Persistent bounded RocksDB binding; production storage capture/encoding/replay; fixed qualifier | binding-final-review.md; live-wiring-final-review.md; live-first-run-final-review.md |
| `8a4a613dbbc4c82dcf5d608efb6f65d9d288f048` | Charge all cross-proof comparisons while avoiding repeated prior-header/frame decoding; unchanged limits and raw bytes | frame-cost-implementation-review.md; frame-cost-live-final-review.md; frame-cost-commit-final-review.md |

Earlier accepted dependencies remain: delayed headers `1fd035545`, quota reset
checks `edcb0ed5c`, Aspen counters `b158dc4c6`, synthetic witness `be62cc368`,
and borrowed views `470322313`. Their original contracts, reviews and freezes
remain authoritative for their separate scopes.

The binding retains the existing DB owner, CF cleanup and borrowed pin/cursor
lifetimes. Its bounded read-only route does not create directories or discover
CFs. Source-owned descriptors preserve comparators. Open, pinned-read and cursor
errors use bounded conversion; ordinary production error behavior is preserved.
No Cargo registry edit, separate raw-C DB owner or C++ production route was added.

One non-cloneable storage budget covers open metadata, pair views, both
constructors, authentication, all payloads and encoding. Charges precede
application allocations, copies, traversal and DB operations. Pin/cursor lengths
are checked before copying. Complete root/account/path authentication and strict
record order, conflict and consumption checks remain. Physical selection is
observed provenance; it does not prove history completeness. Engine memory,
allocator overhead, OS I/O and RSS are outside the application bound.

## Live result and stops

The first authorized capture stopped at FrameInspections300000/required300001,
payload index83, jail D25706929. No bundle or accepted prefix was published.
No missing/unavailable/retention error was observed; the remaining12 tuples were
unobserved. The comparison correction received separate contract/source/gate
review and a new single-use command authority. No ceiling or manifest was raised.

The second capture completed both constructors:51 registrations,50 distinct
identities, then the fixed96 payloads for D25706894..25706941. It used3108
records,6897 operations and116613/300000 frame inspections. The captured RCB1
bundle is1026265 bytes, SHA256
`a0a36875f90defe9dad0d7450416c75f57c06e9eee16cc53f27c666f95fcc761`.
Three known directory metadata identities matched before and after capture.
They are provenance checks, not database-byte hashes or immutability proofs.

Independent offline replay exited0. All96 proofs and canonical native
preconditions passed; all48 totals are534879. Replay used122829/300000 frame
inspections. Total absence never becomes zero; jail accepts authenticated
nonmembership or the canonical empty list only. Final output acceptance is
recorded in `frame-cost-offline-final-review.md`.

Exact commands and single-use stops are in `frame-cost-live-contract.md` and
`frame-cost-live-command-final-review.md`. Inputs remain the original pinned
interval, parent and delayed-header artifacts; the accepted codecs and
independently authenticated roots are unchanged. There were no inventory sweeps,
expanded requests, repeated unavailable tuples or post-failure retries.

## Validation and evidence

Binding:8 focused tests and3 compile-fail lifetime/ownership tests passed.
Final storage:29 focused tests,159 package tests,1 doctest and1 pinned Go
parity test passed;1 existing live test remains ignored. Qualifier:4 focused
and70 library tests passed. Format, affected package check and production
Clippy with warnings denied passed. Wider test lint retains15 existing findings.
`RUSTAXA_ENABLE=ON`; bridge built with12 jobs and all4 tests passed. Fast gate:
2118 passed,0 failed,47 summaries/42 nonzero (`frame-cost-fast.json/log`).
The earlier capability fast gate had2113 passes. Actual configured source commit
hooks passed; the cost hook took229.26s. Full fast logs are retained separately
from the condensed RTK commit output. Documentation uses a separate hook commit.

All first failures remain. Corrections include typed point errors, metadata key1,
sealed owner cleanup, two new-test lint findings, two copied vendor documents'
whitespace, replay source profiling, and the preflight gate/report filename
collision. The original collision opened no DB; corrected exclusive recording
and distinct filenames were reviewed. Historical freezes are not rewritten.

Source/input/executable/gate freezes: `first-source-commit-freeze.json`,
`frame-cost-validation-freeze.json`297 pins, `frame-cost-preprobe-freeze.json`301
pins, `frame-cost-qualifier-freeze.json`, and `frame-cost-capture-output-freeze.json`.
The original and v2 executables have separate immutable artifact paths.
Final replay/output pins and review are in `frame-cost-offline-output-freeze.json`.
The complete closeout freeze is `storage-final-closeout-freeze.json`.

## Quota, routing and pending state

Original baseline90%, floor80%; last valid observation83% (7 points consumed),
including account-wide side use. Fresh resumed telemetry was unresolved; no
current numeric usage or billing claim is made. User instruction
“continue and ignore budget for now” suspended the quota stop. The original
quota-budget.json was not edited or rebased; SHA256 remains
`bd5664c3f082e5b961e5fb67deffde261e0bf72ea60112233af44c007fe7695a`.
The original telemetry stop and routing records remain intact.

Luna medium started first and supplied bounded audits; Sol6.1 medium implemented
the binding/live/qualifier and comparison work. Astra medium supplied independent
contract/source/final reviews and replay. Requested routing and actual startup
results are in `routing.json` and `frame-cost-routing-closeout.md`; child runtime
identities remain unverified. No model failure or billing inference is asserted.
Luna's initial Cargo-root map error and its correction are both retained.

No owned source change remains pending. The8 unrelated files retain their
original hashes in `closeout-pending-paths.json`, confirmed by Luna. Preserve
modified workflow, N6 queue/checkpoint and active prompt; untracked next-ten-point
plan and3 token audits; `.agents/` as an uninspected directory. Do not stage them.

## Blockers and next action

Storage binding, capture wiring and native data-retention dependencies for this
fixed diagnostic family are closed. No missing tuple or retention blocker was
observed in the accepted capture/replay. No further historical probe is authorized
by these consumed single-use contracts.

Next executable action: verify the complete frozen evidence without a DB open:

```sh
rtk proxy python3 -E /home/fry/artifacts/n4-binding-resume-20261011/verify-final-closeout.py
```

Then use the qualified
48 totals plus48 jail preconditions as inputs to a separately complete N4
authority contract. Producer/configuration, voter facts/weights, lambda, retained
cache equality and coherent historical owner remain outside this storage slice.
Full reward-cache construction, writable owner creation,19-transfer execution
and production adoption still require their separate complete authority.
Keep H25706949/P25706948/session E25706944/certificate D25706942 distinct.
