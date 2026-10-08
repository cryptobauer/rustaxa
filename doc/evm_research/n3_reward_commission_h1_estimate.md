# Positive commission H1 gas estimation parity

This bounded test derivative uses separately measured same-H1 owner commission claim and canonical reward-only P1. Actual pinned DryRunner calls execute first/repeat perprobe, with full92-row P1/root unchanged. No observation hook is installed; unused shared main only requires the compiled nil balance helper.

Three measured profiles: cap200000 yields actual caps200000/120732/81098/61281/51372/46418/43941/42702, all success gas41464; minimum41464 succeeds once; below41463 reports outofgas/gas21464 once. The unchanged compiled C++ eth_estimateGas body (algorithm7136e3d4...be7759) checks every cap and complete consumption. Results42702/8,41464/1,outofgas/1. Ten actual Go probes perpin are measured. Source/support/input/harness/DryRunner/C++/diagnostic/output hashes remain in the manifest.

Public Rust estimate_gas consumes actual transcripts with fresh explicit historical1 ports. Two owners/two authenticated reader opens/two searches produce80 fresh simulations/24 searches. Each probe checks gas/error/return/log/identity, ownercurrentnonce+1 authority, immutable original nonce, full concrete inventory, semantic owner header/accounts/principal4000/stakes2000 and exact C++ result/fullconsumption. No returned native nonce witness or terminal publication is claimed.

Required producer record/reproduce/Python/target/fullEVM/checkClippy/serialfast/explicitwhite/freeze/hash and same-profile independent Sol medium review apply. Production source is unchanged; retain6fe consensus1523/ON12-job/all15bridge evidence explicitly. General commission/history/headadvance/root/realwindow/RPC/recovery/cutover and production admission remain open. No push.
