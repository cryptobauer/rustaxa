# S5 stateless precompile inventory

This is a source inventory only. It does not select a generic Ethereum spec,
define Taraxa activation rules, or claim byte/gas/error parity.

The pinned Go registry is in `submodules/taraxa-evm/core/vm/contracts.go`:

| Registry | Addresses | Go implementation | Rust availability observed locally |
|---|---|---|---|
| Californicum (`:64-73`) | 1 ECRECOVER, 2 SHA256, 3 RIPEMD160, 4 identity, 5 MODEXP, 6/7/8 BN254 | `ecrecover :126`, `sha256hash :167`, `ripemd160hash :182`, `dataCopy :198`, `bigModExp :280`, BN operations `:337/:360/:390` | `k256 0.13`, `sha2`, `ripemd`, `aurora-engine-modexp`, `ark-bn254` are locked transitively through REVM (`rust/Cargo.lock` package entries). Only `k256` is a direct dev dependency of `rustaxa-evm`; no Rustaxa stateless wrapper exists. |
| Ficus (`:75-94`) | Californicum plus 9 BLAKE2F and 11–19 BLS12-381 | `blake2F :443`; BLS operations `:498-936` | REVM’s pinned precompile crate is in `/cargo/git/checkouts/revm-0a89e51b0ec51a84/6014612/crates/precompile`; its source has BLAKE2 and BLS12-381 modules. It is transitive, not a declared Rustaxa API. |
| Cacti (`:97-123`) | 1–9; remapped BLS 11–17; 0x0100 P-256; 0xfa1c Falcon | BLS mapping differs from Ficus; `p256Verify :975`; `falcon512 :1016` | `p256` is locked through REVM. No Falcon implementation dependency or Rustaxa Falcon kernel was found. |

Address `0x0a` (KZG point evaluation) is commented out in both Ficus and Cacti
registries (`contracts.go:85,108`), so it is not registered by those pinned Go
tables. The Go fork selection is `EVM.SetBlock` in `core/vm/evm.go:232-248`:
Cacti selects its table, Ficus selects its table, otherwise Californicum.

Existing Rustaxa code is not a stateless-precompile surface. The only nearby
cryptographic helper found is storage-local signature recovery in
`rust/crates/rustaxa-storage/src/main.rs:1395` (`ecrecover_address`), which is
not an EVM precompile API. `rustaxa-evm` currently has no precompile module.

Before implementation, each registry entry still needs exact input padding,
validation, output, gas, error and activation comparisons against the pinned Go
functions. Directly consuming REVM precompile tables would require an explicit
review because their registry and `SpecId` selection are Ethereum-oriented.
