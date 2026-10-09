# Positive claim-all public trace oracle

The producer executes the public pinned Go TraceRunner twice per case over the
complete canonical reward-only P1. Both source pins agree. Cases have either
an empty prefix or one ordinary zero-value self-transfer from the existing
non-native caller, followed by one direct noarg `0b83a727` target. Execution
period is2; reader period is1. Supplied nonces remain2^512 and, after the prefix,
2^512+1. No canonical seed is changed or committed.

The ordinary prefix is measured from its actual `EVM.Main` result inside the
TraceRunner prefix loop. A guarded copy-only tap records the returned gas,
errors, output/log count and account facts after that call. This tap runs in
both observed and native-hook-free controls. It does not replace the call or
change its inputs, result or state. Its original source hash, insertion bytes
and helper hash are in the manifest. Prefix gas is21000, all errors/output are
empty, no logs are emitted, caller balance stays3000, native balance4800, and
caller nonce becomes2^512+1. The prefix does not invoke native code.

The target's actual default JSON has gas111272, failed=false, empty returnValue
and structLogs. Separate whole-trace copy-only observers measure twelve ordered
native raw writes, balance replacements4800->4404 and3000->3396, and one396
reward event. These freshly executed vectors equal the accepted direct oracle;
that equality is a check, not a replacement measurement. The second indexed
validator writes its checkpoint without a payout/event. Default JSON does not
export these native vectors; they remain separately named observations.

Native raw/account/log observer calls are omitted in controls. After stripping
only `observation_witness`, observed and control outputs agree, including the
copied prefix result, default target output and complete P1 before/after state.
Each producer execution includes a fresh repeated trace and complete committed
P1 comparison. Both pins and all empty diagnostic artifacts are retained in
`fixtures/native_reward_claim_all_trace`; the manifest binds producer/harness,
support and observer source hashes, source pins and baseline dependencies.

This is a same-profile trace derivative of the accepted finite non-owner,
first-native, direct pendingP2 claim-all path. It does not measure a preceding
native claim, repeated claim-all, historical period1, nested/static/revert,
owner/batch/general topology, later own failure, production or root parity.
Record/reproduction/Python/serialfast/whitespace and independent frozen review
are required before an oracle preparation commit.
