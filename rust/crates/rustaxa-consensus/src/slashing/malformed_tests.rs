//! Independent short RLP encodings for malformed slashing input.

use super::*;

fn bytes(value: &[u8]) -> Vec<u8> {
    if value.len() == 1 && value[0] < 0x80 {
        return value.to_vec();
    }
    let mut out = if value.len() <= 55 {
        vec![0x80 + value.len() as u8]
    } else {
        assert!(value.len() < 256);
        vec![0xb8, value.len() as u8]
    };
    out.extend_from_slice(value);
    out
}

fn list(fields: &[Vec<u8>]) -> Vec<u8> {
    let size: usize = fields.iter().map(Vec::len).sum();
    assert!(size < 256);
    let mut out = if size <= 55 {
        vec![0xc0 + size as u8]
    } else {
        vec![0xf8, size as u8]
    };
    for field in fields {
        out.extend_from_slice(field);
    }
    out
}

fn sortition() -> Vec<Vec<u8>> {
    vec![vec![1], vec![2], vec![4], bytes(&[0x5a; 80])]
}

fn vote(sortition: &[u8], block: u8) -> Vec<Vec<u8>> {
    vec![bytes(&[block; 32]), bytes(sortition), bytes(&[0; 65])]
}

#[test]
fn malformed_slashing_rlp_rejects_each_position_without_panic() {
    let good_sortition = list(&sortition());
    let fields = vote(&good_sortition, 1);
    let good_a = list(&fields);
    let good_b = list(&vote(&good_sortition, 2));
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("vote empty", vec![]),
        ("vote string", bytes(&[1, 2, 3])),
        ("vote missing item", list(&fields[..2])),
        (
            "vote extra item",
            list(&[fields.clone(), vec![vec![0x80]]].concat()),
        ),
        ("vote truncated header", vec![0xf8]),
        (
            "vote truncated payload",
            good_a[..good_a.len() - 1].to_vec(),
        ),
        ("vote trailing bytes", [good_a.clone(), vec![0x80]].concat()),
        ("vote long short list", vec![0xf8, 1, 0x80]),
        (
            "vote zero length prefix",
            [vec![0xf9, 0, good_a[1]], good_a[2..].to_vec()].concat(),
        ),
        ("vote oversized list", vec![0xfb, 1, 0, 0, 0]),
        (
            "vote overflowing list",
            [vec![0xff], vec![0xff; 8]].concat(),
        ),
        (
            "vote overflowing string",
            [vec![0xbf], vec![0xff; 8]].concat(),
        ),
    ];
    for (name, index, replacement) in [
        ("block short", 0, bytes(&[1; 31])),
        ("block long", 0, bytes(&[1; 33])),
        ("block list", 0, vec![0xc0]),
        ("sortition list item", 1, list(&sortition())),
        ("signature short", 2, bytes(&[0; 64])),
        ("signature long", 2, bytes(&[0; 66])),
        ("signature list", 2, vec![0xc0]),
        ("child oversized string", 1, vec![0xbb, 1, 0, 0, 0]),
        (
            "child overflowing string",
            1,
            [vec![0xbf], vec![0xff; 8]].concat(),
        ),
        ("child truncated header", 2, vec![0xb8]),
        ("child nonminimal string", 0, vec![0xb8, 1, 1]),
    ] {
        let mut value = fields.clone();
        value[index] = replacement;
        cases.push((name, list(&value)));
    }
    let sf = sortition();
    let mut bad_sortitions = vec![
        ("sortition empty", vec![]),
        ("sortition string", bytes(&[1, 2, 3])),
        ("sortition missing item", list(&sf[..3])),
        (
            "sortition extra item",
            list(&[sf.clone(), vec![vec![0x80]]].concat()),
        ),
        ("sortition truncated header", vec![0xf8]),
        (
            "sortition truncated payload",
            good_sortition[..good_sortition.len() - 1].to_vec(),
        ),
        (
            "sortition trailing",
            [good_sortition.clone(), vec![0x80]].concat(),
        ),
        ("sortition nonminimal list", vec![0xf8, 1, 0x80]),
        (
            "sortition zero length prefix",
            [
                vec![0xf9, 0, good_sortition[1]],
                good_sortition[2..].to_vec(),
            ]
            .concat(),
        ),
        ("sortition oversized", vec![0xfb, 1, 0, 0, 0]),
        (
            "sortition overflowing",
            [vec![0xff], vec![0xff; 8]].concat(),
        ),
    ];
    for (name, index, replacement) in [
        ("period list", 0, vec![0xc0]),
        ("round list", 1, vec![0xc0]),
        ("step list", 2, vec![0xc0]),
        ("period width", 0, bytes(&[1; 9])),
        ("round width", 1, bytes(&[1; 5])),
        ("step width", 2, bytes(&[1; 5])),
        ("period nonminimal single", 0, vec![0x81, 1]),
        ("period leading zero", 0, vec![0x82, 0, 1]),
        ("round nonminimal zero", 1, vec![0]),
        ("step long short string", 2, vec![0xb8, 1, 4]),
        ("proof short", 3, bytes(&[0; 79])),
        ("proof long", 3, bytes(&[0; 81])),
        ("proof list", 3, vec![0xc0]),
        ("proof truncated header", 3, vec![0xb8]),
        (
            "proof overflowing length",
            3,
            [vec![0xbf], vec![0xff; 8]].concat(),
        ),
    ] {
        let mut value = sf.clone();
        value[index] = replacement;
        bad_sortitions.push((name, list(&value)));
    }
    for (name, malformed) in bad_sortitions {
        cases.push((name, list(&vote(&malformed, 1))));
    }
    cases.push((
        "incomplete child after complete vote",
        list(&[fields.clone(), vec![vec![0xb8]]].concat()),
    ));
    let incomplete_sortition = list(&[sortition(), vec![vec![0xb8]]].concat());
    cases.push((
        "incomplete child after complete sortition",
        list(&vote(&incomplete_sortition, 1)),
    ));
    assert_eq!(cases.len(), 51);
    for (name, malformed) in &cases {
        assert!(malformed.len() < 256, "{name}");
        for position in 0..2 {
            let input = if position == 0 {
                commit_double_voting_proof_call_data(malformed, &good_b)
            } else {
                commit_double_voting_proof_call_data(&good_a, malformed)
            };
            let result =
                std::panic::catch_unwind(|| verify_legacy_double_voting_proof_call_data(&input));
            assert!(result.is_ok(), "panic: {name} position {position}");
            assert!(
                matches!(
                    result.unwrap(),
                    Err(LegacyDoubleVotingProofDecodeError::NestedRlp(_))
                ),
                "{name} position {position}"
            );
        }
    }
    let control = verify_legacy_double_voting_proof_call_data(
        &commit_double_voting_proof_call_data(&good_a, &good_b),
    );
    assert!(
        matches!(control, Err(LegacyDoubleVotingProofDecodeError::Semantic(message)) if message == "invalid vote signature")
    );
    eprintln!(
        "SLASHING_RLP_CASES malformed=51 positions=2 classified=102 canonical_control=1 maximum_vote_bytes=255"
    );
}
