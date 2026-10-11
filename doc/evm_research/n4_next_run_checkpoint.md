# N4 checkpoint — October 11

Branch `feat/rust/evm-state-db`; accepted implementation HEAD `47032231369aca78904c4a367830973d9e18e935`.
No push. Original base `eb13f969acb49c593692ad651a070abb20b62711`;
prior implementation `de81438f478fe34b379d5eb34801934e3a500604`.
Artifacts: `/home/fry/artifacts/n4-run-01a12b7a-2f06-7661-9183-a37257d00590-20261011`. Final records supersede interim stop/progress records;
historical failures and routing evidence remain intact.

## Accepted dependencies

| Local commit | Result | Independent acceptance |
| --- | --- | --- |
| `1fd035545` | Seven delayed headers900..894 linked from the accepted48 full-list periods;17 calls/60237 bytes/zero trie reads | Sol ACCEPT, exact offline replay; delayed-final-review.md/header-final-freeze.json |
| `edcb0ed5c` | Quota reset jitter at most±2 seconds from the immutable baseline; larger drift, expiry and rollback reject | Astra ACCEPT; quota-reset-final-review.md;31 tests per helper copy |
| `b158dc4c6` | Aspen candidate counters:48 periods/139 signed DAGs,64 tied-minimum blocks/60 author rows | Sol ACCEPT, byte-identical replay; aspen-final-review.md/aspen-final-freeze.json |
| `be62cc368` | Synthetic bounded account→storage witness capture/replay; shared accounting, strict transcript and bounded canonical reconstruction | Sol ACCEPT, independent11 witness+1 pinned Go tests; witness-final-review.md/witness-final-freeze.json |
| `470322313` | Three additive borrowed backend views for genesis, metadata and header, with unchanged existing APIs | Sol ACCEPT, independent3 tests; application-views-final-review.md/application-views-final-freeze.json |

Qualifier gates: header12 focused/59 library/1 adapter and Aspen7 focused/66
library/1 adapter, format/production lint/build/fast/hooks passed. Storage gates:
witness138 package tests and views141, each with one existing ignored live fixture
and one passing doctest; focused11/3 pass. Rust-enabled bridge built with12 jobs
and all4 tests passed for each slice. Format, production Clippy, whitespace,
fast gates and both commit hooks passed. The wider test lint retains15 existing
findings; complete first failures are saved. Both original view-file test modules
are unchanged, independently checked. No existing tests were weakened.

Freezes retain39 witness and31 view pins, including copied test executables and
complete gate outputs. Accepted header/Aspen executable copies remain available.
No new state/trie probe, inventory, C++ change or production route was added.
The synthetic constructor budget registrations are accounting, not authenticated
constructor proofs. No complete owner/cache or real19-transfer replay is accepted.

## Allowance and routing

Original baseline90% at2026-10-11T15:00:15.132Z; stop80%; reset1792322995.
Latest83% at2026-10-11T19:29:57.263000+00:00;
7 points consumed, 3 points remain above the floor.
This is account-wide allowance, including side use, not task tokens or billing.
Resume the same quota-budget.json, SHA256
`bd5664c3f082e5b961e5fb67deffde261e0bf72ea60112233af44c007fe7695a`;
do not initialize or rebase. Stop reason: reviewed ready slices complete; live
export remains blocked by the exact dependencies below, not an asserted quota stop.

Lead session01a12b7a-2f06-7661-9183-a37257d00590 is confirmed Sol6.1 medium.
Luna medium started first. Independent Sol6.1 medium reviewed headers, Aspen,
witness and views. Astra medium reviewed authority and quota recovery. All ran;
child runtime identities remain unverified. No routing failure or billing inference.
Detailed assignments/results remain in resume-routing.json.

## Blockers and next action

Live witness export needs a persistent reviewed RocksDB binding patch. Pinned
read-only CF subset open is supported, but the Rust wrapper calls create_dir_all
and allocates uncapped error strings before caller handling. Fix non-creating open
and bounded errors across open, pinned reads and raw-iterator status under the
existing DB owner. Do not patch disposable Cargo registry files or bypass the
owner with a separate raw-C handle. See witness-live-open-feasibility.md.

Then implement bounded live capture and shared constructor/pair wiring, including
fixed descriptors, finite path/metadata charges and all constructor proofs; review
the exact96-tuple live contract before a probe. Typed borrowed views now exist.
Raw reverse selection is observed provenance, not proof of history completeness.
Application caps exclude engine memory, OS I/O and RSS and are not yet proven
adequate for the copied database. No total probe ran or may be repeated blindly.

Missing total-vote inputs:48 authenticated native totalsD25706894..25706941 and
48 jailed-list preconditions. DPoS key is Keccak256(preimage04); slashing key is
leftpad32(02), followed by the established storage-path codec. Roots alone prove
neither native values nor absence. Missing/tombstone/unavailable never means zero.
Read total-vote-contract.md, bounded-witness-contract.md and witness-api-feasibility.md.

Delayed root commitments and default-mainnet/likelyv1.14.1 Aspen candidate policy
are accepted. Producer binary/configuration, native voter facts/weights, total/jail
facts, lambda, independent retained cache values, complete model and coherent
historical owner remain open. Keep H25706949/P25706948/sessionE25706944/
certificateD25706942 distinct. No writable owner, production adoption or full cache.

No owned implementation is pending. Checkpoint/scorecard closeout is separate from
source commits. Preserve initial edits to workflow, N6 queue/checkpoint and active
prompt, plus untracked .agents/, next-ten-point plan and three token audits. The
skill quota helper copy is synchronized; its pre-existing package is not staged.
