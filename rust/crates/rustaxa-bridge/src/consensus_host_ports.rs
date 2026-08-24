//! Thin CXX adapters for the blocking native consensus application runner.
//!
//! This module is the only Rust code allowed to invoke the process-local C++
//! host ports. It converts operation-specific carriers and borrows every port
//! for exactly one blocking run; no CXX reference or secret key material is
//! stored in the native application.

use crate::application_host_ffi::application_host_ffi::{
    CanonicalBytes, ConsensusProcessPort as CxxProcessPort,
    ConsensusRunExit as FfiConsensusRunExit, ConsensusSignerPort as CxxSignerPort,
    ConsensusTransportPort as CxxTransportPort, DagHash, ExternalEvmPort as CxxExternalEvmPort,
    HostAddress20, HostEffectId, HostEvmFinalizationRequest, HostFinalChainAccountFactsRequest,
    HostGossipPillarVoteRequest, HostGossipVoteBundleRequest, HostGossipVoteRequest,
    HostMaliciousPeerRequest, HostPillarAnchorStateRequest, HostSetSyncPeriodRequest,
    HostSignRequest, HostTransportReport, HostVrfRequest, HostWaitRequest,
};
use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::ffi::rustaxa_ffi::HostConsensusLiveStatus;
use anyhow::{bail, Result};
use rustaxa_consensus::consensus_application_runtime::{
    FinalChainAccountFact, FinalChainAccountFactsReport, FinalChainAccountFactsRequest,
    PillarAnchorStateReport, PillarAnchorStateRequest, PillarAnchorValidatorVoteCount,
};
use rustaxa_consensus::{
    ConsensusEffectId, ConsensusProcessPort, ConsensusRunReason, ConsensusSignReport,
    ConsensusSignRequest, ConsensusSigningPort, ConsensusTransportReport, ConsensusVrfReport,
    ConsensusVrfRequest, ConsensusWaitOutcome, ConsensusWaitReport, ConsensusWaitRequest,
    EvmFinalizationReport, EvmFinalizationRequest, GossipPillarVoteRequest,
    GossipVoteBundleRequest, GossipVoteRequest, ReportMaliciousPeerRequest, SetSyncPeriodRequest,
};

fn to_ffi_effect_id(id: ConsensusEffectId) -> HostEffectId {
    HostEffectId {
        generation: id.generation,
        sequence: id.sequence,
    }
}

fn to_native_effect_id(id: HostEffectId) -> ConsensusEffectId {
    ConsensusEffectId {
        generation: id.generation,
        sequence: id.sequence,
    }
}

fn to_native_transport_report(report: HostTransportReport) -> ConsensusTransportReport {
    ConsensusTransportReport {
        effect_id: to_native_effect_id(report.effect_id),
        succeeded: report.succeeded,
        error_code: report.error_code,
    }
}

struct ProcessPortAdapter<'a>(&'a CxxProcessPort);

impl ConsensusProcessPort for ProcessPortAdapter<'_> {
    fn now_millis(&self) -> u64 {
        self.0.consensus_now_millis()
    }

    fn unix_time_seconds(&self) -> u64 {
        self.0.consensus_unix_time_seconds()
    }

    fn wait(&self, request: &ConsensusWaitRequest) -> Result<ConsensusWaitReport> {
        let report = self.0.consensus_wait(&HostWaitRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            delay_ms: request.delay_ms,
        })?;
        let outcome = match report.outcome {
            0 => ConsensusWaitOutcome::Elapsed,
            1 => ConsensusWaitOutcome::Stopped,
            code => bail!("CONSENSUS_HOST_WAIT_INVALID_OUTCOME: {code}"),
        };
        Ok(ConsensusWaitReport {
            effect_id: to_native_effect_id(report.effect_id),
            outcome,
        })
    }

    fn stop_requested(&self, generation: u64) -> bool {
        self.0.consensus_stop_requested(generation)
    }
}

struct SigningPortAdapter<'a>(&'a CxxSignerPort);

impl ConsensusSigningPort for SigningPortAdapter<'_> {
    fn sign_digest(&self, request: &ConsensusSignRequest) -> Result<ConsensusSignReport> {
        let report = self.0.consensus_sign_digest(&HostSignRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            wallet_index: request.wallet_index,
            digest: request.digest,
        })?;
        Ok(ConsensusSignReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            signature: report.signature,
            error_code: report.error_code,
        })
    }

    fn prove_vrf(&self, request: &ConsensusVrfRequest) -> Result<ConsensusVrfReport> {
        let report = self.0.consensus_prove_vrf(&HostVrfRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            wallet_index: request.wallet_index,
            message: request.message.clone(),
        })?;
        Ok(ConsensusVrfReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            proof: report.proof,
            output: report.output,
            error_code: report.error_code,
        })
    }
}

struct TransportPortAdapter<'a>(&'a CxxTransportPort);

impl rustaxa_consensus::ConsensusTransportPort for TransportPortAdapter<'_> {
    fn gossip_vote(&self, request: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
        Ok(to_native_transport_report(self.0.consensus_gossip_vote(
            &HostGossipVoteRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                vote_rlp: request.vote_rlp.clone(),
                proposed_block_rlp: request.proposed_block_rlp.clone(),
                rebroadcast: request.rebroadcast,
            },
        )?))
    }

    fn gossip_vote_bundle(
        &self,
        request: &GossipVoteBundleRequest,
    ) -> Result<ConsensusTransportReport> {
        Ok(to_native_transport_report(
            self.0
                .consensus_gossip_vote_bundle(&HostGossipVoteBundleRequest {
                    effect_id: to_ffi_effect_id(request.effect_id),
                    votes_bundle_rlp: request.votes_bundle_rlp.clone(),
                    rebroadcast: request.rebroadcast,
                })?,
        ))
    }

    fn gossip_pillar_vote(
        &self,
        request: &GossipPillarVoteRequest,
    ) -> Result<ConsensusTransportReport> {
        Ok(to_native_transport_report(
            self.0
                .consensus_gossip_pillar_vote(&HostGossipPillarVoteRequest {
                    effect_id: to_ffi_effect_id(request.effect_id),
                    pillar_vote_rlp: request.pillar_vote_rlp.clone(),
                    rebroadcast: request.rebroadcast,
                })?,
        ))
    }

    fn set_sync_period(&self, request: &SetSyncPeriodRequest) -> Result<ConsensusTransportReport> {
        Ok(to_native_transport_report(
            self.0
                .consensus_set_sync_period(&HostSetSyncPeriodRequest {
                    effect_id: to_ffi_effect_id(request.effect_id),
                    period: request.period,
                })?,
        ))
    }

    fn transport_status(&self) -> rustaxa_consensus::ConsensusTransportStatus {
        let status = self.0.consensus_transport_status();
        rustaxa_consensus::ConsensusTransportStatus {
            available: status.available,
            pbft_syncing: status.pbft_syncing,
        }
    }

    fn report_malicious_peer(
        &self,
        request: &ReportMaliciousPeerRequest,
    ) -> Result<ConsensusTransportReport> {
        Ok(to_native_transport_report(
            self.0
                .consensus_report_malicious_peer(&HostMaliciousPeerRequest {
                    effect_id: to_ffi_effect_id(request.effect_id),
                    peer_id: request.peer_id,
                    evidence_rlp: request.evidence_rlp.clone(),
                })?,
        ))
    }
}

struct ExternalEvmPortAdapter<'a>(&'a CxxExternalEvmPort);

impl rustaxa_consensus::ConsensusExecutionPort for ExternalEvmPortAdapter<'_> {
    fn load_final_chain_account_facts(
        &self,
        request: &FinalChainAccountFactsRequest,
    ) -> Result<FinalChainAccountFactsReport> {
        let report = self.0.consensus_load_final_chain_account_facts(
            &HostFinalChainAccountFactsRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                addresses: request
                    .addresses
                    .iter()
                    .copied()
                    .map(|bytes| HostAddress20 { bytes })
                    .collect(),
            },
        )?;
        Ok(FinalChainAccountFactsReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            observed_block: report.observed_block,
            accounts: report
                .accounts
                .into_iter()
                .map(|fact| FinalChainAccountFact {
                    address: fact.address,
                    found: fact.found,
                    nonce: fact.nonce,
                    balance: fact.balance,
                })
                .collect(),
            error_code: report.error_code,
        })
    }

    fn load_pillar_anchor_state(
        &self,
        request: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        let report = self
            .0
            .consensus_load_pillar_anchor_state(&HostPillarAnchorStateRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                period: request.period,
                pillar_block_period: request.pillar_block_period,
                signer_addresses: request
                    .signer_addresses
                    .iter()
                    .copied()
                    .map(|bytes| HostAddress20 { bytes })
                    .collect(),
            })?;
        Ok(PillarAnchorStateReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            block_header_rlp: report.block_header_rlp,
            state_root: report.state_root,
            bridge_root: report.bridge_root,
            bridge_epoch: report.bridge_epoch,
            validator_vote_counts: report
                .validator_vote_counts
                .into_iter()
                .map(|fact| PillarAnchorValidatorVoteCount {
                    address: fact.address,
                    vote_count: fact.vote_count,
                })
                .collect(),
            signer_vote_counts: report.signer_vote_counts,
            total_eligible_vote_count: report.total_eligible_vote_count,
            error_code: report.error_code,
        })
    }

    fn execute_finalization(
        &self,
        request: &EvmFinalizationRequest,
    ) -> Result<EvmFinalizationReport> {
        let report = self
            .0
            .consensus_execute_finalization(&HostEvmFinalizationRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                period_data_rlp: request.period_data_rlp.clone(),
                previous_cert_vote_rlps: request
                    .previous_cert_vote_rlps
                    .iter()
                    .cloned()
                    .map(|data| CanonicalBytes { data })
                    .collect(),
                finalized_dag_hashes: request
                    .finalized_dag_hashes
                    .iter()
                    .copied()
                    .map(|hash| DagHash { hash })
                    .collect(),
                blocks_per_year: request.blocks_per_year,
                synchronous: request.synchronous,
                anchor_block_rlp: request.anchor_block_rlp.clone(),
            })?;
        Ok(EvmFinalizationReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            status: report.status,
            last_block_number: report.last_block_number,
            error_code: report.error_code,
        })
    }
}

/// Runs one native consensus generation against four borrowed exact host ports.
///
/// The native runner implementation is connected here rather than in CXX so
/// effect identity validation and all protocol progression remain in Rust.
pub fn consensus_application_run(
    application: &BridgeConsensusApplication,
    process: &CxxProcessPort,
    signer: &CxxSignerPort,
    transport: &CxxTransportPort,
    external_evm: &CxxExternalEvmPort,
) -> Result<FfiConsensusRunExit> {
    let process = ProcessPortAdapter(process);
    let signer = SigningPortAdapter(signer);
    let transport = TransportPortAdapter(transport);
    let external_evm = ExternalEvmPortAdapter(external_evm);
    let exit = application
        .0
        .run_consensus(&process, &signer, &transport, &external_evm)?;
    let reason = match exit.reason {
        ConsensusRunReason::Stopped => 0,
        ConsensusRunReason::Completed => 1,
    };
    Ok(FfiConsensusRunExit {
        generation: exit.generation,
        reason,
        error_code: String::new(),
    })
}

/// Returns the application-root hot PBFT status without an executor snapshot.
pub fn consensus_application_live_status(
    application: &BridgeConsensusApplication,
) -> Result<HostConsensusLiveStatus> {
    let status = application.0.consensus_live_status()?;
    let vote_status = application.0.consensus_vote_status()?;
    Ok(HostConsensusLiveStatus {
        period: status.period,
        round: status.round,
        step: status.step,
        finalized_chain_size: status.finalized_chain_size,
        syncing_period: status.syncing_period,
        sync_queue_size: status.sync_queue_size,
        has_current_node_votes: vote_status.current_node_votes.is_some(),
        current_node_votes: vote_status.current_node_votes.unwrap_or_default(),
        has_total_eligible_votes: vote_status.total_eligible_votes.is_some(),
        total_eligible_votes: vote_status.total_eligible_votes.unwrap_or_default(),
    })
}
