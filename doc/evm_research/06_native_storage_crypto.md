# Native, physical storage and crypto feasibility

This is the final experimental continuation of [the creation checkpoint](05_creation_frames.md).
The [direction decision](07_direction_decision.md) interprets all three source reports and the
executable evidence. Experiments remain under `experiments/evm_feasibility`, on
`feat/rust/evm-state-db`; there is no production routing or protocol change.

## Actual Rust native kernel during EVM execution

The Go exporter now executes six `setCommission(address,uint16)` cases through real
EVM CALL/STATICCALL instructions and the actual DPoS contract. Both pinned Go revisions
produce identical output. Initial state includes a legacy four-field validator row,
owner and iterable membership rows, the sender and parent contract, and a funded DPoS
account. Recorded inputs include bytecode, ABI and every initial raw key/value. The
complete setup and fork configuration are in
[`reference_native.go`](../../experiments/evm_feasibility/reference_native.go).

The Rust comparison executes the parent bytecode using REVM. On a yielded native call,
it uses existing `decode_dpos_transaction_for_execution` and
`FinalChain::apply_dpos_mutation_transaction`. These are the actual existing Rust kernel,
not a reimplementation of commission rules. A disposable copy of the production Rust
workspace receives a test-only child module so the private kernel can be exercised
without changing production visibility, dependencies or ownership. The overlay has its
own [lockfile](../../experiments/evm_feasibility/native.Cargo.lock).

| Reference case | Measured result |
| --- | --- |
| Successful CALL | Kernel changes commission; exact extended validator bytes, ordered write, log, gas and roots agree |
| Successful native CALL followed by parent REVERT | Native raw mutation survives; emitted log disappears; gas, parent error and roots agree |
| Wrong owner | Kernel rejects mutation; no write/log; caller's result and charged gas agree |
| Commission above 10,000 | Kernel rejects mutation; no write/log; caller's result and charged gas agree |
| Before nested-call fix | `RequiredGas` quotes and EVM charges 20,000; `Run` rejects nested depth before raw business reads or the kernel |
| STATICCALL mutation | Reference accepts the native mutation despite static execution; Rust test preserves that historical behavior |

The serializer writes only the touched validator row. It encodes the mutated existing
kernel snapshot as `[ValidatorV1, pending_count]`, with zero pending count for this input,
and retains all other original raw bytes. It does not choose a projection candidate or
rebuild the database from normalized snapshots. The comparison checks ordered writes,
raw values, logs, gas, return/error fields, storage root, account commitment leaves and
combined account-state root against both references. Independent Rust RLP/trie hashing
supplies the commitments. The combined root is for the complete bounded post-EVM fixture;
it does not include period rewards or operational persistence.

**Conclusion:** the native handoff and exact mutation boundary are implementable while
reusing Rust FinalChain kernels. The two historically unusual rollback/static behaviors
are concrete requirements for the compatibility profile. Enforcing ordinary Ethereum
write protection or rolling every native write back with its parent would diverge.

**Limits:** one native business method, fixture-sized balances, one yielded native child,
and selected forks. The adapter's fixed action charge and hand-written result insertion
are specific to this corpus. It is not a generic CALL executor, complete native journal,
slashing/reward implementation, historical kernel audit or production API design. The
known pre-Magnolia pending-count correction in existing Rust still needs an explicit
historical compatibility path; this experiment does not erase that exception.

## Physical nodes, deletion and independent commitments

[`reference_nodes.go`](../../experiments/evm_feasibility/reference_nodes.go) performs eight
insert/update/delete/reinsert stages through the Go trie writer. Each stage starts a new
writer from the preceding root and persisted in-memory node/value bytes. It captures
accumulated nodes, values and tombstones, including an empty-root transition and restart
from empty. Thus subsequent operations must read the physical representation instead of
retaining the preceding writer's in-memory tree.

The independent [Rust reader](../../experiments/evm_feasibility/src/nodes.rs) decodes
Taraxa's physical branch/short nodes, resolves out-of-line values and leaf hash hints,
normalizes five-field physical accounts to four-field commitment accounts, and rebuilds
hash encodings. It checks referenced node hashes and obtains exact raw leaf maps. The
separate triehash oracle then reconstructs the root from those leaves.

Both references pass all eight history stages and the 14 original account/slot commitment
cases, including arbitrary-width account nonces and embedded/hash child boundaries.
Corruption tests reject missing root nodes, missing external values, malformed root bytes
and referenced tombstones. The reader has bounded depth and key width and rejects inputs
outside its test contract.

**Conclusion:** Taraxa physical representation is independently decodable in Rust; its
commitments are reproducible without an Ethereum account model. Keep the existing format
and implement its codecs directly around existing Rust domain types.

**Limits:** the update writer is Go; the Rust side is a bounded reader and full-root oracle.
No Rust incremental writer, versioned RocksDB traversal, pruning, import, code-store
verification, I/O error recovery or power-loss test is implied. The production writer must
pass those acceptance tests. `triehash` is an oracle here, not the selected production
incremental database implementation.

## Mixed gas and opcode customization

The [profile probe](../../experiments/evm_feasibility/src/profile.rs) matches six single-frame
Go opcode fixtures on result, gas, refund and ordinary/transient storage. It starts from
Istanbul's instruction machinery, sets Taraxa's tested SSTORE costs/refunds, and adds
PUSH0 and legacy transient aliases, enabling standard transient opcodes only for Cacti.
Small wrappers temporarily select the instruction's required REVM feature level and
restore it after the instruction. No REVM source patch is needed.

This establishes that the mixed profile is expressible through instruction/gas tables
and narrow overrides. It does not designate Istanbul or any other Ethereum `SpecId` as
Taraxa's protocol. The nested transient-revert Go fixture is not executed by this probe;
full original/current/new SSTORE cases, low-gas sentry order, stipend handling, other calls,
SELFDESTRUCT and all fork combinations remain implementation acceptance work.

## Historical Falcon dependency decision

[`reference_crypto.go`](../../experiments/evm_feasibility/reference_crypto.go) deterministically
generates three synthetic FN-DSA-512 key/signature pairs with the pinned historical Go
`go-fn-dsa v0.2.0`, using empty, short and 257-byte messages. Each gets five variants:
valid, modified message, modified signature, truncated signature and truncated key.
Signing keys are transient and never recorded. The 15 rows include raw verification and
actual Cacti Falcon precompile ABI output/gas from both Go references.

The [Rust comparison](../../experiments/evm_feasibility/src/crypto.rs) compiles both published
verifiers with separate exact version pins:

| Candidate | Mismatches against the 15 historical raw verification results |
| --- | --- |
| `fn-dsa-vrfy = 0.3.0` | 0 |
| `fn-dsa-vrfy = 0.4.0` | 3: rejects each historical valid signature |

The Go ABI separately rejects an empty message even when the raw signature is valid.
Its 32-byte result uses zero for valid and one for invalid; the probe also checks this
wrapper behavior and the observed intrinsic/precompile gas formula. These ABI checks
are independent assertions over recorded inputs, not a complete Rust ABI implementation.

**Conclusion:** algorithm name and newest version are insufficient. Use exactly pinned
0.3.0 as the historical-compatible implementation candidate and freeze these vectors as
an upgrade guard. Do not route 0.4.0 into historical execution. The measured mismatch is
the decision evidence; this experiment does not establish a security assessment or full
standardization history for either version. Full malformed ABI/point cases, FN-DSA-1024,
P256 and BLS address/gas/encoding compatibility remain precompile implementation gates.
Historical wire semantics may require a maintained compatibility dependency even when a
newer cryptographic standard is available.

## Historical data availability

The bounded [availability probe](../../experiments/evm_feasibility/network_probe.py) attempted
one request per documented mainnet/testnet RPC and snapshot endpoint, with 15-second
per-request timeouts and response limits. The checked-in
[observation](../../experiments/evm_feasibility/fixtures/network_availability.json), captured
2026-09-12, records RPC DNS failures and snapshot timeouts. No network replay input or
paired database checkpoint was acquired. These are failures from this environment,
not evidence of global service availability or chain status.

The endpoint selection follows official [connection documentation](https://taraxa.gitbook.io/taraxa-network/develop/connect-to-taraxas-network)
and [snapshot documentation](https://taraxa.gitbook.io/taraxa-network/node-setup/syncing-from-snapshot).
The latter advertises light snapshots and says full snapshots were discontinued; an
available light snapshot would still not establish complete historical replay coverage.
The RPC probe requests `taraxa_getVersion`, following the official
[RPC specification](https://taraxa.gitbook.io/taraxa-network/develop/taraxa-rpc-specs), alongside
chain identity and block observations. A successful RPC response alone would not prove
source-build provenance or provide all historical raw native state.

## Validation and reproducibility

From the repository root:

```sh
python3 experiments/evm_feasibility/reference.py
cargo test --locked --manifest-path experiments/evm_feasibility/Cargo.toml
cargo clippy --locked --manifest-path experiments/evm_feasibility/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path experiments/evm_feasibility/Cargo.toml --check
rustfmt --edition 2024 --check experiments/evm_feasibility/src/native.rs
python3 experiments/evm_feasibility/native.py
python3 experiments/evm_feasibility/native.py --clippy
make rewrite-validate-fast
```

The fixture manifest hashes all five Go exporters and both byte-identical reference
artifacts. Its additional-input descriptions identify each corpus's independent setup.
The separate native overlay lockfile pins its test graph; production source is the
repository checkout, so reproduction must use the research commit being evaluated.
Rust toolchain is 1.98.1; Go is 1.24.4. Network/cache access is needed for dependencies
not already installed. Availability observations are intentionally time-dependent;
`network_probe.py` refreshes that separate artifact and is not a deterministic parity gate.

The isolated suite has 12 passing tests; the native overlay has one passing test covering
six cases against both references. The repository fast gate, strict isolated Clippy and formatting pass. Overlay Clippy
passes with 86 warnings in unchanged production tests and none in the new probe, whose
module denies warnings. An initial overlay-wide `-D warnings` run failed on those existing
findings; no production tests or lint baselines were modified. No production storage module or
C++ code changed. Expensive full-node/differential, storage bridge and power-loss gates
were not run for these unlinked prototypes; their absence limits deployment evidence.
