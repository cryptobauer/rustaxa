//! Application-root construction and operation-shaped DAG/transaction bridge leaves.

use crate::ffi::rustaxa_ffi::*;
use anyhow::Result;
use ethereum_types::U256;
use rustaxa_consensus::ConsensusApplication;

/// Opaque CXX lifetime receiver for one fully restored native application.
/// No native manager or service handle is constructible or retrievable from CXX.
pub struct BridgeConsensusApplication(pub(crate) ApplicationBackend);

pub(crate) enum ApplicationBackend {
    Historical(ConsensusApplication),
    Production(rustaxa_execution::ProductionApplication),
}

impl ApplicationBackend {
    pub(crate) fn historical(&self) -> Result<&ConsensusApplication> {
        match self {
            Self::Historical(application) => Ok(application),
            Self::Production(_) => anyhow::bail!("PRODUCTION_LEGACY_OPERATION_UNAVAILABLE"),
        }
    }

    #[cfg(feature = "production_integration_test")]
    pub(crate) fn production(&self) -> Result<&rustaxa_execution::ProductionApplication> {
        match self {
            Self::Production(application) => Ok(application),
            Self::Historical(_) => anyhow::bail!("PRODUCTION_APPLICATION_REQUIRED"),
        }
    }
}

pub(crate) fn public_transaction_request_to_native(
    request: PublicTransactionSubmissionRequest,
) -> rustaxa_consensus::PublicTransactionSubmissionRequest {
    rustaxa_consensus::PublicTransactionSubmissionRequest {
        transaction_rlp: request.transaction_rlp,
        expected_chain_id: request.expected_chain_id,
        maximum_gas_limit: request.maximum_gas_limit,
        minimum_gas_price: U256::from_big_endian(&request.minimum_gas_price),
        last_block_number: request.last_block_number,
        cornus_active: request.cornus_active,
    }
}

pub(crate) fn public_transaction_report_to_ffi(
    report: rustaxa_consensus::PublicTransactionSubmissionReport,
) -> PublicTransactionSubmissionReport {
    let (queue_status_found, queue_status) = report.queue_status.map_or((false, 0), |status| {
        use rustaxa_consensus::transaction_queue::TransactionQueueInsertStatus;
        let code = match status {
            TransactionQueueInsertStatus::Inserted => 0,
            TransactionQueueInsertStatus::InsertedNonProposable => 1,
            TransactionQueueInsertStatus::Known => 2,
            TransactionQueueInsertStatus::Overflow => 3,
        };
        (true, code)
    });
    PublicTransactionSubmissionReport {
        transaction_hash: report.transaction_hash.into(),
        accepted: report.accepted,
        message: report.message,
        verification_status: report.verification_status.as_u8(),
        queue_status_found,
        queue_status,
        transaction_observed: report.transaction_observed,
    }
}

/// Submits one canonical signed transaction through the native application root.
pub fn consensus_application_submit_transaction(
    application: &BridgeConsensusApplication,
    request: PublicTransactionSubmissionRequest,
) -> Result<PublicTransactionSubmissionReport> {
    let request = public_transaction_request_to_native(request);
    let report = match &application.0 {
        ApplicationBackend::Historical(owner) => {
            owner.submit_public_transaction_from_native_state(request)?
        }
        ApplicationBackend::Production(owner) => {
            owner.submit_public_transaction_from_native_state(request)?
        }
    };
    Ok(public_transaction_report_to_ffi(report))
}

pub fn create_consensus_application(
    config: ConsensusApplicationBootstrapConfig,
) -> Result<Box<BridgeConsensusApplication>> {
    let root = rustaxa_consensus::production_bootstrap::application_bootstrap_from_bridge(config)?
        .bootstrap()?;
    Ok(Box::new(BridgeConsensusApplication(
        ApplicationBackend::Historical(root),
    )))
}
