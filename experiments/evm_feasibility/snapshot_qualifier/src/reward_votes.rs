//! Bounded compatibility decoding of the retained optimized certificate bundle.
//! No voter weight or expected BlockStats output enters reconstruction. The
//! caller must bind the containing PeriodData bytes and authenticate all state
//! inputs independently before validating these signed canonical votes.
use anyhow::{Result, ensure};
use ethereum_types::H256;
use rlp::{Rlp, RlpStream};

/// Bundle identity and canonical unweighted signed votes. The period, round,
/// step and block hash are reconstructed solely from the optimized source bytes.
pub struct ReconstructedVotes {
    pub certified_period: u64,
    pub round: u64,
    pub step: u64,
    pub canonical: Vec<Vec<u8>>,
}

/// Reconstructs canonical cert votes from a four/five-field PeriodData envelope.
/// Requires exact vote shape,65-byte signatures and cert step3; malformed RLP
/// fails. Voter recovery, bounded cardinality and cryptographic validation are
/// caller responsibilities; no trusted sidecar weight is manufactured.
pub fn reconstruct_cert_votes(period_data: &[u8]) -> Result<ReconstructedVotes> {
    let period = Rlp::new(period_data);
    ensure!(matches!(period.item_count()?, 4 | 5));
    let bundle = period.at(1)?;
    ensure!(bundle.item_count()? == 5);
    let block_hash: H256 = bundle.val_at(0)?;
    let certified_period: u64 = bundle.val_at(1)?;
    let round: u64 = bundle.val_at(2)?;
    let step: u64 = bundle.val_at(3)?;
    ensure!(step == 3, "previous reward bundle is not cert votes");
    let optimized = bundle.at(4)?;
    ensure!(optimized.item_count()? > 0);
    let canonical = optimized
        .iter()
        .map(|vote| {
            ensure!(vote.item_count()? == 2);
            let proof = vote.at(0)?.data()?;
            let signature = vote.at(1)?.data()?;
            ensure!(signature.len() == 65);
            let mut sortition = RlpStream::new_list(4);
            sortition.append(&certified_period);
            sortition.append(&round);
            sortition.append(&step);
            sortition.append(&proof);
            let mut canonical = RlpStream::new_list(3);
            canonical.append(&block_hash);
            canonical.append(&sortition.out().as_ref());
            canonical.append(&signature);
            Ok(canonical.out().to_vec())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ReconstructedVotes {
        certified_period,
        round,
        step,
        canonical,
    })
}
