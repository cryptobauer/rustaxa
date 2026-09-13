# S5 BLS12-381 compatibility

`bls.rs` implements the two Taraxa BLS12-381 registry shapes as immutable
prepared stateless calls. The historical profile classifier remains outside the
module: callers must supply `BlsRegistry::Ficus` or `BlsRegistry::Cacti`.
Production routing is unchanged.

| Operation | Ficus | Cacti | Gas |
| --- | ---: | ---: | ---: |
| G1 add | 11 | 11 | 600 |
| G1 multiply | 12 | absent | 12,000 |
| G1 multi-exp | 13 | 12 | discounted G1 multiply gas |
| G2 add | 14 | 13 | 4,500 |
| G2 multiply | 15 | absent | 55,000 |
| G2 multi-exp | 16 | 14 | discounted G2 multiply gas |
| Pairing | 17 | 15 | 115,000 + 23,000 per complete pair |
| Map field to G1 | 18 | 16 | 5,500 |
| Map extension field to G2 | 19 | 17 | 110,000 |

The multi-exp quote follows the pinned Go code rather than a current Ethereum
table. It computes `k = input_length / element_length`; inputs shorter than one
element quote zero. For `k > 128`, Go replaces `k` with 128 before both the
discount lookup and multiplication. Consequently 128 and 129 pairs have the
same quote: 267,264 for G1 and 1,224,960 for G2. Execution still rejects empty
or non-multiple input lengths. Pairing always includes base gas, so empty input
quotes 115,000 and then fails validation. Funding is checked before validation.

The adapter uses individual BLS functions from REVM at the repository-pinned
revision `6014612c86f3690e4e9173a8c4deade396af398d`; it does not consume REVM's
registry or `SpecId`. Addition, maps, subgroup-valid MSM and pairing use those
primitives directly. Taraxa's pinned Go behavior differs in two places:

- Go checks curve membership but accepts non-subgroup points for addition,
  Ficus single multiplication and both multi-exp operations. Pairing alone
  rejects G1 and G2 non-subgroup points. REVM's MSM primitive rejects them.
  The adapter therefore uses REVM addition to reproduce the accepted Go MSM
  result after an REVM subgroup failure.
- REVM has MSM but no separate EIP-2537 single-multiply entry. The Ficus
  multiply path fast-paths valid subgroup points through the one-element REVM
  MSM. Its fallback reproduces gnark-crypto v0.12.1's GLV scalar split and G1/G2
  endomorphism before composing the result with REVM addition. This preserves
  the actual Go output for non-subgroup points, including large scalars where
  plain double-and-add is observably different.

The GLV constants are derived from the pinned gnark source. With subgroup order
`r` and eigenvalue `L = 228988810152649578064853576960394133503`,
`r = L² + L + 1` and `floor(sqrt(r)) = L`. The lattice precomputation's strict
greater-than loop does not execute, leaving basis vectors `V1 = (L, -1)` and
`V2 = (1, L + 1)` with determinant `+r`. The adapter uses the corresponding
signed `2^512 / r` rounded coefficients. Across the complete 256-bit input
range the absolute split components remain below the subgroup order, so the Go
conversion to field scalars introduces no further reduction before the two
endomorphism products are composed.

Input lengths are exact. Every 48-byte field element has 16 leading zero bytes
in the 64-byte ABI slot and must be smaller than the BLS12-381 field modulus.
The adapter preserves Go's distinct errors for top padding, noncanonical field
values, off-curve points and pairing subgroup failures. Infinity is encoded as
all-zero coordinates. Sequential validation preserves Go error precedence,
including first-point curve failures, G2 decoding before a G1 pairing subgroup
failure, and a first pair's subgroup failure before later-pair field errors.
Successful and contract-failure outcomes consume the quote and contain no
account mutations, raw mutations or logs.

## Evidence

`bls_reference.go` executes `RequiredGas` and `Run` from the actual Ficus and
Cacti maps at both immutable Go revisions. The 210-row corpus is identical at
public `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b` and local
`bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`. It covers all nine semantic
operations and both address layouts; valid arithmetic and maps; exact, short,
long and trailing lengths; infinity; invalid top bytes; field modulus values;
off-curve points; accepted non-subgroup add/multiply/MSM; rejected pairing
subgroups; scalar reduction and gnark GLV behavior at `r - 1`, `r`, `r + 1`,
`2r - 1`, `2r`, `2r + 1` and the maximum 256-bit scalar; mixed subgroup and
non-subgroup MSM streams; pairing true/false; error order; and multi-exp
discount entries 1, 2, 128 and the capped 129 case. Large
discount rows contain only infinity points and zero scalars, stored compactly as
an exact element plus repeat count.

The Python verifier rebuilds both pins in disposable trees sourced from
`submodules/taraxa-evm`, validates the corpus shape and exact address coverage,
then checks fixture bytes plus manifest and exporter hashes. Rust compares every
row's selected operation, quote, exact output/error and below-quote admission.
It separately proves the complete Ficus/Cacti mapping, high-address rejection,
owned call context and no state effects.

```sh
TARAXA_EVM_SOURCE=/workspaces/rustaxa-evm/submodules/taraxa-evm \
  python3 -O experiments/evm_feasibility/bls_reference.py
cargo test --manifest-path rust/Cargo.toml -p rustaxa-evm --test bls_reference
cargo clippy --manifest-path rust/Cargo.toml -p rustaxa-evm \
  --lib --test bls_reference --no-deps -- -D warnings
```

This establishes bounded direct primitive parity. Historical period
classification, frame gas settlement, persisted execution, broad resource
limits and production routing remain integration and acceptance work.
