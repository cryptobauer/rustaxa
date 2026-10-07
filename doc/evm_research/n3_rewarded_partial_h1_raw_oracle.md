# Same-H1 positive-reward raw-write oracle

This derivative adds actual ordered raw observations to the accepted
[canonical reward-only H1 oracle](n3_rewarded_partial_h1_oracle.md). It retains
its exact P1 identity, configuration, accounts, ten calls, repeated results and
complete committed isolation. Both pins and actual uninstrumented controls match;
removing only the two observer vectors also matches the accepted baseline.

The existing hash/shape guarded Account.SetStateRawIrreversibly observer copies
address, key and bytes before the unchanged mutation body. It adds no reads or
cache initialization. Per-call raw observations reset before both attempts;
repeat vectors match. Eight stdout/stderr files are explicit even when empty.
Manifest hashes bind observer helper, target, baseline producer/harness inputs,
supports, references and outputs. No mixed raw/account publication order is claimed.

Partial300 has12 entries over10 keys. Source NODE0 decrements3->2->1, caller
source700/cursor1, NODE1 index396000/count2, source stake1700/head1, pool0/commission8;
then destination NODE0 decrements3->2->1, caller destination1300/cursor1,
NODE1 index0/count2, destination stake2300/head1 and zero rewards. Both repeated
keys remain in order. Exact keys and values are measured fixture bytes; they
are not copied from the P2 corpus. Other nine cases have no raw writes.
FE sub396/caller add396 remains a separate ordered account vector. Complete P1
is unchanged after each call. This does not establish a Rust post-simulation
root, production admission or parent-frame outcomes.

Checks: actual record/reproduce, Python compile, serial fast, explicit16-path
whitespace/frozen hashes and independent Sol medium review. The run4 artifact
record retains first commands, full outputs/exits and routing.
