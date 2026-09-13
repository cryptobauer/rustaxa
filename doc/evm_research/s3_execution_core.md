# S3 bounded execution-core milestone

The isolated `rustaxa-evm` crate now exposes a canonical-input adapter, a wide
transaction envelope, a concrete journal, a host-generic instruction profile and
a bounded top-level CALL/CREATE driver. Production routing is unchanged. Nested
frame/native dispatch, complete period execution and persisted executor parity
remain open.

The journal preserves signed intermediate balances, arbitrary-width nonces,
ordinary/raw visibility, irreversible raw and transient effects, nested ordinary
undo, empty-account cleanup and the RIPEMD touch exception. Code installation
computes its own Keccak identity. Its transaction write plan separates ordinary
and raw writes so the sink applies raw writes last. Settlement is explicitly a
single-transaction boundary: its reset does not claim the reference's same-block
account/cache behavior or persistence.

The envelope retains full-width fee arithmetic and reference affordability,
nonce and intrinsic-gas ordering, including Cornus and zero-sender distinctions.
Zero debits preserve Go's ensure-only behavior; zero credits use its empty-account
touch semantics. Returned frame facts remain an input boundary. Transfer
consensus errors retain the attempted creation address and full-cap charging;
normal settlement applies the half-spent refund cap. Its focused arithmetic test
also uses a gas price wider than U256.

The profile explicitly configures the interpreter runtime to Istanbul and adds
only the proved Taraxa aliases/gas exceptions. Its SSTORE wrapper skips the
Ethereum EIP-2200 stipend sentry locally while retaining REVM's net-cost/refund
implementation, matching Go's EIP-1283 behavior. It restores runtime selection on
success and errors; the global CALL stipend is unchanged.

The journal-backed host executes actual account/code/storage/transient/log
operations and obtains historical block hashes through the existing domain
port. Full-width account emptiness is preserved separately from operand-stack
values, including `EXTCODEHASH` on balance 2^256. Code reads validate physical
hash, size and presence. Both runtime and initcode use legacy bytecode decoding:
an `ef01` prefix cannot activate REVM's EIP-7702 parser or panic before ordinary
invalid-opcode handling. SSTORE rejects original/current values above 256 bits
before mutation because the full-width Go gas comparison is not implemented yet.

The CALL driver composes real interpreter execution with envelope settlement,
including the zero-sender transfer exception and untouched absent recipients of
zero-value calls. Application-supplied native classification rejects native
dispatch as unavailable. CREATE retains arbitrary-width RLP nonce derivation,
creator/child nonce ordering, collision checks, initcode storage writes and
runtime code deposit. Pre-entry child failures return gas; exceptional child
failures consume it. Infrastructure and unsupported-operation errors require
discarding the pending execution, never publishing a partial journal.

## Evidence and validation limits

- Thirty-one direct journal fixtures come from real Go TransitionState/TrieSink
  runs against both pinned revisions. The Rust tests compare journal views and
  a simulated write-plan reopen; they do not validate Rust trie persistence.
- Sixteen envelope fixtures cover admission and settlement boundaries. Successful
  frame gas facts are supplied by the fixture harness, not produced by a complete
  Rust bytecode driver. Separate tests cover zero-amount lifecycle and nonzero
  refund-cap arithmetic.
- Six opcode cases and eleven seeded SSTORE sequences execute actual REVM
  instructions against narrow fixture hosts and compare Go gas/refund/state.
  The SSTORE corpus's nested STATICCALL parent remains evidence for the upcoming
  frame driver, not an executed comparison in this milestone.
- The actual top-level CALL driver executes the Go SSTORE set/clear case through
  admission, interpreter, journal and refund settlement: 21,412 charged gas and
  19,800 refund units before settlement. Ten host/driver tests cover this path,
  wide-value behavior, code validation, native refusal and legacy bytecode.
- CREATE address fixtures include five pinned Go cases with zero and
  55/56/65/257-byte nonces. The CREATE driver currently has a synthetic initcode
  storage/runtime test; its exact execution gas and persisted roots still await
  the independent two-period S4 oracle.
- The repository fast gate, affected storage-package tests and required four
  storage bridge tests pass. The explicit independent-snapshot reader gate passes
  at head 25,706,949 and prior period 25,706,948. That gate is read evidence only.

The snapshot original remains preserved. No broad replay, fault campaign,
reference-binary reopen or operational gate was run. The first complete
persisted path still requires the writer/lifecycle and native/reward composition
listed in [the S4 integration map](s4_integration_map.md).
