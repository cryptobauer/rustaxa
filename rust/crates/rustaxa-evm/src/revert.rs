//! Pinned Go ABI revert-reason decoding for disposable call diagnostics.
//!
//! This is presentation after an ordinary REVERT, not EVM execution or gas
//! policy. Go strings can contain arbitrary bytes, so reasons and diagnostics
//! stay byte-valued until the RPC serializer applies its string policy.

/// Borrows the reason accepted by the pinned Go `abi.UnpackRevert` function.
///
/// Only `Error(string)` is recognized. Offsets and lengths use all 256 bits;
/// unaligned offsets, offset zero, empty strings and trailing bytes are allowed
/// when their actual bounds are valid. Missing words, oversized integers and
/// truncated data return `None`. No allocation, UTF-8 validation or Solidity
/// canonical-padding requirement is introduced.
#[must_use]
pub fn revert_reason_bytes(data: &[u8]) -> Option<&[u8]> {
    if data.get(..4)? != [0x08, 0xc3, 0x79, 0xa0] {
        return None;
    }
    let body = &data[4..];
    let offset = word_index(body.get(..32)?)?;
    let start = offset.checked_add(32)?;
    let length = word_index(body.get(offset..start)?)?;
    let end = start.checked_add(length)?;
    // The reference explicitly requires offsets and total sizes to fit int64.
    if end > i64::MAX as usize {
        return None;
    }
    body.get(start..end)
}

/// Returns the exact byte diagnostic appended by Go `DryRunner.Apply` on REVERT.
///
/// A successfully decoded reason, including an empty or non-UTF-8 reason, adds
/// `": "` and its bytes to `"execution reverted"`. Invalid/unknown ABI data keeps
/// the base diagnostic. Call only for the ordinary REVERT outcome; native
/// contract failures retain their own error. Return data itself is unchanged.
#[must_use]
pub fn dry_run_revert_diagnostic(data: &[u8]) -> Vec<u8> {
    let mut result = b"execution reverted".to_vec();
    if let Some(reason) = revert_reason_bytes(data) {
        result.extend_from_slice(b": ");
        result.extend_from_slice(reason);
    }
    result
}

fn word_index(word: &[u8]) -> Option<usize> {
    word.iter().try_fold(0_usize, |value, byte| {
        value.checked_mul(256)?.checked_add(usize::from(*byte))
    })
}
