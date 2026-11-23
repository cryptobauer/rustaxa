//! Pinned Go ABI compatibility for the three metadata-update arguments.
//!
//! Decode fields in declaration order, retain arbitrary string bytes and accept
//! dirty address high bytes, overlapping/unaligned tails and omitted padding.
//! Bounds errors reproduce Go's decimal arbitrary-width offset/length messages.
//! This codec has no state access; selector admission belongs to the session.

use super::*;

pub(super) fn decode(input: &[u8], owner: [u8; 20]) -> Result<DposTransaction, String> {
    let data = input
        .get(4..)
        .ok_or_else(|| "metadata selector is absent".to_owned())?;
    let word = |index: usize| {
        data.get(index..index + 32).ok_or_else(|| {
            format!(
                "abi: cannot marshal in to go type: length insufficient {} require {}",
                data.len(),
                index + 32
            )
        })
    };
    let validator = word(0)?[12..]
        .try_into()
        .expect("ABI address has twenty bytes");
    let dynamic = |index: usize| -> Result<Vec<u8>, String> {
        let offset_end = BigUint::from_bytes_be(word(index)?) + BigUint::from(32_u8);
        let output_length = BigUint::from(data.len());
        if offset_end > output_length {
            return Err(format!(
                "abi: cannot marshal in to go slice: offset {offset_end} would go over slice boundary (len={output_length})"
            ));
        }
        if offset_end.bits() > 63 {
            return Err(format!("abi offset larger than int64: {offset_end}"));
        }
        let end = usize::try_from(&offset_end).expect("offset is bounded by input length");
        let length = BigUint::from_bytes_be(&data[end - 32..end]);
        let total = &offset_end + length;
        if total.bits() > 63 {
            return Err(format!("abi length larger than int64: {total}"));
        }
        if total > output_length {
            return Err(format!(
                "abi: cannot marshal in to go type: length insufficient {output_length} require {total}"
            ));
        }
        let total = usize::try_from(&total).expect("total is bounded by input length");
        Ok(data[end..total].to_vec())
    };
    let description = dynamic(32)?;
    let endpoint = dynamic(64)?;
    Ok(DposTransaction::SetValidatorInfo {
        owner,
        validator,
        description,
        endpoint,
    })
}
