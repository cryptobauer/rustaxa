# Finite positive claim-all public parity

The noarg `0b83a727` path admits one direct zero-value CALL in an unpublished
pending period2 session over finalized P1. It is the first native invocation.
It requires post-FixClaimAll, Aspen1, Magnolia, Cornus and Ficus, pre-Aspen2,
and the canonical maximum stake1000000. Other configurations and call shapes
are outside this profile. No upstream C++, fallback or production route changes.

The [actual oracle](n2_reward_claim_all_oracle.md) uses both pinned Go producers,
fresh repeats, hook-free controls and the complete unchanged92-row P1. Its
ordered caller index has two validators; count is present LE32(2). Item keys use
the effective suffix2 from the pinned Go slice alias, not nominal suffix0.
Preparation reads only the count. Invocation rechecks that classified value,
binds the semantic pair, then authenticates item1 and item2 in order. It does
not eagerly read reverse indices, reward rows or balances for gas.

Qualification is structural and independent of address literals. Both validators
share a distinct owner; caller and owner each delegate1000 to each validator,
whose total stake is2000. Caller/global/owner index order agrees. Complete
principal and reward histories are required. Exactly two live genesis reward
nodes have zero indices/count3, heads and both cursors at0, and no current node.
An older genesis head is not Rust's exceptional legacy stale-head flag. The
canonical scalar validator-index mirror map is absent; delegation mirrors are
zero. First validator pools are792/8; second pools are zero. Other graph,
reward-role or index shapes are excluded.

Invocation uses the existing scalar claim kernel twice on one private evolving
semantic snapshot, full-width account port and ordered raw trace. Each step's
serializer sees its immediate before/after views. The first claim pays396 and
emits one event; the second pays zero and emits no account effect/event, while
still writing six checkpoint rows. The complete outcome has twelve ordered raw
writes, two balance replacements, one event, empty output and native gas90000.
Transaction gas111272 includes pinned intrinsic gas21272. Raw writes retain
intermediate expectations; they are not collapsed into a final map.

Count/order disagreement, missing/corrupt state, infrastructure failure or an
unexpected scalar result aborts the whole unpublished session. No outcome or
semantic state is exposed on error. This is an admission boundary, not parity
for Go's excluded later-inner-business-failure path, which retains an irreversible
raw prefix while ordinary effects revert. The old global claim-all failure
kernel is not used.

The public EVM test finalizes/reopens Rust P1 through public FinalChain APIs,
then uses an independent authenticated complete Go reader for execution. Its
168 disposable sessions span both fresh/reopened owners. Preparation has one
count read; invocation has thirteen raw reads including count, ordered items
and two scalar serializers. Fourteen total reads cover thirteen unique keys,
with twelve present observations and two absent current nodes. Each actual
read has a read-error/corruption fault and, when present, a missing-row fault;
each fault is followed immediately by a clean retry. A preparation failure also
has a clean retry. The mixed port catches a changed second count read before
the session's own integrity comparison; both paths abort without an outcome.
Every attempt drops one port, retains the original request, and preserves all
physical P1 and public Rust owner rows, accounts, receipts and metadata.

The focused consensus test uses renamed synthetic semantic addresses to prove
structural admission, exact request/quote retention, successful two-step effects,
and repeated/owner/duplicate/reordered/trailing/depth/static/value/low-gas/
historical/incomplete/missing/altered-reward/mirror rejection. Changed count and
item order fail closed. An injected scalar affordability failure leaves the
semantic snapshot and raw state unchanged, with no outcome. This unit fixture
is not measurement authority; the public physical fixture owns parity evidence.

Runtime changes require fresh full consensus/EVM and package check/Clippy,
RUSTAXA_ENABLE=ON bridge build with12jobs/all15 tests, serial fast checks,
whitespace and independent frozen review. Exact logs, first failures/corrections,
source contract and acceptance status are in the current run directory linked
by the [checkpoint](n6_restart_checkpoint.md). A targeted pass alone is not
acceptance.

Owner-caller, empty/one/large/reordered/general claim-all, repeated invocation,
historical claim-all, batch ABI, nested/static/reverted/value/low-gas/trailing
profiles and own later failure remain open. Synthetic root composition does
not prove root parity, real-window acceptance, recovery, N4-N6 or cutover.
