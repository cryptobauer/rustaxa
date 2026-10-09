# Restart checkpoint: bounded Sol medium run

Branch `feat/rust/evm-state-db`; lead session
`01a11b9c-47e5-7f92-a0f8-c4b9d7725b34`, confirmed `gpt-6.1-sol` medium.
Initial accepted HEAD was `5ff1f0fc5056656d78e5ea6d4a26333d84e6230c`.
The original pending selector-only claim failure closed first as `e940e9419`.
Nothing was pushed. Milestone 10, N4–N6 and production cutover remain open.

Run directory:
`/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008`.
Read `current-checkpoint.md`, `current-pending.json`, `accepted-commits.json` and
the selected closeout there. Final checkpoint, closeout and quota files, when
present, supersede this interim record. Do not resume a closed run or its agents.

The quota baseline remains 77% at 2026-10-08T13:03:36.441Z, floor 67%, reset
1791948695. The latest interim observation was 69% at
2026-10-09T16:08:44.262Z: 8 points consumed, 2 left. Refresh the same saved
`quota-budget.json`; never initialize or rebase it on resume. At the floor,
start no new work, gates or delegation. Finish only in-flight atomic work and
minimal closeout. Account-wide use counts. Old run 4 remains closed at 79%.

## Accepted results and current derivative

`accepted-commits.json` lists all accepted local commits. Through `b8a14fa44`,
there are 21: pending failure closure; commission, delegator and mixed claim
sequences; positive, guard and repeated frames; finite pending claim-all;
checkpoint repair; claim-all trace; literal H1 claim-all; and the funded H1
estimation oracle. Each accepted unit has a source/evidence freeze and
independent Sol medium acceptance. Original routing and failures remain recorded.

The pending claim-all runtime at `546ac3eeb` has 168 public ports and all 11
required gates. The trace oracle/runtime derivative is accepted as
`148fcb325`/`9755e29cb`, with 336 ports. Its first review found incomplete owner
row preservation; the correction compares every owner column and row after
each attempt and reopen. Both review reports and comparator correction remain.

The literal H1 oracle at `fc5de00da` and runtime at `23e859793` close only the
finite HistoricalSimulation period 1 class alongside PendingPeriod 2.
Count-only preparation, ordered item traversal and private scalar staging stay
intact. Actual Go measures native gas 90000, transaction gas 111272, 12 ordered
raw writes, 2 balance changes and one positive 396 reward log. H1 public tests
cover 176 ports, faults and immediate retries. All 12 required runtime gates
passed: focused tests, consensus 1526, public EVM 65, check/clippy, ON bridge
build with 12 jobs and all 15 tests, serial fast and whitespace. No underfunded
or broader claim-all behavior was admitted.

The funded H1 estimation oracle at `b8a14fa44` measures caps
200000/111272/111273, estimates 116817/111272/111273 and 5/1/1 probes.
Actual dual-pin runs, repeats, controls, full P1, copied post-state and unchanged
C++ transcript reproduction passed all five preparation checks and frozen review.

The next derivative appends a public estimation test without changing runtime
or the existing 176-port H1 test. It checks 572 estimate attempts, 1788 callbacks
and 1774 allocated ports over fresh/reopened owners. Each callback error stops
the estimator with the exact typed error, no estimate and no completed failed
native outcome; a clean whole estimate immediately follows. Complete reader
and owner rows and the original wide nonce remain unchanged after each callback.
`claim-all-h1-estimate-public-closeout.json`, when accepted, owns final checks,
review and commit status; otherwise use `current-pending.json` for remaining work.

## Authority and next action

Canonical reward P1 has 92 physical rows and root
`0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b`.
Historical simulation uses literal period 1; pending execution/trace uses
period 2 over reader 1. Supplied reader data is never mutated. Synthetic
composition does not establish Rust/Go root parity or production authority.

Luna medium was confirmed first at startup and on the October 9 resume.
Sol medium implements serial work and reviews frozen settled derivatives.
Astra medium handled named traversal, H1 and underfunded source questions.
Worker runtime metadata, when unavailable, is not inferred from assignments.
No per-model quota or billing inference is made.

After funded estimation closes, a new profile requires a fresh reviewer context
and available capacity. Underfunded claim-all must first measure the boundary
probes in `claim-all-underfunded-contract.md`; removing the preparation gas
limit alone is unsafe. H1 claim-all trace also needs its own source contract
and actual dual-pin measurement. Check capacity and quota before starting.

General histories, qualified producer inputs, real-window/root parity, recovery,
N4–N6 and cutover stay open. No upstream C++, fallback, protocol change, supplied
seed mutation, publication or broad/differential gate is authorized. Unrelated
prompt, skill and audit edits remain excluded.
