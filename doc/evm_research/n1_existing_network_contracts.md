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
The staged session currently admits only setCommission, delegate, undelegateV2
and confirmUndelegateV2; even these retain the bounded historical limitations
recorded by M1–M6. All remaining adapters are N2 work.

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
