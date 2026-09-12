# Execution compatibility specification and reference evidence

> Source assessment at the pinned baseline. Subsequent executable evidence and the resolved
> architecture recommendation are recorded in the [direction decision](07_direction_decision.md).

## Decision and scope

The first migration must remain compatible with existing Taraxa networks. Preserve canonical execution and all
protocol-valid inputs, including unusual inputs absent from sampled history. Do not introduce Ethereum nonce caps,
fee rules, gas schedules, transaction formats, or rollback behavior as incidental engine changes. A later network
upgrade can change future-period rules only through a separate decision.

This specification identifies the highest-risk execution differences and their source-level requirements. It is
an implementation contract to refine with differential fixtures, not a claim that every possible interpreter path has
been proved equivalent. The parent proposal's description of “older gas behavior” is insufficient: Taraxa combines
individual rules from different eras, and the exact constants and transition order matter.

## Reference identity and ancestry

The local EVM history contains Ethereum interpreter commits from 2017 onward, followed by substantial Taraxa
restructuring and selective additions. A declared `go-ethereum v1.13.10` module dependency does not identify the
ancestry or behavior of the fork-owned `core/vm`. Treat the fork as a separate execution specification.[^1]

| Reference | Revision | Role |
| --- | --- | --- |
| Rustaxa source baseline | `922dd662502a0f932c033c7de142737b472af9a1` | Current Rust orchestration, native semantics, tests, configuration |
| Concrete executor baseline | `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418` | Exact local Go executor and concrete lifecycle |
| Public Taraxa EVM ancestor | `28c69ca6a4f9d65e9537f728c43a0f063fa3abc2` | Merge of the v1.14.0 release branch; public historical reference candidate |
| Taraxa node v1.14.1 EVM pin | `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` | Submodule revision returned by the published node release's contents API |
| Local change | `edccf3919` | Malformed slashing RLP handling; two changed files |
| Local change | `bb0ab67c8` | Concrete-root lifecycle, execution observations, staged reads/writes, rewards checks; 13 changed files |

The last two commits must be classified individually when selecting an established network reference. In particular,
the lifecycle change is not merely a tracing patch: it also changes staged persistence and API behavior. Compare the
Rust replacement both to the local reference composition and to a selected established Taraxa release with matching
network configuration. Neither reference automatically substitutes for the other.[^2]

Useful ancestry landmarks are the 2022 nonce/CREATE correction (`befcbc4e2`), removal of the general hardfork feature
from the interpreter (`dfcd1a5d1`), Cornus insufficient-gas-funds nonce change (`b2d7fc82a`), and transient opcode
address change (`430aa2679`). Commit titles guide investigation; executable source and applicable network rules decide
the contract. The exact original Ethereum fork point remains unestablished and is not needed to assume a named fork.

The GitHub release API identifies node v1.14.1, published 2026-02-16, as its latest release; the EVM `master` API
returns the public ancestor above. These are repository metadata observations, not evidence of the binary currently
deployed by network validators. Pin the node release's own submodule rather than assuming its EVM revision is the
same commit as the EVM repository head.[^14]

For these particular two revisions, local Git tree comparison establishes identical contents: both the node
v1.14.1 EVM pin and the public merge have tree `db119d09f081019c3ab567e15a49043bcd776295`. Their commit identities
differ, but there is no source-content difference between them. The two subsequent local changes still require
separate classification.

## Configured network profiles

These are values in the pinned repository's genesis JSON files, not independently verified live network schedules.
The manifest includes the complete configured hardfork sections, genesis hashes, and initial allocation counts.[^3]

| Rule boundary | Mainnet 841 | Testnet 842 | Devnet 843 |
| --- | ---: | ---: | ---: |
| Redelegation fix | 3,091,000 | 0 | 0 |
| Magnolia | 5,730,000 | 0 | 0 |
| Phalaenopsis | 6,943,000 | 0 | 0 |
| Claim-all fix | 7,600,000 | disabled sentinel | disabled sentinel |
| Aspen part one | 8,118,000 | 0 | 0 |
| Aspen part two | 8,572,000 | 0 | 0 |
| Ficus | 11,616,000 | 1,000 | 100 |
| Cornus | 15,610,000 | 1,000 | 1,000 |
| Soleirolia | 17,380,000 | 2,358,000 | 1,000 |
| Cacti | 24,350,801 | 2,934,000 | 0 |

The default fixture configuration is chain 844 and also has Cacti at genesis. Negative JSON sentinels and zero
activation require explicit conversion tests; C++ configuration decoding and Go `isForked` are part of that contract.
Do not sort hardfork names and assume their activation order. Cacti selects an instruction table that inherits Ficus
instructions even when another configured Ficus effect activates later. Independent native-contract/fork flags still
govern their own operations. Genesis and restart must produce the same combination of rules.[^3][^4]

## Transaction and environment requirements

Let `n` be the submitted nonce, `N` the account nonce, `G` the transaction gas cap, `p` its gas price, and `B` the
sender balance. The current execution sequence below is established by `EVM.Main`; it is not Ethereum admission
semantics. The zero-address system sender has separate affordability treatment.[^5]

| ID | Requirement | Consequence for replacement |
| --- | --- | --- |
| T01 | Normal wire transactions use nine-field legacy RLP; the decoded nonce is U256 | Preserve raw bytes/signing/hash semantics; do not admit typed Ethereum envelopes accidentally |
| T02 | FinalChain account nonce uses arbitrary-width `BigUint`; successor can exceed U256 | Never narrow persisted or journaled account nonces; U256 wire width and account width are different |
| T03 | For nonzero sender with `B < G*p`, charge `floor(B/p)*p`; report the affordable gas amount | Preserve the remaining balance smaller than one unit of gas price |
| T04 | On that insufficient-funds path, Cornus advances nonce to `n+1` only if `n >= N` | Do not replace this with a host validation rejection or unconditional increment |
| T05 | Otherwise deduct `G*p` before rejecting `n < N` | A stale transaction can charge the full cap without running bytecode |
| T06 | `n > N` is permitted | Wide-nonce reachability cannot be dismissed by estimating sequential transaction counts |
| T07 | Intrinsic-gas errors advance to `n+1` under Cornus and report cap consumption | Preserve pre-Cornus differences and exact error classification |
| T08 | Top-level creation first sets the caller nonce to `n`; creation derives the address and increments it | Default engine nonce handling and address derivation cannot run independently |
| T09 | Top-level calls set nonce to `n+1` before entering their frame | Frame rollback must preserve the surrounding envelope's mutations |
| T10 | Insufficient value balance and execution errors take different result paths | Preserve consensus-error versus execution-error mapping, receipt status, gas, and return data |
| T11 | Remaining gas reimbursement follows a refund cap of half gas consumed | No automatic London refund policy or beneficiary payment |
| T12 | Dry-run sets a transient nonce to requested-state account nonce plus one | Historical simulation is a distinct envelope; do not reuse finalized transaction prechecks verbatim |

The integer audit must also cover balance and intermediate fee arithmetic. Go uses big integers in accounts while
current Rust native value types use bounded arithmetic. Prove the reachable range, particularly system-sender and
reward operations, before deciding whether an engine's U256 balance is sufficient. No unbounded-balance exploit or
network divergence is asserted here; this is an explicit proof obligation.[^5][^6]

Block environment values must remain FinalChain-derived: author, timestamp, period/number, gas limit, chain ID,
historical block hashes, and DIFFICULTY. Preserve the actual implementation of each opcode, including missing-opcode
behavior, rather than populating an Ethereum post-Merge environment with plausible defaults. The ordered transaction
list comes from the existing Rust application; a replacement execution library must not select or reorder it.[^4][^5]

## Instruction and gas profile

| ID | Established source behavior | Required discrimination |
| --- | --- | --- |
| G01 | Base intrinsic gas 21,000; creation 53,000; zero byte 4; nonzero byte 68 | Calldata pricing does not match modern Ethereum's nonzero-byte price |
| G02 | SLOAD 800; SSTORE no-op/dirty 200; clean zero creation 20,000; clean nonzero update 5,000 | A single upstream fork's storage schedule is insufficient |
| G03 | Storage clear refund 15,000; reset refunds 4,800/19,800; SELFDESTRUCT refund 24,000 | Preserve refund accumulation, rollback, and half-consumed cap |
| G04 | External-code and BALANCE costs 700; CALL base 700 | Do not silently introduce Berlin warm/cold pricing |
| G05 | PUSH0 in base table; MCOPY added by Ficus | Opcode availability is separate from global gas-rule choice |
| G06 | `0xb3`/`0xb4` transient aliases in base; `0x5c`/`0x5d` added by Cacti | Cacti inherits the old aliases; removing them is a protocol change |
| G07 | All selected tables use `GasTableCalifornicum` | Ficus/Cacti precompile and opcode changes do not imply corresponding Ethereum fork activation |
| G08 | Runtime code limit 24,576 bytes; creation checks and code-deposit gas live in custom frame logic | Audit initcode limits, prefix restrictions, collision rules, and depth separately |

These constants and table entries are direct source observations.[^4][^7] The full fixture specification must also
cover memory expansion/overflow, CALL gas forwarding and stipends, SSTORE's original/current-value selection,
SELFDESTRUCT deletion timing, empty-account cleanup, code hash/existence, and precompile failure gas. Shared opcode
names or comments naming an EIP do not establish identical edge behavior.

## Native contracts, cryptography, and rollback

DPoS and slashing are stateful native contracts with gas and context-dependent behavior. Their raw storage adapter
uses irreversible writes. The existing concrete projection distinguishes normal completion, own-frame reversion,
and enclosing-frame reversion with surviving native writes. Keep native business transitions in the already-written
Rust kernels; a new EVM must invoke them during nested execution rather than only after the block.[^8]

| ID | Compatibility requirement |
| --- | --- |
| N01 | Separate ordinary storage/balance/nonce rollback from native raw writes; logs have their own journal behavior |
| N02 | Preserve CALL, CALLCODE, DELEGATECALL, and STATICCALL caller/value/storage context; inspect native static-call behavior directly |
| N03 | Preserve delayed eligibility reads versus live same-block stake/reward reads and fork-dependent payability |
| N04 | Preserve native ABI outputs, malformed-input handling, ordering of business failures, gas, and account initialization |
| N05 | Keep native raw slot values as bytes, including values larger than an EVM word |
| N06 | Preserve genesis native state, DPoS code replacements, and installed OP-related bytecode at exact activation periods |
| N07 | Execute canonical system transactions and reward changes once, in Rust-planned order; preserve fee ownership changes |

**Transient storage discrepancy:** `SetTransientState` directly calls its setter without registering an undo closure,
despite a comment describing journaling. Transaction completion clears the map. Source therefore supports concern
about writes surviving nested reverts, but a differential fixture must establish the complete observable behavior.
This is distinct from the intentionally irreversible native raw-storage path. Neither issue authorizes a historical
behavior correction in the new implementation.[^9]

| Precompile profile | Address/format requirements |
| --- | --- |
| Base | `0x01`–`0x08`; preserve legacy BN gas and modular-exponentiation schedule |
| Ficus | Adds `0x09`; BLS operations occupy `0x0b`–`0x13`, including separate multiply operations |
| Cacti | BLS mapping changes to `0x0b`–`0x11`; adds P-256 at `0x0100`, Falcon at `0xfa1c`; KZG `0x0a` is not enabled in the tables |
| P-256 | 160-byte input, success word one, empty return for invalid input/signature; gas 6,900 |
| Falcon | Selector `0xde8f50a1`; three ABI byte arguments; FN-DSA-512 fixed-size signature/key; raw-message mode with no context; valid returns zero word, invalid generally one word |

Falcon shorter-than-selector and wrong-selector inputs return errors; other malformed forms often return the invalid
word. Gas is `1465 + 6*ceil(input_length/32)`. ABI offset conversion, bounds, empty messages, and accepted signature
encoding must be preserved—not replaced by a convenient generic ABI decoder or a similarly named crypto library.
The distinctions above are source facts, not cryptographic implementation endorsements.[^10]

## State commitments and public surfaces

The account disk codec has five fields including code size; the trie hash leaf has four with normalized empty
code/storage hashes. Concrete projection account RLP is a different representation again. Native raw slots use
RLP-wrapped byte values and share the account storage commitment with ordinary EVM slots. Preserve these distinctions,
key hashes, zero/deletion rules, and node hashing. A successful execution returning matching balances can still have
the wrong canonical root.[^11]

Public parity covers account/code/storage at a specified historical block, call results and errors, estimation,
traces, transaction receipt/log order, roots and finalized hashes, pruning, snapshots, and restart. Client-visible
encoding matters even where it is not consensus-critical. Wall-clock trace durations and performance counters should
not be compared as deterministic execution facts. Native consensus snapshots are not a substitute for arbitrary
contract state, nor are imported trie bytes sufficient to reconstruct all native semantic history.[^6][^11]

## Reference corpus and confidence limits

The checked-in tests provide reusable anchors: `nonce_above_u64_round_trips_account_receipt_and_restart`, native
DPoS/slashing parity families, concrete projection/discard/restart cases, and Rust account nonce-above-U256 tests.
The StateAPI Ethereum smoke and one DPoS integration test are explicitly disabled. Go's CTest entry runs `go test
./...`, but its presence is not evidence of a successful run. The packaged old Ethereum block data is not Taraxa
network history.[^12]

Taraxa's official snapshot instructions describe paired `db` and `state_db` directories and advertise light snapshots;
the page states full-node snapshots stopped being provided. This is an operational lead, not proof that a complete
archive corpus is available. The official connection page identifies public mainnet/testnet RPC endpoints, but that
does not guarantee historical state retention or a stable reference software version.[^13]

Bounded requests to the documented mainnet and testnet snapshot metadata APIs both timed out after 15 seconds during
this assessment. No snapshot archive was downloaded. Snapshot availability and completeness remain unresolved; the
design cannot depend on an assumed public archive download.

The required future evidence package should identify an established reference binary/revision, exact genesis and
effective configuration, block/transaction bytes, any state checkpoint and its trusted root, retention horizon,
fork-boundary ranges, and artifact checksums. Test mainnet before/at/after every configured boundary; also test
genesis activation and non-monotonic profile combinations using testnet/devnet fixtures. Require adversarial
wide-nonce and rollback cases regardless of whether network sampling finds them.

Outstanding evidence is bounded: full historical behavior remains unproved until replay; live software and network
heads are not inferred from checked-in schedules; a complete source diff against the exact original Ethereum fork
point is not available; and the source discrepancy around transient storage needs execution confirmation. These
limits do not prevent engine design research, but they prevent a deployment or full-parity claim.

## Sources

All repository paths refer to the baseline revisions recorded in [baseline.json](baseline.json); local links open
the corresponding checked-in files. Publisher for these sources is Taraxa Project or the Rustaxa repository unless
otherwise stated. GitHub commit links pin ancestry independently of moving branches.

[^1]: [Go module](../../submodules/taraxa-evm/go.mod) and [fork-owned interpreter](../../submodules/taraxa-evm/core/vm/evm.go); Git history of `core/vm/evm.go` at the EVM baseline.
[^2]: Taraxa Project, [public release merge, 2026-01-09](https://github.com/Taraxa-project/taraxa-evm/commit/28c69ca6a4f9d65e9537f728c43a0f063fa3abc2); Rustaxa EVM [malformed RLP change](https://github.com/cryptobauer/taraxa-evm/commit/edccf3919), [concrete lifecycle change](https://github.com/cryptobauer/taraxa-evm/commit/bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418). Local commits establish content, not network adoption.
[^3]: [Mainnet genesis](../../libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json), [testnet genesis](../../libraries/cli/include/cli/config_jsons/testnet/testnet_genesis.json), [devnet genesis](../../libraries/cli/include/cli/config_jsons/devnet/devnet_genesis.json), [default genesis](../../libraries/cli/include/cli/config_jsons/default/default_genesis.json), [C++ fork decoding](../../libraries/config/src/hardfork.cpp), [Go fork rules](../../submodules/taraxa-evm/taraxa/state/chain_config/chain_config.go).
[^4]: [Instruction tables](../../submodules/taraxa-evm/core/vm/jump_table.go), [opcode numbers](../../submodules/taraxa-evm/core/vm/opcodes.go), [EIP additions](../../submodules/taraxa-evm/core/vm/eips.go), [hardfork state changes](../../submodules/taraxa-evm/taraxa/state/state_transition/state_hardforks.go).
[^5]: [EVM.Main and call/create frames](../../submodules/taraxa-evm/core/vm/evm.go), [opcode implementations](../../submodules/taraxa-evm/core/vm/instructions.go), [transaction envelope](../../rust/crates/rustaxa-types/src/transaction.rs), [account nonce](../../rust/crates/rustaxa-types/src/final_chain.rs).
[^6]: [DryRunner.Apply](../../submodules/taraxa-evm/taraxa/state/state_dry_runner/dry_runner.go), [StateAPI public surface](../../libraries/core_libs/consensus/include/final_chain/state_api.hpp), [native FinalChain](../../rust/crates/rustaxa-consensus/src/final_chain.rs).
[^7]: [Gas constants](../../submodules/taraxa-evm/core/vm/constants.go), [gas table](../../submodules/taraxa-evm/core/vm/gas_table.go), [dynamic gas functions](../../submodules/taraxa-evm/core/vm/gas.go).
[^8]: [Native contract storage adapter](../../submodules/taraxa-evm/taraxa/state/contracts/storage/evm_state_storage_adapter.go), [account mutations](../../submodules/taraxa-evm/taraxa/state/state_evm/account.go), [revert dispositions](../../rust/crates/rustaxa-consensus/src/concrete_state_projection.rs), [native semantic replay](../../rust/crates/rustaxa-consensus/src/final_chain.rs).
[^9]: [TransitionState.SetTransientState, RevertToSnapshot, CommitTransaction](../../submodules/taraxa-evm/taraxa/state/state_evm/transition_state.go).
[^10]: [Precompile implementation and tables](../../submodules/taraxa-evm/core/vm/contracts.go), [precompile fixtures](../../submodules/taraxa-evm/core/vm/contracts_test.go).
[^11]: [Account codec](../../submodules/taraxa-evm/taraxa/state/state_db/main_trie.go), [slot codec](../../submodules/taraxa-evm/taraxa/state/state_db/account_trie.go), [trie sink](../../submodules/taraxa-evm/taraxa/state/state_transition/trie_sink.go), [concrete projection](../../rust/crates/rustaxa-consensus/src/concrete_state_projection.rs).
[^12]: [FinalChain reference tests](../../tests/final_chain_test.cpp), [StateAPI tests](../../tests/state_api_test.cpp), [Go test registration](../../tests/CMakeLists.txt), [Rust FinalChain tests](../../rust/crates/rustaxa-consensus/src/final_chain.rs).
[^13]: Taraxa Project, [Syncing From Snapshot](https://taraxa.gitbook.io/taraxa-network/node-setup/syncing-from-snapshot), [Connecting to Taraxa](https://taraxa.gitbook.io/taraxa-network/develop/connect-to-taraxas-network); accessed 2026-09-12, publication revision unspecified.
[^14]: Taraxa Project, [node v1.14.1 release](https://github.com/Taraxa-project/taraxa-node/releases/tag/v1.14.1), [release submodule metadata](https://api.github.com/repos/Taraxa-project/taraxa-node/contents/submodules/taraxa-evm?ref=v1.14.1), [EVM master metadata](https://api.github.com/repos/Taraxa-project/taraxa-evm/commits/master); retrieved 2026-09-12.
