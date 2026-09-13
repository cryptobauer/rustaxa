# S5 BN254 and BLAKE2F compatibility

`curve_precompiles.rs` adds immutable prepared stateless calls for exact addresses
6–9 using the already pinned REVM primitives. No dependency, production registry
or fork selection changes. Addresses 6–8 belong to Californicum; address 9 is
available only when the caller's full native classifier activates Ficus behavior.

| Primitive | Native gas | Input and failure behavior |
| --- | ---: | --- |
| BN254 add, 6 | 500 | Right-pad to 128 bytes; ignore trailing bytes |
| BN254 multiply, 7 | 40,000 | Right-pad to 96 bytes; ignore trailing bytes |
| BN254 pairing, 8 | 100,000 + 80,000 per complete pair | Require multiples of 192 bytes at execution; empty input returns true |
| BLAKE2F, 9 | Input round count; zero for wrong length | Require 213 bytes and final flag 0 or 1; preserve little-endian compression fields/output |

Preparation owns every call-context field and quotes without cryptographic work.
Invocation checks funding first. Funded malformed input completes with an exact
Go-shaped contract error and consumes the quote. The adapter translates only
reviewed primitive validation errors; unexpected backend failures abort execution.
BN254 coordinate errors distinguish equality with the field modulus from exceeding
it and preserve Go's validation order. Successful and failed calls produce no
account, raw-storage or log effects.

The opt-in frame driver now dispatches reviewed stateless addresses 1–9. Expected
curve failures pass through its native failure settlement, retaining unused gas
and reverting ordinary call-value effects. They never enter the consensus port
or produce a consensus observation. Other precompiles remain unavailable.

## Evidence

`curve_precompiles_reference.go` executes actual Go `RequiredGas` and `Run` at both
pinned revisions. The 40-row corpus covers infinity, valid arithmetic, scalars,
padding/trailing input, pairing true/false and invalid lengths, field violations,
in-field off-curve G2, invalid later pairs, and first-point error precedence.
BLAKE2F includes the known `abc` compression vector, bounded round counts,
non-final compression, wrong lengths and invalid flags.

The default Python verifier checks both reference outputs, exact fixture bytes
and the manifest/exporter hashes with explicit exceptions. It also reproduces
under `python -O`. Rust compares quotes, outputs and exact errors at below/exact/
above gas boundaries for every CALL-family kind, retaining full-width values and
all owned context fields. The driver additionally executes all 40 rows as funded
top-level calls and checks output/error, intrinsic plus quoted gas, value rollback
and absence of consensus facts. An omitted address-9 classifier proves that the
driver does not infer Ficus activation.

```sh
python3 -O experiments/evm_feasibility/curve_precompiles_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test curve_precompiles_reference --test native_driver_reference
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm --lib --test curve_precompiles_reference --test native_driver_reference --no-deps -- -D warnings
make rewrite-validate-fast
```

This is bounded primitive and frame evidence. It excludes BLS, P-256, Falcon,
full historical registry/fork coverage, resource-exhaustion workloads, persisted
native periods and production routing.
