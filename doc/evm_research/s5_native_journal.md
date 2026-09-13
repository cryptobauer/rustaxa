# S5 native result application

Status: reviewed result-to-journal adapter for the existing S1 native port.
The adapter itself adds no native address registration and does not claim
business-kernel, native ABI or persisted native-period parity. Explicit opt-in
CALL/CREATE driver entry points now apply these results to native frames.

`rustaxa-evm/src/native.rs` prepares and invokes one operation through
`NativeExecutionPort`. Quote identity is checked before invocation, and the
returned funding/charged-gas facts are validated before journal effects. The
driver allocates period-bound consensus invocation identities; the port validates
their ordering and owns exact request/quote binding and staged kernel state.
The caller's frame owns its checkpoint, call-value transfer, nonce rules
and success/failure settlement.

The adapter applies ordered ordinary mutations through the existing journal,
then compares and applies every raw mutation in sequence, then appends logs.
It preserves absence, tombstones, empty bytes and leading-zero bytes exactly.
In particular a live raw delete reads back as `Present(empty)`; it is not
normalized to a persisted tombstone. A normal contract failure still applies
its returned effects before the caller rolls back the frame's ordinary lane.
Any port/result/journal integrity error instead aborts the entire pending period;
the potentially advanced journal and staged kernel session cannot be retried.

The returned `NativeFrameOutcome` retains the accepted required-gas quote,
remaining child gas, output and typed status. Insufficient gas retains all
supplied child gas. A completed native failure retains supplied gas minus the
quote and preserves its exact native error payload. Frame integration must use
those facts rather than burning gas through a generic bytecode-exception path.
No transaction fee or CALL base cost is charged here.

Six targeted tests use deliberately untrusted port results with the real journal.
They prove quote rejection before invocation, charged-gas rejection before
effects, sequential raw put/delete/put handling, existing-account raw survival
through failed-frame rollback, new-account raw removal on rollback, and retained
gas/quote facts. They do not stand in for the separate pinned native-kernel corpus.

```sh
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --test native_adapter
cargo clippy --locked --manifest-path rust/Cargo.toml -p rustaxa-evm --lib --test native_adapter --no-deps -- -D warnings
make rewrite-validate-fast
```

The opt-in driver checks that the consensus-native classifier is a subset of the
full native registry. Classified stateless addresses remain explicitly unavailable
in this route. Consensus sequence advances after generic depth/funds admission,
including quote underfunding and native business failure, and survives enclosing
ordinary rollback. Native failures retain returndata and unused gas but do not
copy bytes into the CALL output region. Nine targeted driver tests cover these
rules, full-width DELEGATECALL context and native calls during CREATE initcode.

The [staged setCommission kernel](s5_native_kernel_implementation.md) and
[ordered concrete overlay](s4_ordered_overlay_evidence.md) are separate components;
complete native-period composition and remaining native methods remain open.
Production routing and protocol changes remain unauthorized.
