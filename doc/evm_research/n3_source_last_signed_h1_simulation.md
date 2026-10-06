# Signed source-last current H1 simulation

This new profile combines accepted source-last current-node behavior with public
signed H1 history. Base `681a4e9af`; the later checkpoint commit changes docs only.
The fresh Astra medium authority record is `source-last-signed-h1-contract.md`
under the run2 artifact directory. Runtime and previous corpora stay unchanged.

Only initial validator order changes from signed source-first history to
[33,31,32]. Caller C is `77952ce83ca3cad9f7adcfabeda85bd2f1f52008`, derived from
key31 repeated32; VRF/metadata stay mapped by validator address. Both actual Go
pins recover the canonical chain666 signed nonce0 prefix, hash
`12fc7be2f39eff9ac60ea6be7436f9d298223c39be3996719124a15f7cdeaec8`,
then execute partial30031->33. Rust publicly decodes the same canonical envelope,
creates normal PBFT PeriodData and finalizes it once; reopen never refinalizes.
No desired poststate or private semantic snapshot is imported.

Actual after-prefix, after-EndBlock and after-Commit selected raw/decoded facts
agree: caller order [33,31], other/global [33,31,32], C31=700/C33=1300 with raw
last_updated1, current stakes1700/1000/2300, other pairs1000/updated0 and total5000.
Source head/cursor/node is1/count2/index0; destination head0/count2/index0, current
node1 and C cursor/mirror absent. Pools/index mirrors have exact zero/presence
authority. Complete actual H1 seed has119 rows and root
`4edf61341f5fbae1d13dfb50fc8552d37bd21374af06db0a05b272a69f6812c1`.
This new root differs from the retained signed source-first corpus.

Committed C balance3000/nonce1, a1 absent and native5000/nonce1 follow equivalent
Go debit and Rust post-debit constructor inputs. Concrete reader verifies full
physical account RLP, nonce/balance, native code/hash and all42 selected raw rows
from this actual seed. Rust semantic ordinary balances/nonces and native calls
are checked independently. The public delayed stake list stays address-sorted
[31,32,33] at genesis values; current getValidator and ordered getDelegations
carry H1 facts. Reward query word0 is distinct from raw pair last_updated1.

Actual same-H1 DryRunner full70031->32 runs twice on each revision. It succeeds
at gas101912, returns no bytes/errors and emits one exact700 event. Supplied
nonce2^512 is preserved separately; effective nonce is committed+1=2. Full seed,
descriptor and all committed account/native facts stay unchanged.

Rust uses fresh public begin_native_simulation(1) at identity.period1, with no
graph advance. Two owner lifetimes/two reader lifetimes run64 disposable sessions:
36 successes and28 independent failures at each late raw-read index9..15, with
a fresh valid retry after every failure. Success measures16 authenticated reads;
failure checks exact prefix, propagation and one session drop. Every attempt
preserves supplied nonce, public committed facts and all physical rows.
Internal Rust outcomes separately check16 writes, source node2->1->2 and no
retained item1/position33 writes. These effects use accepted source-last staged
authority; DryRunner does not export a new raw-write trace.

Manifest binds both pins, complete config, signing/source/support/snapshot helper
targets, output and stderr hashes. Tier1 fixture/test gates are actual dual-pin
reproduction, Python syntax, target/package/check/Clippy, serial fast, whitespace
and independent frozen review. Accepted source-last ON bridge12/all15 evidence
applies to unchanged runtime. No concrete Go/synthetic Rust root parity, traces,
production routing, broad gates, rewards or broader orders are accepted here.
Estimation is the next settled derivative using this exact new H1 identity.

Run evidence:
`/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r2-20261006`,
prefix `source-last-signed-h1-`.
