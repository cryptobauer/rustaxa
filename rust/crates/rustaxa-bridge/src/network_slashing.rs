//! CXX conversion for network-owned slashing submission leaves.
//!
//! These helpers retain only public submitter facts and typed transaction
//! effects needed by tarcap clients; PBFT vote-manager materializers and
//! behavioral adapters are deliberately absent.

use crate::ffi::rustaxa_ffi::{
    PbftSyncIngressStep as FfiPbftSyncIngressStep,
    SlashingSubmitterIdentity as FfiSlashingSubmitterIdentity,
    SlashingTransactionEffect as FfiSlashingTransactionEffect,
};
use ethereum_types::H256;
use ethereum_types::U256;
use rustaxa_consensus::{
    PbftSyncIngressStep, SlashingSubmitterIdentity as DomainSlashingSubmitterIdentity,
    SlashingTransactionEffect as DomainSlashingTransactionEffect,
};

fn pbft_sync_ingress_step_to_ffi(value: PbftSyncIngressStep) -> FfiPbftSyncIngressStep {
    let has_effect = value.slashing_transaction_effect.is_some();
    FfiPbftSyncIngressStep {
        action: value.action.as_u8(),
        error_code: value.error_code,
        source_payload_id: value.source_payload_id,
        block_hash: value.block_hash.0,
        period: value.period,
        max_dag_level: value.max_dag_level,
        last_block: value.last_block,
        current_cert_present: value.current_cert_present,
        has_slashing_transaction_effect: has_effect,
        slashing_transaction_effect: value
            .slashing_transaction_effect
            .map(slashing_transaction_effect_to_ffi)
            .unwrap_or_else(empty_slashing_transaction_effect),
    }
}

/// Starts one native PBFT-sync ingress operation for the network adapter.
pub fn pbft_service_begin_pbft_sync_ingress(
    service: &crate::ffi::BridgeApp,
    packet_rlp: &[u8],
    source_payload_id: u64,
    source_peer_id: [u8; 64],
    slashing_submitters: Vec<FfiSlashingSubmitterIdentity>,
) -> anyhow::Result<FfiPbftSyncIngressStep> {
    service
        .0
        .begin_pbft_sync_ingress(
            &service.0.final_chain_for_bridge(),
            packet_rlp,
            source_payload_id,
            source_peer_id,
            slashing_submitters
                .into_iter()
                .map(slashing_submitter_identity_to_domain)
                .collect(),
        )
        .map(pbft_sync_ingress_step_to_ffi)
}

/// Reports the external slashing-submission result for active sync ingress.
pub fn pbft_service_report_pbft_sync_ingress_slashing(
    service: &crate::ffi::BridgeApp,
    proof_hash: [u8; 32],
    transaction_inserted: bool,
) -> anyhow::Result<FfiPbftSyncIngressStep> {
    service
        .0
        .report_pbft_sync_ingress_slashing(
            &service.0.final_chain_for_bridge(),
            proof_hash.into(),
            transaction_inserted,
        )
        .map(pbft_sync_ingress_step_to_ffi)
}

pub(crate) fn slashing_submitter_identity_to_domain(
    value: FfiSlashingSubmitterIdentity,
) -> DomainSlashingSubmitterIdentity {
    DomainSlashingSubmitterIdentity {
        wallet_index: value.wallet_index,
        address: value.address,
        nonce: U256::from_big_endian(&value.nonce),
        balance: U256::from_big_endian(&value.balance),
    }
}

fn u256_to_bytes(value: U256) -> [u8; 32] {
    value.to_big_endian()
}

pub(crate) fn slashing_transaction_effect_to_ffi(
    value: DomainSlashingTransactionEffect,
) -> FfiSlashingTransactionEffect {
    FfiSlashingTransactionEffect {
        status: value.status.as_u8(),
        proof_hash: value.proof_hash.0,
        wallet_index: value.wallet_index,
        nonce: u256_to_bytes(value.nonce),
        contract_address: value.contract_address,
        value: u256_to_bytes(value.value),
        gas_limit: value.gas_limit,
        call_data: value.call_data,
    }
}

pub(crate) fn empty_slashing_transaction_effect() -> FfiSlashingTransactionEffect {
    FfiSlashingTransactionEffect {
        status: 0,
        proof_hash: [0; 32],
        wallet_index: 0,
        nonce: [0; 32],
        contract_address: [0; 20],
        value: [0; 32],
        gas_limit: 0,
        call_data: Vec::new(),
    }
}

impl crate::ffi::BridgeApp {
    /// Reports one network-executed verified-vote slashing transaction.
    pub fn pbft_service_verified_votes_report_slashing_transaction_submission(
        &self,
        proof_hash: &[u8; 32],
        transaction_inserted: bool,
    ) -> Result<bool, anyhow::Error> {
        Ok(self
            .0
            .report_verified_vote_slashing_transaction_submission(
                H256::from(*proof_hash),
                transaction_inserted,
            )?
            .submitted)
    }
}
