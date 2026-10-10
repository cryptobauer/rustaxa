# H1 initial-cap claim-all estimation

The inputs are the accepted actual dual-pin H1 boundary measurements in
`../native_reward_claim_all_underfunded`. This derivative does not execute Go
again. The manifest binds every source measurement file by SHA-256.

The harness executes the unchanged C++ estimate search with those exact probe
outputs. Each initial cap uses one probe. Cap 21271 returns `intrinsic gas too
low`; caps 21272, 21273, 90000 and 111271 return `out of gas`. Funded controls
111272 and 111273 return their cap. No midpoint or wider estimate is claimed.

Run `python3 experiments/evm_feasibility/native_reward_claim_all_underfunded_estimate_reference.py`
to reproduce. `--record` writes this derivative's C++ output and manifest.
Canonical 92-row P1, literal H1, the original wide nonce, gas and native effects
remain bound to the accepted boundary oracle. This is a finite test profile.
