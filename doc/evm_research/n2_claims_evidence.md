# Bounded staged reward claims

The existing Rust delegator and commission kernels now serve staged
`claimRewards` and `claimCommissionRewards` sessions. The implementation is
`d620a5711` plus `12fa2f48f`, independently approved for unpublished semantic
sessions. It does not establish VM, publication or reopen parity for claims.

The actual Go exporter runs reward distribution before nonzero claims and zero
repeats, and records exact errors, logs, raw-write order, gas and balances at both
immutable pins. The manifest records exporter, instrumentation, support-source
and output hashes; both reference outputs are byte-identical. Rust comparisons
consume those outputs and apply staged account effects to a full-width model.

Delegator claims with zero reward do not read ordinary accounts. Commission
claims with zero reward retain `EnsureExists` followed by `Touch`. Nonzero
claims preserve the reference error order. Separate regressions exercise balances
above 256 bits, signed insufficient balances, typed source/recipient read errors,
and poisoned-session rejection without partial semantic effects. The shared
delegator kernel's zero-reward read correction also preserves the actual Go
cancellation path's conditional transfer behavior.

```sh
python3 -O experiments/evm_feasibility/native_claims_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-consensus claims
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-consensus final_chain::native_session
```

The worker passed seven focused claims tests and 1,410 consensus library tests;
independent review reproduced both actual-Go fixtures. The integrated branch
passed all 61 native-session tests. Package clippy passed with existing unrelated
warnings; strict warning denial is not claimed.

Zero-stake commission, claim-all, pre-Magnolia behavior, actual VM composition,
scheduler authority and claims publication/reopen remain outside this evidence.
These results advance N2 but do not close milestone 10.
