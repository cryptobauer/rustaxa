# Signed current-source H1 estimation

This settled derivative uses the accepted signed H1 simulation history from
`e9fb32012`. The caller, canonical real prefix, full 119-row root, configuration,
ordinary accounts, graph nodes and public Rust finalization/reopen are unchanged.
See [simulation authority](n3_current_source_signed_h1_simulation.md).

The new producer executes actual DryRunner probes on both pinned Go revisions.
Each probe runs twice from fresh state. The unchanged upstream C++ gas-search
body consumes this transcript and requires every requested gas and probe count
to match. The 200000 cap uses six probes and returns 104977. Two actual boundary
requests establish success at 101912 and out-of-gas at 101911. The failed probe
uses 21912 gas, emits no logs and has empty return and consensus error.
No guessed error or synthetic measurement supplies these results.

Rust runs the same search over the actual transcript, with one fresh native
simulation session for every probe at the same H1. Eight estimate repetitions
across two owner lifetimes and two concrete reader lifetimes execute 64 probes.
Every probe compares admission, gas, error, output and exact logs, preserves the
supplied nonce, uses committed nonce plus one (2), and checks current and delayed
public state, ordinary accounts, descriptor/root and all physical rows before
the next probe. The source-current node is not advanced to H2.

The manifest binds unchanged upstream search, revisions, all producer support,
snapshot helpers/targets, signed simulation input, harness, output and stderr.
Prior corpora and runtime code remain unchanged. This fixture/test-only Tier 1
slice needs dual-pin reproduction, Python syntax, targeted and package Rust
tests/check/Clippy, serial fast, whitespace and independent frozen review before
local acceptance. Existing ON bridge evidence applies to unchanged runtime.
No production, RPC or concrete/synthetic Rust root equality is claimed.

Evidence directory:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`.
Files use the `signed-h1-estimate` prefix. Boundaries are limited to this actual
request and configuration. Broader semantic errors and nonzero reward profiles
remain open.
