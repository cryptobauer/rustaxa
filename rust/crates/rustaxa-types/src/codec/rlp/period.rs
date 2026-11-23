//! Canonical-byte assembly of the four-field finalized-period envelope.
//!
//! The envelope preserves supplied PBFT, certificate, DAG and transaction RLP
//! bytes. Semantic validation remains with the consumers of those signed items.

/// Wraps already-encoded period parts without decoding or normalizing them.
///
/// `pbft_block` and each transaction must contain one complete canonical RLP
/// item. Nonempty bundle arguments must likewise contain one complete item;
/// an empty bundle argument encodes the legacy empty-data sentinel, which is
/// distinct from an explicitly supplied empty list. Transactions retain iterator
/// order. This codec trusts these input preconditions and grants no execution or
/// publication authority; callers must validate signed contents separately.
pub fn encode_period_data<'a>(
    pbft_block: &[u8],
    previous_certificate_bundle: &[u8],
    dag_bundle: &[u8],
    transactions: impl ExactSizeIterator<Item = &'a [u8]>,
) -> Vec<u8> {
    let mut stream = rlp::RlpStream::new_list(4);
    stream.append_raw(pbft_block, 1);
    for bundle in [previous_certificate_bundle, dag_bundle] {
        if bundle.is_empty() {
            stream.append_empty_data();
        } else {
            stream.append_raw(bundle, 1);
        }
    }
    stream.begin_list(transactions.len());
    for transaction in transactions {
        stream.append_raw(transaction, 1);
    }
    stream.out().to_vec()
}

#[cfg(test)]
mod tests {
    use super::encode_period_data;

    #[test]
    fn preserves_empty_data_sentinels_and_explicit_empty_lists() {
        assert_eq!(
            encode_period_data(&[0xc0], &[], &[], std::iter::empty()),
            [0xc4, 0xc0, 0x80, 0x80, 0xc0]
        );
        assert_eq!(
            encode_period_data(&[0xc0], &[0xc0], &[0xc0], std::iter::empty()),
            [0xc4, 0xc0, 0xc0, 0xc0, 0xc0]
        );
    }

    #[test]
    fn retains_raw_bundle_bytes_and_transaction_order() {
        let transactions = [&[0xc1, 0x04][..], &[0xc1, 0x03][..]];
        assert_eq!(
            encode_period_data(
                &[0xc1, 0x01],
                &[0xc2, 0x81, 0x80],
                &[0xc1, 0x02],
                transactions.into_iter()
            ),
            [
                0xcc, 0xc1, 0x01, 0xc2, 0x81, 0x80, 0xc1, 0x02, 0xc4, 0xc1, 0x04, 0xc1, 0x03
            ]
        );
    }
}
