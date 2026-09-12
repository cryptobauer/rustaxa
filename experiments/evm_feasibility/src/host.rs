//! Fail-closed interpreter probe host. Only gas parameters and GASPRICE are
//! implemented; unexpected state/environment access aborts the experiment.
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

    fn tstore(&mut self, _address: Address, _key: StorageKey, _value: StorageValue) {
        panic!("unimplemented probe host operation")
    }

    fn tload(&mut self, _address: Address, _key: StorageKey) -> StorageValue {
        panic!("unimplemented probe host operation")
    }

    fn load_account_info_skip_cold_load(
        &mut self,
        _address: Address,
        _load_code: bool,
        _skip_cold_load: bool,
    ) -> Result<AccountInfoLoad<'_>, LoadError> {
        panic!("unimplemented probe host operation")
    }

    fn sstore_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _value: StorageValue,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<SStoreResult>, LoadError> {
        panic!("unimplemented probe host operation")
    }

    fn sload_skip_cold_load(
        &mut self,
        _address: Address,
        _key: StorageKey,
        _skip_cold_load: bool,
    ) -> Result<StateLoad<StorageValue>, LoadError> {
        panic!("unimplemented probe host operation")
    }
}
