# S4 isolated persisted-period integration map

Status: preparatory API map, independently reviewed; the persisted execution
path is not implemented or qualified yet. The S1 descriptor-only composition
must remain a refusal test until real S2/S3 state and execution can replace it.

Use the existing `execute_final_chain_application_task` entry point in
`rustaxa-consensus/src/final_chain_execution.rs`. It already owns preflight,
execution/rewards validation, pending application publication, concrete commit
intent and final publication. A test-only `ConsensusExecutionPort` adapter can
borrow the existing `FinalChain` and own pending executor/writer state; the
blanket `FinalChainExecutionLeaf` adapter supplies the application boundary.
Do not introduce a parallel chain manager or reenter an application-owner lock.

## Fresh state and execution inputs

Create independent temporary application and concrete databases. Seed real
synthetic genesis state through the compatible writer, persist a fresh database
identity and generation-zero provenance under the existing policy, then use
`FinalChain::new_with_genesis_state_root` with that exact root and enforcement
enabled. Derive the chain identity from the same concrete configuration. The
markerless imported light snapshot does not authorize this bootstrap.

The selected fixture starts with a funded ordinary sender and no contract code
at genesis. Period one includes a transfer and CREATE whose initcode writes
storage and installs a small runtime; after reopening, period two calls that
runtime with a different slot value. This avoids seeding concrete genesis code
that the existing `GenesisAccount` input cannot represent in FinalChain's native
snapshot. An all-transfer period can stay on the native path after an empty
system plan, so it is insufficient evidence of the new executor composition.
System-transaction facts must come from the chosen fixture's real bridge/pillar
configuration; an empty plan is an observed planner result, not an adapter
shortcut. Rewards likewise need verified native execution or a real
configuration that proves the selected effects neutral.

The bounded fixture uses no validators, configured yield zero, transaction gas
price zero, and no activated corrections. Magnolia and Cornus are active from
zero; Aspen part two, Cacti and redelegation fixes are beyond the two fixture
periods. Each period supplies a real DAG fact containing its transaction hashes,
no certificate votes, and a block gas limit of 1,000,000. The bridge account is
actually absent and neither period is a pillar period. These conditions must be
asserted by the adapter, not inferred from an empty rewards response.

FinalChain computes the actual `FinalChainEvmRewardsRequest.distribution_stats`;
the adapter encodes those bytes with the existing rewards-input codec. Commit
preparation independently runs the existing reward/native projection kernels,
including the configured-yield-zero path, before authorizing persistence. A
neutral proposed rewards root is valid only after that check. The full prior
native storage catalog remains present even when no native row changes. This
fixture does not establish nonzero rewards or native execution parity.

## Exact reports and persistence

The application validates the concrete projection and independently replays
native and reward effects before authorizing persistence. The adapter needs
per-transaction intermediate roots, exact touched account/raw projections,
native invocation facts, the complete native storage catalog and post-reward
state/provenance. A transaction execution report alone cannot authorize commit.

Reuse the public StateAPI encoders in `concrete_state_projection` for the
seven-field execution transaction, six-field result and ordered rewards-input
list. Signed wire RLP is not the StateAPI transaction encoding. Existing fake
session-test projection helpers do not establish valid application integration:
they use synthetic roots and omit facts needed by the full validator.

At concrete commit, require the exact accepted intent and verify the durable
application pending-publication marker exists while the published head still
identifies the prior generation. Commit concrete rows, descriptor/provenance
and marker transitions with the required persistence ordering; report the
observed descriptor to the existing application pipeline. Only that pipeline
publishes the application generation.

Drop all handles and reopen both independent databases. Run
`recover_final_chain_application_state` and execute a second period through the
same entry point. Check exact headers/hashes/receipts, account/slot/code bytes,
state roots and paired descriptors, with no unacknowledged pending publication.

## Remaining implementation dependencies

- Compatible incremental trie writer and durable concrete lifecycle.
- Journal continuation across transactions, including ordinary/raw cached views.
- Exact transaction/reward projections and native catalog serialization.
- Native/reward adapter using existing Rust kernels.
- Pinned reference roots/receipts and required targeted storage bridge validation.

Full differential replay, fault injection and operational gates remain subject
to the repository's explicit approval requirements. Production routing is not
part of this test composition.
