//! Saved P native undelegation scalar and child catalog. No physical reader exists.
#[path = "offline_undelegations/admission.rs"]
mod admission;
#[path = "offline_undelegations/codec.rs"]
mod codec;
use admission::*;
use anyhow::{Context, Result, ensure};
use codec::*;
use rustaxa_snapshot_qualifier::interval_history;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
const OUTPUT_CAP: usize = 16 * 1024 * 1024;
const SOURCES: [(&str, &str, usize, usize); 3] = [
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261011-round2/validator-seed-audit.json",
        "3a5b8398b9f497572c9678c19f43ea1c85418d696d9dc3cf9461bc96439f04d3",
        1181838,
        2 * 1024 * 1024,
    ),
    (
        "/home/fry/artifacts/n4-run-01a126b2-3ae0-7883-a5e3-b4618ade8cd5-20261010/parent-raw-inventory.json",
        "63dc30da528d294418f237e24e580d78d17653de643847140dedfac4faf51056",
        2894865,
        4 * 1024 * 1024,
    ),
    (
        "/home/fry/artifacts/light-snapshot-check-20261010/availability.json",
        "b9063f278cab05a4ac1e8ff3cd82b6a07ddfbecb1fe030d2ee22c9179dcf09e1",
        1088773,
        2 * 1024 * 1024,
    ),
];
/// Literal prefix composition; hash only after all typed components are present.
fn pre(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}
fn prefix(kind: u8, d: &[u8; 20]) -> Vec<u8> {
    pre(&[&[3, kind], d])
}
fn item(prefix: &[u8], p: u32) -> Vec<u8> {
    pre(&[prefix, &[2], &p.to_le_bytes()])
}
fn reverse(prefix: &[u8], member: &[u8]) -> Vec<u8> {
    pre(&[prefix, &[2], member])
}
fn scalar_pre(d: &[u8; 20], role: usize) -> Vec<u8> {
    match role {
        0 => pre(&[&prefix(1, d), &[1]]),
        1 => pre(&[&prefix(2, d), &[1]]),
        2 => prefix(4, d),
        _ => unreachable!(),
    }
}
fn stage_seeds(stage: &Value, raw: &Raw) -> Result<Seeds> {
    ensure!(
        stage["schema"] == 1
            && stage["status"] == "offline_seed_links_and_plan_verified"
            && stage["period"] == P
            && stage["state_root_hex"] == ROOT
            && stage["account_hex"] == ACCOUNT
            && stage["storage_root_hex"] == STORAGE_ROOT
            && stage["raw_entries_sha256"] == RAW_DIGEST,
        "Stage A identity/status drift"
    );
    for (k, n) in [
        ("verified_links", 589),
        ("seed_occurrences", 392),
        ("unique_seeds", 290),
        ("candidate_tuples", 870),
        ("prior_actual_tuples", 69),
        ("excluded_tuples", 60),
        ("residual_tuples", 810),
        ("live_candidate_paths", 70),
        ("not_live_candidate_paths", 800),
    ] {
        ensure!(stage["counts"][k] == n, "Stage A counters drift");
    }
    ensure!(
        stage["source_manifest"][0]["sha256"] == SOURCES[2].1
            && stage["source_manifest"][1]["sha256"] == SOURCES[1].1,
        "Stage A source binding drift"
    );
    let links = stage["verified_links"].as_array().context("links")?;
    ensure!(links.len() == 589, "links count");
    let mut seen = BTreeSet::new();
    for row in links {
        let key = canonical(&row["logical_key_hex"], Some(32))?;
        let path = hash(&key);
        ensure!(
            canonical(&row["trie_path_hex"], Some(32))? == path && seen.insert(path),
            "link path drift"
        );
        let (index, bytes) = raw.get(&path).context("link missing")?;
        ensure!(
            row["raw_row_index"] == *index
                && canonical(&row["raw_hex"], None)? == *bytes
                && row["raw_sha256"] == sha(bytes),
            "link bytes drift"
        );
    }
    let mut seeds = Seeds::new();
    let mut previous = None;
    for row in stage["seeds"].as_array().context("seeds")? {
        let d = address(&row["address_hex"])?;
        ensure!(previous.is_none_or(|p| p < d), "seed order/duplicate");
        previous = Some(d);
        let refs = row["provenance_indexes"]
            .as_array()
            .context("seed refs")?
            .iter()
            .map(|v| {
                v.as_u64()
                    .context("ref")
                    .and_then(|n| usize::try_from(n).map_err(Into::into))
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            !refs.is_empty() && refs.iter().all(|n| *n < 392),
            "seed refs drift"
        );
        seeds.insert(d, refs);
    }
    let provenance = stage["seed_provenance"].as_array().context("provenance")?;
    ensure!(
        seeds.len() == 290 && provenance.len() == 392,
        "seed catalog count"
    );
    ensure!(
        catalog_digest(&seeds, provenance)? == stage["digests"]["catalog_sha256"],
        "seed digest drift"
    );
    let residual = stage["residual_tuples"].as_array().context("residual")?;
    let candidates = stage["candidate_tuples"].as_array().context("candidates")?;
    ensure!(
        residual.len() == 810
            && candidates.len() == 870
            && plan_digest(residual)? == stage["digests"]["residual_sha256"],
        "residual digest drift"
    );
    let expected: Vec<_> = candidates
        .iter()
        .filter(|r| {
            r["prior_observations"]
                .as_array()
                .is_some_and(|a| a.is_empty())
        })
        .cloned()
        .collect();
    ensure!(expected == *residual, "residual exact subset drift");
    Ok(seeds)
}
/// One examined tuple. Skipped descendants have no fabricated key or lookup.
struct Catalog<'a> {
    raw: &'a Raw,
    rows: Vec<Value>,
    matched: BTreeSet<[u8; 32]>,
    lookups: usize,
    bytes: usize,
}
impl<'a> Catalog<'a> {
    fn new(raw: &'a Raw) -> Self {
        Self {
            raw,
            rows: Vec::new(),
            matched: BTreeSet::new(),
            lookups: 0,
            bytes: 0,
        }
    }
    fn lookup(
        &mut self,
        d: &[u8; 20],
        refs: &[usize],
        family: &str,
        preimage: Vec<u8>,
        origin: (Option<usize>, Option<u32>),
        child: bool,
    ) -> Result<(usize, Option<Vec<u8>>)> {
        let (parent, position) = origin;
        ensure!(self.lookups < 1017, "logical lookup cap");
        self.lookups += 1;
        let logical = hash(&preimage);
        let path = hash(&logical);
        let id = self.rows.len();
        let mut row = json!({"period":P,"account_hex":ACCOUNT,"seed_hex":hex::encode(d),"seed_provenance_indexes":refs,"family":family,"preimage_hex":hex::encode(preimage),"logical_key_hex":hex::encode(logical),"trie_path_hex":hex::encode(path),"parent_tuple":parent,"map_position":position,"status":"unresolved_not_in_live_inventory","reason":"no saved live raw row; physical outcome unknown","raw_artifact_sha256":SOURCES[1].1,"raw_row_index":null,"raw_hex":null,"raw_sha256":null,"raw_bytes":null,"decoded":null});
        let bytes = if let Some((index, bytes)) = self.raw.get(&path) {
            if child {
                ensure!(bytes.len() <= 4096, "child row byte cap");
                self.bytes = self
                    .bytes
                    .checked_add(bytes.len())
                    .context("child byte overflow")?;
                ensure!(self.bytes <= 602112, "child aggregate byte cap");
            }
            row["status"] = json!("live_unvalidated");
            row["reason"] = Value::Null;
            row["raw_row_index"] = json!(index);
            row["raw_hex"] = json!(hex::encode(bytes));
            row["raw_sha256"] = json!(sha(bytes));
            row["raw_bytes"] = json!(bytes.len());
            Some(bytes.clone())
        } else {
            None
        };
        self.rows.push(row);
        Ok((id, bytes))
    }
    fn validate<T>(
        &mut self,
        index: usize,
        result: Result<T>,
        decoded: impl FnOnce(&T) -> Value,
    ) -> Option<T> {
        match result {
            Ok(v) => {
                self.rows[index]["status"] = json!("validated_live");
                self.rows[index]["decoded"] = decoded(&v);
                let p: [u8; 32] = hex::decode(
                    self.rows[index]["trie_path_hex"]
                        .as_str()
                        .expect("owned path"),
                )
                .expect("owned hex")
                .try_into()
                .expect("owned width");
                self.matched.insert(p);
                Some(v)
            }
            Err(e) => {
                self.rows[index]["status"] = json!("invalid_live");
                self.rows[index]["reason"] = json!(format!("{e:#}"));
                None
            }
        }
    }
    fn skip(&mut self, d: &[u8; 20], refs: &[usize], family: &str, parent: usize) {
        self.rows.push(json!({"period":P,"account_hex":ACCOUNT,"seed_hex":hex::encode(d),"seed_provenance_indexes":refs,"family":family,"parent_tuple":parent,"status":"skipped_dependency_unresolved","reason":"parent not validated; key not derivable","preimage_hex":null,"logical_key_hex":null,"trie_path_hex":null,"raw_hex":null,"decoded":null}));
    }
}
#[derive(Clone)]
struct Scalar {
    v1: Option<u32>,
    v2: Option<u32>,
    cursor: Option<u64>,
    indexes: [usize; 3],
}
fn scalars(
    c: &mut Catalog<'_>,
    seeds: &Seeds,
    stage: &Value,
) -> Result<BTreeMap<[u8; 20], Scalar>> {
    let expected = stage["candidate_tuples"].as_array().context("candidates")?;
    let mut out = BTreeMap::new();
    let mut sum = [0u32; 2];
    let mut present = [0usize; 3];
    let mut zeros = [0usize; 2];
    let mut cursors = Vec::new();
    for (d, refs) in seeds {
        let mut values = [None; 3];
        let mut indexes = [0; 3];
        for role in 0..3 {
            let preimage = scalar_pre(d, role);
            let logical = hash(&preimage);
            let source = &expected[c.lookups];
            ensure!(
                source["address_hex"] == hex::encode(d)
                    && source["slot_role"]
                        == [
                            "v1_validator_count",
                            "v2_validator_count",
                            "v2_last_allocated_id"
                        ][role]
                    && canonical(&source["logical_key_hex"], Some(32))? == logical
                    && canonical(&source["trie_path_hex"], Some(32))? == hash(&logical),
                "scalar tuple binding drift"
            );
            let (index, bytes) = c.lookup(
                d,
                refs,
                [
                    "v1_validator_count",
                    "v2_validator_count",
                    "v2_last_allocated_id",
                ][role],
                preimage,
                (None, None),
                false,
            )?;
            indexes[role] = index;
            if let Some(bytes) = bytes {
                ensure!(
                    source["raw_membership"] == "present_in_authenticated_live_inventory"
                        && canonical(&source["raw_hex"], None)? == bytes
                        && source["raw_sha256"] == sha(&bytes)
                        && source["raw_row_index"] == c.rows[index]["raw_row_index"],
                    "scalar member bytes drift"
                );
                let decoded = if role == 2 {
                    decode_le_u64(&bytes, "cursor")
                } else {
                    decode_le_u32(&bytes, "count").map(u64::from)
                }?;
                values[role] = Some(decoded);
                present[role] += 1;
                if role < 2 {
                    let n = u32::try_from(decoded)?;
                    ensure!(n <= if role == 0 { 26 } else { 2 }, "per-seed count cap");
                    sum[role] = sum[role].checked_add(n).context("count overflow")?;
                    ensure!(
                        sum[role] <= if role == 0 { 45 } else { 2 },
                        "total count cap"
                    );
                    zeros[role] += usize::from(n == 0);
                } else {
                    cursors.push(decoded);
                }
                c.validate(index, Ok(decoded), |n| json!({"value":n}));
            } else {
                ensure!(
                    source["raw_membership"] == "not_in_live_inventory"
                        && source["raw_hex"].is_null(),
                    "scalar miss binding drift"
                );
            }
        }
        out.insert(
            *d,
            Scalar {
                v1: values[0].map(|n| n as u32),
                v2: values[1].map(|n| n as u32),
                cursor: values[2],
                indexes,
            },
        );
    }
    ensure!(
        present == [60, 5, 5] && zeros == [47, 4] && sum == [45, 2] && cursors == [6, 1, 22, 1, 4],
        "fixed scalar summary drift"
    );
    Ok(out)
}
fn fixed_address(bytes: &[u8]) -> Result<[u8; 20]> {
    bytes.try_into().context("validator requires 20 bytes")
}
/// Child traversal follows live forward entries only; partial branches stay explicit.
fn children(
    c: &mut Catalog<'_>,
    seeds: &Seeds,
    scalar: &BTreeMap<[u8; 20], Scalar>,
) -> Result<(Vec<Value>, Vec<Value>, Vec<Value>)> {
    let mut maps = Vec::new();
    let mut v1 = Vec::new();
    let mut v2 = Vec::new();
    let mut groups = Vec::new();
    let mut total_ids = 0u32;
    for (d, refs) in seeds {
        let s = &scalar[d];
        for version in [1u8, 2] {
            let count = if version == 1 { s.v1 } else { s.v2 };
            let parent = s.indexes[usize::from(version - 1)];
            let mut complete = count.is_some();
            let mut seen = BTreeSet::new();
            if let Some(count) = count {
                for position in 1..=count {
                    let pref = prefix(version, d);
                    let (forward, bytes) = c.lookup(
                        d,
                        refs,
                        if version == 1 {
                            "v1_validator_item"
                        } else {
                            "v2_validator_item"
                        },
                        item(&pref, position),
                        (Some(parent), Some(position)),
                        true,
                    )?;
                    let validator = bytes.and_then(|b| {
                        c.validate(
                            forward,
                            fixed_address(&b).and_then(|v| {
                                ensure!(seen.insert(v), "duplicate validator");
                                Ok(v)
                            }),
                            |v| json!({"validator_hex":hex::encode(v)}),
                        )
                    });
                    let Some(v) = validator else {
                        complete = false;
                        c.skip(d, refs, "validator_reverse_and_record_or_group", forward);
                        continue;
                    };
                    let (rev, bytes) = c.lookup(
                        d,
                        refs,
                        if version == 1 {
                            "v1_validator_reverse"
                        } else {
                            "v2_validator_reverse"
                        },
                        reverse(&pref, &v),
                        (Some(forward), Some(position)),
                        true,
                    )?;
                    let reverse_valid = bytes
                        .and_then(|b| {
                            c.validate(
                                rev,
                                decode_le_u32(&b, "reverse").and_then(|p| {
                                    ensure!(p == position, "reverse position disagreement");
                                    Ok(p)
                                }),
                                |p| json!({"position":p}),
                            )
                        })
                        .is_some();
                    if !reverse_valid {
                        complete = false;
                        c.skip(d, refs, "record_or_group_count", rev);
                        continue;
                    }
                    if version == 1 {
                        let (row, bytes) = c.lookup(
                            d,
                            refs,
                            "v1_record",
                            pre(&[&[3, 0], &v, d]),
                            (Some(forward), Some(position)),
                            true,
                        )?;
                        if let Some(object) = bytes.and_then(|b| {
                            c.validate(
                                row,
                                decode_undelegation_v1(&b),
                                |x| json!({"amount_hex":uint_hex(&x.amount),"block":x.block}),
                            )
                        }) {
                            v1.push(json!({"seed_hex":hex::encode(d),"validator_hex":hex::encode(v),"position":position,"tuple":row,"amount_hex":uint_hex(&object.amount),"block":object.block}));
                        } else {
                            complete = false;
                        }
                    } else {
                        let pref = pre(&[&[3, 3], d, &v]);
                        let (row, bytes) = c.lookup(
                            d,
                            refs,
                            "v2_group_id_count",
                            pre(&[&pref, &[1]]),
                            (Some(forward), Some(position)),
                            true,
                        )?;
                        let n = bytes.and_then(|b| {
                            c.validate(
                                row,
                                decode_le_u32(&b, "ID count").and_then(|n| {
                                    ensure!(n > 0, "enumerated group has zero ID count");
                                    Ok(n)
                                }),
                                |n| json!({"count":n}),
                            )
                        });
                        if let Some(n) = n {
                            ensure!(n <= 2, "per-group ID cap");
                            total_ids = total_ids.checked_add(n).context("ID count overflow")?;
                            ensure!(total_ids <= 2, "aggregate ID cap");
                            groups.push((*d, refs.clone(), v, position, row, n, pref));
                        } else {
                            complete = false;
                            c.skip(d, refs, "v2_id_items", row);
                        }
                    }
                }
            }
            maps.push(json!({"seed_hex":hex::encode(d),"version":version,"count_tuple":parent,"count":count,"outer_and_v1_records_validated":complete,"complete":if version==1||count==Some(0){complete}else{false},"reason":if count.is_none(){Some("count not in live inventory")}else{None}}));
        }
    }
    // Every available nested count and aggregate limit is validated before any ID lookup.
    let mut seen_ids = BTreeMap::<[u8; 20], BTreeSet<u64>>::new();
    let mut per_seed = BTreeMap::<[u8; 20], u32>::new();
    for (d, _, _, _, _, n, _) in &groups {
        let total = per_seed.entry(*d).or_default();
        *total = total.checked_add(*n).context("seed ID count overflow")?;
        ensure!(*total <= 2, "per-seed ID cap");
    }
    for (d, refs, v, vpos, parent, n, pref) in groups {
        let mut complete = true;
        for ipos in 1..=n {
            let (forward, bytes) = c.lookup(
                &d,
                &refs,
                "v2_id_item",
                item(&pref, ipos),
                (Some(parent), Some(ipos)),
                true,
            )?;
            let id = bytes.and_then(|b| {
                c.validate(
                    forward,
                    decode_le_u64(&b, "ID").and_then(|id| {
                        ensure!(
                            id > 0 && seen_ids.entry(d).or_default().insert(id),
                            "zero or duplicate delegator ID"
                        );
                        if let Some(cursor) = scalar[&d].cursor {
                            ensure!(id <= cursor, "ID exceeds allocation cursor");
                        }
                        Ok(id)
                    }),
                    |id| json!({"id":id,"cursor_validated":scalar[&d].cursor.is_some()}),
                )
            });
            let Some(id) = id else {
                complete = false;
                c.skip(&d, &refs, "v2_id_reverse_and_record", forward);
                continue;
            };
            if scalar[&d].cursor.is_none() {
                complete = false;
            }
            let (rev, bytes) = c.lookup(
                &d,
                &refs,
                "v2_id_reverse",
                reverse(&pref, &id.to_le_bytes()),
                (Some(forward), Some(ipos)),
                true,
            )?;
            let valid = bytes
                .and_then(|b| {
                    c.validate(
                        rev,
                        decode_le_u32(&b, "ID reverse").and_then(|p| {
                            ensure!(p == ipos, "ID reverse position disagreement");
                            Ok(p)
                        }),
                        |p| json!({"position":p}),
                    )
                })
                .is_some();
            if !valid {
                complete = false;
                c.skip(&d, &refs, "v2_record", rev);
                continue;
            }
            let (row, bytes) = c.lookup(
                &d,
                &refs,
                "v2_record",
                pre(&[&[3, 0], &d, &v, &id.to_le_bytes()]),
                (Some(forward), Some(ipos)),
                true,
            )?;
            if let Some((object, _)) = bytes.and_then(|b| {
                c.validate(
                    row,
                    decode_undelegation_v2(&b).and_then(|(o, embedded)| {
                        ensure!(embedded == id, "embedded ID disagreement");
                        Ok((o, embedded))
                    }),
                    |x| json!({"amount_hex":uint_hex(&x.0.amount),"block":x.0.block,"id":x.1}),
                )
            }) {
                v2.push(json!({"seed_hex":hex::encode(d),"validator_hex":hex::encode(v),"validator_position":vpos,"id_position":ipos,"id":id,"tuple":row,"amount_hex":uint_hex(&object.amount),"block":object.block}));
            } else {
                complete = false;
            }
        }
        for map in &mut maps {
            if map["seed_hex"] == hex::encode(d) && map["version"] == 2 {
                let outer = map["outer_and_v1_records_validated"] == true;
                let prior = map["group_results"].as_array().cloned().unwrap_or_default();
                let mut result = prior;
                result.push(json!({"validator_hex":hex::encode(v),"validator_position":vpos,"id_count_tuple":parent,"id_count":n,"complete":complete}));
                map["complete"] = json!(
                    outer
                        && result.len() as u64 == map["count"].as_u64().unwrap_or(0)
                        && result.iter().all(|r| r["complete"] == true)
                );
                map["group_results"] = json!(result);
            }
        }
    }
    Ok((maps, v1, v2))
}
fn catalog_digest_rows(rows: &[Value]) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"offline-undelegation-catalog-v1");
    h.update((rows.len() as u32).to_be_bytes());
    for row in rows {
        let bytes = serde_json::to_vec(row)?;
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }
    Ok(hex::encode(h.finalize()))
}
fn audit(values: &[Value; 3]) -> Result<Value> {
    let raw = raw_index(&values[1], 23278, 259077, RAW_DIGEST)?;
    let base = coverage(&values[2], &values[1])?;
    ensure!(
        base["matched_live_entries"] == 2065 && base["matched_live_value_bytes"] == 35694,
        "base coverage drift"
    );
    let seeds = stage_seeds(&values[0], &raw)?;
    let mut c = Catalog::new(&raw);
    let scalar = scalars(&mut c, &seeds, &values[0])?;
    let (maps, v1, v2) = children(&mut c, &seeds, &scalar)?;
    let bytes: usize = c.matched.iter().map(|p| raw[p].1.len()).sum();
    let invalid = c
        .rows
        .iter()
        .filter(|r| r["status"] == "invalid_live")
        .count();
    let unresolved = c
        .rows
        .iter()
        .filter(|r| r["status"] == "unresolved_not_in_live_inventory")
        .count();
    Ok(
        json!({"schema":1,"status":if invalid==0&&v1.len()==45&&v2.len()==2{"reachable_live_family_decoded"}else{"partial_reachable_family"},"period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"storage_root_hex":STORAGE_ROOT,"raw_entries_sha256":RAW_DIGEST,"source_manifest":SOURCES.iter().map(|(p,s,n,c)|json!({"path":p,"sha256":s,"bytes":n,"read_cap":c})).collect::<Vec<_>>(),"seed_provenance":values[0]["seed_provenance"],"seeds":values[0]["seeds"],"catalog_digest_version":"offline-undelegation-catalog-v1","catalog_sha256":catalog_digest_rows(&c.rows)?,"tuples":c.rows,"maps":maps,"v1_entries":v1,"v2_entries":v2,"counts":{"logical_lookups":c.lookups,"validated_live_paths":c.matched.len(),"validated_live_bytes":bytes,"invalid_live_rows":invalid,"unresolved_rows":unresolved},"coverage":{"scope":"family-local validated raw paths only","base_union_verified":false,"base_union_unresolved_reason":"base inverse has no explicit path ledger; full B reconstruction deferred","base_intersection":null,"resulting_union_rows":null,"resulting_union_bytes":null,"matched_paths_hex":c.matched.iter().map(hex::encode).collect::<Vec<_>>()},"runtime":{"database_opens":0,"storage_points":0,"new_inventories":0},"bounds":{"seeds":290,"scalar_tuples":870,"v1_entries":45,"v2_groups":2,"v2_ids":2,"child_tuples":147,"total_lookups":1017,"child_row_bytes":4096,"child_value_bytes":602112,"total_input_bytes":8*1024*1024,"output_bytes":OUTPUT_CAP},"source_pins":{"adapter_sha256":sha(include_bytes!("offline_undelegations.rs")),"admission_sha256":sha(include_bytes!("offline_undelegations/admission.rs")),"codec_sha256":sha(include_bytes!("offline_undelegations/codec.rs"))},"qualification":{"all_seed_maps_complete":false,"global_delegator_complete":false,"logical_catalog_complete":false,"semantic_snapshot_complete":false,"historical_owner_ready":false,"complete_owner_database_preservation":false,"reward_transition_executed":false,"adoption_authorized":false,"production_routing_authorized":false},"reference_policy_assumption":"owner-supplied default mainnet, likely v1.14.1; actual producer unverified"}),
    )
}
fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let output = PathBuf::from(args.next().context("usage: offline_undelegations OUTPUT")?);
    ensure!(
        args.next().is_none() && !output.to_string_lossy().starts_with("--"),
        "unexpected argument"
    );
    let parent = fs::canonicalize(output.parent().context("output parent")?)?;
    outside_db(&parent)?;
    let output = parent.join(output.file_name().context("output name")?);
    ensure!(!output.exists(), "output exists");
    let mut bytes = Vec::new();
    let mut total = 0usize;
    for (p, s, n, c) in SOURCES {
        let b = evidence(Path::new(p), Path::new(p), s, n, c)?;
        total = total.checked_add(b.len()).context("input byte overflow")?;
        ensure!(total <= 8 * 1024 * 1024, "input cap");
        bytes.push(b);
    } // All pins precede parsing.
    let values = bytes
        .iter()
        .map(|b| serde_json::from_slice(b))
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    let values: [Value; 3] = values
        .try_into()
        .map_err(|_| anyhow::anyhow!("source count"))?;
    let report = audit(&values)?;
    let bytes = interval_history::encode_bounded(&report, OUTPUT_CAP)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    println!("offline undelegations report_sha256={}", sha(&bytes));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn addr(n: u32) -> [u8; 20] {
        let mut a = [0; 20];
        a[16..].copy_from_slice(&n.to_be_bytes());
        a
    }
    fn insert(raw: &mut Raw, parts: &[&[u8]], bytes: &[u8]) {
        let logical = hash(&parts.concat());
        let path = hash(&logical);
        raw.insert(path, (raw.len(), bytes.to_vec()));
    }
    fn scalar_fixture() -> (Seeds, Raw, Value) {
        let seeds = (1..=290)
            .map(|n| (addr(n), vec![n as usize % 392]))
            .collect::<Seeds>();
        let mut raw = Raw::new();
        for n in 1..=60 {
            let count = match n {
                1..=10 => 1u32,
                11 => 4,
                12 => 5,
                13 => 26,
                _ => 0,
            };
            insert(&mut raw, &[&[3, 1], &addr(n), &[1]], &count.to_le_bytes());
        }
        for (n, cursor) in [6u64, 1, 22, 1, 4].iter().enumerate() {
            let n = n as u32 + 1;
            insert(
                &mut raw,
                &[&[3, 2], &addr(n), &[1]],
                &(if n == 1 { 2u32 } else { 0 }).to_le_bytes(),
            );
            insert(&mut raw, &[&[3, 4], &addr(n)], &cursor.to_le_bytes());
        }
        let mut candidates = Vec::new();
        for d in seeds.keys() {
            for role in 0..3 {
                let key = hash(&scalar_pre(d, role));
                let path = hash(&key);
                let mut r = json!({"address_hex":hex::encode(d),"slot_role":(["v1_validator_count","v2_validator_count","v2_last_allocated_id"][role]),"logical_key_hex":hex::encode(key),"trie_path_hex":hex::encode(path),"raw_membership":"not_in_live_inventory"});
                if let Some((index, b)) = raw.get(&path) {
                    r["raw_membership"] = json!("present_in_authenticated_live_inventory");
                    r["raw_hex"] = json!(hex::encode(b));
                    r["raw_sha256"] = json!(sha(b));
                    r["raw_row_index"] = json!(index);
                }
                candidates.push(r);
            }
        }
        (seeds, raw, json!({"candidate_tuples":candidates}))
    }
    fn branch_fixture() -> (Seeds, Raw, BTreeMap<[u8; 20], Scalar>) {
        let d = addr(0xd1);
        let v = addr(0x31);
        let w = addr(0x32);
        let seeds = BTreeMap::from([(d, vec![0, 1])]);
        let mut raw = Raw::new();
        insert(&mut raw, &[&[3, 1], &d, &[2], &1u32.to_le_bytes()], &v);
        insert(&mut raw, &[&[3, 1], &d, &[2], &v], &1u32.to_le_bytes());
        insert(
            &mut raw,
            &[&[3, 0], &v, &d],
            &hex::decode("c482012c04").unwrap(),
        );
        for (p, v, id) in [(1u32, v, 1u64), (2, w, 4)] {
            insert(&mut raw, &[&[3, 2], &d, &[2], &p.to_le_bytes()], &v);
            insert(&mut raw, &[&[3, 2], &d, &[2], &v], &p.to_le_bytes());
            insert(&mut raw, &[&[3, 3], &d, &v, &[1]], &1u32.to_le_bytes());
            insert(
                &mut raw,
                &[&[3, 3], &d, &v, &[2], &1u32.to_le_bytes()],
                &id.to_le_bytes(),
            );
            insert(
                &mut raw,
                &[&[3, 3], &d, &v, &[2], &id.to_le_bytes()],
                &1u32.to_le_bytes(),
            );
            let b = if id == 1 {
                "c6c482012c0401"
            } else {
                "c6c482012c0404"
            };
            insert(
                &mut raw,
                &[&[3, 0], &d, &v, &id.to_le_bytes()],
                &hex::decode(b).unwrap(),
            );
        }
        (
            seeds,
            raw,
            BTreeMap::from([(
                d,
                Scalar {
                    v1: Some(1),
                    v2: Some(2),
                    cursor: Some(22),
                    indexes: [0, 1, 2],
                },
            )]),
        )
    }
    #[test]
    fn scalar_width_membership_and_limits_precede_children() {
        let (seeds, raw, stage) = scalar_fixture();
        let mut c = Catalog::new(&raw);
        let s = scalars(&mut c, &seeds, &stage).unwrap();
        assert_eq!(c.lookups, 870);
        assert_eq!(c.matched.len(), 70);
        assert_eq!(s[&addr(1)].cursor, Some(6));
        assert!(s[&addr(290)].v1.is_none());
        assert_eq!(
            c.rows
                .iter()
                .filter(|r| r["status"] == "unresolved_not_in_live_inventory")
                .count(),
            800
        );
        for bytes in [vec![1; 3], vec![1; 5], 27u32.to_le_bytes().to_vec()] {
            let mut r = raw.clone();
            let p = hash(&hash(&[&[3, 1][..], &addr(1), &[1]].concat()));
            r.get_mut(&p).unwrap().1 = bytes.clone();
            let mut source = stage.clone();
            source["candidate_tuples"][0]["raw_hex"] = json!(hex::encode(&bytes));
            source["candidate_tuples"][0]["raw_sha256"] = json!(sha(&bytes));
            let mut c = Catalog::new(&r);
            assert!(scalars(&mut c, &seeds, &source).is_err());
            assert_eq!(c.lookups, 1);
        }
        let mut bad = stage.clone();
        bad["candidate_tuples"][0]["logical_key_hex"] = json!("00".repeat(32));
        assert!(scalars(&mut Catalog::new(&raw), &seeds, &bad).is_err());
    }
    #[test]
    fn ordered_live_children_and_partial_dependencies() {
        let (seeds, raw, s) = branch_fixture();
        let mut c = Catalog::new(&raw);
        let (maps, v1, v2) = children(&mut c, &seeds, &s).unwrap();
        assert_eq!(v1.len(), 1);
        assert_eq!(v2.len(), 2);
        assert!(maps.iter().all(|m| m["complete"] == true));
        assert_eq!(c.lookups, 15);
        assert_eq!(v2[1]["id"], 4);
        assert_eq!(
            c.rows[0]["preimage_hex"],
            hex::encode([&[3, 1][..], &addr(0xd1), &[2], &1u32.to_le_bytes()].concat())
        );
        let digest = catalog_digest_rows(&c.rows).unwrap();
        assert_eq!(digest, catalog_digest_rows(&c.rows).unwrap());
        let mut reordered = c.rows.clone();
        reordered.swap(0, 1);
        assert_ne!(digest, catalog_digest_rows(&reordered).unwrap());
        let mut missing = raw.clone();
        missing.remove(&hash(&hash(
            &[&[3, 1][..], &addr(0xd1), &[2], &1u32.to_le_bytes()].concat(),
        )));
        let mut c = Catalog::new(&missing);
        let (maps, v1, v2) = children(&mut c, &seeds, &s).unwrap();
        assert!(v1.is_empty());
        assert_eq!(v2.len(), 2);
        assert_eq!(maps[0]["complete"], false);
        assert!(
            c.rows
                .iter()
                .any(|r| r["status"] == "skipped_dependency_unresolved")
        );
        assert_eq!(c.lookups, 13);
        let mut wrong = raw.clone();
        insert(
            &mut wrong,
            &[&[3, 1], &addr(0xd1), &[2], &addr(0x31)],
            &2u32.to_le_bytes(),
        );
        let mut c = Catalog::new(&wrong);
        let (maps, v1, _) = children(&mut c, &seeds, &s).unwrap();
        assert!(v1.is_empty());
        assert_eq!(maps[0]["complete"], false);
        assert!(c.rows.iter().any(|r| r["status"] == "invalid_live"));
    }
    #[test]
    fn nested_caps_cursor_and_object_identity() {
        let (seeds, raw, s) = branch_fixture();
        let d = addr(0xd1);
        let v = addr(0x31);
        let mut overflow = raw.clone();
        insert(&mut overflow, &[&[3, 3], &d, &v, &[1]], &2u32.to_le_bytes());
        let mut c = Catalog::new(&overflow);
        assert!(children(&mut c, &seeds, &s).is_err());
        assert!(!c.rows.iter().any(|r| r["family"] == "v2_id_item"));
        let mut low = s.clone();
        low.get_mut(&d).unwrap().cursor = Some(1);
        let mut c = Catalog::new(&raw);
        let (maps, _, v2) = children(&mut c, &seeds, &low).unwrap();
        assert_eq!(v2.len(), 1);
        assert_eq!(maps[1]["complete"], false);
        let mut unknown = s.clone();
        unknown.get_mut(&d).unwrap().cursor = None;
        let mut c = Catalog::new(&raw);
        let (maps, _, v2) = children(&mut c, &seeds, &unknown).unwrap();
        assert_eq!(v2.len(), 2);
        assert_eq!(maps[1]["complete"], false);
        let mut embedded = raw.clone();
        insert(
            &mut embedded,
            &[&[3, 0], &d, &v, &1u64.to_le_bytes()],
            &hex::decode("c6c482012c0402").unwrap(),
        );
        let mut c = Catalog::new(&embedded);
        let (maps, _, v2) = children(&mut c, &seeds, &s).unwrap();
        assert_eq!(v2.len(), 1);
        assert_eq!(maps[1]["complete"], false);
        let mut zero = raw.clone();
        insert(&mut zero, &[&[3, 3], &d, &v, &[1]], &0u32.to_le_bytes());
        let mut c = Catalog::new(&zero);
        let (maps, _, _) = children(&mut c, &seeds, &s).unwrap();
        assert_eq!(maps[1]["complete"], false);
        let mut oversized = raw.clone();
        insert(&mut oversized, &[&[3, 0], &v, &d], &vec![0; 4097]);
        assert!(children(&mut Catalog::new(&oversized), &seeds, &s).is_err());
    }
    #[test]
    fn independent_go_key_and_codec_parity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/native_cancel_custody");
        let mut admitted = Vec::new();
        for name in ["public.json", "local.json"] {
            let p = root.join(name);
            admitted.push(
                evidence(
                    &p,
                    &p,
                    "12808dffae3a3bece2532aeea9ad036a8a008ca57e2f2c185e6381930399e6c7",
                    23761,
                    1024 * 1024,
                )
                .unwrap(),
            );
        }
        let values = admitted
            .iter()
            .map(|b| serde_json::from_slice::<Value>(b).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values[0], values[1]);
        let d = addr(0xd1);
        let v = addr(0x31);
        let object = hash(&pre(&[&[3, 0], &v, &d]));
        let forward = hash(&item(&prefix(1, &d), 1));
        let reverse = hash(&reverse(&prefix(1, &d), &v));
        let writes = values[0]["scenarios"][0]["transactions"][0]["ordered_raw_writes"]
            .as_array()
            .unwrap();
        for (key, value) in [
            (object, "c482012c04".to_string()),
            (forward, hex::encode(v)),
            (reverse, "01000000".to_string()),
        ] {
            assert!(
                writes
                    .iter()
                    .any(|r| r["key"] == hex::encode(key) && r["value"] == value)
            );
        }
        let all_writes = values[0]["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|s| s["transactions"].as_array().unwrap())
            .filter_map(|t| t["ordered_raw_writes"].as_array())
            .flatten()
            .collect::<Vec<_>>();
        for (preimage, expected) in [
            (
                pre(&[&[3, 0], &d, &v, &1u64.to_le_bytes()]),
                "c5c381c80401".to_string(),
            ),
            (
                pre(&[&[3, 3], &d, &v, &[2], &1u32.to_le_bytes()]),
                "0100000000000000".to_string(),
            ),
            (pre(&[&[3, 4], &d]), "0100000000000000".to_string()),
        ] {
            let key = hex::encode(hash(&preimage));
            assert!(
                all_writes
                    .iter()
                    .any(|r| r["key"] == key && r["value"] == expected)
            );
        }
        assert_eq!(
            decode_undelegation_v1(&hex::decode("c482012c04").unwrap())
                .unwrap()
                .block,
            4
        );
        assert_eq!(
            decode_undelegation_v2(&hex::decode("c6c482012c0401").unwrap())
                .unwrap()
                .1,
            1
        );
        assert_eq!(
            uint_hex(
                &decode_undelegation_v1(&hex::decode("c28080").unwrap())
                    .unwrap()
                    .amount
            ),
            ""
        );
        for b in [
            "c482012c0400",
            "c382000104",
            "c3808104",
            "c2820101",
            "c3010401",
            "c301c0",
            "c60189010000000000000000",
        ] {
            assert!(
                decode_undelegation_v1(&hex::decode(b).unwrap()).is_err(),
                "{b}"
            );
        }
        for b in ["c6c482012c040100", "c4010401", "c6c482012c0480ff"] {
            assert!(decode_undelegation_v2(&hex::decode(b).unwrap()).is_err());
        }
        assert_ne!(
            hash(&[&prefix(1, &d)[..], &[0], &1u32.to_le_bytes()].concat()),
            forward
        );
        assert_ne!(hash(&forward), forward);
        assert!(decode_le_u64(&[0; 9], "bad").is_err());
    }
    #[test]
    fn admission_and_output_fail_closed() {
        use std::io::Cursor;
        assert!(read_capped(Cursor::new([1; 5]), 4).is_err());
        assert_eq!(read_capped(Cursor::new([1; 4]), 4).unwrap(), [1; 4]);
        assert!(interval_history::encode_bounded(&json!({"x":"oversized"}), 2).is_err());
        let repo =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
        assert!(outside_db(&repo.join("data/x")).is_err());
        assert!(outside_db(&repo.join("local/evm-state-db/snapshot-litenode-copy/x")).is_err());
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/offline-admission-fixture.json");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let mut f = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&p)
            .unwrap();
        f.write_all(b"{}").unwrap();
        assert!(
            OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&p)
                .is_err()
        );
        assert!(evidence(&p, &p, &"00".repeat(32), 2, 4).is_err());
        assert!(evidence(&p, &p, &sha(b"{}"), 3, 4).is_err());
        assert!(evidence(&p, &p, &sha(b"{}"), 2, 1).is_err());
        fs::remove_file(p).unwrap();
    }
    fn fixture_pair() -> Value {
        json!({"head":P+1,"prior_period":P,"prior_state_root_hex":ROOT,"state_root_hex":"b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227","genesis_hash_hex":"8129076db1332837152b0212faad56ab882c1d511e0aac495f200f0a08cb6377","current_and_prior_roots_authenticated":true,"descriptor_matches_header":true})
    }
    #[test]
    fn raw_rows_and_identity_selectors_fail_closed() {
        let value = json!({"schema":1,"status":"raw_capture_complete","digest_version":"sha256-ordered-path32-length-u64be-value-v1","period":P,"state_root_hex":ROOT,"account_hex":ACCOUNT,"pair":fixture_pair(),"storage_root_hex":STORAGE_ROOT,"nodes_visited":2,"row_count":2,"value_bytes":2,"entries":[{"hashed_path_hex":hex::encode([1;32]),"value_hex":"00ff"},{"hashed_path_hex":hex::encode([2;32]),"value_hex":""}]});
        let mut value = value;
        let entries = vec![
            rustaxa_storage::ConcreteStorageInventoryEntry {
                hashed_path: [1; 32],
                value: vec![0, 255],
            },
            rustaxa_storage::ConcreteStorageInventoryEntry {
                hashed_path: [2; 32],
                value: vec![],
            },
        ];
        let digest = rustaxa_snapshot_qualifier::native_inverse::digest_entries(&entries);
        value["entries_sha256"] = json!(digest);
        assert_eq!(raw_index(&value, 2, 2, &digest).unwrap().len(), 2);
        for field in [
            "period",
            "state_root_hex",
            "account_hex",
            "storage_root_hex",
            "row_count",
            "value_bytes",
            "entries_sha256",
            "digest_version",
        ] {
            let mut x = value.clone();
            x[field] = json!("changed");
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        let mut x = value.clone();
        x["entries"].as_array_mut().unwrap().swap(0, 1);
        assert!(raw_index(&x, 2, 2, &digest).is_err());
        let mut x = value.clone();
        x["entries"][1] = x["entries"][0].clone();
        assert!(raw_index(&x, 2, 2, &digest).is_err());
        for bad in ["F".repeat(64), "z".repeat(64), "01".to_owned()] {
            let mut x = value.clone();
            x["entries"][0]["hashed_path_hex"] = json!(bad);
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        for bad in ["0", "FF", "0x00", "zz", "00fe"] {
            let mut x = value.clone();
            x["entries"][0]["value_hex"] = json!(bad);
            assert!(raw_index(&x, 2, 2, &digest).is_err());
        }
        let dep = json!({"period":P,"state_root_hex":ROOT,"root_authenticated":true,"header_sha256":"1e9c7f4e77cbbabad4ec2f8b065332aa998c881dff503c2fcbcfb0d21005adac","accounts":[{"address_hex":ACCOUNT,"status":"complete_live_inventory","storage_root_hex":STORAGE_ROOT,"live_entries":23278,"value_bytes":259077,"entries_sha256":RAW_DIGEST,"inverse":{"status":"partial_inverse_completed","coverage":{}}}]});
        let a = json!({"pair":fixture_pair(),"native_dependencies":[dep]});
        assert!(coverage(&a, &value).is_ok());
        let mut x = a.clone();
        let duplicate = x["native_dependencies"][0].clone();
        x["native_dependencies"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(coverage(&x, &value).is_err());
        let mut x = a.clone();
        x["native_dependencies"][0]["period"] = json!(P + 1);
        assert!(coverage(&x, &value).is_err());
        let mut x = a.clone();
        x["native_dependencies"][0]["accounts"][0]["address_hex"] =
            json!("00000000000000000000000000000000000000ee");
        assert!(coverage(&x, &value).is_err());
    }

    #[test]
    fn duplicate_members_and_numeric_overflow_are_invalid() {
        let (seeds, raw, s) = branch_fixture();
        let d = addr(0xd1);
        let v = addr(0x31);
        let w = addr(0x32);
        let mut duplicate = raw.clone();
        insert(
            &mut duplicate,
            &[&[3, 2], &d, &[2], &2u32.to_le_bytes()],
            &v,
        );
        let mut c = Catalog::new(&duplicate);
        let (maps, _, _) = children(&mut c, &seeds, &s).unwrap();
        assert_eq!(maps[1]["complete"], false);
        assert!(c.rows.iter().any(|r| {
            r["reason"]
                .as_str()
                .is_some_and(|s| s.contains("duplicate validator"))
        }));
        let mut duplicate = raw.clone();
        insert(
            &mut duplicate,
            &[&[3, 3], &d, &w, &[2], &1u32.to_le_bytes()],
            &1u64.to_le_bytes(),
        );
        let mut c = Catalog::new(&duplicate);
        let (maps, _, v2) = children(&mut c, &seeds, &s).unwrap();
        assert_eq!(maps[1]["complete"], false);
        assert_eq!(v2.len(), 1);
        assert!(c.rows.iter().any(|r| {
            r["reason"]
                .as_str()
                .is_some_and(|s| s.contains("duplicate delegator ID"))
        }));
        let mut r = rlp::RlpStream::new_list(2);
        r.append(&1u8).append(&vec![1u8; 9]);
        assert!(decode_undelegation_v1(&r.out()).is_err());
        let mut r = rlp::RlpStream::new_list(2);
        r.append_raw(&hex::decode("c20104").unwrap(), 1)
            .append(&vec![1u8; 9]);
        assert!(decode_undelegation_v2(&r.out()).is_err());
        let mut r = rlp::RlpStream::new_list(2);
        r.append(&vec![1u8; 33]).append(&4u64);
        assert_eq!(
            decode_undelegation_v1(&r.out())
                .unwrap()
                .amount
                .to_bytes_be()
                .len(),
            33
        );
    }
}
