# Restart checkpoint: run2 source-last work

Branch `feat/rust/evm-state-db`; current accepted runtime `4e14964ae`.
No push. N1–N6/Milestone10 and production cutover remain open.
Previous run:19 accepted commits049ec5e73..fd6a7313c, all required gates/reviews,
66->56% consumed10. Its artifact directory without r2 suffix is immutable.

Run2 authorized another10 points. Lead session01a10f48-83ee-7a32-8bff-722d10c36359,
actualgpt-6.1-sol medium. Persistent artifacts:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`.
quota-budget.json initialized once2026-10-06T12:51:46.374Z: baseline56%,target46%,
reset1791584594. Latest49% at2026-10-06T14:24:52.450Z:7/10 consumed,3remain.
Preserve baseline on resume; check fresh telemetry before new work/gates.
Root sole source writer and /build owner; Luna medium startup confirmed. Named
fresh Astra medium contracts and independent Sol medium frozen reviews confirmed;
actual routing and any corrections are recorded in reports. No billing inferred.

Accepted run2 commits: prep e0fe4e125; signed source-first H1 simulation e9fb32012
and estimation01060d04e; checkpoint b5d61a8f1; bounded source-last current-node
runtime da7357032 and frames681a4e9af; checkpointd4ed7b43c; signed source-last H1
simulation0fee6fd32 and estimationa11202839; source-last both-absent4e14964ae; checkpoint8b341e2; genesisframes9a63cff;
ordinary-prefixH1simulation47bb9e8 andestimation19aca73.
See [scorecard](../codex_slice_scorecard.md) and Git history for exact closeouts.
Signed derivatives use real canonical EIP155 prefix/public Rust finalize/reopen,
actual complete119rowH1 roots; source-last root4edf613... preserves currentnode1
count2. Simulation64freshsessions/28latefailures+retries and estimate64freshprobes/
24searchcalls pass. Both pins give101912/minimum and104977 C++ search. Their
10/11 frozen hashes and all required package/API/fast gates passed independent
review; source-last-signed-h1[-estimate]-closeout.json/review.md.

Current runtime: source-last current-node remains accepted; new exact genesis
both-absent path uses no prefix, caller[33,31]->[33,32], full1000, currentblock1,
oldcounts3/2/3, oldheads/cursors0, zero indices/pools and complete histories.
Go actual17writes15reads/gas101912; Rust17cold/5warm authenticated keys,
44failures+44freshretries, five new parity/exclusion/equivalence tests. All53
redelegation regressions and1501consensus tests/check/Clippy/EVM/ONbridge12+15/
Python/reproduction/serialfast/whitespace pass. Independent13correctedhashes
accepted. Contract corrected real-genesis index map omission; explicit-zero
equivalence added after review. Old semantic-only order probes stillRawIntegrity.
See [bounded runtime](n2_source_last_absent.md), source-last-absent-closeout.json
and review.md. Shared current helper/kernel/generic serializers unchanged.

Accepted genesisframes9a63cff: sevenactualdualpin/controlcases/no prefix/nonce0,
17writes15Goreads/singlecurrentnodecreation/retainednoeffects/cumulativeLAST;
all17frameregressions/EVM/check/Clippy/Python/reproduction/serialfast/whitespace
pass;18hashesindependentSolaccepted. Firstcopiednonce1failure corrected.
SignedordinaryprefixH1simulation47bb9e8: canonicalC->C selftransfernonce0,
receipt21000/noevent; all42nativebefore/prefix/end/commit views andcomplete
nativeaccountRLP/storage root unchanged. Newactual100rowH1rootef22a728...,
publicRustcanonicalfinalizeonce/reopen;136freshsessions/64latecorruption+reader
failuresandfreshretries/17reads;36API/EVM/check/Clippy/Python/reproduction/serialfast
pass;10hashesfreshSolaccepted. Firstold119rowexpectation correctedactual100.
Estimation19aca73: actualnew100rowidentity/8probes/104977search/min101912/below
101911OOG21912;64freshRustsessions/24searchcalls/perprobephysical/publicisolation;
37API/EVM/check/Clippy/Python/reproduction/serialfast/whitespacepass;11hashesSol
accepted. source-last-absent-h1[-estimate]-closeout.json/review.md.

Next namedcontract inprogress: existing-destination source-LAST full-source H2
trace derivative; realtwo-validator genesis[32,31], caller[32,31], full31->32,
sourcevalidator retainedbyotherowner. Luna map luna-after-absent-h1-seam.md names
originalfull-sourceproducer/Rustowner paths. Fresh Astra checks smallestnew
order-only profile; no new producer/code yet. Do not copy signedordinaryprefix
or alter oldconfiguration merely because the earlierprofile used it. New actual
H1root/rows/nativeorder andH2trace outputs remainmandatory. The genesisboth-
absent runtime's invocation1 bound doesnot authorizeH2 trace behavior.

Continue approved ready N2/N3 gaps after accepted closeout, within remaining3points.
At46 stop new work/delegation/largegates; finish in-flight atomic work and minimal
checkpoint. Keep unchecked work explicit. Do not alter supplied data/upstreamC++,
add fallback, run broad/differential gates, push or widen production scope.
Unknown real producer overrides/configuration/capture facts and real-window/root
parity/N4–N6 remain open; synthetic fixtures do not close those gates.
