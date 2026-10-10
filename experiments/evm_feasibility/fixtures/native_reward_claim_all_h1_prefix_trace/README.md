# Literal H1/P0 ordinary-prefix claimAll trace

Both Go pins execute the exact ordinary self-call prefix at gas21000, then the
zero-value claimAll target at gas200000 over the captured canonical P0 reader0
and private execution1 context. Each observed/control run repeats twice and
preserves complete P0/P1 sources and original wide nonce inputs.

The measured prefix has no native quote/run, unchanged native raw state and
caller nonce2^512+1. The target uses111272 gas, quotes90000 with native178728,
performs48 native reads/12 raw writes with no payout/logs, and ends at nonce2^512+2.
Transaction position1 is ordered-composition authority, not original Go metadata.

The manifest binds exact source/support, pins, instrumentation and captured
outputs. This oracle does not admit a Rust profile, other caps, production,
historical root parity, recovery or a real network window.
