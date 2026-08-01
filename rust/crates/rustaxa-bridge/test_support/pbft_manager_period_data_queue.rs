use super::*;
use crate::ffi::rustaxa_ffi::PbftSyncTransactionHash;
use ethereum_types::H256;
use rustaxa_consensus::period_data_queue::{PeriodDataQueuePopPlan, PeriodDataQueuePushOutcome};

#[test]
fn period_data_queue_adapters_preserve_all_boundary_fields() {
    let request: PeriodDataQueuePushRequest = PeriodDataQueuePushFfiInput {
        entry_id: 41,
        period: 42,
        block_hash: [0x11; 32],
        prev_block_hash: [0x22; 32],
        pivot_hash: [0x33; 32],
        final_chain_hash: [0x44; 32],
        reward_vote_hashes: vec![PbftSyncTransactionHash { hash: [0x55; 32] }],
        pillar_vote_rlps: vec![FfiPeriodDataQueuePillarVotePayload {
            vote_rlp: vec![0xa1],
        }],
        transaction_rlps: vec![FfiPeriodDataQueueTransactionPayload {
            transaction_rlp: vec![0xb1],
        }],
        previous_cert_vote_rlps: vec![FfiPeriodDataQueuePbftVotePayload {
            vote_rlp: vec![0xc1],
        }],
        dag_transaction_hashes: vec![PbftSyncTransactionHash { hash: [0x66; 32] }],
        period_data_transaction_hashes: vec![PbftSyncTransactionHash { hash: [0x77; 32] }],
        period_data_transaction_identities: vec![FfiPeriodDataQueueTransactionIdentity {
            input_index: 3,
            hash: [0x88; 32],
            transaction_nonce: [0x99; 32],
            sender: [0xaa; 20],
        }],
        previous_cert_votes_present: true,
        previous_cert_first_vote_has_weight: false,
        pillar_votes_present: true,
        extra_data_present: true,
        extra_data_pillar_block_hash_present: false,
        max_pbft_size: 40,
        current_block_cert_vote_rlps: vec![FfiPeriodDataQueuePbftVotePayload {
            vote_rlp: vec![0xd1],
        }],
    }
    .into();

    assert_eq!((request.entry.entry_id, request.entry.period), (41, 42));
    assert_eq!(
        (
            request.entry.block_hash,
            request.entry.prev_block_hash,
            request.entry.pivot_hash,
            request.entry.final_chain_hash,
        ),
        (
            H256::repeat_byte(0x11),
            H256::repeat_byte(0x22),
            H256::repeat_byte(0x33),
            H256::repeat_byte(0x44),
        )
    );
    assert_eq!(
        request.entry.reward_vote_hashes,
        vec![H256::repeat_byte(0x55)]
    );
    assert_eq!(request.entry.pillar_vote_rlps, vec![vec![0xa1]]);
    assert_eq!(request.entry.transaction_rlps, vec![vec![0xb1]]);
    assert_eq!(request.entry.previous_cert_vote_rlps, vec![vec![0xc1]]);
    assert_eq!(
        request.entry.dag_transaction_hashes,
        vec![H256::repeat_byte(0x66)]
    );
    assert_eq!(
        request.entry.period_data_transaction_hashes,
        vec![H256::repeat_byte(0x77)]
    );
    let identity = &request.entry.period_data_transaction_identities[0];
    assert_eq!(
        (identity.input_index, identity.hash),
        (3, H256::repeat_byte(0x88))
    );
    assert_eq!(
        (identity.transaction_nonce, identity.sender),
        ([0x99; 32], [0xaa; 20])
    );
    assert_eq!(
        (
            request.entry.previous_cert_votes_present,
            request.entry.previous_cert_first_vote_has_weight,
            request.entry.pillar_votes_present,
            request.entry.extra_data_present,
            request.entry.extra_data_pillar_block_hash_present,
        ),
        (true, false, true, true, false)
    );
    assert_eq!(request.max_pbft_size, 40);
    assert_eq!(request.current_block_cert_vote_rlps, vec![vec![0xd1]]);

    let entry_ffi: FfiPeriodDataQueueEntryRef = request.entry.clone().into();
    assert_eq!((entry_ffi.entry_id, entry_ffi.period), (41, 42));
    assert_eq!(
        (
            entry_ffi.block_hash,
            entry_ffi.prev_block_hash,
            entry_ffi.pivot_hash,
            entry_ffi.final_chain_hash,
        ),
        ([0x11; 32], [0x22; 32], [0x33; 32], [0x44; 32])
    );
    assert_eq!(entry_ffi.reward_vote_hashes[0].hash, [0x55; 32]);
    assert_eq!(entry_ffi.pillar_vote_rlps[0].vote_rlp, vec![0xa1]);
    assert_eq!(entry_ffi.transaction_rlps[0].transaction_rlp, vec![0xb1]);
    assert_eq!(entry_ffi.previous_cert_vote_rlps[0].vote_rlp, vec![0xc1]);
    assert_eq!(entry_ffi.dag_transaction_hashes[0].hash, [0x66; 32]);
    assert_eq!(entry_ffi.period_data_transaction_hashes[0].hash, [0x77; 32]);
    let identity_ffi = &entry_ffi.period_data_transaction_identities[0];
    assert_eq!(
        (identity_ffi.input_index, identity_ffi.hash),
        (3, [0x88; 32])
    );
    assert_eq!(
        (identity_ffi.transaction_nonce, identity_ffi.sender),
        ([0x99; 32], [0xaa; 20])
    );
    assert_eq!(
        (
            entry_ffi.previous_cert_votes_present,
            entry_ffi.previous_cert_first_vote_has_weight,
            entry_ffi.pillar_votes_present,
            entry_ffi.extra_data_present,
            entry_ffi.extra_data_pillar_block_hash_present,
        ),
        (true, false, true, true, false)
    );

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
            outcome.clear_existing,
            outcome.expected_next_period,
            outcome.actual_period,
            outcome.current_period,
            outcome.effective_size,
        ),
        (true, true, 42, 43, 43, 4)
    );

    let pop: FfiPeriodDataQueuePopPlan = PeriodDataQueuePopPlan {
        entry_id: 41,
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
        period_data_transaction_identities: request.entry.period_data_transaction_identities,
        previous_cert_votes_present: true,
        previous_cert_first_vote_has_weight: false,
        pillar_votes_present: true,
        extra_data_present: true,
        extra_data_pillar_block_hash_present: false,
        use_last_block_cert_votes: true,
        next_entry_id: 44,
        current_period: 43,
        effective_size: 2,
    }
    .into();
    assert_eq!((pop.entry_id, pop.entry_period), (41, 42));
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
            pop.use_last_block_cert_votes,
        ),
        (true, false, true, true, false, true)
    );
    assert_eq!(
        (pop.next_entry_id, pop.current_period, pop.effective_size),
        (44, 43, 2)
    );
}
