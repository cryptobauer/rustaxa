# Bounded conditional native-effects witness

This is a source-derived diagnostic contract for H=25,706,949, reviewed by
Astra high because initialization, delayed policy and cleanup behavior interact.
It authorizes no replay, native-session execution, publication or producer-policy
claim. The existing Rust snapshot owner continues to require complete native state.

## Input boundary

Bind the frozen transaction preflight report SHA-256
`d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437`
and its source SHA-256
`0cbcc3c97cea6eed0784be84fafbf401a43ceb6ae2e3bfc647ccde2c386ee66c`.
Its exact 19 ordered transactions are ordinary signed calls; receiver accounts
were checked for zero code size before execution, and the executor rejected code,
ordinary-storage and raw-storage writes. These gates establish no receiver
bytecode or nested native calls; empty calldata alone would not suffice.
This is reused historical evidence, not a fresh transaction replay.

Also bind the independently reconstructed reward-plan report, requiring empty
distributions and current-period caching. Do not substitute header total_reward=0
for that requirement. Pin candidate mainnet policy and retained CF8 baseline
configuration evidence. The sole retained baseline corroborates delay five at H
as well as Q, but does not authenticate the producer's hardfork/PBFT policy.

## New read bound

Requalify the guarded working pair through existing Rust owners. At the exact
parent H−1=25,706,948 root
`926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`,
perform only three top-level calls over two logical targets:

1. Authenticated `account_at(0xfe)`: require Present with code_size > 0.
2. Physical `storage_at(0xee, leftpad32(02))` and authenticated proof of that
   jailed-address list: require Member with exact physical bytes `c0`.

This deliberately narrower implementation rejects missing/nonmember jail-list
history instead of extending its absence contract. Any nonempty list, missing
DPoS account, zero code size, proof mismatch or unavailable required dependency
stops the conditional witness. Owner-internal root/path reads and fixed pair
qualification are additional; no code bytes, nonce, supply, reward counters,
validator inventory, other jail keys or historical ranges are requested.

## Conditional source argument

- Candidate system schedule uses H+5=25,706,954, remainder 2,954 modulo pillar
  interval 4,000. The existing Rust system-transaction planner returns empty on
  this non-pillar branch. Unread bridge facts are dead-branch placeholders,
  never assertions that a bridge account or code is absent.
- Go BeginBlock installs Aspen DPoS code only when code_size is zero; positive
  parent code size excludes it. H differs from exact Cornus height 15,610,000,
  excluding DPoS/OP code replacements. Slashing Init/Register modify memory;
  storage initialization/nonce mutation occurs only on a jail mutation.
- Empty reward distributions cause zero DistributeRewards calls. EndBlock still
  runs, so empty distributions alone cannot prove empty effects.
- The proved ordinary transfers and empty rewards cannot dirty deferred DPoS
  counters. Cold lazy counters remain untouched; a warm prior EndBlock normalized
  their originals. H differs from redelegation correction height 3,091,000.
- With an empty parent jail list, slashing cleanup has no durable writes for
  either cold or warm cleanup timer state.

This explains a conditional absence of additional durable effects under the
pinned source/candidate policy. It does not execute Rust EndBlock, qualify the
producer binary, close reward-root replay, reconstruct a complete DposSnapshot,
or authorize adoption. Keep all those report flags false.

## Reference pins

Immutable Go revision `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`:

| Source below `taraxa/` | SHA-256 |
| --- | --- |
| `C/state.go` | `b6effa29e1d465a0bcc1badcf398a57e982d16b0d6f4f9b14844632306f7e3ec` |
| `state/state_transition/state_transition.go` | `d447ac4599f0107ccad4379f95a7269a6d5729283d8dcf00db850ea8fe119e90` |
| `state/state_transition/state_hardforks.go` | `1432c0e39e7ecaee8683c3e9fd28571e97f816e906c3938b0f38fa886c246aae` |
| `state/contracts/dpos/precompiled/dpos_contract.go` | `28bb58adb9d99d604fdacf0c284deea49c6887eee789381a5c5a231b5c89f565` |
| `state/contracts/slashing/precompiled/slashing_contract.go` | `2ac21dbe9ece16e11f1ab85d2670488d24a2a0e8940d2c181ced142511c6edfb` |

Rust references are `ConcreteCheckpointReaders`,
`final_chain_execution.rs::plan_external_evm_system_transactions`, and the
pillar-period predicate in `consensus_application_runtime.rs`. No new production
facade or fallback is needed for this diagnostic.
