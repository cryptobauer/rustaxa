# Signed ordinary-prefix source-last genesis H1 simulation

Dependency: bounded runtime4e14964ae; runtime/public owners unchanged. Existing
current-node signed H1 fixtures stay intact. The new actual producer signs
canonical EIP155 chain666/key31*32 C->C nonce0,price/value0,gas200000,emptyinput.
C is77952ce83ca3cad9f7adcfabeda85bd2f1f52008. a1 is absent after its genesis
staking debit, so it is not used as a preexisting recipient. Public Rust canonical
decode/recovery binds sender/hash/recipient/emptydata; public PBFT/PeriodData
finalization runs once atH1 then history reopens normally. Receipt status1,
gas21000,no native logs. No reward/graph owner is bypassed.

Actual both unchanged Go pins preserve42selected native presence/bytes/facts
before/prefix/EndBlock/Commit. Complete native account RLP (includingstorage root
andcodehash) remains identical. Oldnode0 counts3/2/3 andindices0,head/cursors0,
allcurrent1 absent; caller[33,31],owner/global[33,31,32],caller1000/1000,
validatorstakes2000/1000/2000,total5000/zero rewards. Cbalance3000/nonce1,a1absent,
nativebalance5000/nonce1/exactcode. New complete100rowH1 root
`ef22a728c0d063b9d9366ba2052632f6b33d11905442a528d2e5c605013bb0a0`.
Signedhash06eb38c2c4bb2350cbe153af6ed8a3806018e96ea0711c2b02f785fc27c021d6.
No oldroot/rowcount is relabeled as new evidence. The first Rust target retained
an old119row expectation, then corrected to actual100 before success.

Same-H1 DryRunner full1000C31->32 gives101912gas/one1000event/emptyoutput,
suppliednonce2^512 preserved separately, actualeffective2. Repeatexact andcomplete
H1 physicalrows/root/accounts/publicfacts remainunchanged. Rust uses fresh public
begin_native_simulation(1), no graph advancement or snapshot insertion. Physical
Go reader/accountserialization is checked independently of public semantic owner;
no syntheticRustroot equality is claimed. Rust internal17write witness/current1
singlecreation comes from separately accepted staged oracle, not DryRunnerwrites.

Across twoowner andtworeader lifetimes136freshsessions give72successes and64
latefailures withfreshretry. Actual17authenticatedkeys; indices9..16 each get
independentcorruption andreadererror, exactreadprefix, propagatederror, single
sessiondrop, preserved suppliednonce andfullcommittedphysical/publicisolation.
Success checks exact gas/output/log andretainedmembershipnoeffects. Public reads
bind delayed/currentstakes,pairs/order/rewardwords/accountnonces/balances before
andaftereveryattempt. Completephysicalreader binds allaccountRLP/code and42rows.

Tier1 checks: newtarget/historicalAPIregressions/EVMpackage/check/Clippy,
actualdualpinrecord/reproduction/Pythonsyntax,serialfast/precommit/whitespace,
frozenindependentSolmediumreview. Runtime ONbridge12/all15 remainsaccepted and
unchanged. Fullfirstoutputs/corrections use source-last-absent-h1- prefix in the
persistent r2 artifacts. Acceptance requires everygate andfrozenreview.
Estimation requires the newactualH1 identity andnewsearchtranscript; traces,
laterblocks/rewards/broaderorders,production/C++/storage/fallback/protocol and
push remainoutside this slice. Realnetwork N4–N6 acceptance remainsopen.
