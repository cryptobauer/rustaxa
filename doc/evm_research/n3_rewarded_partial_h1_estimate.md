# Reward-only H1 gas estimation parity

This bounded derivative composes public historical reward simulation with the
existing estimator. Its input is the accepted canonical reward-only complete
P1 oracle: literal reader/EVM/nativeperiod1, actual caller nonce0 and effective
nonce1, root0f3680f4c7acd788e123aa95831050f369b605730dd71c7cd4fb6a1d1ae8c21b,
92 physical rows, mint800/pool792/commission8/native4800. It adds no ordinary or
principal P1 transaction and no configuration, principal or nonce overlay.

Both pinned Go DryRunner implementations execute12 request profiles: all ten
accepted success/business/ABI/nonpayable/native-gas/intrinsic-gas cases, plus
success-boundary caps101912 and101911. Every first/repeated probe owns a reset
transient view; complete committed state is checked after both and at closeout.
First probes match the accepted baseline after removing only its account observer
vector. The estimator producer does not instrument account or raw mutation.
Its unused shared main needs the hash-bound observer helper to compile, but
no hook is installed or activated in actual estimation.

Actual success transcript is200000,150956,126434,114173,108042,104977; each uses
101912 gas, pays396 and emits the same two logs. The unchanged extracted C++
eth_estimateGas search consumes all six probes and returns104977. Minimum cap
101912 returns101912 in one probe;101911 yields out of gas with21912 used.
Each of the other nine errors exits after its first actual probe with the exact
Go error, including intrinsic gas too low as a consensus failure. Total17 probes.

The C++ helper compiles the exact unchanged upstream search body; callbacks
check each requested cap and require complete transcript consumption. Algorithm
hash7136e3d4b31368406303db6b2b26a887adbf0793e484921980ef9ed3f1be7759 is bound
with full source hash. Manifest also binds reference pins, Go version, exact
DryRunner, producer/harness/support sources, accepted input, helper guard/source,
outputs and both explicit stderr files. Producer outputs on both pins match.

The public Rust test materializes the full Go concrete P1 with authenticated
complete-seed absence. Independently, public FinalChain reward finalization
creates semantic P1 and reopens its owner. Existing begin_native_simulation(1)
and SimulationNativeExecutionPort run each real estimator probe in a fresh
journal/session. The test compares exact requested caps, every Go output/error/
gas/log, effective nonce from the authoritative sender's next nonce, reader
identity and unchanged input. Rust estimator results and consumed counts match
compiled C++ for all12 profiles. Two owner states, two reader openings and two
search repeats produce136 real probes. Complete concrete rows and public owner
accounts/header/principal/stakes remain unchanged after every probe.

No production runtime changes are made. Existing H1/P2 guards, account staging
and ordered codecs retain accepted6fe7b2c39 bytes. Its ON12-job/all15 bridge and
full consensus checks are retained explicitly. Required derivative checks:
actual record/reproduce, Python compile, targeted/fullEVM tests, check/Clippy,
serial fast, explicit tracked/untracked whitespace and frozen independent Sol
medium review. Run4 records retain first commands/full outputs/exits and routing.
No mixed publication order, trace acceptance, real-window or Rust root-algorithm
claim follows from this synthetic fixture.
