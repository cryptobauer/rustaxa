//! Constructor-time enrichment of FinalChain's configured genesis accounts.
//!
//! FinalChain owns semantic genesis balances while StateAPI owns authenticated
//! account metadata and code. This module joins those facts at construction
//! time without publishing storage or treating a concrete reader as chain
//! authority. Application bootstrap must establish the database/chain pairing
//! before calling this opt-in method.

use super::*;
use rustaxa_types::concrete_state::{ConcreteAccountRecord, ConcreteRead, ConcreteStateRead};

impl FinalChain {
    /// Enriches configured period-zero accounts from an authenticated concrete reader.
    ///
    /// The reader must be pinned to period zero and the exact concrete genesis
    /// root configured on this FinalChain. Every address already present in the
    /// semantic genesis snapshot must have an authenticated account row. Its
    /// nonce and full balance must equal the configuration-derived semantics;
    /// storage root, code hash, and code size are then copied into the genesis
    /// snapshot after the referenced code bytes are read and validated.
    /// The numeric balance remains unchanged while its snapshot-byte provenance
    /// becomes the canonical minimal concrete representation. Configuration must
    /// therefore include zero-balance hardfork code accounts that need metadata
    /// hydration; this method does not discover the account trie or invent
    /// semantic balances.
    ///
    /// All reads and validation complete before in-memory state changes. At a
    /// genesis head, the hydrated snapshot also becomes the live account map.
    /// During restart at a later head, only historical snapshot zero changes;
    /// the loaded latest snapshot remains untouched. A pending external-EVM
    /// publication, missing account/code, root mismatch, semantic mismatch,
    /// oversized balance, or corrupt code fails without mutation. The method
    /// performs no database writes and does not authorize adopting an imported
    /// concrete database; application bootstrap owns pairing and recovery.
    pub fn hydrate_concrete_genesis_accounts(
        &mut self,
        reader: &dyn ConcreteStateRead,
    ) -> Result<(), anyhow::Error> {
        anyhow::ensure!(
            self.enforce_genesis_state_root,
            "FINAL_CHAIN_CONCRETE_GENESIS_ROOT_POLICY_DISABLED"
        );
        let identity = reader.identity();
        anyhow::ensure!(
            identity.period == FinalChainBlockNumber::GENESIS,
            "FINAL_CHAIN_CONCRETE_GENESIS_PERIOD_MISMATCH"
        );
        anyhow::ensure!(
            H256::from(identity.state_root) == self.genesis_state_root,
            "FINAL_CHAIN_CONCRETE_GENESIS_ROOT_MISMATCH"
        );
        anyhow::ensure!(
            !self.has_external_evm_pending_publication()?,
            "FINAL_CHAIN_CONCRETE_GENESIS_PENDING_PUBLICATION"
        );

        let head = self.last_block_number_typed()?;
        let latest_snapshot = *self
            .latest_account_snapshot_block
            .get_mut()
            .map_err(|_| anyhow::anyhow!("final-chain latest account snapshot lock poisoned"))?;
        anyhow::ensure!(
            latest_snapshot == head,
            "FINAL_CHAIN_CONCRETE_GENESIS_LATEST_SNAPSHOT_HEAD_MISMATCH"
        );
        let configured = self
            .account_snapshots
            .get_mut()
            .map_err(|_| anyhow::anyhow!("final-chain account snapshot lock poisoned"))?
            .get(&FinalChainBlockNumber::GENESIS)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_GENESIS_SNAPSHOT_MISSING"))?;
        if head.is_genesis() {
            anyhow::ensure!(
                *self
                    .accounts
                    .get_mut()
                    .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?
                    == configured,
                "FINAL_CHAIN_CONCRETE_GENESIS_LIVE_SNAPSHOT_MISMATCH"
            );
        }

        let mut addresses = configured.keys().copied().collect::<Vec<_>>();
        addresses.sort_unstable();
        let mut hydrated = HashMap::with_capacity(configured.len());
        for address in addresses {
            let expected = &configured[&address];
            let record = match reader.account(address)? {
                ConcreteRead::Present(record) => record,
                ConcreteRead::Absent => {
                    anyhow::bail!("FINAL_CHAIN_CONCRETE_GENESIS_ACCOUNT_ABSENT:{address:?}")
                }
                ConcreteRead::Tombstone => {
                    anyhow::bail!("FINAL_CHAIN_CONCRETE_GENESIS_ACCOUNT_TOMBSTONE:{address:?}")
                }
            };
            hydrated.insert(
                address,
                hydrate_account(reader, address, expected, &record)?,
            );
        }

        let accounts = self
            .accounts
            .get_mut()
            .map_err(|_| anyhow::anyhow!("final-chain account lock poisoned"))?;
        let snapshots = self
            .account_snapshots
            .get_mut()
            .map_err(|_| anyhow::anyhow!("final-chain account snapshot lock poisoned"))?;
        if head.is_genesis() {
            *accounts = hydrated.clone();
        }
        snapshots.insert(FinalChainBlockNumber::GENESIS, hydrated);
        Ok(())
    }
}

fn hydrate_account(
    reader: &dyn ConcreteStateRead,
    address: [u8; 20],
    expected: &Account,
    record: &ConcreteAccountRecord,
) -> Result<Account, anyhow::Error> {
    anyhow::ensure!(
        record.account.nonce == expected.nonce,
        "FINAL_CHAIN_CONCRETE_GENESIS_NONCE_MISMATCH:{address:?}"
    );
    let mut balance_bytes = record.account.balance.value().to_bytes_be();
    if balance_bytes == [0] {
        balance_bytes.clear();
    }
    anyhow::ensure!(
        balance_bytes.len() <= 32,
        "FINAL_CHAIN_CONCRETE_GENESIS_BALANCE_EXCEEDS_U256:{address:?}"
    );
    let balance = U256::from_big_endian(&balance_bytes);
    anyhow::ensure!(
        balance == *expected.balance.as_u256(),
        "FINAL_CHAIN_CONCRETE_GENESIS_BALANCE_MISMATCH:{address:?}"
    );

    if record.account.code_size > 0 && record.account.code_hash.is_none() {
        anyhow::bail!("FINAL_CHAIN_CONCRETE_GENESIS_CODE_HASH_MISSING:{address:?}");
    }
    if let Some(code_hash) = record.account.code_hash {
        let code = match reader.code(code_hash)? {
            ConcreteRead::Present(code) => code,
            ConcreteRead::Absent => {
                anyhow::bail!("FINAL_CHAIN_CONCRETE_GENESIS_CODE_ABSENT:{address:?}")
            }
            ConcreteRead::Tombstone => {
                anyhow::bail!("FINAL_CHAIN_CONCRETE_GENESIS_CODE_TOMBSTONE:{address:?}")
            }
        };
        let expected_size = usize::try_from(record.account.code_size)
            .map_err(|_| anyhow::anyhow!("FINAL_CHAIN_CONCRETE_GENESIS_CODE_SIZE_OVERFLOW"))?;
        anyhow::ensure!(
            code.len() == expected_size,
            "FINAL_CHAIN_CONCRETE_GENESIS_CODE_SIZE_MISMATCH:{address:?}"
        );
        anyhow::ensure!(
            concrete_storage_key(&[code.as_slice()]) == code_hash,
            "FINAL_CHAIN_CONCRETE_GENESIS_CODE_HASH_MISMATCH:{address:?}"
        );
    }

    Ok(Account {
        nonce: record.account.nonce.clone(),
        balance: rustaxa_types::FinalChainAccountBalance::from_snapshot_bytes(&balance_bytes)?,
        storage_root_hash: record.account.storage_root.unwrap_or_default(),
        code_hash: record.account.code_hash.unwrap_or_default(),
        code_size: record.account.code_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;
    use rustaxa_storage::{Config, FinalChainExternalEvmPendingPublication};
    use rustaxa_types::concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteReadError, ConcreteStateIdentity,
        ConcreteStorageKey,
    };
    use rustaxa_types::GenesisValidatorMetadata;
    use serde_json::Value;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const ORACLE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../experiments/evm_feasibility/fixtures/mixed_local_observer.json"
    ));

    #[derive(Clone)]
    struct FixtureReader {
        identity: ConcreteStateIdentity,
        accounts: BTreeMap<[u8; 20], ConcreteAccountRecord>,
        codes: BTreeMap<[u8; 32], Vec<u8>>,
    }

    impl ConcreteStateRead for FixtureReader {
        fn identity(&self) -> ConcreteStateIdentity {
            self.identity
        }

        fn account(
            &self,
            address: [u8; 20],
        ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
            Ok(self
                .accounts
                .get(&address)
                .cloned()
                .map(ConcreteRead::Present)
                .unwrap_or(ConcreteRead::Absent))
        }

        fn storage(
            &self,
            _address: [u8; 20],
            _key: ConcreteStorageKey,
        ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
            Err(ConcreteReadError::HistoryUnavailable(self.identity))
        }

        fn code(&self, code_hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
            Ok(self
                .codes
                .get(&code_hash)
                .cloned()
                .map(ConcreteRead::Present)
                .unwrap_or(ConcreteRead::Absent))
        }
    }

    fn fixture() -> Value {
        serde_json::from_str(ORACLE).expect("mixed observer fixture is valid JSON")
    }

    fn field<'a>(value: &'a Value, name: &str) -> &'a str {
        value[name]
            .as_str()
            .unwrap_or_else(|| panic!("{name} must be a string"))
    }

    fn hex_bytes(raw: &str, name: &str) -> Vec<u8> {
        assert!(raw.len().is_multiple_of(2), "{name} has even hex width");
        raw.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("hex pair is UTF-8");
                u8::from_str_radix(pair, 16)
                    .unwrap_or_else(|error| panic!("{name} must be hex: {error}"))
            })
            .collect()
    }

    fn fixed_hex<const N: usize>(value: &Value, name: &str) -> [u8; N] {
        let bytes = hex_bytes(field(value, name), name);
        bytes.try_into().unwrap_or_else(|bytes: Vec<u8>| {
            panic!("{name} must contain {N} bytes, got {}", bytes.len())
        })
    }

    fn uint(value: &Value, name: &str) -> U256 {
        U256::from_dec_str(field(value, name)).expect("fixture amount fits uint256")
    }

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time follows Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustaxa-concrete-genesis-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn fixture_reader(oracle: &Value) -> FixtureReader {
        let accounts = oracle["genesis"]["native_catalog"]["accounts"]
            .as_array()
            .expect("native accounts are an array")
            .iter()
            .map(|row| {
                let storage_root = row["storage_root"]
                    .as_str()
                    .map(|raw| fixed_hex_string(raw, "storage root"));
                let code_hash = row["code_hash"]
                    .as_str()
                    .map(|raw| fixed_hex_string(raw, "code hash"));
                (
                    fixed_hex(row, "address"),
                    ConcreteAccountRecord {
                        account: ConcreteAccount {
                            nonce: FinalChainNonce::from_bytes(&minimal_bytes(uint(row, "nonce")))
                                .expect("canonical fixture nonce"),
                            balance: ConcreteAccountBalance::new(BigUint::from_bytes_be(
                                &minimal_bytes(uint(row, "balance")),
                            )),
                            storage_root,
                            code_hash,
                            code_size: row["code_size"].as_u64().expect("code size is an integer"),
                        },
                        physical_rlp: hex_bytes(field(row, "raw_account"), "raw account"),
                    },
                )
            })
            .collect();
        let codes = oracle["genesis"]["native_catalog"]["accounts"]
            .as_array()
            .expect("native accounts are an array")
            .iter()
            .filter_map(|row| {
                row["code_hash"].as_str().map(|hash| {
                    (
                        fixed_hex_string(hash, "code hash"),
                        hex_bytes(field(row, "code"), "code"),
                    )
                })
            })
            .collect();
        FixtureReader {
            identity: ConcreteStateIdentity {
                period: FinalChainBlockNumber::GENESIS,
                state_root: fixed_hex_string(field(&oracle["genesis"], "root"), "genesis root"),
            },
            accounts,
            codes,
        }
    }

    fn fixed_hex_string<const N: usize>(raw: &str, name: &str) -> [u8; N] {
        let bytes = hex_bytes(raw, name);
        bytes.try_into().unwrap_or_else(|bytes: Vec<u8>| {
            panic!("{name} must contain {N} bytes, got {}", bytes.len())
        })
    }

    fn minimal_bytes(value: U256) -> Vec<u8> {
        if value.is_zero() {
            return Vec::new();
        }
        let bytes = value.to_big_endian();
        bytes[bytes.iter().position(|byte| *byte != 0).unwrap()..].to_vec()
    }

    fn configured_chain(storage: Arc<Storage>, oracle: &Value) -> FinalChain {
        let dpos_address = DPOS_CONTRACT_ADDRESS;
        let accounts = oracle["genesis"]["native_catalog"]["accounts"]
            .as_array()
            .expect("native accounts are an array")
            .iter()
            .filter(|row| fixed_hex::<20>(row, "address") != dpos_address)
            .map(|row| GenesisAccount {
                address: fixed_hex(row, "address"),
                balance: rustaxa_types::FinalChainAccountBalance::from_cpp_genesis_bytes(
                    &uint(row, "balance").to_big_endian(),
                )
                .expect("fixed genesis balance"),
            })
            .collect();
        let validator = &oracle["inputs"]["initial_validator"];
        let delegations = validator["delegations"]
            .as_array()
            .expect("delegations are an array")
            .iter()
            .map(|row| {
                (
                    fixed_hex(row, "delegator"),
                    uint(row, "amount").to_big_endian().to_vec(),
                )
            })
            .collect::<Vec<_>>();
        let total_stake = delegations.iter().fold(U256::zero(), |total, (_, amount)| {
            total + U256::from_big_endian(amount)
        });
        let root = fixture_reader(oracle).identity.state_root;
        FinalChain::new_with_genesis_state_root(
            storage,
            oracle["configuration"]["block_gas_limit"]
                .as_u64()
                .expect("block gas limit is an integer")
                .into(),
            0,
            H256::from(root),
            true,
            accounts,
            vec![GenesisValidator {
                address: fixed_hex(validator, "address"),
                vrf_key: fixed_hex(validator, "vrf_key"),
                total_stake: total_stake.to_big_endian().to_vec(),
                delegations,
                metadata: GenesisValidatorMetadata {
                    owner: fixed_hex(validator, "owner"),
                    commission: validator["commission"].as_u64().unwrap() as u16,
                    ..Default::default()
                },
            }],
            GenesisDposConfig {
                eligibility_balance_threshold: U256::from(100).into(),
                vote_eligibility_balance_step: U256::from(10).into(),
                validator_maximum_stake: U256::from(1_000_000).into(),
                minimum_deposit: U256::from(1).into(),
                delegation_delay: 1,
                ..Default::default()
            },
            FinalChainRewardsConfig {
                cornus_delegation_locking_period: 1,
                dpos_delegation_locking_period: 1,
                cacti_delegation_locking_period: 1,
                ..Default::default()
            },
        )
        .expect("construct configured FinalChain")
    }

    fn assert_genesis_metadata(chain: &mut FinalChain, reader: &FixtureReader) {
        let snapshot = chain
            .account_snapshots
            .get_mut()
            .unwrap()
            .get(&FinalChainBlockNumber::GENESIS)
            .unwrap();
        assert_eq!(snapshot.len(), reader.accounts.len());
        for (address, record) in &reader.accounts {
            let actual = &snapshot[address];
            assert_eq!(actual.nonce, record.account.nonce);
            assert_eq!(
                *actual.balance.as_u256(),
                U256::from_big_endian(&record.account.balance.value().to_bytes_be())
            );
            assert_eq!(
                actual.storage_root_hash,
                record.account.storage_root.unwrap_or_default()
            );
            assert_eq!(
                actual.code_hash,
                record.account.code_hash.unwrap_or_default()
            );
            assert_eq!(actual.code_size, record.account.code_size);
        }
    }

    fn snapshot_maps(
        chain: &mut FinalChain,
    ) -> (HashMap<[u8; 20], Account>, HashMap<[u8; 20], Account>) {
        (
            chain.accounts.get_mut().unwrap().clone(),
            chain.account_snapshots.get_mut().unwrap()[&FinalChainBlockNumber::GENESIS].clone(),
        )
    }

    #[test]
    fn hydrates_exact_genesis_metadata_fresh_and_after_nonzero_head_reopen() {
        let oracle = fixture();
        let reader = fixture_reader(&oracle);
        assert_eq!(reader.accounts.len(), 17);
        let path = temp_path("fresh-reopen");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut chain = configured_chain(storage.clone(), &oracle);

        chain.hydrate_concrete_genesis_accounts(&reader).unwrap();
        assert_genesis_metadata(&mut chain, &reader);
        assert_eq!(
            *chain.accounts.get_mut().unwrap(),
            chain.account_snapshots.get_mut().unwrap()[&FinalChainBlockNumber::GENESIS]
        );

        let mut head_accounts = chain.accounts.get_mut().unwrap().clone();
        let sender: [u8; 20] = fixed_hex(&oracle["inputs"], "sender");
        head_accounts.get_mut(&sender).unwrap().balance =
            rustaxa_types::FinalChainAccountBalance::new_account(U256::from(999_999u64));
        storage
            .final_chain()
            .write_block_header_with_snapshots(
                1,
                H256::from_low_u64_be(1),
                &[0xc0],
                &[0xc0],
                None,
                Some(&encode_account_snapshot_rlp(&head_accounts)),
            )
            .unwrap();
        drop(chain);
        drop(storage);

        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut reopened = configured_chain(storage.clone(), &oracle);
        assert_eq!(reopened.last_block_number().unwrap(), 1);
        let live_before = reopened.accounts.get_mut().unwrap().clone();
        assert_eq!(
            *live_before[&sender].balance.as_u256(),
            U256::from(999_999u64)
        );
        reopened.hydrate_concrete_genesis_accounts(&reader).unwrap();
        assert_genesis_metadata(&mut reopened, &reader);
        assert_eq!(*reopened.accounts.get_mut().unwrap(), live_before);
        assert_eq!(
            *reopened.latest_account_snapshot_block.get_mut().unwrap(),
            FinalChainBlockNumber::new(1)
        );

        drop(reopened);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn rejects_root_nonce_code_and_pending_without_partial_mutation() {
        let oracle = fixture();
        let reader = fixture_reader(&oracle);
        let path = temp_path("invalid");
        let storage = Arc::new(Storage::new(Config::new(path.clone())).unwrap());
        let mut chain = configured_chain(storage.clone(), &oracle);
        let before = snapshot_maps(&mut chain);

        let mut wrong_root = reader.clone();
        wrong_root.identity.state_root[0] ^= 1;
        assert!(chain
            .hydrate_concrete_genesis_accounts(&wrong_root)
            .unwrap_err()
            .to_string()
            .contains("ROOT_MISMATCH"));
        assert_eq!(snapshot_maps(&mut chain), before);

        let mut wrong_nonce = reader.clone();
        let first = *wrong_nonce.accounts.keys().next().unwrap();
        wrong_nonce.accounts.get_mut(&first).unwrap().account.nonce = FinalChainNonce::from_u64(9);
        assert!(chain
            .hydrate_concrete_genesis_accounts(&wrong_nonce)
            .unwrap_err()
            .to_string()
            .contains("NONCE_MISMATCH"));
        assert_eq!(snapshot_maps(&mut chain), before);

        let mut corrupt_code = reader.clone();
        corrupt_code.codes.values_mut().next().unwrap()[0] ^= 1;
        let code_error = chain
            .hydrate_concrete_genesis_accounts(&corrupt_code)
            .unwrap_err()
            .to_string();
        assert!(code_error.contains("CODE_HASH_MISMATCH"), "{code_error}");
        assert_eq!(snapshot_maps(&mut chain), before);

        storage
            .final_chain()
            .write_external_evm_pending_publication(FinalChainExternalEvmPendingPublication {
                payload: &[1],
            })
            .unwrap();
        assert!(chain
            .hydrate_concrete_genesis_accounts(&reader)
            .unwrap_err()
            .to_string()
            .contains("PENDING_PUBLICATION"));
        assert_eq!(snapshot_maps(&mut chain), before);

        drop(chain);
        drop(storage);
        std::fs::remove_dir_all(path).unwrap();
    }
}
