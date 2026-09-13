# S5 bounded native composition evidence

Status: implemented and tested for the six synthetic `setCommission(address,uint16)` fixtures. This is an isolated
test composition of the iterative EVM driver, journal lanes and the staged consensus-owned FinalChain native session.
It adds no production classifier, precompile registration, FinalChain publication path or protocol change.

## Composition boundary

`rustaxa-evm/tests/native_session_reference.rs` supplies two test-only adapters. `SessionPort` translates the S1
`NativeExecutionPort` request, exact quote and result types to the consensus-owned `FinalChainNativeSession` without
moving EVM types into `rustaxa-consensus`. `FixtureState` provides the fixture's three logical accounts, code and five
DPoS raw rows to the normal `ExecutionJournal`. The test then calls `execute_top_level_call_with_native`; it does not
invoke `FinalChain::call` or reproduce native business rules in the EVM crate.

Both native classifiers recognize only the DPoS address. The period sequence starts at zero for period one and is
shared with the driver. The single reached consensus-native child in every fixture consumes sequence zero, including
business failure and the historical depth rejection, so the next sequence is one. The transaction envelope remains
owned by the EVM driver: sender nonce and fee debit, CALL/STATICCALL opcode cost, EIP-150 forwarding, returned child gas
and top-level success or revert all use the ordinary execution path. The session owns only the quoted 20,000 native
action gas, raw serializer, staged DPoS state and native log.

The port passes journal-backed raw reads into `prepare` and `invoke`. On success the EVM native-result adapter validates
the invocation and gas result before appending the exact raw replacement and log. A parent REVERT removes the ordinary
log while preserving the raw write. Contract failures return a zero EVM word from the fixture's CALL and leave both raw
rows and logs unchanged. STATICCALL retains the pinned legacy mutation behavior for this historical profile.

## Six-case evidence

The test requires the `native_calls` arrays in `public.json` and `local.json` to be identical before running any row.
Those files were generated directly from Go references `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`, as recorded in `fixtures/manifest.json`. For every row the Rust composition
checks:

- exact total transaction gas, return bytes and top-level success/revert status;
- one exact 20,000-gas native quote and invocation, including transaction/sequence identity, Go depth one, CALL kind,
  static mode, caller/code/state addresses, zero value, 50,000 supplied gas and exact ABI;
- exact surviving logs, including rollback of the successful native child's log when its parent reverts;
- exact ordered raw mutation and complete final five-row logical DPoS raw map;
- all three final account nonces, balances, code bytes and code hashes;
- exact fixture account disk bytes decoded by the production physical codec and exact four-field commitment bytes
  reconstructed by that codec;
- zero refund, no ordinary storage write, no attempted contract address and one consumed native sequence.

The cases are successful CALL, successful native mutation followed by parent REVERT, wrong owner, commission overflow,
pre-fix nested rejection and STATICCALL mutation. The success, parent-revert and STATICCALL rows contain the same exact
validator replacement from commission 100 to 200. Only the first and last retain the native log.

| Case | Total gas | Returned word | Top-level result | Raw writes | Surviving logs |
| --- | ---: | --- | --- | ---: | ---: |
| `call-success` | 41,763 | `1` | success | 1 | 1 |
| `call-parent-revert` | 41,763 | `1` | revert | 1 | 0 |
| `call-wrong-owner` | 41,763 | `0` | success | 0 | 0 |
| `call-overflow` | 41,763 | `0` | success | 0 | 0 |
| `call-before-fix` | 41,763 | `0` | success | 0 | 0 |
| `staticcall-mutation` | 41,760 | `1` | success | 1 | 1 |

## Commitment evidence boundary

`reference_native.go` begins with a map of five logical DPoS rows and a nil storage root. After execution it builds a new
storage trie from the enumerated final map, substitutes that reconstructed root into the DPoS account, and builds a new
account trie from the three enumerated accounts. Consequently `storage_root`, each account `disk`/`leaf`, and `root` are
useful deterministic reconstruction facts. They are not an observation of Go's incremental persistence path, prior
physical trie nodes, RocksDB history, or a published FinalChain root.

The composition test treats that distinction explicitly. It checks the complete logical raw map, decodes every exact
account disk row and reconstructs every exact account commitment leaf. It also checks that the DPoS disk row contains
the fixture's reconstructed storage root. It does not present the fixture's combined root as incremental or physical
publication evidence. The S4 persisted-period corpus remains the evidence for incremental writer and FinalChain
publication behavior.

## Validation and remaining scope

Focused validation is:

```sh
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test native_session_reference -- --nocapture
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --test native_session_reference --no-deps -- -D warnings
python3 experiments/evm_feasibility/reference.py
```

The composition test passes one test containing all six rows. Strict no-dependency Clippy passes for the EVM target,
and the Python verifier reproduces byte-identical output from both pinned Go references. Strict Clippy with local
dependencies enabled reaches existing repository-wide `rustaxa-consensus` findings outside this slice; none is in the
session facade or composition test.

This closes the bounded six-fixture composition for the first native mutation. It does not enable a production native
route or cover a second native call in one transaction, the remaining DPoS and slashing methods, DPoS read/cache
lifetimes, CALLCODE or DELEGATECALL, native account creation/deletion, rewards and cleanup, historical mainnet rows,
RocksDB continuation, recovery or pruning. Those operations require their own Go evidence and ordered serializers.
