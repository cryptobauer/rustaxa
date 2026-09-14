# Shared setCommission semantic port

The reviewed draft and completion are integrated as `c3f53a0e2` and `02281c89d`.
One shared kernel owns the existing Rust `setCommission` rule order. The original
FinalChain method delegates through a complete-snapshot adapter; native sessions
invoke the same kernel through a private checkpoint-row adapter.

The checkpoint adapter authenticates prepared/current validator and owner bytes,
the request's validator, and the complete session snapshot's corresponding tuple.
It preserves full-width stake and untouched reward/undelegation fields when
encoding the replacement row. Only a successful opaque update can mutate the
authoritative session snapshot. Unavailable reads remain typed errors; malformed
or mismatched raw observations cannot silently become absence.

The existing identity-pinned reader remains responsible for concrete-state
identity and prior journal overlays. This seam does not authenticate arbitrary
caller-supplied rows in isolation and grants no database, complete-snapshot,
sparse-cache or publication authority. It routes only `setCommission`; claims,
reward scheduling and publication are separate work.

Independent Sol review compared both actual Go pins' rule order and validator
codecs, verified the existing fixture/exporter hashes, and approved the complete
draft plus completion. On the integrated branch, all 64 native-session tests,
three existing commission rule-order tests and the six-case dual-pin
driver/journal/FinalChain composition pass:

```sh
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-consensus final_chain::native_session
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-consensus apply_dpos_commission_update
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test native_session_reference
```

This is an unpublished semantic seam, not qualified existing-network adoption.
The incomplete inverse inventory and complete-snapshot publication boundary
remain unchanged; milestone 10 stays open.
