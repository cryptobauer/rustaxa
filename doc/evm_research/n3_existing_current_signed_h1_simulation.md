# Signed existing-current H1 simulation

This test uses the accepted runtime `072fc39fb` and the actual dual-pin
[H1 oracle](n3_existing_current_signed_h1_oracle.md). It adds a separate test
module and registers that module. Runtime, shared support and prior corpora
are unchanged.

The public Rust owner starts with two validators, addresses31 and32. Each has
caller stake1000 and ownera1 stake1000. Post-debit accounts are caller3000,
owner1000 and native4000; maximum supply8000. The explicit locking periods are
DPOS2, Cornus3 and disabled Cacti7. The canonical signed partial300 transaction
is decoded and recovered, stored in ordinary PBFT PeriodData, then passed to
public finalization exactly once. Receipt status, gas101912 and event300 match
Go. A second owner lifetime reopens H1 without finalizing it again. Public
reads check caller700/1300, owner1000/1000, both membership lists[31,32], current
validator totals1700/2300, delayed totals2000/2000, principal4000, zero pending
rewards and all three account balances/nonces.

The physical reader uses the separate complete105-row Go H1 identity, root
e4781dc89abf0c3ff461290bc536b5ed807a39ded5c23cf9525349e07727e8b7.
Account RLP, nonce, balance, code and native raw presence/bytes are checked.
The public semantic owner is not hydrated from those rows. The corpus producer
checks that both current nodes retain count2 after the prefix, EndBlock and
Commit. No Rust/Go root equality is claimed.

Each reader lifetime first measures the successful sequence of15 distinct
authentication keys. At every key it then injects corrupt present bytes and a
reader error; each failure is followed immediately by a fresh successful
session. Failure reads must stop at that key. Corruption returns RawIntegrity;
reader errors retain the injected invariant error. Every attempt drops its
disposable native session exactly once. Four reader lifetimes across two owner
lifetimes give248 sessions:120 failures and128 successful calls, including120
fresh retries. Native invocation is period1; the supplied2^512 request nonce
is unchanged, while Go reports effective nonce2. Success gas101912, return and
event700 match both pinned Go outputs. The public H1 header/semantic state and
all complete physical rows remain unchanged after each attempt and reopen.

Required checks are the targeted test, both-pin oracle reproduction, Python
compile, EVM package/check/Clippy, serial rewrite-validate-fast and whitespace.
The unchanged runtime inherits its accepted Rust-enabled bridge build with12
jobs and all15 tests; this test-only slice does not claim a new bridge run.
First-run output, corrections, frozen hashes and independent Sol medium review
are retained in the run3 artifact directory. The first compile found a missing
address helper; direct hex decoding corrected it before the targeted test passed.

Historical estimation, trace/H2, real-window root parity and production routing
remain separate gates. No upstream C++, storage owner, supplied data, fallback,
protocol change, broad differential gate or push is included.
