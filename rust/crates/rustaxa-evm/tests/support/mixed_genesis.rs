//! Test-only bootstrap helpers for the mixed Go genesis observer fixture.
//!
//! The helpers construct the Rust FinalChain from the fixture's configured
//! effective allocations and independently create the concrete state from the
//! observed Go account, code and raw-slot rows.  They deliberately do not
//! hydrate FinalChain from the concrete reader: recovery owns that sequencing.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{ensure, Context, Result};
use ethereum_types::{H256, U256};
use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::RlpStream;
use rustaxa_consensus::{
    FinalChain, FinalChainRewardsConfig, GenesisAccount, GenesisDposConfig, GenesisValidator,
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteStateLifecycle,
    ConcreteStateMutationBatch, ConcreteStorageMutation, Config, Storage,
};
use rustaxa_types::{
    concrete_lifecycle::ConcreteStorageSlot,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteStorageKey,
    },
    FinalChainAccountBalance, FinalChainBlockNumber, FinalChainNonce, GenesisValidatorMetadata,
};
use serde_json::Value;

const DPOS: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];
const FIXTURE_GENESIS_HASH: [u8; 32] = [7; 32];

/// Opens an empty FinalChain database with the fixture's configured genesis.
///
/// On a fresh `path`, the helper initializes the fixture's fixed genesis
/// metadata hash. On an existing path, it first requires that same metadata
/// hash before constructing the chain; it never adopts a foreign database.
/// `fixture` supplies configuration, allocations and validator inputs. The
/// returned chain has no concrete-account hydration; callers must complete
/// recovery before separately hydrating it from the concrete reader.
pub(super) fn open_chain(path: &Path, fixture: &Value) -> Result<(Arc<Storage>, FinalChain)> {
    let configuration = object(fixture, "configuration")?;
    let inputs = object(fixture, "inputs")?;
    let genesis = object(fixture, "genesis")?;
    let dpos = object(configuration, "dpos")?;
    let hardforks = object(configuration, "hardforks")?;
    let initial_validator = object(inputs, "initial_validator")?;

    let original_allocations = array(inputs, "genesis_allocations")?
        .iter()
        .map(|row| Ok((fixed_hex(row, "address")?, decimal(row, "balance")?)))
        .collect::<Result<BTreeMap<[u8; 20], U256>>>()?;
    let mut effective_allocations = original_allocations.clone();
    let delegations = array(initial_validator, "delegations")?
        .iter()
        .map(|row| Ok((fixed_hex(row, "delegator")?, decimal(row, "amount")?)))
        .collect::<Result<Vec<([u8; 20], U256)>>>()?;
    let total_stake = delegations
        .iter()
        .try_fold(U256::zero(), |total, (_, amount)| {
            total
                .checked_add(*amount)
                .context("fixture genesis stake sum overflows uint256")
        })?;
    for (delegator, amount) in &delegations {
        let balance = effective_allocations.get_mut(delegator).with_context(|| {
            format!(
                "delegator {} lacks configured allocation",
                hex::encode(delegator)
            )
        })?;
        *balance = balance.checked_sub(*amount).with_context(|| {
            format!(
                "delegator {} allocation is below its delegation",
                hex::encode(delegator)
            )
        })?;
    }

    let mut genesis_accounts = Vec::new();
    for row in array(object(genesis, "native_catalog")?, "accounts")? {
        if row.get("present") != Some(&Value::Bool(true)) {
            continue;
        }
        let address = fixed_hex(row, "address")?;
        let observed = decimal(row, "balance")?;
        let configured = if address == DPOS {
            // The constructor credits total stake exactly once. A real configured
            // DPoS allocation remains an independent input and is retained.
            original_allocations
                .get(&address)
                .copied()
                .unwrap_or_default()
        } else if let Some(balance) = effective_allocations.get(&address) {
            *balance
        } else {
            // Go can install a zero-balance code account. Keep it as an
            // explicit FinalChain input, but never synthesize a positive
            // allocation outside the configured genesis allocation set.
            ensure!(
                observed.is_zero(),
                "Go-present account {} lacks configured allocation",
                hex::encode(address)
            );
            U256::zero()
        };
        let expected = if address == DPOS {
            configured
                .checked_add(total_stake)
                .context("configured DPoS allocation plus stake overflows uint256")?
        } else {
            configured
        };
        ensure!(
            expected == observed,
            "effective allocation differs from Go genesis account {}",
            hex::encode(address)
        );
        genesis_accounts.push(GenesisAccount {
            address,
            balance: account_balance(configured)?,
        });
    }

    let validator = GenesisValidator {
        address: fixed_hex(initial_validator, "address")?,
        vrf_key: fixed_hex(initial_validator, "vrf_key")?,
        total_stake: total_stake.to_big_endian().to_vec(),
        delegations: delegations
            .iter()
            .map(|(delegator, amount)| (*delegator, amount.to_big_endian().to_vec()))
            .collect(),
        metadata: GenesisValidatorMetadata {
            owner: fixed_hex(initial_validator, "owner")?,
            commission: number(initial_validator, "commission")?
                .try_into()
                .context("commission fits u16")?,
            description: String::new(),
            endpoint: String::new(),
        },
    };
    let genesis_dpos = GenesisDposConfig {
        eligibility_balance_threshold: decimal(dpos, "eligibility_balance_threshold")?.into(),
        vote_eligibility_balance_step: decimal(dpos, "vote_eligibility_balance_step")?.into(),
        validator_maximum_stake: decimal(dpos, "validator_maximum_stake")?.into(),
        minimum_deposit: decimal(dpos, "minimum_deposit")?.into(),
        commission_change_delta: number(dpos, "commission_change_delta")?
            .try_into()
            .context("commission delta fits u16")?,
        commission_change_frequency: number(dpos, "commission_change_frequency")?
            .try_into()
            .context("commission frequency fits u32")?,
        delegation_delay: number(dpos, "delegation_delay")?,
        dag_vdf_sortition_total_vote_count_until_period: FinalChainBlockNumber::GENESIS,
    };
    let aspen = object(hardforks, "aspen")?;
    let cornus = object(hardforks, "cornus")?;
    let magnolia = object(hardforks, "magnolia")?;
    let cacti = object(hardforks, "cacti")?;
    let rewards = FinalChainRewardsConfig {
        committee_size: number(configuration, "committee_size")?
            .try_into()
            .context("committee size fits u32")?,
        magnolia_period: number(magnolia, "block")?.into(),
        phalaenopsis_period: number(hardforks, "phalaenopsis_block")?.into(),
        aspen_part_one_period: number(aspen, "part_one_block")?.into(),
        fix_claim_all_block_num: number(hardforks, "fix_claim_all_block")?.into(),
        fix_redelegate_block_num: number(hardforks, "fix_redelegate_block")?.into(),
        aspen_part_two_period: checked_u64(
            decimal(aspen, "part_two_block")?,
            "Aspen part two period",
        )?
        .into(),
        max_block_author_reward_percent: number(dpos, "max_block_author_reward_percent")?
            .try_into()
            .context("author reward fits u16")?,
        dag_proposers_reward_percent: number(dpos, "dag_proposers_reward_percent")?
            .try_into()
            .context("DAG reward fits u16")?,
        yield_percentage: number(dpos, "yield_percentage")?
            .try_into()
            .context("yield fits u16")?,
        dpos_blocks_per_year: number(dpos, "blocks_per_year")?
            .try_into()
            .context("blocks per year fits u32")?,
        dpos_delegation_locking_period: number(dpos, "delegation_locking_period")?,
        cornus_period: number(cornus, "block")?.into(),
        cornus_delegation_locking_period: number(cornus, "delegation_locking_period")?,
        genesis_balance_sum: Some(
            original_allocations
                .values()
                .try_fold(U256::zero(), |sum, value| {
                    sum.checked_add(*value)
                        .context("configured genesis allocation sum overflows uint256")
                })?
                .into(),
        ),
        aspen_max_supply: decimal(aspen, "max_supply")?.into(),
        aspen_generated_rewards: decimal(aspen, "generated_rewards")?.into(),
        cacti_period: checked_u64(decimal(cacti, "block")?, "Cacti period")?.into(),
        cacti_delegation_locking_period: number(cacti, "delegation_locking_period")?,
        magnolia_jail_time: number(magnolia, "jail_time")?,
        cacti_jail_time: number(cacti, "jail_time")?,
        rewards_distribution_frequency: vec![(FinalChainBlockNumber::GENESIS, 1)],
        ..Default::default()
    };

    let existing_path = path.exists();
    let storage = Arc::new(Storage::new(Config::new(path.to_path_buf()))?);
    match storage.metadata().genesis_hash()? {
        Some(existing) => ensure!(
            existing == FIXTURE_GENESIS_HASH,
            "existing application metadata has a different genesis hash"
        ),
        None if !existing_path => storage
            .metadata()
            .set_genesis_hash_if_empty(&FIXTURE_GENESIS_HASH)?,
        None => anyhow::bail!("existing application database has no genesis metadata"),
    }
    let chain = FinalChain::new_with_genesis_state_root(
        storage.clone(),
        number(configuration, "block_gas_limit")?.into(),
        0,
        H256(fixed_hex(genesis, "root")?),
        true,
        genesis_accounts,
        vec![validator],
        genesis_dpos,
        rewards,
    )?;
    Ok((storage, chain))
}

/// Creates a fresh concrete database from actual observer rows and verifies its root.
///
/// `path` must be absent. `chain` supplies the derived concrete-chain identity.
/// The helper uses logical raw keys from `native_catalog`, not hashed trie paths,
/// and passes no expected root to storage before comparing the computed result.
pub(super) fn fresh_concrete(
    path: &Path,
    chain: &FinalChain,
    fixture: &Value,
) -> Result<ConcreteStateLifecycle> {
    let genesis = object(fixture, "genesis")?;
    let mut accounts = Vec::new();
    let mut code = BTreeMap::new();
    for row in array(object(genesis, "native_catalog")?, "accounts")? {
        if row.get("present") != Some(&Value::Bool(true)) {
            continue;
        }
        let address = fixed_hex(row, "address")?;
        let code_bytes = bytes(row, "code")?;
        let code_hash = optional_fixed_hex(row, "code_hash")?;
        ensure!(
            code_hash
                .map(|hash| keccak256(&code_bytes).0 == hash)
                .unwrap_or(code_bytes.is_empty()),
            "code hash disagrees with Go code for {}",
            hex::encode(address)
        );
        if let Some(hash) = code_hash {
            match code.entry(hash) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(code_bytes);
                }
                std::collections::btree_map::Entry::Occupied(entry) => ensure!(
                    entry.get() == &code_bytes,
                    "same Go code hash has conflicting bytes"
                ),
            }
        } else {
            ensure!(
                code_bytes.is_empty(),
                "code without a hash for {}",
                hex::encode(address)
            );
        }
        let record = record(
            nonce(row)?,
            BigUint::parse_bytes(string(row, "balance")?.as_bytes(), 10)
                .context("Go account balance is decimal")?,
            code_hash,
            number(row, "code_size")?,
        );
        if row.get("storage_root") == Some(&Value::Null) {
            ensure!(
                record.physical_rlp == bytes(row, "raw_account")?,
                "reconstructed Go empty-root account differs for {}: rust {}, Go {}",
                hex::encode(address),
                hex::encode(&record.physical_rlp),
                string(row, "raw_account")?
            );
        }
        accounts.push(ConcreteAccountMutation::Upsert { address, record });
    }
    let slots = array(object(genesis, "native_catalog")?, "slots")?;
    let mut storage = Vec::with_capacity(slots.len());
    let mut catalog = Vec::with_capacity(slots.len());
    for row in slots {
        ensure!(
            row.get("present") == Some(&Value::Bool(true)),
            "genesis catalog has non-present slot"
        );
        let address = fixed_hex(row, "address")?;
        let key = ConcreteStorageKey(fixed_hex(row, "key")?);
        let value = bytes(row, "value")?;
        ensure!(!value.is_empty(), "live native catalog value is empty");
        storage.push(ConcreteStorageMutation {
            address,
            key,
            value: Some(value),
        });
        catalog.push(ConcreteStorageSlot {
            address,
            key: key.0,
        });
    }
    let concrete = ConcreteStateLifecycle::create_fresh_exclusive(
        path,
        chain.concrete_chain_identity()?,
        ConcreteStateMutationBatch {
            accounts,
            storage,
            code: code
                .into_iter()
                .map(|(code_hash, code)| ConcreteCodeInsertion { code_hash, code })
                .collect(),
        },
        catalog,
    )?;
    let computed_root = concrete.observation()?.committed.state_root;
    let observed_root = fixed_hex(genesis, "root")?;
    ensure!(
        computed_root == observed_root,
        "computed concrete genesis root {} differs from Go observer root {}",
        hex::encode(computed_root),
        hex::encode(observed_root)
    );
    Ok(concrete)
}

fn record(
    nonce: FinalChainNonce,
    balance: BigUint,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
) -> ConcreteAccountRecord {
    let mut raw = RlpStream::new_list(5);
    raw.append(&nonce.to_bytes());
    let balance_bytes = if balance == BigUint::default() {
        Vec::new()
    } else {
        balance.to_bytes_be()
    };
    raw.append(&balance_bytes);
    // The writer derives this from the supplied logical raw slots.
    raw.append_empty_data();
    match code_hash {
        Some(hash) => raw.append(&hash.as_slice()),
        None => raw.append_empty_data(),
    };
    raw.append(&code_size);
    ConcreteAccountRecord {
        account: ConcreteAccount {
            nonce,
            balance: ConcreteAccountBalance::new(balance),
            storage_root: None,
            code_hash,
            code_size,
        },
        physical_rlp: raw.out().to_vec(),
    }
}

fn account_balance(value: U256) -> Result<FinalChainAccountBalance> {
    FinalChainAccountBalance::from_cpp_genesis_bytes(&value.to_big_endian())
        .context("fixture balance fits FinalChain genesis boundary")
}

fn object<'a>(value: &'a Value, field: &str) -> Result<&'a Value> {
    value
        .get(field)
        .filter(|value| value.is_object())
        .with_context(|| format!("{field} must be an object"))
}

fn array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .with_context(|| format!("{field} must be an array"))
}

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("{field} must be a string"))
}

fn number(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .with_context(|| format!("{field} must be a u64"))
}

fn decimal(value: &Value, field: &str) -> Result<U256> {
    U256::from_dec_str(string(value, field)?)
        .with_context(|| format!("{field} must be uint256 decimal"))
}

fn checked_u64(value: U256, field: &str) -> Result<u64> {
    ensure!(value <= U256::from(u64::MAX), "{field} exceeds u64");
    Ok(value.low_u64())
}

fn nonce(row: &Value) -> Result<FinalChainNonce> {
    let value = BigUint::parse_bytes(string(row, "nonce")?.as_bytes(), 10)
        .context("Go account nonce is decimal")?;
    let bytes = if value == BigUint::default() {
        Vec::new()
    } else {
        value.to_bytes_be()
    };
    FinalChainNonce::from_bytes(&bytes).context("Go account nonce is canonical")
}

fn bytes(value: &Value, field: &str) -> Result<Vec<u8>> {
    hex::decode(string(value, field)?).with_context(|| format!("{field} must be hexadecimal"))
}

fn fixed_hex<const N: usize>(value: &Value, field: &str) -> Result<[u8; N]> {
    let bytes = bytes(value, field)?;
    bytes.try_into().map_err(|bytes: Vec<u8>| {
        anyhow::anyhow!("{field} must contain {N} bytes, got {}", bytes.len())
    })
}

fn optional_fixed_hex<const N: usize>(value: &Value, field: &str) -> Result<Option<[u8; N]>> {
    match value.get(field) {
        Some(Value::String(raw)) => {
            let bytes = hex::decode(raw).with_context(|| format!("{field} must be hexadecimal"))?;
            Ok(Some(bytes.try_into().map_err(|bytes: Vec<u8>| {
                anyhow::anyhow!("{field} must contain {N} bytes, got {}", bytes.len())
            })?))
        }
        Some(Value::Null) | None => Ok(None),
        Some(_) => anyhow::bail!("{field} must be a string or null"),
    }
}
