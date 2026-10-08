# Direct positive-reward TraceRunner oracle

This experiment measures one direct partial300 target with an empty prefix over
the complete accepted reward-only P1. TraceRunner selects committedP1, executes
and registers native period2, and preserves the supplied nonce2^512. H1
DryRunner simulation uses a different period/nonce policy. No historical wrapper
advance is allowed. Source-only authority is reward-trace-contract.md in run4.

The canonical P1 root/92 rows/configuration/account/semantic/raw snapshot equals
the accepted [H1 oracle](n3_rewarded_partial_h1_oracle.md). No ordinary or principal
P1 transaction, prefix mutation or nonce overlay is added. Before/after snapshots
match after each first and repeated trace; default JSON and witnesses repeat.
Both pinned public/local executions match, including actual uninstrumented
controls after removing only the observation_witness field.

Actual default structured JSON is one row: gas101912, failedfalse, empty
returnValue and structLogs. Direct precompile execution has no opcode rows.
JSON contains no payout, native event logs or raw operations. Separate copy-only
archive witnesses record FE sub396 to4404, caller add396 to3396,12 ordered raw
writes over10 keys, and RewardsClaimed396 then Redelegated300. These observations
use the same actual TraceRunner call without adding witness calls or a prefix.
Raw/account/log vectors are separate; mixed publication order is unclaimed.

Source NODE0 decrements3->2->1; caller source700/cursor2; currentNODE2 index396000/
count2; source stake1700/head2 and pool0/commission8. Destination NODE0 also
3->2->1, caller destination1300/cursor2, NODE2 index0/count2, stake2300/head2,
zero rewards. Period2 keys/bytes are actual new measurements, not transplanted
H1 currentNODE1 values. No deletion occurs in this finite partial.

Existing account/raw observer targets are hash/shape guarded and unchanged
mutation bodies run once. A new guarded TraceRunner insertion observes ret.Logs
immediately after actual EVM.Main and before existing JSON formatting. Its
callback copies existing fields to strings and reads no account/raw/cache.
The control archives contain the same helper declarations with all hooks absent.
They execute real TraceRunner and compare full public JSON, diagnostics, exact
request/configuration and complete snapshots. Manifest binds both TraceRunner
source pins, helpers/guards, Go version, producer/harness/support/baseline inputs,
all outputs and eight explicit stdout/stderr files, including empty diagnostics.

Required checks: actual record/reproduce, Python compile, serial fast, explicit
tracked/untracked whitespace, frozen live hashes and fresh independent Sol
medium review. Run4 retains first commands/full outputs/exits and routing.
Public Rust trace composition needs separate validation of unpublished
begin_native_session(2,1), exact identity, one invocation/drop, payout/raw/log
witnesses, failure disposal and fresh retries. No Rust trace, production route,
root algorithm, real-window, delayed query, nested target, OE or RPC acceptance
is claimed by this oracle.
