//! Synthetic recorded owners exercise stops only; they confer no historical authority.
use super::*;
use k256::ecdsa::SigningKey;
use tiny_keccak::{Hasher, Keccak};
fn pins() -> Value {
    json!({"historical_capture":{},"current_compiled":compiled_pins(),
        "executable":{"path":"/synthetic/test-executable","bytes":1,"sha256":"00".repeat(32)}})
}

fn evidence() -> Evidence {
    Evidence {
        rows: serde_json::from_slice(include_bytes!(
            "../tests/fixtures/interval_history_rows.json"
        ))
        .unwrap(),
        historical_pins: json!({}),
    }
}
fn hash(raw: &[u8]) -> H256 {
    let mut hasher = Keccak::v256();
    hasher.update(raw);
    let mut out = [0; 32];
    hasher.finalize(&mut out);
    out.into()
}
fn replace(raw: &[u8], position: usize, value: &[u8]) -> Vec<u8> {
    let row = Rlp::new(raw);
    let mut s = RlpStream::new_list(row.item_count().unwrap());
    for i in 0..row.item_count().unwrap() {
        s.append_raw(
            if i == position {
                value
            } else {
                row.at(i).unwrap().as_raw()
            },
            1,
        );
    }
    s.out().to_vec()
}
fn bytes(raw: &[u8]) -> Vec<u8> {
    rlp::encode(&raw).to_vec()
}
#[derive(Clone)]
struct Recorded {
    requests: Vec<Read>,
    rows: Vec<Option<Vec<u8>>>,
    offset: usize,
    io_at: Option<usize>,
}
impl PointSource for Recorded {
    fn read(&mut self, request: &Read) -> Result<Option<Vec<u8>>> {
        assert_eq!(request, &self.requests[self.offset]);
        let i = self.offset;
        self.offset += 1;
        if self.io_at == Some(i) {
            anyhow::bail!("recorded io failure");
        }
        Ok(self.rows[i].clone())
    }
}
/// Generates signed, linked synthetic pairs independently from the verifier.
/// Only this private test code can construct a synthetic sealed context.
fn chain() -> (HeaderContext, Recorded) {
    chain_format(true)
}
fn chain_format(extra: bool) -> (HeaderContext, Recorded) {
    let evidence = evidence();
    let mut requests = vec![Read::Head, Read::Genesis, Read::Header(head::H)];
    let mut rows = vec![
        Some(evidence.rows.head.head),
        Some(evidence.rows.head.genesis),
        Some(evidence.rows.head.header),
    ];
    let template: parent::ParentHistoryRows =
        serde_json::from_slice(include_bytes!("../tests/fixtures/parent_history_rows.json"))
            .unwrap();
    let signing = SigningKey::from_slice(&[1; 32]).unwrap();
    let mut prev_pbft = H256::from_low_u64_be(893);
    let mut prev_final = H256::from_low_u64_be(1893);
    let mut pairs = Vec::new();
    for period in FIRST..=LAST {
        let mut unsigned = RlpStream::new_list(if extra { 8 } else { 7 });
        unsigned.append(&prev_pbft);
        for n in [2, 3, 4] {
            unsigned.append(&H256::from_low_u64_be(n));
        }
        unsigned.append(&period);
        unsigned.append(&(period + 100));
        unsigned.begin_list(0);
        if extra {
            unsigned.append(&vec![0x22_u8].as_slice());
        } // nine-field extra data
        let unsigned = unsigned.out();
        let (sig, id) = signing
            .sign_prehash_recoverable(hash(&unsigned).as_bytes())
            .unwrap();
        let mut signature = sig.to_bytes().to_vec();
        signature.push(id.to_byte());
        let mut signed = RlpStream::new_list(if extra { 9 } else { 8 });
        for i in 0..if extra { 8 } else { 7 } {
            signed.append_raw(Rlp::new(&unsigned).at(i).unwrap().as_raw(), 1);
        }
        signed.append(&signature.as_slice());
        let signed = signed.out();
        let mut data = RlpStream::new_list(4);
        data.append_raw(&signed, 1);
        for _ in 0..3 {
            data.begin_list(0);
        }
        let data = data.out().to_vec();
        let mut stored = decode_header(&template.header).unwrap();
        stored.parent_hash = prev_final;
        stored.state_root = H256::from_low_u64_be(period);
        let header = StoredBlockHeaderRlpOwned::from(&stored).as_bytes().to_vec();
        let legacy = LegacyBlockHeaderRlp::try_from(
            LegacyBlockHeaderRlpInput::new(StoredBlockHeaderRlp::new(&header), 315_000_000, 0)
                .signed_pbft_block(SignedPbftBlockRlp::new(&signed))
                .block_number(FinalChainBlockNumber::new(period)),
        )
        .unwrap();
        prev_final = legacy.hash().unwrap();
        prev_pbft = hash(&signed);
        pairs.push((period, header, data));
    }
    for (period, header, data) in pairs.into_iter().rev() {
        requests.extend([Read::Header(period), Read::PeriodData(period)]);
        rows.extend([Some(header), Some(data)]);
    }
    (
        HeaderContext {
            period: LAST,
            expected_final: prev_final,
            expected_pbft: prev_pbft,
        },
        Recorded {
            requests,
            rows,
            offset: 0,
            io_at: None,
        },
    )
}
#[test]
fn delayed_headers_fixed_interval_revalidates_boundary_and_preserves_old_endpoint() {
    let mut evidence = evidence();
    assert_eq!(evidence.context().unwrap().period, LAST);
    let old = interval::authenticate_interval(&evidence.rows).unwrap();
    assert_eq!(old.first_period, LAST + 1);
    evidence.rows.predecessors[47].header[5] ^= 1;
    assert!(evidence.context().is_err());
}
#[test]
fn delayed_headers_all_seven_synthetic_pairs_seal_and_preserve_roots() {
    let (context, mut source) = chain();
    let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
    assert_eq!(source.offset, CALL_CAP);
    assert_eq!(report["actual"]["accepted_calls"], CALL_CAP);
    let rows = report["authentication"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), COUNT);
    for (i, row) in rows.iter().enumerate() {
        let p = LAST - i as u64;
        assert_eq!(row["period"], p);
        assert_eq!(row["state_root_hex"], hex::encode(H256::from_low_u64_be(p)));
    }
    assert_eq!(report["qualification"]["state_available"], false);
    // Public entry cannot adopt these caller-chosen links/roots.
    let (_, mut source) = chain();
    let report = acquire(&evidence(), &mut source, pins()).unwrap();
    assert_eq!(source.offset, 5);
    assert!(report["authentication"]["rows"].is_null());
}
#[test]
fn delayed_headers_every_stage_failure_stops_before_next_read_and_never_leaks_prefix() {
    for i in 0..CALL_CAP {
        for failure in 0..4 {
            let (context, mut source) = chain();
            match failure {
                0 => source.rows[i] = None,
                1 => source.rows[i] = Some(vec![]),
                2 => source.rows[i] = Some(vec![0; ROW_CAP + 1]),
                _ => source.io_at = Some(i),
            }
            let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
            assert_eq!(source.offset, i + 1, "stage {i} failure {failure}");
            assert_eq!(report["actual"]["attempted_calls"], i + 1);
            assert!(report["authentication"]["rows"].is_null());
            assert_eq!(
                report["qualification"]["headers_and_roots_authenticated_to_local_h"],
                false
            );
            if failure == 2 {
                assert_eq!(report["observations"][i]["returned_bytes"], ROW_CAP + 1);
            }
            if failure == 3 {
                assert_eq!(report["authentication"]["status"], "rejected");
            }
        }
    }
}
#[test]
fn delayed_headers_identity_canonical_signature_scalar_and_domain_faults() {
    for i in 0..3 {
        let (context, mut source) = chain();
        source.rows[i].as_mut().unwrap()[0] ^= 1;
        let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
        assert_eq!(source.offset, i + 1);
        assert!(report["authentication"]["rows"].is_null());
    }
    let (context, source) = chain();
    let header = source.rows[3].as_ref().unwrap();
    let data = source.rows[4].as_ref().unwrap();
    assert!(context.check(header, data).is_ok());
    let signed = Rlp::new(data).at(0).unwrap();
    for pbft in [
        replace(signed.as_raw(), 4, &rlp::encode(&(LAST - 1))),
        replace(signed.as_raw(), 5, &[0]),
        replace(signed.as_raw(), 8, &bytes(&[0; 65])),
        replace(signed.as_raw(), 7, &[0xc0]),
        vec![0xc0],
    ] {
        assert!(context.check(header, &replace(data, 0, &pbft)).is_err());
    }
    let mut wrong_final = HeaderContext {
        period: LAST,
        expected_final: context.expected_final,
        expected_pbft: context.expected_pbft,
    };
    wrong_final.expected_final = H256::zero();
    assert!(
        wrong_final
            .check(header, data)
            .unwrap_err()
            .to_string()
            .contains("FinalChain")
    );
    let mut wrong_pbft = HeaderContext {
        period: LAST,
        expected_final: context.expected_final,
        expected_pbft: context.expected_pbft,
    };
    wrong_pbft.expected_pbft = H256::zero();
    assert!(
        wrong_pbft
            .check(header, data)
            .unwrap_err()
            .to_string()
            .contains("PBFT")
    );
    for bad in [
        vec![0xc0],
        {
            let mut v = header.clone();
            v.push(0);
            v
        },
        replace(header, 1, &[0x80]),
    ] {
        assert!(decode_header(&bad).is_err());
    }
    for next in [LAST - 1, LAST + 1] {
        let other = HeaderContext {
            period: next,
            expected_final: context.expected_final,
            expected_pbft: context.expected_pbft,
        };
        assert!(other.check(header, data).is_err());
    }
}
#[test]
fn delayed_headers_swapped_skipped_repeated_pairs_cannot_complete() {
    for (a, b) in [(3, 5), (4, 6), (3, 7)] {
        let (context, mut source) = chain();
        source.rows.swap(a, b);
        let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
        assert!(report["authentication"]["rows"].is_null());
        assert!(source.offset <= 5);
    }
    let (context, mut source) = chain();
    source.rows[5] = source.rows[3].clone();
    source.rows[6] = source.rows[4].clone();
    let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
    assert_eq!(source.offset, 7);
    assert!(report["authentication"]["rows"].is_null());
}
#[test]
fn delayed_headers_strict_admission_and_replay_reject_edits() {
    for raw in [
        b"{\"x\":1,\"x\":2}".as_slice(),
        b"{\"x\":1.0}",
        b"{} {}",
        b"{\"x\":[256]}garbage",
    ] {
        assert!(parse_unique(raw).is_err());
    }
    assert!(Evidence::decode(b"{}").is_err());
    let mut rows = evidence().rows;
    let value = serde_json::to_value(&rows).unwrap();
    let mut value = value;
    value["head"]["head"][0] = json!(256);
    assert!(serde_json::from_value::<interval::IntervalHistoryRows>(value).is_err());
    rows.predecessors.swap(46, 47);
    assert!(interval::authenticate_interval(&rows).is_err());
    let mut source = Recorded {
        requests: vec![Read::Head],
        rows: vec![None],
        offset: 0,
        io_at: None,
    };
    let report = acquire(&evidence(), &mut source, pins()).unwrap();
    let encoded = interval::encode_bounded(&report, OUTPUT_CAP).unwrap();
    assert_eq!(replay(&evidence(), &encoded).unwrap(), report);
    for field in [
        "schema",
        "actual",
        "authentication",
        "content_sha256",
        "qualification",
        "extra",
    ] {
        let mut changed = report.clone();
        changed[field] = json!(true);
        assert!(replay(&evidence(), &serde_json::to_vec(&changed).unwrap()).is_err());
    }
}
#[test]
fn delayed_headers_exact_budgets_rlp_depth_nodes_and_transaction_shape() {
    let mut budget = Budget::default();
    let mut source = Recorded {
        requests: vec![Read::Header(LAST), Read::PeriodData(LAST)],
        rows: vec![Some(vec![1; ROW_CAP]); 2],
        offset: 0,
        io_at: None,
    };
    budget.fetch(&mut source, Read::Header(LAST)).unwrap();
    budget.fetch(&mut source, Read::PeriodData(LAST)).unwrap();
    assert_eq!(budget.pair, ROW_CAP * 2);
    budget.attempted = CALL_CAP;
    assert!(budget.fetch(&mut source, Read::Head).is_err());
    for returned in [TOTAL_CAP, TOTAL_CAP - ROW_CAP] {
        let mut budget = Budget {
            returned,
            ..Default::default()
        };
        let mut source = Recorded {
            requests: vec![Read::Header(LAST)],
            rows: vec![Some(vec![1; ROW_CAP])],
            offset: 0,
            io_at: None,
        };
        assert_eq!(
            budget.fetch(&mut source, Read::Header(LAST)).is_ok(),
            returned == TOTAL_CAP - ROW_CAP
        );
    }
    let mut nested = vec![0xc0];
    for _ in 0..64 {
        let mut s = RlpStream::new_list(1);
        s.append_raw(&nested, 1);
        nested = s.out().to_vec();
    }
    assert!(head::canonical_bounded(&nested, 32768).is_ok());
    let mut s = RlpStream::new_list(1);
    s.append_raw(&nested, 1);
    assert!(head::canonical_bounded(&s.out(), 32768).is_err());
    let mut nodes = RlpStream::new_list(32767);
    for _ in 0..32767 {
        nodes.append_empty_data();
    }
    assert!(head::canonical_bounded(&nodes.out(), 32768).is_ok());
    let mut nodes = RlpStream::new_list(32768);
    for _ in 0..32768 {
        nodes.append_empty_data();
    }
    assert!(head::canonical_bounded(&nodes.out(), 32768).is_err());
    let (context, source) = chain();
    let data = source.rows[4].as_ref().unwrap();
    let header = source.rows[3].as_ref().unwrap();
    for n in [256, 257] {
        let mut txs = RlpStream::new_list(n);
        for _ in 0..n {
            txs.append_empty_data();
        }
        assert_eq!(
            context.check(header, &replace(data, 3, &txs.out())).is_ok(),
            n == 256
        );
    }
    for n in [16380, 16384] {
        // includes encoded item overhead
        let mut txs = RlpStream::new_list(1);
        txs.append(&vec![1_u8; n].as_slice());
        assert_eq!(
            context.check(header, &replace(data, 3, &txs.out())).is_ok(),
            n == 16380
        );
    }
}
#[test]
fn delayed_headers_exclusive_publication_cleans_temp_and_preserves_existing_output() {
    let root = std::env::temp_dir().join(format!("delayed-publication-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let output = root.join("report.json");
    let report = json!({"x":1});
    let encoded = interval::encode_bounded(&report, OUTPUT_CAP).unwrap();
    assert_eq!(
        interval::encode_bounded(&report, encoded.len()).unwrap(),
        encoded
    );
    assert!(interval::encode_bounded(&report, encoded.len() - 1).is_err());
    publish(&output, &report).unwrap();
    assert!(publish(&output, &json!({"x":2})).is_err());
    assert_eq!(fs::read(&output).unwrap(), encoded);
    assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn delayed_headers_eight_field_format_and_terminal_context() {
    let (context, mut source) = chain_format(false);
    let report = acquire_sealed(&evidence(), context, &mut source, pins()).unwrap();
    assert_eq!(
        report["qualification"]["headers_and_roots_authenticated_to_local_h"],
        true
    );
    assert_eq!(source.offset, CALL_CAP);
}
#[test]
fn delayed_headers_fixed_admission_size_hash_schema_and_path_guards() {
    let root = std::env::temp_dir().join(format!("delayed-admission-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let path = root.join("evidence.json");
    let other = root.join("other.json");
    fs::write(&path, b"{}").unwrap();
    fs::write(&other, b"{}").unwrap();
    assert!(admission::evidence(&path, &path, &sha(b"{}"), 2, 2).is_ok());
    assert!(admission::evidence(&path, &other, &sha(b"{}"), 2, 2).is_err());
    assert!(admission::evidence(&path, &path, &sha(b"{}"), 1, 2).is_err());
    assert!(admission::evidence(&path, &path, &sha(b"[]"), 2, 2).is_err());
    assert!(admission::read_capped(b"{} ".as_slice(), 2).is_err());
    let report = json!({"schema":1,"authentication":{"status":"complete_interval_lists_authenticated","rows":evidence().rows},
        "anchor_period":head::H,"first_period":parent::MIN_PERIOD,"last_period":parent::P,"source_pins":{}});
    assert!(Evidence::decode_admitted(&serde_json::to_vec(&report).unwrap()).is_ok());
    for field in ["schema", "anchor_period", "first_period", "last_period"] {
        let mut changed = report.clone();
        changed[field] = json!(0);
        assert!(Evidence::decode_admitted(&serde_json::to_vec(&changed).unwrap()).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn delayed_headers_oversized_failure_replay_validates_metadata_without_allocating_bytes() {
    // The real fixed boundary is retained; acquisition stops at first delayed header.
    for position in 0..4 {
        let (_, mut source) = chain();
        source.rows[position] = Some(vec![0; ROW_CAP + 1]);
        let report = acquire(&evidence(), &mut source, pins()).unwrap();
        let raw = serde_json::to_vec(&report).unwrap();
        assert_eq!(replay(&evidence(), &raw).unwrap(), report);
        assert_eq!(report["actual"]["returned_calls"], position + 1);
        assert!(report["authentication"]["rows"].is_null());
        for field in ["returned_bytes", "raw_sha256", "accepted", "extra"] {
            let mut changed = report.clone();
            changed["observations"][position][field] = json!(true);
            assert!(replay(&evidence(), &serde_json::to_vec(&changed).unwrap()).is_err());
        }
    }
}
#[test]
fn delayed_headers_source_pins_require_fixed_keys_and_compiled_identity() {
    let evidence = evidence();
    let good = pins();
    assert!(validate_source_pins(&evidence, &good).is_ok());
    for field in [
        "historical_capture",
        "current_compiled",
        "executable",
        "extra",
    ] {
        let mut changed = good.clone();
        changed[field] = json!(true);
        assert!(validate_source_pins(&evidence, &changed).is_err());
    }
    for field in ["path", "bytes", "sha256", "extra"] {
        let mut changed = good.clone();
        changed["executable"][field] = json!(true);
        assert!(validate_source_pins(&evidence, &changed).is_err());
    }
}
