# Staged Phalaenopsis escrow entry

Base: `1e49be922`. Objective: admit the exact active escrow-transfer selector
through pending/historical native sessions while preserving frame ownership of
value transfer. No production route or new C++/storage-module owner.

## Contract and implementation

Luna medium identified this as the smallest ready non-metadata session gap.
Independent Astra medium contract review settled the branch before runtime
implementation. Go accepts exact four-byte `44df8e70` only at/after Phalaenopsis,
quotes 1,000 native gas and returns success before the ABI nonpayability check.
Funding precedes the pre-fix nested-depth restriction. `RequiredGas` lazily
initializes native caches and may read global rows; those setup observations
remain separate from business reads in the oracle.

FinalChain's existing successful no-op now has a documented pure helper reused
by both full dispatch and staged invocation. Sessions use the existing exact
decoder, quote and admission policy, then return empty output/logs/account/raw
effects without consulting either reader or changing semantic state. There is
no custody/Magnolia gate. The EVM driver remains the only owner of transferred
value and ordinary frame rollback. Inactive, trailing-byte and unknown selectors
remain explicitly unsupported at this staged boundary; this slice does not add
their Go ABI lookup error presentation. Pre-Cornus session constructors and
delegate/call-code remain unsupported.

## Independent references and comparisons

The actual dual-pin Go EVM exporter has 12 cases: direct zero/value, nested value,
static zero, parent revert, insufficient native gas, mainnet Phalaenopsis neighbors,
trailing input, two explicitly synthetic pre-fix ordering cases and an inactive
post-Cornus case. Its wrapper forwards CALLVALUE, rather than the metadata
wrapper's zero. It records actual frame funding/depth/caller, quote, native and
parent status/output/gas, logs, setup/business reads, ordered writes and all three
account balances/nonces. Native raw rows remain unchanged. Shared source/harness
and output hashes bind the immutable public/local references.

Eight source-qualified active cases are compared through both real session kinds
with readers that panic on any raw/account access: 16 executions. Exact Rust
activation/trailing checks, quote mismatch retention, insufficient-gas sequence
consumption/continuation and a value above 256 bits cover the pure adapter bounds.
The existing exact-selector/activation and genesis escrow regressions also pass.
Pre-Cornus mainnet neighbors are Go evidence only; the post-Cornus inactive and
trailing staged results deliberately remain typed unsupported outcomes.

The actual Rust driver/journal compares eight frame cases including final account
balances/nonces, native quote/funding/caller/depth/value, revert disposition and
absence of native/ordinary storage writes. Parent revert restores transferred
balances. Sixteen repeated historical API probes match the same actual Go EVM
results and retain all reader accounts, code, raw rows and identity. This is
bounded synthetic composition, not a separate Go DryRunner escrow oracle or
network replay qualification.

## Validation and review

Artifacts: `/home/fry/artifacts/evm-branch-2026-10-01-2233/escrow-*`.
The first harness copied a stale 14-case count; it was corrected to its declared
12 cases. Initial Rust compilation exposed the receipt-log versus call-log
conversion and the consensus crate's lack of a `hex` dependency. Existing typed
conversion and a test-only parser corrected these without adding dependencies.
All failed/corrected logs and exit codes remain available.

Five targeted consensus tests and four focused frame/support tests passed.
Both pinned outputs matched and reproduced. The Rust-enabled consensus bridge
built with 12 jobs and all 15 bridge tests passed. Workspace fast gate passed.
Independent frozen Astra medium review accepted the sources/evidence without
findings. N1–N6 and Milestone 10 remain open.

Requested and confirmed routes: Sol medium implementation, Luna medium bounded
map and Astra medium contract review. Final review requested and confirmed Astra
medium. No routing failures. Allowance remained 25% after validation/review;
no task billing is inferred.
