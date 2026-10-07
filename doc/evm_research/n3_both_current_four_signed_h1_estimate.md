# Count4 signed H1 gas estimation

This settled derivative uses accepted signed H1 simulationbe90ad91f and
unchanged runtimea16acef1e. New producer, harness, fixtures and Rust test preserve
all earlier corpora, shared supports, storage and public APIs. The exact new H1
root a8ea1ecbd354137b6ed5d14084c673eb9bd959979175f110b24cd20a56903b71 has108rows.
Three canonical transactions from two signers C0/O0/O1 finalize once. Caller
nonce1/owner2/native1, pairsC700/1300/O800/1200, totals1500/2500, bothnodes4,
heads/allfourcursors1 and old0absence are the accepted input authority.

Each pinned Go run rebuilds H1 once, then runs every actual DryRunner probe over
that committed seed. The cap200000 search uses six probes and returns104977;
minimum101912 succeeds, below101911 returns out of gas. Total eight probes are
repeated internally, and full committed snapshots before/after remain equal.
The Python harness compiles the unchanged C++ search body and requires every
callback gas and all transcript entries to match. Its algorithm SHA256 is
7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759.
This is measured search parity, not a new estimator policy.

Rust runs64 fresh simulations in24 searches across two public owner opens,
two independent complete concrete readers and two repetitions. Public H1 uses
all three canonical envelopes, persisted receipts and reopen without refinalize
or semantic raw injection. Each actual probe checks gas/error/output/logs,
identity and supplied wide nonce preservation with private effective nonce2/native
invocation1/EVMcontext1. Before/after every probe and reader disposal it compares
all108 Go concrete rows plus ALL public owner database Column rows, committed
height/header, accounts, pairs/order/rewards, all three receipt RLPs and full
public validator metadata. Public owner rows are also compared across reopen.
No Rust/Go root equality or production root acceptance is claimed.

The manifest binds exact pins, new producer/harness, accepted H1 input, unchanged
support, snapshot insertion/helper targets, all output/diagnostic hashes and C++
algorithm. Required gates are actual record/reproduction, Python compile,
targeted estimate test, EVM package/check/Clippy, serial fast/whitespace and
independent frozen Sol medium review. Runtime stays unchanged and its ONbridge
build12/all15 is inherited separately. H2 retained-excess trace remains a source
contract awaiting actual measurement. General counts/rewards, qualified real
window, recovery/integratedN4–N6 and production cutover remain open. No fallback,
supplied-data or upstream/storage-owner change, broad gate or push is included.

Run4 source correction: same-H1 Go DryRunner and public Rust simulation use literal
EVM block/native period1. Effective nonce2 in this older caller-nonce1 profile is
independent of block identity. The original prose bytes are frozen in run4
h1-context-before; previous checks, routing and historical report bytes remain
unchanged. The new source-only reward-h1-contract records exact source evidence.
