use super::*;
use ethereum_types::H256;
use rustaxa_consensus::period_data_queue::{PeriodDataQueuePopPlan, PeriodDataQueuePushOutcome};

#[test]
fn period_data_queue_adapters_preserve_all_boundary_fields() {
    let outcome: FfiPeriodDataQueuePushOutcome = PeriodDataQueuePushOutcome {
        accepted: true,
        clear_existing: true,
        expected_next_period: 42,
        actual_period: 43,
        current_period: 43,
        effective_size: 4,
    }
    .into();
    assert_eq!(
        (
            outcome.accepted,
            outcome.expected_next_period,
            outcome.actual_period,
            outcome.current_period,
            outcome.effective_size,
        ),
        (true, 42, 43, 43, 4)
    );

    let pop: FfiPeriodDataQueuePopPlan = PeriodDataQueuePopPlan {
        period_data_rlp: vec![0x11, 0x22],
        source_peer_id: [0x55; 64],
        entry_period: 42,
        block_hash: H256::repeat_byte(0x11),
        prev_block_hash: H256::repeat_byte(0x22),
        pivot_hash: H256::repeat_byte(0x33),
        final_chain_hash: H256::repeat_byte(0x44),
        reward_vote_hashes: vec![H256::repeat_byte(0x55)],
        pillar_vote_rlps: vec![vec![0xa1]],
        transaction_rlps: vec![vec![0xb1]],
        cert_vote_rlps: vec![vec![0xd1]],
        previous_cert_vote_rlps: vec![vec![0xc1]],
        dag_transaction_hashes: vec![H256::repeat_byte(0x66)],
        period_data_transaction_hashes: vec![H256::repeat_byte(0x77)],
        period_data_transaction_identities: vec![
            rustaxa_consensus::period_data_queue::PeriodDataQueueTransactionIdentity {
                input_index: 3,
                hash: H256::repeat_byte(0x88),
                transaction_nonce: [0x99; 32],
                sender: [0xaa; 20],
            },
        ],
        previous_cert_votes_present: true,
        previous_cert_first_vote_has_weight: false,
        pillar_votes_present: true,
        extra_data_present: true,
        extra_data_pillar_block_hash_present: false,
        use_last_block_cert_votes: true,
        current_period: 43,
        effective_size: 2,
    }
    .into();
    assert_eq!((pop.source_peer_id, pop.entry_period), ([0x55; 64], 42));
    assert_eq!(pop.period_data_rlp, vec![0x11, 0x22]);
    assert_eq!(
        (
            pop.block_hash,
            pop.prev_block_hash,
            pop.pivot_hash,
            pop.final_chain_hash
        ),
        ([0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32])
    );
    assert_eq!(pop.reward_vote_hashes[0].hash, [0x55; 32]);
    assert_eq!(pop.pillar_vote_rlps[0].vote_rlp, vec![0xa1]);
    assert_eq!(pop.transaction_rlps[0].transaction_rlp, vec![0xb1]);
    assert_eq!(pop.cert_vote_rlps[0].vote_rlp, vec![0xd1]);
    assert_eq!(pop.previous_cert_vote_rlps[0].vote_rlp, vec![0xc1]);
    assert_eq!(pop.dag_transaction_hashes[0].hash, [0x66; 32]);
    assert_eq!(pop.period_data_transaction_hashes[0].hash, [0x77; 32]);
    let pop_identity = &pop.period_data_transaction_identities[0];
    assert_eq!(
        (pop_identity.input_index, pop_identity.hash),
        (3, [0x88; 32])
    );
    assert_eq!(
        (pop_identity.transaction_nonce, pop_identity.sender),
        ([0x99; 32], [0xaa; 20])
    );
    assert_eq!(
        (
            pop.previous_cert_votes_present,
            pop.previous_cert_first_vote_has_weight,
            pop.pillar_votes_present,
            pop.extra_data_present,
            pop.extra_data_pillar_block_hash_present,
        ),
        (true, false, true, true, false)
    );
}
