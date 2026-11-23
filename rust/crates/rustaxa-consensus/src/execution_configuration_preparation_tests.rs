fn preparation_fixture() -> NativeConfigurationSeed {
    let (mut seed, _) = fixture();
    seed.rewards.aspen_max_supply = amount(1_000);
    seed.input.dpos.validator_maximum_stake = amount(100);
    seed.input.original_sum = seed.rewards.genesis_balance_sum;
    seed
}

fn prepare_seed(
    seed: &NativeConfigurationSeed,
    accounts: Vec<rustaxa_types::GenesisAccount>,
) -> anyhow::Result<NativeConfigurationSeed> {
    FinalChain::prepare_native_execution_configuration(
        seed.input.block_gas_limit,
        seed.input.genesis_timestamp,
        seed.input.genesis_root,
        seed.input.enforce,
        accounts,
        seed.input.validators.clone(),
        &seed.input.dpos,
        seed.rewards.clone(),
        seed.input.ficus,
    )
}

fn preparation_accounts(seed: &NativeConfigurationSeed) -> Vec<rustaxa_types::GenesisAccount> {
    seed.input
        .accounts
        .iter()
        .map(|(address, balance)| rustaxa_types::GenesisAccount {
            address: *address,
            balance: rustaxa_types::FinalChainAccountBalance::from_snapshot_bytes(balance).unwrap(),
        })
        .collect()
}

#[test]
fn pure_preparation_preserves_existing_configuration_encodings() {
    let expected = preparation_fixture();
    let prepared = prepare_seed(&expected, preparation_accounts(&expected)).unwrap();
    assert_eq!(prepared, expected);
    for stage in [
        CorrectionStage::Historical,
        CorrectionStage::TransientRollback,
        CorrectionStage::SingleSelfDestructRefund,
        CorrectionStage::CheckedReturnDataCopy,
        CorrectionStage::BoundedFalconAbi,
        CorrectionStage::SlashingHardError,
    ] {
        assert_eq!(prepared.fixed(stage), expected.fixed(stage));
    }
    for activation in [None, Some(0.into()), Some(123.into())] {
        assert_eq!(
            prepared.activation(activation),
            expected.activation(activation)
        );
    }
    assert_ne!(
        prepared
            .fixed(CorrectionStage::Historical)
            .canonical_bytes(),
        prepared.activation(None).canonical_bytes()
    );
}

#[test]
fn pure_preparation_preserves_last_account_and_escrow_supply_normalization() {
    let mut input = preparation_fixture();
    input.rewards.genesis_balance_sum = None;
    let mut accounts = preparation_accounts(&input);
    accounts.insert(
        0,
        rustaxa_types::GenesisAccount {
            address: [1; 20],
            balance: rustaxa_types::FinalChainAccountBalance::new_account(99.into()),
        },
    );
    let prepared = prepare_seed(&input, accounts).unwrap();
    let mut expected = input;
    expected.input.original_sum = None;
    // Effective accounts hold two tokens; validator escrow adds six tokens.
    expected.rewards.genesis_balance_sum = Some(amount(8));
    assert_eq!(prepared, expected);
}

#[test]
fn pure_preparation_rejects_invalid_validator_and_reward_inputs() {
    let input = preparation_fixture();
    let mut invalid = input.clone();
    invalid.input.validators[0].total_stake = vec![1; 33];
    assert_eq!(
        prepare_seed(&invalid, preparation_accounts(&invalid))
            .unwrap_err()
            .to_string(),
        "genesis DPoS validator stake encoding exceeds 32 bytes"
    );

    let mut invalid = input.clone();
    invalid.input.validators[0].total_stake = vec![101];
    assert_eq!(
        prepare_seed(&invalid, preparation_accounts(&invalid))
            .unwrap_err()
            .to_string(),
        "genesis DPoS validator stake exceeds maximum stake"
    );

    let mut invalid = input.clone();
    invalid.rewards.aspen_part_two_period = 3.into();
    assert_eq!(
        prepare_seed(&invalid, preparation_accounts(&invalid))
            .unwrap_err()
            .to_string(),
        "FINAL_CHAIN_DPOS_ASPEN_PART_TWO_BEFORE_PART_ONE"
    );

    let mut invalid = input;
    invalid.rewards.aspen_max_supply = amount(1);
    assert_eq!(
        prepare_seed(&invalid, preparation_accounts(&invalid))
            .unwrap_err()
            .to_string(),
        "FINAL_CHAIN_DPOS_ASPEN_CONFIGURED_SUPPLY_EXCEEDS_MAXIMUM"
    );
}

#[test]
fn pure_preparation_rejects_escrow_overflow() {
    let input = preparation_fixture();
    let mut accounts = preparation_accounts(&input);
    accounts.push(rustaxa_types::GenesisAccount {
        address: {
            let mut address = [0; 20];
            address[19] = 0xfe;
            address
        },
        balance: rustaxa_types::FinalChainAccountBalance::new_account(ethereum_types::U256::MAX),
    });
    assert_eq!(
        prepare_seed(&input, accounts).unwrap_err().to_string(),
        "genesis DPoS contract balance overflow"
    );
}

#[test]
fn pure_application_preparation_uses_existing_encoder_and_conflict_checks() {
    let (seed, config) = fixture();
    let native = seed.fixed(CorrectionStage::Historical);
    let expected = ExecutionConfiguration::compose(H256([10; 32]), &native, &config, [17; 20]);
    let prepared = ExecutionConfiguration::prepare(H256([10; 32]), &native, &config, [17; 20]);
    assert_eq!(prepared, expected);
    assert!(prepared.is_ok());
    let mut invalid = config;
    invalid.pbft.network_identity.genesis_hash = [99; 32];
    let expected = ExecutionConfiguration::compose(H256([10; 32]), &native, &invalid, [17; 20]);
    let prepared = ExecutionConfiguration::prepare(H256([10; 32]), &native, &invalid, [17; 20]);
    assert_eq!(prepared, expected);
    assert!(prepared.is_err());
}
