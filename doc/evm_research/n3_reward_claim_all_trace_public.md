# Positive claim-all public API trace parity

The public Rust trace runner compares both actual Go default JSON cases in the
[trace oracle](n3_reward_claim_all_trace_oracle.md). Empty and ordinary zero
self-transfer prefixes retain one journal, native port and sequence over the
canonical P1. Public FinalChain finalizes P1 and reopens it independently of the
authenticated complete92-row Go reader. Native execution is pending2 over reader1.

Each target is the first native call: sequence0, direct non-static CALL, zero
value and required gas90000. Its transaction position equals prefix length;
supplied native gas is178728. Original transactions retain their512-bit nonces
through success and every failure. At native preparation the driver has advanced
the target caller nonce to2^512+1 (empty prefix) or2^512+2 (ordinary prefix),
while caller/native balances remain3000/4800. The ordinary prefix is non-native
and does not change the qualified native graph.

One outcome matches twelve actual ordered Go raw writes, two balance effects,
one396event and empty output. The public default JSON matches gas111272,
failed=false, empty returnValue/structLogs. The zero-reward second validator
still writes its checkpoint without a second payout or event. Vectors come from
fresh actual measurements and stay separate from default JSON fields.

The test creates336disposable ports across both cases and fresh/reopened owners.
Actual authentication has one preparation count read and thirteen invocation
reads: fourteen observations over thirteen unique keys, twelve present and two
absent current nodes. Every read gets read-error and corruption faults; present
reads also get a missing-row fault. Each fault and preparation failure has an
immediate clean retry. Every failure is exactly Target0, exposes no outcome and
has the expected read prefix. Changed repeated count bytes are rejected by the
mixed port's phase integrity guard before the session comparison. Every attempt
drops one port and preserves reader identity, full physical P1 and all public
owner rows, accounts, receipts and metadata. A fresh read-only RocksDB view
compares every owner column name and ordered logical row after each attempt,
including empty columns. The same row vector is checked across owner reopen;
this does not flush, commit or change the live owner.

Runtime code is unchanged from accepted546ac3eeb129. Its consensus1525 and
RUSTAXA_ENABLE=ON12-job/all15 bridge evidence is retained through the exact
`claim-all-public-closeout.json`. The derivative requires a nonzero focused
public test, full EVM, affected package check/Clippy, reproduction/Python,
serialfast/whitespace and independent frozen review. First compile failures
(field name and explicit journal-error conversion) are retained. The first
frozen review found that the old owner observation covered selected accounts,
stakes and header only; the complete row comparison closes that gap. A focused
pass is not acceptance; exact closeout status is in the current run directory.

This finite synthetic composition grants no historical/nested/repeated/batch/
owner/general claim-all, own later failure, production, real-window/root parity,
recovery, N4-N6 or cutover acceptance.
