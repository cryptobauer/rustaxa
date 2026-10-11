//! Strict borrowed binary admission; no payload or record vector is allocated.
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
    if bytes.len() as u64 > budget.limits.limit(R::EncodedBytes) {
        return Err(budget.stop(ConcreteWitnessError::LimitExceeded {
            resource: R::EncodedBytes,
            limit: budget.limits.limit(R::EncodedBytes),
            required: bytes.len() as u64,
        }));
    }
    budget.state.borrow_mut().usage.admitted_input_bytes = bytes.len() as u64;
    let mut c = Cursor { bytes, offset: 0 };
    if c.array::<4>()? != *b"RCW1" || c.u16()? != 1 || c.u16()? != 0 {
        return Err(ConcreteWitnessError::Schema);
    }
    let identity = c.identity()?;
    let address = c.array()?;
    let key = ConcreteStorageKey(c.array()?);
    let records = c.u32()?;
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
