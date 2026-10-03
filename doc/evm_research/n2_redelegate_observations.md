# Redelegation reference observations

This report records reference preparation and later bounded N2 steps.
The initial observation slice did not admit staged redelegation. The authenticated
partial adapter below now admits decoded calls within its stated success scope.
The [exporter](../../experiments/evm_feasibility/native_redelegate_observation_reference.go)
executes actual Go `StateTransition` transactions from both pinned archives.
The [harness](../../experiments/evm_feasibility/native_redelegate_observation_reference.py)
records 12 cases and 13 attempts with exact input, nonce, gas, errors, output,
logs, separate ordered storage-read and irreversible-write vectors, and final
source/destination stake and total principal. No supplied data is used.

## Profile and observation boundary

Synthetic period 1 activates Magnolia, Ficus and Cornus at zero; the redelegation
fix is zero. Aspen part two and Cacti remain inactive. Two genesis validators,
`31` and `32`, each have 1,000 principal from caller `d1`, zero rewards and
commission 100. Minimum deposit is 100; the destination-cap case uses maximum
stake 1,500. All other cases use 1,000,000. The genesis balances and supply come
from the reused custody profile with explicit two-validator adjustments.

Read observations are completed synchronous `EVMStateStorage.GetAccountStorage`
requests. They include address, key, copied value and whether the callback ran.
They are not all native API lookups: Go caches native rows in the same block.
Write observations use the existing synchronous copy-only observer before
`Account.SetStateRawIrreversibly`. Both observers exist only in disposable Git
archives. Target source hashes and observer hashes are guarded and recorded in
the fixture manifest. Original upstream files are unchanged. Separate vectors
do not establish read/write interleaving.

Each pin also executes a control with both observer forwarding paths absent.
The full exported execution fields, logs and final stakes match the observed
run after only the two observation vectors are removed. This checks observer
noninterference for this corpus, not arbitrary future execution.

## Results and implementation constraints

| Case | Result | Storage reads | Irreversible writes |
| --- | --- | ---: | ---: |
| First partial 300 | success; source 700, destination 1,300 | 14 | 12 |
| Repeat partial 300 in same block | success; source 400, destination 1,600 | 0 | 10 |
| Destination cap plus insufficient source | destination-cap error first | 4 | 0 |
| Missing source validator | validator error | 1 | 0 |
| Missing destination validator | validator error | 3 | 0 |
| Missing source delegation | delegation error | 5 | 0 |
| Insufficient source or remainder below minimum | insufficient delegation | 5 | 0 |
| Same validator, insufficient native gas, nonpayable | early error | 0 | 0 |
| Zero before Aspen part two | success; principal unchanged | 14 | 12 |
| Full source | success; source 0, destination 2,000 | 16 | 25 |

The successful partial attempts use 101,912 transaction gas and an 80,000 native
action quote. Total delegated principal remains 2,000 in every case. Normal
failures have no native writes or logs. Zero and full-source success are oracle
boundary facts; they are outside the proposed first staged adapter scope.

The first five successful read requests authenticate source validator, source
rewards, destination validator, destination rewards and source delegation, in
that order. Destination-cap validation occurs before the fifth read. Subsequent
reads include current reward nodes, shared old head/cursor nodes, destination
delegation and validator/delegation membership positions. Source writes complete
before destination writes. The first call writes each old shared head/cursor
key twice: count one, then deletion. The repeated call writes current nodes
twice: count one, then count three. Preserve these exact bytes and repetitions;
do not replace them with final-state-only comparison or deduplicated writes.

Independent Astra contract review requires normal preflight error precedence
before successful-transition scope rejection. Zero reward pools alone do not
prove zero payout: authenticate cursor/head/current reward indices and counts.
Use the existing business kernel, one raw trace, separate source/destination
node traces, and no call-level terminal global writes. A narrow account-port
conversion of the two existing kernel parameters is the next implementation
dependency. The complete staged adapter must still settle its authenticated-read
contract explicitly; it cannot claim full Go API or cache parity from these
storage vectors alone.

## Validation and limits

Both pins reproduce byte-identical observations. Both uninstrumented controls
match execution fields. The first harness assertion incorrectly required reads
on the cached second success; the actual output exposed that error and the
corrected harness now checks first/repeated counts explicitly. Complete first,
corrected, control and reproduction logs are retained under
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`.

Workspace fast checks passed. Frozen independent Astra medium review accepted
all six source/fixture/report hashes without findings. Requested and confirmed
routes were Sol medium implementation and Astra medium review; Luna's prior
bounded map identified this dependency. There were no routing failures.
No Rust or C++ implementation changed in this reference slice. Reward-bearing,
new-destination, raw-reader failure/corruption, post-Aspen-zero rejection,
historical same-validator correction, API simulation and rollback coverage
remain future gates. N1–N6 and Milestone 10 remain open.

## Account-port dependency

A subsequent narrow refactor changes only the `accounts` parameter of
`apply_dpos_redelegate` and `apply_dpos_redelegate_destination` to the existing
`DposAccountPort`, with `?Sized` support at the boundary. Both methods already
use the generic reward/removal helpers. The existing `HashMap` implementation
continues to serve FinalChain calls. No business decision, ABI, serializer,
state ownership or session admission changes. Method documentation states the
port's role and the caller's responsibility to discard scratch state on hard
errors. This is a prerequisite, not staged redelegation acceptance.

The ten existing redelegation codec/kernel/finalization/correction tests passed
after the refactor. Workspace fast, ON consensus bridge build with 12 jobs and
all 15 bridge tests passed. Independent Sol medium review accepted the corrected
freeze. Review corrected two documentation claims: future staged integration
and the historical same-validator exception to principal preservation. Runtime
code was unchanged by those corrections. These tests exercise
the existing map path; they do not prove staged account effects for redelegation.

## Kernel and serializer composition prerequisite

A separate test-only slice now compares the existing kernel and two existing
serializers with both actual partial calls. It constructs the two-validator
zero-reward semantic profile, starts a private period-one session, and invokes
the existing kernel through `StagedDposAccountPort`. Its account reader panics
on any access; both calls produce no ordinary account effects. This directly
exercises the new port capability without changing session admission.

One raw trace serializes the entire source transition before the destination.
Every address/key/value operation matches Go, including all 12 first-call and
10 repeated-call writes. Each operation's expected classified bytes must match
the evolving raw overlay before application. Native output and exact event logs
also match. Principal, aggregate votes and both membership orders remain
unchanged; final source/destination stakes match Go. Dropping the private session
leaves the committed genesis principal unchanged.

The [test](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_reference_tests.rs)
seeds raw observations only from the first successful Go call. Absence authority
is limited to this synthetic fixture, and deletion remains a present-empty
scratch value to retain repeated-operation expectations. It does not establish
physical tombstone behavior, account funding, Go cache/read parity, normal
preflight errors, scope guards or reader corruption handling. Staged redelegation
remains unsupported. Runtime methods and serializers are unchanged.

The new test passed on its first run. All 11 targeted redelegation tests, reused
dual-pin observation reproduction and controls, and workspace fast checks passed.
Independent Astra medium review accepted all three frozen source/report hashes
subject to the fast gate; that gate completed successfully. Requested and
confirmed routes were Sol medium implementation and Astra medium review, with
no routing failures or corrections. Logs use the `redelegate-composition-` prefix
under the existing persistent artifact directory.

## Normal kernel preflight prerequisite

A further test-only step compares seven actual-Go normal failures through the
same account port: destination cap before insufficient source, missing source
validator, missing destination validator, missing source delegation, insufficient
source, remainder below minimum and same validator after the fix. It compares
status, exact legacy error text, output and empty logs/account effects. The
working semantic clone and saved committed genesis snapshot remain unchanged.
The shared synthetic chain constructor preserves the earlier successful test's
profile and assertions, with maximum stake changed only for the cap case.

This does not exercise the session's raw-read prefixes or scope guards. The
kernel reads semantic state; no native request was admitted by this test.
All 12 targeted redelegation tests and workspace fast checks passed. Independent
Sol medium review accepted both frozen source/report hashes subject to the fast
gate, which completed successfully. Requested and confirmed routes were Sol
medium implementation and Sol medium review. No corrections or routing failures
occurred. Logs use the `redelegate-preflight-` prefix.

## Settled bounded adapter contract

Independent Astra medium contract review accepts fresh authentication on every
invocation for consistent, complete snapshots. It does not reproduce Go's block
cache or claim read-count parity for repeated calls. This decision is a contract
for the next implementation, not completed session behavior.

Use one invocation-local raw trace for preflight, node/membership checks and
serialization, so later accesses retain the same observed bytes. Preserve cold
normal-failure prefixes of zero, one, three, four or five reads as listed above;
authenticate absence as well as present rows. Integrity mismatch or reader error
aborts without effects or state advancement. The pure semantic preflight result
must not escape before its required authentication. Return authenticated normal
failures before rejecting unsupported successful branches.

Success requires Magnolia/Ficus, strictly post-fix distinct validators, positive
partial source principal, existing positive destination delegation and retained
positive validator stakes. Authenticate both head/cursor/current reward nodes,
including expected current-node absence; require zero pools and relevant indices,
agreement with semantic cursor fields and counts sufficient for exact decrements.
Authenticate validator and delegation membership positions against complete
semantic ordering. Run the existing kernel on a clone with no account access or
effects, serialize source then destination, and advance only after all checks
pass. Unsupported success branches remain explicit scope errors.

Exit tests must include normal failure read prefixes, repeated authentication,
bad node/membership rows, reader failure and unsupported successes, all with no
partial effects or semantic advancement. The existing write-composition tests
remain applicable. Malformed ABI and historical same-validator behavior are
outside the adapter contract. No higher-reasoning escalation was needed; the
requested and confirmed contract route was Astra medium, at 22% allowance.

## Authenticated staged partial adapter — 2026-10-03

The adapter reuses the settled contract above and the existing kernel, account
port and source/destination serializers. One fresh invocation-local trace binds
all preflight, reward-node, membership and serialization reads. Semantic normal
failures return only after their authenticated cold prefix. Success requires
active Magnolia/Ficus, strictly post-fix distinct validators, positive partial
principal, an existing positive destination delegation, positive retained stakes,
complete semantic history/order and zero relevant reward pools/indices. An empty
account context prevents account access. Source serialization finishes before
destination serialization, and semantic state advances only on full success.

Six new staged tests exercise pending and disposable finalized historical
sessions, both actual successful calls, all seven normal preflight failures,
every authenticated cold-row corruption and each reader-failure position,
expected absence, all warm-row corruptions, excluded successful zero/full/new
destination/reward/history branches, and inactive/pre-fix profiles. Every hard
failure returns no outcome/effects, poisons the session and retains semantic
state/sequence. Normal failures retain the session and return empty effects.
The actual 12/10 ordered writes, output and logs match the unchanged Go corpus;
Rust authentication uses 14/12 distinct reads, without claiming Go warm-cache
parity. Historical test state is explicitly synthetic and inserted through the
existing snapshot owner; this does not establish actual DryRunner/API parity.

First-run commands, full output and exit codes are under
`/home/fry/artifacts/evm-redelegate-2026-10-03/`, with the `adapter-` prefix.
The first compile exposed local count/index type errors; the first test compile
exposed a stored-token constructor error. A new pre-fix scope test initially
entered the earlier nested-call admission failure; its direct depth-zero input
now isolates the adapter scope. Existing tests and supplied fixtures are unchanged.
All 18 targeted redelegation tests, affected-package check/clippy, actual dual-pin
Go reproduction and ON consensus bridge build with 12 jobs plus all 15 bridge
tests pass. The full workspace fast gate passes with `RUST_TEST_THREADS=1`.
Its first parallel run had a temporary RocksDB lock collision in an existing
rewards fixture (1,465 consensus tests passed); no test or source was changed
for the serial retry. Independent Astra medium review found no blocking issue;
final acceptance is recorded in `adapter-review.md` under the artifact directory.

ABI error precedence, actual frame rollback/value/gas composition and historical
DryRunner/estimation/trace composition remain next dependencies. No upstream C++,
storage module, production route, fallback or supplied data changed. N1–N6 and
Milestone 10 remain open.
