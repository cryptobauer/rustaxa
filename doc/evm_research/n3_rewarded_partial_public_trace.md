# Public direct rewarded trace parity

This test composes the accepted [actual Go TraceRunner oracle](n3_rewarded_partial_trace_oracle.md)
with the existing public Rust run_structured_trace_with_native API. It uses
complete canonical reward-only P1 as the selected reader and semantic parent,
then unpublished begin_native_session(2,1) plus MixedNativeExecutionPort.
HistoricalSimulation1 is not a trace substitute. No production source changes
are made; the pending2 finite reward guard and staged account/codecs remain intact.

Public FinalChain reward finalization creates P1 and its owner is reopened.
An independent concrete reader materializes all92 actual Go rows and verifies
absent paths against the measured root. Full identity and execution2 are checked
in the factory. Empty prefix and one direct partial300 target preserve nonce2^512.
One native invocation is observed: period2/depth0/sequence0/transaction0/CALL,
caller/input/value matching the original transaction, supplied nativegas178088.
Every constructed port drops once. Terminal rewards/publication are never called.

Default structured JSON matches both Go pins: gas101912, failedfalse, empty
returnValue and structLogs. The captured native outcome separately matches all
12 ordered raw bytes, two current balances FE4800->4404/C3000->3396 and two logs.
The public JSON does not expose these effects. Native charge80000 and success
status are checked; raw/account vectors do not establish mixed publication order.

Across two public owner states,352 disposable sessions cover clean repetitions,
29 raw reader failures,29 corruptions,27 missing present rows and one prepare
infrastructure failure per owner. Every fault has a clean retry. Consumed keys
are measured from actual invocation and raw presence is read from authenticated
P1; all29 are distinct. Fault results identify target0 and return no partial
successful JSON. Failed invocations expose no successful native outcome. Read
prefixes stop at the selected fault. State-period mismatch and factory identity
qualification failure also return no JSON and create no port.

Each attempt preserves full concrete physical inventory/root identity and public
owner header/accounts/principal4000/two stakes2000. Original target nonce remains
unchanged. A final owner reopen also passes committed checks. This is synthetic
P1 composition; it does not establish a general older-state trace owner after
FinalChain advances, root algorithm parity, real-window execution, delayed query,
prefix/nested/OE/RPC trace support or production admission.

Required checks: targeted/fullEVM tests, check/Clippy, serial fast, explicit
tracked/untracked whitespace, frozen live hashes and independent Sol medium
review. Accepted6fe7b2c39 production bytes are unchanged; retain its full
consensus1523 and ON12-job/all15 bridge evidence rather than claim a rerun.
The run4 artifact records retain first commands/full outputs/exits and routing.

Review correction: each state-period mismatch and factory qualification failure now has its own fresh successful trace retry, including the reopened owner. These errors create no native port; 352 counts constructed sessions only.
