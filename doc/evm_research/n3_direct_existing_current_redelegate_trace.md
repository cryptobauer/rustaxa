# One direct existing-current H2 trace

The fresh Astra medium contract is existing-current-trace-contract.md in the
run3 artifact directory. Runtime072fc39fb and the signed105-row H1 identity
remain unchanged. The new Go/Python producer and Rust sibling execute one
partial30031->existing32 prefix, then full40031->32 in one H2 session/journal.
The semantic owner comes from accepted public canonical H1 finalization, then
reopen. The physical owner has the exact complete H1 root
e4781dc89abf0c3ff461290bc536b5ed807a39ded5c23cf9525349e07727e8b7.
Caller3000nonce1, ordinary owner1000nonce0, native4000nonce1 and supply8000
are unchanged. No physical hydration of the semantic owner is used.

Both pinned actual default TraceRunner executions repeat from fresh state and
produce identical target JSON: gas101912, failedfalse, empty returnValue and
structLogs. Diagnostics are empty. Complete committed H1 before/after matches
the accepted signed fixture exactly. The original supplied nonces N=2^512 and
N+1 remain unchanged. TraceRunner does not apply DryRunner's nonce replacement.
Execution is period2, selected state1, timestamp1700000002, author31, gas200000,
price/value0, direct Call and nonstatic. The manifest binds all producer/support,
snapshot-helper/target, seed/configuration, stderr and output identities.

The Rust public begin_native_session(execution2,selected1) owns one factory,
port, sequence and journal. Before target quote, every cumulative last prefix
write matches live raw_storage. Each outcome's ordered expected values are
replayed against that journal plus earlier local operations. The prefix has12
serialized writes, events300, caller400/1600 and totals1400/2600; both current
node2 counts are2 and both old node1 entries are deleted. The initial new test
incorrectly expected14 serialized writes; the saved first failure led to the
correct12 count with explicit deletion and preserved-owner assertions. No old
test, runtime or corpus was changed.

The target witness has15 writes, source node2 count2->1->2, destination node2
count2->1->3, source caller deletion, destination caller2000/cursor2, totals
1000/3000 and source-first membership swap/removal to[32]. Owner pairs and node0
entries receive no mutations; native account mutations are empty. Event400 is
checked. These are internal Rust effect witnesses supported by reviewed Go
source and accepted runtime authority. Default Go JSON does not measure native
writes, logs or node counts.

Eight normal runs span two semantic owner lifetimes, two reader lifetimes each
and two fresh traces. The measured15 target authentication keys are distinct
and stable, and include both node2 entries. At every key a forwarding live
journal injects corruption and read failure. Prefix and target quote errors are
also injected. Each failure has exact Prefix/Target stage/index0 and the required error text fragment,
one disposal, no partial response, retained local prefix when applicable and
unchanged public H1 header/state/full physical rows. Every fault is followed by
a fresh trace that runs its own prefix and succeeds. This gives128 hard faults
and128 fresh retries; total264 sessions. Normal and retry calls match actual
Go JSON, original requests and native invocation metadata/funding178088/quote80000.

Required checks are actual dual-pin record/reproduction, Python compile, targeted
trace, EVM package/check/Clippy, serial rewrite-validate-fast, whitespace and fresh
independent Sol medium frozen review. Runtime is unchanged, so its accepted
Rust-enabled bridge build12/all15 is inherited separately. First output, failures,
corrections and checks remain in the run3 directory with existing-current-h2-trace
prefixes.

No intermediate settlement, EndBlock, new journal or publication occurs in H2.
This one default trace does not close OE/RPC, nested/delayed trace, rewards,
real-window root parity, production routing or Milestone10. No supplied-data,
C++/storage owner, protocol/fallback, broad differential gate or push scope.
