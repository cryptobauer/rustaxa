# FinalChain Shim Design (Batch 1 Rust Index Reads)

Date: 2026-04-20

## Objective

Introduce an additive shim seam for `final_chain::FinalChain` equivalent to the existing `DbStorage` shim pattern:

- no changes to public API shape
- no destructive C++ refactor
- clear migration seam for future Rust-backed FinalChain logic
- no fallback to `FinalChainOld` for public shim methods that are not implemented yet

## Build and Activation Model

New CMake option:

- `RUSTAXA_ENABLE_FINAL_CHAIN` (default `OFF`)

When `RUSTAXA_ENABLE_FINAL_CHAIN=ON`:

1. `core_libs` gets a **BEFORE** include path:
   - `libraries/core_libs/final_chain_shim/include`
2. Legacy implementation file:
   - `libraries/core_libs/consensus/src/final_chain/final_chain.cpp`
   is compiled with:
   - `COMPILE_DEFINITIONS "FinalChain=FinalChainOld"`
3. Shim source is added:
   - `libraries/core_libs/final_chain_shim/src/final_chain_shim.cpp`

## Header Overlay Pattern

`libraries/core_libs/final_chain_shim/include/final_chain/final_chain.hpp` overlays the legacy include path:

- imports legacy header as `FinalChainOld`
- conditionally includes shim facade (`final_chain_shim.hpp`) when `FinalChain` macro is not preset

This mirrors the storage shim strategy and allows:

- legacy code to compile as `FinalChainOld`
- all external call sites to continue using `final_chain::FinalChain`

## Shim Class Strategy (Batch 1)

`final_chain::FinalChain` is defined as:

- `class FinalChain : public FinalChainOld`

All public methods are explicitly redeclared so external call sites go through the shim:

- establishes migration seams per API
- keeps callsites stable
- makes per-method migration to Rust decision-local in future batches
- routes implemented methods to Rust
- throws `DbException` for unimplemented methods instead of falling back to `FinalChainOld`

Batch 1 Rust-backed methods:

- `lastBlockNumber()`
- `blockNumber(h256 const&)`
- `blockHash(std::optional<EthBlockNumber>)`

Batch 1 Rust data sources:

- `final_chain_meta` key `DBMetaKeys::LAST_NUMBER`
- `final_chain_blk_number_by_hash`
- `final_chain_blk_hash_by_number`

## Current Dependency and Dataflow Notes

FinalChain currently spans:

- DbStorage final-chain indexes and receipts
- period/transaction helper APIs
- state execution via `StateAPI` (`taraxa-evm`)
- rewards aggregation side effects
- caches and async finalize execution path

Direct final-chain column usage in `final_chain.cpp`:

- `final_chain_meta`
- `final_chain_blk_by_number`
- `final_chain_blk_hash_by_number`
- `final_chain_blk_number_by_hash`
- `final_chain_receipt_by_period`
- `final_chain_receipt_by_trx_hash`
- `final_chain_log_blooms_index`

## Out-of-Scope in Batch 1

- no finalization path migration
- no StateAPI/evm integration changes
- no schema/column changes
- no block header, transaction, receipt, bloom, DPoS, bridge, prune, or lifecycle behavior beyond explicit unimplemented throws

## Migration Guidance for Future Batches

1. start with read/index APIs where Rust storage coverage already exists
2. preserve cache behavior and API semantics while swapping backend reads
3. defer `finalize_` write path until read/query parity and tests are stable
4. treat `prune`, `snapshot`, and state transition boundaries as explicit high-risk milestones
