# Rustaxa Storage TODOs

## Architecture
- [ ] Implement `Storage` struct with `Arc<rocksdb::DB>` and `WriteBatch` support.
- [ ] Implement C++ Shim (`DbStorage` facade) to forward calls to Rust.

## Repositories (Repository Pattern)

### 1. DagRepository
Handles DAG blocks and levels.
- [ ] `saveDagBlock`, `getDagBlock`, `dagBlockInDb`
- [ ] `getBlocksByLevel`, `getDagBlocksAtLevel`, `getLastBlocksLevel`
- [ ] `getNonfinalizedDagBlocks`
- [ ] `removeDagBlock`, `removeDagBlockBatch`
- [ ] `getDagBlockPeriod`
- [ ] `getProposalPeriodForDagLevel`, `saveProposalPeriodDagLevelsMap`

### 2. TransactionRepository
Handles transactions and receipts.
- [ ] `getTransaction`, `transactionInDb`, `transactionFinalized`
- [ ] `addTransactionToBatch`
- [ ] `getTransactionLocation`, `addTransactionLocationToBatch`
- [ ] `getBlockReceipts`, `getTransactionReceipt`
- [ ] `getSystemTransaction`, `addSystemTransactionToBatch`
- [ ] `getPeriodSystemTransactions`

### 3. PbftRepository
Handles PBFT blocks and chain state.
- [ ] `getPbftBlock`, `saveProposedPbftBlock`, `pbftBlockInDb`
- [ ] `getPeriodData`, `savePeriodData`
- [ ] `getPbftHead`, `savePbftHead`
- [ ] `getPeriodBlockHash`
- [ ] `getPbftMgrField`, `savePbftMgrField`

### 4. VoteRepository
Handles consensus votes.
- [ ] `saveOwnVerifiedVote`, `getOwnVerifiedVotes`
- [ ] `replaceTwoTPlusOneVotes`, `getAllTwoTPlusOneVotes`
- [ ] `getPeriodCertVotes`
- [ ] `getRewardVotes`, `saveExtraRewardVote`

### 5. PillarRepository
Handles Pillar Chain blocks and votes.
- [ ] `savePillarBlock`, `getPillarBlock`, `getLatestPillarBlock`
- [ ] `saveOwnPillarBlockVote`, `getOwnPillarBlockVote`
- [ ] `saveCurrentPillarBlockData`

### 6. ChainRepository
Handles system state and config.
- [ ] `getGenesisHash`, `setGenesisHash`
- [ ] `getStatusField`, `saveStatusField`
- [ ] `saveSortitionParamsChange`, `getParamsChangeForPeriod`
- [ ] `getDbVersions`, `updateDbVersions`
- [ ] `getPeriodLambda`, `savePeriodLambda`
