//! Pure binary/schema/consistency fixtures. These bytes are not MPT proofs and
//! never establish accepted native values or historical root authority.
use super::super::{ConcreteStorageWitness, ReplayStore, live};
use super::*;
use std::cell::Cell;

fn identity(period: u64) -> ConcreteStateIdentity {
    ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(period),
        state_root: [42; 32],
    }
}
fn frames(identity: ConcreteStateIdentity, count: usize) -> Vec<binary::Frame<'static>> {
    (0..count)
        .map(|ordinal| binary::Frame {
            ordinal: ordinal as u64,
            identity,
            operation: 0,
            column: 2,
            request: [ordinal as u8; 32],
            period: identity.period.as_u64(),
            outcome: binary::Outcome::Node(&[1]),
        })
        .collect()
}
fn header(identity: ConcreteStateIdentity, count: usize) -> binary::Header {
    binary::Header {
        identity,
        address: [0; 20],
        key: ConcreteStorageKey([0; 32]),
        records: count as u32,
    }
}
fn encoded(header: binary::Header, records: &[binary::Frame<'_>], account: bool) -> Vec<u8> {
    let budget = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    budget.charge(R::Records, records.len() as u64).unwrap();
    binary::encode_transcript(header, records, &budget, 0, account).unwrap()
}
struct Fixture {
    header: binary::Header,
    records: Vec<binary::Frame<'static>>,
    bytes: Vec<u8>,
    account: bool,
}
impl AsRef<[u8]> for Fixture {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}
fn schedule(count: usize) -> (live::ConcreteDiagnosticPlan, Vec<Fixture>) {
    let head = identity(live::HEAD);
    let parent = identity(live::PARENT);
    let delayed = std::array::from_fn(|i| identity(live::FIRST_DELAYED + i as u64));
    let tuples = std::array::from_fn(|i| {
        let mut address = [0; 20];
        address[19] = if i < 48 { 0xfe } else { 0xee };
        let key = if i < 48 {
            ConcreteStorageKey(codec::keccak256(&[4]))
        } else {
            let mut key = [0; 32];
            key[31] = 2;
            ConcreteStorageKey(key)
        };
        live::ConcreteDiagnosticTuple {
            identity: delayed[i % 48],
            address,
            key,
        }
    });
    let plan = live::ConcreteDiagnosticPlan {
        genesis_hash: [9; 32],
        head_header_sha256: [0; 32],
        head,
        parent,
        delayed,
        tuples,
    };
    let identities = std::iter::once(parent)
        .chain(std::iter::once(head))
        .chain(delayed)
        .chain(std::iter::once(head))
        .chain(delayed)
        .chain(delayed);
    let fixture = identities
        .enumerate()
        .map(|(i, id)| {
            let mut h = header(id, count);
            let account = i < 51;
            if !account {
                h.address = plan.tuples[i - 51].address;
                h.key = plan.tuples[i - 51].key;
            }
            let records = frames(id, count);
            let bytes = encoded(h, &records, account);
            Fixture {
                header: h,
                records,
                bytes,
                account,
            }
        })
        .collect();
    (plan, fixture)
}
fn registration(plan: &live::ConcreteDiagnosticPlan, index: usize, budget: &ConcreteWitnessBudget) {
    if index == 0 {
        budget
            .register_constructor(&[plan.parent, plan.head])
            .unwrap();
    }
    if index == 2 {
        let mut ids = [plan.head; 49];
        ids[..48].copy_from_slice(&plan.delayed);
        budget.register_constructor(&ids).unwrap();
    }
    if index >= 51 {
        let t = plan.tuples[index - 51];
        budget.tuple(t.identity, t.address, t.key).unwrap();
    }
}

// Test-only old traversal oracle. It retains the original header-per-current-
// frame/decode-per-pair work. This is not a production fallback or proof route.
fn old_consistency<T: AsRef<[u8]>>(
    id: ConcreteStateIdentity,
    current: &[binary::Frame<'_>],
    prior: &[T],
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    for frame in current {
        if frame.identity != id {
            return Err(budget.stop(ConcreteWitnessError::Identity));
        }
        if frame.period != id.period.as_u64() {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        for bytes in prior {
            budget.charge(R::FrameInspections, 1)?;
            let bytes = bytes.as_ref();
            if bytes.get(8..16) != Some(id.period.as_u64().to_le_bytes().as_slice())
                || bytes.get(16..48) != Some(id.state_root.as_slice())
            {
                continue;
            }
            let mut offset = binary::HEADER_BYTES;
            while offset < bytes.len() {
                let (old, next) = inspected_frame(bytes, offset, budget)?;
                if old.operation == frame.operation
                    && old.column == frame.column
                    && old.request == frame.request
                    && old.period == frame.period
                    && old.outcome != frame.outcome
                {
                    return Err(budget.stop(ConcreteWitnessError::Conflict));
                }
                offset = next;
            }
        }
    }
    Ok(())
}

#[test]
fn fixed147_transcript31_record_binary_stress_preserves_all_bytes_within_original_cap() {
    let (plan, fixture) = schedule(31);
    live::validate_plan(&plan).unwrap();
    assert_eq!(fixture.len(), 147);
    let capture = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let mut capture_outputs = Vec::new();
    for (index, proof) in fixture.iter().enumerate() {
        registration(&plan, index, &capture);
        let baseline = capture.usage().used(R::Records);
        capture.charge(R::Records, 31).unwrap();
        capture
            .charge(
                R::CanonicalBytes,
                (31 * size_of::<binary::Frame<'_>>()) as u64,
            )
            .unwrap();
        consistent_frames(
            proof.header.identity,
            &proof.records,
            &fixture[..index],
            &capture,
        )
        .unwrap();
        let output = binary::encode_transcript(
            proof.header,
            &proof.records,
            &capture,
            baseline,
            proof.account,
        )
        .unwrap();
        assert_eq!(output, proof.bytes);
        capture_outputs.push(output);
    }
    assert_eq!(capture.usage().used(R::FrameInspections), 232040);
    assert_eq!(capture.usage().used(R::Records), 4557);
    assert_eq!(capture.usage().used(R::RawCopyBytes), 4557);
    assert_eq!(capture.usage().used(R::EncodedBytes), 470988);
    assert_eq!(capture.usage().used(R::Registrations), 51);
    assert_eq!(capture.usage().used(R::DistinctIdentities), 50);
    assert_eq!(capture.usage().used(R::PayloadTuples), 96);
    let replay = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    for (index, proof) in fixture.iter().enumerate() {
        registration(&plan, index, &replay);
        let h = binary::admit_transcript(
            &proof.bytes,
            &replay,
            Some((
                proof.header.identity,
                proof.header.address,
                proof.header.key,
            )),
            proof.account,
        )
        .unwrap();
        consistent_transcript(&proof.bytes, h, &fixture[..index], &replay).unwrap();
        let witness = ConcreteStorageWitness {
            bytes: &proof.bytes,
            header: h,
        };
        let store = ReplayStore {
            witness: &witness,
            budget: &replay,
            offset: Cell::new(binary::HEADER_BYTES),
            ordinal: Cell::new(0),
            last_read: None,
        };
        // Consume the exact synthetic requests, without invoking or accepting an
        // MPT proof. Real root authentication remains in the live family tests.
        for record in &proof.records {
            assert_eq!(store.node("2", record.request).unwrap(), Some(vec![1]));
        }
        assert_eq!(store.offset.get(), proof.bytes.len());
        assert_eq!(store.ordinal.get(), 31);
    }
    assert_eq!(replay.usage().used(R::FrameInspections), 241154);
    assert_eq!(replay.usage().used(R::Records), 4557);
    assert_eq!(replay.usage().used(R::Operations), 4557);
    assert_eq!(replay.usage().used(R::RawCopyBytes), 4557);
    assert_eq!(
        replay.usage().used(R::CanonicalBytes),
        (4557 * size_of::<binary::Frame<'_>>()) as u64
    );
    assert_eq!(replay.usage().admitted_input_bytes, 0);
    assert_eq!(replay.usage().used(R::Registrations), 51);
    assert_eq!(replay.usage().used(R::DistinctIdentities), 50);
    assert_eq!(replay.usage().used(R::PayloadTuples), 96);
    let old = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let result = (|| -> Result<(), ConcreteWitnessError> {
        for (index, proof) in fixture.iter().enumerate() {
            let baseline = old.usage().used(R::Records);
            old.charge(R::Records, 31)?;
            old_consistency(
                proof.header.identity,
                &proof.records,
                &fixture[..index],
                &old,
            )?;
            binary::encode_transcript(proof.header, &proof.records, &old, baseline, proof.account)?;
        }
        Ok(())
    })();
    assert_eq!(
        result,
        Err(ConcreteWitnessError::LimitExceeded {
            resource: R::FrameInspections,
            limit: 300000,
            required: 300001
        })
    );
    assert_eq!(old.usage().used(R::FrameInspections), 300000);
    assert_eq!(old.charge(R::Operations, 1), result);
    assert_eq!(old.usage().used(R::Operations), 0);
    // RCB1 raw serialization is unchanged as well. Pair data here is deliberately
    // not a valid authority input, and this test never accepts native totals.
    let pairs = std::array::from_fn(|_| vec![1]);
    let descriptors = std::array::from_fn(|_| vec![2]);
    let expected: Vec<_> = fixture.iter().map(|p| p.bytes.clone()).collect();
    let got = encode(&pairs, &descriptors, &capture_outputs, &capture).unwrap();
    let expected = encode(
        &pairs,
        &descriptors,
        &expected,
        &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
    )
    .unwrap();
    assert_eq!(got, expected);
    assert_eq!(&got[..4], b"RCB1");
}

#[test]
fn batch_consistency_matches_finite_oracle_for_all_tagged_outcomes_and_identity_keys() {
    let id = identity(live::HEAD);
    let mut original = frames(id, 4);
    let h = header(id, 4);
    let cases = [
        (
            0,
            2,
            binary::Outcome::Node(&[1][..]),
            binary::Outcome::Node(&[2][..]),
        ),
        (0, 2, binary::Outcome::End, binary::Outcome::Node(&[][..])),
        (0, 2, binary::Outcome::Error(0), binary::Outcome::Error(1)),
        (
            1,
            3,
            binary::Outcome::Row(&[0; 40], &[1]),
            binary::Outcome::Row(&[0; 40], &[2]),
        ),
        (
            1,
            3,
            binary::Outcome::Row(&[0; 40], &[1]),
            binary::Outcome::Row(&[1; 40], &[1]),
        ),
        (1, 3, binary::Outcome::End, binary::Outcome::Error(3)),
    ];
    for (operation, column, old, new) in cases {
        original[3].operation = operation;
        original[3].column = column;
        original[3].outcome = old;
        let prior = [encoded(h, &original, false)];
        let mut current = frames(id, 4);
        current[3].operation = operation;
        current[3].column = column;
        current[3].outcome = old;
        for expected in [Ok(()), Err(ConcreteWitnessError::Conflict)] {
            if expected.is_err() {
                current[3].outcome = new;
            }
            let oracle = old_consistency(
                id,
                &current,
                &prior,
                &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default()),
            );
            let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
            let actual = consistent_frames(id, &current, &prior, &b);
            assert_eq!(actual, oracle);
            assert_eq!(actual, expected);
            if let Err(error) = actual {
                assert_eq!(b.error(), Some(error));
            }
        }
    }
    let prior = [encoded(h, &frames(id, 4), false)];
    for change in 0..5 {
        let mut current = frames(id, 4);
        let expected_id = match change {
            0 => identity(live::PARENT),
            1 => ConcreteStateIdentity {
                state_root: [43; 32],
                ..id
            },
            _ => id,
        };
        if change < 2 {
            for f in &mut current {
                f.identity = expected_id;
                f.period = expected_id.period.as_u64();
                f.outcome = binary::Outcome::Error(0);
            }
        }
        if change == 2 {
            current[3].column = 4;
            current[3].outcome = binary::Outcome::Error(0);
        }
        if change == 3 {
            current[3].request = [99; 32];
            current[3].outcome = binary::Outcome::Error(0);
        }
        if change == 4 {
            current[3].operation = 1;
            current[3].column = 3;
            current[3].outcome = binary::Outcome::Error(0);
        }
        assert_eq!(
            consistent_frames(
                expected_id,
                &current,
                &prior,
                &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default())
            ),
            old_consistency(
                expected_id,
                &current,
                &prior,
                &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default())
            )
        );
    }
    for mixed_period in [false, true] {
        let mut current = frames(id, 4);
        if mixed_period {
            current[3].period -= 1;
        } else {
            current[3].identity = identity(live::PARENT);
        }
        let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let actual = consistent_frames(id, &current, &prior, &b);
        assert_eq!(
            actual,
            old_consistency(
                id,
                &current,
                &prior,
                &ConcreteWitnessBudget::new(ConcreteWitnessLimits::default())
            )
        );
        assert!(actual.is_err());
        assert_eq!(b.error(), actual.err());
    }
}

#[test]
fn caps_stop_before_header_access_comparison_or_replay_metadata_allocation() {
    struct NeverRead;
    impl AsRef<[u8]> for NeverRead {
        fn as_ref(&self) -> &[u8] {
            panic!("header accessed after stop")
        }
    }
    let id = identity(live::HEAD);
    let current = frames(id, 1);
    for limit in [0, 1] {
        let b = ConcreteWitnessBudget::new(
            ConcreteWitnessLimits::default()
                .lowered(R::FrameInspections, limit)
                .unwrap(),
        );
        let e = consistent_frames(id, &current, &[NeverRead], &b).unwrap_err();
        assert!(matches!(
            e,
            ConcreteWitnessError::LimitExceeded {
                resource: R::FrameInspections,
                ..
            }
        ));
        let before = b.usage();
        assert_eq!(b.charge(R::Operations, 1), Err(e));
        assert_eq!(b.usage(), before);
        assert_eq!(consistent_frames(id, &[], &[NeverRead], &b), Err(e));
    }
    let prior = [encoded(header(id, 1), &current, false)];
    let mut conflict = frames(id, 1);
    conflict[0].outcome = binary::Outcome::Error(0);
    for (limit, expected) in [
        (
            3,
            ConcreteWitnessError::LimitExceeded {
                resource: R::FrameInspections,
                limit: 3,
                required: 4,
            },
        ),
        (4, ConcreteWitnessError::Conflict),
    ] {
        let b = ConcreteWitnessBudget::new(
            ConcreteWitnessLimits::default()
                .lowered(R::FrameInspections, limit)
                .unwrap(),
        );
        assert_eq!(consistent_frames(id, &conflict, &prior, &b), Err(expected));
        assert_eq!(b.error(), Some(expected));
    }
    let bytes = &prior[0];
    let b = ConcreteWitnessBudget::new(
        ConcreteWitnessLimits::default()
            .lowered(R::CanonicalBytes, 0)
            .unwrap(),
    );
    let admitted = binary::admit_transcript(bytes, &b, None, false).unwrap();
    let before = b.usage();
    let error = consistent_transcript(bytes, admitted, &[] as &[Vec<u8>], &b).unwrap_err();
    assert!(matches!(
        error,
        ConcreteWitnessError::LimitExceeded {
            resource: R::CanonicalBytes,
            ..
        }
    ));
    assert_eq!(b.usage(), before);
    assert_eq!(b.charge(R::Operations, 1), Err(error));
}

#[test]
fn malformed_prior_and_wrapper_decode_overflow_schema_and_count_failures_are_sticky() {
    let id = identity(live::HEAD);
    let current = frames(id, 1);
    let h = header(id, 1);
    let valid = encoded(h, &current, false);
    for tamper in 0..3 {
        let mut prior = valid.clone();
        match tamper {
            0 => {
                prior.truncate(binary::HEADER_BYTES);
                prior.extend_from_slice(&u32::MAX.to_le_bytes());
            }
            1 => {
                prior[binary::HEADER_BYTES + 4 + 8..binary::HEADER_BYTES + 4 + 16]
                    .copy_from_slice(&live::PARENT.to_le_bytes());
                prior[binary::HEADER_BYTES + 4 + 82..binary::HEADER_BYTES + 4 + 90]
                    .copy_from_slice(&live::PARENT.to_le_bytes());
            }
            _ => {
                prior[100..104].copy_from_slice(&2u32.to_le_bytes());
            }
        }
        let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
        let e = consistent_frames(id, &current, &[prior], &b).unwrap_err();
        assert!(matches!(
            e,
            ConcreteWitnessError::Schema | ConcreteWitnessError::Identity
        ));
        assert_eq!(b.error(), Some(e));
        assert_eq!(b.charge(R::Operations, 1), Err(e));
    }
    let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    assert!(matches!(
        inspected_frame(&valid, usize::MAX, &b),
        Err(ConcreteWitnessError::Overflow)
    ));
    assert_eq!(b.error(), Some(ConcreteWitnessError::Overflow));
    let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let admitted = binary::admit_transcript(&valid, &b, None, false).unwrap();
    let mut malformed = valid.clone();
    malformed.truncate(binary::HEADER_BYTES + 95);
    let error = consistent_transcript(&malformed, admitted, &[] as &[Vec<u8>], &b).unwrap_err();
    assert_eq!(error, ConcreteWitnessError::Schema);
    assert_eq!(b.error(), Some(error));
    assert_eq!(b.usage().used(R::RawCopyBytes), 0);
    let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let error = consistent_transcript(&valid, h, &[] as &[Vec<u8>], &b).unwrap_err();
    assert_eq!(error, ConcreteWitnessError::Schema);
    assert_eq!(b.usage().used(R::CanonicalBytes), 0);
    assert_eq!(b.error(), Some(error));
}

#[test]
fn ordered_consumer_still_rejects_reordering_and_preserves_unused_records() {
    let id = identity(live::HEAD);
    let current = frames(id, 2);
    let h = header(id, 2);
    let bytes = encoded(h, &current, false);
    let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let admitted = binary::admit_transcript(&bytes, &b, None, false).unwrap();
    consistent_transcript(&bytes, admitted, &[] as &[Vec<u8>], &b).unwrap();
    let witness = ConcreteStorageWitness {
        bytes: &bytes,
        header: admitted,
    };
    let store = ReplayStore {
        witness: &witness,
        budget: &b,
        offset: Cell::new(binary::HEADER_BYTES),
        ordinal: Cell::new(0),
        last_read: None,
    };
    assert!(store.node("2", current[1].request).is_err());
    assert_eq!(b.error(), Some(ConcreteWitnessError::Request));
    let b = ConcreteWitnessBudget::new(ConcreteWitnessLimits::default());
    let admitted = binary::admit_transcript(&bytes, &b, None, false).unwrap();
    consistent_transcript(&bytes, admitted, &[] as &[Vec<u8>], &b).unwrap();
    let witness = ConcreteStorageWitness {
        bytes: &bytes,
        header: admitted,
    };
    let store = ReplayStore {
        witness: &witness,
        budget: &b,
        offset: Cell::new(binary::HEADER_BYTES),
        ordinal: Cell::new(0),
        last_read: None,
    };
    store.node("2", current[0].request).unwrap();
    assert_ne!(store.offset.get(), bytes.len());
    assert_ne!(store.ordinal.get(), u64::from(h.records));
}
