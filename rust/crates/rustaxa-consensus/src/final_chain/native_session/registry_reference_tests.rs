//! Owner configuration controls registry membership without account changes.
use super::*;
use std::sync::Arc;

#[test]
fn native_registry_uses_owner_magnolia_boundary_independent_of_accounts() {
    let path = crate::closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!("rustaxa-native-registry-{}", std::process::id())),
        true,
    );
    for activation in [0_u64, 7, u64::MAX] {
        let directory = path.join(activation.to_string());
        let storage = Arc::new(Storage::new(rustaxa_storage::Config::new(directory)).unwrap());
        let chain = FinalChain::new_with_rewards_config(
            storage,
            10_000_000.into(),
            0,
            Vec::new(),
            Vec::new(),
            GenesisDposConfig::default(),
            FinalChainRewardsConfig {
                magnolia_period: activation.into(),
                ..Default::default()
            },
        )
        .unwrap();
        let before = chain.account(SLASHING_CONTRACT_ADDRESS).unwrap();
        for period in [0_u64, 6, 7, 8, u64::MAX] {
            let period = FinalChainBlockNumber::new(period);
            assert_eq!(
                chain.native_contract_kind(period, DPOS_CONTRACT_ADDRESS),
                Some(FinalChainNativeContractKind::Dpos)
            );
            assert_eq!(
                chain.native_contract_kind(period, SLASHING_CONTRACT_ADDRESS),
                (period.as_u64() >= activation).then_some(FinalChainNativeContractKind::Slashing)
            );
            assert_eq!(chain.native_contract_kind(period, [1; 20]), None);
        }
        assert_eq!(chain.account(SLASHING_CONTRACT_ADDRESS).unwrap(), before);
        assert_eq!(
            chain.last_block_number_typed().unwrap(),
            FinalChainBlockNumber::GENESIS
        );
        // With Cornus active, the old compatibility constructor supports genesis
        // simulation. The unregistered address must still fail request admission.
        let simulation = chain
            .begin_native_simulation(FinalChainBlockNumber::GENESIS)
            .unwrap();
        let request = FinalChainNativeRequest {
            id: FinalChainNativeInvocationId {
                transaction: FinalChainTransactionPosition::new(0),
                sequence: 0,
            },
            period: FinalChainBlockNumber::GENESIS,
            depth: 0,
            kind: FinalChainNativeCallKind::Call,
            is_static: false,
            caller: [1; 20],
            contract: SLASHING_CONTRACT_ADDRESS,
            state_address: SLASHING_CONTRACT_ADDRESS,
            value: FinalChainNativeValue::default(),
            input: Vec::new(),
            supplied_gas: 100_000.into(),
        };
        let mut wrong_period = request.clone();
        wrong_period.period = FinalChainBlockNumber::new(1);
        assert!(matches!(
            simulation.session.validate_request(&wrong_period),
            Err(FinalChainNativeSessionError::PeriodMismatch)
        ));
        let validation = simulation.session.validate_request(&request);
        if activation == 0 {
            assert!(validation.is_ok());
        } else {
            assert!(matches!(
                validation,
                Err(FinalChainNativeSessionError::UnsupportedAddress)
            ));
        }
        drop(simulation);
        drop(chain);
    }
    crate::closeout_test_scratch::dispose(&path).unwrap();
}
