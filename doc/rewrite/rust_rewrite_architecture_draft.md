# Rust Rewrite Architecture Draft

Date: 2026-04-21

Purpose: define an idiomatic Rust architecture for the rewrite that preserves throughput, keeps module boundaries clear, and prevents wrong-type/wrong-layer API usage.

## Goals

- Keep high data throughput on storage and FinalChain paths.
- Keep APIs type-safe so incorrect values are hard to pass by mistake.
- Keep modules unit-testable without requiring full node runtime wiring.
- Preserve incremental migration from current C++ aggregation model.

## Core Recommendation

Use aggregation in Rust, but express it as explicit struct composition plus trait-based ports.

In practice:

- `FinalChain` aggregates only the dependencies it needs.
- Dependencies are narrow traits (ports), not a broad "global context."
- Construction happens through explicit constructors (`new(...)`), not service locators.
- Hot paths prefer static dispatch (`S: Trait`) unless runtime polymorphism is required.

This keeps C++-style testability while aligning with Rust idioms.

## Why This Fits Rust

Aggregation in C++ is usually "pass db/finalchain/... around."
In Rust, the idiomatic equivalent is:

- ownership/borrowing to make data flow explicit,
- traits to decouple behavior from concrete backends,
- small interfaces to limit accidental cross-module coupling.

## Architecture Pattern

### 1. Domain modules define ports

Each domain module (for example `final_chain`) defines the capabilities it needs:

- `FinalChainReadStore`
- `FinalChainWriteStore`
- `FinalChainClock`
- `FinalChainTxSource`

Keep ports task-oriented and narrow.

### 2. Infrastructure modules implement ports

Storage adapters implement the domain ports:

- RocksDB-backed implementation
- CXX bridge-backed implementation during migration
- in-memory/fake test implementation

### 3. Runtime wiring assembles concrete graph

A composition root (`node runtime`, `app bootstrap`, or equivalent) wires concrete implementations into domain services.

Domain modules should not know how dependencies are instantiated.

## Dispatch Strategy

### Prefer static dispatch in throughput-critical paths

Use generics for core loops:

```rust
pub struct FinalChain<S> {
    store: S,
}

impl<S: FinalChainReadStore + FinalChainWriteStore> FinalChain<S> {
    pub fn apply_block(&mut self, block: &Block) -> Result<()> {
        // ...
        Ok(())
    }
}
```

Pros:
- zero virtual dispatch overhead
- strong compile-time contracts

Tradeoff:
- more monomorphized code

### Use trait objects at wiring boundaries

Use `Arc<dyn Trait + Send + Sync>` when runtime flexibility matters (plugins, shared service registries, late binding).

Pros:
- simpler runtime composition
- single concrete type in service graph

Tradeoff:
- vtable indirection (typically small, but avoid in hottest loops if unnecessary)

## Access Pattern Guidelines

### Avoid "everything bag" context structs

Do not pass a giant mutable `Context` carrying db, config, networking, consensus, etc.

Instead:
- define per-domain dependency structs with only required capabilities
- pass focused references per method

### Keep dependency direction one-way

- Domain -> port traits
- Infra -> domain trait impls
- Runtime -> wire modules together

Avoid circular dependencies between domain modules.

## Type Safety Strategy

Use domain newtypes at module boundaries:

- `PbftPeriod`, `BlockNumber`, `TrxPosition`, `Gas`, `BlockHash`

Keep these wrappers `#[repr(transparent)]` and cheap to copy for scalar classes.

This prevents swapped-argument bugs while keeping runtime overhead near zero.

## Data Throughput Strategy

- Keep DB payloads as bytes in storage layer until typed fields are needed.
- Decode late and only once per operation.
- Reuse/carry canonical encoded bytes when repeatedly hashing/writing.
- Prefer contiguous data (`Vec`, arrays, plain structs) and avoid pointer-heavy object graphs.
- Avoid over-abstracted trait chains in storage-critical loops unless benchmarked.

## Unit Testing Model

This model keeps C++-style aggregation testability:

- test domain services against fake/in-memory trait impls
- run adapter tests against real RocksDB/bridge integration
- keep conformance tests for C++ vs Rust parity at boundary points

Suggested split:

- domain unit tests: no DB process, no CXX bridge
- adapter tests: storage encoding/decoding and persistence behavior
- conformance tests: C++ transcript vs Rust transcript parity

## Example: FinalChain Accessing Storage

```rust
pub trait FinalChainReadStore {
    fn get_block_header(&self, n: BlockNumber) -> Result<Option<BlockHeader>>;
    fn get_receipt(&self, tx: TxHash) -> Result<Option<TransactionReceipt>>;
}

pub trait FinalChainWriteStore {
    fn put_block_header(&self, n: BlockNumber, header: &BlockHeader) -> Result<()>;
    fn put_receipt(&self, tx: TxHash, receipt: &TransactionReceipt) -> Result<()>;
}

pub struct FinalChain<S> {
    store: S,
}

impl<S> FinalChain<S>
where
    S: FinalChainReadStore + FinalChainWriteStore,
{
    pub fn import_block(&self, block: &Block) -> Result<()> {
        self.store.put_block_header(block.number(), block.header())?;
        Ok(())
    }
}
```

Notes:

- `FinalChain` depends on behavior, not RocksDB directly.
- store implementation can be swapped for tests without changing domain code.

## Migration Plan for Current Rewrite

### Phase 1: Introduce ports without moving logic

- Define narrow read/write traits in Rust domain crates.
- Implement traits by delegating to current storage bridge/repository functions.
- Keep external behavior unchanged.

### Phase 2: Move call sites to typed boundaries

- Replace raw primitive-heavy signatures with domain newtypes.
- Keep conversion at boundary adapters.

### Phase 3: Pull logic behind domain services

- Move selected FinalChain read/write workflows into Rust services.
- Keep C++ shim surface stable while internals migrate.

### Phase 4: Optimize hot paths based on measurements

- Benchmark generic vs dyn dispatch on critical operations.
- Collapse abstractions only where profiling justifies changes.

## Decision Rules

Use this quick decision table:

- Need maximum throughput and call is in hot loop: use generic aggregation (`S: Trait`).
- Need runtime pluggability or shared global service instance: use `Arc<dyn Trait>`.
- API crosses module boundary with semantically distinct numbers/ids: use newtypes.
- Data is only forwarded to DB/network: keep as bytes and avoid eager decode.
- Logic needs validated fields: decode into domain type at that boundary.

## Anti-Patterns to Avoid

- Global mutable singleton/service locator.
- Passing broad context objects through many layers.
- Traits with very large method surfaces ("god traits").
- Repeated encode/decode in tight paths.
- Conflating FFI transport structs with internal domain models.

## Outcome Target

If followed, this architecture should deliver:

- C++-equivalent testability through explicit aggregation seams,
- Rust-idiomatic module boundaries and type safety,
- high-throughput storage/final-chain behavior with controlled abstraction cost.
