# Accepted fixed VRF successors eight through ten

Source commit `b16511d9eeee881397ef70e5c46dc736927ff326` extends the accepted shared
path to ten sealed profiles. Three independently accepted 47-row windows add 141
facts:349 to 490, and seven to ten complete voters. This is diagnostic VRF input
qualification. Broader cache, weight, producer, owner and production flags remain false.

R is `/home/fry/artifacts/n4-vrf-successors-01a13763-641e-7371-96ad-102630143805-20261013`.
Read R/successor-authority-amendment.md, code-acceptance-review.md and
metadata-fit-supplement.md. Each ordinal-voter directory has its own
command-ready-review.md, family-final-review.md, lead/independent replay and
fact-outcome-freeze.json. Old shared and R2/R3 evidence is immutable.

| Voter | Fixed identity | First-D parent/selection/proof | New facts | Outcome occurrences/bytes |
| --- | --- | --- | --- | --- |
| Eight | d27c327e3dfcfce0acf72438f6e7a7e6c87e06ef |44/14/65|47|345/417158352|
| Nine | d6d73e4c65c92787a2bbc80d109c293d8d3477af |47/15/66|47|345/417135714|
| Ten | f27d87cda5bc0bfac6ef93222cfd83c6e8dc9f0d |56/18/69|47|345/417137389|

## Source and API contract

`vrf_shared_window.rs` owns the finite private Profile enum, exact voter prefix,
47 row indices, prior indices, selection, fresh authentication, capture and replay.
The original seven table entries and wrappers stay fixed. New Eighth/Ninth/Tenth
entries derive identities, roots and keys from authenticated raw inputs, then
check the reviewed exact bindings. There is no caller voter, root, key or cap API.
Before a remaining plan or opener, the engine authenticates the six raw inputs
and freshly consumes all 71 first-D proofs.

For each of eighth, ninth and tenth, these public fixed functions return
`Result<()>`. The six CLI entry points dispatch only to their fixed functions.

| Function | Inputs and output | Invariants and errors |
| --- | --- | --- |
| `*_preflight(output: &Path)` | Exclusive outside-DB report path; bounded authenticated report | No DB inspection/open. Full raw and prior barriers precede all 47 tuples. Invalid authority or output admission returns an error; no partial authority qualifies. |
| `*_capture(bundle: &Path, report: &Path)` | Distinct exclusive outside-DB bundle/report paths | Use only the fixed read-only copy after exact guard READY. Complete 47 requests and fixed 98 proofs. Storage diagnostic failure attempts a stopped report; authority, path and publication errors return a Result error and can leave no report. No case qualifies a prefix. |
| `*_replay(bundle: &Path, report: &Path)` | Outside-DB bounded bundle and exclusive report | No DB input. Consume all 98 proofs and exact EOF. Diagnostic failures attempt a stopped report; admission, authority and publication errors can leave no report. Every failure returns an error without prefix qualification. Final fact acceptance additionally requires 47 positive members. |

`bounded_vrf.py` retains the deterministic builder and one-use guard.
`sealed_profile(profile)` returns only one fixed seventh/eighth/ninth/tenth record
or rejects an unknown profile. `build(spec_path)` reads capped explicit pinned
metadata, checks the selected fixed preparation, exact 74 + 2 source closure, exact
embedded 19+shared+two-CLI paths and ordered 25 owner paths, and returns deterministic
freeze bytes. It has no DB access and grants no READY. New preparations use only
the fixed ordinal basename under the admitted run; their exact pins and complete
rows require independent review.

`capture(freeze_path, reviewed_sha)` authenticates and parses the same bounded
bytes, admits the fixed run/profile and consumes the exclusive ordinal marker
before offline pin, reconstruction, HEAD, output or metadata checks. All admitted
failures keep the marker consumed. Only three fixed metadata records per pass
and one read-only child are allowed. Invalid/unknown input fails closed; no retry,
scan, supplied-data write or automatic fact acceptance exists.

The closure has 74 Rust/owner paths and two Python sources, totaling 1319694 bytes.
Only the shared engine changes among the previous 68 Rust paths;67 remain exact.
Six new fixed CLIs are added; no copied adapters, C++, storage owner or production
route changes. Reports retain 22 embedded entries and their checked repeated sums.
Source-successor and binary-successor mappings remain explicit and frozen.

## Bounds and metadata correction

All 13 witness ceilings,47 payloads,98 proofs and the fresh 71-prior barrier remain.
Metadata 64 KiB, freeze 128 KiB,400 occurrences/450 MiB, per-source 1 MiB/aggregate 4 MiB,
sink 64 KiB/result 16 KiB/deadline 300 s and the exact historical 87841-byte exception
stay unchanged.

The first compact eighth spec measured 72070 bytes and was rejected before any
spec/freeze or physical attempt. Astra reviewed an exact 31-representation exclusion:
18 new argv logs and nine new frozen instruction copies have retained pinned
records; current source/tool manifest files duplicate directly charged 74 + 2 arrays;
current source-only review and binary-freeze records are in the retained code/gate
freeze and direct two-binary array. All 180 historical occurrences, current actual
instruction pins, mandatory records and complete gate record/log pairs remain.
There is no deduplication of charged occurrences or cap increase. The final
supplement preserves the first amendment by exact path/bytes/SHA.

Actual specs 65434/65422/65422 and freezes 77303/77284/77284 pass admission. Each
command has 314 occurrences; its complete measured total is below 450 MiB.
Each outcome independently charges its own fixed command parent and 31 outcome
pins:345 occurrences. These are separate bounded commands, not a combined capture.

## Validation and facts

Frozen independent source/gate ACCEPT covers 186 package tests, three focused tests
with complete seven-old-report equivalence and new substitution/prior/reverse
controls,22 Python controls, one canonical parity test,18 actual invalid-argv
controls, fmt/clippy/all-bin builds, fast 2130 and actual pre-commit 2130.
Storage 172/ONbridge 4 reuse follows 106 unchanged source/evidence occurrences and
exact cache checks. Changed qualifier behavior has fresh gates.

Each exact command received independent READY before one guarded capture. All
three succeeded. Lead and independent full Rust replay reports match byte for
byte; all 141 new values are proved nonzero 32-byte Members with matching Present
length/SHA. Fixed original-profile reverses stop at Identity/Payload/index 0 and
zero payload tuples. All proof/value/provenance, selections, raw inputs, prior
barriers, bounds and qualification fields are compared. Before/after fixed
metadata equality is narrower than full database preservation.

R/timing-record.json records gate/commit duration, request-to-report review wall
intervals, corrections and recurring instance times. It does not estimate billing.
Fresh quota 97%/floor 87% is retained in R/quota-budget.json; final observation is
in R/final-quota.json. Requested routes are Sol 6.1 medium lead/reviewer, Luna
medium bounded maps and Astra medium authority. All ran; child runtime identity
is unknown. Original routing transcripts remain preserved with a separate correction.

All ready fixed successors are complete. The
[current checkpoint](n4_next_run_checkpoint.md) retains the 424 remaining VRF,
1828 stake/jail and lambda/cache/producer/owner/real replay gaps. Local commits
only; no broader N4 or production acceptance follows.
