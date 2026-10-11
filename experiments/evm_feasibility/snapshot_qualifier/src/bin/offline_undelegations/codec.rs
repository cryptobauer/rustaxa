//! Pure retained Go undelegation codecs. Exact RLP re-encoding rejects drift.
use anyhow::{Context, Result, ensure};
use num_bigint::BigUint;
use rlp::{Rlp, RlpStream};
#[derive(Debug)]
pub(super) struct UndelegationRow {
    pub amount: BigUint,
    pub block: u64,
}
pub(super) fn decode_undelegation_v1(bytes: &[u8]) -> Result<UndelegationRow> {
    decode_undelegation_row(
        exact_rlp(bytes, "V1 undelegation")?,
        bytes,
        "V1 undelegation",
    )
}

pub(super) fn decode_undelegation_v2(bytes: &[u8]) -> Result<(UndelegationRow, u64)> {
    let outer = exact_rlp(bytes, "V2 undelegation")?;
    ensure!(
        outer.item_count()? == 2,
        "V2 undelegation must have two fields"
    );
    let embedded = outer.at(0)?;
    ensure!(
        embedded.is_list(),
        "V2 undelegation base must be an RLP list"
    );
    let row = decode_undelegation_row(embedded, outer.at(0)?.as_raw(), "V2 undelegation base")?;
    let id_item = outer.at(1)?;
    require_data(&id_item, "V2 undelegation ID")?;
    let id = id_item.as_val::<u64>().context("V2 undelegation ID")?;
    let mut base = RlpStream::new_list(2);
    append_uint(&mut base, &row.amount);
    base.append(&row.block);
    let base = base.out();
    let mut canonical = RlpStream::new_list(2);
    canonical.append_raw(&base, 1).append(&id);
    ensure!(
        canonical.out().as_ref() == bytes,
        "V2 undelegation is not canonical or completely consumed"
    );
    Ok((row, id))
}

fn decode_undelegation_row(row: Rlp<'_>, bytes: &[u8], label: &str) -> Result<UndelegationRow> {
    ensure!(row.is_list(), "{label} must be an RLP list");
    ensure!(row.item_count()? == 2, "{label} must have two fields");
    let amount = decode_uint_item(&row.at(0)?, "undelegation amount")?;
    let block_item = row.at(1)?;
    require_data(&block_item, "undelegation block")?;
    let block = block_item.as_val::<u64>().context("undelegation block")?;
    let mut canonical = RlpStream::new_list(2);
    append_uint(&mut canonical, &amount);
    canonical.append(&block);
    ensure!(
        canonical.out().as_ref() == bytes,
        "{label} is not canonical or completely consumed"
    );
    Ok(UndelegationRow { amount, block })
}

fn exact_rlp<'a>(bytes: &'a [u8], label: &str) -> Result<Rlp<'a>> {
    let row = Rlp::new(bytes);
    let payload = row.payload_info().with_context(|| format!("{label} RLP"))?;
    let total = payload
        .header_len
        .checked_add(payload.value_len)
        .with_context(|| format!("{label} RLP length overflow"))?;
    ensure!(total == bytes.len(), "{label} has trailing bytes");
    Ok(row)
}

fn decode_uint_item(item: &Rlp<'_>, label: &str) -> Result<BigUint> {
    let bytes = require_data(item, label)?;
    ensure!(
        bytes.first() != Some(&0),
        "{label} has a leading-zero unsigned integer"
    );
    Ok(BigUint::from_bytes_be(bytes))
}

fn require_data<'a>(item: &'a Rlp<'_>, label: &str) -> Result<&'a [u8]> {
    ensure!(!item.is_list(), "{label} must be RLP data");
    item.data().with_context(|| format!("{label} bytes"))
}

fn append_uint(stream: &mut RlpStream, value: &BigUint) {
    let bytes = value.to_bytes_be();
    let bytes = if bytes == [0] { Vec::new() } else { bytes };
    stream.append(&bytes.as_slice());
}

pub(super) fn uint_hex(value: &BigUint) -> String {
    let bytes = value.to_bytes_be();
    hex::encode(if bytes == [0] { &[][..] } else { &bytes })
}

pub(super) fn decode_le_u32(bytes: &[u8], label: &str) -> Result<u32> {
    Ok(u32::from_le_bytes(bytes.try_into().with_context(|| {
        format!("{label} must contain four little-endian bytes")
    })?))
}

pub(super) fn decode_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(bytes.try_into().with_context(|| {
        format!("{label} must contain eight little-endian bytes")
    })?))
}
