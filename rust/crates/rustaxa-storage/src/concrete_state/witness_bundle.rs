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

/// Inspect one frame and make every bare decoder failure sticky before return.
fn inspected_frame<'a>(
    bytes: &'a [u8],
    offset: usize,
    budget: &ConcreteWitnessBudget,
) -> Result<(binary::Frame<'a>, usize), ConcreteWitnessError> {
    binary::frame_at(bytes, offset, budget).map_err(|error| budget.stop(error))
}

/// Compare all current frames with prior same-identity transcripts. Current
/// identity inspections, prior header/decode work and each pair comparison are
/// charged before access. Prior proofs must already be encoded/admitted against
/// their expected identity. No payload, matching-proof list or index is copied.
pub(super) fn consistent_frames<T: AsRef<[u8]>>(
    identity: ConcreteStateIdentity,
    frames: &[binary::Frame<'_>],
    prior: &[T],
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    // Empty transcripts still propagate an existing stop before success.
    budget.charge(R::FrameInspections, 0)?;
    for frame in frames {
        budget.charge(R::FrameInspections, 1)?;
        if frame.identity != identity {
            return Err(budget.stop(ConcreteWitnessError::Identity));
        }
        if frame.period != identity.period.as_u64() {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
    }
    if frames.is_empty() {
        return Ok(());
    }
    for bytes in prior {
        budget.charge(R::FrameInspections, 1)?;
        let bytes = bytes.as_ref();
        if bytes.len() < binary::HEADER_BYTES {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        let period = u64::from_le_bytes(
            bytes
                .get(8..16)
                .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?
                .try_into()
                .map_err(|_| budget.stop(ConcreteWitnessError::Schema))?,
        );
        let root = bytes
            .get(16..48)
            .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?
            .try_into()
            .map_err(|_| budget.stop(ConcreteWitnessError::Schema))?;
        let prior_identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(period),
            state_root: root,
        };
        if prior_identity != identity {
            continue;
        }
        let records = u32::from_le_bytes(
            bytes
                .get(100..104)
                .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?
                .try_into()
                .map_err(|_| budget.stop(ConcreteWitnessError::Schema))?,
        );
        let mut offset = binary::HEADER_BYTES;
        let mut ordinal = 0u64;
        while offset < bytes.len() {
            let (old, next) = inspected_frame(bytes, offset, budget)?;
            if old.identity != prior_identity || old.period != period || old.ordinal != ordinal {
                return Err(budget.stop(ConcreteWitnessError::Identity));
            }
            if ordinal >= u64::from(records) {
                return Err(budget.stop(ConcreteWitnessError::Schema));
            }
            for frame in frames {
                budget.charge(R::FrameInspections, 1)?;
                if old.operation == frame.operation
                    && old.column == frame.column
                    && old.request == frame.request
                    && old.period == frame.period
                    && old.outcome != frame.outcome
                {
                    return Err(budget.stop(ConcreteWitnessError::Conflict));
                }
            }
            offset = next;
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
        }
        if offset != bytes.len() || ordinal != u64::from(records) {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
    }
    Ok(())
}

/// Collect only metadata for one already admitted constituent, then compare it
/// with prior proofs. Allocation uses the admitted count and original budget;
/// input/Records are not re-admitted, payloads are borrowed, and metadata is
/// dropped before ordered proof consumption. Every failure remains sticky.
pub(super) fn consistent_transcript<T: AsRef<[u8]>>(
    bytes: &[u8],
    header: binary::Header,
    prior: &[T],
    budget: &ConcreteWitnessBudget,
) -> Result<(), ConcreteWitnessError> {
    budget.charge(R::FrameInspections, 0)?;
    let count =
        usize::try_from(header.records).map_err(|_| budget.stop(ConcreteWitnessError::Overflow))?;
    let raw = bytes
        .len()
        .checked_sub(binary::HEADER_BYTES)
        .ok_or_else(|| budget.stop(ConcreteWitnessError::Schema))?;
    // An End frame has the smallest size: four length bytes and91 body bytes.
    if count > raw / 95 || u64::from(header.records) > budget.usage().used(R::Records) {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    let allocation = count
        .checked_mul(size_of::<binary::Frame<'_>>())
        .ok_or_else(|| budget.stop(ConcreteWitnessError::Overflow))?;
    budget.charge(R::CanonicalBytes, allocation as u64)?;
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(count)
        .map_err(|_| budget.stop(ConcreteWitnessError::Allocation))?;
    let mut offset = binary::HEADER_BYTES;
    for ordinal in 0..count {
        let (frame, next) = inspected_frame(bytes, offset, budget)?;
        if frame.identity != header.identity || frame.ordinal != ordinal as u64 {
            return Err(budget.stop(ConcreteWitnessError::Identity));
        }
        if frame.period != header.identity.period.as_u64() {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        // The exact loop count stays within the fallibly reserved capacity.
        frames.push(frame);
        offset = next;
    }
    if offset != bytes.len() || frames.len() != count {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    consistent_frames(header.identity, &frames, prior, budget)
}

#[cfg(test)]
#[path = "witness_frame_cost_tests.rs"]
mod tests;
