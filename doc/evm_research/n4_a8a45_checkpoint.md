# A8a45 checkpoint

Accepted local Rust commit: `3276d5887fc91614f903007002e51cf672c59587`.
No push. It adds the sealed a8a2 constructor and shared qualifier profile for
45 requests, with real omissions at delayed indices 24 and 45. Full 48-context,
96-proof RCV4 and fresh 71-proof prior checks remain required. Old profiles
keep their behavior. This is diagnostic support, not new fact acceptance.

Independent Rust-only acceptance covers five files. Storage 188, qualifier 196,
canonical parity 1, four ON storage bridge tests, six invalid-argv controls,
full native/first-D equality, fmt, clippy and all-bin checks passed. The fast
gate and actual commit hook each passed 2147 tests. The bridge build used 12 jobs.

VRF facts remain **673 accepted / 241 residual**. No new command READY, guarded
capture, independent final proof replay, fact ACCEPT or seal ACCEPT exists.

The two Python guard files remain uncommitted and unaccepted. They extend the
fixed builder and shared verifier. Final command/native comparison review,
complete positive builder coverage and final source acceptance remain required.
Do not execute capture from this checkpoint.

Current run artifacts and the controlling compact state are under:
`/home/fry/artifacts/vrf-residual-20261014-current/checkpoint-current.md`.
The machine checkpoint records pending hashes, gate/review results and next action.
Rust acceptance: `sol/rust-core-final-source-acceptance.md`.
Validation freeze: `source/rust-core-validation-freeze.json`.
Finite candidate authority: `astra/a8a45-successor-contract-v9.json`.
Historical contracts, logs, archives, captures and markers stay immutable.

Lead session: `01a13cd7-42b3-7f01-aaaf-2cc5b8493d55`.
Saved budget: `quota-budget.json` in the run directory. Fresh baseline 65%,
stop floor 55%; reuse this file without initialization on same-session resume.
The latest observation and consumed points belong in the machine checkpoint.
A new run needs explicit authorization and its own fresh baseline.

Next: finish guard checks and positive builder coverage; freeze and obtain
independent source acceptance; commit accepted guard work locally. Then build
an exact new command, obtain independent READY, execute one guarded capture,
run both full proof replays and reverse controls, and obtain fact/seal ACCEPT.
Preserve all 13 ceilings, 400 roles, finite body coverage and one-use guards.
Stake/jail/cache/owner/producer and real-replay gaps remain open.
