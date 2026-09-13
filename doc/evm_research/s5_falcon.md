# S5 Cacti Falcon-512 compatibility

Status: bounded direct-kernel implementation and dual-pin oracle. This slice adds
no production registry, historical fork selection, frame route, state access,
protocol change, snapshot change, or publication path.

## Implemented contract

`rustaxa-evm/src/falcon.rs` owns one immutable stateless invocation at exact
address `0x000000000000000000000000000000000000fa1c`. Preparation binds the
complete call context and quotes 1,465 action gas plus 6 gas per ceiling-divided
32-byte word of the complete input. Invocation checks funding before ABI parsing
or cryptography.

The funded contract requires the four-byte `de8f50a1` selector for
`verify(bytes,bytes,bytes)`. Fewer than four bytes fail with `invalid input
format`; another selector fails with `invalid method signature`. Those are
contract failures with empty output. All later ABI and cryptographic rejection
is normal success with the 32-byte word one. A valid signature returns the
32-byte zero word, which is Falcon's inverse of the P-256 result convention.

The post-selector ABI has three 32-byte offset words for signature, verifying key,
and raw message. Like Go `big.Int.Uint64`, each offset and length uses only its
low 64 bits. Offsets need not be aligned, ordered, or canonical. Every offset and
length must be nonzero and identify an in-bounds field after Go's post-selector
`getData` call right-pads the view with four zero bytes. Go converts the outer
bounds expression to signed `int`; consequently a message length with bit 63 set
can bypass that check and consume the finite tail. The adapter preserves that
accepted finite behavior. If unsigned start-plus-length wraps and the Go
reference would panic while slicing, the adapter reports an explicit
infrastructure error rather than inventing a normal invalid result. The signature must be
exactly 666 bytes and the verifying key exactly 897 bytes. Empty messages are
rejected before cryptography even when the signature is mathematically valid;
trailing input is ignored.

Cryptography uses `fn-dsa-vrfy = "=0.3.0"`, `VerifyingKeyStandard`, no domain
context, and the raw-message hash identifier. The exact 0.3 dependency is a
consensus compatibility requirement: the existing comparison probe proves 0.3
matches all historical Go vectors while 0.4 rejects three historically valid
signatures. The helper emits no account, raw-storage, or log effects.

The surrounding application still owns Cacti activation and native-address
classification. The frame driver still owns CALL-family context, value transfer,
gas forwarding and return, and rollback. Integration must add the helper only to
the Cacti stateless route and must add the exact dependency plus its generated
lockfile entries.

## Reference evidence

`falcon_reference.go` obtains exact address `0xfa1c` from the actual Go Cacti
registry and calls its `RequiredGas` and `Run` methods directly. Its deterministic
keys and signatures use the same SHAKE256 seeds and historical Go FN-DSA library
as the earlier compatibility evidence. The Python harness archives and executes
both pinned Go revisions:

- public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`;
- local `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`.

Their 41-row artifacts are byte-identical with SHA-256
`4600f39d337401bad132b6fd37074de0f53ec8555f8ddaee25ea9d5e2969bc88`.
The exporter SHA-256 is
`5c2c326632ff1324f0d34a69dc861acc16942c28c4254e591bda129642e8aaf4`.
The corpus covers empty and short input, the wrong selector, truncated headers,
each zero/out-of-range offset, each zero/truncated length, wrong fixed field
lengths, historical valid short and 257-byte messages, invalid signature/message,
a historically valid empty message rejected by ABI, reordered and unaligned
fields, nonzero high offset and length bits, and accepted trailing bytes. It also
distinguishes a signed message reconstructed by Go's four-byte right-padding from
a field extending beyond that padding. It also proves that the signed `int`
length edge can verify from the finite tail and records the unsigned-wrap panic
as an explicit diagnostic fixture. Seven added cases pin the upfront all-offset
zero check and the distinction between declared fixed-field lengths and clamped
slice lengths, including cases where checking the next field would panic.

The Rust test compares every row one gas below, exactly at, and one gas above its
quote for CALL, CALLCODE, DELEGATECALL, and STATICCALL contexts. It validates the
quote/result identity, proves that underfunding precedes parsing and verification,
checks exact status/error/output/effect shape, and rejects address lookalikes.

## Validation

```sh
TARAXA_EVM_SOURCE=/workspaces/rustaxa-evm/submodules/taraxa-evm \
  python3 -O experiments/evm_feasibility/falcon_reference.py
cargo test --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test falcon_reference
cargo clippy --locked --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test falcon_reference --no-deps -- -D warnings
```

The exact dependency, crate export and Cacti frame route are now integrated.
Full top-level frame tests cover all 41 rows below/at/above the quote, value
credit/rollback and zero consensus-sequence consumption. Reference panics become
explicit infrastructure errors requiring the containing session to be discarded.
The resumed independent reviewer approved the field-order corrections and found
one additional allocation edge: a declared message length of `i64::MAX` passes
Go's wrapped bounds check, then panics in `make`. The 41st dual-pin witness
records `runtime error: makeslice: len out of range`. Rust reserves padding
fallibly before copying or resizing, returning an infrastructure error on
reservation failure. Independent closeout review approved this correction and
the scoped shared dispatch integration. The 41-row primitive suite, all 23
native-driver tests and `make rewrite-validate-fast` passed.
