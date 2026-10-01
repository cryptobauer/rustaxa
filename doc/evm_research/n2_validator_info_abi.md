# Staged metadata ABI and admission

Base: `86e74ff`, on `feat/rust/evm-state-db`. This follows the
[decoded metadata slice](n2_staged_validator_info.md). It closes the selected
method's staged ABI/admission dependency; it does not close N1, N2, N3 or
milestone 10.

## Contract and change

The staged session recognizes selector `0babea4c` before unpacking arguments.
It uses existing native admission to quote 20,000 gas (zero for nonzero value
at/after Cornus), reject insufficient funding, then enforce historical depth
before nonpayability. Only admitted calls enter the new operation-specific
codec. A normal ABI failure binds and consumes its exact invocation quote,
returns the pinned Go error with no state effects, and permits the next call.
Request or quote mismatch retains the valid preparation.

The codec reproduces pinned `accounts/abi/unpack.go` and declaration-order field
unpacking: address first, then description, then endpoint. It ignores address
high bytes, preserves arbitrary string bytes, and accepts unaligned/overlapping
tails, head aliases, trailing bytes and absent unused padding. Offset and total
length arithmetic uses `BigUint`; decimal diagnostics and the signed-int64
length bound follow Go error precedence. No raw state is read during admission
or ABI decode. Valid calls use the already reviewed metadata kernel and exact
owner/info/membership authentication. The general FinalChain decoder and other
methods are unchanged. Domain state remains independent of this ABI codec.

Public constructors still reject pre-Cornus sessions. Sparse owner/info state,
zero-caller absent-owner semantics, delegate/call-code and full Rust frame/API
composition remain open. This slice does not add a production route or change
publication, reward completion, protocol rules or C++ ownership.

## Independent oracle and checks

The new exporter runs the actual pinned EVM at synthetic period 25,706,949 with
chain ID 841 and Cacti/Ficus/Magnolia/Cornus VM flags. Native configuration pins
the relevant mainnet activation numbers. The accounts, raw state, supply and
reward parameters are explicitly synthetic; no complete mainnet producer or
historical state is inferred. Both public
`6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418` run from disposable archives.
They produce byte-identical JSON and native diagnostic stdout.

Shared observer support is copied unchanged from the prior exporter, with only
its unused `main` name changed in the archive. Gas funding, quote, caller and
depth are observed from the real native frame. Native results, errors, storage
operations and EVM execution are forwarded unchanged. Go prints ABI diagnostics
before its terminal JSON; the harness retains those bytes in `.stdout.txt`
artifacts and checks them on reproduction. Source and output hashes bind both
pins and both outputs. Existing corpus files are unchanged.

Twenty-one cases cover canonical Cacti execution, seven short-head lengths,
large description/endpoint offsets and lengths, insufficient dynamic bytes,
description error before a missing endpoint head, dirty address/trailing data,
overlapping tails, absent padding, unaligned tails, head aliases, insufficient
gas before malformed ABI and nonpayability before malformed ABI. The Rust tests
compare all 21 cases through both public session types. Synthetic Rust fixtures
rebase active native rules to periods 1 and 0, with Cacti/Ficus active, without
fabricating mainnet history. Exact quotes, errors, native outputs, ordered read
keys, writes, logs and terminal metadata match. Ordinary account effects and
reads remain empty. ABI-error readers deliberately fail if consulted.

An additional test proves exact quote binding and successful continuation after
an ABI error. The unchanged prior metadata/raw-integrity tests also pass.
All seven targeted tests passed on the first run. Both pinned oracle runs and
reproduction passed. The workspace fast gate passed. Rust-enabled `/build`
consensus bridge build (12 jobs) and all 15 bridge tests passed; master cache
option `RUSTAXA_ENABLE:BOOL=ON` was checked. No storage-module or C++ source was
changed. No broad, differential or fault gate was run. The existing unused-import
warning in `consensus_application.rs` remains outside scope.

Complete first-run commands, logs, exit codes and frozen sources remain under
`/home/fry/artifacts/evm-branch-2026-10-01-2233/`, with `abi-` prefixes.
No failed gate or implementation correction occurred before source freeze.

## Review and next dependency

Sol medium implemented directly under the prior read-only Astra contract review.
The separate Astra medium reviewer is reused after source/evidence freeze.
Luna medium is mapping the next existing Rust frame/API test harness without
editing source. All three requested routes were previously confirmed from
current log metadata. Allowance was 28% at slice start and 27% after targeted
checks; these shared-account observations are not token or billing estimates.
Independent Astra medium final review accepted the frozen source and evidence
with no findings. All freeze hashes matched; no validation was repeated during
the read-only review. No correction batch was required.

Next ready work is actual Rust EVM frame/API composition, using this same session
adapter and historical view. Complete native/slashing/reward coverage, producer
qualification, complete historical inputs, real-window receipts and roots,
bootstrap, publication/recovery/retention and integrated acceptance remain open.
Supplied data and historical copies were not opened or changed. No push occurred.
