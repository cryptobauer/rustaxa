# Funded H1 positive claim-all estimate oracle

Actual Go DryRunner probes at literal block1 run on both pinned archives over
the same complete canonical92-row P1 as the accepted H1 claim-all oracle.
Each probe has a fresh repeat and unchanged committed physical P1. Explicit
caps are200000,111272 and111273. Original512-bit nonces are copied into each
request; DryRunner's effective transaction nonce is1 and post-call caller
nonce is2. Zero value, zero gas price, explicit caller and exact0b83a727 selector
keep RPC defaults outside this measurement.

The actual200000cap transcript is200000/155636/133454/122363/116817.
All five probes succeed with gasused111272, twelve ordered raw writes, two
balance changes and one396reward event. Native gas supplied is178728/134364/
112182/101091/95545; each quote is90000. Each boundary cap has one successful
probe:111272 supplies90000, and111273 supplies90001. Their effects, copied
post-call accounts and23watched raw after-rows match the separately measured
H1 success. Vectors are freshly observed for every probe and repeat.

The harness compiles the unchanged C++ search body from eth_estimateGas and
checks every requested cap against the actual Go transcript. It consumes all
probes. Measured estimates are116817/111272/111273 with5/1/1probes. The C++
algorithm hash remains7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759.
Both pinned Go transcripts and native-observer-disabled controls match.

Copy-only gas quote and post-call-state taps run in observed and control
executions, with the accepted original per-pin EVM source/unique-expression
and DryRunner source guards. Native raw-write and balance observers run only
in observed executions. The control comparison removes only observation_witness;
it retains every actual output/log/quote, nonce, copied account/raw state,
committed P1 and repetition assertion. The fixed23key union is independent of
observed writes. The shared H1 harness/support files remain unchanged and their
exact bytes are included in provenance. All shell subprocesses, including the
extracted C++ compiler and binary, run through rtk proxy.

Required checks are actual record/reproduce, Python compilation, serial fast
and scoped whitespace, then independent frozen review and a local preparation
commit. This record is oracle evidence only. Rust public estimation composition
is a separate derivative over the already accepted H1 runtime. Underfunded and
negative estimates, general claim-all, publication, root parity, production,
recovery and N4–N6 remain excluded.
