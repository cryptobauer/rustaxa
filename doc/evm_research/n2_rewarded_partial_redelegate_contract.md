# N2 positive-reward partial redelegation: source contract

Decision: one bounded source-valid synthetic sequence is available. Actual positive payout is NOT measured or accepted by this report. Next work must first measure the pinned Go sequence below. No production, historical capture, upstream, storage-owner, supplied-data, or wider routing acceptance follows. Existing zero-reward profiles stay unchanged.

Routing: requested Astra source-contract work; confirmed runtime gpt-6-astra, medium, session 01a1171d-c0c5-7761-8f45-9a3afac96b51. Startup quota observation 2026-10-07T16:06:32.289Z: 90% remaining, allow_start. Root baseline supplied: 99%; root stop: 89%; reset 1791948695. No billing inference. No routing failure. Only this report was written; no producer, test, build, gate, source edit, commit, push, or delegation was performed.

## Exact proposed Go sequence

Use existingCurrentConfig(0x31), based on custodyConfig(true,true,false): validators 31 and 32; common caller C and owner O each delegate 1000 to each; genesis pre-custody balances C=5000/O=3000; minimum deposit 100; maximum stake 1,000,000; commission=100/10000 (1%); delegation delay=1. Keep Magnolia/Ficus/Cornus at zero, Aspen part one at zero, Aspen part two and Cacti disabled, FixRedelegateBlockNum=0. Keep MaxSupply=8000. Only change YieldPercentage from 0 to 20. BlocksPerYear already equals 1; MaxBlockAuthorReward and DagProposersReward already equal zero. No additional change to these fields is necessary. Aspen part two is disabled, so MaxSupply does not cap this fixed-yield reward period.

Create one live StateTransition through nativeSimulationTransition(newNativeSimulationDB(), &cfg), which initializes genesis. P1: call BeginBlock with vm.BlockInfo{Author:31, GasLimit:1000000, Difficulty:new(big.Int)}; assert BlockNumber()==1. No transaction is needed. Call DistributeRewards with RewardsStats{BlockAuthor:31, BlocksPerYear:1, ValidatorsStats:{31:{VoteWeight:1, FeesRewards:new(big.Int)}}, TotalVotesWeight:1, MaxVotesWeight:1}; DagBlocksCount and TotalDagBlocksCount are zero. This supplies coherent synthetic reward statistics directly to the actual reward API; it does not claim PBFT-derived production statistics. Then EndBlock(); Commit().

P2: BeginBlock with the same BlockInfo; assert BlockNumber()==2. ExecuteTransaction from C to the DPoS address, nonce=0, gas=200000, gas price=0, value=0, calldata reDelegate(address,address,uint256)(31,32,300). Observe live state, then EndBlock(); Commit(); inspect committed P2 state. Do not distribute another reward in P2. The existing accrued cancellation producer supplies the source precedent for DistributeRewards/EndBlock/Commit followed by a next-period operation; its undelegation is not part of this sequence.

## Predictions, not measurements

Total delegated principal is 4000. P1 mint is floor(4000*20/(100*1))=800. One voter receives all 800; commission is 8 and source delegator pool is 792. Destination pools remain zero. At P2, source index increment is floor(792*1000000/2000)=396000. C's source payout is floor(396000*1000/1000000)=396, strictly positive. Thus no rounding threshold or missing reward input blocks this candidate. These calculations must be checked against both actual pinned executions.

| Item | After genesis | After P1 commit | After P2 successful partial |
|---|---:|---:|---:|
| C / O / native balances | 3000 / 1000 / 4000 | 3000 / 1000 / 4800 | 3396 / 1000 / 4404 |
| C principal 31 / 32 | 1000 / 1000 | 1000 / 1000 | 700 / 1300 |
| Validator stake 31 / 32 | 2000 / 2000 | 2000 / 2000 | 1700 / 2300 |
| Source pool / commission | 0 / 0 | 792 / 8 | 0 / 8 |

Global principal stays 4000. P1 reward distribution does not move head/cursor checkpoints. Each genesis node has three references (validator, C, O). At P2 each endpoint's old node is written count 2, then count 1; its current node is created count 2. Source current index is 396000; destination index is zero. C and validator heads move to period 2; O's pair and cursor remain at genesis. O retains an unpaid source entitlement of 396 through the old cursor and new head. Membership ordering is unchanged. Source commission 8 is not paid by redelegation. P1 also updates the Aspen-part-one minted-token accumulator; capture its exact row rather than synthesizing it.

Go ordering: preflight validator existence, destination maximum and source delegation/minimum checks precede reward transfer. Source checkpoint/head decrement and source cursor decrement precede native debit and caller credit; RewardsClaimed(C,31,396) precedes source principal changes. Source writes finish before destination checkpoint/cursor and principal writes. Final log is Redelegated(C,31,32,300); destination reward log is absent because its reward is zero. EndBlock persists deferred aggregates. Exact raw read/write and account-effect order still needs measurement.

Normal preflight contract failures must return their reference error without reward payout or native logs. Outer transaction nonce/gas semantics remain the transaction owner's responsibility; the successful zero-price transaction predicts C nonce 1. Missing/inconsistent account or raw authority is a hard adapter error, not a new contract failure. Test rollback after payout with an enclosing frame revert, and test injected account/raw publication failure: discard account effects, raw writes, logs, and cloned DPoS state together. Distinguish invocation rollback from the enclosing transaction's retained nonce.

## Existing Rust owners and minimum extension

final_chain.rs already has apply_dpos_redelegate -> remove_dpos_delegation_stake -> apply_dpos_delegator_reward_claim_with_cursor, followed by apply_dpos_redelegate_destination. checkpoint_validator_reward_per_stake consumes the pool; dpos_reward_graph.rs owns checkpoint creation, reference counts, cursor movement, exact index arithmetic and reward arithmetic. Reuse these units. No new reward algorithm is needed.

DposAccountPort already exposes account/subtract_balance/add_balance. StagedDposAccountPort::from_state reads authoritative current facts through public FinalChainNativeStateRead::account and records ordered expected-value BalanceReplace effects. transfer_dpos_contract_balance checks native funds, debits native, then credits caller. FinalChainNativeOutcome already carries account_mutations. Terminal finish_rewards is a separate existing owner for reward mint reconstruction; do not route redelegation through that terminal operation.

Current blockers are explicit: custody/redelegate.rs authenticate_reward_scope rejects nonzero pools, commission, indexes and cursor mirrors; invoke_redelegate constructs StagedDposAccountPort::new([]), rejects all account mutations; redelegate_result always returns an empty account list. The principal serializers already encode final reward nodes and pools and preserve decrement/write order, but this positive-reward shape has no accepted oracle evidence. Do not remove their checks or replace exact old raw values with fabricated rows.

Minimal proposal: add one separate profile for this two-validator partial, old head/cursors at genesis, current nodes absent, positive source pool and commission, zero destination rewards, complete two-member ledgers with retained O. Authenticate exact pools, heads, C/O cursor nodes, counts, reward mirrors and raw rows. Bind P1 committed state through existing state owners. Use from_state only when the kernel reaches payout, preserving early failure order. Return its account effects with existing raw effects and logs; publish only after all authentication and serialization succeeds. Reuse source-then-destination serializers after oracle proof; extend serialization only if the measured ordered trace requires it. No general positive-reward graph admission or account cache independent of the current journal.

First required measurement: both pinned Go archives must produce P1 mint, committed P1 rows/accounts, P2 pre-state pending reward, exact native account debit/caller credit, logs, ordered raw reads/writes, post-EndBlock rows and committed P2 state. This is one successful partial first; do not substitute hand-written raw reward rows. Then verify the Rust kernel's predicted 396 transfer and counts; staged adapter exact raw/effect/log parity; public native transaction and frame-revert behavior; missing/mismatched authoritative account/raw rollback; normal preflight failures without account access. Add the narrow Rust-enabled checks required by the slice workflow only after implementation is authorized.

Remaining unknowns: no actual new oracle output, serialized P1 supply row, measured P2 trace, Rust adapter parity, or runtime acceptance. Source predictions do not establish these results.

## Source pins and hashes

Authoritative reference.py pins public=6c7e5338b22d5e596cc2365a88d1f94840e1ee1b and local=bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418. Accepted runtime supplied by root: a16acef1e. Inspected checkout HEAD: 05e92e685bc7b74842c52bb27f07d806def7a5c1. Hashes below use exact byte spans including line endings. Go reward-function differences between pins are whitespace only; redelegate and transfer spans are identical.

```text
public taraxa/state/contracts/dpos/precompiled/dpos_contract.go:885-1040 3b5c90fc6f2e5abf81d27222033fa7b73c0ee696913bf9a0da41876e7e452a47
public taraxa/state/contracts/dpos/precompiled/dpos_contract.go:1341-1494 f5a720ef488e71e2f41633834a326e6ded6bc3bb552d6a777434bd7a451c3111
public taraxa/state/contracts/dpos/precompiled/dpos_contract.go:2101-2110 5b2bf6378d6713de33a1f28b123738c7dfc0c26d81e9fe0f95a82cf1b6593ff4
public taraxa/state/state_transition/state_transition.go:106-113 20bbfe11b210b6896fb2645023f94e28978173f935679bfbfc2d0d76385ff31d
public taraxa/state/state_transition/state_transition.go:130-140 5d44b952a91965a12eb17a4b9836a59d60e392c0a1054ddecb16a798a7c38be5
public taraxa/state/state_transition/state_transition.go:142-150 8feb4acb40922c26c17bfbb8181714b8ccbb3ec110d19ee90cdfd3b665e94b65
public taraxa/state/state_transition/state_transition.go:160-173 0515cd6832f0b7c1df829c7b1fab2560cc98559d99c8b1b56387cd87696b24bb
local taraxa/state/contracts/dpos/precompiled/dpos_contract.go:897-1051 ecb138e3c7cc9afa72730c7872c93cb9cb0c0d520ee976aa3dbe6903a9f8e07c
local taraxa/state/contracts/dpos/precompiled/dpos_contract.go:1352-1505 f5a720ef488e71e2f41633834a326e6ded6bc3bb552d6a777434bd7a451c3111
local taraxa/state/contracts/dpos/precompiled/dpos_contract.go:2112-2121 5b2bf6378d6713de33a1f28b123738c7dfc0c26d81e9fe0f95a82cf1b6593ff4
local taraxa/state/state_transition/state_transition.go:131-139 b5d160b5a45a839ae3bee564551d6bdee5c80c80a26f4edf0b974102ea68c294
local taraxa/state/state_transition/state_transition.go:249-259 5d44b952a91965a12eb17a4b9836a59d60e392c0a1054ddecb16a798a7c38be5
local taraxa/state/state_transition/state_transition.go:261-269 8feb4acb40922c26c17bfbb8181714b8ccbb3ec110d19ee90cdfd3b665e94b65
local taraxa/state/state_transition/state_transition.go:288-301 0515cd6832f0b7c1df829c7b1fab2560cc98559d99c8b1b56387cd87696b24bb
checkout rust/crates/rustaxa-consensus/src/final_chain.rs:5095-5162 21db8ce4c30c2cd17d23edcec651fa86e32caa371fef63e7be6810fc842e1b13
checkout rust/crates/rustaxa-consensus/src/final_chain.rs:7677-7770 bf847ea182704d4ec53064e28906c2e8cca4416dd942d624454abaa00bfab08d
checkout rust/crates/rustaxa-consensus/src/final_chain.rs:8982-9136 ca3be26283d8ac0a3c07d036dec37e4b8f5e02a92d54c62e8589c21e4232991b
checkout rust/crates/rustaxa-consensus/src/final_chain.rs:9770-9930 3edb5aff9f722ec71d335ddd12a084c7c9623599c201e786d1f479b4bb3b7656
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate.rs:450-493 aeccfb04e3584c2b314f08ebdb47f123ba0913d7e5d1cda156213af22c11fd5a
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate.rs:713-810 90963ec51ed4178d1afdb149f9a438f4d0d722630b51447a3f2ff0d1d23bb0ef
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate.rs:887-920 a72e33bc7e5daadd3b9557f8f8d145ffb0b4fd8e0c6b053992430ed53fb89386
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/custody.rs:227-400 af4173a956f9c647f41cf0476ed3624c1f5392db4f9ca9cfb482b6ede1172129
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/account.rs:76-228 a3606f66deff02dd447923c490468d925875613d2bd86c6dcc6742a1caa0f211
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session/account.rs:277-290 877d461a3ccbf8e8e2911b350aea821ea7f297a23711ec425cd7ebae33b8b420
checkout rust/crates/rustaxa-consensus/src/final_chain/native_session.rs:139-163 b4a392b8082593b3de5d662cd647aac982f4405386ba51bad02b49c762ec7351
checkout rust/crates/rustaxa-consensus/src/dpos_reward_graph.rs:358-417 779ccd6daa2371166a57655a5965fc28f339d0a7a5af7fd0478a55c979c15007
checkout rust/crates/rustaxa-consensus/src/dpos_reward_graph.rs:713-775 ffdeb291ce97f6802f6af2ccdb3c12cdb2082ea5c558ed762fb8d222f80967f8
checkout rust/crates/rustaxa-consensus/src/dpos_reward_graph.rs:907-938 855f83941076a507907415323bf6bfb2b398391c24f7c2eccd0d83d2e2f435d3
checkout experiments/evm_feasibility/native_redelegate_existing_current_reference.go:46-63 697ecfd2faaffad1296f09a0c3526887fbffb94cbb6a23abc27dd7cd3b54fd2f
checkout experiments/evm_feasibility/native_v1_custody_reference.go:111-150 fcaa3ad429243cdef0eb083d0ad17c03c6da7ea8f57747cd1a8c5f503ba5ebe1
checkout experiments/evm_feasibility/native_v1_custody_reference.go:211-221 f2efc854d931390d6fe1e2b470d5e621be80d8f2d94ce12ad33c2509f9c33ba7
checkout experiments/evm_feasibility/native_cancel_reward_reference.go:29-106 1fa105bdc82e30f1180b51b86308e96e139220f7f05895c87b398e224a3ca601
```
