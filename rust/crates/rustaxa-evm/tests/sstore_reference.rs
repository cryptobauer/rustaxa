//! Pinned Go SSTORE matrix intake.
//!
//! The full executable host-backed comparison is intentionally left to the
//! journal owner: this additive test preserves the independently generated
//! matrix and its exact historical values without asserting a new Rust policy.

use std::{borrow::Cow, collections::BTreeMap};

use revm::{
    bytecode::Bytecode,
    context_interface::{
        Host,
        cfg::GasParams,
        context::{SStoreResult, StateLoad},
        host::LoadError,
        journaled_state::AccountInfoLoad,
    },
    interpreter::{Gas, Interpreter, InterpreterAction},
    primitives::{Address, B256, Log, StorageKey, StorageValue, U256},
    state::AccountInfo,
};
use rustaxa_evm::profile::TaraxaProfile;

/// Test-only storage host with separately seeded original and current values.
struct SstoreHost {
    gas: GasParams,
    original: U256,
    slots: BTreeMap<U256, U256>,
}
impl SstoreHost {
    fn new(gas: GasParams, original: U256) -> Self {
        Self {
            gas,
            original,
            slots: BTreeMap::new(),
        }
    }
}
impl Host for SstoreHost {
    fn basefee(&self) -> U256 {
        U256::ZERO
    }
    fn blob_gasprice(&self) -> U256 {
        U256::ZERO
    }
    fn gas_limit(&self) -> U256 {
        U256::ZERO
    }
    fn difficulty(&self) -> U256 {
        U256::ZERO
    }
    fn prevrandao(&self) -> Option<U256> {
        None
    }
    fn block_number(&self) -> U256 {
        U256::ZERO
    }
    fn timestamp(&self) -> U256 {
        U256::ZERO
    }
    fn beneficiary(&self) -> Address {
        Address::ZERO
    }
    fn slot_num(&self) -> U256 {
        U256::ZERO
    }
    fn chain_id(&self) -> U256 {
        U256::ZERO
    }
    fn effective_gas_price(&self) -> U256 {
        U256::ZERO
    }
    fn caller(&self) -> Address {
        Address::ZERO
    }
    fn blob_hash(&self, _: usize) -> Option<U256> {
        None
    }
    fn max_initcode_size(&self) -> usize {
        0
    }
    fn gas_params(&self) -> &GasParams {
        &self.gas
    }
    fn is_amsterdam_eip8037_enabled(&self) -> bool {
        false
    }
    fn block_hash(&mut self, _: u64) -> Option<B256> {
        None
    }
    fn selfdestruct(
        &mut self,
        _: Address,
        _: Address,
        _: bool,
    ) -> Result<StateLoad<revm::context_interface::context::SelfDestructResult>, LoadError> {
        panic!("unused")
    }
    fn log(&mut self, _: Log) {}
    fn tstore(&mut self, _: Address, _: StorageKey, _: StorageValue) {}
    fn tload(&mut self, _: Address, _: StorageKey) -> StorageValue {
        U256::ZERO
    }
    fn load_account_info_skip_cold_load(
        &mut self,
        _: Address,
        _: bool,
        _: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        Ok(AccountInfoLoad {
            account: Cow::Owned(AccountInfo::default()),
            is_cold: false,
            is_empty: false,
        })
    }
    fn sstore_skip_cold_load(
        &mut self,
        _: Address,
        key: StorageKey,
        value: StorageValue,
        _: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        let present = self.slots.get(&key).copied().unwrap_or(self.original);
        self.slots.insert(key, value);
        Ok(StateLoad::new(
            SStoreResult {
                original_value: self.original,
                present_value: present,
                new_value: value,
            },
            false,
        ))
    }
    fn sload_skip_cold_load(
        &mut self,
        _: Address,
        key: StorageKey,
        _: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        Ok(StateLoad::new(
            self.slots.get(&key).copied().unwrap_or(self.original),
            false,
        ))
    }
}

/// Ensures both pinned Go exports agree and retain every requested SSTORE case.
#[test]
fn pinned_go_sstore_matrix_is_complete_and_identical() {
    let local: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_local.json"
    ))
    .expect("local SSTORE fixture JSON");
    let public: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_public.json"
    ))
    .expect("public SSTORE fixture JSON");
    assert_eq!(local, public, "pinned Go references must agree");
    let cases = local["sstore"].as_array().expect("SSTORE cases");
    assert_eq!(cases.len(), 12);
    for expected in [
        "clean0-to0",
        "clean0-to1",
        "dirty0-to1-to0",
        "dirty0-to1-to2",
        "clean7-to7",
        "clean7-to0",
        "clean7-to8",
        "dirty7-to8-to0",
        "dirty7-to8-to7",
        "sentry-2300",
        "sentry-2301",
        "static-rejection",
    ] {
        assert!(
            cases.iter().any(|row| row["case"] == expected),
            "missing {expected}"
        );
    }
}

/// Executes each non-static Go row through the profile with its original slot seed.
#[test]
fn profile_executes_seeded_sstore_rows_from_go() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/sstore_local.json"
    ))
    .unwrap();
    for row in fixtures["sstore"].as_array().unwrap() {
        if row["case"] == "static-rejection" {
            continue;
        }
        let profile = TaraxaProfile::new(false);
        let mut host = SstoreHost::new(
            profile.gas_params(),
            U256::from(row["original"].as_u64().unwrap()),
        );
        let (table, costs) = profile.instruction_table::<SstoreHost>();
        let code = hex::decode(row["code"].as_str().unwrap()).unwrap();
        let gas_cap = row["gas_cap"].as_u64().unwrap();
        let mut interpreter = Interpreter::default().with_bytecode(Bytecode::new_raw(code.into()));
        interpreter.gas = Gas::new(gas_cap - 21000);
        profile.configure_interpreter(&mut interpreter);
        let InterpreterAction::Return(mut result) =
            interpreter.run_plain(&table, &costs, &mut host)
        else {
            panic!("frame")
        };
        if !result.result.is_ok() {
            result.gas.spend_all()
        };
        let spent = gas_cap - result.gas.remaining();
        let refund = u64::try_from(result.gas.refunded()).unwrap();
        assert_eq!(
            spent - refund.min(spent / 2),
            row["gas_used"].as_u64().unwrap(),
            "{}",
            row["case"]
        );
        assert_eq!(refund, row["refund"].as_u64().unwrap(), "{}", row["case"]);
        assert_eq!(
            host.slots
                .get(&U256::ZERO)
                .copied()
                .unwrap_or(host.original)
                .to_string(),
            row["storage"].as_str().unwrap(),
            "{}",
            row["case"]
        );
    }
}
