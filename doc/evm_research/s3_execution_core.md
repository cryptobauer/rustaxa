# S3 bounded execution-core milestone

The isolated `rustaxa-evm` crate now exposes a canonical-input adapter, a wide
transaction envelope, a concrete journal, a host-generic instruction profile and
an iterative ordinary CALL/CREATE frame driver. Production routing is unchanged.
Opt-in [native frame handling](s5_native_journal.md),
[stateless dispatch](s5_stateless_frames.md) and
[wide SSTORE comparisons](s3_wide_sstore.md), and
[SELFDESTRUCT lifecycle handling](s3_selfdestruct.md) are now implemented with
bounded evidence. General period execution and full persisted executor parity
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
invalid-opcode handling. SSTORE preserves full-width zero/equality relations
for gas accounting while retaining actual stack-width new values in the journal.

The CALL driver composes real interpreter execution with envelope settlement,
including the zero-sender transfer exception and untouched absent recipients of
zero-value calls. Application-supplied native classification rejects native
dispatch as unavailable. CREATE retains arbitrary-width RLP nonce derivation,
creator/child nonce ordering, collision checks, initcode storage writes and
runtime code deposit. Pre-entry child failures return gas; exceptional child
failures consume it. Nested CALL/CALLCODE/DELEGATECALL/STATICCALL and CREATE/CREATE2
use an explicit interpreter stack and immediate-parent refund propagation.
Infrastructure and unsupported-operation errors require
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
  The nested driver also executes the SSTORE corpus's STATICCALL parent and
  compares its storage-change rejection with the pinned reference.
- The actual top-level CALL driver executes the Go SSTORE set/clear case through
  admission, interpreter, journal and refund settlement: 21,412 charged gas and
  19,800 refund units before settlement. Ten host/driver tests cover this path,
  wide-value behavior, code validation, native refusal and legacy bytecode.
- CREATE address fixtures include five pinned Go cases with zero and
  55/56/65/257-byte nonces. The CREATE driver also matches the independent
  [two-period S4 oracle](s4_persisted_period.md), including exact execution gas
  and the integrated persisted roots, receipts and physical history.
- Eight nested-frame tests include all 16 existing creation rows, nested
  transient revert, static SSTORE rejection, CALLCODE/DELEGATECALL context,
  signed refunds through enclosing rollback, exactly 1,025 active bytecode
  entries at the depth boundary, stipend return after pre-entry funds rejection,
  and prefix-only return-memory copying. CALL code loading now follows depth
  and funds admission: missing target code is not read for rejected frames,
  while admitted calls and EXTCODE operations retain strict code validation.
  Targeted tests also distinguish absent accounts from existing semantically
  empty accounts for the 25,000-gas value-CALL surcharge, including full-width
  balances.
- The repository fast gate, affected storage-package tests and required four
  storage bridge tests pass. The explicit independent-snapshot reader gate passes
  at head 25,706,949 and prior period 25,706,948. That gate is read evidence only.

The snapshot original remains preserved. No broad replay, fault campaign,
reference-binary reopen or operational gate was run. The first complete
persisted path is covered by [the bounded S4 composition](s4_persisted_period.md);
general native/execution and full historical coverage remain open.
