//! Synthetic controls exercise only DAG counters; they are not historical cache facts.
use super::*;
use ethereum_types::H160;
use rustaxa_consensus::{
    FinalizedRewardsPeriodFact, RewardDagBlockFact, RewardsStatsConfig, RewardsStatsRuntime,
    RewardsStatsStatus, decode_rewards_block_distributions,
};

fn dags(values: &[(u8, u16)]) -> Vec<DagInput> {
    values
        .iter()
        .enumerate()
        .map(|(i, (a, d))| DagInput {
            position: i as u32,
            signed_hash: [i as u8; 32],
            author: [*a; 20],
            difficulty: *d,
        })
        .collect()
}
fn row(period: u64, blocks: &[DagInput]) -> Value {
    projection(period, [1; 32], [2; 32], blocks).unwrap()
}
fn runtime_projection(period: u64, blocks: &[DagInput], expected: &Value) {
    let mut runtime = RewardsStatsRuntime::new(
        RewardsStatsConfig {
            committee_size: 0,
            magnolia_period: 0,
            aspen_part_one_period: ASPEN,
        },
        vec![],
        vec![],
    )
    .unwrap();
    let plan = runtime.process_period(FinalizedRewardsPeriodFact {
        period,
        block_author: H160::zero(),
        blocks_per_year: 0,
        dpos_eligible_total_vote_count: 0,
        transactions: vec![],
        cert_votes: vec![],
        dag_blocks: blocks
            .iter()
            .map(|b| RewardDagBlockFact {
                author: H160::from(b.author),
                difficulty: b.difficulty,
                transaction_hashes: vec![],
            })
            .collect(),
    });
    assert_eq!(plan.status, RewardsStatsStatus::Applied);
    assert!(!plan.cache_current_period && !plan.clear_cached_stats);
    let decoded = decode_rewards_block_distributions(&plan.distribution_stats).unwrap();
    assert_eq!(decoded.len(), 1);
    assert_eq!(decoded[0].period, period);
    assert_eq!(
        json!(decoded[0].total_dag_blocks_count),
        expected["total_dag_blocks_count"]
    );
    let authors: Vec<_> = decoded[0]
        .validators_stats
        .iter()
        .filter(|(_, v)| v.dag_blocks_count > 0)
        .map(|(a, v)| json!({"author_hex":hex::encode(a),"dag_blocks_count":v.dag_blocks_count}))
        .collect();
    assert_eq!(json!(authors), expected["per_author_dag_counts"]);
}
fn fixture() -> (IntervalHistoryRows, Value) {
    let rows = serde_json::from_slice(include_bytes!(
        "../tests/fixtures/interval_history_rows.json"
    ))
    .unwrap();
    let metadata = fields::ParentCacheMetadata {
        rows: (0..49)
            .map(|i| fields::CacheExpectedRow {
                period: FIRST + i,
                bytes: if i == 0 {
                    2548
                } else if i == 48 {
                    566
                } else {
                    500
                },
                sha256: "00".repeat(32),
            })
            .collect(),
    };
    let saved = fields::derive_parent_reward_fields(&rows, &metadata)
        .unwrap()
        .value;
    (rows, saved)
}
fn implementation_fixture() -> Value {
    implementation_fields("00".repeat(32))
}

fn replace_rlp(raw: &[u8], index: usize, replacement: &[u8]) -> Vec<u8> {
    let list = rlp::Rlp::new(raw);
    let mut out = rlp::RlpStream::new_list(list.item_count().unwrap());
    for i in 0..list.item_count().unwrap() {
        out.append_raw(
            if i == index {
                replacement
            } else {
                list.at(i).unwrap().as_raw()
            },
            1,
        );
    }
    out.out().to_vec()
}

#[test]
fn real_signed_dag_order_duplicate_and_signature_mutations_fail() {
    let (rows, saved) = fixture();
    let index = rows
        .predecessors
        .iter()
        .position(|r| {
            rlp::Rlp::new(&r.period_data)
                .at(2)
                .unwrap()
                .at(2)
                .ok()
                .and_then(|v| v.item_count().ok())
                .unwrap_or(0)
                >= 2
        })
        .unwrap();
    let original = &rows.predecessors[index].period_data;
    let bundle = rlp::Rlp::new(original).at(2).unwrap();
    for duplicate in [false, true] {
        let mut modified = bundle.as_raw().to_vec();
        for part in [1, 2] {
            // Keep transaction-index lists with their signed blocks.
            let list = bundle.at(part).unwrap();
            let count = list.item_count().unwrap();
            let mut out = rlp::RlpStream::new_list(count);
            for position in 0..count {
                let source = if duplicate && position == 1 {
                    0
                } else if !duplicate && position < 2 {
                    1 - position
                } else {
                    position
                };
                out.append_raw(list.at(source).unwrap().as_raw(), 1);
            }
            modified = replace_rlp(&modified, part, &out.out());
        }
        let mut changed = rows.clone();
        changed.predecessors[index].period_data = replace_rlp(original, 2, &modified);
        assert!(derive(&changed, &saved).is_err());
    }
    let compact = bundle.at(2).unwrap();
    let first = compact.at(0).unwrap();
    let bad_signature = replace_rlp(first.as_raw(), 5, &rlp::encode(&vec![0_u8; 65]));
    let compact = replace_rlp(compact.as_raw(), 0, &bad_signature);
    let bundle = replace_rlp(bundle.as_raw(), 2, &compact);
    let mut changed = rows.clone();
    changed.predecessors[index].period_data = replace_rlp(original, 2, &bundle);
    assert!(derive(&changed, &saved).is_err());
}

#[test]
fn minimum_ties_empty_max_and_boundary_match_runtime() {
    for (values, minimum, positions, counts) in [
        (vec![], json!(null), json!([]), json!([])),
        (vec![(2, 7)], json!(7), json!([0]), json!([1])),
        (
            vec![(2, u16::MAX), (2, u16::MAX), (1, u16::MAX)],
            json!(65535),
            json!([0, 1, 2]),
            json!([1, 2]),
        ),
        (
            vec![(2, 9), (2, 3), (1, 3), (2, 3)],
            json!(3),
            json!([1, 2, 3]),
            json!([1, 2]),
        ),
    ] {
        let blocks = dags(&values);
        for period in [ASPEN, FIRST] {
            let result = row(period, &blocks);
            assert_eq!(result["minimum_raw_difficulty"], minimum);
            assert_eq!(result["qualifying_positions"], positions);
            assert_eq!(
                json!(
                    result["per_author_dag_counts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|a| a["dag_blocks_count"].clone())
                        .collect::<Vec<_>>()
                ),
                counts
            );
            runtime_projection(period, &blocks, &result);
        }
        assert!(projection(ASPEN - 1, [1; 32], [2; 32], &blocks).is_err());
    }
}
#[test]
fn positions_duplicates_independent_periods_and_order_identity() {
    let mut blocks = dags(&[(2, 4), (1, 4)]);
    let a = row(FIRST, &blocks);
    let b = row(FIRST + 1, &blocks);
    assert_eq!(a["total_dag_blocks_count"], b["total_dag_blocks_count"]);
    blocks.swap(0, 1);
    assert!(projection(FIRST, [1; 32], [2; 32], &blocks).is_err());
    for (i, b) in blocks.iter_mut().enumerate() {
        b.position = i as u32;
    }
    let reordered = row(FIRST, &blocks);
    assert_eq!(
        a["per_author_dag_counts"],
        reordered["per_author_dag_counts"]
    );
    assert_ne!(
        fields::normalized_digest(&a).unwrap(),
        fields::normalized_digest(&reordered).unwrap()
    );
    runtime_projection(FIRST, &blocks, &reordered);
    blocks[1].signed_hash = blocks[0].signed_hash;
    assert!(projection(FIRST, [1; 32], [2; 32], &blocks).is_err());
    assert!(projection(FIRST, [1; 32], [2; 32], &dags(&[(1, 0); 11])).is_err());
}
#[test]
fn all_authenticated_periods_match_runtime_and_exact_replay() {
    let (rows, saved) = fixture();
    let policy = derive(&rows, &saved).unwrap();
    assert_eq!(policy.periods.len(), 48);
    for p in &policy.periods {
        let blocks = p["dag_blocks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| DagInput {
                position: b["position"].as_u64().unwrap() as u32,
                signed_hash: hex_array(&b["signed_hash_hex"]).unwrap(),
                author: hex_array(&b["author_hex"]).unwrap(),
                difficulty: b["raw_difficulty"].as_u64().unwrap() as u16,
            })
            .collect::<Vec<_>>();
        runtime_projection(p["period"].as_u64().unwrap(), &blocks, p);
    }
    let report = policy.report(implementation_fixture()).unwrap();
    policy.validate_report(&report).unwrap();
    assert_eq!(
        report,
        derive(&rows, &saved)
            .unwrap()
            .report(implementation_fixture())
            .unwrap()
    );
    assert_eq!(report["actual"]["dag_blocks"], 139);
    assert!(
        crate::interval_history::encode_bounded(&report, OUTPUT_CAP)
            .unwrap()
            .len()
            <= OUTPUT_CAP
    );
    // Expected cache hashes are metadata, never computation inputs.
    let mut changed = saved.clone();
    changed["missing_inputs"]["expected_cache_rows"][0]["expected_sha256"] = json!("01".repeat(32));
    changed["digests"]["missing_manifest_digest"] =
        json!(fields::normalized_digest(&changed["missing_inputs"]).unwrap());
    assert_eq!(
        report,
        derive(&rows, &changed)
            .unwrap()
            .report(implementation_fixture())
            .unwrap()
    );
}
#[test]
fn claimed_fields_and_raw_mutations_cannot_supply_authority() {
    let (rows, saved) = fixture();
    for pointer in [
        "/periods/0/dags/0/author_hex",
        "/periods/0/dags/0/raw_difficulty",
        "/qualification/owner_authority",
    ] {
        let mut changed = saved.clone();
        *changed.pointer_mut(pointer).unwrap() = json!("altered");
        assert!(derive(&rows, &changed).is_err());
    }
    let mut changed = rows.clone();
    let last = changed.predecessors[0].period_data.len() - 1;
    changed.predecessors[0].period_data[last] ^= 1;
    assert!(derive(&changed, &saved).is_err());
    changed = rows.clone();
    changed.predecessors.swap(0, 1);
    assert!(derive(&changed, &saved).is_err());
    let mut changed = saved.clone();
    changed["status"] = json!("cache_complete");
    assert!(derive(&rows, &changed).is_err());
}
#[test]
fn report_schema_policy_digests_and_authority_are_recomputed() {
    let (rows, saved) = fixture();
    let policy = derive(&rows, &saved).unwrap();
    let report = policy.report(implementation_fixture()).unwrap();
    for (pointer, value) in [
        ("/reference_policy/basis", json!("verified producer")),
        ("/reference_policy/aspen_part_one_period", json!(0)),
        ("/reference_policy/producer_verified", json!(true)),
        ("/bounds/output_bytes", json!(OUTPUT_CAP + 1)),
        ("/periods/0/minimum_raw_difficulty", json!(null)),
        ("/periods/0/total_dag_blocks_count", json!(999)),
        ("/periods/0/dag_blocks/0/position", json!(99)),
        ("/periods/0/dag_blocks/0/author_hex", json!("00")),
        ("/periods/0/qualifying_positions", json!([1, 0])),
        ("/sources/0/sha256", json!("00".repeat(32))),
        ("/implementation/core_sha256", json!("00".repeat(32))),
    ] {
        let mut bad = report.clone();
        *bad.pointer_mut(pointer).unwrap() = value;
        bad["digests"]["policy_rows_sha256"] =
            json!(fields::normalized_digest(&bad["periods"]).unwrap());
        assert!(policy.validate_report(&bad).is_err(), "{pointer}");
    }
    for flag in report["qualification"].as_object().unwrap().keys() {
        let mut bad = report.clone();
        bad["qualification"][flag] = json!(!report["qualification"][flag].as_bool().unwrap());
        assert!(policy.validate_report(&bad).is_err());
        let mut bad = report.clone();
        bad["qualification"].as_object_mut().unwrap().remove(flag);
        assert!(policy.validate_report(&bad).is_err());
    }
    for pointer in [
        "",
        "/qualification",
        "/periods/0",
        "/periods/0/dag_blocks/0",
        "/reference_policy",
        "/digests",
    ] {
        let mut bad = report.clone();
        bad.pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(false));
        assert!(policy.validate_report(&bad).is_err());
    }
    let mut bad = report.clone();
    bad["periods"].as_array_mut().unwrap().pop();
    assert!(policy.validate_report(&bad).is_err());
    let mut bad = report.clone();
    bad["sources"].as_array_mut().unwrap().swap(0, 1);
    assert!(policy.validate_report(&bad).is_err());
    let index = report["periods"]
        .as_array()
        .unwrap()
        .iter()
        .position(|p| p["per_author_dag_counts"].as_array().unwrap().len() >= 2)
        .unwrap();
    let mut bad = report.clone();
    bad["periods"][index]["per_author_dag_counts"]
        .as_array_mut()
        .unwrap()
        .swap(0, 1);
    assert!(policy.validate_report(&bad).is_err());
    let mut bad = report.clone();
    bad["periods"][index]["per_author_dag_counts"][0]["dag_blocks_count"] = json!(0);
    assert!(policy.validate_report(&bad).is_err());
    let mut without = report.clone();
    without["digests"]
        .as_object_mut()
        .unwrap()
        .remove("report_content_sha256");
    assert_eq!(
        report["digests"]["report_content_sha256"],
        fields::normalized_digest(
            &json!({"domain":"n4/aspen-candidate-policy/report/v1","report":without})
        )
        .unwrap()
    );
}
#[test]
fn admission_json_pins_growth_and_exclusive_output() {
    for raw in [
        br#"{"a":1,"a":2}"#.as_slice(),
        br#"{"a":{"b":1,"b":2}}"#,
        br#"{"a":1.0}"#,
        br#"{}{}"#,
    ] {
        assert!(parse_unique(raw).is_err());
    }
    assert!(admission::read_capped(&b"12345"[..], 4).is_err());
    assert!(
        crate::interval_history::encode_bounded(&json!("x".repeat(OUTPUT_CAP)), OUTPUT_CAP)
            .is_err()
    );
    let dir = std::env::temp_dir().join(format!("aspen-policy-test-{}", std::process::id()));
    fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let dir = fs::canonicalize(dir).unwrap();
    let input = dir.join("input");
    fs::write(&input, b"{}").unwrap();
    assert_eq!(read_fixed(&input, 2, &sha(b"{}")).unwrap(), b"{}");
    assert!(read_fixed(&input, 1, &sha(b"{}")).is_err());
    assert!(read_fixed(&input, 2, &"00".repeat(32)).is_err());
    let alias = dir.join("alias");
    std::os::unix::fs::symlink(&input, &alias).unwrap();
    assert!(read_fixed(&alias, 2, &sha(b"{}")).is_err());
    assert!(output_path(&alias).is_err());
    let symlink_parent = dir.join("link");
    std::os::unix::fs::symlink(&dir, &symlink_parent).unwrap();
    assert!(output_path(&symlink_parent.join("out")).is_err());
    let repo = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")).unwrap();
    assert!(admission::outside_db(&repo.join("data/out")).is_err());
    assert!(
        admission::outside_db(&repo.join("local/evm-state-db/snapshot-litenode-copy/out")).is_err()
    );
    let output = dir.join("out");
    let (rows, saved) = fixture();
    let policy = derive(&rows, &saved).unwrap();
    let report = policy.report(implementation_fixture()).unwrap();
    publish(&output, &policy, &report).unwrap();
    let bytes = fs::read(&output).unwrap();
    assert!(publish(&output, &policy, &report).is_err());
    assert_eq!(bytes, fs::read(&output).unwrap());
    assert!(admission::publish_exclusive(&output, b"bad").is_err());
    assert!(
        !dir.join(format!(".out.pending-{}", std::process::id()))
            .exists()
    );
}
