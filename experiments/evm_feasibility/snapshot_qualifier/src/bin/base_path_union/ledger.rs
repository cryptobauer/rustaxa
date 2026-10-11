//! Exact offline base reconstruction kernel. Inputs are fixed admitted evidence,
//! not a physical reader. Saved non-live statuses remain saved classifications.
use anyhow::{Context, Result, bail, ensure};
use num_bigint::BigUint;
#[cfg(test)]
use rustaxa_snapshot_qualifier::native_inverse as native;
use rustaxa_snapshot_qualifier::native_inverse::base_rows as codec;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use tiny_keccak::{Hasher, Keccak};
const PERIOD: u64 = 25706948;
const ACCOUNT: &str = "00000000000000000000000000000000000000fe";
const ROOT: &str = "926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2";
const STORAGE: &str = "c62767fbe35b09e45975d25132970f8bc5e6614b41e3f3ea51e1f25b77dcf504";
const RAW_DIGEST: &str = "a1b2bc6fe3c31e8fde3e597338e2c63a536a4c7badb4de3bcf44a304bc65765c";
fn str_field<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().with_context(|| format!("string field {k}"))
}
fn number(v: &Value, k: &str) -> Result<u64> {
    v[k].as_u64().with_context(|| format!("unsigned field {k}"))
}
fn array<'a>(v: &'a Value, k: &str, count: usize) -> Result<&'a [Value]> {
    let a = v[k]
        .as_array()
        .with_context(|| format!("array field {k}"))?;
    ensure!(a.len() == count, "{k} cardinality");
    Ok(a)
}
fn hex_bytes(s: &str, cap: usize) -> Result<Vec<u8>> {
    ensure!(
        s.len() <= cap * 2 && s.len().is_multiple_of(2),
        "hex byte cap/shape"
    );
    ensure!(
        s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "noncanonical hex"
    );
    Ok(hex::decode(s)?)
}
fn fixed<const N: usize>(s: &str) -> Result<[u8; N]> {
    hex_bytes(s, N)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("fixed hex width {N}"))
}
fn amount(v: &Value, k: &str) -> Result<BigUint> {
    let raw = hex_bytes(str_field(v, k)?, 4096)?;
    ensure!(raw.first() != Some(&0), "nonminimal amount {k}");
    Ok(BigUint::from_bytes_be(&raw))
}
fn hash(bytes: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();
    k.update(bytes);
    let mut out = [0; 32];
    k.finalize(&mut out);
    out
}
fn compact(n: u64) -> Vec<u8> {
    let b = n.to_be_bytes();
    b[b.iter().position(|x| *x != 0).unwrap_or(8)..].to_vec()
}
/// Admitted ordered raw rows, preserving saved row indexes and exact bytes.
pub(super) struct Raw {
    pub rows: BTreeMap<[u8; 32], (usize, Vec<u8>)>,
}
impl Raw {
    pub fn admit(v: &Value) -> Result<Self> {
        ensure!(
            v["period"] == PERIOD
                && v["account_hex"] == ACCOUNT
                && v["state_root_hex"] == ROOT
                && v["storage_root_hex"] == STORAGE,
            "raw identity"
        );
        ensure!(
            v["row_count"] == 23278
                && v["value_bytes"] == 259077
                && v["entries_sha256"] == RAW_DIGEST,
            "raw manifest"
        );
        let entries = array(v, "entries", 23278)?;
        let mut rows = BTreeMap::new();
        let mut previous = None;
        let mut bytes = 0usize;
        let mut digest = Sha256::new();
        for (i, e) in entries.iter().enumerate() {
            let path = fixed(str_field(e, "hashed_path_hex")?)?;
            ensure!(
                previous.is_none_or(|p| p < path),
                "raw row order/duplicate at {i}"
            );
            previous = Some(path);
            let encoded = str_field(e, "value_hex")?;
            ensure!(
                encoded.len() / 2 <= 259077usize.saturating_sub(bytes),
                "aggregate raw byte cap"
            );
            let value = hex_bytes(encoded, 259077)?;
            bytes += value.len();
            digest.update(path);
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(&value);
            rows.insert(path, (i, value));
        }
        ensure!(
            bytes == 259077 && hex::encode(digest.finalize()) == RAW_DIGEST,
            "raw byte count/digest"
        );
        Ok(Self { rows })
    }
}
/// One matched path with its original family and all justified references.
#[derive(Clone, Debug)]
pub(super) struct Row {
    pub preimage: Vec<u8>,
    pub key: [u8; 32],
    pub refs: Vec<Value>,
    pub family: String,
}
/// In-memory reconstruction state; no physical callback or production authority.
#[derive(Clone)]
pub(super) struct Base {
    pub rows: BTreeMap<[u8; 32], Row>,
    pub lookups: usize,
    pub bytes: usize,
    families: BTreeMap<String, u64>,
    repeated_bytes: usize,
}
impl Base {
    fn new() -> Self {
        Self {
            rows: BTreeMap::new(),
            lookups: 0,
            bytes: 0,
            families: BTreeMap::new(),
            repeated_bytes: 0,
        }
    }
    fn visit(
        &mut self,
        raw: &Raw,
        preimage: Vec<u8>,
        family: &str,
        selector: Value,
        status: &str,
        check: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<()> {
        let reward = matches!(
            family,
            "current_reward_node" | "seeded_delegation_reward_node"
        );
        self.lookups += 1;
        ensure!(self.lookups <= 2296, "base derivation/lookup cap");
        let key = hash(&preimage);
        let path = hash(&key);
        let row = raw.rows.get(&path);
        if !matches!(
            status,
            "present" | "live_present" | "live_present_already_matched"
        ) {
            ensure!(
                matches!(
                    status,
                    "absent" | "tombstone" | "history_unavailable" | "pruned"
                ),
                "unsupported saved status {status}"
            );
            ensure!(
                row.is_none(),
                "saved non-live path is live: {}",
                hex::encode(path)
            );
            return Ok(());
        }
        let (index, value) =
            row.with_context(|| format!("missing live {family} path {}", hex::encode(path)))?;
        check(value).with_context(|| format!("typed {family} equality at {selector}"))?;
        let reference = json!({"selector":selector,"saved_status":status,"raw_row_index":index});
        if let Some(previous) = self.rows.get_mut(&path) {
            ensure!(
                reward && status == "live_present_already_matched",
                "unexpected ordinary/new duplicate"
            );
            ensure!(
                matches!(
                    previous.family.as_str(),
                    "current_reward_node" | "seeded_delegation_reward_node"
                ) && previous.preimage == preimage
                    && previous.key == key,
                "reward path collision/family"
            );
            self.repeated_bytes += value.len();
            ensure!(
                self.repeated_bytes <= 1024 * 1024,
                "repeated reference byte cap"
            );
            previous.refs.push(reference);
        } else {
            ensure!(
                status != "live_present_already_matched",
                "repeated status without earlier reward"
            );
            self.bytes += value.len();
            ensure!(self.bytes <= 35694, "base matched byte cap");
            *self.families.entry(family.into()).or_default() += 1;
            self.rows.insert(
                path,
                Row {
                    preimage,
                    key,
                    refs: vec![reference],
                    family: family.into(),
                },
            );
        }
        Ok(())
    }
}
fn pre(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}
fn equal(good: bool) -> Result<()> {
    ensure!(good, "canonical raw fields differ from saved facts");
    Ok(())
}
fn reward(
    base: &mut Base,
    raw: &Raw,
    v: [u8; 20],
    cursor: u64,
    fact: &Value,
    family: &str,
    selector: Value,
) -> Result<()> {
    let status_field = if family == "current_reward_node" {
        "current_reward_node_result"
    } else {
        "reward_node_result"
    };
    let status = str_field(fact, status_field)?;
    if !matches!(
        status,
        "present" | "live_present" | "live_present_already_matched"
    ) {
        ensure!(
            fact["reward_node_count"].is_null() && fact["reward_per_stake_hex"].is_null(),
            "non-live reward fields must be null"
        );
    }
    base.visit(
        raw,
        pre(&[&[1], &v, &compact(cursor)]),
        family,
        selector,
        status,
        |b| {
            let row = codec::reward_node(b)?;
            equal(
                row.count == u32::try_from(number(fact, "reward_node_count")?)?
                    && row.reward_per_stake == amount(fact, "reward_per_stake_hex")?,
            )
        },
    )
}
/// Rebuild the fixed P base in original attribution order. Exact counts, family
/// totals and full residual path/entry digests must pass; no U or union claim.
pub(super) fn rebuild(availability: &Value, raw: &Raw) -> Result<Base> {
    let dependencies = availability["native_dependencies"]
        .as_array()
        .context("dependencies")?;
    let selected: Vec<_> = dependencies
        .iter()
        .filter(|d| d["period"] == PERIOD)
        .collect();
    ensure!(selected.len() == 1, "P dependency selector");
    let d = selected[0];
    ensure!(
        d["root_authenticated"] == true
            && d["state_root_hex"] == ROOT
            && d["header_sha256"]
                == "1e9c7f4e77cbbabad4ec2f8b065332aa998c881dff503c2fcbcfb0d21005adac",
        "P identity/header"
    );
    let accounts = d["accounts"].as_array().context("accounts")?;
    let selected: Vec<_> = accounts
        .iter()
        .filter(|a| a["address_hex"] == ACCOUNT)
        .collect();
    ensure!(selected.len() == 1, "FE selector");
    let a = selected[0];
    ensure!(
        a["storage_root_hex"] == STORAGE
            && a["status"] == "complete_live_inventory"
            && a["entries_sha256"] == RAW_DIGEST
            && a["inverse"]["status"] == "partial_inverse_completed",
        "FE/inverse identity"
    );
    let facts = &a["inverse"]["coverage"];
    ensure!(facts["validator_count"] == 196, "validator count");
    let validators = array(facts, "validators", 196)?;
    let mut base = Base::new();
    base.visit(
        raw,
        vec![0, 5, 1],
        "validator_index_count",
        json!({"global_count":true}),
        "present",
        |b| equal(codec::index(b)? == 196),
    )?;
    let mut roles: BTreeMap<[u8; 20], BTreeSet<&str>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (i, v) in validators.iter().enumerate() {
        let position = u32::try_from(i + 1)?;
        ensure!(
            number(v, "position")? == position as u64,
            "validator position order"
        );
        let address = fixed(str_field(v, "address_hex")?)?;
        ensure!(seen.insert(address), "duplicate validator");
        let owner = fixed(str_field(v, "owner_hex")?)?;
        roles.entry(address).or_default().insert("validator");
        roles.entry(owner).or_default().insert("owner");
        base.visit(
            raw,
            pre(&[&[0, 5, 2], &position.to_le_bytes()]),
            "validator_index_item",
            json!({"validator_position":position}),
            "present",
            |b| equal(codec::address(b)? == address),
        )?;
        base.visit(
            raw,
            pre(&[&[0, 5, 2], &address]),
            "validator_index_reverse",
            json!({"validator_position":position}),
            "present",
            |b| equal(codec::index(b)? == position),
        )?;
        for (field, family) in [
            (0, "validator_record"),
            (1, "validator_info"),
            (2, "validator_rewards"),
            (3, "validator_owner"),
            (4, "validator_vrf"),
        ] {
            base.visit(
                raw,
                pre(&[&[0, field], &address]),
                family,
                json!({"validator_position":position}),
                "present",
                |b| match field {
                    0 => {
                        let r = codec::validator(b)?;
                        let encoding = match r.encoding {
                            codec::ValidatorEncoding::V1 => "validator_v1",
                            codec::ValidatorEncoding::ExtendedV1 => {
                                "embedded_validator_v1_with_count"
                            }
                        };
                        let count = if v["undelegations_count"].is_null() {
                            None
                        } else {
                            Some(u16::try_from(number(v, "undelegations_count")?)?)
                        };
                        equal(
                            encoding == str_field(v, "validator_encoding")?
                                && r.stake == amount(v, "stake_hex")?
                                && r.commission == u16::try_from(number(v, "commission")?)?
                                && r.last_commission_change == number(v, "last_commission_change")?
                                && r.reward_head == number(v, "reward_head")?
                                && r.undelegations_count == count,
                        )
                    }
                    1 => {
                        let r = codec::info(b)?;
                        equal(
                            r.description == hex_bytes(str_field(v, "description_hex")?, 4096)?
                                && r.endpoint == hex_bytes(str_field(v, "endpoint_hex")?, 4096)?,
                        )
                    }
                    2 => {
                        let r = codec::rewards(b)?;
                        equal(
                            r.delegator == amount(v, "delegator_rewards_hex")?
                                && r.commission == amount(v, "commission_rewards_hex")?,
                        )
                    }
                    3 => equal(codec::address(b)? == owner),
                    4 => equal(codec::vrf(b)? == fixed(str_field(v, "vrf_hex")?)?),
                    _ => bail!("unsupported payload field"),
                },
            )?;
        }
        let reward_fact = json!({"current_reward_node_result":v["current_reward_node_result"],"reward_node_count":v["current_reward_node_count"],"reward_per_stake_hex":v["current_reward_per_stake_hex"]});
        reward(
            &mut base,
            raw,
            address,
            number(v, "reward_head")?,
            &reward_fact,
            "current_reward_node",
            json!({"validator_position":position}),
        )?;
    }
    for (i, v) in array(facts, "scalars", 5)?.iter().enumerate() {
        let field = u8::try_from(number(v, "field")?)?;
        ensure!(field == i as u8 + 4, "scalar field order");
        ensure!(
            str_field(v, "name")?
                == [
                    "total_eligible_votes",
                    "total_staked",
                    "minted_tokens",
                    "total_supply",
                    "yield"
                ][i],
            "scalar name/field mismatch"
        );
        let kind = codec::ScalarField::try_from(field)?;
        let status = str_field(v, "physical_result")?;
        if status != "present" {
            ensure!(v["value_hex"].is_null(), "non-live scalar must be null");
        }
        base.visit(
            raw,
            vec![field],
            "global_scalar",
            json!({"field":field}),
            status,
            |b| {
                let r = codec::scalar(kind, b)?;
                let n = match r {
                    codec::Scalar::EligibleVotes(n) | codec::Scalar::Yield(n) => BigUint::from(n),
                    codec::Scalar::TotalStake(n)
                    | codec::Scalar::MintedTokens(n)
                    | codec::Scalar::TotalSupply(n) => n,
                };
                equal(n == amount(v, "value_hex")?)
            },
        )?;
    }
    let seeded = &facts["seeded_delegations"];
    ensure!(
        seeded["candidate_count"] == 290 && seeded["live_delegation_entries"] == 108,
        "seeded counts"
    );
    let candidates = array(seeded, "candidates", 290)?;
    let delegations = array(seeded, "delegations", 108)?;
    ensure!(roles.len() == 290, "validator/owner address union");
    let mut histogram = BTreeMap::<String, u64>::new();
    let mut visited = 0usize;
    for ((expected, expected_roles), c) in roles.iter().zip(candidates) {
        let delegator = fixed(str_field(c, "address_hex")?)?;
        ensure!(&delegator == expected, "candidate order/membership");
        let sources = c["sources"].as_array().context("candidate sources")?;
        ensure!(
            sources.len() == expected_roles.len()
                && sources
                    .iter()
                    .map(|x| x.as_str().context("source role"))
                    .collect::<Result<BTreeSet<_>>>()?
                    == *expected_roles,
            "candidate roles"
        );
        let status = str_field(c, "delegation_count_result")?;
        *histogram.entry(status.into()).or_default() += 1;
        let children: Vec<_> = delegations
            .iter()
            .filter(|x| x["delegator_hex"] == c["address_hex"])
            .collect();
        let prefix = pre(&[&[2, 1], &delegator]);
        let count = if status == "live_present" {
            u32::try_from(number(c, "delegation_count")?)?
        } else {
            ensure!(
                c["delegation_count"].is_null() && children.is_empty(),
                "non-live count/children"
            );
            0
        };
        ensure!(
            children.len() == count as usize,
            "delegation count/children disagreement"
        );
        base.visit(
            raw,
            pre(&[&prefix, &[1]]),
            "seeded_delegation_count",
            json!({"delegator_hex":c["address_hex"]}),
            status,
            |b| equal(codec::index(b)? == count),
        )?;
        let mut child_validators = BTreeSet::new();
        for (i, child) in children.iter().enumerate() {
            visited += 1;
            let position = u32::try_from(i + 1)?;
            ensure!(
                number(child, "position")? == position as u64,
                "delegation position order"
            );
            let validator = fixed(str_field(child, "validator_hex")?)?;
            ensure!(
                child_validators.insert(validator),
                "duplicate delegated validator"
            );
            let selector = json!({"delegator_hex":c["address_hex"],"position":position});
            base.visit(
                raw,
                pre(&[&prefix, &[2], &position.to_le_bytes()]),
                "seeded_delegation_item",
                selector.clone(),
                "present",
                |b| equal(codec::address(b)? == validator),
            )?;
            base.visit(
                raw,
                pre(&[&prefix, &[2], &validator]),
                "seeded_delegation_reverse",
                selector.clone(),
                "present",
                |b| equal(codec::index(b)? == position),
            )?;
            base.visit(
                raw,
                pre(&[&[2, 0], &validator, &delegator]),
                "seeded_delegation_record",
                selector.clone(),
                "present",
                |b| {
                    let r = codec::delegation(b)?;
                    equal(
                        r.stake == amount(child, "stake_hex")?
                            && r.last_updated == number(child, "last_updated")?,
                    )
                },
            )?;
            reward(
                &mut base,
                raw,
                validator,
                number(child, "last_updated")?,
                child,
                "seeded_delegation_reward_node",
                selector,
            )?;
        }
    }
    ensure!(
        visited == 108
            && json!(histogram) == seeded["count_results"]
            && histogram.get("live_present") == Some(&103)
            && histogram.get("history_unavailable") == Some(&187),
        "candidate status histogram/unvisited children"
    );
    ensure!(
        base.rows.len() == 2065
            && base.bytes == 35694
            && json!(base.families) == facts["matched_by_family"],
        "base exact count/bytes/families"
    );
    let mut paths = Sha256::new();
    let mut entries = Sha256::new();
    let mut count = 0;
    let mut bytes = 0;
    let mut first = Vec::new();
    for (path, (_, value)) in &raw.rows {
        if !base.rows.contains_key(path) {
            count += 1;
            bytes += value.len();
            paths.update(path);
            entries.update(path);
            entries.update((value.len() as u64).to_be_bytes());
            entries.update(value);
            if first.len() < 16 {
                first.push(hex::encode(path));
            }
        }
    }
    ensure!(
        count == 21213
            && bytes == 223383
            && hex::encode(paths.finalize()) == str_field(facts, "unexplained_paths_sha256")?
            && hex::encode(entries.finalize()) == str_field(facts, "unexplained_entries_sha256")?
            && json!(first) == facts["first_unexplained_paths_hex"],
        "exact base residual mismatch"
    );
    Ok(base)
}
/// Verify accepted U's catalog/raw links, then measure actual path partitions.
/// Completeness remains false. Each set visits accepted raw rows exactly once.
pub(super) fn union_report(
    availability: &Value,
    raw: &Raw,
    base: &Base,
    u: &Value,
) -> Result<Value> {
    ensure!(
        u["schema"] == 1
            && u["status"] == "reachable_live_family_decoded"
            && u["period"] == PERIOD
            && u["account_hex"] == ACCOUNT
            && u["state_root_hex"] == ROOT
            && u["storage_root_hex"] == STORAGE
            && u["raw_entries_sha256"] == RAW_DIGEST,
        "U identity/schema"
    );
    ensure!(
        u["catalog_digest_version"] == "offline-undelegation-catalog-v1",
        "U digest version"
    );
    let tuples = array(u, "tuples", 1017)?;
    let mut digest = Sha256::new();
    digest.update(b"offline-undelegation-catalog-v1");
    digest.update(1017u32.to_be_bytes());
    let mut matched: BTreeMap<[u8; 32], (&Value, Vec<usize>)> = BTreeMap::new();
    let mut unresolved = 0usize;
    for (i, t) in tuples.iter().enumerate() {
        let encoded = serde_json::to_vec(t)?;
        digest.update((encoded.len() as u64).to_be_bytes());
        digest.update(encoded);
        ensure!(
            t["period"] == PERIOD
                && t["account_hex"] == ACCOUNT
                && t["raw_artifact_sha256"]
                    == "63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056",
            "U tuple identity/pin"
        );
        let preimage = hex_bytes(str_field(t, "preimage_hex")?, 128)?;
        let key = hash(&preimage);
        let path = hash(&key);
        ensure!(
            hex::encode(key) == str_field(t, "logical_key_hex")?
                && hex::encode(path) == str_field(t, "trie_path_hex")?,
            "U hash levels at {i}"
        );
        let row = raw.rows.get(&path);
        match str_field(t, "status")? {
            "validated_live" => {
                let (index, value) = row.with_context(|| format!("U missing live path at {i}"))?;
                ensure!(
                    number(t, "raw_row_index")? == *index as u64
                        && number(t, "raw_bytes")? == value.len() as u64
                        && str_field(t, "raw_hex")? == hex::encode(value)
                        && str_field(t, "raw_sha256")? == hex::encode(Sha256::digest(value)),
                    "U raw link at {i}"
                );
                if let Some((first, refs)) = matched.get_mut(&path) {
                    ensure!(
                        first["preimage_hex"] == t["preimage_hex"]
                            && first["logical_key_hex"] == t["logical_key_hex"],
                        "U path collision"
                    );
                    refs.push(i);
                } else {
                    matched.insert(path, (t, vec![i]));
                }
            }
            "unresolved_not_in_live_inventory" => {
                unresolved += 1;
                ensure!(row.is_none(), "U unresolved path is live");
                for field in [
                    "raw_row_index",
                    "raw_hex",
                    "raw_bytes",
                    "raw_sha256",
                    "decoded",
                ] {
                    ensure!(t[field].is_null(), "U unresolved field {field} not null");
                }
            }
            status => bail!("unsupported accepted U tuple status {status}"),
        }
    }
    ensure!(
        hex::encode(digest.finalize())
            == "894d4c638a61407167c0b868d92d11ff2e22c45ee55b701f5e5ce8dcdf2db8b6"
            && u["catalog_sha256"]
                == "894d4c638a61407167c0b868d92d11ff2e22c45ee55b701f5e5ce8dcdf2db8b6",
        "U catalog digest"
    );
    ensure!(
        matched.len() == 217 && unresolved == 800,
        "U cardinality/statuses"
    );
    let paths: Vec<_> = matched.keys().map(hex::encode).collect();
    ensure!(
        json!(paths) == u["coverage"]["matched_paths_hex"],
        "U exact path set"
    );
    let mut sets: [SetDigest; 8] = std::array::from_fn(|_| SetDigest::default());
    let mut overlaps = Vec::new();
    let mut base_ledger = Vec::new();
    let mut u_links = Vec::new();
    for (path, (index, value)) in &raw.rows {
        let b = base.rows.get(path);
        let t = matched.get(path);
        let membership = [
            b.is_some(),
            t.is_some(),
            b.is_some() && t.is_some(),
            b.is_some() || t.is_some(),
            b.is_some() && t.is_none(),
            b.is_none() && t.is_some(),
            b.is_none() && t.is_none(),
            b.is_none(),
        ];
        for (set, selected) in sets.iter_mut().zip(membership) {
            if selected {
                set.add(path, value);
            }
        }
        if let Some(b) = b {
            base_ledger.push(json!({"period":PERIOD,"state_root_hex":ROOT,"account_hex":ACCOUNT,"primary_family":b.family,"provenance":b.refs,"preimage_hex":hex::encode(&b.preimage),"logical_key_hex":hex::encode(b.key),"trie_path_hex":hex::encode(path),"expected_bytes":value.len(),"expected_sha256":hex::encode(Sha256::digest(value)),"raw_row_index":index,"raw_bytes":value.len(),"raw_sha256":hex::encode(Sha256::digest(value)),"raw_artifact_sha256":"63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056","canonical_typed_equality":true}));
        }
        if let Some((t, refs)) = t {
            u_links.push(json!({"trie_path_hex":hex::encode(path),"tuple_indexes":refs,"tuple_references":refs.iter().map(|i|tuples[*i].clone()).collect::<Vec<_>>(),"raw_row_index":index}));
            if let Some(b) = b {
                ensure!(
                    hex::encode(&b.preimage) == str_field(t, "preimage_hex")?
                        && hex::encode(b.key) == str_field(t, "logical_key_hex")?,
                    "B/U overlap key collision"
                );
                overlaps.push(json!({"trie_path_hex":hex::encode(path),"base_primary_family":b.family,"base_provenance":b.refs,"undelegation_tuple_indexes":refs,"raw_row_index":index,"raw_sha256":hex::encode(Sha256::digest(value))}));
            }
        }
    }
    ensure!(sets[1].bytes == 2277, "U matched value bytes");
    ensure!(
        sets[3].count == sets[0].count + sets[1].count - sets[2].count
            && sets[3].bytes == sets[0].bytes + sets[1].bytes - sets[2].bytes
            && sets[3].count + sets[6].count == 23278
            && sets[3].bytes + sets[6].bytes == 259077,
        "union partition/inclusion-exclusion"
    );
    let names = [
        "base",
        "undelegation",
        "intersection",
        "union",
        "base_only",
        "undelegation_only",
        "union_residual",
        "base_residual",
    ];
    let partitions: serde_json::Map<String, Value> = names
        .into_iter()
        .zip(sets)
        .map(|(n, s)| (n.to_string(), s.finish()))
        .collect();
    Ok(
        json!({"schema":1,"profile":"fixed-p-base-undelegation-path-union-v1","status":"exact_saved_path_union_verified","period":PERIOD,"state_root_hex":ROOT,"account_hex":ACCOUNT,"storage_root_hex":STORAGE,"raw_entries_sha256":RAW_DIGEST,"partitions":partitions,"base_ledger":base_ledger,"undelegation_links":u_links,"overlaps":overlaps,"accepted_undelegation_catalog_sha256":u["catalog_sha256"],"saved_inverse_facts":availability["native_dependencies"][0]["accounts"][0]["inverse"]["coverage"],"bounds":{"base_derivations_and_lookups":2296,"undelegation_tuple_lookups":1017,"total_derivations_and_link_lookups":3313,"raw_rows_per_partition_pass":23278,"raw_partition_passes":1,"output_bytes":16*1024*1024},"actual":{"base_derivations_and_lookups":base.lookups,"undelegation_tuple_lookups":tuples.len(),"raw_partition_iterations":raw.rows.len()},"runtime":{"database_opens":0,"storage_points":0,"new_inventories":0},"reference_policy_assumption":u["reference_policy_assumption"],"qualification":{"base_union_verified":true,"semantic_snapshot_complete":false,"global_delegator_complete":false,"logical_catalog_complete":false,"historical_owner_ready":false,"complete_owner_database_preservation":false,"reward_transition_executed":false,"adoption_authorized":false,"production_routing_authorized":false}}),
    )
}
#[derive(Default)]
struct SetDigest {
    count: usize,
    bytes: usize,
    paths: Sha256,
    entries: Sha256,
}
impl SetDigest {
    fn add(&mut self, path: &[u8; 32], value: &[u8]) {
        self.count += 1;
        self.bytes += value.len();
        self.paths.update(path);
        self.entries.update(path);
        self.entries.update((value.len() as u64).to_be_bytes());
        self.entries.update(value);
    }
    fn finish(self) -> Value {
        json!({"rows":self.count,"value_bytes":self.bytes,"paths_sha256":hex::encode(self.paths.finalize()),"entries_sha256":hex::encode(self.entries.finalize())})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookup_distinct_bytes_and_repeated_bytes_have_exact_boundaries() {
        let preimage = vec![1, 2, 3];
        let key = hash(&preimage);
        let path = hash(&key);
        let raw = Raw {
            rows: BTreeMap::from([(path, (0, vec![0xc2, 0x80, 0x80]))]),
        };
        let mut base = Base::new();
        base.lookups = 2295;
        base.visit(
            &Raw {
                rows: BTreeMap::new(),
            },
            vec![9],
            "ordinary",
            Value::Null,
            "absent",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(base.lookups, 2296);
        assert_eq!(
            base.visit(
                &raw,
                preimage.clone(),
                "ordinary",
                Value::Null,
                "present",
                |_| Ok(())
            )
            .unwrap_err()
            .to_string(),
            "base derivation/lookup cap"
        );
        let mut base = Base::new();
        base.bytes = 35691;
        base.visit(
            &raw,
            preimage.clone(),
            "current_reward_node",
            Value::Null,
            "present",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(base.bytes, 35694);
        let mut overflow = Base::new();
        overflow.bytes = 35694;
        assert_eq!(
            overflow
                .visit(
                    &raw,
                    preimage.clone(),
                    "ordinary",
                    Value::Null,
                    "present",
                    |_| Ok(())
                )
                .unwrap_err()
                .to_string(),
            "base matched byte cap"
        );
        base.repeated_bytes = 1024 * 1024 - 3;
        base.visit(
            &raw,
            preimage.clone(),
            "seeded_delegation_reward_node",
            Value::Null,
            "live_present_already_matched",
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(base.repeated_bytes, 1024 * 1024);
        assert_eq!(
            base.visit(
                &raw,
                preimage,
                "seeded_delegation_reward_node",
                Value::Null,
                "live_present_already_matched",
                |_| Ok(())
            )
            .unwrap_err()
            .to_string(),
            "repeated reference byte cap"
        );
    }
    #[test]
    fn typed_equality_status_and_shared_reward_replay_fail_closed() {
        let preimage = vec![1, 2, 3];
        let key = hash(&preimage);
        let path = hash(&key);
        let raw = Raw {
            rows: BTreeMap::from([(path, (0, vec![0xc2, 0x80, 0x80]))]),
        };
        let mut base = Base::new();
        base.visit(
            &raw,
            preimage.clone(),
            "current_reward_node",
            json!({"head":1}),
            "present",
            |b| equal(codec::reward_node(b)?.count == 0),
        )
        .unwrap();
        base.visit(
            &raw,
            preimage.clone(),
            "seeded_delegation_reward_node",
            json!({"delegation":1}),
            "live_present_already_matched",
            |b| equal(codec::reward_node(b)?.count == 0),
        )
        .unwrap();
        assert_eq!(base.rows.len(), 1);
        assert_eq!(base.rows[&path].refs.len(), 2);
        assert_eq!(base.bytes, 3);
        assert!(
            base.visit(
                &raw,
                preimage.clone(),
                "seeded_delegation_reward_node",
                Value::Null,
                "live_present",
                |_| Ok(())
            )
            .is_err()
        );
        assert!(
            base.visit(
                &raw,
                preimage.clone(),
                "ordinary",
                Value::Null,
                "present",
                |_| Ok(())
            )
            .is_err()
        );
        assert!(
            Base::new()
                .visit(
                    &raw,
                    preimage.clone(),
                    "reward",
                    Value::Null,
                    "live_present_already_matched",
                    |_| Ok(())
                )
                .is_err()
        );
        assert!(
            Base::new()
                .visit(
                    &raw,
                    preimage.clone(),
                    "reward",
                    Value::Null,
                    "tombstone",
                    |_| Ok(())
                )
                .is_err()
        );
        assert!(
            Base::new()
                .visit(&raw, preimage, "reward", Value::Null, "present", |_| equal(
                    false
                ))
                .is_err()
        );
        assert_eq!(
            native::validator_seed_item_key(128).0,
            hash(&[0, 5, 2, 128, 0, 0, 0])
        );
    }
}
