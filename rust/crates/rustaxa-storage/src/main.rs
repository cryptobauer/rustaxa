use anyhow::{Context, Result};
use ethereum_types::H256;
use rlp::Rlp;
use rustaxa_storage::{AccessMode, Column, Config, StatusField, Storage};
use std::collections::HashMap;
use std::path::PathBuf;

// PeriodData RLP positions
const PBFT_BLOCK_POS: usize = 0;
const DAG_BLOCKS_POS: usize = 2;

// PbftBlock RLP positions
const PBFT_PREV_HASH_POS: usize = 0;
const PBFT_PIVOT_DAG_POS: usize = 1;
const PBFT_PERIOD_POS: usize = 4;
const PBFT_TIMESTAMP_POS: usize = 5;

fn h256_short(h: &H256) -> String {
    format!(
        "{:#010x}…",
        h.as_bytes()[0..4]
            .iter()
            .fold(0u32, |acc, &b| acc << 8 | b as u32)
    )
}

fn decode_pbft_block_fields(rlp_bytes: &[u8]) -> Result<(H256, H256, u64, u64)> {
    let rlp = Rlp::new(rlp_bytes);
    let prev_hash: H256 = rlp.val_at(PBFT_PREV_HASH_POS)?;
    let pivot_dag: H256 = rlp.val_at(PBFT_PIVOT_DAG_POS)?;
    let period: u64 = rlp.val_at(PBFT_PERIOD_POS)?;
    let timestamp: u64 = rlp.val_at(PBFT_TIMESTAMP_POS)?;
    Ok((prev_hash, pivot_dag, period, timestamp))
}

fn main() -> Result<()> {
    let db_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/data/taraxa/db".to_string());
    println!("=== Taraxa Chain Diagnostic Tool ===");
    println!("Database path: {db_path}");

    let mut config = Config::new(
        PathBuf::from(&db_path)
            .parent()
            .unwrap_or(std::path::Path::new(&db_path))
            .to_path_buf(),
    );
    // Open in primary mode for direct read access (no C++ running concurrently)
    config.access_mode = AccessMode::Primary;
    config.db_path = PathBuf::from(&db_path);
    config.create_if_missing = false;
    config.create_missing_column_families = false;

    let storage = Storage::new(config).context("Failed to open database")?;

    println!();
    print_section("1. DATABASE STATUS FIELDS");
    let dag_blk_count = storage
        .metadata()
        .status_field(StatusField::DagBlkCount as u8)?;
    let dag_edge_count = storage
        .metadata()
        .status_field(StatusField::DagEdgeCount as u8)?;
    let executed_dag_blk_count = storage
        .metadata()
        .status_field(StatusField::ExecutedBlkCount as u8)?;
    let executed_trx_count = storage
        .metadata()
        .status_field(StatusField::ExecutedTrxCount as u8)?;
    let trx_count = storage
        .metadata()
        .status_field(StatusField::TrxCount as u8)?;
    let db_major = storage
        .metadata()
        .status_field(StatusField::DbMajorVersion as u8)?;
    let db_minor = storage
        .metadata()
        .status_field(StatusField::DbMinorVersion as u8)?;
    println!("  DB version:              {db_major}.{db_minor}");
    println!(
        "  Executed DAG blocks:     {executed_dag_blk_count} (cumulative finalized DAG blocks)"
    );
    println!("  Executed txns:           {executed_trx_count}");
    println!("  Non-finalized txn count: {trx_count}");
    println!("  DAG block count:         {dag_blk_count}");
    println!("  DAG edge count:          {dag_edge_count}");

    println!();
    print_section("2. LAST FINALIZED PERIOD");
    // Read from final_chain_meta (key LAST_NUMBER=1) — this is the actual last finalized block number
    let last_period = read_last_block_number(&storage)?;
    println!("  Last finalized period (final_chain_meta): {last_period}");

    // Also read PBFT chain head from pbft_head (key = zero hash = JSON)
    let (pbft_chain_size, pbft_last_hash) = read_pbft_chain_head(&storage)?;
    println!("  PBFT chain size (pbft_head):              {pbft_chain_size}");
    println!("  PBFT last block hash:                     {pbft_last_hash:?}");
    if last_period != pbft_chain_size {
        println!(
            "  ⚠ MISMATCH: final_chain_meta ({last_period}) != pbft_chain size ({pbft_chain_size})"
        );
    }

    // Read the last few period_data entries
    println!();
    print_section("3. RECENT FINALIZED PERIODS (last 5)");
    let start = if last_period > 4 { last_period - 4 } else { 1 };
    let mut last_anchor = H256::zero();
    for p in start..=last_period {
        match read_period_summary(&storage, p) {
            Ok(Some(summary)) => {
                println!(
                    "  Period {}: prev={} anchor={} dag_blocks={} txns={} time={}",
                    summary.period,
                    h256_short(&summary.prev_hash),
                    if summary.pivot_dag.is_zero() {
                        "null".to_string()
                    } else {
                        h256_short(&summary.pivot_dag)
                    },
                    summary.dag_count,
                    summary.txn_count,
                    format_timestamp(summary.timestamp),
                );
                if p == last_period {
                    last_anchor = summary.pivot_dag;
                }
            }
            Ok(None) => println!("  Period {p}: <missing>"),
            Err(e) => println!("  Period {p}: decode error: {e}"),
        }
    }
    println!(
        "  Last anchor DAG hash: {}",
        if last_anchor.is_zero() {
            "null".to_string()
        } else {
            format!("{last_anchor:?}")
        }
    );

    println!();
    print_section("4. PBFT MANAGER STATE");
    // PbftMgrField: 0=PbftRound, 1=PbftStep
    let round = storage.pbft().pbft_mgr_field(0)?;
    let step = storage.pbft().pbft_mgr_field(1)?;
    println!("  PBFT round: {:?}", round);
    println!("  PBFT step:  {:?}", step);
    if let Some(cert_rlp) = storage.pbft().cert_voted_block_in_round_rlp()? {
        let rlp = Rlp::new(&cert_rlp);
        let cert_round: u64 = rlp.val_at(0).unwrap_or(0);
        println!("  Cert voted block round: {cert_round}");
    } else {
        println!("  Cert voted block: none");
    }

    println!();
    print_section("5. PROPOSED PBFT BLOCKS");
    let proposed = storage.pbft().proposed_pbft_blocks_rlp()?;
    println!("  Total count: {}", proposed.len());

    // Aggregate by period and anchor
    let mut by_period: HashMap<u64, Vec<(H256, H256, H256, u64)>> = HashMap::new();
    let mut decode_errors = 0;
    for block_rlp in &proposed {
        match decode_pbft_block_fields(block_rlp) {
            Ok((prev_hash, pivot_dag, period, timestamp)) => {
                let block_hash = keccak256(block_rlp);
                by_period
                    .entry(period)
                    .or_default()
                    .push((block_hash, pivot_dag, prev_hash, timestamp));
            }
            Err(_) => decode_errors += 1,
        }
    }
    if decode_errors > 0 {
        println!("  Decode errors: {decode_errors}");
    }

    let mut periods: Vec<_> = by_period.keys().copied().collect();
    periods.sort();
    let mut target_anchors: Vec<H256> = Vec::new();
    for period in &periods {
        let blocks = &by_period[period];
        let mut anchor_counts: HashMap<H256, usize> = HashMap::new();
        for (_, pivot_dag, _, _) in blocks {
            *anchor_counts.entry(*pivot_dag).or_default() += 1;
        }
        let sample = &blocks[0];
        println!(
            "  Period {period}: {count} proposed blocks, {anchors} unique anchors, prev={prev}, sample_time={ts}",
            count = blocks.len(),
            anchors = anchor_counts.len(),
            prev = h256_short(&sample.2),
            ts = format_timestamp(sample.3),
        );
        for (anchor, count) in &anchor_counts {
            let label = if anchor.is_zero() {
                "null".to_string()
            } else {
                format!("{anchor:?}")
            };
            print!("    anchor {label} ({count}x): ");
            if !anchor.is_zero() {
                check_dag_block_existence_inline(&storage, *anchor)?;
                if *period == last_period + 1 {
                    target_anchors.push(*anchor);
                }
            } else {
                println!("null anchor");
            }
        }
    }

    println!();
    print_section("6. NON-FINALIZED DAG BLOCKS");
    let nonfinalized = storage.dag().nonfinalized_dag_blocks()?;
    let total_nonfinalized: usize = nonfinalized.values().map(|v| v.len()).sum();
    println!("  Total non-finalized DAG blocks: {total_nonfinalized}");
    if !nonfinalized.is_empty() {
        let min_level = *nonfinalized.keys().next().unwrap();
        let max_level = *nonfinalized.keys().last().unwrap();
        println!("  Level range: {min_level} .. {max_level}");
        println!("  Number of levels with blocks: {}", nonfinalized.len());

        let levels: Vec<_> = nonfinalized.keys().copied().collect();
        let show_start = if levels.len() > 10 {
            levels.len() - 10
        } else {
            0
        };
        println!("  Recent levels:");
        for &lvl in &levels[show_start..] {
            let blocks = &nonfinalized[&lvl];
            println!("    Level {lvl}: {} blocks", blocks.len());
        }
    }

    // Determine the last mapping boundary level
    let last_mapping_level = find_last_mapping_level(&storage, last_period)?;
    println!(
        "  Last proposal_period_levels_map boundary: level {} → period {last_period}",
        last_mapping_level
    );

    println!();
    print_section("6b. RECOVERY SIMULATION (recoverDag)");
    // Simulate what recoverDag would do for each non-finalized block.
    // getNonfinalizedDagBlocks iterates the dag_blocks column by hash key, groups by level.
    // We need to iterate raw to get the hash keys in their actual DB order.
    let target_anchor = if target_anchors.is_empty() {
        H256::zero()
    } else {
        target_anchors[0]
    };
    let mut target_anchor_level = 0u64;

    // Build a map: level → [(hash, DagBlock)] in hash-key iteration order
    let mut blocks_by_level: std::collections::BTreeMap<u64, Vec<(H256, rustaxa_types::DagBlock)>> =
        std::collections::BTreeMap::new();
    for res in storage.iter(Column::DagBlocks) {
        let (key, value) = res?;
        let hash = H256::from_slice(&key);
        match rustaxa_types::DagBlock::from_rlp_bytes(&value) {
            Ok(block) => {
                if hash == target_anchor {
                    target_anchor_level = block.level;
                }
                blocks_by_level
                    .entry(block.level)
                    .or_default()
                    .push((hash, block));
            }
            Err(e) => eprintln!("  [debug] Failed to decode block {hash:?}: {e}"),
        }
    }

    if !target_anchor.is_zero() && target_anchor_level > 0 {
        println!("  Target anchor {target_anchor:?}");
        println!("  Target anchor level: {target_anchor_level}");
        println!();

        if let Some(blocks) = blocks_by_level.get(&target_anchor_level) {
            println!("  Blocks at level {target_anchor_level} (in DB iteration order):");
            let mut target_position = 0;
            for (i, (hash, block)) in blocks.iter().enumerate() {
                let is_target = *hash == target_anchor;
                if is_target {
                    target_position = i;
                }
                let marker = if is_target { " ← TARGET ANCHOR" } else { "" };

                // recoverDag check 1: is block "actually finalized"?
                let period_entry = storage.dag().dag_block_period(*hash);
                let finalized_status = match &period_entry {
                    Ok((p, _)) => format!("⚠ FINALIZED in period {p} (causes break!)"),
                    Err(_) => "non-finalized ✓".to_string(),
                };

                // recoverDag check 2: proposal period exists? (uses Seek = first key >= level)
                let proposal_period = get_proposal_period_seek(&storage, block.level)?;
                let pp_status = match proposal_period {
                    Some(pp) => format!("period {pp} ✓"),
                    None => "⚠ NO MAPPING (causes assert+break!)".to_string(),
                };

                // recoverDag check 3: VDF/VRF — we can't verify from Rust, but flag it
                let vdf_note = match proposal_period {
                    Some(pp) => {
                        // Check if period data exists (getPeriodBlockHash reads from period_data)
                        let pd_raw = storage.period().period_data_raw(pp)?;
                        let hash_status = if !pd_raw.is_empty() {
                            "exists"
                        } else {
                            "MISSING"
                        };
                        format!("(VDF input: period_data for period {pp}: {hash_status})")
                    }
                    None => "(VDF/VRF check: skipped — no proposal period)".to_string(),
                };

                println!("    [{i}] {hash:?}{marker}");
                println!("        finalized check: {finalized_status}");
                println!("        proposal period: {pp_status}");
                println!("        {vdf_note}");
                println!(
                    "        pivot: {pivot}, tips: {tips}",
                    pivot = h256_short(&block.pivot),
                    tips = block.tips.len()
                );
            }

            if target_position > 0 {
                println!();
                println!(
                    "  ⚠ TARGET ANCHOR is at position [{target_position}] — {target_position} block(s) are processed before it"
                );
                println!(
                    "    If any preceding block fails validation, the `break` in recoverDag() skips the target anchor!"
                );
            } else {
                println!();
                println!(
                    "  ✓ TARGET ANCHOR is at position [0] — it's processed first at its level"
                );
            }
        }
    } else if !target_anchor.is_zero() {
        println!("  Target anchor {target_anchor:?} not found in non-finalized blocks");
    } else {
        println!("  No target anchor identified from proposed blocks");
    }

    // Show ALL levels and their recoverDag simulation
    println!();
    println!("  Full recovery simulation (all levels):");
    for (lvl, blocks) in &blocks_by_level {
        let mut would_break = false;
        let mut break_reason = String::new();
        let mut break_at = 0;

        for (i, (hash, _block)) in blocks.iter().enumerate() {
            // Check 1: finalized?
            if let Ok((p, _)) = storage.dag().dag_block_period(*hash) {
                would_break = true;
                break_reason = format!(
                    "block [{i}] {short} is finalized in period {p}",
                    short = h256_short(hash)
                );
                break_at = i;
                break;
            }
            // Check 2: proposal period? (uses Seek = first key >= level, matching C++)
            match get_proposal_period_seek(&storage, *lvl)? {
                Some(_) => {} // OK
                None => {
                    would_break = true;
                    break_reason = format!(
                        "block [{i}] {short} has no proposal period mapping",
                        short = h256_short(hash)
                    );
                    break_at = i;
                    break;
                }
            }
        }

        let has_target = blocks.iter().any(|(h, _)| *h == target_anchor);
        if would_break {
            let skipped = blocks.len() - break_at - 1;
            let target_skipped = if has_target {
                let target_pos = blocks
                    .iter()
                    .position(|(h, _)| *h == target_anchor)
                    .unwrap_or(0);
                target_pos > break_at
            } else {
                false
            };
            let target_marker = if target_skipped {
                " ← TARGET ANCHOR SKIPPED!"
            } else {
                ""
            };
            println!(
                "    Level {lvl}: {count} blocks — ⚠ BREAK at [{break_at}]: {break_reason} (skips {skipped} remaining){target_marker}",
                count = blocks.len()
            );
        } else if has_target {
            println!(
                "    Level {lvl}: {count} blocks — ✓ all pass, target anchor included",
                count = blocks.len()
            );
        } else {
            println!(
                "    Level {lvl}: {count} blocks — ✓ all pass",
                count = blocks.len()
            );
        }
    }

    // Blocks beyond last explicit mapping entry
    let blocks_beyond_mapping: usize = blocks_by_level
        .iter()
        .filter(|(lvl, _)| **lvl > last_mapping_level)
        .map(|(_, blocks)| blocks.len())
        .sum();
    if blocks_beyond_mapping > 0 {
        println!();
        println!("  Note: {blocks_beyond_mapping} blocks at levels > {last_mapping_level}");
        println!("  These use range-query semantics and may still resolve to period {last_period}");
    }

    // Build a hash→block lookup for non-finalized blocks
    let all_nonfinalized: HashMap<H256, &rustaxa_types::DagBlock> = blocks_by_level
        .values()
        .flat_map(|v| v.iter().map(|(h, b)| (*h, b)))
        .collect();

    println!();
    print_section("6c. PIVOT CHAIN & pivotAndTipsAvailable ANALYSIS");
    // recoverDag() calls pivotAndTipsAvailable(blk) which needs the pivot and all tips
    // to exist in the in-memory DAG or as loadable blocks. But during recovery, only
    // blocks that have ALREADY been added (lower levels first) are in the graph.
    // The anchor is added first with addToDag(anchor, kNullBlockHash, {}, 0, true) — so it's level 0 in the graph.
    // Then non-finalized blocks are added level-by-level.
    // pivotAndTipsAvailable checks if pivot AND tips exist via getDagBlock() — which checks in-memory + DB.
    // BUT addDagBlock (with save=false during recovery) calls pivotAndTipsAvailable which uses getDagBlock.
    // getDagBlock checks: 1) seen_blocks_ cache, 2) nonfinalized DB, 3) finalized DB
    // So the pivot just needs to exist SOMEWHERE in the DB.

    if !target_anchor.is_zero() && target_anchor_level > 0 {
        // Trace the pivot chain from the target anchor
        println!("  Target anchor pivot chain:");
        let mut current = target_anchor;
        let mut depth = 0;
        loop {
            if depth > 10 {
                println!("    ... (truncated)");
                break;
            }
            let block = all_nonfinalized.get(&current);
            match block {
                Some(b) => {
                    let pivot = b.pivot;
                    let _tips_count = b.tips.len();
                    let in_nonfinalized = all_nonfinalized.contains_key(&pivot);
                    let in_finalized = storage.dag().dag_block_period(pivot).is_ok();
                    let pivot_status = if pivot.is_zero() {
                        "genesis/null".to_string()
                    } else if in_nonfinalized {
                        "non-finalized ✓".to_string()
                    } else if in_finalized {
                        "finalized ✓".to_string()
                    } else {
                        "⚠ MISSING! (pivotAndTipsAvailable would fail)".to_string()
                    };
                    println!("    [{depth}] {current:?}");
                    println!(
                        "        pivot: {pivot} → {pivot_status}",
                        pivot = h256_short(&pivot)
                    );
                    // Check tips too
                    for (ti, tip) in b.tips.iter().enumerate() {
                        let tip_in_nf = all_nonfinalized.contains_key(tip);
                        let tip_in_f = storage.dag().dag_block_period(*tip).is_ok();
                        let tip_status = if tip_in_nf {
                            "non-finalized ✓"
                        } else if tip_in_f {
                            "finalized ✓"
                        } else {
                            "⚠ MISSING!"
                        };
                        println!(
                            "        tip[{ti}]: {tip} → {tip_status}",
                            tip = h256_short(tip)
                        );
                    }
                    if pivot.is_zero() {
                        break;
                    }
                    current = pivot;
                }
                None => {
                    // Not in non-finalized, check finalized
                    let in_finalized = storage.dag().dag_block_period(current).is_ok();
                    if in_finalized {
                        println!(
                            "    [{depth}] {current:?} → finalized ✓ (chain ends in finalized data)"
                        );
                    } else {
                        println!(
                            "    [{depth}] {current:?} → ⚠ MISSING from both non-finalized and finalized!"
                        );
                    }
                    break;
                }
            }
            depth += 1;
        }

        // Also check: would the target anchor's pivot be in the graph BEFORE the anchor is processed?
        // recoverDag adds: anchor first (as level 0), then processes non-finalized blocks level by level.
        // The target anchor is NOT the "recovery anchor" (0x861dce07) — it's the proposed anchor for the NEXT period.
        // The recovery anchor is the last finalized anchor which gets added first.
        println!();
        println!("  Recovery anchor (last finalized): {last_anchor:?}");
        println!(
            "  The recovery code adds this anchor first, then processes non-finalized blocks."
        );
        println!("  For the target anchor to be added to the DAG via addDagBlock, its pivot");
        println!("  must exist via getDagBlock (DB lookup, not just in-memory graph).");
        let target_pivot = all_nonfinalized
            .get(&target_anchor)
            .map(|b| b.pivot)
            .unwrap_or(H256::zero());
        if !target_pivot.is_zero() {
            let pivot_in_nonfinalized = all_nonfinalized.contains_key(&target_pivot);
            let pivot_in_finalized = storage.dag().dag_block_period(target_pivot).is_ok();
            // Check the pivot's level — it must be processed BEFORE the target anchor's level
            let pivot_level = all_nonfinalized.get(&target_pivot).map(|b| b.level);
            println!("  Target anchor pivot: {target_pivot:?}");
            println!("    In non-finalized DB: {pivot_in_nonfinalized}");
            println!("    In finalized DB: {pivot_in_finalized}");
            if let Some(pl) = pivot_level {
                println!("    Pivot level: {pl} (target anchor level: {target_anchor_level})");
                if pl >= target_anchor_level {
                    println!("    ⚠ Pivot level >= anchor level — ordering issue!");
                } else {
                    println!("    ✓ Pivot level < anchor level — will be processed first");
                }
                // Check if the pivot's level had a break
                if let Some(pblocks) = blocks_by_level.get(&pl) {
                    let pivot_present = pblocks.iter().any(|(h, _)| *h == target_pivot);
                    println!(
                        "    Pivot present at its level in non-finalized iteration: {pivot_present}"
                    );
                }
            }
        }
    }

    println!();
    print_section("6d. VDF/SORTITION DATA FOR TARGET ANCHOR");
    if !target_anchor.is_zero() && target_anchor_level > 0 {
        let propose_period = get_proposal_period_seek(&storage, target_anchor_level)?;
        println!("  Proposal period for level {target_anchor_level} (Seek): {propose_period:?}");

        if let Some(pp) = propose_period {
            // Check sortition params
            let sp = storage.metadata().params_change_for_period_rlp(pp)?;
            match &sp {
                Some(data) => println!(
                    "  Sortition params change at/before period {pp}: {} bytes RLP",
                    data.len()
                ),
                None => println!("  ⚠ No sortition params change found at/before period {pp}"),
            }

            // Check period block hash (getPeriodBlockHash reads PBFT block for that period)
            let pd_raw = storage.period().period_data_raw(pp)?;
            if !pd_raw.is_empty() {
                let period_rlp = Rlp::new(&pd_raw);
                let pbft_rlp = period_rlp.at(PBFT_BLOCK_POS)?;
                let pbft_block_hash = keccak256(pbft_rlp.as_raw());
                println!("  Period block hash for period {pp}: {pbft_block_hash:?}");

                // Reconstruct the VRF input: RLP(level) || RLP(period_block_hash)
                // This is what makeVrfInput(level, period_block_hash) produces
                let mut vrf_input = Vec::new();
                // RLP encode level (uint64)
                let level_rlp = rlp::encode(&target_anchor_level);
                vrf_input.extend_from_slice(&level_rlp);
                // RLP encode hash (H256 = 32 bytes)
                let hash_rlp = rlp::encode(&pbft_block_hash);
                vrf_input.extend_from_slice(&hash_rlp);
                let vrf_input_hex = hex_encode(&vrf_input);
                println!("  Reconstructed VRF input: {vrf_input_hex}");
                println!(
                    "  (Compare with error log VRF input to check if propose_period resolved correctly)"
                );
            } else {
                println!("  ⚠ Period data MISSING for period {pp}");
            }

            // Also check the PREVIOUS period — maybe the block was created with that
            if pp > 0 {
                let prev_pp = pp - 1;
                let prev_raw = storage.period().period_data_raw(prev_pp)?;
                if !prev_raw.is_empty() {
                    let prev_period_rlp = Rlp::new(&prev_raw);
                    let prev_pbft_rlp = prev_period_rlp.at(PBFT_BLOCK_POS)?;
                    let prev_hash = keccak256(prev_pbft_rlp.as_raw());
                    let mut alt_vrf_input = Vec::new();
                    let level_rlp = rlp::encode(&target_anchor_level);
                    alt_vrf_input.extend_from_slice(&level_rlp);
                    let hash_rlp = rlp::encode(&prev_hash);
                    alt_vrf_input.extend_from_slice(&hash_rlp);
                    let alt_hex = hex_encode(&alt_vrf_input);
                    println!();
                    println!("  Alternative: if propose_period was {prev_pp} instead:");
                    println!("    Period block hash for period {prev_pp}: {prev_hash:?}");
                    println!("    VRF input would be: {alt_hex}");
                }
            }

            // CRITICAL CHECK: Seek(target_level + 1) simulates what the map returned
            // BEFORE the finalization of period {pp} inserted the entry at exactly target_level.
            // If a map entry was added at key=target_level during finalization, the ORIGINAL
            // lookup when the block was created would have found the NEXT entry instead.
            let seek_next = get_proposal_period_seek(&storage, target_anchor_level + 1)?;
            if let Some(next_pp) = seek_next {
                println!();
                println!(
                    "  CRITICAL: Seek(level+1) = period {next_pp} (pre-finalization lookup simulation)"
                );
                let next_raw = storage.period().period_data_raw(next_pp)?;
                if !next_raw.is_empty() {
                    let next_period_rlp = Rlp::new(&next_raw);
                    let next_pbft_rlp = next_period_rlp.at(PBFT_BLOCK_POS)?;
                    let next_hash = keccak256(next_pbft_rlp.as_raw());
                    let mut next_vrf_input = Vec::new();
                    let level_rlp = rlp::encode(&target_anchor_level);
                    next_vrf_input.extend_from_slice(&level_rlp);
                    let hash_rlp = rlp::encode(&next_hash);
                    next_vrf_input.extend_from_slice(&hash_rlp);
                    let next_hex = hex_encode(&next_vrf_input);
                    println!("    Period block hash for period {next_pp}: {next_hash:?}");
                    println!("    VRF input would be: {next_hex}");
                    if next_pp != pp {
                        println!(
                            "    ⚠ PROPOSE_PERIOD MISMATCH: Seek(level)={pp} vs Seek(level+1)={next_pp}"
                        );
                        println!(
                            "    → This means finalization of period {pp} inserted map entry at key {target_anchor_level},"
                        );
                        println!(
                            "      changing the Seek result for this level. The block was likely created with period {next_pp}."
                        );
                    }
                }
            }

            // Check the last few sortition params changes to see if params changed recently
            let recent_sp = storage.metadata().last_sortition_params_changes_rlp(3)?;
            println!();
            println!("  Last {} sortition params changes:", recent_sp.len());
            for (i, sp_rlp) in recent_sp.iter().enumerate() {
                let rlp = Rlp::new(sp_rlp);
                // SortitionParamsChange RLP: [period, params_data...]
                let sp_period: u64 = rlp.val_at(0).unwrap_or(0);
                println!("    [{i}] period: {sp_period}, {} bytes RLP", sp_rlp.len());
            }
        }

        // Dump VDF bytes from the target anchor block
        if let Some(blk) = all_nonfinalized.get(&target_anchor) {
            println!();
            println!("  Target anchor VDF data: {} bytes", blk.vdf.len());
            if !blk.vdf.is_empty() {
                println!(
                    "    First 20 bytes hex: {}",
                    hex_encode(&blk.vdf[..blk.vdf.len().min(20)])
                );
            }
        }

        // Check blocks at adjacent levels to see if they share the same proposal period
        println!();
        println!("  Proposal period lookup for each non-finalized level (Seek semantics):");
        for (lvl, blocks) in &blocks_by_level {
            let pp = get_proposal_period_seek(&storage, *lvl)?;
            let pp_str = match pp {
                Some(p) => format!("period {p}"),
                None => "⚠ NONE (Seek past end)".to_string(),
            };
            let has_target = blocks.iter().any(|(h, _)| *h == target_anchor);
            let marker = if has_target {
                " ← target anchor level"
            } else {
                ""
            };
            println!(
                "    Level {lvl}: {pp_str}{marker} ({} blocks)",
                blocks.len()
            );
        }

        // Dump the proposal_period_levels_map entries around the target level
        println!();
        println!("  Proposal period map entries (raw Seek walk around target level):");
        // Scan entries starting from a level well below the target
        let scan_from = target_anchor_level.saturating_sub(200);
        let mut seen = 0;
        let mut prev_key = 0u64;
        for res in storage.iter(Column::ProposalPeriodLevelsMap) {
            let (key, value) = res?;
            if key.len() == 8 && value.len() == 8 {
                let mut k = [0u8; 8];
                let mut v = [0u8; 8];
                k.copy_from_slice(&key);
                v.copy_from_slice(&value);
                let map_level = u64::from_le_bytes(k);
                let map_period = u64::from_le_bytes(v);
                // Show entries near our target
                if map_level >= scan_from && map_level <= target_anchor_level + 200 {
                    let gap = if prev_key > 0 {
                        format!(" (gap: {} levels)", map_level - prev_key)
                    } else {
                        String::new()
                    };
                    let marker = if map_level == target_anchor_level {
                        " ← EXACT target level"
                    } else if map_level > target_anchor_level && seen == 0 {
                        " ← first entry ABOVE target"
                    } else {
                        ""
                    };
                    if map_level > target_anchor_level {
                        seen += 1;
                    }
                    println!("    map[{map_level}] = period {map_period}{gap}{marker}");
                    if seen > 5 {
                        break;
                    }
                }
                if map_level >= scan_from {
                    prev_key = map_level;
                }
            }
        }
    } else {
        println!("  No target anchor to analyze");
    }

    println!();
    print_section("7. DAG BLOCKS LEVEL INDEX (last 10 levels)");
    let last_dag_level = storage.dag().last_blocks_level()?;
    println!("  Last indexed DAG level: {last_dag_level}");
    let check_start = if last_dag_level > 9 {
        last_dag_level - 9
    } else {
        1
    };
    for lvl in check_start..=last_dag_level {
        let blocks = storage.dag().blocks_by_level(lvl)?;
        println!("    Level {lvl}: {} block(s)", blocks.len());
    }

    // Check for gaps in recent levels
    println!();
    print_section("8. DAG LEVEL GAP ANALYSIS (last 100 levels)");
    let gap_start = if last_dag_level > 99 {
        last_dag_level - 99
    } else {
        1
    };
    let mut gaps = Vec::new();
    for lvl in gap_start..=last_dag_level {
        let blocks = storage.dag().blocks_by_level(lvl)?;
        if blocks.is_empty() {
            gaps.push(lvl);
        }
    }
    if gaps.is_empty() {
        println!("  No gaps found in last 100 levels");
    } else {
        println!(
            "  Found {} empty levels: {:?}",
            gaps.len(),
            &gaps[..gaps.len().min(20)]
        );
    }

    println!();
    print_section("9. ANCHOR BLOCK ANALYSIS");
    // Analyze the last finalized anchor
    if !last_anchor.is_zero() {
        println!("  Last finalized anchor:");
        analyze_dag_block(&storage, last_anchor, "    ")?;
    } else {
        println!("  Last finalized anchor: null (empty PBFT block)");
    }

    // Analyze target anchors from proposed blocks for the next period
    for anchor in &target_anchors {
        println!("  Target anchor (proposed for period {}):", last_period + 1);
        analyze_dag_block(&storage, *anchor, "    ")?;
    }

    println!();
    print_section("10. PROPOSAL PERIOD LEVEL MAP (around target period)");
    let target_period = last_period + 1;
    // Find the DAG level that maps to target_period
    // We'll scan a range of the proposal_period_levels_map
    println!("  Target period: {target_period}");
    println!("  Checking proposal levels near target...");

    // Use the last_dag_level as a reference and scan backward
    let scan_start = if last_dag_level > 200 {
        last_dag_level - 200
    } else {
        1
    };
    let mut found_levels_for_target = Vec::new();
    let mut found_levels_nearby = Vec::new();
    for lvl in scan_start..=last_dag_level {
        if let Some(period) = storage.dag().proposal_period_for_dag_level(lvl)? {
            if period == target_period {
                found_levels_for_target.push(lvl);
            }
            if period >= target_period.saturating_sub(2) && period <= target_period + 2 {
                found_levels_nearby.push((lvl, period));
            }
        }
    }
    if !found_levels_for_target.is_empty() {
        println!(
            "  DAG levels mapping to period {target_period}: {:?}",
            found_levels_for_target
        );
    } else {
        println!("  No DAG levels found mapping to period {target_period}");
    }
    if !found_levels_nearby.is_empty() {
        println!("  Nearby level→period mappings:");
        for (lvl, period) in &found_levels_nearby {
            let blocks = storage.dag().blocks_by_level(*lvl)?;
            let nonfinalized_at = nonfinalized.get(lvl).map(|v| v.len()).unwrap_or(0);
            println!(
                "    Level {lvl} → period {period} (indexed: {} blocks, nonfinalized: {nonfinalized_at})",
                blocks.len()
            );
        }
    }

    println!();
    print_section("11. PILLAR CHAIN STATE");
    match storage.pillar().current_pillar_block_data_rlp()? {
        Some(data) => {
            let rlp = Rlp::new(&data);
            println!(
                "  Current pillar block data: {} RLP items, {} bytes",
                rlp.item_count().unwrap_or(0),
                data.len()
            );
        }
        None => println!("  No current pillar block data"),
    }
    match storage.pillar().latest_pillar_block_rlp()? {
        Some(data) => println!("  Latest pillar block: {} bytes", data.len()),
        None => println!("  No latest pillar block"),
    }

    println!();
    print_section("12. DIAGNOSIS SUMMARY");
    println!();
    println!("  Last finalized period:  {last_period}");
    println!("  PBFT chain size:        {pbft_chain_size}");
    println!("  Next expected period:   {}", last_period + 1);
    println!("  Target period (proposed): {target_period}");
    println!("  PBFT round:             {:?}", round);
    println!("  PBFT step:              {:?}", step);
    println!("  Non-finalized DAG:      {total_nonfinalized} blocks");
    println!(
        "  Stale proposed blocks:  {} (from old periods)",
        by_period
            .iter()
            .filter(|(p, _)| **p < target_period)
            .map(|(_, v)| v.len())
            .sum::<usize>()
    );
    println!(
        "  Current proposed blocks: {}",
        by_period.get(&target_period).map(|v| v.len()).unwrap_or(0)
    );
    println!();

    // Check consistency
    let mut issues = Vec::new();
    let mut ok_items = Vec::new();

    if last_period != pbft_chain_size {
        issues.push(format!("final_chain_meta period ({last_period}) != pbft_chain size ({pbft_chain_size}) — possible incomplete finalization"));
    } else {
        ok_items.push(format!(
            "PBFT chain size matches final_chain_meta ({last_period})"
        ));
    }

    if total_nonfinalized == 0 {
        issues.push(
            "NO non-finalized DAG blocks — anchor cannot be loaded into DAG graph".to_string(),
        );
    } else {
        ok_items.push(format!(
            "{total_nonfinalized} non-finalized DAG blocks present"
        ));
    }

    if found_levels_for_target.is_empty() {
        issues.push(format!("No proposal_period_levels_map entry for period {target_period} — DAG level mapping incomplete"));
    } else {
        ok_items.push(format!(
            "Proposal period map exists for period {target_period}"
        ));
    }

    for anchor in &target_anchors {
        let exists = storage.dag().dag_block_in_db(*anchor)?;
        if exists {
            ok_items.push(format!("Target anchor {} exists in DB", h256_short(anchor)));
        } else {
            issues.push(format!(
                "Target anchor {} MISSING from DB",
                h256_short(anchor)
            ));
        }
    }

    let stale_count: usize = by_period
        .iter()
        .filter(|(p, _)| **p < target_period)
        .map(|(_, v)| v.len())
        .sum();
    if stale_count > 0 {
        let stale_periods: Vec<_> = by_period
            .keys()
            .filter(|&&p| p < target_period)
            .copied()
            .collect();
        issues.push(format!("{stale_count} stale proposed blocks from old periods {stale_periods:?} — should be cleaned up"));
    }

    if round.unwrap_or(0) > 5 {
        issues.push(format!(
            "PBFT round {} is high — consensus is stuck cycling",
            round.unwrap_or(0)
        ));
    }

    println!("  Issues found: {}", issues.len());
    for issue in &issues {
        println!("  ⚠ {issue}");
    }
    println!();
    println!("  OK checks: {}", ok_items.len());
    for item in &ok_items {
        println!("  ✓ {item}");
    }

    println!();
    println!("  Root cause analysis:");
    println!("  → recoverDag() processes non-finalized blocks level by level.");
    println!(
        "    Within each level, if any block fails validation, `break` skips remaining blocks at THAT level."
    );
    println!("    (The break only exits the inner loop, not the outer level loop.)");
    println!();
    println!("  → Validation checks in recoverDag (any failure → break for that level):");
    println!("    1. Block already finalized (dag_block_period exists)");
    println!(
        "    2. Proposal period mapping missing (getProposalPeriodForDagLevel = Seek, not exact)"
    );
    println!("    3. VRF key missing for sender at proposal period");
    println!("    4. VDF verification failure (InvalidVdfSortition exception)");
    println!(
        "    5. pivotAndTipsAvailable fails (pivot/tip not found via getDagBlock = DB + cache)"
    );
    println!("    6. addDagBlock fails (should not happen with save=false)");
    println!();
    println!("  → If the target anchor or its pivot block fail checks 3 or 4 (VDF/VRF),");
    println!("    the anchor is never added to the in-memory DAG graph.");
    println!("  → computeOrder() then returns null_vertex → 'Create period anchor failed'.");
    println!("  → This cascades to 'Missing dag blocks' and PBFT consensus failure.");
    println!();
    println!("  Note: Checks 3-4 (VRF key, VDF) cannot be verified from this tool — they");
    println!("  require chain state access. If all other checks pass, VDF/VRF failure is the");
    println!("  most likely root cause. Check node logs for 'failed on VDF verification' or");
    println!("  'missing VRF key' near the stuck period.");

    Ok(())
}

fn check_dag_block_existence_inline(storage: &Storage, hash: H256) -> Result<()> {
    let in_nonfinalized = storage.dag().dag_block_in_db(hash)?;
    let in_period = storage.dag().dag_block_period(hash);
    match (in_nonfinalized, &in_period) {
        (true, Ok((period, pos))) => {
            println!("EXISTS (finalized period {period}, pos {pos})");
        }
        (true, Err(_)) => {
            println!("EXISTS (non-finalized)");
        }
        (false, _) => {
            println!("⚠ MISSING from database!");
        }
    }
    Ok(())
}

fn analyze_dag_block(storage: &Storage, hash: H256, indent: &str) -> Result<()> {
    println!("{indent}Hash: {hash:?}");
    let in_nonfinalized = storage.dag().dag_block_in_db(hash)?;
    let in_period = storage.dag().dag_block_period(hash);
    match (in_nonfinalized, &in_period) {
        (true, Ok((period, pos))) => {
            println!("{indent}Status: finalized (period {period}, pos {pos})")
        }
        (true, Err(_)) => println!("{indent}Status: non-finalized"),
        (false, _) => {
            println!("{indent}Status: ⚠ MISSING!");
            return Ok(());
        }
    }
    match storage.dag().dag_block(hash) {
        Ok(block) => {
            println!("{indent}Level: {}", block.level);
            println!("{indent}Pivot: {:?}", block.pivot);
            println!("{indent}Tips: {} tips", block.tips.len());
            println!("{indent}Transactions: {}", block.transactions.len());
            println!("{indent}Timestamp: {}", format_timestamp(block.timestamp));
            // Check pivot exists
            if !block.pivot.is_zero() {
                let pivot_exists = storage.dag().dag_block_in_db(block.pivot)?;
                println!("{indent}Pivot exists: {pivot_exists}");
            }
            // Check proposal period for this level (using Seek semantics like C++)
            match get_proposal_period_seek(storage, block.level)? {
                Some(pp) => println!(
                    "{indent}Proposal period for level {} (Seek): {pp}",
                    block.level
                ),
                None => println!(
                    "{indent}Proposal period for level {} (Seek): ⚠ NOT SET",
                    block.level
                ),
            }
        }
        Err(e) => println!("{indent}Could not decode block: {e}"),
    }
    Ok(())
}

fn read_last_block_number(storage: &Storage) -> Result<u64> {
    // DBMetaKeys::LAST_NUMBER = 1, stored in final_chain_meta
    // C++ enum class is sizeof(int) = 4 bytes
    let key_4 = 1i32.to_le_bytes();
    if let Some(bytes) = storage.get_raw(Column::FinalChainMeta, &key_4)? {
        if bytes.len() >= 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&bytes[..8]);
            return Ok(u64::from_le_bytes(buf));
        } else if bytes.len() >= 4 {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&bytes[..4]);
            return Ok(u32::from_le_bytes(buf) as u64);
        }
    }
    // Fallback: try iterating for any key
    eprintln!("  [debug] final_chain_meta: key [1,0,0,0] not found, iterating column...");
    for res in storage.iter(Column::FinalChainMeta) {
        let (key, value) = res?;
        eprintln!(
            "  [debug] final_chain_meta key: {:?} ({} bytes), value: {} bytes",
            &*key,
            key.len(),
            value.len()
        );
        if value.len() >= 8 {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&value[..8]);
            let num = u64::from_le_bytes(buf);
            eprintln!("  [debug]   as u64: {num}");
        }
        if value.len() >= 4 {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&value[..4]);
            let num = u32::from_le_bytes(buf);
            eprintln!("  [debug]   as u32: {num}");
        }
    }
    Ok(0)
}

fn read_pbft_chain_head(storage: &Storage) -> Result<(u64, H256)> {
    // PBFT head is stored in pbft_head with key = zero hash, value = JSON string
    let zero_hash = H256::zero();
    match storage.pbft().pbft_head(zero_hash)? {
        Some(data) => {
            let json_str = String::from_utf8_lossy(&data);
            let size = extract_json_u64(&json_str, "size").unwrap_or(0);
            let last_hash_str =
                extract_json_string(&json_str, "last_pbft_block_hash").unwrap_or_default();
            let last_hash = if last_hash_str.len() == 66 {
                H256::from_slice(&hex_decode(&last_hash_str[2..]).unwrap_or_default())
            } else if last_hash_str.len() == 64 {
                H256::from_slice(&hex_decode(last_hash_str).unwrap_or_default())
            } else {
                H256::zero()
            };
            Ok((size, last_hash))
        }
        None => {
            // Fallback: iterate pbft_head to find entries
            eprintln!("  [debug] pbft_head: zero-hash key not found, iterating column...");
            for res in storage.iter(Column::PbftHead) {
                let (key, value) = res?;
                let json_str = String::from_utf8_lossy(&value);
                eprintln!(
                    "  [debug] pbft_head key: {} bytes, value preview: {:.200}",
                    key.len(),
                    json_str
                );
                let size = extract_json_u64(&json_str, "size").unwrap_or(0);
                if size > 0 {
                    let last_hash_str =
                        extract_json_string(&json_str, "last_pbft_block_hash").unwrap_or_default();
                    let last_hash = if last_hash_str.len() == 66 {
                        H256::from_slice(&hex_decode(&last_hash_str[2..]).unwrap_or_default())
                    } else if last_hash_str.len() == 64 {
                        H256::from_slice(&hex_decode(last_hash_str).unwrap_or_default())
                    } else {
                        H256::zero()
                    };
                    return Ok((size, last_hash));
                }
            }
            Ok((0, H256::zero()))
        }
    }
}

fn extract_json_u64(json: &str, key: &str) -> Option<u64> {
    let pattern = format!("\"{key}\"");
    let key_pos = json.find(&pattern)?;
    let after_key = &json[key_pos + pattern.len()..];
    let colon_pos = after_key.find(':')?;
    let rest = after_key[colon_pos + 1..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn extract_json_string<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pattern = format!("\"{key}\"");
    let key_pos = json.find(&pattern)?;
    let after_key = &json[key_pos + pattern.len()..];
    let colon_pos = after_key.find(':')?;
    let rest = after_key[colon_pos + 1..].trim_start();
    if rest.starts_with('"') {
        let start = 1;
        let end = rest[start..].find('"')? + start;
        Some(&rest[start..end])
    } else {
        None
    }
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        bytes.push(u8::from_str_radix(&s[i..i + 2], 16).ok()?);
    }
    Some(bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct PeriodSummary {
    period: u64,
    prev_hash: H256,
    pivot_dag: H256,
    timestamp: u64,
    dag_count: usize,
    txn_count: usize,
}

fn read_period_summary(storage: &Storage, p: u64) -> Result<Option<PeriodSummary>> {
    let raw = storage.period().period_data_raw(p)?;
    if raw.is_empty() {
        return Ok(None);
    }
    let period_rlp = Rlp::new(&raw);
    let pbft_rlp = period_rlp.at(PBFT_BLOCK_POS)?;
    let (prev_hash, pivot_dag, period, timestamp) = decode_pbft_block_fields(pbft_rlp.as_raw())?;
    let dag_blocks_rlp = period_rlp.at(DAG_BLOCKS_POS)?;
    let dag_count = if dag_blocks_rlp.is_empty() {
        0
    } else if dag_blocks_rlp.is_list() {
        dag_blocks_rlp.item_count().unwrap_or(0)
    } else {
        0
    };
    // Transactions at position 3
    let txn_count = if period_rlp.item_count().unwrap_or(0) > 3 {
        let txns_rlp = period_rlp.at(3)?;
        if txns_rlp.is_list() {
            txns_rlp.item_count().unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };
    Ok(Some(PeriodSummary {
        period,
        prev_hash,
        pivot_dag,
        timestamp,
        dag_count,
        txn_count,
    }))
}

fn keccak256(data: &[u8]) -> H256 {
    use tiny_keccak::{Hasher, Keccak};
    let mut hasher = Keccak::v256();
    hasher.update(data);
    let mut output = [0u8; 32];
    hasher.finalize(&mut output);
    H256::from(output)
}

/// Replicate C++ getProposalPeriodForDagLevel() which uses Seek (first key >= level).
/// The Rust proposal_period_for_dag_level() uses exact key lookup, which is WRONG for
/// levels between mapping entries.
fn get_proposal_period_seek(storage: &Storage, level: u64) -> Result<Option<u64>> {
    match storage.seek_forward(Column::ProposalPeriodLevelsMap, &level.to_le_bytes())? {
        Some((_key, value)) => {
            if value.len() != 8 {
                return Ok(None);
            }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&value);
            Ok(Some(u64::from_le_bytes(buf)))
        }
        None => Ok(None),
    }
}

fn format_timestamp(ts: u64) -> String {
    // Convert epoch seconds to a readable date
    let days = ts / 86400;
    let h = (ts % 86400) / 3600;
    let m = (ts % 3600) / 60;
    let s = ts % 60;
    // Days since 1970-01-01
    let (year, month, day) = days_to_date(days);
    format!("{year}-{month:02}-{day:02} {h:02}:{m:02}:{s:02} UTC")
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Civil days to Y/M/D (Euclidean affine algorithm)
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let yr = if mo <= 2 { y + 1 } else { y };
    (yr, mo, d)
}

fn print_section(title: &str) {
    println!("--- {title} ---");
}

/// Find the last mapping level that maps to the given period.
/// The proposal_period_levels_map stores: (anchor_level + kMaxLevelsPerPeriod) → period
/// We scan backward through the map to find the entry for our target period.
fn find_last_mapping_level(storage: &Storage, target_period: u64) -> Result<u64> {
    // Scan recent levels from the DAG level index
    let last_level = storage.dag().last_blocks_level()?;
    let scan_start = last_level.saturating_sub(500);
    let mut best_level = 0u64;
    for lvl in scan_start..=last_level {
        if let Some(period) = storage.dag().proposal_period_for_dag_level(lvl)?
            && period == target_period
        {
            best_level = lvl;
        }
    }
    if best_level > 0 {
        return Ok(best_level);
    }
    // Fallback: iterate the whole column
    for res in storage.iter(Column::ProposalPeriodLevelsMap) {
        let (key, value) = res?;
        if key.len() == 8 && value.len() == 8 {
            let mut k = [0u8; 8];
            let mut v = [0u8; 8];
            k.copy_from_slice(&key);
            v.copy_from_slice(&value);
            let period = u64::from_le_bytes(v);
            if period == target_period {
                best_level = u64::from_le_bytes(k);
            }
        }
    }
    Ok(best_level)
}
