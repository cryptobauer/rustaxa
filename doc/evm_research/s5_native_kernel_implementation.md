# S5 bounded native kernel session

Status: implemented and tested for the first `setCommission(address,uint16)` slice. This module is an unpublished
consensus-domain component. It adds no production route, protocol change, FinalChain publication path, C++ dependency,
or dependency from consensus to `rustaxa-evm`.

## Implemented boundary

`final_chain/native_session.rs` adds `FinalChain::begin_native_session(pending_period, expected_parent)`. Construction
requires `expected_parent` to equal the selected finalized head and `pending_period` to be its checked successor. The
first slice rejects pre-Cornus periods, clones `dpos_snapshot_at_finalized_block(expected_parent)`, and advances the
existing reward-reference graph to the pending period. The clone remains private to the session.

The public request mirrors the immutable facts in the S1 EVM native-port contract without importing that crate:

- period-local transaction position and monotonically increasing call sequence;
- period, depth, CALL-family kind and effective static mode;
- caller, code address and state address;
- arbitrary-width unsigned value, exact calldata and supplied child gas.

The session supports `CALL` and `STATICCALL` when both code and state address are the DPoS precompile. It decodes through
`decode_dpos_transaction_for_execution` and admits only `SetCommission`. `CALLCODE`, `DELEGATECALL`, all DPoS reads,
other DPoS mutations, slashing, unknown selectors and malformed `setCommission` calldata return explicit adapter
errors. Before `FixRedelegateBlockNum`, nonzero Go-compatible EVM depth is a prepared terminal contract failure. It
consumes the method quote and session sequence without reading business rows or invoking the kernel.

`prepare` allows one outstanding quote. The retained request includes every input field, and `invoke` requires the exact
request and quote. A mismatch leaves the original preparation available. Every normal `Completed` or
`InsufficientGas` result consumes the quote and increments the sequence. Infrastructure, raw-integrity and kernel
errors poison the session; every later `prepare` or `invoke` returns `Aborted`.

## Gas, value and state ownership

The full `FinalChainNativeValue(BigUint)` is tested for zero before any EVM-word conversion. For post-Cornus
`setCommission`, a nonzero value prepares a zero-gas terminal result and invocation returns the exact `Method is not
payable` contract failure. A zero-value call quotes the existing kernel's 20,000 action gas. If supplied gas is below
that amount, invocation returns `InsufficientGas { required_gas: 20000 }`. These terminal paths execute before raw reads
and never call the DPoS kernel.

The pre-fix nested-depth rule follows the distinct Go `RequiredGas` and `Run` order. Preparation returns 20,000 for a
zero-value method (or zero for post-Cornus nonpayability), then child-gas admission occurs. An admitted invocation checks
nonzero depth before raw business reads and before the nonpayable failure. Its exact diagnostic is `only top-level calls
are allowed`. General Go `RequiredGas` lazy-cache initialization is not yet modeled because this operation's fixed gas
does not depend on that cache.

The native quote covers only native action gas. The session does not repeat transaction intrinsic gas, CALL opcode gas,
EIP-150 forwarding, fee debit or refund. It does not transfer CALL value or advance a transaction/creator nonce. Those
remain owned by the EVM envelope and frame driver. `setCommission` produces no ordinary account or refund mutation, so
the bounded consensus outcome contains only raw mutations and logs.

Logs remain ordinary frame-journal effects and may be rolled back. A successful validator raw write and the matching
semantic DPoS session advance survive ordinary frame rollback. A later call in the same pending period therefore reads
and validates the earlier raw replacement and executes against the already advanced semantic snapshot.

## Exact raw validation and serialization

Only sufficiently funded zero-value calls touch raw state. Preparation reads exactly two DPoS logical keys:

```text
validator = keccak256(0x0000 || validator_address)
owner     = keccak256(0x0003 || validator_address)
```

The validator lookup must be `Present`. Its RLP must consume every byte and match staged total stake, commission, last
commission-change period, reward-reference head and undelegation count. Before Magnolia, only the four-field legacy
list is accepted. At and after Magnolia, the nested extended list is accepted; a historical four-field row is also
accepted only when the staged undelegation count is zero. Counts that cannot be represented as `u16` are errors rather
than saturating. The owner lookup must be `Present` and contain exactly the staged 20-byte owner address; it is not RLP.

Invocation rereads both exact classified values (`Present`, `Absent` and `Tombstone` remain distinct) and refuses any
intervening change before kernel execution. It then clones staged DPoS state, invokes
`apply_dpos_mutation_transaction` once, and serializes the successful validator row before swapping the clone into the
session. Kernel or serializer errors cannot expose partial semantic or raw effects. Normal kernel business failures
consume 20,000 gas and return the exact legacy diagnostic with no raw write or log.

The successful serializer emits one nonempty put. It does not regenerate the DPoS map or write the owner row:

```text
ValidatorV1 = RLP([stake, commission, last_change, reward_head])
legacy      = ValidatorV1
extended    = RLP([ValidatorV1, undelegations_count])
```

For the public fixture's validator `0000000000000000000000000000000000000031`, the tests bind these exact facts:

```text
logical key  4ec8e987ae8c32d5e6a93d7f1a07e4068caa2697c4e26d8444fd89965bf0660a
old row      c6822710648080
new row      c9c782271081c8018080
action gas   20000
```

They also check the exact commission event address, topics and 32-byte data produced by the existing kernel, its
commission-overflow and wrong-owner diagnostics, STATICCALL admission, and a second successful call in period one after
the first raw write has been applied while its ordinary log is treated as rolled back.

## Validation

Focused validation from the repository root:

```sh
cargo test --manifest-path rust/Cargo.toml -p rustaxa-consensus native_session -- --nocapture
```

The five tests cover:

1. real-kernel execution against the exact public native fixture row and log, legacy-to-extended serialization,
   business failures, STATICCALL, and two calls in the same period;
2. a value wider than 256 bits and insufficient gas, both against an erroring reader that proves zero raw accesses;
3. changed request fields and a stale quote as retryable binding errors, followed by changed raw bytes that poison the
   session against reuse;
4. pre-fix nested-depth quoting and rejection before raw reads, including insufficient-gas ordering, precedence over a
   nonpayable value, and admission at the exact fix boundary;
5. pre-Magnolia legacy output and rejection of an extended input row before its encoding boundary.

Package-wide strict Clippy currently reports existing findings outside this slice. The new module's one initial
large-enum finding was removed by boxing the prepared operation payload. Focused tests and compilation pass with the
temporary parent-module export used only in this worktree.

## Remaining work

The application/test composition still must convert between these consensus types and `NativeExecutionPort`, map
current EVM journal read errors without erasing them, validate/apply the returned exact raw operation, and decide which
ordinary logs survive each frame. No production EVM driver dispatch is enabled by this slice.

Further DPoS mutations need operation-owned ordered serializers, including deletions and iterable-index changes, plus
ordinary account working sets for payable/custody operations. DPoS reads need their distinct current, gas-cache and
delayed-snapshot lifetimes. Slashing requires its own fixtures, delayed snapshots and exact serializers. FinalChain
reward cleanup, fee/minted rewards, full projection validation and publication remain outside this session.

The current adapter intentionally does not model nonexistent-validator `setCommission` as a normal business failure,
because the bounded integrity contract requires an existing exact validator and owner raw row. It also does not establish
new-account raw-lane lifecycle behavior, mainnet historical serializer coverage, RocksDB continuation, or a complete S5
period-boundary equivalence proof.
