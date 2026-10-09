# Literal H1 positive noarg claim-all oracle

The producer runs actual Go DryRunner at EVM and native block 1 over the accepted
canonical 92-row reward-only P1. Both pinned archives execute the exact four-byte
`0b83a727` input from the existing non-owner caller to DPoS. Each has a fresh
repeat. Full committed physical P1 remains unchanged after every call and repeat.
The Go DryRunner replaces the supplied 512-bit nonce with 1 in its local call;
the post-call caller account has nonce 2. These are separate observations.

The measured native quote is 90,000, supplied native gas is 178,728, and total
transaction gas is 111,272 with zero value and gas price. Return bytes and errors
are empty. The first validator pays 396 and emits one reward log; the second
pays zero and emits none. Twelve ordered raw writes retain both six-write scalar
checkpoints at literal block 1. The first six match the separately measured H1
single-claim fixture. The second block-1 node key is
`dccd1018a8d3ace84d0ec5bda96921351a6f1dc3974d1ae867905e02b03ec6dc`.
No pending-block-2 write vector substitutes for this measurement.

Both observed and control runs contain copy-only quote and post-call state taps.
The quote tap returns the actual RequiredGas result without changing its value
or scope. The public and local EVM source SHA guards differ because their pinned
sources differ. Unique call-site guards preserve each source's surrounding gas
logic. The post-call tap copies existing caller, owner and native accounts and
23 fixed watched raw keys. That union is the accepted pending2 union plus both
block-1 node keys; it does not depend on observed writes. No checkpoint, cache
clear or commit occurs while dirty state is read.

Native raw-write and balance observers occur only in observed runs. The control
comparison removes only those two observer vectors. It retains output, logs,
quote, nonce, all copied account fields, watched prior/after raw rows, complete
physical P1 and repetition assertions. The observed ordered writes fold exactly
into the copied after-state from the copied prior-state. Caller balance becomes
3396, native balance 4404, and owner balance stays 1000. Controls have native
observers disabled; they still have the two state-copy taps.

The harness records archive revisions, source/helper/needle hashes, actual Go
version, fixture digests and support provenance. First recording stopped at a
source guard because the public EVM uses an inline RequiredGas condition and the
local EVM has a separate assignment. The corrected copy tap wraps the same
unique RequiredGas expression in both shapes; the first failure is retained.
Required gates are actual record/reproduce, Python compilation, serial fast
validation, scoped whitespace and independent review of frozen files/evidence.

This is oracle preparation only. Rust H1 claim-all admission, general traversal,
estimation, tracing, frames, root parity, publication, real-window behavior,
production routing, recovery and N4–N6 remain open.
