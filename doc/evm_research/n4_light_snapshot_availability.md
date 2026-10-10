# N4 light-node snapshot availability check — October 10

The light-node copy retains the complete live DPoS and slashing storage at all three required historical identities. This check found no missing referenced trie nodes or live values in those six native-account inventories. A full-node snapshot is not currently justified as a way to recover these live bytes. Complete Rust staking models, historical key catalogs, producer policy and a historical execution owner remain open.

This updates the availability question in [the N4 restart contract](n4_owner_input_restart_contract.md). Its requirement for complete semantic state remains valid; it must not be read as proof that the snapshot lacks the underlying live database bytes. No real reward transition was executed or accepted.

## Fresh observations

The probe used only the guarded independent `local/evm-state-db/snapshot-litenode-copy` pair. Both databases were opened read-only through existing Rust readers. The supplied `data` tree was not opened. The mainnet genesis, stored head H=25,706,949, exact head header and parent root passed the existing pair qualification. No database head, state or configuration was changed through the probe.

| Dependency | Period | Finalized state root | DPoS live rows | Slashing live rows |
|---|---:|---|---:|---:|
| Parent P | 25,706,948 | `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2` | 23,278 | 54 |
| Session eligibility H−5 | 25,706,944 | `33199a435b8e4e4496fd7fd2bd8dd5bbb52263b775bc42b80f377a7e78b69014` | 23,278 | 54 |
| Certificate eligibility D | 25,706,942 | `dc4c6771a9ed227db8179b68783eefc78bb0f6a443c573d71457c8d516f8d041` | 23,278 | 54 |

Each historical identity came from its retained finalized header. Each native inventory authenticated the account and visited every referenced live storage leaf within fixed bounds: 50,000 nodes, 50,000 leaves and 32 MiB of selected values per account. A missing node or value would fail the inventory rather than return partial success.

The DPoS inventories are identical at these three identities: storage root `c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504`, 31,164 visited nodes and 259,077 value bytes. Their sorted path/value SHA-256 is `a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c`. Slashing inventories also match: 71 nodes and 128 value bytes per identity. This equality concerns the two native storage tries, not all accounts or the full chain state.

The existing partial inverse decoder found 196 validators and explained 2,065 DPoS rows at each identity. It left 21,213 live rows without a decoded meaning. These rows are present in the authenticated inventory; they are not missing data. Remaining families include delegator-scoped records, undelegations and non-head reward graph records. Hashed storage paths do not by themselves reveal their logical keys. Deleted and all-ever historical keys remain outside live-inventory coverage. No Rust DPoS sidecar exists at any of the three periods.

The parent vote-count and total-stake origin rows are physically present. The minted-token row is a tombstone; this check does not turn it into an invented zero. Full semantic reconstruction and native-session validation must preserve that distinction.

The reward-cache column was exhausted within 128 accepted rows and 1 MiB of accepted key/value bytes. All 49 rows decoded through the existing Rust rewards codec. Periods are exactly 25,706,901 through H=25,706,949; total key/value bytes are 27,006. Excluding H leaves all 48 periods expected for P under the checked-in candidate frequency of 100. This provides a candidate reconstruction source, not proof of the producer's historical cache rules or cache at P. There is one overflow lookahead; these are accepted-data bounds, not strict bounds on RocksDB allocation for a single item.

A fresh run of the existing retained-configuration tool exhausted CF8 and decoded its single 2,904-byte initial DPoS record. Delegation delay is 5 and the tested staking policy scalars match the checked-in candidate settings. This metadata is not authenticated by the state root and does not establish the producer binary, complete hardfork configuration, PBFT configuration or reward schedule.

## Evidence and validation

Persistent evidence is under `/home/fry/artifacts/light-snapshot-check-20261010`:

- `availability.json`: SHA-256 `b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1`.
- `config.json`: SHA-256 `3c817ff7974da68e0d5865b34e92765c7e50c71a6892c81d299f28c579e38634`.
- `probe.log`, `config.log`, `clippy.log`, `tests.log` and independent `review.md`.

The new experimental binary is `snapshot_input_availability`. Its source SHA-256 is `8dd61ce1ba618ac453ffbddf120f10a05d16bd1c51e6c39d466b79249f0067d0`. The report includes this pin. It reuses the existing path guards, paired qualification, read-only checkpoint reader, bounded inventory and partial inverse decoder; production code is unchanged.

Both read-only commands exited 0. The new binary passed `rustfmt --check` and targeted Clippy with warnings denied for this package. The qualifier library tests passed: 8 tests, including path/output guards, identity drift and partial-inverse checks. An existing unused-import warning in the consensus dependency remains outside this change. This is a data-availability diagnostic; production parity and storage-module routing gates do not apply.

No full database-content preservation comparison was performed. Read-only open paths and the untouched source tree do not establish a full logical-row preservation result for every database column. Independent review is recorded in the linked artifact; its scope is the diagnostic and evidence, not N4 execution acceptance.

## Next input decision

Use this copy to design complete native decoding and catalog reconstruction. Do not request another snapshot merely to obtain the six checked live inventories. A full-node snapshot could still help if older chain history is needed to recover key identities or if later checks find unavailable historical records. That need has not been established here.

Producer version and effective configuration remain external inputs unless other retained evidence can establish them. The current Rust owner still requires a coherent parent head, complete semantic snapshots and reward runtime. A reviewed historical-owner construction method is still needed; changing the input database's head to bypass its constructor is not accepted.
