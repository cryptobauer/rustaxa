# Creation frame feasibility checkpoint

> Historical checkpoint. The [final experiments](06_native_storage_crypto.md) and
> [direction decision](07_direction_decision.md) supersede its next-work and engine-selection status.

This continues [the initial checkpoint](04_feasibility.md) from `4033d6fea` on
`feat/rust/evm-state-db`. All new executable code is isolated under
`experiments/evm_feasibility`; production routing, consensus, FinalChain, native
kernels, the production dependency graph and upstream sources are unchanged.

## Result and decision

A bounded Taraxa-owned frame driver using the pinned REVM interpreter matches both
Go references on **16 creation scenarios**, comparing exact gas, return/error
fields, all account fields, account disk/commitment encodings and complete
post-EVM account roots. This closes the initial handoff-only gap for the tested
zero-value, no-storage subset. Continue with interpreter/host integration as the
preferred experimental depth; this is not a production-engine selection.

No upstream REVM patch was necessary. Rust reuses `FinalChainNonce` directly for
all creation state and never substitutes a shadow u64. The driver owns creation
address derivation, nonce increment/checkpoint order, collision handling, child
settlement, code deposit and parent rollback. REVM supplies instruction execution,
stack/memory, CREATE/CREATE2 gas forwarding and yielded child requests.

The experiment deliberately retains the original application's authority. It does
not create a second consensus/FinalChain manager or implement native business
logic. It has no CXX or production entry point.

## Corpus and independent comparison

[`reference_frames.go`](../../experiments/evm_feasibility/reference_frames.go)
builds complete in-memory account inputs and executes the actual pinned Go EVM.
The public/local revisions are unchanged from the previous checkpoint; their
outputs remain byte-identical. Manifest schema two hashes every exporter Go file,
including the new frame exporter. A fresh exporter run reproduced both artifacts.

The sender at `0xaa` starts with nonce 1 and balance 1,000,000. It calls the parent
at `0xbb` with nonce 1, price 1 and value zero. The parent starts with the recorded
wide nonce and bytecode; collision fixtures also initialize the expected child.
Only Cornus is enabled. Gas cap is 300,000 except the code-deposit-OOG fixture,
which uses 54,500. Exact bytecode, candidate addresses and output accounts are
recorded in each row. No signed wire transaction or network checkpoint is implied.

The allowed bytecode can create only the parent’s child and its grandchild. The
exporter enumerates those plus sender/parent, records absence explicitly, and
uses the Go trie writer for the root. Rust independently executes the recorded
parent code, encodes its resulting accounts and computes the root with triehash.
The root describes complete post-EVM state for this bounded corpus, before physical
database flush/reopen; it is not an operational RocksDB commitment test.

| Cases | Observed compatible behavior |
| --- | --- |
| CREATE at nonce 2^64, U256::MAX, and 2^256 | Exact address derivation and arbitrary-width successor; 53,039 total gas in the success cases |
| CREATE collision and exceptional child halt | Caller nonce increment survives child failure; child gas is consumed and parent receives zero; 296,154 total gas |
| Child REVERT with byte `ab` | Child disappears, caller nonce increment survives, RETURNDATASIZE/COPY exposes `ab`; parent succeeds with 53,066 total gas |
| Successful child followed by parent REVERT | Parent nonce and created account revert; outer transaction nonce/fees persist; top-level return bytes remain observable |
| Child creates grandchild | Both created accounts and child nonce two persist; 85,048 total gas |
| Grandchild followed by child or parent REVERT | Appropriate enclosing checkpoint removes descendant state; the outer surviving creator increment remains |
| Runtime-code deposit | Single byte `ef` installs successfully; 53,257 total gas. No modern invalid-code-prefix restriction is introduced |
| Code-deposit out of gas | Child account reverts, parent returns zero and succeeds; 54,489 total gas |
| CREATE2 at U256::MAX | Salt/initcode-derived address and wide caller successor; 53,048 total gas |
| Repeated CREATE2 | First child survives, second creation collides, creator increments twice; 296,654 total gas |
| CREATE2 child/parent REVERT | Same checkpoint distinction, without nonce-derived address substitution |

## Maintenance surface and remaining risks

The driver uses a deliberately small opcode allowlist, zero-value creation, a
fixed valid envelope and depth below eight. Unsupported opcodes/host calls panic,
so incomplete infrastructure cannot become a fabricated execution result. Whole
account-map clones implement checkpoints for auditability; this is explicitly not
a proposed hot-path journal. Account balances are fixture-sized u64 values and
nonzero contract transfers are rejected. This does not settle Taraxa balance-width
requirements or general affordability.

The interpreter's `CreateInputs::created_address` still takes u64. The driver
bypasses it for CREATE with exact native-nonce RLP, and derives CREATE2 independently
from caller/salt/initcode. Newer EIP-8037 interpreter branches themselves inspect
bounded account nonce state. The experiment fixes `SpecId::ISTANBUL` and disables
state-gas behavior; that choice covers the shared operations tested here and is
**not** a Taraxa fork profile. A production profile must explicitly prevent those
bounded defaults from becoming active while supplying Taraxa’s mixed opcode/gas
rules. A dependency upgrade must review these exact functions and rerun the corpus.

The custom frame driver now demonstrates real child execution and settlement, not
just an action yield. It still does not implement CALL/CALLCODE/DELEGATECALL/
STATICCALL, value transfer, native dispatch, storage/log journals, max-depth parity,
SELFDESTRUCT, creation into balance-only accounts, code-size boundary cases,
general transaction failures or simulation. REVM's default framework remains a
comparison baseline with proven width/admission obstacles; a fully customized
framework was not implemented or benchmarked.

Next decisive work remains a real native kernel invoked inside a reverting CALL,
with exact ordered mutations and combined account/storage roots. Reuse the staged
FinalChain kernel through a narrow test composition or domain extraction; do not
reimplement DPoS transitions in this driver. Historical kernel exceptions and
incomplete snapshot provenance from the initial report remain open. Physical
node decoding/reopen and historical archive acquisition are separate outstanding
gates. No production routing or protocol change follows from this checkpoint.

## Validation and reproduction

From the repository root:

```sh
python3 experiments/evm_feasibility/reference.py
cargo test --locked --manifest-path experiments/evm_feasibility/Cargo.toml
cargo clippy --locked --manifest-path experiments/evm_feasibility/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path experiments/evm_feasibility/Cargo.toml --check
make rewrite-validate-fast
```

Both references reproduce; all eight isolated tests pass, with the new test
checking all 16 rows against both references. Clippy and formatting pass. The
repository fast gate also passes; expensive differential,
full-node, storage bridge and C++ checks are not needed for this unlinked Rust/Go
prototype and were not invoked. No existing tests were altered.
