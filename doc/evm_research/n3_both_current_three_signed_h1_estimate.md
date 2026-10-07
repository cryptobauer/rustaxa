# Two-signer count3 H1 gas estimation

This settled derivative uses the accepted[H1 simulation](n3_both_current_three_signed_h1_simulation.md)
at85f846521 and unchanged runtime70399811f. Separate producer/harness/corpus/test
preserve all earlier data and support. No runtime, search algorithm, bridge,
storage owner, upstream C++ or production behavior change is included.

Each pinned Go run rebuilds true two-signer H1, then runs all probes over that
committed seed: Ckey31/OkeyA1 are independently
recovered, real signednonce0 caller300 then owner100, one EndBlock/Commit. The
complete108-row H1/configuration/root exactly matches accepted simulation.
Both signer nonces are1; C/O balances3000/1000/native4000, pairsC700/1300 and
O900/1100, both currentcount3 nodes/allcursors1/old0absent. Explicit config is
unchanged, includingDPOS2/Cornus3/disabledCacti7 and Aspen1=0/Aspen2off.

New dual-pin DryRunner output measures8 probes:6 for gascap200000 and one each
at101912 and101911. The unchanged C++ search body hash
7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759
uses these exact callbacks, requires all transcript calls consumed, and returns
104977, minimum101912 and below-minimum `out of gas`. Successful probes use
101912 totalgas/event700; below minimum reports21912 gas and no event. Repeated
probe outcomes match and committed fullH1 before/after is unchanged. C++ search
execution is reference tooling; upstream source is not changed.

Rust runs the same public canonical two-transaction finalized history and opens
it again, using the independent complete108-row Go concrete reader. Across two
owner opens, two reader opens and two repeats,24 estimate calls consume64 actual
probes in exact gas order. Every probe owns a fresh simulation journal/session,
compares status/error/gas/output/logs/effective nonce to actual Go and leaves the
original supplied wide nonce unchanged. Every probe checks public pairs/order/
nonces/balances, owner header, both persisted receipt RLPs, full public validator
metadata and all concrete physical rows. The persisted public receipt/metadata
identity also matches across reopen. Each search result/error and consumed count
matches C++ exactly. Go root equality is only within its complete identity;
synthetic Rust/Go root parity is not claimed.

The first targeted compile needed a qualified sibling caller helper; its output
is retained and the corrected target passes. Required checks are dual-pin record/
reproduction, Python compilation, targeted and EVM package tests, check/Clippy,
serial rewrite-validate-fast, whitespace and independent frozen Sol medium review.
Capture prefix is both-current-three-h1-estimate in run3 artifacts. Unchanged
70399811f runtime ONbridgebuild12/all15 gates remain separately inherited.
H2 trace/general counts/rewards/realwindow root parity and production routing
remain open. No fallback, protocol/supplied-data change, broad differential or push.
