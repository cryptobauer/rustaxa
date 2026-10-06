# Signed current-source H1 simulation

This bounded test profile closes the same-height history authority gap. It does
not change runtime code or production routing. Implementation base `e0fe4e125`.
The prior synthetic d1 corpora and all shared producer support remain unchanged.

The new caller is `77952ce83ca3cad9f7adcfabeda85bd2f1f52008`, derived from
secp256k1 key bytes `31` repeated 32 times. Both pinned Go revisions sign a
canonical EIP-155 legacy transaction for chain 666, recover its sender, and
execute its decoded fields. Rust decodes the same public envelope and constructs
public finalization metadata from those facts. The real prefix moves 300 from
validator 31 to existing validator 33 at H1. A normal signed PBFT PeriodData
input and public `finalize_block` build Rust history; no private snapshot or
native poststate is imported into its semantic owner.

Genesis has total stake 5000, validator stakes 2000/1000/2000, caller pairs
31=1000 and 33=1000, and other delegator a1=1000 at each validator. Go debits
genesis staking. Rust receives the equivalent post-debit ordinary caller
balance 3000 and absent a1 account. Native escrow is 5000 with nonce 1.
The emitted config records all hardfork, reward, supply and delegation values.

Actual Go H1 after prefix, EndBlock and Commit has identical 42 selected raw
and decoded native facts. Stakes are 1700/1000/2300; caller order is [31,33]
with pairs 700/1300. Source head/cursor/current node is 1 with count 2.
Destination 32 has head 0 and current node 1 absent. Pools and mirrors are
zero as recorded. The full physical H1 seed has 119 rows and root
`c79da310733d2b159cd62bd85890483e0a1267838c35867316ad25f86a3018e1`.
Caller committed nonce is 1. The full seed includes ordinary accounts, native
account/code, trie/version rows, and native membership absence proofs.

Go DryRunner executes full 700 from 31 to absent caller destination 32 at
the same H1, then repeats from a fresh reader. It overrides supplied nonce
2^512 with committed nonce plus one, 2. Both revisions return success, gas
101912, empty return/errors and one exact 700 event. Full committed state is
unchanged. The producer uses copy-only cache snapshots with an immutable
genesis fallback and respects empty tombstones. It does not patch source
control flow or fabricate a native poststate.

Rust independently builds and reopens public H1. Its ordinary account and
current native calls match the committed facts. The public stake-list API uses
delegation delay and therefore still returns genesis stakes; current H1 stakes
are checked through `getValidator(address)`. This distinction prevents delayed
eligibility reads from being mistaken for current state.

Two owner lifetimes and two concrete reader lifetimes run 80 fresh disposable
sessions: 44 successes and 36 injected late authentication failures, each
followed by a fresh success. Every success authenticates 18 native raw reads.
Each failure at indices 9 through 17 stops at the selected read, drops its
session once, and preserves all public committed facts and full physical rows.
Successful probes compare gas, output and logs with actual Go. Supplied nonce,
descriptor/root, physical rows and committed owner remain unchanged.

Evidence lives in the new `native_redelegate_current_source_simulation`
fixture directory. The manifest binds both revisions, unchanged source targets,
copy-only snapshot helpers, producer support, harness, output and stderr hashes.
The run directory is
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`.
Its authority contract is `current-source-h1-authority-contract.md`.
First failed producer and Rust gates are retained there; final validation and
independent frozen review must pass before acceptance.

Tier 1 applies to this fixture/test-only slice. Required checks are actual
dual-pin reproduction, Python syntax, affected Rust check/Clippy/package tests,
and serial fast validation. Existing accepted ON bridge evidence for unchanged
runtime remains applicable. No synthetic Rust header/root parity is claimed.
Estimation is the next settled derivative; error variants need actual same-H1
Go measurements. Broader reward/order profiles and production acceptance stay
open.
