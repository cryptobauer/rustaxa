# Engine and library decision assessment

## Recommendation

Prefer REVM components with Taraxa-owned transaction and state policy, but do not assume its default account,
transaction, or frame implementations can be reused. The strongest remaining design choice is between a custom
REVM framework integration and a lower-level REVM interpreter with a Taraxa host/frame executor. Both reuse the
maintained Rust interpreter; they differ in how much Ethereum transaction/state machinery they adopt.

The first feasibility work should compare these two integration depths against the requirements in
[the compatibility specification](01_compatibility.md). A full Reth adoption expands the migration unnecessarily.
evmone remains a useful independent candidate, but its current CREATE implementation also assumes bounded nonces;
it is not an automatic escape from REVM's integer constraints.

This is a source-based decision assessment. No candidate has been compiled, benchmarked, or shown to reproduce a
Taraxa state root. Evidence establishes where customization is required; runtime correctness remains unresolved.

## Inspected revisions and dependency compatibility

The [library evidence manifest](library_sources.json) records immutable source URLs, content hashes, and resolved
revisions. The selected versions establish a reproducible comparison, not a final production dependency set.

| Component | Inspected identity | Established constraint |
| --- | --- | --- |
| REVM | Git tag `v117`, commit `6014612c86f3690e4e9173a8c4deade396af398d` | Release title says v43.0.1; workspace declares umbrella `revm` 43.0.0 with some component crates 43.0.1; Rust requirement 1.91.0 |
| evmone | v0.23.0, `2184b24461ae20398520f0bc5023cb7dcc0e003c` | C++ interpreter; Ethereum revision tables and CREATE nonce handling remain engine concerns |
| Reth | v2.5.2, `5a6940e351fed80458fe6c9da8581cbe4b8bd036` | Declares REVM 42.0.1, alloy-evm 0.38.0, Rust 1.95 |
| alloy-evm | v0.38.0, `aa16b156595d7de8d25c31fc0c346e608fa7eb9c` | Declares REVM 42.0.1 and Rust 1.94.1 |
| alloy-trie | Published crate 0.9.5, VCS `9c5a16b21c6d46f2f6c5bfc619969c1aa0813354` | Rust 1.85; account convenience type has a u64 nonce; raw-leaf builder accepts bytes |
| Rust FN-DSA | Source `0629bb118ebf077857e78c1bd067c704a37795ed`; published verifier 0.4.0 is a separate artifact | Source explicitly allows breaking encoding changes before 1.0; July 2026 changes need comparison to Taraxa's Go 0.2.0 |

Release labels, Cargo package versions, and individual component versions are not interchangeable. In particular,
the inspected Reth/alloy-evm pair does not declare the inspected REVM 43 major. Do not combine the newest release of
each package without selecting a compatible dependency graph and validating it later.[^1][^2][^3]

GitHub's latest-release endpoint for alloy-trie returned 0.9.0, while docs.rs exposed published 0.9.5. Resolving the
`v0.9.5` tag in the original repository failed; the published crate was therefore inspected directly, including its
VCS metadata and archive checksum. This bounds the source identity without equating stale GitHub release metadata
with the newest published package.[^4]

## REVM customization map

Classifications: **extension** means a relevant public interface exists; **replacement** means a Taraxa-specific
implementation must bypass Ethereum defaults; **unresolved** means the interfaces alone do not establish feasibility.

| Requirement | Source-level integration point | Assessment |
| --- | --- | --- |
| Legacy nonce skipping, failure charging, system sender | Handler validation, pre-execution, post-execution | Replacement: apply Taraxa envelope and error/fee rules in their original order |
| U256 transaction nonce | `Transaction::nonce() -> u64` | Unresolved in the full framework: a custom transaction type alone cannot widen the trait |
| Arbitrary account nonce | `AccountInfo.nonce`, journal account `nonce`, setters and bump | Unresolved in the full framework: concrete types and u64 methods remain even with a custom journal |
| CREATE address and overflow | Frame reads old nonce, calls `bump_nonce`, computes address | Replacement: custom creation frames/host logic, not only altered admission |
| U256 gas-price domain | `Transaction::gas_price() -> u128`; interpreter host returns U256 effective price | Preserve wide price in a Taraxa context; prove any narrower adapter bound or replace its consumers |
| Opcode aliases/static gas | InstructionProvider/custom instruction and static gas tables | Extension available; requires a Taraxa instruction profile |
| Mixed dynamic gas and fork semantics | SSTORE/CALL/SELFDESTRUCT instruction functions and runtime flags | Replacement of selected operations or gas callbacks; static gas overrides alone are insufficient |
| Stateful native precompiles | `PrecompileProvider::run` receives mutable context and call inputs | Extension available; existing Rust kernels must be integrated with the correct state view |
| Native failure gas/errors | Provider output conversion and frame return/revert behavior | Replacement where Ethereum halt conversion consumes gas differently |
| Irreversible native writes and transient quirks | Journal checkpoint/revert and host state methods | Custom state policy; standard journal behavior is not parity |
| Historical account/code/storage | Database interface plus Rust state reader | Extension available; database interface does not supply history, commitment, or pruning |
| Tracing | Inspectors and frame/step events | Extension available; Taraxa trace formatting/context remains adapter work |

These boundaries are established by the pinned REVM interfaces and implementations.[^5][^6][^7][^8] They support a
custom engine design, but not the claim that ordinary REVM execution with a few configuration flags is sufficient.

### Integer constraints are deeper than transaction validation

The frame's creation path reads the caller's u64 nonce, increments it with a bounded journal operation, and derives
the new address before initializing the created account. The journal's account interface itself exposes u64 nonce
methods, and account emptiness also depends on that concrete field. Disabling nonce equality checks leaves these
other behaviors unchanged.[^5][^6]

A side map is possible only as a carefully specified representation strategy. Every path affecting collision checks,
creation address, nonce mutation/rollback, code hash/existence, serialization, and public reads must consult the true
nonce. Substituting a fake bounded nonce and hoping the engine never observes it is not acceptable. Prefer a design
where the custom host/frame owns the real nonce directly over pervasive corrections after execution.

Gas price adds another width obligation: Taraxa's legacy transaction decoder represents it as U256; REVM's transaction
trait returns u128. The interpreter Host's effective-price method returns U256 and GASPRICE reads that method, offering
a lower-level route to preserve the value. Admission/affordability constraints may restrict which very large prices
can execute, but this must be proved for every envelope, including simulation and system calls; it cannot justify
unconditional truncation.[^5][^9]

### Compare two REVM integration depths

| Design | Ownership | Benefit | Risk and acceptance criterion |
| --- | --- | --- | --- |
| A. Custom framework | Replace transaction policy, creation/call frames and journal as needed; reuse framework sequencing/inspectors | Reuses more lifecycle infrastructure | Accept only if true Taraxa values never leak into bounded defaults and all replacement points are explicit |
| B. Interpreter plus Taraxa host | Rustaxa owns envelope, frame stack, journals and native-contract dispatch; REVM supplies bytecode interpreter and selected operations | Clearer authority over unusual nonces, raw state and rollback | More call-frame/gas/error logic to maintain; accept only with bounded local code and strong parity corpus |

The interpreter host API does not require the caller to use the framework's complete Ethereum transaction model.
Custom instruction/gas tables are directly supported. These facts make Design B a credible option, not a proven
implementation. Trace both candidate designs through nested CREATE/CALL and transaction finalization before choosing;
the smallest import graph is not necessarily the smallest long-term maintenance burden.[^7][^9]

Do not patch every upstream crate preemptively. First enumerate the exact replacement surface, which upstream
implementations remain reused, and which behaviors need local code. Record required changes by function and invariant.
If the integration becomes a broad second interpreter, the maintenance case for REVM must be reassessed.

## evmone and Reth

evmone v0.23.0 performs an internal CREATE nonce read through the host, checks `MAX_NONCE`, and computes the creation
address before the host call. Its storage costs and opcode availability are keyed to Ethereum revisions. A Rust host
would still need engine-side customization for Taraxa's nonce and mixed-gas profile. Moving state ownership out of
the interpreter is useful, but it does not remove all protocol assumptions.[^10][^11]

Compared with REVM, evmone would retain a C++ execution dependency and a foreign-function boundary. It can still be
valuable as an independent reference for shared Ethereum operations or an alternate architecture if a bounded patch
set proves simpler. That determination requires evaluating the exact release and ABI, not relying on descriptions of
older EVMC host responsibilities. No speed comparison has been performed.

Reth and alloy-evm are useful sources of implementation patterns and selected components. They do not solve the
Taraxa-specific field-width and state-journal requirements of their underlying engine. Adopting an Ethereum block
executor, transaction pool, provider schema, or Engine API would also duplicate or displace responsibilities already
owned by Rust consensus/FinalChain. Defer those broader integrations unless a concrete requirement justifies them.
Retain the current ordered execution port and bounded query interfaces.[^2][^3]

## State and supporting libraries

Keep RocksDB for the initial state rewrite and reuse `rustaxa-storage` infrastructure. This isolates the execution
transition from a simultaneous database-engine change. Root computation, durable history, and physical storage are
separate components; neither REVM's in-memory database nor an MPT hash builder is a production state database.

Alloy-trie's raw-leaf builder can receive Taraxa's exact encoded account/slot leaves. Its `TrieAccount` convenience
type cannot represent the full nonce domain. Root-builder acceptance additionally requires matching key hashing,
node encoding, deletion/empty roots, and proof semantics. Its ordered-leaf interface does not supply a complete
incremental historical trie writer; database cursors and retained subtrees need their own design.[^4]

The existing Rust `triehash` dependency is useful as a small comparison calculator, subject to the same codec
requirements. Do not replace domain `FinalChainNonce`, existing transaction RLP, or account codecs with Alloy types
merely to eliminate conversions. Keep conversions at engine boundaries and preserve canonical bytes.

For stateless crypto precompiles, prefer existing maintained implementations but retain Taraxa's address registry,
gas formulas and input/output adapters. Shared cryptographic mathematics does not establish equivalent malformed-input
acceptance, subgroup checks, infinity representation, field padding, signature rules, or return conventions.
Use the existing Taraxa precompile fixtures and add boundary cases before choosing exact implementations.

### Falcon/FN-DSA requires a historical verifier decision

The same author maintains Go and Rust FN-DSA implementations. Rust provides a verifier-only crate and raw-message/
empty-context APIs, making it a relevant candidate. However, the inspected source explicitly warns that pre-1.0
keys/signatures may cease to be accepted and notes a July 2026 change. Taraxa pins `go-fn-dsa v0.2.0`; choosing the
newest Rust verifier without comparing the actual signature construction can change consensus.[^12]

The future comparison must pin the Go verifier's exact source and evaluate a historically corresponding Rust
revision as well as the current version. Compare degree, key/signature length, header bytes, hashing/domain
separation, canonical padding, and all malformed-input behavior. Preserve Taraxa's ABI and its inverted success
word. A standardization update belongs to a future activated protocol version, not an automatic dependency update.
The research identifies candidates, not a verified compatible Rust verifier.

## Maintenance, review and performance criteria

Choose one exact dependency graph, pin the Rust toolchain and source/package checksums, and keep a written list of
local compatibility overrides. Each upstream upgrade should review changes in those functions and run the Taraxa
differential corpus. Do not activate new Ethereum rules by allowing a default `SpecId` to follow a dependency update.

Prioritize independent review of custom frame/journal boundaries, integer conversions, native ABI validation,
cryptographic input compatibility, and cross-database recovery. A mature interpreter's security evidence does not
automatically cover Taraxa-owned adapter code. Distinguish deterministic protocol failures from database/infrastructure
errors; the latter cannot become fabricated transaction receipts.

Measure interpreter time separately from native execution, trie updates, storage commit, RPC reads, memory use, and
catch-up throughput. Require identical inputs, cache policy, compiler settings, hardware, and durability settings.
No performance win is assumed. Speculative parallel execution, JIT compilation, and a new database engine introduce
additional correctness dimensions and should follow a proved serial baseline.

## Decision gates

**Proceed to isolated feasibility work** when Designs A/B each have a concrete override map and exact dependency
baseline. Compare wide nonce/gas price, CREATE and nested creation, Taraxa gas, native rollback/transient behavior,
and raw-leaf root construction first. These experiments need separate implementation authorization.

**Select an engine** only after the experiments demonstrate all mandatory behaviors and the patch/replacement set
has a defensible maintenance owner. A small measured framework adapter wins over a custom frame executor; a clear
interpreter/host integration wins over a framework full of hidden shadow values. Failure to preserve valid behavior
rejects the design, not the input or the existing-network requirement.

**Do not claim production readiness** from engine feasibility. Historical database conformance, native kernel
integration, queries, full-node recovery, and network-reference evidence are independent gates.

## Sources

All external sources were inspected on 2026-09-12. Source-level identities and hashes are in
[library_sources.json](library_sources.json). Recommendations and integration-depth comparisons are engineering
judgments derived from these interfaces; library compatibility and performance remain unmeasured.

[^1]: Bluealloy, [v117 release](https://github.com/bluealloy/revm/releases/tag/v117), [pinned workspace manifest](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/Cargo.toml), [umbrella manifest](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/revm/Cargo.toml); release published 2026-08-28.
[^2]: Paradigm/Reth contributors, [v2.5.2 manifest](https://github.com/paradigmxyz/reth/blob/5a6940e351fed80458fe6c9da8581cbe4b8bd036/Cargo.toml), [EVM crate](https://github.com/paradigmxyz/reth/blob/5a6940e351fed80458fe6c9da8581cbe4b8bd036/crates/evm/evm/src/lib.rs); release published 2026-09-02.
[^3]: Alloy contributors, [v0.38.0 manifest](https://github.com/alloy-rs/evm/blob/aa16b156595d7de8d25c31fc0c346e608fa7eb9c/Cargo.toml), [EVM crate](https://github.com/alloy-rs/evm/blob/aa16b156595d7de8d25c31fc0c346e608fa7eb9c/crates/evm/src/lib.rs); release published 2026-08-26.
[^4]: Alloy contributors, published alloy-trie 0.9.5 [VCS metadata](https://docs.rs/crate/alloy-trie/0.9.5/source/.cargo_vcs_info.json), [account type](https://docs.rs/crate/alloy-trie/0.9.5/source/src/account.rs), [hash builder](https://docs.rs/crate/alloy-trie/0.9.5/source/src/hash_builder/mod.rs), [manifest](https://docs.rs/crate/alloy-trie/0.9.5/source/Cargo.toml.orig). The downloaded package checksum is recorded in the manifest.
[^5]: Bluealloy, [transaction trait](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/context/interface/src/transaction.rs), [account type](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/state/src/account_info.rs), [journal account](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/context/interface/src/journaled_state/account.rs).
[^6]: Bluealloy, [frame implementation](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/frame.rs), [handler](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/handler.rs), [pre-execution](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/pre_execution.rs).
[^7]: Bluealloy, [instruction provider](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/instructions.rs), [host/storage instructions](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/instructions/host.rs).
[^8]: Bluealloy, [precompile provider and output conversion](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/handler/src/precompile_provider.rs), [stateful precompile example](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/examples/custom_precompile_journal/src/main.rs), [journal interface](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/context/interface/src/journaled_state.rs).
[^9]: Bluealloy, [interpreter Host](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/context/interface/src/host.rs), [GASPRICE instruction](https://github.com/bluealloy/revm/blob/6014612c86f3690e4e9173a8c4deade396af398d/crates/interpreter/src/instructions/tx_info.rs); Taraxa [transaction codec](../../rust/crates/rustaxa-types/src/transaction.rs).
[^10]: Ipsilon, evmone v0.23.0 [CREATE/CALL implementation](https://github.com/ipsilon/evmone/blob/2184b24461ae20398520f0bc5023cb7dcc0e003c/lib/evmone/instructions_calls.cpp), [release](https://github.com/ipsilon/evmone/releases/tag/v0.23.0), published 2026-08-11.
[^11]: Ipsilon, [storage gas rules](https://github.com/ipsilon/evmone/blob/2184b24461ae20398520f0bc5023cb7dcc0e003c/lib/evmone/instructions_storage.cpp), [revision instruction tables](https://github.com/ipsilon/evmone/blob/2184b24461ae20398520f0bc5023cb7dcc0e003c/lib/evmone/instructions_traits.hpp).
[^12]: Thomas Pornin, [pinned Rust FN-DSA README](https://github.com/pornin/rust-fn-dsa/blob/0629bb118ebf077857e78c1bd067c704a37795ed/README.md), [verifier source](https://github.com/pornin/rust-fn-dsa/blob/0629bb118ebf077857e78c1bd067c704a37795ed/fn-dsa-vrfy/src/lib.rs), [published verifier 0.4.0](https://docs.rs/crate/fn-dsa-vrfy/0.4.0), published 2026-07-22; Taraxa [Go dependency pin](../../submodules/taraxa-evm/go.mod).
