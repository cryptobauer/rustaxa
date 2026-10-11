//! Exact-sized canonical buffers for the diagnostic physical-node verifier.
use super::codec::corrupt;
use rustaxa_types::concrete_state::ConcreteReadError;

pub(crate) enum Part<'a> {
    Raw(&'a [u8]),
    Data(&'a [u8]),
}
fn length_prefix(payload: usize) -> Result<usize, ConcreteReadError> {
    if payload < 56 {
        Ok(1)
    } else {
        Ok(1 + (usize::BITS - payload.leading_zeros()).div_ceil(8) as usize)
    }
}
fn data_len(bytes: &[u8]) -> Result<usize, ConcreteReadError> {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        Ok(1)
    } else {
        bytes
            .len()
            .checked_add(length_prefix(bytes.len())?)
            .ok_or_else(|| corrupt("canonical length overflow"))
    }
}
fn prefix(out: &mut Vec<u8>, length: usize, short: u8, long: u8) {
    if length < 56 {
        out.push(short + length as u8);
    } else {
        let raw = length.to_be_bytes();
        let first = raw.iter().position(|b| *b != 0).unwrap();
        out.push(long + (raw.len() - first) as u8);
        out.extend_from_slice(&raw[first..]);
    }
}
fn append_data(out: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() != 1 || bytes[0] >= 0x80 {
        prefix(out, bytes.len(), 0x80, 0xb7);
    }
    out.extend_from_slice(bytes);
}
fn allocate(
    length: usize,
    charge: impl Fn(usize) -> Result<(), ConcreteReadError>,
    allocation_error: impl Fn() -> ConcreteReadError,
) -> Result<Vec<u8>, ConcreteReadError> {
    charge(length)?;
    let mut out = Vec::new();
    out.try_reserve_exact(length)
        .map_err(|_| allocation_error())?;
    Ok(out)
}
pub(crate) fn data(
    bytes: &[u8],
    charge: impl Fn(usize) -> Result<(), ConcreteReadError>,
    allocation_error: impl Fn() -> ConcreteReadError,
) -> Result<Vec<u8>, ConcreteReadError> {
    let length = data_len(bytes)?;
    let mut out = allocate(length, charge, allocation_error)?;
    append_data(&mut out, bytes);
    debug_assert_eq!(out.len(), length);
    Ok(out)
}
pub(crate) fn list(
    parts: &[Part<'_>],
    charge: impl Fn(usize) -> Result<(), ConcreteReadError>,
    allocation_error: impl Fn() -> ConcreteReadError,
) -> Result<Vec<u8>, ConcreteReadError> {
    let payload = parts.iter().try_fold(0usize, |n, p| {
        n.checked_add(match p {
            Part::Raw(b) => b.len(),
            Part::Data(b) => data_len(b)?,
        })
        .ok_or_else(|| corrupt("canonical length overflow"))
    })?;
    let length = payload
        .checked_add(length_prefix(payload)?)
        .ok_or_else(|| corrupt("canonical length overflow"))?;
    let mut out = allocate(length, charge, allocation_error)?;
    prefix(&mut out, payload, 0xc0, 0xf7);
    for part in parts {
        match part {
            Part::Raw(b) => out.extend_from_slice(b),
            Part::Data(b) => append_data(&mut out, b),
        }
    }
    debug_assert_eq!(out.len(), length);
    Ok(out)
}
