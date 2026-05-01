# FinalChain Rust Domain Types Backlog

Date: 2026-04-27

This document tracks Rust domain types needed for the FinalChain rewrite. It is not a 1:1 mirror of the C++ class layout.

The Rust type crate should group concepts by domain meaning and boundary role:

- `rustaxa_types::pbft`: PBFT block metadata and eventually PBFT block/vote domain objects.
- `rustaxa_types::final_chain`: final-chain block, receipt, log, and bloom domain objects.
- `rustaxa_types::dag`: DAG block and future DAG-related domain objects.
- `rustaxa_types::codec`: orthogonal encoders/decoders such as RLP. Codecs convert bytes into domain types and export domain types into compatibility/wire formats. Any hash/signature helpers needed for legacy codec compatibility should remain private codec implementation details.

Compatibility adapters may keep legacy names in function names when they explicitly produce C++-compatible wire formats, but core structs should use semantic Rust names rather than C++ class names.

## Usage

For each type:

- `Status`: `todo` | `in_progress` | `done`
- `Priority`: `P0` | `P1` | `P2`
- `Owner`: team or contributor
- `Notes`: encoding, compatibility, or dependency constraints

## P0 Types (needed for core FinalChain read/write migration)

### Final-chain block models

- `StoredFinalChainBlockHeader` — `done`, `P0`
  - Notes: storage payload decoded from `final_chain_blk_by_number`; replaces direct use of C++ `BlockHeaderData` naming in Rust.
- `FinalChainBlockHeader` — `done`, `P0`
  - Notes: materialized block header with PBFT metadata applied.
- `BlockHeaderContext` — `done`, `P0`
  - Notes: construction context that combines a computed hash, PBFT metadata, configured gas limit, and genesis timestamp without baking node config into the domain type.
- `FinalChainBlockHeaderBuilder` — `done`, `P0`
  - Notes: domain builder for constructing a materialized header from a stored header, PBFT metadata, configuration values, and an already-computed hash.
- `NewBlock` — `todo`, `P0`
- `FinalizationResult` — `todo`, `P0`
- `BlocksBlooms` (`std::array<LogBloom, c_bloomIndexSize>`) — `todo`, `P0`

### PBFT metadata used by final-chain headers

- `PbftBlockMetadata` — `done`, `P0`
  - Notes: decoded from signed PBFT block RLP when final-chain header materialization needs proposer, period, timestamp, and extra data.
  - Boundary: lives in `rustaxa_types::pbft`, not `final_chain`, because PBFT metadata will be reused outside the final-chain shim.

### Compatibility adapters

- `LegacyBlockHeaderRlpInput` / `LegacyBlockHeaderRlp` — `done`, `P0`
  - Notes: adapter types for the current C++ shim boundary; `TryFrom<LegacyBlockHeaderRlpInput>` composes stored-header decoding, PBFT metadata decoding, compatibility hash calculation, and legacy `BlockHeader` RLP export.

### Transaction/receipt and location (`transaction/receipt.hpp`)

- `LogEntry` — `todo`, `P0`
- `TransactionReceipt` — `todo`, `P0`
- `TransactionLocation` — `todo`, `P0`
  - Notes: preserve optional `is_system` encoding semantics from C++ RLP shape.

### State API payloads (`final_chain/state_api_data.hpp`)

- `EVMBlock` — `todo`, `P0`
- `EVMTransaction` — `todo`, `P0`
- `LogRecord` — `todo`, `P0`
- `ExecutionResult` — `todo`, `P0`
- `TransactionsExecutionResult` — `todo`, `P0`
- `RewardsDistributionResult` — `todo`, `P0`
- `Account` — `todo`, `P0`
- `StateDescriptor` — `todo`, `P0`
- `Tracing` — `todo`, `P0`

## P1 Types (DPoS and validator query surface)

- `ValidatorStake` — `todo`, `P1`
- `ValidatorVoteCount` — `todo`, `P1`

## P2 / Later

- bridge-specific call payload wrappers used by `getBridgeRoot` and `getBridgeEpoch`
- optional trace/debug-specific JSON adapters (only if FinalChain trace path is migrated)

## Type Introduction Order (recommended)

1. Receipt/location + block-header data model types
2. Core state API execution result types
3. Finalization result aggregation types
4. DPoS validator query types

## Compatibility Constraints

- RLP and hash behavior must remain byte-compatible with C++ outputs.
- Any Rust type used through CXX bridge should prefer stable plain-data transport wrappers where possible.
- Do not couple initial type introduction to finalize-path migration; start with read/query parity first.

## Rust Type Design Policy (throughput + correctness)

These rules define how to introduce and use Rust domain types for FinalChain while preserving high data throughput.

### 1. Keep strong typing at API boundaries

- Use domain types to prevent wrong-type information passing between APIs.
- Use `#[repr(transparent)]` newtypes for semantically distinct scalar values:
  - `PbftPeriod(u64)`, `BlockNumber(u64)`, `TrxPosition(u32)`, `Gas(u64)`, `BlockHash([u8; 32])`, etc.
- Keep fields private and expose validated constructors / `TryFrom` for invariants.
- Prefer explicit conversions over implicit casts.

Why: this keeps safety at compile time with near-zero runtime overhead.

### 2. Separate domain and wire/storage representations

- Keep two forms where needed:
  - `domain`: validated typed structures used by business logic.
  - `wire`/`storage`: canonical bytes used for DB and bridge transport (`Vec<u8>` / borrowed slices).
- Convert at clear boundaries:
  - decode bytes -> domain when logic needs typed access
  - encode domain -> bytes at persistence/export boundaries

Why: avoids decoding and re-encoding overhead on byte-pass-through paths.

Current applied pattern:

- `StoredFinalChainBlockHeader` represents the compact DB payload.
- `PbftBlockMetadata` represents the signed PBFT block facts needed by other domains.
- `FinalChainBlockHeader` represents the materialized logical header.
- `FinalChainBlockHeaderBuilder` constructs the logical header after all boundary-specific inputs have been decoded or computed.
- `rustaxa_types::codec::rlp::final_chain` decodes the stored header payload, decodes PBFT metadata, computes the C++/Ethereum-compatible header hash, and exports the legacy `BlockHeader` RLP for existing C++ callers.
- `rustaxa_types::codec::rlp::dag` decodes DAG block RLP into `DagBlock`; `DagBlock` itself does not depend on RLP.
- RLP conversion entry points use explicit wrapper types with `From`/`TryFrom`, for example `StoredBlockHeaderRlp`, `SignedPbftBlockRlp`, `DagBlockRlp`, and `LegacyBlockHeaderRlp`.

### 2a. Keep codecs orthogonal

- Domain modules should not expose `from_rlp`, `to_rlp`, or `legacy_rlp` methods by default.
- Put encoding-specific code under `rustaxa_types::codec::<format>::<domain>`.
- Prefer `TryFrom<CodecSpecificInput>` for decoding and `From<&DomainType>` for encoding.
- Avoid `TryFrom<&[u8]> for DomainType`; raw bytes do not identify the encoding or payload shape.
- If multiple encodings emerge, add sibling codec modules instead of expanding core domain structs.
- Codec modules may use compatibility-oriented names when the target format is explicitly legacy or bridge-specific.

Why: domain code remains usable by consensus/storage logic without carrying serialization details, and future formats can be added without reshaping the core types.

### 2b. Use builders at composition boundaries

- Prefer small domain builders when constructing an aggregate needs multiple independently sourced facts.
- Builders should take domain values, not raw encoded bytes.
- Encoding-specific builders or helper functions can live in codec modules and should decode/compute boundary-specific data before invoking domain builders.

Current example:

- `FinalChainBlockHeaderBuilder` takes a `StoredFinalChainBlockHeader`, optional `PbftBlockMetadata`, gas/timestamp config, and a computed hash.
- `LegacyBlockHeaderRlp::try_from(LegacyBlockHeaderRlpInput::new(...))` is the RLP compatibility builder for the current shim boundary.

### 3. Optimize for contiguous data and predictable control flow

- In hot paths, prefer contiguous layouts (`Vec<T>`, arrays, plain structs).
- Avoid pointer-heavy object graphs and unnecessary heap allocations.
- Avoid trait-object dispatch in critical paths unless measured and justified.
- Prefer straightforward code paths over heavily abstracted polymorphic layers in throughput-sensitive logic.

Why: reduces cache misses, branch churn, and allocator pressure.

### 4. Decode late, encode early, and cache canonical bytes when useful

- Decode only when fields are actually needed.
- For frequently persisted or hashed entities, keep canonical encoded bytes available.
- If the type was created from canonical RLP, preserve those bytes to avoid re-encoding.

Why: minimizes repeated work in high-frequency storage operations.

### 5. Keep FFI transport plain and stable

- CXX bridge surface should use stable, plain-data-compatible wrappers and byte buffers.
- Convert bridge payloads to domain types immediately after crossing into Rust logic.
- Keep bridge-specific wrappers separate from core domain model types.

Why: stable interop contracts and fewer accidental ABI/representation pitfalls.

### 6. Error handling and invariants

- Failed decode / invalid payload must return explicit errors.
- Do not silently coerce malformed data.
- Validation should happen once at type-construction boundaries; internal logic should assume valid state.

Why: preserves correctness guarantees without scattering defensive checks everywhere.

## Example Patterns

### Example A: zero-cost domain scalar wrappers

```rust
#[repr(transparent)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct PbftPeriod(pub u64);

#[repr(transparent)]
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct TrxPosition(pub u32);
```

Usage: function signatures take `PbftPeriod` or `TrxPosition` instead of raw integers to prevent accidental argument swaps.

### Example B: safe composite domain model

```rust
#[derive(Clone, Debug)]
pub struct TransactionLocation {
    period: PbftPeriod,
    position: TrxPosition,
    is_system: bool,
}
```

Rule: keep fields private and construct through validated constructors when invariants apply.

### Example C: codec-specific RLP wrappers

Goal: keep RLP as an orthogonal codec while still using idiomatic conversions.

```rust
pub struct StoredBlockHeaderRlp<'a>(&'a [u8]);

impl<'a> StoredBlockHeaderRlp<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self(bytes)
    }
}

pub struct LegacyBlockHeaderRlp(Vec<u8>);

impl LegacyBlockHeaderRlp {
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

pub struct LegacyBlockHeaderRlpInput<'a> {
    stored_header: StoredBlockHeaderRlp<'a>,
    signed_pbft_block: Option<SignedPbftBlockRlp<'a>>,
    block_gas_limit: u64,
    genesis_timestamp: u64,
}

impl<'a> LegacyBlockHeaderRlpInput<'a> {
    pub fn new(
        stored_header: StoredBlockHeaderRlp<'a>,
        block_gas_limit: u64,
        genesis_timestamp: u64,
    ) -> Self {
        Self {
            stored_header,
            signed_pbft_block: None,
            block_gas_limit,
            genesis_timestamp,
        }
    }
}

impl TryFrom<StoredBlockHeaderRlp<'_>> for StoredFinalChainBlockHeader {
    type Error = anyhow::Error;

    fn try_from(value: StoredBlockHeaderRlp<'_>) -> Result<Self, Self::Error> {
        decode_stored_header_rlp(value.0)
    }
}

impl From<&FinalChainBlockHeader> for LegacyBlockHeaderRlp {
    fn from(header: &FinalChainBlockHeader) -> Self {
        encode_legacy_header_rlp(header)
    }
}
```

Notes:
- Implement `From`, not `Into`; Rust derives `Into` from `From`.
- Implement `TryFrom`, not `TryInto`; Rust derives `TryInto` from `TryFrom`.
- Avoid `TryFrom<&[u8]>` where multiple byte formats could exist.
- Keep wrapper fields private unless direct field access is the intended stable API.

## When not to add more abstraction

- Do not wrap every transient local in hot loops with additional heap-backed abstractions.
- Do not replace simple byte pass-through flows with eager full decoding unless required by logic.
- Do not introduce deep trait hierarchies for storage-critical read/write paths without benchmark evidence.

## Rollout Guidance for this repository

### Phase 1 (P0 scalar safety first)

- Introduce scalar newtypes for period/number/index/hash/address classes used by FinalChain APIs.
- Update function signatures at module boundaries first; keep internals incremental.

### Phase 2 (P0 composite domain models)

- Introduce `TransactionLocation`, receipt/log models, and block-header data models with strict decode/encode compatibility.
- Keep canonical byte compatibility tests against C++ fixtures as gate criteria.

### Phase 3 (broader state API surfaces)

- Add remaining state API domain models once read/query parity is stable.
- Keep bridge payload wrappers separate from internal domain types.

## Required checklist for each new domain type

- Invariant definition documented.
- Canonical encoding shape documented (RLP or other).
- C++ compatibility expectation stated (hash/rlp/db payload behavior).
- Allocation/throughput impact considered for hot paths.
- Boundary conversion points identified (DB decode/encode, CXX bridge).
- Tests include byte-compatibility and malformed-input behavior.
