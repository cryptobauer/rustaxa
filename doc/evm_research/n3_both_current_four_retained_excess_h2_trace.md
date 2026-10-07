# Count4 H1 retained-excess H2 trace

This bounded test uses accepted signed H1be90ad91f and unchanged runtimea16acef1e.
The fresh source contract is both-current-four-h2-contract.md in the run3
artifacts. Separate producer/harness/corpus/Rust test preserve old corpora,
shared support, guards and public storage/runtime APIs.

Selected H1 has108completeGo rows at root
`a8ea1ecbd354137b6ed5d14084c673eb9bd959979175f110b24cd20a56903b71`.
Three signed prefixes C0/O0/O1 leave C700/1300 and O800/1200, totals1500/2500,
both node1counts4 and all heads/cursors1, old0absent. Cnonce1/Ononce2/native1
and liquid balances3000/1000/4000 belong to this committed authority. Exact
config/derived identities/locking2/3/7 and all physical rows remain bound to H1.

Actual default TraceRunner executes one direct H2 partial300 prefix and full400
target in the same context2. Supplied nonces2^512 and2^512+1 stay unchanged;
trace behavior is separate from DryRunner's effective nonce override. Both
pinned results repeat/reproduce and retain complete original H1 before/after.
Default JSON reports gas101912, failedfalse, empty return and structLogs, with
separate diagnostics. This JSON does not expose the internal native raw vector.

A second independently equal H1 seed supplies an actual two-ExecuteTransaction
live-state witness, without EndBlock/Commit. It uses passive cache copies over
committed H1, with tombstones preventing fallback. The witness records exact
requests, per-step selected native raw/facts/accounts/gas/logs and node1/node2
keys. Both operations use101912 gas and emit300/400. After the prefix old1nodes
retain count2, Ocursor1/pairs8001200 unchanged; new2nodes count2, C4001600,
totals12002800. After the full old1count2 remains, source2count2/destination2count3,
Csourceabsent/destination2000, totals8003200/principal4000. Oaccountnonce2 and
balance1000 stay fixed, Cwide nonce advances twice. Original default-trace H1
closure stays immutable; the supplemental transition's pending physical rows
are separate authority. No pending root or default-trace native-write claim.

Rust opens public H1 history without refinalization or semantic raw injection,
plus independent complete Go readers. One public native session in context2
runs the real prefix/target through separate journals. Before every prepare and
after every native success, raw journals compare with the corresponding actual
Go witness. These witness reads are outside the fault wrapper. Rust checks
ordered prefix12/target15 writes and intermediate expected values: prefixold1
4->3->2/current2count2; targetsource2->1->2/destination2->1->3. These internal
ordered vectors are checked against source semantics and the actual state
witness, not an exported Go H2 ordered-write vector. LAST effects remain visible.

The fault wrapper measures14unique prefix keys includingold1 and15target keys
atcurrent2, explicitly excluding old1 and retained-owner pair/identity keys.
Every measured key receives corruption and a read error, with a fresh successful
retry; whole-prefix/whole-target quote failures are also checked. Total488fresh
sessions cover240faults240retries. Late target errors retain one completed prefix
before full session disposal. Each attempt compares full108concreteGo rows and
ALLpublicowner Column rows, height/header, accounts/pairs/order/rewards, all
three persisted receipts and full public validator metadata. Reopen and reader
session disposal keep both physical inventories unchanged. Separate witnesses
prove old1/O preservation; target authentication alone does not prove them.

Required gates are record/reproduction/Python compile, focused trace test,
EVM package/check/Clippy, serial fast/whitespace and fresh independent Sol medium
frozen review. Initial compile missed the new owner-row baseline binding; fixed
only in this new test. Copied count3 diagnostic labels were corrected to count4
before final package checks; parity assertions are unchanged. Runtime ONbridge
build12/all15 is inherited separately froma16acef1e. General counts/rewards,
nested H2, qualified real-window roots, recovery/integratedN4–N6 and production
cutover remain open. No fallback, upstream/storage-owner or supplied-data change,
broad gate or push is included.
