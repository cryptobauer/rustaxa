# Restart checkpoint: run2 bounded native work

Branch `feat/rust/evm-state-db`; current accepted runtime `072fc39fb`.
No push. N1–N6/Milestone10 and production cutover remain open.
Previous run is closed:66->56%,10 points,19 accepted commits
049ec5e73..fd6a7313c; its unsuffixed artifact directory is immutable.

Run2 lead session01a10f48-83ee-7a32-8bff-722d10c36359, actualgpt-6.1-sol medium.
Persistent artifacts:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`.
quota-budget.json initialized once2026-10-06T12:51:46.374Z:baseline56%,target46%,
reset1791584594. Final46% at2026-10-06T15:04:05.135Z:10 points consumed; run closed.
See final-quota.json. No new work may start under this completed budget.
Preserve baseline on resume; check fresh telemetry before any new work/gate.
At46 start no new implementation, delegation or large gate. Finish only in-flight
atomic work and minimal checkpoint; never commit unchecked work.

Accepted run2 work through `072fc39fb`: signed source-first H1 simulation/estimate;
source-last current-node runtime/frames/signed H1 APIs; exact block-one source-last
both-current-absent runtime/genesisframes/signed ordinary-prefix H1 APIs; bounded
existing-destination BOTH-current oracle `4ec89b7c1` and runtime `072fc39fb`.
Accepted local commits include preparation/checkpoints. Exact commits,
corrections/checks/routes are in [scorecard](../codex_slice_scorecard.md), Git and
per-slice closeout JSON/review files. Root is sole source writer/build owner;
Luna medium startup confirmed, named fresh Astra medium contracts and independent
Sol medium frozen reviews confirmed. A stale reviewer telemetry event was resolved
once; no model substitution or billing inference.

Current runtime: real two-validator genesis [31,32], caller and other each1000 at
both validators; partial30031->existing32 then full700 sameperiod1. Both current
nodes count2 beforetarget. Go/Rust target15writes/101912gas, source2->1->2 and
destination2->1->3; sourcecaller deleted/destination2000, memberships[31,32]->[32],
aggregates1000/3000/principal4000. Separate bounded source-copy helper preserves
full/new helper and generic serializers; shared staged eligibility rejects other
current-source full/existing shapes. Fourtests/88corruption+reader failures and
88freshretries/25exclusions/late local-serialization rollback. All57redelegation,
1505consensus/EVM/check/Clippy/ONbridge12+15/serialfast/Python/reproduction/whitespace
pass;12frozenhashes accepted. See [runtime](n2_existing_destination_current.md),
existing-current-runtime-closeout.json/review.md. Earlier accepted profiles remain
covered; prior corpora/support/upstream C++/storage owners unchanged.

Accepted existing-current frames `7531d4dc2`: sevenactualdualpin/controlcases,
realprefix12/target15writes/source2/destination3/caller swap, CALL/STATICCALL/
parentrawsurvival/logrollback/twochildren/nonpayable/underfunded. Review corrected
RustAspen1DefaultGENESIS toMAX matching Go. All19frameregressions/EVM/Clippy/
serialfast/Python/reproduction/whitespacepass;18correctedhashesaccepted, inherited
unchangedruntimeONbridge12+15. See [frames](n2_existing_destination_current_frames.md)
and existing-current-frames-closeout.json/review.md.

Fresh Astra medium H1 contract is settled: existing-current-h1-contract.md.
NewactualcanonicalC nativepartial30031->existing32 preserves BOTHnode1count2
throughEndBlockCommit. Oracle preparation has105completeH1rows/root
e4781dc89abf0c3ff461290bc536b5ed807a39ded5c23cf9525349e07727e8b7,
prefixhashcfc88dd3.../gas101912/event300; sameH1DryRunnerfull700 repeats unchanged
H1/effective2/101912/event700. Caller3000nonce1/owner1000nonce0/native4000nonce1.
Review found inheritedlocking1; newproducer explicitlysetsDPOS2/Cornus3/Cacti7
matchingtop-level profile. Correctedrecord/reproduction/Python/whitespacepass;
Rustsourceunchangedserialfastpass. Eightcorrectedhashesaccepted by freshSolmedium, localcommitdd0c2f458; do not claim publicRust history acceptance from this oracle.
See [oracle preparation](n3_existing_current_signed_h1_oracle.md) and
existing-current-h1-oracle-closeout.json/review.md.

Next work needs a new authorized budget: add publicRust canonicalH1 finalize/
reopen using the new105row physical identity and caller3000 AND ordinaryowner1000
accounts, native4000. Preserve BOTHcurrentnode1count2 before disposable simulation;
measure actual newauthenticationsequence and inject everykey corruption/reader
failure withfreshretry/isolation. No newRustH1code yet. Nativeinvocation1 is not
H2. Estimate/trace remain separate. The prior order-only source-last H2 proposal
is deferred because accepted full-source single_full_last_item covers its branch.

Unknown real producer overrides/configuration/capture facts and real-window/root
parity/N4–N6 remain open. Synthetic fixtures cannot close these gates. No push,
production route, fallback, supplied-data/upstream exception or broad differential
gate is authorized. This run is complete. Do not reset its budget or continue implementation without
a newly authorized run.
