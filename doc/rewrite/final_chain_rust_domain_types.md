# FinalChain Rust Domain Types Backlog

Date: 2026-04-20

This document tracks C++ FinalChain-related domain types that should be introduced on the Rust side over time.

## Usage

For each type:

- `Status`: `todo` | `in_progress` | `done`
- `Priority`: `P0` | `P1` | `P2`
- `Owner`: team or contributor
- `Notes`: encoding, compatibility, or dependency constraints

## P0 Types (needed for core FinalChain read/write migration)

### FinalChain data models (`final_chain/data.hpp`)

- `BlockHeaderData` — `todo`, `P0`
  - Notes: DB payload format compatibility (`serializeForDB`) and RLP fields must match C++.
- `BlockHeader` — `todo`, `P0`
  - Notes: requires exact hash/rlp compatibility with C++ `ethereumRlp()` and header hash derivation.
- `NewBlock` — `todo`, `P0`
- `FinalizationResult` — `todo`, `P0`
- `BlocksBlooms` (`std::array<LogBloom, c_bloomIndexSize>`) — `todo`, `P0`

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

### Example C: getting RLP representation efficiently

Goal: expose RLP bytes without repeated re-encoding on hot paths.

```rust
pub struct BlockHeader {
    fields: BlockHeaderFields,
    rlp_cache: Option<Vec<u8>>,
}

impl BlockHeader {
    pub fn rlp_bytes(&mut self) -> &[u8] {
        if self.rlp_cache.is_none() {
            self.rlp_cache = Some(encode_header_rlp(&self.fields));
        }
        self.rlp_cache.as_deref().unwrap()
    }

    pub fn from_rlp(bytes: &[u8]) -> Result<Self, DecodeError> {
        let fields = decode_header_fields(bytes)?;
        Ok(Self {
            fields,
            rlp_cache: Some(bytes.to_vec()),
        })
    }
}
```

Notes:
- If the object originates from canonical RLP, preserve that canonical byte form.
- If mutation occurs, invalidate cache and re-encode lazily.

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
