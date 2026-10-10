# N4 v1.14.1 mainnet policy basis — October 10

The task owner reports that the snapshot was likely produced by v1.14.1 and directs use of the repository's default mainnet configuration. Use that as the explicit reference assumption for further N4 work. Do not require another configuration upload to derive this reference. The likely release is not proof of the actual producer binary or capture process.

The [upstream release](https://github.com/Taraxa-project/taraxa-node/releases/tag/v1.14.1) identifies commit `a0e85fe31eb03573cd92c165a5f81035cec9907e`. That commit is already available in the local Git object database; no fetch, checkout, database open or replay was needed. Its EVM submodule pin is `6c7e5338b22d5e596cc2365a88d1f94840e1ee1b`, the existing public reference revision. The rewrite checkout pins the separate local oracle revision `bb0ab67c8cda1220aed74ecb01d2c4ca7c9bb418`; this comparison does not claim those two Go revisions are identical.

## Checks

Seven relevant files were compared byte for byte between the release commit and the current checkout. All match:

- `libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json`.
- `libraries/cli/include/cli/config_jsons/mainnet/mainnet_config.json`.
- `libraries/config/src/genesis.cpp` and `libraries/config/src/state_config.cpp`.
- `libraries/config/src/hardfork.cpp` and `libraries/config/include/config/hardfork.hpp`.
- `libraries/core_libs/consensus/src/rewards/rewards_stats.cpp`.

All 12 retained DPoS scalar fields also match these release defaults. This uses the fresh bounded CF8 report from [the availability check](n4_light_snapshot_availability.md). The derived initial `blocks_per_year` is 8,523,243: `31,536,000,000 / (2 × 1,500 + 700)`, with integer division, as in `GenesisConfig::calcBlocksPerYear`. This initial value differs from the previously derived post-Cacti dynamic reward rate of 9,275,294, which uses the retained lambda 1,500 and Cacti consensus delay 400. The two values apply at different stages and must not be interchanged.

The initial validator count also matches at 20. This comparison did not establish equality of every initial validator field or delegation. The retained DPoS row is database metadata, not state-root-authenticated producer configuration. File equality and scalar agreement support the stated reference assumption; they do not turn “likely” into an observed binary identity.

Under the supplied default-mainnet assumption:

| Input or branch | Value at H=25,706,949 |
|---|---|
| Chain ID | 841 |
| Committee size | 1,000 |
| Delegation delay | 5 |
| Session eligibility | 25,706,944 |
| Certificate eligibility | 25,706,942 |
| Reward frequency | 100 |
| Reward distribution at H | No; remainder is 49 |
| Pillar boundary at H | No; remainder modulo 4,000 is 2,949 |

The operational config's default `is_light_node` is false. Light mode can be enabled separately through the plugin. Thus “default mainnet configuration” supplies the consensus/state reference settings; it does not establish the node's actual light-plugin launch options. No launch-option claim is inferred from the snapshot name.

## Remaining work

Use the pinned mainnet configuration to derive the execution, hardfork, reward and slashing branches. Retain the owner-supplied assumption in evidence and compare independent outputs against stored results. Exact producer identity and capture provenance remain unverified, but are not a reason to stop deriving a reproducible reference from the supplied basis.

The snapshot availability check already establishes the live bytes in the three required DPoS and slashing tries. Complete semantic decoding, logical key catalogs and historical reward-graph reconstruction remain implementation work. The retained cache provides a candidate parent subset under the now explicitly supplied frequency of 100; a historical owner still needs reviewed cache reconstruction and a coherent parent head/generation. No owner was constructed and no real reward transition was executed by this comparison.

Evidence: `/home/fry/artifacts/n4-v1.14.1-policy-20261010/comparison.json`, containing release/checkout file hashes, all 12 scalar comparisons and derived period identities. Independent review is recorded as `review.md` in that directory. This record updates the configuration basis of the earlier N4 restart contract while preserving its semantic-state and historical-owner requirements.
