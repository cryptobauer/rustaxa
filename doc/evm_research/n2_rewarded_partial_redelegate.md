# N2 finite positive-reward partial adapter

The separate P2 profile consumes the [actual dual-pin oracle](n2_rewarded_partial_redelegate_oracle.md)
accepted in c652fc315. It reuses the existing Rust reward/redelegation kernel,
source/destination serializers and lazy StagedDposAccountPort. Existing zero-reward
admission, kernels, corpora and tests keep their rules. No upstream C++, storage
owner, bridge payload, protocol, fallback or production route changes.

The finite guard requires period2 in the measured post-fix/Magnolia/Ficus/Aspen1,
pre-Aspen2 regime; distinct source/destination and caller/common-owner identities;
exactly two validators and two positive delegators under both; complete ordered
validator/caller/owner memberships and principal/history provenance. Source pool
and commission are positive, destination rewards zero, and the existing exact
reward helper must compute a positive caller payout. Queues, slashing and corruption
are excluded. Addresses, principals, pools and amount remain typed domain inputs;
this does not admit arbitrary positive-index/history graphs or full/zero transfers.

Only two genesis nodes are live, both index0/count3. Both heads and all four graph
cursors are0, with four existing zero cursor mirrors and no stale heads. Current2
nodes are absent. The validator index map must be exactly empty: actual genesis
initializes it empty and P1 rewards leave it empty. The first checkpoint creates
its entries. A fresh Astra authentication report initially required two entries;
actual reward-kernel diagnostics and the bounded source correction disproved that
proposal. The original report and appended correction remain in run4
reward-auth-contract.md. No synthetic mirror entries are inserted.

The authenticator binds both validator and pool/commission rows, both owner rows,
all four principal/cursor rows, both old nodes and absent current nodes, and complete
iterable counts/items/reverse positions to current raw authority. These29 unique
keys cover the retained owner as well as the caller. It does not enumerate unrelated
physical orphans or remint supply. Normal preflight failures retain their cold
prefix and read no accounts. Lazy accounts are created only after this authority
check. Missing, mismatched or unavailable authority is a hard adapter failure.

Success returns native4800->4404 then caller3000->3396 as two expected-value balance
effects, the exact12 Go raw writes and RewardsClaimed396/Redelegated300. Source
old0 count3->2->1, new2 index396000/count2, caller700/cursor2, total1700 and pool0/
commission8; destination old0 count3->2->1, new2 index0/count2, caller1300/cursor2,
total2300. Owner principals1000/1000 and cursors0 remain. Assign session state only
after both serializers finish; any earlier error discards cloned state, balances,
trace and logs. Publication failure must abort the enclosing pending period.

The five new Rust tests compose actual finish_rewards(800) from complete genesis,
compare P1 reward effects to actual Go, then advance that complete unpublished
snapshot to P2. Raw authority is independently read from complete actual Go P1
versioned rows. They do not hydrate semantic state from Go rows or publish/reopen
P1. Success compares every selected final row, logs and separate ordered vectors,
with a repeated fresh session. There are29 reader failures,29 raw corruptions,
27 missing required rows and4 account faults:89 failures and89 fresh successful
retries, including two caller authority failures after staged native debit. All
29 raw keys are cached before the kernel, so these are not serializer faults.
A separate kernel/codec composition test removes destination principal only from
a disposable post-kernel clone. Destination serialization then fails after the
completed payout and eight scratch raw writes; the live session, reader accounts
and raw overlay remain unchanged. This is not a full-invoke late reader fault. There
are22 semantic exclusions plus5 fork exclusions and two normal preflight cases.
Initial API/type/setup/guard failures and corrections are retained. Earlier tests
are not retargeted or weakened.

Material limits: the outcome carries separate raw/account vectors. Go interleaves
the debit/credit after two raw writes; Rust publication applies account effects
before raw writes. There is no intermediate callback during publication, so this
slice compares final state, gas/logs and each vector. It does not claim exact mixed
publication ordering or journal rollback within invoke_native. Positive-reward
parent-frame revert, public EVM/journal publication faults, real public P1 commit/
reopen and historical API parity need new measured evidence and separate checks.
No synthetic result closes real-window/root parity, N1-N6, recovery or cutover.

Required validation: focused five tests and full affected consensus package,
EVM regression package, all-target checks/Clippy, ON rust_consensus_tests build
with12 jobs/all15 tests, serial rewrite-validate-fast, whitespace and independent
frozen Sol review. First-run commands/outputs/exits and exact freeze/review records
are under /home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007.
No push is authorized.
