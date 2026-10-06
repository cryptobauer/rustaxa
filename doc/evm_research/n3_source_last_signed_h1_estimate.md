# Signed source-last H1 estimation

This derivative uses the exact new signed source-last H1 simulation seed and
public Rust history. Caller, order, signed prefix, configuration, 119-row root
`4edf6134...`, committed accounts and current source node are unchanged. See
[simulation authority](n3_source_last_signed_h1_simulation.md). No runtime or
previous corpus/support changes occur.

Actual Go probes run twice at the same H1 on both pins. The unchanged upstream
C++ gas-search body consumes the complete transcript and exact requested gas
sequence. The normal200000 cap uses6 probes and returns104977. Boundary caps are
derived from this new simulation's measured used gas:101912 succeeds;101911
returns out-of-gas with21912 used gas, no logs/return/consensus error. These are
new actual observations, not copied source-first output.

Rust runs8 complete case suites across2 owner and2 concrete reader lifetimes,
with2 repetitions each. The3 search requests use64 fresh native simulation
sessions, one per probe at identity.period1. Every probe compares gas, admission,
error, return and exact logs; checks effective nonce2 while preserving the
original supplied nonce; then verifies all committed public account/current
native/delayed stake facts and every physical row before the next probe. Reopen
uses public history without refinalization. No graph advance toH2 occurs.

Manifest binds unchanged C++ search/support, exact signed simulation input,
both revisions, producer support/snapshot targets/helpers, harness, output and
stderr. Target/package/check/Clippy, Python syntax, actual reproduction, serial
fast, whitespace and independent frozen review must pass before local acceptance.
Tier1 fixture/test scope inherits accepted unchanged source-last runtime ON
bridge12/all15 evidence. No RPC, traces, production routing, synthetic/concrete
root equality, broader orders or reward/error variants follow from this evidence.

Run artifacts:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`,
prefix `source-last-signed-h1-estimate-`.
