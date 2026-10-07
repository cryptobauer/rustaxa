# Positive-reward partial frame oracle

This separate experiment measures seven fresh EVM.Main frames after the accepted
[reward P1 snapshot](n2_rewarded_partial_redelegate_oracle.md). It does not claim
StateTransition publication, historical/root parity or production acceptance.

The seed is the complete actual committed P1 inventory of91 physical rows and
account facts: callerD1 balance3000/nonce0, ownerA1 balance1000/nonce0, nativeFE
balance4800/nonce1. Each of50 selected logical rows is resolved forward with
keccak(address+keccak(slot)), latest physical period<=1, and compared with the
accepted seed. Globals4/5/6/7 are resolved the same way from physical rows; no literal
values or reverse hashes are used. The full P1 fixture hash remains authority.
The52-key view (50 seed entries,48 unique plus four globals) includes actual minted_tokens(global6) and actual absent
total_supply(global7). Every measured setup/Run backend read must be selected.
Only code fields/code bytes on existingD1 change for the declared wrapper.
Direct sends D1->FE; nested sends A1->D1, so D1 is the native caller. No fabricated
funding, principal or nonce is added. Native config is the exact seed chain_config;
EVM uses params.TestChainConfig, period2, timestamp/gasprice0, blockgas1000000,
transactiongas200000. Chain666 is the declared Rust comparison context.

| Case | Transaction gas | Actual result |
| --- | --- | --- |
| Direct |101912|396 payout,12 raw writes,2 logs |
| Nested CALL |102678|same native effects,wrapper returns1 |
| STATICCALL |102676|same native effects,wrapper returns1 |
| Parent REVERT |102678|12 raw writes survive; payout accounts/logs roll back |
|300 then invalid701, parent REVERT |183431|second Insufficient delegation; first raw survives, accounts/logs roll back |
| Nonpayable value1 |29378|Method is not payable,quote0; no native effects; outer1 transfer stays |
| Native gas79999 |22678|quote80000,Run not called; no native effects |

Successful native calls match the exact accepted12 raw writes, two account
arithmetic observations (FE sub396->4404,D1 add396->3396), and RewardsClaimed
then Redelegated logs. Attempted per-call balances are separate from the entire
transaction arithmetic vector: nonpayable includes two outer and two reverted
child value-transfer operations, while its native method has no account effect.
Final accounts and raw rows are measured after the parent completes. Raw snapshots
include selected seed keys and actual dirty keys; they are not a complete trie.
No mixed raw/account publication-order claim follows from these separate vectors.

The harness archives public6c7e5338b22d5e596cc2365a88d1f94840e1ee1b and
localbb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418. It copies observer inputs without
new backend reads and records per-call quote/funding/caller/depth/input/value,
setup/Run reads, raw writes, logs and nonzero account arithmetic. Actual account
observer targets are hash/shape guarded; read/write observation uses the unchanged
support backend wrapper. Uninstrumented native/backend/account controls compare
all semantic outputs, gas, logs, final accounts and raw state. Both pins and both
controls match byte-for-byte. Manifest hashes cover references, Go version,
seed/support/exporter/harness/balance-helper targets and all outputs/diagnostics.
All eight stdout/stderr diagnostic files are explicit even when empty.

Required checks: record and exact reproduce, Python compile, serial
rewrite-validate-fast, explicit tracked/untracked whitespace and frozen independent
Sol medium review. Full first outputs/exits and routing remain in the run4 artifact
records. Public Rust frame composition is a later acceptance step; the adapter
commit alone does not prove these parent rollback outcomes.

Review corrected a seed gap: initial50-key capture omitted present P1 minted_tokens
and treated that setup read as absent. Its full output/freeze and finding are
retained in run4. Forward-resolving globals6/7 fixes authority without guessed
values. Corrected capture/reproduction and final review bind the52-key profile.
