# Bounded S3 REVM profile evidence

This profile is a deliberately small interpreter configuration, not a Taraxa
fork schedule. It was checked against the pinned Go `opcodeFixtures` rows in
`experiments/evm_feasibility/fixtures/local.json`, excluding the nested-frame
rollback row.

The Go reference builds its ordinary table from the Californicum/Istanbul-era
rules, adds PUSH0 at `0x5f`, keeps transient operations at legacy aliases
`0xb3`/`0xb4`, and conditionally installs `0x5c`/`0x5d` for Cacti. Its net
SSTORE calculation uses 200, 20,000, 5,000, 19,800, and 4,800 gas/refund
components. REVM therefore starts at `SpecId::ISTANBUL`, applies the matching
five `GasId` overrides, and locally admits the three opcode implementations.
The local admission temporarily changes REVM's opcode guard only for the
instruction call, then restores Istanbul; it does not globally enable Shanghai
or Cancun semantics.

REVM's generic instruction table retains handlers for later Ethereum opcodes.
Callers must pair the table with `TaraxaProfile::configure_interpreter`, which
sets the interpreter runtime to Istanbul. The profile test verifies that
BASEFEE and MCOPY remain unavailable and that a transient alias restores that
base after both stack-underflow and static-call errors.

The focused integration test consumes each Go row's bytecode, Cacti flag,
error, gas used, refund, ordinary slot value, and transient value. It proves
the six direct single-frame cases: PUSH0, SLOAD, set-and-clear SSTORE, legacy
transient aliases, rejection of Cacti aliases before Cacti, and acceptance at
Cacti. The test host is test-only and uses REVM's `Host` trait; the public
profile has no prototype host.

This evidence does not cover a complete historical activation profile, nested
transient rollback, general SSTORE matrices, frame settlement, or production
routing. Those remain S3/S4 work and require review beyond this bounded table.
