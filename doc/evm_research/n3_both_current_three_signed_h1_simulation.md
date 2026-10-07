# Canonical two-signer count3 H1 simulation

This profile follows the fresh Astra two-signer H1 source contract. It uses
unchanged bounded runtime70399811f. Separate new producer/harness/corpus/test
preserve every earlier support file, corpus, test and runtime predicate.

Caller C derives from key31 repeated32:
77952ce83ca3cad9f7adcfabeda85bd2f1f52008. Common owner O derives independently
from keyA1 repeated32:5d5c99edf529335160ff180fa141dd4967fc00d2. Both are distinct
from literal validators31/32. True Go genesis uses C5000/O3000 before staking,
four1000 principal pairs, both owner metadata fields O, then C3000/O1000/native
escrow4000 and maxsupply8000. Explicit config has min100/maxstake1000000,
eligibility100/step10/delay1/yield0/blocksperyear1; lockingDPOS2/Cornus3/disabled
Cacti7; fix/Magnolia/Ficus/Cornus/Aspen1 at0, Aspen2/Cacti/Soleirolia off.

Two canonical EIP155 chain666 nonce0 envelopes are signed, decoded/reencoded and
independently recovered before VM requests are derived. Real sameH1 caller300
then owner100 transfers31->32 have gas101912/101848 and individual event300/100.
There is one BeginBlock, EndBlock and Commit. StateTransition returns transaction
log vectors; unlike the separately accepted live Main frame producer these are
not cumulative. The first new export incorrectly sliced the second transaction
using the old Main assumption; failed output and both full first observations
remain in artifacts. The corrected export copies each actual transaction vector.
No legacy tests or producer behavior changed to make this pass.

Selected graph facts after owner prefix, EndBlock and complete H1 commit match
exactly on both pins. Both current node1 values arec28003, all heads and four
cursors are1, both old0 rows absent, C700/1300 and O900/1100, totals1600/2400 and
principal4000. Both memberships remain[31,32]; reward indices/pools zero.
Accounts C/O eachnonce1/balances3000/1000; native nonce1/balance4000. Complete
committed H1 has108 physical rows and actual Go root
f83e5d8d0b72a9e1e272dc0a739650609030ba821e2549db9be4ee095b6e6b63.
Before commit, copied cache entries including tombstones shadow immutable genesis;
after commit only complete H1 reader is used. No observer populates live cache.

Actual sameH1 DryRunner fullcaller700 repeats identically, with totalgas101912,
empty output/errors and one event700. Supplied nonce2^512 becomes private effective
nonce2; stored Cnonce1 remains unchanged. Native invocation period1 is separate
from the EVM next-context convention. All committed accounts/native facts/raw/
complete rows/configuration/root remain unchanged.

The Rust test independently builds public history with post-debit C3000/O1000,
all four genesis pairs and dynamic O metadata. Both recovered canonical envelopes
are ordered in one PeriodData and one public finalize_block; each receipt gas/log
matches Go. It reopens without refinalizing. Public current validator totals,
pairs/order/reward results and accounts match; the delayed stake API correctly
returns genesis2000 each. Complete persisted receipt RLP and full public validator
metadata bytes remain equal after every attempt and across reopen, together with
public owner header, nonces/balances and semantic pairs. Semantic state is not
hydrated from exported physical bytes, and synthetic Rust root equality is not
claimed.

An independent complete108-row Go owner supplies concrete accounts/storage/code
and actual identity/root. Every exported account RLP/nonces/balances/code and
selected native raw value is checked. Target authentication measures29 distinct
keys on every repetition. Each key gets corruption and reader failure followed
immediately by fresh success. Across two owner opens and two concrete reader
opens,472 fresh sessions include232 faults/232 successful fresh retries and8 base
successes. Every attempt drops exactly one owned session, preserves supplied wide
nonce, header/receipts/metadata/public semantic owner and all concrete physical
rows. Owner O retains nonce1, balance1000 and900/1100 pairs. Quote80000 and the
exact Go totalgas/output/event remain separate assertions.

Required checks are fresh dualpin record/reproduction/Python compilation, targeted
and affected EVM package tests, all-target check/Clippy, serial rewrite-validate-fast,
whitespace and independent final frozen Sol medium review. All first commands,
outputs and exit codes are retained under both-current-three-h1-* in run3 artifacts.
No runtime or bridge bytes change; accepted70399811f ONbuild12/all15 remains
separately inherited. Estimation/H2trace/generalcounts/rewards/realwindow root
parity, production routing and broad differential gates remain open. No supplied
data/upstream C++/storage owner change, fallback, protocol change or push.
