//! Read-only Taraxa physical-node feasibility decoder. Physical branch/leaf
//! encodings differ from Ethereum hash RLP; this reader reconstructs and verifies
//! hash encodings while loading exact separate value-column bytes. It is bounded,
//! fixture-only, and rejects absent/corrupt required data instead of returning zero.
use revm::primitives::keccak256;
use std::collections::BTreeMap;
type Rows = BTreeMap<String, Vec<u8>>;
type Result<T> = std::result::Result<T, String>;

fn fail<T>(message: &str) -> Result<T> {
    Err(message.into())
}
fn data(rlp: &rlp::Rlp<'_>) -> Result<Vec<u8>> {
    rlp.data().map(Vec::from).map_err(|e| e.to_string())
}
fn at<'a>(rlp: &rlp::Rlp<'a>, index: usize) -> Result<rlp::Rlp<'a>> {
    rlp.at(index).map_err(|e| e.to_string())
}

/// Reconstructs one account hash value from its five-field physical value;
/// arbitrary nonce/balance bytes remain intact, and absent hashes are normalized.
fn value_hash(value: &[u8], account: bool) -> Result<Vec<u8>> {
    if !account {
        return Ok(rlp::encode(&value).to_vec());
    }
    let r = rlp::Rlp::new(value);
    if r.item_count().map_err(|e| e.to_string())? != 5 {
        return fail("account shape");
    }
    let mut s = rlp::RlpStream::new_list(4);
    s.append(&data(&at(&r, 0)?)?).append(&data(&at(&r, 1)?)?);
    for (i, empty) in [(2, keccak256([0x80])), (3, keccak256([]))] {
        let v = data(&at(&r, i)?)?;
        if v.is_empty() {
            s.append(&empty.as_slice());
        } else {
            if v.len() != 32 {
                return fail("account hash width");
            };
            s.append(&v);
        }
    }
    Ok(s.out().to_vec())
}

/// Resolves a physical node with a maximum 128-step traversal. Returns its
/// canonical hash RLP and accumulates exact leaves. Every referenced hash and
/// stored leaf hash hint is verified; input keys are already trie-hashed.
fn child(
    raw: &[u8],
    prefix: Vec<u8>,
    nodes: &Rows,
    values: &Rows,
    account: bool,
    leaves: &mut Rows,
    depth: usize,
) -> Result<Vec<u8>> {
    let enc = node(raw, prefix, nodes, values, account, leaves, depth)?;
    if rlp::Rlp::new(raw).is_list() && enc.len() >= 32 {
        Ok(rlp::encode(&keccak256(enc).as_slice()).to_vec())
    } else {
        Ok(enc)
    }
}

fn node(
    raw: &[u8],
    prefix: Vec<u8>,
    nodes: &Rows,
    values: &Rows,
    account: bool,
    leaves: &mut Rows,
    depth: usize,
) -> Result<Vec<u8>> {
    if depth > 128 || prefix.len() > 64 {
        return fail("node depth");
    }
    let r = rlp::Rlp::new(raw);
    if r.payload_info().map_err(|e| e.to_string())?.total() != raw.len() {
        return fail("trailing node bytes");
    }
    if !r.is_list() {
        let h = data(&r)?;
        if h.is_empty() {
            return Ok(vec![0x80]);
        };
        if h.len() != 32 {
            return fail("node reference width");
        }
        let stored = nodes.get(&hex::encode(&h)).ok_or("missing node")?;
        let enc = node(stored, prefix, nodes, values, account, leaves, depth + 1)?;
        if keccak256(&enc).as_slice() != h {
            return fail("node hash mismatch");
        }
        return Ok(raw.to_vec());
    }
    let count = r.item_count().map_err(|e| e.to_string())?;
    if count == 16 {
        let mut s = rlp::RlpStream::new_list(17);
        for i in 0..16 {
            let mut p = prefix.clone();
            p.push(i as u8);
            s.append_raw(
                &child(
                    at(&r, i)?.as_raw(),
                    p,
                    nodes,
                    values,
                    account,
                    leaves,
                    depth + 1,
                )?,
                1,
            );
        }
        s.append_empty_data();
        return Ok(s.out().to_vec());
    }
    if count != 1 && count != 2 {
        return fail("physical node arity");
    }
    let compact = data(&at(&r, 0)?)?;
    if compact.is_empty() || compact[0] >> 4 > 3 {
        return fail("compact path");
    }
    let flag = compact[0] >> 4;
    let terminal = flag & 2 != 0;
    let mut path = prefix;
    if flag & 1 != 0 {
        path.push(compact[0] & 15)
    } else if compact[0] & 15 != 0 {
        return fail("compact padding");
    }
    for b in &compact[1..] {
        path.extend([b >> 4, b & 15])
    }
    let mut s = rlp::RlpStream::new_list(2);
    s.append(&compact);
    if !terminal {
        if count != 2 {
            return fail("missing extension");
        };
        s.append_raw(
            &child(
                at(&r, 1)?.as_raw(),
                path,
                nodes,
                values,
                account,
                leaves,
                depth + 1,
            )?,
            1,
        );
        return Ok(s.out().to_vec());
    }
    if path.len() != 64 {
        return fail("leaf key width");
    }
    let key = hex::encode(
        path.as_chunks::<2>()
            .0
            .iter()
            .map(|c| (c[0] << 4) | c[1])
            .collect::<Vec<_>>(),
    );
    let content = if count == 2 {
        data(&at(&r, 1)?)?
    } else {
        vec![]
    };
    let value = if content.is_empty() || content.len() == 32 {
        values.get(&key).ok_or("missing value")?.clone()
    } else if content.len() <= 8 {
        content.clone()
    } else {
        return fail("inline value width");
    };
    if value.is_empty() {
        return fail("referenced tombstone");
    }
    let hash_value = value_hash(&value, account)?;
    s.append(&hash_value);
    let enc = s.out().to_vec();
    if content.len() == 32 && keccak256(&enc).as_slice() != content {
        return fail("leaf hash mismatch");
    }
    if leaves.insert(key, value).is_some() {
        return fail("duplicate leaf");
    }
    // Parent embeds small nodes; physical format uses a leaf-hash hint for large
    // ones. The root caller separately verifies the full hash encoding.
    Ok(enc)
}

fn read(root: &str, nodes: &Rows, values: &Rows, account: bool) -> Result<Rows> {
    if root == hex::encode(keccak256([0x80])) {
        return Ok(Rows::new());
    }
    let raw = nodes.get(root).ok_or("missing root")?;
    let mut leaves = Rows::new();
    let enc = node(raw, vec![], nodes, values, account, &mut leaves, 0)?;
    if hex::encode(keccak256(enc)) != root {
        return fail("root mismatch");
    };
    Ok(leaves)
}
fn rows(v: &serde_json::Value) -> Rows {
    v.as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), hex::decode(v.as_str().unwrap()).unwrap()))
        .collect()
}

#[test]
fn persisted_nodes_reopen_and_reconstruct_updated_roots() {
    for source in [
        include_str!("../fixtures/local.json"),
        include_str!("../fixtures/public.json"),
    ] {
        let f: serde_json::Value = serde_json::from_str(source).unwrap();
        for c in f["commitments"].as_array().unwrap() {
            let account = c["kind"] == "account";
            let values = if account {
                BTreeMap::from([(
                    c["key"].as_str().unwrap().into(),
                    hex::decode(c["disk"].as_str().unwrap()).unwrap(),
                )])
            } else {
                rows(&c["values"])
            };
            assert_eq!(
                read(
                    c["root"].as_str().unwrap(),
                    &rows(&c["nodes"]),
                    &values,
                    account
                )
                .unwrap(),
                values
            );
        }
        for c in f["node_history"].as_array().unwrap() {
            let nodes = rows(&c["nodes"]);
            let values = rows(&c["values"]);
            let root = c["root"].as_str().unwrap();
            let live = read(root, &nodes, &values, false).unwrap();
            assert_eq!(
                live,
                values
                    .iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            );
            let independent = triehash::trie_root::<keccak_hasher::KeccakHasher, _, _, _>(
                live.iter()
                    .map(|(k, v)| (hex::decode(k).unwrap(), rlp::encode(v).to_vec())),
            );
            assert_eq!(hex::encode(independent), root);
        }
    }
}

#[test]
fn persisted_node_reader_rejects_corruption_and_missing_values() {
    let f: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/local.json")).unwrap();
    let c = &f["node_history"][1];
    let root = c["root"].as_str().unwrap();
    let nodes = rows(&c["nodes"]);
    let values = rows(&c["values"]);
    assert!(read(root, &Rows::new(), &values, false).is_err());
    assert!(read(root, &nodes, &Rows::new(), false).is_err());
    let mut corrupt = nodes.clone();
    corrupt.get_mut(root).unwrap().push(0);
    assert!(read(root, &corrupt, &values, false).is_err());
    let tombstones = values.keys().map(|k| (k.clone(), vec![])).collect();
    assert!(read(root, &nodes, &tombstones, false).is_err());
}
