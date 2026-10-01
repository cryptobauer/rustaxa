# Metadata frame and historical API composition

Objective: compose the staged metadata kernel with the actual Rust EVM driver,
native journal and disposable historical simulation API. Base: `8945453`.
Production routing, publication, persisted historical inputs, RPC formatting,
trace parity and a new Go DryRunner oracle are outside this slice.

## Contract and evidence

The existing `native_validator_info` corpus comes from actual Go EVM calls at
public pin `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local pin
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. Both pins are executed again and
must reproduce the unchanged fixture bytes and manifest. Tests reconstruct the
exporter's exact synthetic wrapper, accounts and raw native rows. The selected
method rules are rebased to Rust genesis/pending period one; this does not assert
full network configuration or historical-state authority.

Eight cases cover arbitrary-byte replacement, empty and maximum strings, both
length errors, missing validator, STATICCALL mutation and parent REVERT. The
pending test uses the existing real `FinalChainNativeSession` port, actual driver
and journal. It compares parent status, gas, output and logs; native quote,
funding, caller, depth, input, error, output and disposition; ordered raw writes;
and final metadata. Parent REVERT keeps the irreversible metadata write and
removes settled logs. The native observation retains the successful event.

The historical test uses the existing test-only real simulation port and
`simulate_with_native`. Two independent probes per case compare the same Go EVM
execution observations, retain the reader identity and leave all raw rows,
accounts and code unchanged. A supplied nonce of 999 is preserved in the caller's
request while simulation applies its existing nonce policy. A subsequent fresh
pending session authenticates the unchanged metadata against committed semantic
state. This is API composition evidence; it does not establish separate Go
DryRunner metadata policy parity, persisted-reader coverage or trace parity.

## Validation and review

Artifacts: `/home/fry/artifacts/evm-branch-2026-10-01-2233/`.
The first targeted run passed simulation and failed an incorrect test assumption
that successful metadata calls emit no event. The corrected assertion compares
the actual Go event and its outer-frame disposition; no runtime behavior or
existing test was changed. Full logs and exit codes are retained.

Required checks: complete `native_session_reference` target (five tests), dual-pin
metadata fixture reproduction and `make rewrite-validate-fast`. Only test Rust
and documentation change; no runtime library, C++ or storage module changes.
The previous slice's ON bridge gate is retained as previous evidence, not
reported as a new gate for this slice.

All three required checks passed. Independent Astra medium review accepted the
frozen sources and evidence without findings. Requested and confirmed routes:
Sol medium implementation and Astra medium review. No routing failure. Allowance
was 27% after validation and review; this is not task billing.
N1–N6 and Milestone 10 remain open.
