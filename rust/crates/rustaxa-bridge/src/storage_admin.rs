use crate::ffi::rustaxa_ffi;
use crate::ffi::BridgeConsensusApplication;

/// Executes the sole production storage-admin task on the native application root.
///
/// The request contains exclusive logical cutoffs only. Rust owns validation, index selection, the complete atomic
/// batch, and idempotent retry semantics; no column, iterator, batch, or storage lifetime crosses CXX.
pub fn consensus_application_prune_light_history(
    runtime: &BridgeConsensusApplication,
    request: rustaxa_ffi::LightHistoryPruneRequest,
) -> Result<rustaxa_ffi::LightHistoryPruneReport, anyhow::Error> {
    let report = runtime
        .0
        .prune_light_history(rustaxa_storage::LightHistoryPruneRequest {
            end_period_exclusive: request.end_period_exclusive,
            first_retained_dag_level: request.first_retained_dag_level,
            live_cleanup: request.live_cleanup,
            non_block_periods_to_keep: request.non_block_periods_to_keep,
        })?;
    Ok(rustaxa_ffi::LightHistoryPruneReport {
        changed: report.changed,
        end_period_exclusive: report.end_period_exclusive,
        first_retained_dag_level: report.first_retained_dag_level,
        rebuilt_secondary_indexes: report.rebuilt_secondary_indexes,
    })
}

/// Runs the single allowlisted v1 production-root storage conformance scenario.
///
/// The fresh fixture root owns every read and atomic write. Only ordered string observations cross CXX; raw storage
/// capabilities and schema selectors remain private to native Rust.
pub fn consensus_application_run_storage_conformance_v1(
    runtime: &BridgeConsensusApplication,
) -> Result<Vec<rustaxa_ffi::StorageConformanceObservation>, anyhow::Error> {
    Ok(runtime
        .0
        .run_storage_conformance_v1()?
        .into_iter()
        .map(|observation| rustaxa_ffi::StorageConformanceObservation {
            key: observation.key,
            value: observation.value,
        })
        .collect())
}
