# Agent handoffs and routing outcomes

Continuation baseline: `5a6d87a62`, branch `feat/rust/evm-state-db`.
These records distinguish requested/assigned models from observed execution.
They do not establish billing or quota consumption.

| Work / handoff | Requested model in this continuation | Agent ran? | Observed result / routing status |
| --- | --- | --- | --- |
| Capacity inventory | No model request; `agents.list_agents` | No new agent | Seven threads listed, including root and a completed thread. No close-thread tool is exposed. No free slot was established. |
| Reserved Luna helper | No spawn request sent; intended assignment `gpt-5.6-luna` | No | Existing thread-capacity blocker remains. No unchanged retry and no Sol/Astra helper substituted. |
| Existing execution/native/state workers | No new model request; prior assignment `gpt-5.6-sol` | No work in this continuation | Existing threads report model/account usage-limit errors. These are separate from the capacity blocker. No unchanged retry. |
| Independent closeout reviewer | No new model request; prior assignment `gpt-6-astra` | No work in this continuation | Existing reviewer thread reports a usage-limit error. Review of final Falcon/zero-yield/shared integration and new changes remains open. |
| RETURNDATACOPY integration | Existing lead session, Astra lead assignment; no spawn/model-routing request | Lead worked locally | Commit `f7e3fa49f`: implemented profile/driver correction and compared 69 full-frame programs against both immutable Go references. Result is an implementation slice pending independent review. |
| Persisted historical API validation | Existing lead session, Astra lead assignment; no spawn/model-routing request | Lead worked locally | Commit `00195b807`: added actual Go seed-row export and RocksDB materialization/reopen tests, including a distinct newer state, fresh gas probes and missing-dependency failures. No production code or bootstrap authority added by this test fixture. |

The last two rows are lead integration work. Luna's bounded helper work remains
unassigned because a slot could not be established. Completion or failure of an
existing thread is not treated as proof that its slot has been released.


Validation for these lead slices: all 23 native-driver tests and eight simulation
tests passed in `make rewrite-validate-fast`; affected-target strict clippy and
formatting/whitespace checks passed. All three relevant exporters reproduced
both pinned artifacts under Python `-O`. No expensive gate was requested or run.


## Resumed continuation at `b717bde0b`

The owner clarified that account usage remained available. A single retry on
existing related threads succeeded; earlier tool error strings are historical
routing observations, not verified account quota or billing facts.

| Handoff | Requested model | Agent ran? | Result / routing status |
| --- | --- | --- | --- |
| Independent integrated review | Existing assigned `gpt-6-astra`, no new model request | Yes; startup confirmed | Approved scoped zero-yield, RETURNDATACOPY and persisted API changes; found Falcon MaxInt allocation blocker. Resumed follow-up review; no routing failure. |
| Staged native queries and simulation session | Existing assigned `gpt-5.6-sol`, no new model request | Yes; startup confirmed | Refined current/staged versus frozen delayed-view contract; implementation underway. No routing failure. |
| Concrete checkpoint reader and authenticated inventory | Existing assigned `gpt-5.6-sol`, no new model request | Yes; startup confirmed | Refined explicit identity, bounded traversal and native-catalog completeness contract; implementation underway. No routing failure. |
| Reserved Luna bounded helper | Intended `gpt-5.6-luna`; no spawn request | No | Seven threads remain listed; no close-thread tool or free slot established. No substitute launched. |
| Falcon allocation correction | Existing lead session; no model request | Lead worked locally | Added actual dual-pin MaxInt panic witness and fallible padding allocation; 41-row primitive and 23 native-driver tests plus fast gate passed; independent reviewer approved. |

| Actual native DryRunner oracle | Existing assigned `gpt-5.6-sol`, no new model request | Yes; startup confirmed | Implementing actual Go staged-delegation/current-query/delayed-eligibility fixture. No routing failure observed. |
| Multi-validator rewards | Existing assigned `gpt-5.6-sol`, no new model request | Yes; startup confirmed | Resumed related rewards work; zero-yield patch already integrated as `6228744df`. Designing permutation and commutativity evidence. No routing failure observed. |
| Native simulation facade | Existing lead session; no model request | Lead worked locally | Added consuming factory composition and lifetime/sequence test. Independent reviewer approved ownership boundary; strict clippy and fast gate passed (24 native-driver/eight simulation tests). Actual native oracle remains separate. |

State reader handoff: worker commit `b73a25da8` integrated as `4ff0cf8af`.
Requested existing Sol worker ran successfully; Astra reviewer approved the
read-only API and historical error identity. Eight focused reader tests, the
fast gate and all four required Rust-enabled C++ storage bridge tests passed.
The authenticated inventory is the worker's next separate slice.


## Integrated resumed slices

| Slice | Requested/assigned model; ran? | Result and routing |
| --- | --- | --- |
| Historical native session/query | Existing Sol high; yes | `ed38b8d27` integrated as `46eaf2e9e`; 18 focused tests and independent source review passed. No routing failure. |
| Actual native DryRunner oracle | Existing Sol high; yes | `d8300c934`→`89d0d57c4`, typed-log follow-up `717b94ea9`→`4ccc0078e`; both pins reproduced. No routing failure. |
| Authenticated storage inventory | Existing Sol high; yes | `46c981560`→`1a3f2db23`, evidence `148de2bb2`→`3eca1c369`; independent review and bounded copied-head reproduction passed. No routing failure. |
| Multi-validator reward map | Existing Sol high; yes | `c90cd4064`→`c535f4327`; actual Go permutations, focused tests and independent source review passed. No routing failure. |
| Persisted native API integration | Existing Astra lead; lead worked | Exact six-case Go comparison, two concrete opens and missing-known-row negative test passed; independent reviewer approved. No new model request. |
| V1 custody pair follow-up | Existing Sol high; startup confirmed | Source mapping completed; native worker implementing existing-kernel port/serializer extension and execution worker producing actual Go oracle. No routing failure. |
| Jailed cleanup follow-up | Existing Sol high; startup confirmed | Go EndBlock ordering and existing Rust kernel contract settled; implementation underway. No routing failure. |
| Retained historical snapshot audit | Existing Sol high; existing thread resumed | Checking H-5..H availability separately from semantic reconstruction; no inference of pruning from lite-node provenance alone. |

No new helper slot was established, so Luna remains unlaunched; no Sol/Astra
helper replaced that assignment. These are execution/routing observations,
not evidence of billing or account quota consumption.

Integrated closeout for these chunks: fast gate, strict native-simulation clippy,
four native-simulation/support tests, four storage bridge tests and four focused
FinalChain/account-query/result bridge tests passed. The master CMake switch
remained `RUSTAXA_ENABLE:BOOL=ON`. Qualified-copy inventory reproduction passed
with the exact recorded digest; no original snapshot writes occurred.


## Active integration after owner status correction

The lead previously ended turns while describing the milestone as ongoing.
The owner correctly identified the lack of continued lead integration. The lead
resumed actual work from clean `38dac469b`; no background-progress claim is made
from the assignment alone.

- Existing Sol V1 owner `1731b73f8` integrated as `71f6d9ab2`; independent source
  review approved after preserving typed recipient account-read failures.
- Existing Sol actual-Go oracle `49aac0745` integrated as `a6d800912`; both pins
  reproduced under Python `-O`.
- Lead validation passed: 37 native-session tests, all five existing V1 lifecycle
  tests, `make rewrite-validate-fast`, Rust-enabled consensus bridge build with
  12 jobs and four focused FinalChain/account-query/result bridge tests.
- Related Sol threads resumed for direct V1 corpus assertions, bounded structured
  trace design and concrete-backed account working-set design. Tool inventory
  reports those threads running; implementation results remain pending.
- Jailed cleanup candidates `70b34743e` and `7434d39b7` remain unintegrated.
  Independent review found scheduler-lifetime and decreasing-Cacti-duration
  mismatches; the rewards owner is correcting the scope and witnesses.

No new agent was spawned; no Luna slot became available or substitute helper
was assigned. No model/account quota conclusion follows from this handoff.


## Checkpoint reader, custody corpus and trace integration

Requested models remain Astra lead/reviewer and Sol high implementation owners.
The existing threads ran successfully; no new spawn or routing failure occurred.
No Luna slot was established; no model substitution or billing inference follows.

- Sol rewards owner: approved scoped cleanup `16778db9d` integrated as
  `25c9e41e1`. Earlier rejected candidates remain excluded. Process-lifetime
  scheduler design is a separate active task.
- Astra lead: revert bytes/oracle `4c7c77983`, independently reviewed. Actual Go
  ABI decoder and ordinary DryRunner diagnostics pass targeted comparisons.
- Sol execution owner: collector `1bc14a4e0` and phase documentation `b476da104`
  integrated as `7a005eae9` / `d21d1769a`; three collector tests pass. Subsequent
  driver hooks remain under independent review and are not accepted by assignment.
- Sol native owner: direct V1 Go corpus tests `59db07f6a9` integrated as
  `ee9fc24f3`; both tests pass. Paired V1/V2 cancellation is active.
- Sol state owner: checkpoint native adapter `5a4d00619` integrated as
  `b73055338`; five adapter tests pass. Bounded native inverse coverage tooling
  is active, with no semantic completeness or adoption authority claimed.
- Astra lead: persisted Go-seed adapter coverage passes all five native simulation
  tests, including reopen, full-width values, unavailable raw history and exact
  unchanged database rows. This uses synthetic fixture provenance only.

Combined `make rewrite-validate-fast` passed, as did strict affected-test
clippy with `--no-deps`, a Rust-enabled `rust_consensus_tests` build with 12 jobs
and all four focused FinalChain/account-query/result bridge tests. Strict
dependency-wide clippy encountered existing consensus warnings; the normal
repository gate passed without weakening checks. Expensive broad replay and
production routing remain unauthorized.

## Block selection and opcode hook handoff

- Requested Sol high execution owner ran and produced reviewed `d2b0157df`,
  integrated as `46380b0a3`. Root's six focused driver tests and affected strict
  clippy passed. Related default structured serialization work resumed in the
  same thread; full trace API parity remains open.
- Requested Astra lead implemented pure block selection and an actual C++ method
  extraction oracle with 188 cases. Independent Astra source review approved;
  root corpus test, Python `-O` reproduction and strict affected clippy passed.
- Requested Sol high rewards owner and Astra reviewer agreed the scheduler must
  bind to actual process-local StateAPI epochs, source FinalChain instance and
  publication lifecycle. Coherent epoch/transport/session implementation now
  belongs to the existing rewards thread, including named rewrite-owned boundary
  files. No backend routing or durable encoding change is authorized.
- Sol state owner continues bounded inverse decoding; independent review found
  permissive RLP shape/length handling that must be corrected before approval.
  An authenticated live inventory alone cannot establish semantic completeness.

No new agent was spawned, no new routing failure occurred, and no Luna slot or
billing/quota usage is inferred from these assignments.


## Cancellation and structured trace integration

- Sol high native owner ran and produced `588a2175b3`, integrated as `cb8016217`.
  Independent review approved same-block zero-yield V1/V2 cancellation only.
  Root's 54 integrated native-session tests, dual-pin optimized reproduction,
  fast gate, Rust-enabled bridge build and all four focused bridge tests passed.
  Cross-period accrued-reward custody remains the same owner's next task.
- Sol high execution owner ran and produced `4a3dd1813`, integrated as
  `9edaa820f`. Three default serializer comparisons pass. The seven-scenario
  representation test consumes supplied Go facts; it is not additional execution
  evidence by itself. Independent review approved that stated scope.
- Astra lead added actual driver-to-JSON comparisons for four Go scenarios and
  an independently reviewed four-program raw refund oracle. Seven driver tests,
  three serializer tests, strict affected clippy and optimized dual-pin refund
  reproduction pass. Nested traces remain unsupported where facts are unproved.

No new thread, model substitution or routing failure occurred. Assignments and
successful runs do not establish billing or quota consumption.

## Explicit Luna startup after owner correction

The owner correctly noted that this stretch had not used Luna. The lead checked
capacity: seven threads remained (root, five running workers/reviewer, and the
completed `s4_oracle` thread). An explicit bounded read-only selector-map launch
requested `gpt-5.6-luna`, medium reasoning, with no inherited history.

Startup failed with the exact tool error `agent thread limit reached`.
The Luna agent did not run and produced no result or commit. Available tools
provide no thread-close or existing-agent model-change operation; deferred tool
metadata contained no such capability either. The failed launch was not retried,
and the bounded task was not reassigned to Sol/Astra. This is a thread-capacity
failure, not evidence of model/account quota exhaustion or billing consumption.
The reserved-slot rule was not effectively maintained in the existing team;
future team setup must allocate Luna before filling remaining thread capacity.


## Sequence semantics and seeded native coverage

- Astra lead corrected cumulative trace refunds using signed root deltas and
  retained journal base, preserving ordinary transaction settlement. Independent
  review approved the three actual Go sequence witnesses; ten driver tests and
  strict affected clippy pass. Empty-code tracing no longer fabricates STOP.
- Sol execution owner produced `6f5ab7500`, integrated as `193c628a2`, extracting
  persisted API helpers without changing the eight simulation checks. The same
  owner is implementing the one-journal structured runner. Review rejected a
  proposed per-transaction reset before it reached implementation.
- Sol native owner produced accrued-cancellation evidence `6091d6425f`, integrated
  as `a65a73758`. Both three-period Go evidence and scoped two-period Rust semantic
  composition are retained; actual scheduler publication/reopen remains open.
- Sol state owner produced inverse code/evidence `61fe324e3`/`f903ef7d8`, integrated
  as `1deb6f754`/`9a75b0c2a`. Root's six tests and qualified-copy report reproduction
  passed byte-for-byte. Seeded delegation extension `8a6907daf`/`7c65c4ae3` is
  integrated as `93e9b39a0`/`fcff94998`; independent review verified reported source
  hashes and scope. It explains 2,065 live rows, leaving 21,213 unexplained.
- Sol rewards owner produced epoch transport candidate `02cf73d84`, still held
  for independent review corrections: failed discard may reset Go state despite
  an error, and generic recovery paths must check the observed epoch. Scheduler
  runtime remains a separate unaccepted slice. No candidate was integrated early.

These existing assigned threads ran; the explicit Luna startup failure remains
recorded above and was not retried or substituted. No quota/billing inference.


## Owner-requested immediate restart checkpoint

All existing agents stopped new work and saved clean worktrees. The exact commits,
remote checkpoint branches, validation limits and restart procedure are in
[n6_restart_checkpoint.md](n6_restart_checkpoint.md). The feature branch remains
at validated implementation `19e9e7e59` plus documentation. Source-reviewed epoch
transport and its C++ test adaptation are saved separately as `db8ec5e4d` because
the fast gate found missing epoch fields in three EVM Rust fixture files. Its
37 domain and eight C++ tests passed; workspace acceptance remains pending.

Luna must start first in the fresh team. The failed old-session launch did not
run, and no helper result or billing inference is attributed to Luna.

## Luna-first continuation, 2026-09-14

The fresh session checked capacity with only the lead present, explicitly
requested `gpt-5.6-luna` at medium reasoning, and confirmed the helper running
before allocating other workers. The reused `luna_epoch_inventory` thread
completed read-only epoch-fixture, C++ validation, semantic-port registration and
bridge-validation maps. The helper's local runtime `turn_context` independently
records `model: gpt-5.6-luna` and `effort: medium`; this is stronger than merely
recording the requested assignment. It made no commits. Independent review corrected two
overbroad bridge-removal suggestions: the pre-zip result-count check must stay,
and committed-descriptor validation must remain in immediate commit
classification, not only eventual lifecycle acceptance.

- `epoch_fixtures`: requested `gpt-5.6-sol`, high; ran and completed the fixture
  lifecycle update. `trace_review`: requested `gpt-6-astra`, high; ran and approved
  after restoring exact missing-marker coverage and validating epochs before
  injected failure paths. Lead integration is `7b2c29ac3` after the five saved
  epoch commits. The same Sol thread subsequently took the bounded shared
  setCommission kernel slice in its isolated worktree.
- `trace_review` also independently approved saved TraceRunner `b844c1896`,
  integrated as `b936558ab`. Eight runner, ten driver and three serializer tests
  plus both actual-Go trace/refund exporters passed. Acceptance remains default
  structured traces only.
- `scheduler_review`: requested `gpt-6-astra`, high; ran, rejected the saved
  scheduler's incomplete startup/publication/recovery contract, and approved the
  lead's separate epoch bridge contraction after the missing CXX header include
  was corrected. `scheduler_completion`: requested `gpt-5.6-sol`, high; ran to
  address those scheduler findings in its isolated worktree. Saved scheduler WIP
  is not accepted by these epoch results.
- `claims_completion`: requested `gpt-5.6-sol`, high; ran and produced
  `03f3598d3` over saved `f413b296a`. Independent Astra review and both actual-Go
  pins approved the bounded unpublished semantic-session scope. Integrated
  commits are `d620a5711` and `12fa2f48f`; the only first-commit conflict retained
  both the claims and existing checkpoint module declarations. The isolated
  worktree had no configured hook path; the lead owns integrated fast validation.
- The reused `epoch_fixtures` Sol thread completed the setCommission draft as
  `dd64b1125`. Independent Sol review in `claims_completion` approved the full
  draft plus delta against both Go pins; neither author nor helper approved its
  own semantics. Integrated commits are `c3f53a0e2` and `02281c89d`. Lead checks
  passed 64 native-session tests, three commission rule-order tests and the
  six-case driver/journal corpus. The acceptance scope is documented in
  [the semantic-port evidence](n2_commission_semantic_port.md).

Epoch integration passed the full fast gate, 38 execution-domain tests, two
period-codec byte tests, persisted/mixed fixtures (3/5), and eight rebuilt C++
leaf/FinalChain tests with `RUSTAXA_ENABLE:BOOL=ON` and CMake `--parallel 12`.
Targeted C++ formatting passed. All touched C++ paths are main-only; no original
upstream C++ file was modified. The bridge shrank from 4,621 to 4,619 lines with
unchanged carrier/function/handle counts. No guard or test expectation was
weakened to accommodate epoch routing.

Every requested model above started and ran; no startup, capacity or routing
failure occurred in this fresh team. These observations do not establish billing
or quota consumption. Milestone 10 remains open.

## Scheduler integration and resumed continuation

- Reused `luna_epoch_inventory`, requested `gpt-5.6-luna` medium, ran and
  acknowledged resumption before other worker reuse. It mapped the bounded N4
  inverse diagnostic and correctly distinguished unresolved rows from external
  absence. No edits or database opens.
- Reused `scheduler_completion`, requested `gpt-5.6-sol` high, completed
  `e393f542b` over `621241a863e`; integrated as `51c408c20` over
  `f3f6bb7a8`. Reused `claims_completion`, requested Sol high, authored the
  joined actual-native/dual-Go scheduler and stale-publication regressions.
- Reused `scheduler_review`, requested `gpt-6-astra` high, approved completed
  worker source and integrated intent/descriptor boundaries. The combined gate
  subsequently caught duplicate epoch fields from automatic fixture merging;
  the lead removed only duplicates, and the reviewer approved the complete
  fixture comparison. Mixed/persisted tests passed 5/3.
- Reused `trace_review`, requested Astra high, approved the native system-fact
  identity/policy contraction and its two regression tests. The lead implemented
  it; the bridge budget ratcheted from 4,619 to 4,618.
- Reused `epoch_fixtures`, requested Sol high, began a bounded N4 undelegation
  inverse extension using already authenticated address seeds in an isolated
  worktree. Its earlier claim that the semantic-port normal commit ran the hook
  was corrected: hook execution was not verified there. Lead explicit gates are
  authoritative.

The integrated FinalChain Tier 2 gate passed with Rust enabled: workspace fast
checks including 1,441 consensus tests, C++ consensus 15, StateAPI 3, RPC 50,
and binary CLI smoke. Storage bridge 4/4 and targeted C++ formatting also passed.
No required target was skipped. No original upstream C++ changed.
All reused agents ran without routing/capacity failures; no billing inference.
The [scheduler evidence](n5_reward_scheduler_evidence.md) states the bounded
reference and recovery limits. Milestone 10 remains open.

### Bounded N4 undelegation reconstruction

Reused `epoch_fixtures` (requested `gpt-5.6-sol`, high) completed
`e3e586ed3`, integrated as `87479197e`. Reused `trace_review`
(requested `gpt-6-astra`, high) approved source, the exact bounded read-only
operation, and the resulting report. Both ran; no routing failure.

One 23.8-second run on the qualified copy, capped at 300 seconds, added 217
exact current-live rows using 1,017 derived reads over the same 290 seeds.
Coverage is now 2,282/23,278; 20,996 remain unexplained. The lead independently
ran the eight binary tests after integration; the reviewer reconstructed all
new preimages/values and checked artifact/source hashes without reopening the DB.
The original snapshot was untouched, and qualified-copy access was released.
See [the exact evidence and limits](n4_native_seeded_undelegations.md).

## Model refresh validation — 2026-09-30

The agent service initially listed only root; historical threads were not live.
Fresh `luna_setup_validation` requested `gpt-6-luna` medium and started first.
After acknowledgment, `sol_setup_validation` requested `gpt-6.1-sol` high and
`astra_setup_review` requested `gpt-6-astra` high. All three completed bounded
read-only setup tasks. Local runtime `turn_context` records confirm the exact
requested models and efforts; no routing failure occurred. They made no commits.
The lead owns the documentation update. No quota/billing inference is made.

The current lead remains `gpt-6-astra` medium. Project routing now explicitly
assigns high-risk contract decisions and review to Astra high, implementation
to Sol high, and bounded helper work to Luna. Old fixed specialist roles are not
used to select new models. Historical 5.6 assignments above remain unchanged.

The [setup validation](n6_model_setup_validation.md) records the new routing,
agent session IDs, independent review and the environment recovery prerequisite:
old `/tmp` worktrees/reports and both snapshot paths are absent. Surviving reward
branch metadata points only to the already integrated baseline, not a saved
extractor. No implementation or snapshot work resumed during this audit.

### Task-owner effort adjustment — 2026-09-30

After the routing smoke checks, the task owner requested medium as the default
for both GPT-6.1 Sol and GPT-6 Astra. Prospective assignments and restart
instructions now follow that policy, including routine independent review.
High remains available for ambiguous or high-risk semantics, authority changes,
difficult failures and substantial review rework; record the escalation reason.
Historical high-effort startup records above remain accurate. This documentation
change does not retune existing threads, claim medium/high benchmark equivalence,
or change reviewer independence, model ownership or validation requirements.

### Interrupted-work recovery — 2026-09-30

Reused `luna_setup_validation` (GPT-6 Luna medium) acknowledged first, then
mapped sessions and searched filenames. Fresh `recover_reward_source` and
`recover_scout_source` requested GPT-6.1 Sol medium and recovered disjoint
artifacts. Fresh `recovery_review` requested GPT-6 Astra medium and independently
verified recovery fidelity. Runtime logs confirmed all requested model/effort
pairs; all ran successfully with no routing failures. No worker made commits;
the lead integrated and validated the recovery.

Reward source/report hashes match the historical records exactly, and the
manifest/lock delta matches the logged diff. The sender scout yielded only a
partial helper patch, preserved inert with provenance. Locked diagnostic tests
2/2, targeted format/clippy and the explicit full pre-commit gate passed.
No database was opened. See [the recovery checkpoint](n6_recovery_checkpoint.md)
for artifacts, source evidence and missing snapshot inputs.


## September 30 restored N4 pair and sender scout

Continuation base `5aa3febfee14a3ad59c2b24c15d94a49013fd170`, same feature
branch. The lead kept sole database-operation ownership and shared evidence;
Sol alone edited the isolated qualifier crate in the shared checkout. No
production/storage-library/C++ implementation or `/build` operation changed.
Capacity was checked before each launch; Luna acknowledged startup before
Sol or Astra were launched. Runtime `turn_context` records confirm all three
requested model/effort pairs. No routing/capacity failure or escalation occurred;
no billing/quota inference follows.

| Thread | Requested and observed model / effort | Ran and result |
| --- | --- | --- |
| `luna_n4_audit` | `gpt-6-luna`, medium | Started first; recovered-artifact/key map and reused read-only historical sender/copy-evidence audit completed |
| `sol_n4_tools` | `gpt-6.1-sol`, medium | Guarded paths, bounded pair qualifier, four-key scout, provenance/rejection tests and README; 28 tests plus final 3 scout tests, strict clippy and formatting passed |
| `astra_n4_review` | `gpt-6-astra`, medium | Independent source/reference derivation, golden key, report and provenance review; corrections resolved and final source/reports approved |

Runtime session IDs, respectively:
`01a0f259-fabf-7441-a34e-3e5c75b978a7`,
`01a0f25a-56bc-7810-9bfa-62cb834404de`,
`01a0f25b-32c3-7643-afd8-0a4067e369e4`.

Lead validation: independent source/copy manifests and inode checks; fresh
bounded pair qualification; seven-point reward reproduction with only metadata
differences; four-key scout (four `history_unavailable` outcomes); explicit
pre-commit fast gate. A final test-only source-fingerprint change required a
second four-key scout invocation; observations matched exactly. No broad
inventory/replay or production route was run. See
[the execution evidence](n4_restored_snapshot_evidence.md) for artifacts,
remaining gaps and validation limits. Milestone 10 remains open.


## Budgeted continuation: authenticated sender paths

From `bf57aec01`, Luna medium acknowledged first and mapped existing read-only
APIs. Reused Sol medium implemented the four-key proof mode; reused Astra medium
independently approved source and matching report. All ran; no route/capacity
failure. Lead ran four proofs on the qualified copy: all NonMember, while earlier
raw HistoryUnavailable observations remain historical and unchanged. 15 focused
tests, strict qualifier clippy/fmt and explicit fast pre-commit passed. See
[proof evidence](n4_head_sender_proofs.md). No full inventory, replay or production
route changed. The user requested at least 25% weekly allowance remain; closeout
starts at 30% remaining. Initial sampled usage was 37% used.

Separate `astra_reward_contract` explicitly requested `gpt-6-astra`, high, ran
read-only to settle ambiguous vote-period/delegation-delay/jail semantics before
reward implementation. It identified D=H−7 under candidate delay5 and a 59-key
empty-jail-list bound. It made no source edits or DB opens; no routing failure
occurred. This is a contract result, not reward qualification or billing evidence.


## Independent reward vote/rate and retained metadata continuation

Base `3605bf909`. Reused Luna medium completed bounded API/provenance and
remaining-work mapping. Reused Sol6.1 medium (`sol_n4_tools`) implemented the
59-key diagnostic, shared certificate decoder and consensus point-fact facade.
`sol_reward_rate` explicitly requested `gpt-6.1-sol`, medium, started successfully
and implemented the independent rate diagnostic, existing calculator exposure,
and separately capped CF8 configuration diagnostic. Ownership was disjoint;
lead alone ran all database probes, and no worker used `/build`.

Reused Astra6 medium independently reviewed source and all exact report hashes.
Reused `astra_reward_contract` at Astra6 high settled consequential delayed-state,
Magnolia codec and historical jail semantics, then verified the complete Go
configuration codec against immutable source. The high escalation is limited to
these ambiguous contracts. All requested agents ran; no model/capacity routing
failure occurred. Assignment is not a billing/usage assertion.

Review corrections resolved before evidence acceptance: Magnolia activation,
exact-D physical absence reconciliation and tests, pinned expected stats,
canonical nested config RLP, and candid value-only lambda repository provenance.
Lead ran three bounded successful diagnostics, 43 qualifier tests, strict clippy,
format and the explicit pre-commit fast gate with 1,444 consensus tests.
See [the evidence](n4_independent_reward_inputs.md). Latest sampled weekly usage
during this batch was 51% used; stop new slices at70% used to preserve closeout
capacity and the user's requested25% reserve. No producer certification, full
native reconstruction, reward execution or publication is claimed.


## Independent reward planner integration

From `e63be522d`, reused Sol6.1 medium implemented the optional artifact-bound
mode in the existing reward diagnostic. Reused Astra6 medium independently
reviewed both code and fresh independent/legacy outputs; exact delayed identity
and signed certificate-hash gates were tightened before freeze. Root reran five
focused tests, strict targeted clippy, both seven-read modes and explicit fast
pre-commit successfully. All typed fields match using independent inputs; raw
legacy order remains different. See [planner evidence](n4_independent_reward_plan.md).

After delivering frozen source and its successful test handoff, `sol_n4_tools`
reported `Selected model is at capacity`. This is a late model-capacity failure,
not a thread-capacity or account-usage conclusion. No retry or substitution was
needed: root completed execution/validation and Astra approved the result.
Latest sampled weekly usage during this closeout was57% used. The conditional
native-effects diagnostic is separately scoped; it does not extend planner
acceptance to actual EndBlock or publication.
