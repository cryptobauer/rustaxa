# Restart checkpoint: fresh Sol medium run

Branch `feat/rust/evm-state-db`. Fresh lead session
`01a11b9c-47e5-7f92-a0f8-c4b9d7725b34`, confirmed `gpt-6.1-sol` medium.
Initial accepted HEAD `5ff1f0fc5056656d78e5ea6d4a26333d84e6230c`.
Root owns source, producers, gates, database and local commits. Nothing pushed.
Milestone10/N1-N6 and production cutover remain open.

Run directory:
`/home/fry/artifacts/evm-round-01a11b9c-47e5-7f92-a0f8-c4b9d7725b34-r1-20261008`.
Read `current-checkpoint.md`, `accepted-commits.json` and active slice closeout
there first. When `final-closeout.json` and `final-quota.json` exist, they are
final authority and supersede this interim record. Do not resume a closed run.

Fresh quota was recorded once: baseline77% at2026-10-08T13:03:36.441Z,
floor67%, reset1791948695. Reuse this run's `quota-budget.json` without init
or rebasing. Last reading72% at2026-10-08T15:57:07.226Z:5points used,5left.
Refresh before slices, delegation and large gates, after closeout and about
five minutes during long work. At67% stop new work/delegation/gates; finish
only in-flight atomic work and minimal closeout. Account-wide use counts.
Old run4 closed at79%; all old budgets remain immutable. Its final history is
in `/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007`.
Do not use the former repository run4 interim80% facts as startup authority.

Accepted local work in this fresh run:

| Scope | Commit |
| --- | --- |
| finite selector-only claim failure/public H1 parity | `e940e9419250` |
| dual-pin sequential commission oracle | `2f5101071490` |
| public retained commission sequence parity/fault retries | `36e54f036e12` |
| actual dualpin delegator sequence oracle | `651f6f388163` |
| public delegator sequence parity184sessions | `05d30b187aab` |
| actual dualpin mixed claim order oracle | `b8721e19c473` |
| public mixed claim parity160sessions | `c7d902f0401c` |
| actual dualpin positive claim frame oracle | `36f28f21b0e0` |
| public positive claim frames252ports | `f71c75a2b087` |
| actual dualpin claim frame guard oracle | `a769621c3951` |
| public read-free claim frame guards48ports | `35e92734393e` |
| actual dualpin repeated claim frame oracle | `bff5de35a463` |
| public repeated claim frames384ports | `f2c133031d5f` |
| positive claim-all oracle preparation | `3ded1a88a590` |
| finite positive claim-all runtime/public168ports | `546ac3eeb129` |

All accepted units have explicit source/evidence freezes and independent Sol
medium ACCEPT. Exact commands, full outputs, exit codes, routing, first failures
and corrections remain in the run directory. The pending selector-only claim
failure was closed first with fresh consensus1524/EVM/package check/Clippy,
RUSTAXA_ENABLE=ON bridge12jobs/all15, serial fast and whitespace checks.

Test-only sequence/frame derivatives retain those unchanged consensus/bridge
bytes with explicit authority and fresh focused/public EVM/check/Clippy/fast/
whitespace checks. Commission sequences cover72ports, delegator184, mixed160,
positive frames252, read-free frame guards48 and repeated frames384. The latest
accepted public suite before claim-all has62tests. Actual Go dual-pin outputs,
repeats, controls and raw-state snapshots own measurement; prior vectors are
not relabeled as a new measurement. All physical P1 and public Rust owner rows
remain unchanged after each disposable public attempt.

Canonical reward P1 has92physical rows and root
`0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b`.
Caller3000/nonce0, owner1000/nonce0, native4800/nonce1; first pools792/8 from800mint.
Historical simulation uses literal native period1 and explicit constructor origin.
Pending execution/trace uses period2 over reader1. Synthetic composition is not
Rust/Go root parity. Parent revert retains native raw writes while ordinary
balances/logs roll back. Low-gas/nonpayable guard witnesses are read-free.

Claim-all runtime/public parity is accepted at546ac3eeb129. Its separate
`claim-all-public-closeout.json` records8source/22evidence hashes and Sol medium
ACCEPT after11fresh gates: unit/public1each matched, consensus1525/EVMpublic63,
check/Clippy/ON12jobs/all15bridge/serialfast/white. Count-only preparation and
ordered item1/2 invocation use one private semantic/account/raw view. Actual Go
measures native90000, transaction111272, twelve raw writes, payout396 and one
event; second validator writes without payout/event. Public parity covers168
ports, every actual raw fault and immediate clean retries across both owners.

The source contract and corrective addendum remain linked in the closeout.
First compile failures, older-head flag confusion, absent genesis mirror
correction and mixed-port repeated-count integrity guard are retained. The
path is structural and limited to the measured non-owner, first-native, direct
pendingP2 profile. Owner, empty, batch, historical, nested/static/revert, repeats
and own later failure stay open. No pending implementation scope exists at this
checkpoint. Accepted HEAD is546ac3eeb129d067dec8bcc97d0e13a2e054ce05.

Luna medium startup was confirmed before reviewers. Fresh Sol medium reviewers
were confirmed at profile changes; same-profile threads were reused. Astra
medium handled the named unresolved claim-all traversal/failure contract.
Requested routing, actual runtime when available and failures are recorded;
no billing or per-model quota inference is made.

Next: a settled same-profile direct public API trace with empty/ordinary prefixes
may proceed from `claim-all-next-trace-map.md` and new actual dual-pin measurement. General histories, qualified producer inputs, real-window/root
parity, recovery, integrated N4-N6 and cutover stay open. No upstream C++, fallback,
protocol, supplied-data mutation, production route or broad/differential gate is
authorized. Unrelated prompt, skill and prior audit edits remain excluded.
