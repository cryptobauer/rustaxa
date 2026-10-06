# Source-last genesis frame parity

Dependency: accepted bounded runtime4e14964ae; no runtime change in this slice.
The new producer uses the actual first-attempt frozen42 native genesis rows
from native_redelegate_source_last_absent/public.json, full1000 input, no prefix.
Fresh frame account shell is explicit: aa/d1 nonce0,balance1000000; native
nonce1,balance5000; block1,price0,gas200000,TestChainConfig,fix/Magnolia/Ficus/
Cornus0,bothAspendisabled,maxsupply1000000000. This shell and touched native seed
do not prove a complete physical account/checkpoint/root.

Both pinned Go revisions and independent uninstrumented controls agree across
seven cases. Native success has17writes15Go reads; source old0 count3->2->1,
current1 single creationc28001; destination old0 count2->1/new1count2. Direct gas
101912,CALL102678,STATICCALL102676,parentrevert102678,twochildren183422,
nonpayable29378 andunderfunded22678. Underfunded quotes80000 butdoesnotRun;
nonpayable is exact Method is not payable with quote0. Success returns one
1000 Redelegated event and empty native output. Parent rollback preserves native
raw writes but removes receipt logs. The second child has exact missing-source
failure and no additional writes/logs. Retained item1/position33 neverchange.
At price0 sendernonce0->1 persists; othernonce0 andnativenonce1 unchanged.
Nonpayable retains outervalue1 aa->d1 but reverts childnative transfer. Allaccount
fields are compared independently ofnative raw. Setup/Run readvectors, per-call
write/log suffixes and cumulativeLAST reduceALLwrites, includingafteremptyfailure.
Controls compareevery nonobserver output/raw/account/log/refund/status/gas field.
The manifest binds unchangedpins,source/harness/support/actualinitialseed hashes.

Rust builds real semantic genesis[33,31,32], not a changed desired snapshot.
Eachcaseusesonepublicsession,onejournal/sequence andone transactionposition0.
The twochild sequences0/1 observeearlier nativeeffects. RecordingPort checks
cumulativeLAST beforeeveryprepare; tests replay everyexpected-prior value and
compare orderedcalls/writes/gas/output/logs/native dispositions and finalrawplan.
Item2 goes31->absent->32,count2->1->2; retaineditem/position have no orderedwrite
orplanentry. Disposablefixture settlement comparesall selectedraw/accountfacts;
committedchain head0/principal/validatorstakes remainunchanged. The first target
run found a copied nonce1 setting; it was corrected to recordednonce0. Existing
framecorpora/tests are unchanged.

Tier1 derivative checks: newtarget/allframeregressions,EVMcheck/Clippy/package,
Pythoncompile,dualpin/controlrecord/reproduction,serialfast/precommit,whitespace,
frozenindependentSolmediumreview. The unchangedruntime inherits its accepted
ONbridge12/all15 evidence; no extra rebuild is needed for fixture-only changes.
Fullfirstoutputs/corrections remainin persistent r2 artifacts with prefix
source-last-absent-frames-. Acceptance requires all saved gates and review.
Historicalsimulation/estimate/trace,DELEGATECALL,rewards/broaderorders/laterblocks,
completephysicalroot,production,C++/storage/fallback/protocol andpush stayopen.
