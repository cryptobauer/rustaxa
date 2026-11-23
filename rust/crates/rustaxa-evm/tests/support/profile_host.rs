//! Fail-closed interpreter probe host. Only gas parameters and GASPRICE are
//! implemented by default; opt-in fixture state supplies empty-origin slots and
//! one fixed native-account load. Unexpected access aborts the experiment.
//! Host signatures follow REVM v117 (MIT), without adopting its dummy state.
use revm::context_interface::{
    Host,
    cfg::GasParams,
    context::{SStoreResult, SelfDestructResult, StateLoad},
    host::LoadError,
    journaled_state::AccountInfoLoad,
};
use revm::primitives::{Address, B256, Log, StorageKey, StorageValue, U256};

/// Test-only host with an authoritative full-width gas price and explicit gas profile.
pub(crate) struct ProbeHost {
    pub(crate) price: U256,
    pub(crate) gas: GasParams,
    pub(crate) native_load: bool,
    pub(crate) slots: Option<std::collections::BTreeMap<U256, U256>>,
    pub(crate) transient: Option<std::collections::BTreeMap<U256, U256>>,
}
impl Host for ProbeHost {
    fn basefee(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn blob_gasprice(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn gas_limit(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn gas_params(&self) -> &GasParams {
        &self.gas
    }

    fn is_amsterdam_eip8037_enabled(&self) -> bool {
        false
    }

    fn difficulty(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn prevrandao(&self) -> Option<U256> {
        panic!("unimplemented probe host operation")
    }

    fn block_number(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn timestamp(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn beneficiary(&self) -> Address {
        panic!("unimplemented probe host operation")
    }

    fn slot_num(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn chain_id(&self) -> U256 {
        panic!("unimplemented probe host operation")
    }

    fn effective_gas_price(&self) -> U256 {
        self.price
    }

    fn caller(&self) -> Address {
        panic!("unimplemented probe host operation")
    }

    fn blob_hash(&self, _number: usize) -> Option<U256> {
        panic!("unimplemented probe host operation")
    }

    fn max_initcode_size(&self) -> usize {
        panic!("unimplemented probe host operation")
    }

    fn block_hash(&mut self, _number: u64) -> Option<B256> {
        panic!("unimplemented probe host operation")
    }

    fn selfdestruct(
        &mut self,
        _address: Address,
        _target: Address,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SelfDestructResult>, LoadError> {
        panic!("unimplemented probe host operation")
    }

    fn log(&mut self, _log: Log) {
        panic!("unimplemented probe host operation")
    }

    fn tstore(&mut self, _address: Address, key: StorageKey, value: StorageValue) {
        self.transient
            .as_mut()
            .expect("unexpected transient write")
            .insert(key, value);
    }

    fn tload(&mut self, _address: Address, key: StorageKey) -> StorageValue {
        self.transient
            .as_ref()
            .expect("unexpected transient read")
            .get(&key)
            .copied()
            .unwrap_or_default()
    }

    fn load_account_info_skip_cold_load(
        &mut self,
        _address: Address,
        _load_code: bool,
        _skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        assert!(
            self.native_load && _address == Address::with_last_byte(0xfe),
            "unexpected account load"
        );
        Ok(AccountInfoLoad {
            account: std::borrow::Cow::Owned(revm::state::AccountInfo {
                nonce: 1,
                balance: U256::from(10000),
                ..Default::default()
            }),
            is_cold: false,
            is_empty: false,
        })
    }

    fn sstore_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _value: StorageValue,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        let slots = self.slots.as_mut().expect("unexpected storage write");
        let previous = slots.insert(_key, _value).unwrap_or_default();
        Ok(StateLoad::new(
            SStoreResult {
                original_value: U256::ZERO,
                present_value: previous,
                new_value: _value,
            },
            false,
        ))
    }

    fn sload_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        Ok(StateLoad::new(
            self.slots
                .as_ref()
                .expect("unexpected storage read")
                .get(&_key)
                .copied()
                .unwrap_or_default(),
            false,
        ))
    }
}
