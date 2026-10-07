# N2 positive-reward partial: actual Go oracle

This records one synthetic P1 reward/commit and P2 partial redelegation under
public6c7e5338b22d5e596cc2365a88d1f94840e1ee1b and
localbb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418. It implements the accepted
[source contract](n2_rewarded_partial_redelegate_contract.md). This is oracle
preparation; Rust reward admission and adapter/account parity remain open.

The separate native_redelegate_reward_reference.go/.py producer and fixture
preserve earlier zero-reward producers, corpora and runtime. Genesis31/32 has
caller d1 and common owner a1 each1000 on both validators, initial balances5000/
3000 and native principal4000. Only yield changes to20. Blocks/year1, commission
100/10000, author/DAG reward caps0, active fix/Magnolia/Ficus/Cornus/Aspen1,
Aspen2/Cacti disabled and exact full configuration are recorded in the output.
Synthetic coherent P1 stats assign one vote to31; they are not PBFT production
stats. One live StateTransition commits P1, then executes P2 caller nonce0,
price/value0, gas200000, partial300 from31 to32; no P2 reward distribution.

Actual P1 mint800 creates source pool792/commission8 and native balance4800.
The Aspen1 minted-token raw row is0320. P1 EndBlock has no raw writes. Complete
physical rows number82 at genesis,91 afterP1,113 afterP2; account snapshots and
complete versioned rows are captured at each committed boundary. No pending
root or Go/Rust root equality is claimed.

Actual P2 gas101912, empty errors/output,12 ordered raw writes and14 backend
reads. The first two writes reduce old source0 count3->2->1. Next, actual
nonzero Account.SubBalance396 leaves native4404; Account.AddBalance396 leaves
caller3396. Source principal700/cursor2, reward node2 index396000/count2, head2
and source total1700 follow. Source pool becomes0/commission8. Destination
old0 count3->2->1, caller1300/cursor2, node2 index0/count2, head2 and total2300
follow. Owner pairs remain1000/cursor0; membership order stays[31,32]. Principal
remains4000; native4404 covers principal plus unpaid owner396 and commission8.
RewardsClaimed396 precedes Redelegated300. Exact raw vectors and interleaved
raw/account timeline are retained without reducing repeated execution writes.

Copy-only observers are installed solely in disposable pinned archives. Source
hash/needle guards cover raw writes, native backend reads, cache snapshots and
account add/sub bodies. The account observer copies values after unchanged
nonzero balance arithmetic. Frozen native cache observation uses immutable
committedP1 fallback; tombstones do not fall through. Seed/after-state inspection
runs outside execution observation and does not fill native caches. Separate
uninstrumented controls compare all execution, logs, accounts, facts and complete
physical rows; only observer vectors and coverage-derived final_raw are excluded.
Both pins and their controls match. Manifest binds pins, Go version, producer,
harness/support/observer hashes, outputs, controls and all four diagnostic files.

Run4 artifacts: /home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007.
First record failed compilation; the corrected diagnostic attempt exposed duplicate
support declarations and a missing ExtendedReader wrapper. Both failures stay in
first-run records. Corrected record succeeded; reproduction, Python compilation,
serial fast validation passed. The frozen Sol review checked source/control hashes;
its minor genesis0-to-P1 comment correction is retained. Final whitespace,
reproduced manifest identities and corrected frozen review bind local acceptance. No Rust source, storage owner, upstream C++, producer
facts, supplied data, runtime routing, broad/differential gate or push changes.
