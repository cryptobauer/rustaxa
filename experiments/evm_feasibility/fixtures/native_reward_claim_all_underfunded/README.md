# Claim-all gas boundary measurements

Actual executions of both pinned Go revisions, repeated from fresh disposable
owners with observer controls. H1 uses literal block1 DryRunner.Apply over P1;
P2 uses EVM.Main at2 over concrete reader1 and explicit current nonce0. H1
supplied nonce2^512 is overridden by DryRunner to1. Caller local nonce after
every attempt, including intrinsic failure, is2 for H1 and1 for P2. Zero price
and value, selector0b83a727, canonical92-row reward-only P1 are fixed. The full
configuration, errors, raw reads/effects and complete physical source snapshots
are in public.json/local.json; manifest.json pins their source and hashes.

| Transaction cap | Quote / Run | Native supplied / used | Transaction used | Error |
| --- | --- | --- | --- | --- |
| 21271 | no / no | none / none | 21271 | consensus: intrinsic gas too low |
| 21272 | yes / no | 0 / 0 | 21272 | execution: out of gas |
| 21273 | yes / no | 1 / 0 | 21272 | execution: out of gas |
| 90000 | yes / no | 68728 / 0 | 21272 | execution: out of gas |
| 111271 | yes / no | 89999 / 0 | 21272 | execution: out of gas |
| 111272 | yes / yes | 90000 / 90000 | 111272 | none |
| 111273 | yes / yes | 90001 / 90000 | 111272 | none |

Native used/left gas is derived from actual Main gas used for this direct first
call; null denotes no native call. DryRunner exposes no separate parent error
lane (null); P2 Main records it directly. Funded raw/balance/log effects match
the previously accepted H1/P2 positive vectors exactly. Underfunded native calls
have no Run, raw/balance effects or logs, and watched raw state remains fixed.

Go RequiredGas includes lazy setup. Its exact quote order is eligible votes
400 (key Keccak([4])), delegated amount4000 ([5]), absent total supply ([7]),
minted tokens800 ([6]), then caller LE32 count2. Each emits a logical wrapper
request and backend request: ten quote events. Funded cases have the same
prefix plus38 Run events. Below intrinsic, there are no native events. Outer
envelope reads and post-state snapshot reads are separate from these native
read lanes. Observer controls omit read/write/balance taps and retain copy-only
quote/Run/post taps; their execution output and state are identical.

A preloaded Rust semantic session can have only a per-call count read after
explicit source/profile qualification. That boundary does not reproduce Go
bootstrap callback order or injected bootstrap fault timing. This oracle does
not accept Rust runtime, raw-read identity, roots, publication, general state,
production routing, trace, recovery or N4-N6. The successful-source profile
requires the same authenticated P1 and the four mapped counter/supply values.

Reproduce without changing fixtures:

```sh
rtk proxy python3 experiments/evm_feasibility/native_reward_claim_all_underfunded_reference.py --scratch /home/fry/artifacts/<run>/go-scratch --evidence /home/fry/artifacts/<run>/new-evidence
```

The evidence directory must be new. Full failed prediction and source correction
records are retained at
/home/fry/artifacts/evm-run-01a12418-dc34-7013-a51b-57b6ef22163b-20261010.
