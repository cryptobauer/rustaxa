# Executable cold Go native-effects witness

Continuation from `399b3d8ae`. The [exact report](n4_empty_native_go_cold.json)
executes actual `StateTransition.Init -> BeginBlock -> EndBlock -> Close` from
immutable Go revision `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`, in a disposable
archive with an explicit observer-only patch. It tests one cold constructor at
H=25,706,949, without transactions or reward distributions.

The run made **two fixture reads**: the 85-byte parent DPoS account RLP and the
one-byte jailed-list `c0`. Backend Put attempts, Commit attempts and observed
TrieSink mutation attempts were all **zero**. Close drained asynchronous queues
before the success report. Separate-process StartMutation and Delete controls
each produced the exact observer rejection, proving the observer is active.

## Fixture and authority

The two payloads come from the [authenticated parent diagnostic](n4_empty_native_effects.md).
The Go fixture port itself performs **no authentication**: its parent period/root
are caller-supplied identity labels, not a reconstructed trie or complete database.
Every other key/column, repeated read, Put and Commit is denied. Constructor
history-reader selections at 25,706,943 are recorded as labels; every historical
read is denied, rather than substituting parent bytes at a different period.

The input also pins the independent empty reward plan, retained DPoS config,
candidate mainnet configuration and historical transfer preflight. Constructor
policy uses all twelve retained DPoS scalars plus the checked-in candidate's
20 initial validators, balances and Taraxa hardfork configuration. Ethereum
configuration supplies chainId only; no transaction or bytecode runs here.
Unknown fixture/configuration JSON fields are rejected. This does not establish
the producer's effective configuration or binary.

Fixture JSON SHA-256: `96c05f7c7a774fa0eb6ba9ece995155f1c8145c1a2aff29a20479c80fa222cfe`.
Constructor config SHA-256: `ee84a255752ba108f07ff945b9e94a3199d3efb3900ded094490309cf5c0238d`.

## Instrumentation and validation

The runner verifies six immutable source hashes, archives the exact commit and
patches only the disposable `TrieSink.StartMutation` and `TrieSink.Delete` entry
points to reject mutation attempts. Exact file hashes and unique insertion
needles guard the patch. The original Go checkout is unchanged; original,
patched and observer hashes are reported. This is instrumented reference
execution, not an unmodified producer binary run.

```sh
python3 experiments/evm_feasibility/empty_native_effects_reference.py --self-test
python3 experiments/evm_feasibility/empty_native_effects_reference.py local/evm-state-db/reports/empty-native-go-cold-final.json
```

Outputs must be new files outside the supplied DB, working copy and original Go
checkout, including canonical parent aliases. Pure guard checks cover protected
paths, symlinks and existing outputs. The runner does not open either database.
Root reproduced the frozen positive run and both controls with Go1.24.4;
Python execution, Go formatting and whitespace checks passed. Astra medium
independently verified source/archive/report hashes and reconstructed the exact
fixture/config hashes from pinned inputs without executing the harness or
opening a DB. No blocking review finding remains.

Report SHA-256: `80c3b64e6789ce502c1c24ea44cb781ced3acf46696984d31160d0869162f076`.
Go harness SHA-256: `8540e39d3ad6103360b7dfea0279acda4d79f89d5e4cd048d9684682c85c610d`.
Python runner SHA-256: `d2c5d3837e87a2ef7cbf3c9f727a946e2fe5fa77cd716f33850401d5f93ade90`.

## Remaining boundary

This strengthens the earlier source-derived argument with executable **cold Go**
evidence. It does not run a warm constructor, Rust EndBlock, transactions,
DistributeRewards, PrepareCommit, a trie/root calculation, publication or adoption.
The transaction-only root witness remains historical and separate. Full native
reconstruction, producer qualification, reward-inclusive real replay/root proof
and publication/recovery remain open; a sparse fixture must never be passed off
as a complete native snapshot.
