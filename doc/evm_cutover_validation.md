# Rust EVM cutover validation

The production App resumes execution from a qualified pair of existing stores.
Keep the primary store, concrete state store, and their finalized history together.
Fresh bootstrap, migration from an operator store, and coordinated network activation
require separate qualification.

## Local checks

Run `make rewrite-validate-fast` from the checkout. Configure CMake with
`RUSTAXA_ENABLE=ON` for the production daemon. A second configuration with
`RUSTAXA_APP_OFFLINE_TEST=ON` builds `rustaxa_offline_app_test`.
Set `RUSTAXA_OWNER_TEST_SCRATCH_ROOT` to a new empty directory for that binary.

## Release qualification

Follow the [two-peer restart instructions on the feature branch](https://github.com/cryptobauer/rustaxa/blob/feat/rust/evm-state-db/doc/evm_research/two_peer_restart.md).
Verify the synced receipt before and after restart, historical Debug and
OpenEthereum RPC results, and finalization of the successor period.
The feature branch also holds the Go parity suites, witness capture tools,
and reference fixtures.

Production-network continuation, performance qualification, and activation remain
release gates.
