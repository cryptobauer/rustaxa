//! Finite raw family envelope. It carries no captured verdict or root authority.
use super::*;
use std::mem::size_of;
const HEADER: [u8; 16] = [b'R', b'C', b'B', b'1', 1, 0, 0, 0, 4, 0, 2, 0, 51, 0, 96, 0];
const PROOFS: usize = 2 + 49 + 96;

/// Allocate exact metadata capacity for the two constructor groups and payloads.
pub(super) fn proof_vector(
    budget: &ConcreteWitnessBudget,
) -> Result<Vec<Vec<u8>>, ConcreteWitnessError> {
    budget.charge(R::CanonicalBytes, (PROOFS * size_of::<Vec<u8>>()) as u64)?;
    let mut out = Vec::new();
    out.try_reserve_exact(PROOFS)
        .map_err(|_| budget.stop(ConcreteWitnessError::Allocation))?;
    Ok(out)
}
/// Charge the complete final capacity and every copied constituent before reserve.
pub(super) fn encode(
    pair: &[Vec<u8>; 4],
    descriptors: &[Vec<u8>; 2],
    proofs: &[Vec<u8>],
    budget: &ConcreteWitnessBudget,
) -> Result<Vec<u8>, ConcreteWitnessError> {
    if proofs.len() != PROOFS {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    let mut size = HEADER.len();
    let mut raw = 0u64;
    for bytes in pair.iter().chain(descriptors).chain(proofs) {
        u32::try_from(bytes.len()).map_err(|_| budget.stop(ConcreteWitnessError::Overflow))?;
        size = size
            .checked_add(4)
            .and_then(|n| n.checked_add(bytes.len()))
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
        raw = raw
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
    }
    budget.charge(R::EncodedBytes, size as u64)?;
    budget.charge(R::RawCopyBytes, raw)?;
    let mut out = Vec::new();
    out.try_reserve_exact(size)
        .map_err(|_| budget.stop(ConcreteWitnessError::Allocation))?;
    out.extend_from_slice(&HEADER);
    for bytes in pair.iter().chain(descriptors).chain(proofs) {
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
    }
    debug_assert_eq!(out.len(), size);
    Ok(out)
}
pub(super) struct Envelope<'a> {
    pub pair: [&'a [u8]; 4],
    pub descriptors: [&'a [u8]; 2],
    pub proofs: [&'a [u8]; PROOFS],
}
/// Borrow a fixed-count envelope after complete input admission. Reject trailing
/// bytes and admit every raw row width before any header/descriptor decoding.
pub(super) fn admit<'a>(
    bytes: &'a [u8],
    budget: &ConcreteWitnessBudget,
) -> Result<Envelope<'a>, ConcreteWitnessError> {
    if bytes.get(..HEADER.len()) != Some(HEADER.as_slice()) {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    let mut offset = HEADER.len();
    let mut take = || -> Result<&'a [u8], ConcreteWitnessError> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
        let length = u32::from_le_bytes(
            bytes
                .get(offset..end)
                .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?
                .try_into()
                .map_err(|_| budget.stop(ConcreteWitnessError::Schema))?,
        ) as usize;
        let next = end
            .checked_add(length)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
        let value = bytes
            .get(end..next)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?;
        offset = next;
        Ok(value)
    };
    let mut pair = [&[][..]; 4];
    for value in &mut pair {
        *value = take()?;
        budget.charge(R::RowBytes, value.len() as u64)?;
    }
    let mut descriptors = [&[][..]; 2];
    for value in &mut descriptors {
        *value = take()?;
        budget.charge(R::RowBytes, value.len() as u64)?;
    }
    let mut proofs = [&[][..]; PROOFS];
    for value in &mut proofs {
        *value = take()?;
    }
    if offset != bytes.len() {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    Ok(Envelope {
        pair,
        descriptors,
        proofs,
    })
}

/// Compare repeated reads across same-identity constituents without an owning
/// map. Every header/frame inspection uses the family budget. Other identities
/// may legitimately have different selected history and are skipped.
pub(super) fn consistent_frame<T: AsRef<[u8]>>(
    frame: &binary::Frame<'_>,
    prior: &[T],
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    for bytes in prior {
        budget.charge(R::FrameInspections, 1)?;
        let bytes = bytes.as_ref();
        let period = bytes
            .get(8..16)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?;
        let root = bytes
            .get(16..48)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?;
        if period != frame.identity.period.as_u64().to_le_bytes()
            || root != frame.identity.state_root
        {
            continue;
        }
        let mut offset = binary::HEADER_BYTES;
        while offset < bytes.len() {
            let (old, next) = binary::frame_at(bytes, offset, budget)?;
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
    Ok(())
}
pub(super) fn consistent_transcript<T: AsRef<[u8]>>(
    bytes: &[u8],
    prior: &[T],
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    let mut offset = binary::HEADER_BYTES;
    while offset < bytes.len() {
        let (frame, next) = binary::frame_at(bytes, offset, budget)?;
        consistent_frame(&frame, prior, budget)?;
        offset = next;
    }
    Ok(())
}
