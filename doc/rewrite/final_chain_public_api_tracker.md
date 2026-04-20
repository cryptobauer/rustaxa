# FinalChain Public API Rewrite Tracker

Source of truth: `libraries/core_libs/consensus/include/final_chain/final_chain.hpp`

Goal: keep `final_chain::FinalChain` public interface stable while incrementally moving implementation behind an additive shim and Rust-backed components.

## Current Batch Status

- Batch 1 (this change): additive shim scaffold + passthrough wrappers, no behavior change.
- Batch 2+: migrate selected read/index APIs to Rust-backed implementations.

## Legend

- `[x]` Implemented in FinalChain shim (currently passthrough to `FinalChainOld`)
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

- `[x][!]` `stop()`
- `[x]` `delegationDelay() const`
- `[x][!]` `finalize(PeriodData&&, std::vector<h256>&&, uint32_t, std::shared_ptr<DagBlock>&&)`
- `[x][!]` `finalize_(...)`
- `[x]` `waitForFinalized()`
- `[x][!]` `prune(EthBlockNumber)`

### Chain Index and Header/Hash Queries

- `[x]` `blockHeader(std::optional<EthBlockNumber>) const`
- `[x]` `lastBlockNumber() const`
- `[x]` `blockNumber(h256 const&) const`
- `[x]` `blockHash(std::optional<EthBlockNumber>) const`
- `[x]` `finalChainHash(EthBlockNumber) const`

### Transaction and Receipt Surface

- `[x]` `transactionHashes(std::optional<EthBlockNumber>) const`
- `[x]` `transactions(std::optional<EthBlockNumber>) const`
- `[x]` `transactionLocation(h256 const&) const`
- `[x]` `transactionReceipt(EthBlockNumber, uint64_t, std::optional<trx_hash_t>) const`
- `[x][~]` `transaction(EthBlockNumber, uint32_t) const`
- `[x]` `transactionCount(std::optional<EthBlockNumber>) const`
- `[x]` `blockReceipts(std::optional<EthBlockNumber>) const`

### Logs and Bloom Query

- `[x]` `withBlockBloom(LogBloom const&, EthBlockNumber, EthBlockNumber) const`

### State Query / EVM Read APIs

- `[x][!]` `updateStateConfig(state_api::Config const&)`
- `[x]` `getAccount(addr_t const&, std::optional<EthBlockNumber>) const`
- `[x]` `getAccountStorage(addr_t const&, u256 const&, std::optional<EthBlockNumber>) const`
- `[x]` `getCode(addr_t const&, std::optional<EthBlockNumber>) const`
- `[x][!]` `call(state_api::EVMTransaction const&, std::optional<EthBlockNumber>) const`
- `[x][!]` `trace(std::vector<state_api::EVMTransaction>, std::vector<state_api::EVMTransaction>, EthBlockNumber, std::optional<state_api::Tracing>) const`

### DPoS and Bridge Query APIs

- `[x]` `dposEligibleTotalVoteCount(EthBlockNumber) const`
- `[x]` `dposEligibleVoteCount(EthBlockNumber, addr_t const&) const`
- `[x]` `dposIsEligible(EthBlockNumber, addr_t const&) const`
- `[x]` `dposGetVrfKey(EthBlockNumber, addr_t const&) const`
- `[x]` `dposValidatorsTotalStakes(EthBlockNumber) const`
- `[x]` `dposTotalAmountDelegated(EthBlockNumber) const`
- `[x]` `dposValidatorsEligibleVoteCounts(EthBlockNumber) const`
- `[x]` `dposYield(EthBlockNumber) const`
- `[x]` `dposTotalSupply(EthBlockNumber) const`
- `[x]` `getBridgeRoot(EthBlockNumber) const`
- `[x]` `getBridgeEpoch(EthBlockNumber) const`
- `[x][~]` `getBalance(addr_t const&) const`

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

1. **Batch 1**: shim scaffold + passthrough wrappers (done)
2. **Batch 2**: read/index APIs (block/header/hash/meta/receipt lookup paths)
3. **Batch 3**: transaction/receipt/log query helpers and bloom search parity
4. **Batch 4**: finalize/write path (`appendBlock`, counters, index writes)
5. **Batch 5**: StateAPI/DPoS bridge-heavy APIs (only after clear Rust/EVM integration strategy)
