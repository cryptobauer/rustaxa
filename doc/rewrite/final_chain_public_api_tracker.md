# FinalChain Public API Rewrite Tracker

Source of truth: `libraries/core_libs/consensus/include/final_chain/final_chain.hpp`

Goal: keep `final_chain::FinalChain` public interface stable while incrementally moving implementation behind an additive shim and Rust-backed components.

## Current Batch Status

- Batch 1: additive shim scaffold plus Rust-backed chain index reads: `lastBlockNumber`, `blockNumber`, `blockHash`.
- Batch 1 also removed `FinalChainOld` behavior fallback from shim methods: every unimplemented public method now throws in Rust shim mode.
- Batch 2+: migrate remaining selected read/index APIs to Rust-backed implementations.

## Legend

- `[x]` Implemented in FinalChain shim through Rust-backed logic
- `[T]` Declared in the shim and explicitly throws; no `FinalChainOld` fallback
- `[ ]` Not yet routed through Rust-backed FinalChain logic
- `[~]` Public API but mostly composition/cache helper over other primitives
- `[!]` High-risk API (state transition, pruning, or cross-module side effects)

## External Usage Snapshot

Heavy external usage exists across:

- RPC / GraphQL (`eth_*`, log filters, debug tracing)
- PBFT / consensus flow (`finalize`, DPoS eligibility checks)
- integration tests (`tests/final_chain_test.cpp`, `tests/full_node_test.cpp`, `tests/rpc_test.cpp`)

Most frequently used callsites in workspace scans:

- `lastBlockNumber`, `getBalance`, `transactionLocation`, `getAccount`, `transactionReceipt`, `blockHash`, `transactions`

## Public API Tracker

### Public Event and Lifecycle Surface

- `[x]` `block_finalized_` subscriber
- `[x]` `block_applying_` subscriber
- `[x]` constructor `FinalChain(const std::shared_ptr<DbStorage>&, const taraxa::FullNodeConfig&, const addr_t&)`
- `[x]` destructor `~FinalChain()`

### Lifecycle and Finalization

- `[T][!]` `stop()`
- `[T]` `delegationDelay() const`
- `[T][!]` `finalize(PeriodData&&, std::vector<h256>&&, uint32_t, std::shared_ptr<DagBlock>&&)`
- `[T][!]` `finalize_(...)`
- `[T]` `waitForFinalized()`
- `[T][!]` `prune(EthBlockNumber)`

### Chain Index and Header/Hash Queries

- `[T]` `blockHeader(std::optional<EthBlockNumber>) const`
- `[x]` `lastBlockNumber() const`
- `[x]` `blockNumber(h256 const&) const`
- `[x]` `blockHash(std::optional<EthBlockNumber>) const`
- `[T]` `finalChainHash(EthBlockNumber) const`

### Transaction and Receipt Surface

- `[T]` `transactionHashes(std::optional<EthBlockNumber>) const`
- `[T]` `transactions(std::optional<EthBlockNumber>) const`
- `[T]` `transactionLocation(h256 const&) const`
- `[T]` `transactionReceipt(EthBlockNumber, uint64_t, std::optional<trx_hash_t>) const`
- `[T][~]` `transaction(EthBlockNumber, uint32_t) const`
- `[T]` `transactionCount(std::optional<EthBlockNumber>) const`
- `[T]` `blockReceipts(std::optional<EthBlockNumber>) const`

### Logs and Bloom Query

- `[T]` `withBlockBloom(LogBloom const&, EthBlockNumber, EthBlockNumber) const`

### State Query / EVM Read APIs

- `[T][!]` `updateStateConfig(state_api::Config const&)`
- `[T]` `getAccount(addr_t const&, std::optional<EthBlockNumber>) const`
- `[T]` `getAccountStorage(addr_t const&, u256 const&, std::optional<EthBlockNumber>) const`
- `[T]` `getCode(addr_t const&, std::optional<EthBlockNumber>) const`
- `[T][!]` `call(state_api::EVMTransaction const&, std::optional<EthBlockNumber>) const`
- `[T][!]` `trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>, EthBlockNumber, std::optional<state_api::Tracing>) const`

### DPoS and Bridge Query APIs

- `[T]` `dposEligibleTotalVoteCount(EthBlockNumber) const`
- `[T]` `dposEligibleVoteCount(EthBlockNumber, addr_t const&) const`
- `[T]` `dposIsEligible(EthBlockNumber, addr_t const&) const`
- `[T]` `dposGetVrfKey(EthBlockNumber, addr_t const&) const`
- `[T]` `dposValidatorsTotalStakes(EthBlockNumber) const`
- `[T]` `dposTotalAmountDelegated(EthBlockNumber) const`
- `[T]` `dposValidatorsEligibleVoteCounts(EthBlockNumber) const`
- `[T]` `dposYield(EthBlockNumber) const`
- `[T]` `dposTotalSupply(EthBlockNumber) const`
- `[T]` `getBridgeRoot(EthBlockNumber) const`
- `[T]` `getBridgeEpoch(EthBlockNumber) const`
- `[T][~]` `getBalance(addr_t const&) const`

## Current FinalChain Storage Touchpoints (for migration planning)

From `final_chain.cpp`, FinalChain currently depends on:

- final-chain columns:
  - `final_chain_meta`
  - `final_chain_blk_by_number`
  - `final_chain_blk_hash_by_number`
  - `final_chain_blk_number_by_hash`
  - `final_chain_receipt_by_period`
  - `final_chain_receipt_by_trx_hash`
  - `final_chain_log_blooms_index`
- status counters:
  - `StatusDbField::ExecutedBlkCount`
  - `StatusDbField::ExecutedTrxCount`
- additional `DbStorage` helpers:
  - `getPeriodData`, `getPeriodTransactions`, `getPeriodSystemTransactionsHashes`
  - `getTransactionLocation`, `getTransactionCount`, `getBlockReceipts`, `getPbftBlock`
  - batch writes via `insert/remove/commitWriteBatch`
  - maintenance paths `createSnapshot`, `compactColumn`

## Proposed Work Batches

1. **Batch 1**: shim scaffold + Rust-backed chain index reads (`lastBlockNumber`, `blockNumber`, `blockHash`) (done)
2. **Batch 2**: remaining read/index APIs (block header, transaction/receipt count and location paths)
3. **Batch 3**: transaction/receipt/log query helpers and bloom search parity
4. **Batch 4**: finalize/write path (`appendBlock`, counters, index writes)
5. **Batch 5**: StateAPI/DPoS bridge-heavy APIs (only after clear Rust/EVM integration strategy)
