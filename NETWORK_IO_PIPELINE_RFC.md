# RFC: Network I/O Pipeline

## Status

Draft.

## Summary

Rustaxa should introduce a general network I/O pipeline between transport-facing network code and domain-facing
subsystems. Consensus is the highest-pressure consumer, but the boundary should cover all network input and output:
peer lifecycle, tarcap packets, transaction gossip, DAG and PBFT sync, pillar vote traffic, RPC and GraphQL network
views, and WebSocket subscription delivery.

The first goal is architectural clarity, not a wire-protocol change. Current tarcap packet formats, capability versions,
and peer compatibility rules should remain stable while packet handlers become thin transport adapters. They should
decode and encode packets, enforce transport-level peer rules, and then pass normalized data into a narrow pipeline.
Domain-specific state transitions should move behind explicit adapters that can initially call the existing C++ managers
and shim classes, then later be replaced by Rust implementations.

The pipeline should be developed independently from the ongoing consensus rewrite branch. It must not depend on current
`rustaxa-consensus` internals. Existing C++ consensus shims can be used as temporary adapters, but the core interface
should be data-first and Rust-ready.

## Motivation

The current network boundary is difficult to reason about because transport code directly owns substantial domain logic.
Tarcap packet handlers decode packets, update peer metadata, validate consensus or transaction data, call into managers,
decide sync behavior, emit outbound packets, update known-object caches, report malicious peers, and sometimes drive
gossip. This creates a pointer-heavy control flow where data moves through many shared objects instead of through a
clear pipeline.

That shape makes the consensus rewrite harder than it needs to be. Retiring the C++ consensus layer will be easier if
network has a small, stable interface to the rest of the node. The same principle applies beyond consensus: transaction
ingress, sync orchestration, peer state, RPC network views, and subscriptions also benefit from explicit data flow and
observable command output.

The intended direction is:

- network owns transport, peer connectivity, packet compatibility, and delivery mechanics;
- the pipeline owns normalized events, backpressure classification, and command routing;
- domain adapters own calls into consensus, transaction pool, sync, storage-backed views, and subscriptions;
- Rust implementations can replace adapters without changing the network transport surface.

## Current Network Boundary Observations

The current `Network` and tarcap implementation wires many domain managers directly into network construction. The
network layer receives `PbftManager`, `PbftChain`, `VoteManager`, `DagManager`, `TransactionManager`,
`SlashingManager`, `PillarChainManager`, `FinalChain`, and storage handles, then passes them into packet handlers.

`TaraxaCapability` owns capability versioning, peer state, packet queues, DDOS checks, status bootstrapping, and packet
dispatch. It also constructs all latest and compatibility packet handlers with direct manager pointers.

Packet handlers are not only wire adapters:

- DAG block handling decodes a packet, marks peer-known transactions and DAG blocks, checks duplicate DAG blocks,
  verifies the block through `DagManager`, inserts it into DAG state, and triggers DAG sync requests.
- Vote handling decodes a vote packet, reads current PBFT period and round, filters irrelevant votes, validates vote and
  optional block pairing, calls `VoteManager`, `SlashingManager`, and `PbftManager`, then triggers gossip.
- Transaction handling decodes transactions, marks known hashes, validates through `TransactionManager`, inserts into
  the pool, and applies suspicious-packet policy.
- Status handling validates network identity and genesis, promotes pending peers, updates peer DAG/PBFT state, and
  directly triggers PBFT or DAG sync requests.
- PBFT block bundle handling validates sync peer expectations, checks proposal periods and eligibility, and forwards
  proposed blocks to `PbftManager`.

Outbound network calls also expose domain-specific methods directly from `Network`, such as `gossipDagBlock`,
`gossipVote`, `gossipVotesBundle`, `gossipPillarBlockVote`, `requestPillarBlockVotesBundle`, and
`handleMaliciousSyncPeer`.

The non-consensus surface is broader than tarcap consensus traffic:

- transaction gossip is network I/O but belongs to transaction-pool policy;
- peer counts, sync state, packet queue state, and node status are consumed by RPC and GraphQL;
- WebSocket subscriptions emit heads, logs, DAG block, transaction, finalized DAG block, executed PBFT block, and pillar
  block notifications;
- periodic network tasks send transactions, send status, maintain boot nodes, log node stats, and process DDOS stats.

These concerns should remain supported, but they should not require every packet handler to know every domain manager.

## Goals

- Define a general network I/O boundary that covers all inbound and outbound network cases, not only consensus.
- Keep tarcap wire compatibility stable during the first migration.
- Make packet handlers mostly transport-facing: decode, encode, peer lookup, transport validation, queue accounting, and
  dispatch into the pipeline.
- Represent cross-boundary data as explicit event and command values instead of shared manager calls.
- Allow existing C++ managers and shim classes to act as temporary domain adapters.
- Design the interface so a future Rust pipeline can own data classification, ordering, batching, and command emission.
- Make backpressure, malicious-peer decisions, peer metadata updates, gossip, sync requests, and subscription emission
  observable as commands.
- Support phased migration, starting with low-risk read-only/status paths before moving transaction and consensus
  traffic.

## Non-Goals

- Do not change tarcap packet encodings or public protocol compatibility in the first wave.
- Do not depend on the current consensus rewrite implementation or current `rustaxa-consensus` internals.
- Do not remove legacy C++ managers while adapters still need them for behavior parity.
- Do not rewrite RPC, GraphQL, or subscription serving in the first wave.
- Do not move libp2p host/session ownership out of the network module.
- Do not introduce a broad service-locator context object that recreates the current pointer graph behind a new name.

## Network I/O Inventory

The pipeline should account for these current in/out cases.

### Inbound Transport Events

- peer connected;
- peer disconnected;
- initial status packet received;
- standard status packet received;
- transaction packet received;
- DAG block packet received;
- DAG sync request or response received;
- PBFT sync request or response received;
- PBFT vote packet received;
- PBFT votes bundle received;
- request for next votes bundle received;
- PBFT blocks bundle received;
- pillar vote received;
- pillar votes bundle received;
- request for pillar votes bundle received;
- packet rejected before domain dispatch due to transport, queue, handshake, or DDOS policy.

### Outbound Transport Commands

- send status to one peer;
- broadcast status to peers;
- gossip transactions;
- gossip DAG block with transactions;
- gossip PBFT vote with optional block;
- gossip PBFT votes bundle;
- gossip pillar vote;
- send sync request to one peer;
- send sync response to one peer;
- request pending DAG blocks;
- request PBFT next votes at period and round;
- request pillar block votes bundle;
- disconnect peer;
- mark peer malicious;
- update peer-known object cache;
- update peer chain, DAG, round, sync, and light-node metadata.

### Read and Notification Outputs

- network status snapshots for RPC and GraphQL;
- peer count, node count, queue pressure, sync status, and max-chain peer views;
- subscription notifications for heads, logs, DAG blocks, transactions, finalized DAG blocks, executed PBFT blocks, and
  pillar blocks.

## Proposed Architecture

Introduce a `NetworkIoPipeline` boundary with three sides:

- `NetworkIngress`: accepts normalized inbound events from transport handlers.
- `NetworkEgress`: accepts explicit outbound commands and executes them through the existing network module.
- `DomainAdapters`: consume domain-relevant events and return pipeline commands or domain outcomes.

Packet handlers should not call consensus, transaction, sync, or subscription services directly after migration. They
should convert raw packets into normalized events and pass them into `NetworkIngress`. The pipeline should call the
appropriate adapter and emit commands through `NetworkEgress`.

At first this can be implemented in C++ as a compatibility layer. The stable design target should be Rust-owned event
classification and command generation, with C++ shims only at boundaries where existing managers are still required.

### Ownership Model

Network remains responsible for:

- libp2p host and session ownership;
- tarcap capability versions and packet ids;
- peer lookup and pending-peer state needed for transport safety;
- RLP packet decode and encode at the compatibility boundary;
- packet queueing and transport DDOS limits;
- send, disconnect, and peer table operations;
- RPC, GraphQL, and WebSocket server lifecycles.

The pipeline becomes responsible for:

- normalized event definitions;
- domain routing;
- backpressure classification after transport admission;
- command ordering;
- command observability;
- adapter invocation;
- stable data contracts for future Rust implementation.

Domain adapters remain responsible for:

- consensus-specific validation and state transition while C++ consensus is still active;
- transaction verification and pool insertion;
- sync orchestration decisions;
- read-only network status projections;
- subscription payload generation or forwarding.

## Inbound Event Model

Events should be plain data with explicit source metadata. The source metadata should be available to every event without
embedding network implementation types into domain logic.

Minimum metadata:

- tarcap version;
- packet type;
- peer id;
- receive timestamp;
- encoded packet size;
- whether the peer has completed status handshake;
- relevant peer snapshot fields such as DAG level, PBFT chain size, PBFT period, PBFT round, sync status, light-node
  status, and known-object cache summary.

Initial event families:

- `PeerEvent`: connected, disconnected, promoted from pending to ready, marked malicious.
- `StatusEvent`: initial status, standard status, incompatible chain id, incompatible genesis, light-node history data.
- `TransactionEvent`: full transactions plus extra known hashes.
- `DagEvent`: DAG block with included transactions, DAG sync request, DAG sync response.
- `PbftEvent`: vote, vote bundle, proposed block bundle, PBFT sync request, PBFT sync response, next-votes request.
- `PillarEvent`: pillar vote, pillar votes bundle, pillar votes request.
- `RpcViewEvent`: network status snapshot requested or refreshed.
- `SubscriptionEvent`: domain notification ready to publish.

The first implementation can carry existing C++ objects inside compatibility payloads where necessary. The interface
should still expose payload purpose and ownership explicitly so Rust can later replace those payloads with canonical
bytes and Rust domain types.

## Outbound Command Model

Pipeline output should be a sequence of commands. Commands make side effects explicit and testable.

Initial command families:

- `PeerCommand`: promote peer, update peer metadata, update known-object cache, disconnect peer, mark malicious.
- `SendCommand`: send one encoded packet to one peer.
- `BroadcastCommand`: send packet data to selected peers, with rebroadcast and exclude-peer options.
- `SyncCommand`: request pending DAG blocks, request PBFT sync, request next votes, request pillar votes bundle.
- `DomainCommand`: call an adapter for consensus, transaction pool, sync, or subscription behavior during migration.
- `StatsCommand`: record packet processing stats and queue wait time.
- `SubscriptionCommand`: emit a WebSocket subscription payload.
- `ViewCommand`: refresh or expose network status data for RPC and GraphQL.

For parity migration, a command may call a C++ adapter immediately. The important design constraint is that the call is
visible as a command and can be replaced by Rust-owned logic later.

## Domain Adapter Model

Adapters should be narrow and named by domain responsibility.

`ConsensusNetworkAdapter` handles consensus-facing events that currently route to `PbftManager`, `PbftChain`,
`VoteManager`, `DagManager`, `SlashingManager`, `PillarChainManager`, and `FinalChain`. It should be treated as temporary
compatibility glue until Rust consensus owns the relevant behavior.

`TransactionPoolNetworkAdapter` handles transaction verification, insertion, known-transaction checks, pool overflow
policy, and transaction gossip source classification.

`SyncNetworkAdapter` handles DAG, PBFT, vote, and pillar sync decisions. It should eventually replace direct sync
decisions inside status and vote handlers.

`RpcNetworkView` exposes read-only status snapshots for RPC and GraphQL without giving RPC direct access to mutable peer
or packet-handler internals.

`SubscriptionSink` emits subscription payloads for existing WebSocket subscription types. It may initially forward to
the current subscription implementation.

Adapters must avoid becoming broad context objects. Each adapter should expose task-oriented methods that accept event
data and return outcomes or commands.

## RPC, GraphQL, and Subscription Integration

RPC and GraphQL should not be treated as consensus-specific consumers. They need a stable read model over network state:
peer count, node count, syncing state, packet queue pressure, max-chain peer, and node status details.

The RFC proposes a `NetworkStatusView` projection maintained from peer and packet events. Existing RPC and GraphQL code
can keep serving the same APIs while reading from this projection instead of reaching through packet handlers or mutable
peer state.

Subscriptions should be modeled as outbound notification commands. Domain code or adapters can emit typed subscription
events; the network layer remains responsible for WebSocket delivery and JSON-RPC subscription response formatting.

Initial subscription event types should match current behavior:

- new head;
- logs;
- DAG block;
- transaction;
- DAG block finalized;
- PBFT block executed;
- pillar block.

## Peer State, Backpressure, and DDOS Policy

Transport admission checks should stay close to network because they protect the host and packet queues. This includes
handshake readiness, queue size limits, packet processing time limits, and early disconnects for invalid transport
behavior.

After admission, domain-level suspicious behavior should become commands or adapter outcomes. Examples:

- malformed domain packet;
- incompatible chain id or genesis;
- duplicate vote;
- double vote proof;
- transaction pool overflow pressure;
- future PBFT vote that should trigger sync instead of validation;
- missing DAG transaction or tip that should trigger DAG sync.

This separation keeps transport safety local while making domain policy testable and replaceable.

## Rust Data Model Principles

The future Rust implementation should prefer:

- plain event and command structs;
- semantic newtypes for periods, rounds, levels, counts, hashes, and packet sizes;
- canonical encoded bytes preserved when repeated hashing or forwarding would otherwise re-encode data;
- late decode when domain logic does not need typed fields;
- explicit ownership for payloads crossing C++ and Rust;
- narrow traits at domain boundaries;
- static dispatch for hot paths where possible;
- trait objects only at wiring boundaries.

Rust-facing types should not mirror C++ pointer ownership. If an initial adapter must carry `shared_ptr` values for
compatibility, that should be marked as temporary and hidden behind compatibility payloads.

## Migration Plan

### Phase 1: RFC Only

Add this RFC and use it as the reference for future network boundary work.

### Phase 2: Interface Skeleton

Add guarded interface skeletons without changing behavior. Keep packet handlers calling existing logic until each path is
migrated. The skeleton should include event types, command types, ingress, egress, and adapter interfaces.

### Phase 3: Read-Only Network Views

Move low-risk read-only status projection first. RPC and GraphQL should read network status through a projection instead
of directly depending on handler internals where practical.

### Phase 4: Transaction Ingress and Gossip

Route transaction packets through the pipeline. Preserve current transaction verification and insertion through
`TransactionPoolNetworkAdapter`, but make peer-known updates, suspicious-packet outcomes, and outbound transaction gossip
visible as commands.

### Phase 5: Status and Sync Decisions

Route status packets and sync-trigger decisions through the pipeline. Preserve current behavior for initial status
validation, peer promotion, PBFT sync triggers, DAG sync triggers, and next-votes requests.

### Phase 6: Consensus Traffic

Route DAG block, PBFT vote, vote bundle, PBFT block bundle, and pillar vote paths through the pipeline. Use current C++
consensus shims and managers only behind `ConsensusNetworkAdapter` and `SyncNetworkAdapter`.

### Phase 7: Rust-Owned Pipeline

Move event classification, command generation, and selected adapter logic to Rust. C++ packet handlers should remain
transport wrappers. Replace C++ adapters domain by domain as Rust implementations become available.

## Validation Strategy

Markdown-only changes require no build.

For implementation phases:

- add unit tests for event normalization from each packet family;
- add command-generation tests for peer updates, gossip, sync requests, disconnects, and subscription emission;
- add parity tests proving old handler behavior and pipeline behavior produce equivalent manager calls or commands;
- add malformed packet and malicious-peer tests for transport and domain policy separation;
- run targeted network/tarcap C++ tests for changed handlers;
- run relevant Rust crate tests when Rust pipeline code is introduced;
- preserve existing test expectations unless product behavior intentionally changes.

Each migrated packet family should have a focused acceptance test before direct manager calls are removed from that
handler.

## Alternatives Considered

### Consensus-Only Boundary

A narrow consensus-only interface would help the immediate consensus rewrite, but it would leave transaction gossip,
status handling, sync orchestration, RPC views, and subscriptions in the current pointer-heavy shape. It would also risk
creating a second boundary later for non-consensus network behavior.

### Rewrite Packet Handlers Directly in Rust

Directly replacing packet handlers with Rust could reduce C++ surface area, but it couples transport compatibility,
libp2p integration, and domain migration too early. A pipeline boundary allows packet compatibility to stay stable while
domain behavior moves incrementally.

### Keep Existing Manager Injection and Add More Shims

Adding more shims around current manager calls preserves short-term behavior, but it does not create a data-driven
architecture. It keeps network behavior coupled to domain object graphs and makes future Rust ownership less clear.

### Event Bus Without Typed Commands

A generic event bus would reduce direct calls but would hide side effects and make parity testing harder. Typed commands
are preferable because they expose exactly what the network should do after an event is processed.

## Open Questions

- Should the first implementation skeleton live under `libraries/core_libs/network` as a C++ compatibility layer, or
  should the Rust crate and CXX bridge be introduced immediately with no behavior routed through it yet?
- Which packet family should be the first write-path migration after read-only network views: transactions, status/sync,
  or PBFT votes?
- Should peer-known caches remain fully network-owned, or should the pipeline own a normalized known-object model and
  command the network to update tarcap-specific caches?
- How much of subscription payload formatting should remain in the current WebSocket/RPC code versus moving into typed
  notification adapters?
- Should backpressure beyond transport queue admission be centralized in the pipeline, or should each domain adapter own
  its own pressure policy and return commands?
