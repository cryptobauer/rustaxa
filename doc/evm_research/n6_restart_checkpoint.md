# Restart checkpoint: run4 active

Branch `feat/rust/evm-state-db`; run4 base28abfdec9f. No push.
Run4 follows next_executable_slice_prompt.md. Root is sole source, producer,
gate, database and commit writer. N1–N6/Milestone10 and production cutover remain open.

Run directory `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007`.
Root session01a10f48-83ee-7a32-8bff-722d10c36359, actualgpt-6.1-sol medium.
Saved quota-budget.json baseline89% at2026-10-07T22:22:57.349Z, stop79%,
reset1791948695. Reuse this exact budget without --init-budget. Latest reading
82% at2026-10-08T00:21:48.367Z:7points consumed,3remain. Refresh before new work.
At79% stop new work/delegation/large gates; finish atomic work and minimal closeout.
Completed budgets remain immutable: run1 66->56,run2 56->46,run3 99->89.

Accepted run4 local commits:

| Scope | Commit |
| --- | --- |
| Positive reward partial Go oracle | c652fc315 |
| Finite rewarded partial runtime P2 | 4c82359 |
| Rewarded partial Go frame oracle | fff3c4e |
| Public finalized/reopened P1 -> P2 frames | e14922b |
| H1 period/nonce source correction and checkpoint | 554ff65 |
| Canonical reward-only same-H1 Go oracle | 23159409d |
| Actual H1 ordered raw writes | 092f04e2e |
| Finite historical H1 runtime/public simulation | 6fe7b2c39 |
| Same-H1 gas estimation parity | 914515c6a |
| Actual direct P2 Go trace oracle | b3a32cb77 |
| Public rewarded trace/352 disposable sessions | f57dc561a |
| Positive owner commission trace/24 disposable sessions | 1aff45096 |

Each accepted slice has required checks, explicit frozen paths, matching live
hashes and independent Sol medium ACCEPT. Exact commands, outputs, exits,
review, actual routing and closeout records remain in the run directory.
First failures/corrections are preserved. No tests were weakened.

Canonical reward P1 has92physical rows/root0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b,
C3000/nonce0,O1000/nonce0,native4800/nonce1; sourcepool792/commission8 from800mint.
Public FinalChain finalizes P1 then reopens. Concrete Go state and semantic
Rust owner are independent authorities; synthetic composition is not root parity.
Historical simulation uses explicit constructor origin and literal period1;
P2 production guard stays finite. Partial300 pays396 with12raw/2accounts/2logs,
gas101912. H1 estimate uses actual17probes/unchangedC++search104977/min101912/
below101911 outofgas. Trace uses reader1/execution2 and pending session,
empty prefix/original wide nonce. Actual Go JSON excludes native effect vectors;
separate same-call copy-only witnesses are pinned with controls.

Retain accepted6fe production consensus1523 and ON12-job/all15bridge evidence
for test-only derivatives. Required derivative EVM/check/Clippy/serialfast/white
checks run on frozen bytes. No blanket production-gate rerun is claimed.

Luna medium maps, fresh Astra medium auth/H1/trace contracts and profile-specific
Sol medium reviews are preserved with actual sessions and corrections. No billing
or per-model quota inference. General reward/history shapes, qualified producer
inputs, real-window/root parity, recovery, integratedN4–N6 and cutover remain open.
No supplied-data/upstreamC++/storageowner exception, protocol/fallback,
production route, broad/differential gate or push is authorized.

Next active bounded profile: actual positive delegator claim trace measured (gas61464/payout396/6raw/1log); Rust derivative, required gates and fresh review are pending. Producer has complete canonicalP1 and controls; no producer seed is missing. Continue same saved run4 budget.
