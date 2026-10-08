# Reward claim failures: same-H1 Go oracle

This preparation captures six actual DryRunner failures over the accepted reward-only P1. Go native period and EVM block are both1. The supplied nonce is2^512; DryRunner uses committed caller nonce0+1. Keys31/A1 derive identities only. Public and local pins, source guards, support hashes, full92-row state and diagnostics are in the fixture manifest.

| Case | Actual gas | Actual result |
| --- | --- | --- |
| Commission, wrong owner |41464| This account is not owner of specified validator |
| Delegator, absent delegation |61464| Delegation does not exist |
| Commission, absent validator |41464| This account is not owner of specified validator |
| Delegator, malformed ABI |61272| abi: cannot marshal in to go type: length insufficient 0 require 32 |
| Delegator, value1 and malformed ABI |21272| Method is not payable |
| Delegator, gas21000 |21000| intrinsic gas too low (consensus error) |

All cases have zero native raw writes and zero logs. Value1 records ordinary caller debit1/native credit1 before rejection; the complete committed P1 remains unchanged. This transient account observation is not a native payout. Every first/repeat call preserves the full state snapshot; observed and observer-free controls match after witness removal. The harness checks exact measured inputs, senders, errors, gas, nonce and effects. This is Go reference evidence only. Public Rust failure parity remains pending.

Owned units: native_reward_claims_h1_failure_reference.go generates the six requests, repeats each through actual DryRunner and captures copy-only effects; the Python harness runs both pinned archives with guarded hooks and observer-free controls, checks the exact corpus and records/reproduces all13 fixture files. Prior corpora and runtime bytes stay unchanged.

Validation and frozen independent review are recorded in /home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359-r4-20261007/claims-failure-closeout.json when accepted. New checks are record, exact reproduction, Python compile, serial fast validation, explicit whitespace and live hashes. Unchanged Rust package gates are retained from commission-h1-estimate-closeout.json; production consensus1523 and ON12-job/all15 bridge evidence is retained from reward-h1-runtime closeout. No production, root parity, real-window or recovery acceptance follows from this oracle.
