# Shared bounded VRF qualification

The diagnostic qualifier uses one sealed engine for seven fixed voter profiles.
Source commit `3f1eb719d84bd30c682daf65f92235eab8f3913d` replaces six copied
implementations with thin wrappers and adds two fixed seventh CLI entry points.
This change does not route production code or change storage, protocol or codecs.

Run evidence is under
`/home/fry/artifacts/n4-vrf-shared-01a1353c-b1a0-71e3-a562-1d29c68efb96-20261013`.
`common-authority-review.md` and `common-authority-amendment.md` are the accepted
authority. `code-acceptance-review.md`, `command-ready-review.md` and
`seventh-voter/family-final-review.md` give independent source, command and fact
acceptance. Historical contracts and source copies keep their original pins.

## Modules and APIs

`experiments/evm_feasibility/snapshot_qualifier/src/vrf_shared_window.rs` owns
selection, fresh authority, prior proof validation, capture/replay and reports.
Its `Profile` enum is crate-private, fixed and immutable. Each profile has a
fixed voter, ordered candidate prefix, 47 row indices and prior selection index.
The engine derives roots, identities and keys from authenticated raw inputs.
Caller-supplied voters, roots, periods, keys and ceilings are not accepted.

| API | Inputs and result | Invariants and errors |
| --- | --- | --- |
| Six legacy modules' `preflight`, `capture`, `replay` | The same output/input paths as before; `Result<()>` | Forward only to the module's fixed profile. All old test bodies remain unchanged. |
| `seventh_preflight(output)` | Exclusive bounded report path; `Result<()>` | Authenticate six raw inputs plus the fixed prior bundle. Replay all 71 prior proofs. Derive all 47 requests. No database path inspection or open. |
| `seventh_capture(bundle, report)` | Distinct exclusive output paths; `Result<()>` | Use only the fixed read-only copy and existing storage capture owner. Construct the 47-request plan only after the full prior barrier. A failed capture emits a stopped report and qualifies no prefix. Successful capture still requires offline replay. |
| `seventh_replay(bundle, report)` | Outside-database bounded bundle and exclusive report; `Result<()>` | Repeat raw authority and prior proof. Consume the full fixed 98-proof bundle. Reject identity, key, order, unused data, malformed value and bound failures. No database input exists. |

The prior key must be a proved 32-byte nonzero logical member with the matching
physical Present value. Remaining logical absence retains its established
meaning; it never supplies a usable key or an invented zero. Final seventh
acceptance requires all 47 positive members with matching physical provenance.
Broader weight, cache, producer, owner and routing qualification flags stay false.

Existing `parent_input_handoff` and `rustaxa-storage` owners are unchanged.
The raw universe has 2,742 rows and ten complete 48-period candidates. The exact
seventh voter is `c9b2ed9876bd463f67a412bb7ce3ea269e3fdafa`; its first-D binding is
parent 41, selection 13, proof 64. Its derived logical key is
`fdad1437f176f1a0acaf8244356c9d0169241736fde0b7a1355e0d52e1ac2f6f`.

## Builder and guard

`tools/bounded_vrf.py` is the deterministic instance builder and guard.
`build(spec)` returns bounded JSON bytes after it verifies the sealed preflight,
exact source closure, source/binary/evidence pins, prior data, all 13 ceilings,
source sums and command limits. It does not inspect database paths or claim READY.
The exact spec and result need independent review. The build CLI publishes once.

`capture(freeze, reviewed_sha)` reads and authenticates one bounded freeze buffer,
then parses those same bytes. It validates schema and the canonical persistent run.
It consumes the fixed exclusive marker before offline pin, builder, HEAD, output
or database checks. Every later failure leaves the marker consumed. Invalid freeze
admission cannot select a marker elsewhere. An existing or partial marker stops
the command. Neither the guard nor an operator may reset it for a retry.

The guard compares three fixed metadata records before and after one read-only
child capture. Fixed alias checks may inspect those paths and their components;
no discovery, scan, control-file read or source write is permitted. Record equality
is narrower than full database byte preservation. `run_bounded` limits stdout
and the full child deadline, including wait after pipe EOF. Overflow, deadline
and child failures return a failed result. Results never assert fact acceptance.

`test_bounded_vrf.py` retains 19 offline controls for deterministic generation,
data substitution, source/count/byte limits, metadata/command admission, hash drift,
historical exception, collision, lexical database aliases, trusted marker failure,
invalid freezes, fixed metadata records, output overflow and closed-pipe deadline.
The positive builder fixture is synthetic and cannot authorize a physical command.

## Source authority and bounds

The complete Rust source closure is exactly 68 paths: all 65 old sixth paths,
the shared engine, and two seventh CLIs. `lib.rs` and six wrappers are reviewed
successors; 58 paths keep exact pins. Two Python tool/test sources also count
toward the 4 MiB aggregate. Each source has a 1 MiB limit. The measured complete
sum is 1,293,055 bytes.

The report retains 22 embedded source entries. It replaces one adapter pin with
the shared source and retains 19 first-D and two CLI pins. This envelope is not
the complete command source authority. Its repeated source sum is 1,041,247 bytes;
entries are not deduplicated to gain capacity. The complete freeze binds all
wrappers. Old-to-new comparisons permit only verified source/executable successors;
all selection, proof, value, usage, bound and qualification fields must match.

All 13 witness ceilings remain unchanged. The actual scope is 47 payloads,
98 proofs and seven raw artifacts/6,785,636 bytes, with a fresh 71-proof prior
barrier. Limits remain metadata 64 KiB, command freeze 128 KiB, bundle 128 MiB,
report 128 KiB, sink 64 KiB, result 16 KiB and child deadline 300 seconds.
Command limits are 400 occurrence pins, 450 MiB total, 256 MiB per file and an
8 KiB hashing buffer. The one historical 87,841-byte startup-map exception keeps
its exact path/size/hash-before-parse rule. No other metadata limit rises.

The seventh capture freeze has 254 pins/413,670,442 bytes. Its spec is 61,608 bytes
and freeze 62,651 bytes. The outcome seal charges the fixed parent and outcome
as 293 occurrences/415,153,441 bytes. Separate bounded offline replay commands
pin the new replay binary and the original first-profile reverse binary.

## Validation and accepted facts

All six saved accepted windows have complete equivalent successor reports;
their physical captures were not repeated. Gates passed: 113 qualifier library
tests, 185 package tests, two shared tests, 19 tool tests, one canonical parity
test, 19 actual invalid-argv controls, fmt/clippy/all-bin builds, fast gate and
actual source commit hook with 2,130 tests each. Storage 172/ON bridge four
evidence was reused after 106 unchanged pin occurrences and exact cache/evidence
checks. Changed qualifier pins have fresh equivalence/package coverage.

Independent Sol accepted the frozen engine after Astra settled authority. Sol
then accepted the exact fixed seventh command before its one guarded capture.
Lead and independent offline replay reports are byte-identical. All 47 values
are proved nonzero 32-byte members with matching physical Present provenance.
The original first profile rejects the new bundle at Identity/Payload/index 0,
with zero payload tuples and all qualification flags false.

Consolidation adds zero facts. Voter seven adds exactly 47: the total is 349,
with seven complete voters. The new marker and all seven old guards remain
consumed. Remaining diagnostic and production blockers are in the
[current checkpoint](n4_next_run_checkpoint.md). New sealed instances need the
required source/authority delta review and independent exact-command READY.
