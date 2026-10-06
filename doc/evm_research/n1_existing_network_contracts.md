# N1 existing-network contracts and coverage matrix

Status: initial source inventory and implementation handoffs. N1 remains open
until the execution/native/trace matrix and real replay inputs are qualified.
The active milestone is [10](10_existing_network_milestone.md).

## Declared profile

The target is the checked-in mainnet configuration, chain ID 841, including the
Cacti profile active at candidate replay period 25,706,949. The configuration
is an identified reference input, not proof of the snapshot producer's exact
binary or configuration. Relevant historical fixtures bracket each rule below;
a current-profile result does not establish older execution behavior.

The owner-reported likely producer commit exists in the local repository:
`a0e85fe31eb03573cd92c165a5f81035cec9907e` is the release/v1.14.1 merge,
pins EVM `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` (our public oracle),
and contains a byte-identical mainnet genesis file to the current tree. This
supports reference selection; it still does not identify the actual executable,
runtime overrides or snapshot capture command.

| Rule | Mainnet period | Coupled behavior requiring coverage |
| --- | ---: | --- |
| Redelegation correction | 3,091,000 | Top-level native restriction and exact correction writes |
| Magnolia / reward frequency | 5,730,000 | Native custody, fee/reward ordering, jail semantics, distribution frequency 100 |
| Phalaenopsis | 6,943,000 | Special escrow transfer/system selector |
| Claim-all correction | 7,600,000 | Historical selector/ABI behavior |
| Aspen part one | 8,118,000 | Claim-all behavior |
| Aspen part two | 8,572,000 | Supply/yield and rewards serialization |
| Ficus | 11,616,000 | BLAKE2F/BLS registry, custody encoding, pillar/bridge inputs |
| Cornus | 15,610,000 | Envelope failures, native payability/V2 custody, gas limits |
| Soleirolia | 17,380,000 | Transaction gas price/limit admission |
| Cacti | 24,350,801 | Transient aliases, BLS remap, P-256/Falcon, jail/locking changes |

Sources: `libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json`,
the pinned Go chain configuration and registry, and existing Rust FinalChain
configuration conversion. Delay is five periods; delayed native reads require
`max(period - delay, 0)` with operation-specific live versus frozen caches.

## Stateless registry

Addresses are exact zero-prefixed 20-byte addresses. `0x0a` is unregistered in
both pinned registries; no KZG activation is implied. Pre-Ficus has only 1–8.

| Address | Ficus | Cacti |
| --- | --- | --- |
| 1–8 | ECRECOVER, SHA256, RIPEMD160, identity, MODEXP, BN254 add/mul/pair | Same |
| 9 | BLAKE2F | Same |
| 0x0b | BLS G1 add | BLS G1 add |
| 0x0c | BLS G1 mul | BLS G1 multiexp |
| 0x0d | BLS G1 multiexp | BLS G2 add |
| 0x0e | BLS G2 add | BLS G2 multiexp |
| 0x0f | BLS G2 mul | BLS pairing |
| 0x10 | BLS G2 multiexp | BLS map G1 |
| 0x11 | BLS pairing | BLS map G2 |
| 0x12 | BLS map G1 | Unregistered |
| 0x13 | BLS map G2 | Unregistered |
| 0x0100 | Unregistered | P-256 verify |
| 0xfa1c | Unregistered | Historical Falcon verify |

Source: `submodules/taraxa-evm/core/vm/contracts.go`. Existing 1–9 adapters
retain their targeted fixtures. New BLS and P-256 work must compare the actual
Go implementations, including length/gas/error precedence and static/delegate
contexts. Falcon must reuse the already researched historical verifier version;
modern verifier rejection of historical signatures is not acceptable parity.

## Consensus native coverage

Existing Rust FinalChain already owns native business kernels. The following
gap is in staged execution, ordered physical serialization and composition,
not permission to reimplement those kernels in EVM or C++.

DPoS address `0xfe` includes registration, delegate, V1/V2 undelegate,
V1/V2 confirmation/cancellation, redelegate, claim rewards, both historical
claim-all forms, commission claims, set commission, set validator information,
the special escrow transfer, validator/validator-page queries, eligibility/vote
queries, delegation queries and V1/V2 undelegation queries. Registration selector
`d6fdc127` is included even though it was omitted from the first helper inventory.
The initial staged-session inventory admitted only setCommission, delegate,
undelegateV2 and confirmUndelegateV2. Later slices added V1 custody, cancellations,
reward claims and selected queries. The current bounded
[validator metadata slice](n2_staged_validator_info.md) adds decoded
setValidatorInfo over consistent snapshots to pending and historical sessions.
The subsequent [metadata ABI/admission slice](n2_validator_info_abi.md) adds
selector-first admission and pinned dynamic-ABI errors, including a synthetic
Cacti corpus. The [frame/API composition slice](n2_validator_info_frames.md)
adds real pending-driver and disposable historical API tests over eight actual
Go frame cases. [Actual metadata DryRunner parity](n3_metadata_dry_runner.md)
adds nine cases and 36 persisted-reader executions with exact nonce, error,
gas, output, log and disposal comparisons. [Metadata estimation](n3_metadata_estimation.md)
compares actual Go probes, the unchanged C++ search and fresh real Rust sessions
across reader reopen. [Direct metadata structured traces](n3_direct_metadata_traces.md)
add live prefix/target state and exact default JSON over persisted readers.
Sparse-state, delayed native tracing, nested/OpenEthereum and full coverage
limits remain open.
The [actual escrow DryRunner slice](n3_escrow_dry_runner.md) compares five
dual-pin cases with 20 fresh persisted Rust simulations across physical reopen.
[Escrow estimation](n3_escrow_estimation.md) compares 26 actual Go probes with
the unchanged C++ search and 104 Rust simulations. [Direct escrow traces](n3_direct_escrow_traces.md)
compare seven actual Go sequences with 28 Rust runs across reader reopen.
Nested/delayed traces and full-width value funding remain separate gaps.

The [redelegation observation corpus](n2_redelegate_observations.md) captures
actual pinned read/write vectors and failure prefixes before staged wiring.
Go caches all native rows on a second partial call in the same block; storage
observations do not prove all API lookup behavior. The bounded adapter now
admits positive partial distinct-validator zero-reward redelegation with retained
positive validator stakes, including an absent caller destination pair. See the
[existing-pair adapter](n2_redelegate_observations.md),
[ABI/frames](n2_redelegate_frames.md), [simulation](n2_redelegate_simulation.md),
[estimation](n3_redelegate_estimation.md), [direct traces](n3_direct_redelegate_traces.md)
and [new-destination adapter](n2_redelegate_new_destination.md).
New-destination [frames](n2_redelegate_new_destination_frames.md),
[simulation](n3_redelegate_new_destination_simulation.md) and
[estimation](n3_redelegate_new_destination_estimation.md) also have actual bounded
synthetic parity. [Full caller-source removal](n2_redelegate_full_source_retained_validator.md)
also has bounded staged parity when another delegator keeps the source validator
positive and the destination caller pair already exists. Its [complete simulation](n3_redelegate_full_source_simulation.md),
[estimation](n3_redelegate_full_source_estimation.md) and [direct traces](n3_direct_full_source_redelegate_traces.md)
have bounded actual parity. [Zero before Aspen part two](n2_redelegate_zero_existing_pairs.md)
admits only two existing positive caller pairs at the staged boundary; its
[frame composition](n2_redelegate_zero_frames.md) has actual account/cursor/rollback
parity; [historical simulation](n3_zero_redelegate_simulation.md),
[estimation](n3_zero_redelegate_estimation.md) and [direct traces](n3_direct_zero_redelegate_traces.md)
also have bounded actual parity. [Full source into a new destination](n2_redelegate_full_new.md)
has bounded staged parity only for a one-member caller, both validators retained
and both current reward nodes absent. Its [actual frames](n2_redelegate_full_new_frames.md)
prove deleted/reused caller membership through parent revert and a second
source-missing failure. [Complete H1 simulation](n3_redelegate_full_new_simulation.md)
has bounded actual parity with price0 and exact d1/a1 provenance.
[Estimation](n3_redelegate_full_new_estimation.md) matches actual probes and the
unchanged search. [Direct traces](n3_direct_full_new_redelegate_traces.md) compare
actual single-success, retained-prefix missing-source and stale-nonce outcomes.
[Source-first two-member swap and append](n2_redelegate_swap_append.md) adds
bounded staged parity for caller[31,33] becoming[33,32], both affected current
nodes absent and a positive retained third pair. [Actual frames](n2_redelegate_swap_append_frames.md) compare moved/reused
slots and parent rollback. [Complete H1 simulation](n3_redelegate_swap_append_simulation.md)
compares eight disposable sessions and persisted restart; [Estimation](n3_redelegate_swap_append_estimation.md) matches six actual
probes and unchanged search; [Direct traces](n3_direct_swap_append_redelegate_traces.md) compare
actual live prefixes and failure disposal. Source-last and longer full+new orders, existing current
node success, source-validator deletion, zero+absent destination/reward-bearing/new-validator and historical
same-validator success remain excluded. Real-history and production acceptance
remain open.

The [staged escrow-entry slice](n2_staged_escrow_transfer.md) admits exact active
`44df8e70`, with 1,000 gas, arbitrary-width value and empty native effects.
Actual frames prove that ordinary account transfer and rollback remain outside
the native kernel. Inactive/trailing staged ABI error presentation and pre-Cornus
constructors remain unsupported.
Existing adapters retain their recorded historical limitations; this is
not complete N2 acceptance.

Slashing address `0xee` includes commitDoubleVotingProof, getJailBlock and
getJailedValidators. Unknown or truncated selectors fail ABI lookup in Go;
the final switch default cannot be used to claim an unknown-selector no-op.
Delayed jail reads, proof validation and exact mutation/error ordering must use
the existing Rust kernels and be compared through the staged execution boundary.

The terminal rewards adapter now supports Aspen part two and ordered singleton-
validator distributions. Zero configured yield now preserves deferred end-block writes while skipping
distributions. Multi-entry validator maps, jailed cleanup and correction periods remain guarded.
Those restrictions block existing-network period completion and must be removed
through kernel reuse and exact reference serializers, not by filtering inputs.
The real system planner must establish whether each replay period has bridge or
other system actions; a missing stored system list alone does not prove emptiness.

## API and import contracts

The [N3 handoff](n3_api_parity.md) records operation-specific committed-state,
simulation, estimation and trace behavior. Historical reader errors remain
distinct from authenticated absence. API acceptance includes delayed reads,
unavailable/pruned history, future-block policy, fresh gas probes and trace
prefix transactions. Tracing is not closed by returning only a terminal result.

The supplied pair lacks Rust lifecycle/catalog and historical native sidecars.
Existing head-zero initialization cannot be reused as implicit legacy adoption.
Read-only replay preflight is separate from authoritative bootstrap. Bootstrap
must qualify both databases and reconstruct required semantic/catalog inputs
before application-approved metadata adoption on another disposable copy.
No broad 1..head hydration loop or fabricated catalog is an acceptable shortcut.

## Current implementation handoffs

Initial worktrees start at `68d23f19f`; lead integration stays on the feature branch.
The subsequent current-rewards worktree starts at `64c3b4656` and the Falcon
worktree at `a9ea2fcdf`.

| Agent/model | Worktree / branch | Owned work |
| --- | --- | --- |
| Luna helper | `/tmp/rustaxa-evm-api-estimate`, `task/evm-api-estimate` | `estimate.rs`, focused estimator tests; integrated with independent C++ reference |
| Sol execution | `/tmp/rustaxa-evm-p256`, `task/evm-p256` | P-256 adapter, oracle, fixtures and evidence |
| Sol state | `/tmp/rustaxa-evm-state-qualification`, `task/evm-state-qualification` | Experimental read-only head preflight and evidence |
| Sol oracle | `/tmp/rustaxa-evm-api-oracle`, `task/evm-api-oracle` | Actual Go DryRunner oracle and Rust simulation reference tests |
| Sol BLS | `/tmp/rustaxa-evm-bls`, `task/evm-bls` | BLS primitives, exact registry mapping, oracle and evidence |
| Sol rewards | `/tmp/rustaxa-evm-current-rewards`, `task/evm-current-rewards` | Current Aspen2 terminal rewards via existing kernels |
| Sol Falcon | `/tmp/rustaxa-evm-falcon`, `task/evm-falcon` | Historical Falcon ABI/crypto adapter and oracle |
| Astra lead | Feature branch | Simulation facade, shared exports/contracts, reference review and integration |

The existing independent snapshot copy is
`/tmp/rustaxa-evm-s0/.snapshot-work/snapshot-litenode-copy`; the workspace-local
relative spelling in older reproduction examples is not a second existing copy.
Only bounded read-only opens of the qualified copy are used for preflight.

[Source-current oracle preparation](n2_redelegate_current_source_oracle.md)
resolves one live pending observation gap and confirms Go source count2->1->2.
The [bounded Rust repair](n2_redelegate_current_source.md) now passes exact cold/livewarm
parity, authenticated failure isolation and independent review. Source-current
frame/history/API derivatives and broader current-node profiles remain open.
