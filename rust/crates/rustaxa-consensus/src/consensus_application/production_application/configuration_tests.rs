use super::*;

fn historical() -> Result<crate::ExecutionConfiguration> {
    let bootstrap = super::super::tests::execution_configuration_bootstrap(PathBuf::from(
        "unused-pure-configuration",
    ));
    let input = &bootstrap.final_chain;
    let native = FinalChain::prepare_native_execution_configuration(
        input.block_gas_limit,
        input.genesis_timestamp,
        input.state_api_committed_root,
        true,
        input.genesis_accounts.clone(),
        input.genesis_validators.clone(),
        &input.genesis_dpos,
        input.rewards.clone(),
        bootstrap.consensus.pbft.ficus_activation_period.into(),
    )?
    .fixed(CorrectionStage::Historical);
    Ok(crate::ExecutionConfiguration::prepare(
        bootstrap.storage_genesis_hash,
        &native,
        &bootstrap.consensus,
        input.bridge_contract_address,
    )?)
}

#[test]
fn production_identity_contains_exact_historical_bytes_and_minimal_policy() -> Result<()> {
    let historical = historical()?;
    let old_bytes = historical.canonical_bytes().to_vec();
    let old_digest = historical.digest();
    let policy = ProductionAdmissionPolicy {
        transaction_max_gas: 0.into(),
        transaction_min_gas_price: U256::zero(),
    };
    let production = ProductionConfiguration::new(&historical, &policy);
    let record = Rlp::new(production.canonical_bytes());
    assert_eq!(record.item_count()?, 5);
    assert_eq!(
        record.val_at::<Vec<u8>>(0)?,
        b"rustaxa/production-configuration"
    );
    assert_eq!(record.at(1)?.as_raw(), &[1]);
    assert!(record.at(2)?.is_data());
    assert_eq!(record.val_at::<Vec<u8>>(2)?, old_bytes);
    assert_eq!(record.at(3)?.as_raw(), &[0x80]);
    assert_eq!(record.at(4)?.as_raw(), &[0x80]);
    assert_eq!(historical.canonical_bytes(), old_bytes);
    assert_eq!(historical.digest(), old_digest);
    assert_ne!(production.digest(), old_digest);
    Ok(())
}

#[test]
fn both_selected_network_limits_change_production_identity() -> Result<()> {
    let historical = historical()?;
    let policy = ProductionAdmissionPolicy {
        transaction_max_gas: 31_500_000.into(),
        transaction_min_gas_price: U256::one(),
    };
    let original = ProductionConfiguration::new(&historical, &policy);
    let gas = ProductionConfiguration::new(
        &historical,
        &ProductionAdmissionPolicy {
            transaction_max_gas: u64::MAX.into(),
            ..policy.clone()
        },
    );
    let price = ProductionConfiguration::new(
        &historical,
        &ProductionAdmissionPolicy {
            transaction_min_gas_price: U256::from(u64::MAX),
            ..policy
        },
    );
    assert_ne!(original.digest(), gas.digest());
    assert_ne!(original.digest(), price.digest());
    assert_ne!(gas.digest(), price.digest());
    assert_eq!(Rlp::new(gas.canonical_bytes()).val_at::<u64>(3)?, u64::MAX);
    assert_eq!(
        Rlp::new(price.canonical_bytes()).val_at::<U256>(4)?,
        U256::from(u64::MAX)
    );
    let schedule = |digest| PbftExecutionSchedule::new(1, [1; 32], digest, None);
    assert_ne!(schedule(original.digest()), schedule(gas.digest()));
    assert_ne!(schedule(original.digest()), schedule(price.digest()));
    Ok(())
}
