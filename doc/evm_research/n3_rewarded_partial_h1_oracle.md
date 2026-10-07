# Canonical reward-only same-H1 redelegation oracle

This experiment measures actual committed P1 followed by Go DryRunner at the
same height. Both pinned implementations and uninstrumented controls match.
It establishes a finite historical simulation reference. Rust simulation,
root parity and production routing need separate acceptance.

Known fixture keys derive caller77952ce83ca3cad9f7adcfabeda85bd2f1f52008 and
owner5d5c99edf529335160ff180fa141dd4967fc00d2. There are no ordinary or principal
transactions in P1 and no nonce overlay. Genesis gives each caller/owner pair
1000 stake on validators31/32; pre-custody ordinary balances are5000/3000.
Committed P1 balances are caller3000/nonce0, owner1000/nonce0 and nativeFE
4800/nonce1. The reward-only StateTransition uses source31 vote1/max1,
BlocksPerYear1, yield20, maximum supply8000, commission100/10000, author/DAG
caps0, minimum100, maximum1000000 and active fix/Magnolia/Ficus/Cornus/Aspen1.
Aspen2/Cacti are disabled. Actual reward800 gives source pool792 and commission8;
both validators retain stake2000, head0 and genesis count3 nodes. All four
pairs retain principal1000 and cursor0.

Complete committed identity is root
`0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b`,92
physical rows and52 selected native logical facts. The snapshot includes exact
configuration, accounts, current/delayed state, raw values and semantic facts.
No guessed raw values or block-cache copies replace the committed reader.

Literal Go EVM block and native period are both1. DryRunner replaces the supplied
nonce2^512 with current nonce+1, which is1. This corrects older EVM-context2 prose;
nonce and period are separate. Every first and repeated call resets the transient
view. Complete committed snapshots match before and after each attempt and at
final closeout.

| Case | Gas | Actual result |
| --- | --- | --- |
| Partial300 |101912|RewardsClaimed396 then Redelegated300 |
| Missing source |101912|Validator does not exist |
| Missing destination |101912|Validator does not exist |
| Amount1100 |101912|Insufficient delegation |
| Remainder50 |101912|Insufficient delegation |
| Same validator |101912|From and to validators are the same |
| Malformed selector-only |101272|ABI length insufficient0, require32 |
| Nonpayable malformed value1 |21272|Method is not payable |
| Native gas30000 |21912|out of gas |
| Intrinsic gas21000 |21000|intrinsic gas too low consensus error |

Success observes FE sub396 to4404 then caller add396 to3396. Nonpayable observes
attempted ordinary caller sub1 to2999 and FE add1 to4801; these transient transfers
are discarded. Other cases have no nonzero account arithmetic. All returns are
empty; errors have no logs. This oracle measures account arithmetic and committed
isolation. It does not measure an ordered raw write trace, mixed publication order
or a post-simulation committed root.

The harness archives public6c7e5338b22d5e596cc2365a88d1f94840e1ee1b and
localbb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418. Account observer targets are guarded
by source hash and exact mutation shape. Both actual uninstrumented controls
compare every semantic field, full snapshots, errors, gas, logs and effective
nonce after removing only the observer vector. Manifest hashes bind producer,
harness, three support sources, DryRunner, observer/helper guards, references,
Go version and all outputs. All eight diagnostic files are explicit.

Required checks are actual record/reproduce, Python compile, serial fast checks,
explicit tracked/untracked whitespace, frozen live hashes and independent Sol
medium review. First outputs, exits and routing remain in the run4 artifact record.
The source-only historical authority contract is reward-h1-contract.md in that
record. Runtime must require an explicit historical constructor origin and
current period1; request_id=None alone is insufficient authority.
