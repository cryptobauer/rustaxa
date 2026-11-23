//! Bind validated native configuration to a qualified Rust application.
use anyhow::Result;
use rustaxa_execution::{ProductionApplication, ProductionBootstrapConfig};

use crate::dag_transaction_service::{ApplicationBackend, BridgeConsensusApplication};

#[cfg(feature = "production_integration_test")]
fn production_fixture_disk_step(
    source: cxx::UniquePtr<rustaxa_native_config::NativeSelectedProductionConfig>,
    foreign_source: cxx::UniquePtr<rustaxa_native_config::NativeSelectedProductionConfig>,
    request: ffi::AppFixtureDiskRequest,
) -> Result<ffi::AppFixtureDiskReport> {
    use ffi::AppFixtureDiskStep;
    use rustaxa_execution::production_owner::publication_tests::*;
    let selected = rustaxa_native_config::bind_selected_production_config(source)?;
    anyhow::ensure!(
        request.step == AppFixtureDiskStep::CheckPublication || foreign_source.is_null(),
        "PRODUCTION_FIXTURE_UNEXPECTED_FOREIGN_SOURCE"
    );
    let mut report = ffi::AppFixtureDiskReport {
        message: String::new(),
        transactions: Vec::new(),
    };
    match request.step {
        AppFixtureDiskStep::Prepare => {
            report.message =
                rustaxa_consensus::production_bootstrap::prepare_disposable_native_fixture(
                    &selected,
                )?;
        }
        AppFixtureDiskStep::SeedPredecessor => seed_app_predecessor(
            *selected,
            request.secret,
            request.vrf_secret,
            request.stop != 0,
        )?,
        AppFixtureDiskStep::InterruptPublication => {
            report.transactions = interrupt_app_publication(
                *selected,
                request.secret,
                request.vrf_secret,
                request.stop,
            )?
            .into_iter()
            .map(|rlp| ffi::AppFixtureTransaction { rlp })
            .collect();
        }
        AppFixtureDiskStep::CheckRestartImages => {
            check_app_restart_images(&selected, request.after_restart)?
        }
        AppFixtureDiskStep::CheckPublication => {
            let foreign = if foreign_source.is_null() {
                None
            } else {
                Some(*rustaxa_native_config::bind_selected_production_config(
                    foreign_source,
                )?)
            };
            report.message = run_publication_fixture_checks(
                *selected,
                foreign,
                request.secret,
                request.vrf_secret,
                request.stop,
            )?;
        }
        _ => anyhow::bail!("PRODUCTION_FIXTURE_DISK_STEP_INVALID"),
    }
    Ok(report)
}

#[cfg(feature = "production_integration_test")]
fn production_fixture_live_step(
    application: &BridgeConsensusApplication,
    source: cxx::UniquePtr<rustaxa_native_config::NativeSelectedProductionConfig>,
    request: ffi::AppFixtureLiveRequest,
) -> Result<String> {
    use ffi::AppFixtureLiveStep;
    use rustaxa_execution::production_owner::publication_tests::*;
    let selected = rustaxa_native_config::bind_selected_production_config(source)?;
    let owner = application.0.production()?;
    match request.step {
        AppFixtureLiveStep::CheckFixture => {
            rustaxa_execution::production_owner::integration_tests::run_native_fixture_checks(
                owner,
                &selected,
                request.secret,
                request.vrf_secret,
            )
        }
        AppFixtureLiveStep::CheckInterruptedRecovery => {
            check_app_interrupted_recovery(owner, &selected, request.second_open)?;
            Ok(String::new())
        }
        AppFixtureLiveStep::RetainCertificate | AppFixtureLiveStep::RetainNextVotes => {
            let transactions: Vec<_> = request.transactions.into_iter().map(|tx| tx.rlp).collect();
            retain_app_certificate(
                owner,
                &selected,
                &transactions,
                request.vrf_secret,
                request.step == AppFixtureLiveStep::RetainNextVotes,
                request.second_open,
            )?;
            Ok(String::new())
        }
        AppFixtureLiveStep::CheckReproposal | AppFixtureLiveStep::CheckScheduledPillarVote => {
            let found = if request.step == AppFixtureLiveStep::CheckReproposal {
                check_app_reproposal(owner)?
            } else {
                check_app_scheduled_pillar_vote(owner)?
            };
            Ok(if found {
                "verified".into()
            } else {
                String::new()
            })
        }
        _ => anyhow::bail!("PRODUCTION_FIXTURE_LIVE_STEP_INVALID"),
    }
}

fn create_production_application(
    source: cxx::UniquePtr<rustaxa_native_config::NativeSelectedProductionConfig>,
    concrete_path: String,
) -> Result<Box<BridgeConsensusApplication>> {
    let selected = rustaxa_native_config::bind_selected_production_config(source)?;
    Ok(Box::new(BridgeConsensusApplication(
        ApplicationBackend::Production(ProductionApplication::open_existing(
            ProductionBootstrapConfig {
                selected: std::sync::Arc::from(selected),
                concrete_path: concrete_path.into(),
            },
        )?),
    )))
}

#[cxx::bridge(namespace = "rustaxa")]
mod ffi {
    #[cfg(feature = "production_integration_test")]
    struct AppFixtureTransaction {
        rlp: Vec<u8>,
    }

    #[cfg(feature = "production_integration_test")]
    enum AppFixtureDiskStep {
        Prepare,
        SeedPredecessor,
        InterruptPublication,
        CheckRestartImages,
        CheckPublication,
    }
    #[cfg(feature = "production_integration_test")]
    struct AppFixtureDiskRequest {
        step: AppFixtureDiskStep,
        secret: [u8; 32],
        vrf_secret: [u8; 64],
        stop: u8,
        after_restart: bool,
    }
    #[cfg(feature = "production_integration_test")]
    struct AppFixtureDiskReport {
        message: String,
        transactions: Vec<AppFixtureTransaction>,
    }
    #[cfg(feature = "production_integration_test")]
    enum AppFixtureLiveStep {
        CheckFixture,
        CheckInterruptedRecovery,
        RetainCertificate,
        RetainNextVotes,
        CheckReproposal,
        CheckScheduledPillarVote,
    }
    #[cfg(feature = "production_integration_test")]
    struct AppFixtureLiveRequest {
        step: AppFixtureLiveStep,
        secret: [u8; 32],
        vrf_secret: [u8; 64],
        second_open: bool,
        transactions: Vec<AppFixtureTransaction>,
    }

    unsafe extern "C++" {
        include!("rustaxa/selected_production_config.hpp");
        type NativeSelectedProductionConfig = rustaxa_native_config::NativeSelectedProductionConfig;
        include!("rustaxa-bridge/src/ffi.rs.h");
        type BridgeConsensusApplication =
            crate::dag_transaction_service::BridgeConsensusApplication;
    }
    extern "Rust" {
        #[cfg(feature = "production_integration_test")]
        fn production_fixture_disk_step(
            source: UniquePtr<NativeSelectedProductionConfig>,
            foreign_source: UniquePtr<NativeSelectedProductionConfig>,
            request: AppFixtureDiskRequest,
        ) -> Result<AppFixtureDiskReport>;
        #[cfg(feature = "production_integration_test")]
        fn production_fixture_live_step(
            application: &BridgeConsensusApplication,
            source: UniquePtr<NativeSelectedProductionConfig>,
            request: AppFixtureLiveRequest,
        ) -> Result<String>;
        fn create_production_application(
            source: UniquePtr<NativeSelectedProductionConfig>,
            concrete_path: String,
        ) -> Result<Box<BridgeConsensusApplication>>;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_factory_rejects_missing_native_source() {
        let error = super::create_production_application(cxx::UniquePtr::null(), String::new())
            .err()
            .expect("missing native source must fail before storage access");
        assert!(error
            .to_string()
            .contains("PRODUCTION_SELECTED_CONFIG_SOURCE_UNAVAILABLE"));
    }
}
