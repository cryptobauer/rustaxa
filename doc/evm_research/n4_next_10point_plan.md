# N4 next slice: offline parent input handoff

Prepared after implementation `8a4a613dbbc4c82dcf5d608efb6f65d9d288f048` and
checkpoint commit `9539294ba99f6be60f54095b3b0518eabf32ace1`.
The filename retains its historical ten-point name; no new ten-point budget is
authorized or initialized. This preparation does not start implementation or a probe.

Read the accepted [storage checkpoint](n4_next_run_checkpoint.md). Its333-file
freeze remains unchanged. Current preparation artifacts are under
`/home/fry/artifacts/n4-next-prompt-20261011`; prior storage evidence is under
`/home/fry/artifacts/n4-binding-resume-20261011` (S below).

## First slice and acceptance

Create a separate offline parent-input handoff that binds accepted native proofs
and candidate Aspen policy to the authenticated48 source periods25706901..25706948.
It must derive D=s−7 with checked arithmetic and match each complete period/root
identity against the accepted plan. The96 native tuples cover D25706894..25706941.
All48 native totals534879 and48 jail preconditions are accepted only through the
frozen bundle and independent replay; do not inject this observed constant.

The original parent report's null totals, seven missing roots and unqualified DAG
predicate are historical diagnostics. Preserve its bytes and validator. A new
schema must show which exact gaps are closed by later evidence, and which remain.
Candidate Aspen policy is accepted under the default-mainnet assumption:48 periods,
139 DAG blocks,64 minimum-difficulty ties and60 author rows. Producer policy and
configuration remain unverified. Do not treat a candidate predicate as full reward
cache or production authority.

First verify the accepted evidence without opening a DB:

```sh
rtk proxy python3 -E /home/fry/artifacts/n4-binding-resume-20261011/verify-final-closeout.py
```

Then settle the exact offline admission/join contract and obtain independent
Astra review before dependent implementation. This plan is a reviewed handoff
design, not an implementation contract marked READY for execution. The actual
executable, API composition, allocation rules, commands, output pins and test
selectors must be fixed in the next run's contract.

## Fixed artifact inputs

Admit only these six canonical regular files, with exact sizes and hashes.
Follow no path from embedded JSON. `next-input-pins.json` preserves this set.

| ID | Path | Bytes | SHA256 |
| --- | --- | ---: | --- |
| 1 | `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/interval-history-authentication.json` | 2766744 | `3513c1720759ed3dd07784ab66d69661742af82ceeee56d0c527c835da594aa5` |
| 2 | `/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/parent-cache-fields-cap-tests.json` | 2257600 | `b54906443d6504bd1ad59d2208791d4cd710ffe2ae4a0731f8306af7b5521a3b` |
| 3 | `/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/delayed-header-capture.json` | 256840 | `7d57845a66978c6107b2d3f733f7e9e743cde0761926d07060517bc11e7c867f` |
| 4 | `/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011/aspen-candidate-policy.json` | 49682 | `f172a66100d41d26b3d0f860d73892fc8c7f52bb4392ba77c1f91ac292d0fcc8` |
| 5 | `/home/fry/artifacts/n4-binding-resume-20261011/native-live-bundle-frame-cost.rcb1` | 1026265 | `a0a36875f90defe9dad0d7450416c75f57c06e9eee16cc53f27c666f95fcc761` |
| 6 | `/home/fry/artifacts/n4-binding-resume-20261011/native-live-replay-frame-cost.json` | 10911 | `2ccf155e2a851cd52161f090bcd3cc76b09f212a44647ba8f76b7db0b7511fbb` |

The unique raw artifact set is6368042 bytes. Native authority admission uses
inputs1,2,3; Aspen derivation uses inputs1,2. Input5 is the authenticated witness
source. Inputs4 and6 are comparison/provenance reports; their claimed success
flags and expected counters must not replace fresh derivation or proof replay.

Proposed admission limits for the next contract: six distinct artifact files,
at most11 fixed artifact admissions, at most16MiB cumulative raw artifact bytes
(including repeated reads), and262144 encoded output bytes. Reading all six once,
then the three native authority inputs and two Aspen inputs again consumes
16673570 bytes; do not forget these repeated reads or expand the bound silently.
Prefer shared admitted views. Calling native_witness::replay after outer bundle
admission adds a twelfth artifact read and1026265 bytes; it exceeds these
proposals. Use the admitted bundle with replay_diagnostic_bundle and decode_totals,
or stop and obtain review of a corrected composition before implementation.
These limits are proposals, not implemented gates.
Independently confirm actual loader composition and charge each admission before
allocation. Parser/authority allocations are separately source-specific work,
not covered by the storage witness budget; make no whole-process memory claim.

Keep the existing13 witness ceilings unchanged for one complete offline replay.
Require record/schema/root/account/path authentication, strict ordering, conflicting
record rejection and full consumption. No database owner, DB open, point read,
cursor, inventory, state capture or supplied-tree mutation is permitted. Engine
memory, allocator overhead, OS I/O and RSS remain outside the stated application
bound. Source profiles use a fixed pin list; source/executable bytes and hashing
bounds must be named separately before execution. No source-tree scan.

## Existing APIs and owned paths

Inspect only the required spans first:

- `snapshot_qualifier/src/native_witness.rs`: load_fixed_plan, decode_totals and
  the existing offline replay composition.
- `snapshot_qualifier/src/aspen_policy.rs`: load_fixed, derive, AuthenticatedPolicy
  and bounded publication.
- `snapshot_qualifier/src/parent_reward_fields.rs`: derive_available_fields and
  unchanged missing-input/schema validation.
- Storage replay_diagnostic_bundle and its accepted witness/codec/verifier APIs.

Prospective implementation ownership: new
`experiments/evm_feasibility/snapshot_qualifier/src/parent_input_handoff.rs`,
its focused tests and `src/bin/parent_input_handoff.rs`, plus a narrow lib export.
Any required borrowed helper must be additive, preserve old validators/pins, and
receive explicit contract review. Do not generalize historical rebinding or
retarget old source pins merely to make a saved report pass. No storage or C++
change is anticipated; document a concrete blocker before enlarging ownership.

The output contains48 ordered parent rows and exact source/digest bindings.
Reference the2742-row voter manifest by its pinned input, digest and counts instead
of copying it into a262144-byte output. Reject missing, duplicate, reordered,
conflicting or unused bindings. Qualification is per family: native totals/jail
and candidate Aspen facts may be true; producer, voter weights, lambda, cache
equality, model completeness, owner, public finality, transition and production
authority remain false. Failure produces no accepted48-row prefix.

## Remaining authority inputs

| Family | Exact remaining evidence | Next decision |
| --- | --- | --- |
| Voter facts/weights |2742 unqualified value/proof rows for914 delayed-period/voter pairs,20 distinct recovered voters; three semantic keys per pair | Derive keys from accepted codecs and recovered signed votes; select one named consumer family, then new bounded proof contract |
| Lambda |48 nearest-at-or-before selections for source periods25706901..25706948 | Fix policy, source class, selected raw key/value/update period and selection provenance before reads |
| Retained cache comparison |48 expected rows currently have size/hash metadata but lack raw expected RLP in the accepted report | Locate already saved qualified bytes or specify exact export requirements; no cache synthesis as an oracle |
| Producer policy |Binary/configuration and applicable historical native policy remain unverified | Preserve supplied default-mainnet/likelyv1.14.1 as an assumption, not a recovered fact |

After the handoff is accepted, prepare one bounded remaining-family authority
contract. New tuples are not authorized by the old96-tuple driver. Do not infer
that global empty-jail preconditions qualify all per-voter jail keys or weights.
Do not repeat the69 earlier unavailable tuples without evidence that changes the
justified inputs or resolves retention. No speculative address/key sweep, manifest
expansion or automatic cap increase. If a needed input is unavailable, record its
exact identity/key/evidence and recovery requirement; continue independent ready
offline work. New live commands require a separately READY exact-command contract.

## Gates, review and stop rules

For this documentation preparation: check references, input pins, exact scope,
quota instructions and whitespace; obtain independent handoff review; commit
only these documentation changes with the configured hook. Preserve earlier
versions in the preparation artifacts. No new baseline or probe is started.

For future implementation: narrow nonzero matched tests first; wrong period/root,
D=s−7 errors, tampered raw witness, claimed-success-only input, conflicting/reordered
or missing bindings, unconsumed records, cap stops and false authority flags.
Run affected package checks/tests, format and production lint, focused parity,
required fast checks and actual commit hooks. If storage changes, verify
RUSTAXA_ENABLE=ON, build rust_storage_tests with12 jobs and run it. Save complete
first-failure logs. Broad/differential campaigns need approval with exact commands
and datasets prepared first. Freeze sources, inputs, executable, output and gates;
require independent offline replay and final review before acceptance.

Use Sol6.1 medium for serial implementation, Luna medium first for bounded maps,
and a fresh explicitly configured Astra medium reviewer for the new authority
profile. Check capacity and reserve Luna's slot; no silent model substitution.
Record requested routing, confirmed startup, results and routing failures; runtime
identity and billing remain unknown unless independently observed.

Current user instruction is “continue and ignore budget for now”. It overrides
quota stopping until changed. Preserve the original budget file and90/80/83
historical record; never initialize/rebase it or transfer its identity to a new
lead. Fresh quota observations are account-wide, including side use; unresolved
telemetry is recorded without a numeric or billing inference. Preparation grants
no new allowance baseline. An actual model/account limit or missing review
capacity still requires a resumable checkpoint before dependent work.

No full reward-cache construction, writable owner,19-transfer execution, supplied
data changes or production adoption is authorized by this handoff. Keep H25706949,
P25706948, session E25706944 and certificate D25706942 distinct. Commit accepted
work locally, documentation separately; never push. At completion or blockage,
record accepted implementation HEAD, pending paths/hashes, checks/corrections,
reviews, routing, quota history, exact blockers and the next executable action.
