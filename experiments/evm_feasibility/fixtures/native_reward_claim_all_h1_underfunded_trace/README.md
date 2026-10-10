# H1/P0 trace gas boundaries

This fixture measures literal Go TraceRunner at execution1 over the actual genesis P0 captured during the unchanged canonical reward-only P1 history. P0 has82 physical rows; P1 has92. Each cap uses one target, an empty prefix, the supplied nonce2^512 and two fresh trace states. Both pinned revisions and observation-reduced controls agree. Complete P0 and P1 remain unchanged after every call.

| Transaction cap | Native supplied gas | Quote / Run | Transaction gas used | Result |
| --- | --- | --- | --- | --- |
| 21271 | no native call | no / no | 21271 | intrinsic gas too low |
| 21272 | 0 | yes / no | 21272 | out of gas |
| 21273 | 1 | yes / no | 21272 | out of gas |
| 90000 | 68728 | yes / no | 21272 | out of gas |
| 111271 | 89999 | yes / no | 21272 | out of gas |
| 111272 | 90000 | yes / yes | 111272 | success |
| 111273 | 90001 | yes / yes | 111272 | success |

Quote is90000 and observes ten ordered logical/backend events: votes400, delegated4000, absent total supply, absent minted tokens, caller count2. Funded cases observe48 events and twelve ordered writes. All cases have zero native scalar account reads, balance effects and logs. The caller post nonce is2^512+1 in every case, including intrinsic failure. The native contract remains balance4000/nonce1. Native OOG reports both execution and parent errors; the intrinsic consensus error has an empty parent error. Detailed errors remain in post observations because default trace JSON exposes only the failed flag.

The controls retain P0 capture, reader selection, quote/Run and target-post hooks; they suppress only storage-read, raw-write, scalar-account and balance taps. They are observation-reduced controls. Exact guarded copied-source diffs, helper/source hashes and output bytes are in the manifest and instrumentation JSON files. Original pinned trees and accepted funded fixtures are unchanged.

Run `rtk proxy python3 experiments/evm_feasibility/native_reward_claim_all_h1_underfunded_trace_reference.py` to reproduce. The first actual four-run capture was retained before outcome validation. This is finite source-fixture evidence. It does not accept Rust admission, arbitrary bootstrap byte substitution or callback fault timing, external chain/root parity, general traces, recovery or production.
