//! Strict borrowed binary admission and bounded transcript encoding.
//! Admission allocates no payload or record vector.
use super::*;
pub(super) const HEADER_BYTES: usize = 104;
#[derive(Clone, Copy)]
pub(super) struct Header {
    pub identity: ConcreteStateIdentity,
    pub address: [u8; 20],
    pub key: ConcreteStorageKey,
    pub records: u32,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Outcome<'a> {
    End,
    Node(&'a [u8]),
    Row(&'a [u8], &'a [u8]),
    Error(u8),
}
pub(super) struct Frame<'a> {
    pub ordinal: u64,
    pub identity: ConcreteStateIdentity,
    pub operation: u8,
    pub column: u8,
    pub request: [u8; 32],
    pub period: u64,
    pub outcome: Outcome<'a>,
}
struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ConcreteWitnessError> {
        let end = self
            .offset
            .checked_add(n)
            .ok_or(ConcreteWitnessError::Overflow)?;
        let b = self
            .bytes
            .get(self.offset..end)
            .ok_or(ConcreteWitnessError::Schema)?;
        self.offset = end;
        Ok(b)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], ConcreteWitnessError> {
        self.take(N)?
            .try_into()
            .map_err(|_| ConcreteWitnessError::Schema)
    }
    fn u8(&mut self) -> Result<u8, ConcreteWitnessError> {
        Ok(self.array::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, ConcreteWitnessError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, ConcreteWitnessError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, ConcreteWitnessError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
    fn identity(&mut self) -> Result<ConcreteStateIdentity, ConcreteWitnessError> {
        Ok(ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(self.u64()?),
            state_root: self.array()?,
        })
    }
    fn payload(
        &mut self,
        budget: &ConcreteWitnessBudget,
        resource: R,
    ) -> Result<&'a [u8], ConcreteWitnessError> {
        let n = self.u32()?;
        budget.charge(resource, u64::from(n))?;
        self.take(n as usize)
    }
}
pub(super) fn frame_at<'a>(
    bytes: &'a [u8],
    offset: usize,
    budget: &ConcreteWitnessBudget,
) -> Result<(Frame<'a>, usize), ConcreteWitnessError> {
    budget.charge(R::FrameInspections, 1)?;
    let mut c = Cursor { bytes, offset };
    let n = c.u32()? as usize;
    let mut body = Cursor {
        bytes: c.take(n)?,
        offset: 0,
    };
    let ordinal = body.u64()?;
    let identity = body.identity()?;
    let operation = body.u8()?;
    let column = body.u8()?;
    let request = body.array()?;
    let period = body.u64()?;
    let outcome = body.u8()?;
    if period != identity.period.as_u64() || !matches!((operation, column), (0, 2 | 4) | (1, 3 | 5))
    {
        return Err(ConcreteWitnessError::Schema);
    }
    let outcome = match (operation, outcome) {
        (_, 0) => Outcome::End,
        (0, 1) => Outcome::Node(body.payload(budget, R::RowBytes)?),
        (1, 1) => Outcome::Row(
            body.payload(budget, R::KeyBytes)?,
            body.payload(budget, R::RowBytes)?,
        ),
        (_, 2) => {
            let code = body.u8()?;
            if code > 3 {
                return Err(ConcreteWitnessError::Schema);
            }
            Outcome::Error(code)
        }
        _ => return Err(ConcreteWitnessError::Schema),
    };
    if body.offset != body.bytes.len() {
        return Err(ConcreteWitnessError::Schema);
    }
    Ok((
        Frame {
            ordinal,
            identity,
            operation,
            column,
            request,
            period,
            outcome,
        },
        c.offset,
    ))
}
pub(super) fn admit(
    bytes: &[u8],
    budget: &ConcreteWitnessBudget,
    expected: Option<(ConcreteStateIdentity, [u8; 20], ConcreteStorageKey)>,
) -> Result<Header, ConcreteWitnessError> {
    budget.admit_input(bytes.len())?;
    admit_transcript(bytes, budget, expected, false)
}

/// Admit one family constituent after the complete envelope has been admitted.
/// All record and inspection charges remain on the original family budget.
pub(super) fn admit_transcript(
    bytes: &[u8],
    budget: &ConcreteWitnessBudget,
    expected: Option<(ConcreteStateIdentity, [u8; 20], ConcreteStorageKey)>,
    account: bool,
) -> Result<Header, ConcreteWitnessError> {
    let mut c = Cursor { bytes, offset: 0 };
    if c.array::<4>()? != if account { *b"RCA1" } else { *b"RCW1" }
        || c.u16()? != 1
        || c.u16()? != 0
    {
        return Err(ConcreteWitnessError::Schema);
    }
    let identity = c.identity()?;
    let address = c.array()?;
    let key = ConcreteStorageKey(c.array()?);
    let records = c.u32()?;
    if account && (address != [0; 20] || key != ConcreteStorageKey([0; 32])) {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    budget.charge(R::Records, u64::from(records))?;
    let header = Header {
        identity,
        address,
        key,
        records,
    };
    if expected.is_some_and(|tuple| tuple != (identity, address, key)) {
        return Err(budget.stop(ConcreteWitnessError::Identity));
    }
    let mut offset = HEADER_BYTES;
    for ordinal in 0..u64::from(records) {
        let (frame, next) = frame_at(bytes, offset, budget)?;
        if account && !matches!((frame.operation, frame.column), (0, 2) | (1, 3)) {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        if frame.ordinal != ordinal || frame.identity != identity {
            return Err(ConcreteWitnessError::Identity);
        }
        let mut prior = HEADER_BYTES;
        while prior < offset {
            let (old, next_old) = frame_at(bytes, prior, budget)?;
            if old.operation == frame.operation
                && old.column == frame.column
                && old.request == frame.request
                && old.period == frame.period
                && old.outcome != frame.outcome
            {
                return Err(ConcreteWitnessError::Conflict);
            }
            prior = next_old;
        }
        offset = next;
    }
    if offset != bytes.len() {
        return Err(ConcreteWitnessError::Schema);
    }
    Ok(header)
}

/// Encode borrowed records under their original capture budget.
///
/// The header and ordered frames must describe one identity, and the budget's
/// record count must equal this transcript's length. Schema and repeated-request
/// consistency checks charge the same budget before exact-sized output allocation.
/// Returns canonical witness bytes, or a schema, conflict, budget, overflow or
/// allocation error. Budget failures remain sticky; charges are never refunded.
pub(super) fn encode(
    header: Header,
    records: &[Frame<'_>],
    budget: &ConcreteWitnessBudget,
) -> Result<Vec<u8>, ConcreteWitnessError> {
    encode_transcript(header, records, budget, 0, false)
}

/// Encode a transcript against its checked cumulative record-counter delta.
/// Account transcripts bind zero address/key and contain account columns only.
pub(super) fn encode_transcript(
    header: Header,
    records: &[Frame<'_>],
    budget: &ConcreteWitnessBudget,
    record_baseline: u64,
    account: bool,
) -> Result<Vec<u8>, ConcreteWitnessError> {
    if account && (header.address != [0; 20] || header.key != ConcreteStorageKey([0; 32])) {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    if header.records as usize != records.len()
        || budget.usage().used(R::Records).checked_sub(record_baseline)
            != Some(records.len() as u64)
    {
        return Err(budget.stop(ConcreteWitnessError::Schema));
    }
    // Capture validates every frame and consistency on its original budget,
    // before the output allocation. No unreported fresh admission budget.
    for (index, frame) in records.iter().enumerate() {
        budget.charge(R::FrameInspections, 1)?;
        if account && !matches!((frame.operation, frame.column), (0, 2) | (1, 3)) {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        if frame.ordinal != index as u64
            || frame.identity != header.identity
            || frame.period != header.identity.period.as_u64()
            || !matches!((frame.operation, frame.column), (0, 2 | 4) | (1, 3 | 5))
        {
            return Err(budget.stop(ConcreteWitnessError::Schema));
        }
        match frame.outcome {
            Outcome::Node(b) if frame.operation == 0 => {
                budget.charge(R::RowBytes, b.len() as u64)?
            }
            Outcome::Row(k, v) if frame.operation == 1 => {
                budget.charge(R::KeyBytes, k.len() as u64)?;
                budget.charge(R::RowBytes, v.len() as u64)?;
            }
            Outcome::End => (),
            Outcome::Error(code) if code <= 3 => (),
            _ => return Err(budget.stop(ConcreteWitnessError::Schema)),
        }
        for prior in &records[..index] {
            budget.charge(R::FrameInspections, 1)?;
            if prior.operation == frame.operation
                && prior.column == frame.column
                && prior.request == frame.request
                && prior.period == frame.period
                && prior.outcome != frame.outcome
            {
                return Err(budget.stop(ConcreteWitnessError::Conflict));
            }
        }
    }
    let mut size = HEADER_BYTES;
    let mut raw = 0_u64;
    for frame in records {
        let payload = match frame.outcome {
            Outcome::End => 0,
            Outcome::Error(_) => 1,
            Outcome::Node(b) => {
                raw = raw
                    .checked_add(b.len() as u64)
                    .ok_or(ConcreteWitnessError::Overflow)?;
                4 + b.len()
            }
            Outcome::Row(k, v) => {
                raw = raw
                    .checked_add((k.len() + v.len()) as u64)
                    .ok_or(ConcreteWitnessError::Overflow)?;
                8 + k.len() + v.len()
            }
        };
        size = size
            .checked_add(4 + 91 + payload)
            .ok_or(ConcreteWitnessError::Overflow)?;
    }
    budget.charge(R::EncodedBytes, size as u64)?;
    budget.charge(R::RawCopyBytes, raw)?;
    let mut out = Vec::new();
    out.try_reserve_exact(size)
        .map_err(|_| budget.stop(ConcreteWitnessError::Allocation))?;
    out.extend_from_slice(if account { b"RCA1" } else { b"RCW1" });
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&header.identity.period.as_u64().to_le_bytes());
    out.extend_from_slice(&header.identity.state_root);
    out.extend_from_slice(&header.address);
    out.extend_from_slice(&header.key.0);
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for frame in records {
        let payload = match frame.outcome {
            Outcome::End => 0,
            Outcome::Error(_) => 1,
            Outcome::Node(b) => 4 + b.len(),
            Outcome::Row(k, v) => 8 + k.len() + v.len(),
        };
        out.extend_from_slice(
            &u32::try_from(91 + payload)
                .map_err(|_| ConcreteWitnessError::Overflow)?
                .to_le_bytes(),
        );
        out.extend_from_slice(&frame.ordinal.to_le_bytes());
        out.extend_from_slice(&frame.identity.period.as_u64().to_le_bytes());
        out.extend_from_slice(&frame.identity.state_root);
        out.push(frame.operation);
        out.push(frame.column);
        out.extend_from_slice(&frame.request);
        out.extend_from_slice(&frame.period.to_le_bytes());
        match frame.outcome {
            Outcome::End => out.push(0),
            Outcome::Error(code) => {
                out.push(2);
                out.push(code)
            }
            Outcome::Node(b) => {
                out.push(1);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b)
            }
            Outcome::Row(k, v) => {
                out.push(1);
                out.extend_from_slice(&(k.len() as u32).to_le_bytes());
                out.extend_from_slice(k);
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v)
            }
        }
    }
    assert_eq!(out.len(), size);
    Ok(out)
}
