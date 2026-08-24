//! Thin CXX adapters for the blocking native consensus application runner.
//! This sole host caller converts exact carriers without retaining CXX references or secret keys.

use crate::application_host_ffi::application_host_ffi::*;
use crate::dag_transaction_service::BridgeConsensusApplication;
use crate::dag_transaction_service::{
    public_transaction_report_to_ffi, public_transaction_request_to_native,
};
use crate::ffi::{rustaxa_ffi, rustaxa_ffi::HostConsensusLiveStatus, BridgeConsensusNetworkApi};
use crate::network::to_bridge_network_ingress_decision;
use anyhow::{bail, ensure, Result};
use rustaxa_consensus::consensus_application_runtime::{
    FinalChainAccountFact, FinalChainAccountFactsReport, FinalChainAccountFactsRequest,
    PillarAnchorStateReport, PillarAnchorStateRequest, PillarAnchorValidatorVoteCount,
};
use rustaxa_consensus::{
    ConsensusEffectId, ConsensusProcessPort as NativeProcessPort, ConsensusRunReason,
    ConsensusSignReport, ConsensusSignRequest, ConsensusSigningPort as NativeSigningPort,
    ConsensusTransportReport, ConsensusVrfReport, ConsensusVrfRequest, ConsensusWaitOutcome,
    ConsensusWaitReport, ConsensusWaitRequest, DagGasEstimateReport, DagGasEstimateRequest,
    DagGasEstimateResult, EvmFinalizationReport, EvmFinalizationRequest, GossipPillarVoteRequest,
    GossipVoteBundleRequest, GossipVoteRequest, ReportMaliciousPeerRequest, SetSyncPeriodRequest,
};

struct ProcessPortAdapter<'a>(&'a ConsensusProcessPort);

impl rustaxa_consensus::ConsensusVdfPort for ProcessPortAdapter<'_> {
    fn start_dag_vdf(
        &self,
        request: &rustaxa_consensus::DagVdfRequest,
    ) -> Result<rustaxa_consensus::DagVdfStartReport> {
        let report = self.0.consensus_start_dag_vdf(&HostDagVdfRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            wallet_index: request.wallet_index,
            vrf_input: request.vrf_input.clone(),
            vdf_message: request.vdf_message.clone(),
            vote_count: request.vote_count,
            max_vote_count: request.max_vote_count,
            difficulty: request.difficulty,
            lambda_bound: request.lambda_bound,
        })?;
        Ok(rustaxa_consensus::DagVdfStartReport {
            effect_id: to_native_effect_id(report.effect_id),
            started: report.started,
            job_id: report.job_id,
            error_code: report.error_code,
        })
    }

    fn poll_dag_vdf(
        &self,
        request: &rustaxa_consensus::DagVdfPollRequest,
    ) -> Result<rustaxa_consensus::DagVdfPollReport> {
        let report = self.0.consensus_poll_dag_vdf(&HostDagVdfJobRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            job_id: request.job_id,
        })?;
        Ok(rustaxa_consensus::DagVdfPollReport {
            effect_id: to_native_effect_id(report.effect_id),
            job_id: report.job_id,
            complete: report.complete,
            succeeded: report.succeeded,
            cancelled: report.cancelled,
            vdf_rlp: report.vdf_rlp,
            error_code: report.error_code,
        })
    }

    fn cancel_dag_vdf(
        &self,
        request: &rustaxa_consensus::DagVdfCancelRequest,
    ) -> Result<rustaxa_consensus::DagVdfCancelReport> {
        let report = self.0.consensus_cancel_dag_vdf(&HostDagVdfJobRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            job_id: request.job_id,
        })?;
        Ok(rustaxa_consensus::DagVdfCancelReport {
            effect_id: to_native_effect_id(report.effect_id),
            job_id: report.job_id,
            cancelled: report.cancelled,
            error_code: report.error_code,
        })
    }
}

impl rustaxa_consensus::ConsensusObserverPort for ProcessPortAdapter<'_> {
    fn observe(
        &self,
        request: &rustaxa_consensus::ConsensusObservationRequest,
    ) -> Result<rustaxa_consensus::ConsensusObservationReport> {
        let report = self.0.consensus_observe(&HostConsensusObservationRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            kind: request.kind,
            hash: request.hash,
            canonical_rlp: request.canonical_rlp.clone(),
        })?;
        Ok(rustaxa_consensus::ConsensusObservationReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            error_code: report.error_code,
        })
    }
}

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

impl NativeProcessPort for ProcessPortAdapter<'_> {
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

struct SigningPortAdapter<'a>(&'a ConsensusSignerPort);

impl NativeSigningPort for SigningPortAdapter<'_> {
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

struct TransportPortAdapter<'a>(&'a ConsensusTransportPort);

impl rustaxa_consensus::ConsensusTransportPort for TransportPortAdapter<'_> {
    fn gossip_vote(&self, request: &GossipVoteRequest) -> Result<ConsensusTransportReport> {
        let report = self.0.consensus_gossip_vote(&HostGossipVoteRequest {
            effect_id: to_ffi_effect_id(request.effect_id),
            vote_rlp: request.vote_rlp.clone(),
            proposed_block_rlp: request.proposed_block_rlp.clone(),
            rebroadcast: request.rebroadcast,
        })?;
        Ok(to_native_transport_report(report))
    }

    fn gossip_vote_bundle(
        &self,
        request: &GossipVoteBundleRequest,
    ) -> Result<ConsensusTransportReport> {
        let report = self
            .0
            .consensus_gossip_vote_bundle(&HostGossipVoteBundleRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                votes_bundle_rlp: request.votes_bundle_rlp.clone(),
                rebroadcast: request.rebroadcast,
            })?;
        Ok(to_native_transport_report(report))
    }

    fn gossip_pillar_vote(
        &self,
        request: &GossipPillarVoteRequest,
    ) -> Result<ConsensusTransportReport> {
        let report = self
            .0
            .consensus_gossip_pillar_vote(&HostGossipPillarVoteRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                pillar_vote_rlp: request.pillar_vote_rlp.clone(),
                rebroadcast: request.rebroadcast,
            })?;
        Ok(to_native_transport_report(report))
    }

    fn set_sync_period(&self, request: &SetSyncPeriodRequest) -> Result<ConsensusTransportReport> {
        let report = self
            .0
            .consensus_set_sync_period(&HostSetSyncPeriodRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                period: request.period,
            })?;
        Ok(to_native_transport_report(report))
    }

    fn transport_status(&self) -> rustaxa_consensus::ConsensusTransportStatus {
        let status = self.0.consensus_transport_status();
        rustaxa_consensus::ConsensusTransportStatus {
            available: status.available,
            pbft_syncing: status.pbft_syncing,
            packet_queue_over_limit: status.packet_queue_over_limit,
        }
    }

    fn report_malicious_peer(
        &self,
        request: &ReportMaliciousPeerRequest,
    ) -> Result<ConsensusTransportReport> {
        let report = self
            .0
            .consensus_report_malicious_peer(&HostMaliciousPeerRequest {
                effect_id: to_ffi_effect_id(request.effect_id),
                peer_id: request.peer_id,
                evidence_rlp: request.evidence_rlp.clone(),
            })?;
        Ok(to_native_transport_report(report))
    }
}

struct ExternalEvmPortAdapter<'a>(&'a ExternalEvmPort);

impl rustaxa_consensus::ConsensusExecutionPort for ExternalEvmPortAdapter<'_> {
    fn estimate_dag_transaction_gas(
        &self,
        request: &DagGasEstimateRequest,
    ) -> Result<DagGasEstimateReport> {
        let (transaction_hashes, transaction_rlps) = request
            .transactions
            .iter()
            .map(|transaction| {
                (
                    DagHash {
                        hash: transaction.hash,
                    },
                    CanonicalBytes {
                        data: transaction.transaction_rlp.clone(),
                    },
                )
            })
            .unzip();
        let report = self
            .0
            .consensus_estimate_dag_transaction_gas(&HostDagGasBatch {
                effect_id: to_ffi_effect_id(request.effect_id),
                proposal_period: request.proposal_period,
                transaction_hashes,
                transaction_rlps,
                succeeded: false,
                observed_block: 0,
                gas_used: Vec::new(),
                result_rlps: Vec::new(),
                error_code: String::new(),
            })?;
        let hashes_match = report
            .transaction_hashes
            .iter()
            .map(|hash| hash.hash)
            .eq(request
                .transactions
                .iter()
                .map(|transaction| transaction.hash));
        let output_count = (report.gas_used.len(), report.result_rlps.len());
        let expected_output_count = if report.succeeded {
            (request.transactions.len(), request.transactions.len())
        } else {
            (0, 0)
        };
        ensure!(
            report.effect_id.generation == request.effect_id.generation
                && report.effect_id.sequence == request.effect_id.sequence
                && report.proposal_period == request.proposal_period
                && hashes_match
                && output_count == expected_output_count,
            "DAG gas executor changed identity/order or returned partial output"
        );
        Ok(DagGasEstimateReport {
            effect_id: to_native_effect_id(report.effect_id),
            succeeded: report.succeeded,
            observed_block: report.observed_block,
            estimates: report
                .transaction_hashes
                .into_iter()
                .zip(report.gas_used)
                .zip(report.result_rlps)
                .map(|((hash, gas_used), result_rlp)| DagGasEstimateResult {
                    hash: hash.hash,
                    gas_used,
                    result_rlp: result_rlp.data,
                })
                .collect(),
            error_code: report.error_code,
        })
    }

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

/// Runs one native generation against borrowed exact ports; native effect
/// validation/progression errors propagate without retaining host references.
pub fn consensus_application_run(
    application: &BridgeConsensusApplication,
    process: &ConsensusProcessPort,
    signer: &ConsensusSignerPort,
    transport: &ConsensusTransportPort,
    external_evm: &ExternalEvmPort,
) -> Result<ConsensusRunExit> {
    let process = ProcessPortAdapter(process);
    let signer = SigningPortAdapter(signer);
    let transport = TransportPortAdapter(transport);
    let external_evm = ExternalEvmPortAdapter(external_evm);
    let exit = application.0.run_consensus(
        &process,
        &signer,
        &transport,
        &external_evm,
        &process,
        &process,
    )?;
    let reason = match exit.reason {
        ConsensusRunReason::Stopped => 0,
        ConsensusRunReason::Completed => 1,
    };
    Ok(ConsensusRunExit {
        generation: exit.generation,
        reason,
        error_code: String::new(),
    })
}

/// Submits canonical bytes through one validated account-fact request and
/// returns native admission; no manager or decoded transaction crosses CXX.
pub fn consensus_application_submit_transaction_with_execution(
    application: &BridgeConsensusApplication,
    request: rustaxa_ffi::PublicTransactionSubmissionRequest,
    external_evm: &ExternalEvmPort,
) -> Result<rustaxa_ffi::PublicTransactionSubmissionReport> {
    let external_evm = ExternalEvmPortAdapter(external_evm);
    Ok(public_transaction_report_to_ffi(
        application.0.submit_public_transaction_with_execution(
            public_transaction_request_to_native(request),
            &external_evm,
        )?,
    ))
}

/// Routes one canonical packet through native limits, admission, facts, and effects.
pub fn consensus_network_ingest_transaction_packet(
    network: &BridgeConsensusNetworkApi,
    application: &BridgeConsensusApplication,
    request: rustaxa_ffi::NetworkTransactionPacketRequest,
    external_evm: &ExternalEvmPort,
) -> Result<rustaxa_ffi::NetworkTransactionPacketReport> {
    let external_evm = ExternalEvmPortAdapter(external_evm);
    let context = rustaxa_consensus::NetworkTransactionPacketContext {
        transport_lane: request.transport_lane,
        peer_id: request.peer_id,
        source_payload_id: request.source_payload_id,
    };
    let policy = rustaxa_consensus::PublicTransactionSubmissionRequest {
        transaction_rlp: Vec::new(),
        expected_chain_id: request.expected_chain_id,
        maximum_gas_limit: request.maximum_gas_limit,
        minimum_gas_price: ethereum_types::U256::from_big_endian(&request.minimum_gas_price),
        last_block_number: request.last_block_number,
        cornus_active: request.cornus_active,
    };
    let rebroadcast = request.rebroadcast;
    let report = network.network.ingest_transaction_packet(
        context,
        &request.packet_rlp,
        |transaction_rlp| {
            let mut submission = policy.clone();
            submission.transaction_rlp = transaction_rlp;
            application.0.ingest_transaction_packet(
                rustaxa_consensus::TransactionPacketIngressRequest {
                    submission,
                    peer_id: request.peer_id,
                    rebroadcast,
                },
                &external_evm,
            )
        },
    )?;
    Ok(rustaxa_ffi::NetworkTransactionPacketReport {
        decision: to_bridge_network_ingress_decision(report.decision),
        transactions: report
            .transactions
            .into_iter()
            .map(transaction_packet_member_to_ffi)
            .collect(),
        extra_transaction_hashes: report
            .extra_transaction_hashes
            .into_iter()
            .map(|hash| rustaxa_ffi::DagHash { hash })
            .collect(),
    })
}

fn dag_block_ingress_report_to_ffi(
    report: rustaxa_consensus::DagBlockIngressReport,
) -> rustaxa_ffi::DagBlockIngressReport {
    rustaxa_ffi::DagBlockIngressReport {
        block_hash: report.block_hash.into(),
        block_level: report.block_level,
        accepted: report.accepted,
        duplicate: report.duplicate,
        reject_code: report.reject_code,
        observe_block: report.observe_block,
        gossip_block: report.gossip_block,
        block_rlp: report.block_rlp,
    }
}

fn transaction_packet_member_to_ffi(
    member: rustaxa_consensus::TransactionPacketIngressReport,
) -> rustaxa_ffi::NetworkTransactionPacketMemberReport {
    rustaxa_ffi::NetworkTransactionPacketMemberReport {
        submission: public_transaction_report_to_ffi(member.submission),
        observe_transaction: member.observe_transaction,
        gossip_transaction: member.gossip_transaction,
        transaction_rlp: member.transaction_rlp,
    }
}

fn dag_ingress_context(
    request: &rustaxa_ffi::NetworkDagPacketRequest,
    rebroadcast: bool,
) -> rustaxa_consensus::NetworkDagBlockIngressContext {
    rustaxa_consensus::NetworkDagBlockIngressContext {
        transport_lane: request.transport_lane,
        peer_id: request.peer_id,
        source_payload_id: request.source_payload_id,
        rebroadcast,
        peer_dag_synced: request.peer_dag_synced,
        dag_sync_allowed: request.dag_sync_allowed,
        transactions_dropped: request.transactions_dropped,
        pending_dag_request: request.pending_dag_request,
        local_pbft_syncing: request.local_pbft_syncing,
    }
}

fn dag_packet_policy(
    request: &rustaxa_ffi::NetworkDagPacketRequest,
    transaction_rlp: Vec<u8>,
) -> rustaxa_consensus::PublicTransactionSubmissionRequest {
    rustaxa_consensus::PublicTransactionSubmissionRequest {
        transaction_rlp,
        expected_chain_id: request.expected_chain_id,
        maximum_gas_limit: request.maximum_gas_limit,
        minimum_gas_price: ethereum_types::U256::from_big_endian(&request.minimum_gas_price),
        last_block_number: request.last_block_number,
        cornus_active: request.cornus_active,
    }
}

/// Routes one canonical DAG-block packet through native verification/admission.
pub fn consensus_network_ingest_dag_block_packet(
    network: &BridgeConsensusNetworkApi,
    application: &BridgeConsensusApplication,
    request: rustaxa_ffi::NetworkDagPacketRequest,
    external_evm: &ExternalEvmPort,
) -> Result<rustaxa_ffi::NetworkDagBlockIngressReport> {
    let external_evm = ExternalEvmPortAdapter(external_evm);
    let context = dag_ingress_context(&request, request.rebroadcast);
    let report = network.network.ingest_dag_block_packet(
        context,
        &request.packet_rlp,
        |block_rlp, transaction_rlps| {
            application.0.ingest_dag_block_packet(
                rustaxa_consensus::DagBlockIngressRequest {
                    block_rlp,
                    transaction_rlps,
                    proposed: false,
                },
                &external_evm,
            )
        },
    )?;
    let admission_found = report.admission.is_some();
    Ok(rustaxa_ffi::NetworkDagBlockIngressReport {
        decision: to_bridge_network_ingress_decision(report.decision),
        admission_found,
        admission: report
            .admission
            .map(dag_block_ingress_report_to_ffi)
            .unwrap_or_default(),
        rejection_action: report.rejection_action,
    })
}

/// Routes one canonical DAG-sync packet with native sequential partial commits.
pub fn consensus_network_ingest_dag_sync_packet(
    network: &BridgeConsensusNetworkApi,
    application: &BridgeConsensusApplication,
    request: rustaxa_ffi::NetworkDagPacketRequest,
    external_evm: &ExternalEvmPort,
) -> Result<rustaxa_ffi::NetworkDagSyncIngressReport> {
    let external_evm = ExternalEvmPortAdapter(external_evm);
    let context = dag_ingress_context(&request, false);
    let report = network.network.ingest_dag_sync_packet(
        context,
        &request.packet_rlp,
        |transactions, blocks| {
            application.0.ingest_dag_sync_packet(
                rustaxa_consensus::DagSyncIngressRequest {
                    transactions: transactions
                        .into_iter()
                        .map(
                            |transaction_rlp| rustaxa_consensus::TransactionPacketIngressRequest {
                                submission: dag_packet_policy(&request, transaction_rlp),
                                peer_id: request.peer_id,
                                rebroadcast: false,
                            },
                        )
                        .collect(),
                    blocks: blocks
                        .into_iter()
                        .map(|block_rlp| rustaxa_consensus::DagBlockIngressRequest {
                            block_rlp,
                            transaction_rlps: Vec::new(),
                            proposed: false,
                        })
                        .collect(),
                },
                &external_evm,
            )
        },
    )?;
    Ok(rustaxa_ffi::NetworkDagSyncIngressReport {
        decision: to_bridge_network_ingress_decision(report.decision),
        request_period: report.request_period,
        response_period: report.response_period,
        transactions: report
            .transactions
            .into_iter()
            .map(transaction_packet_member_to_ffi)
            .collect(),
        blocks: report
            .blocks
            .into_iter()
            .map(dag_block_ingress_report_to_ffi)
            .collect(),
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
