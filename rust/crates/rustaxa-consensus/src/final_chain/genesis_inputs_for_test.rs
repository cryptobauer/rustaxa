use super::*;
use num_bigint::BigUint;
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteStateMutationBatch, ConcreteStorageMutation,
};
use rustaxa_types::{
    concrete_lifecycle::ConcreteStorageSlot,
    concrete_state::{
        ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord, ConcreteStorageKey,
    },
};
use std::collections::BTreeSet;

impl FinalChain {
    #[cfg(any(test, feature = "production_integration_test"))]
    pub(crate) fn genesis_snapshots_for_integration_test(
        &self,
    ) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            encode_dpos_snapshot_rlp(&self.dpos_snapshot_at_finalized_block(0.into())?)?,
            encode_account_snapshot_rlp(&self.account_snapshot_map_at_block(0.into())?),
        ))
    }

    pub(crate) fn concrete_genesis_inputs_for_test(
        &self,
    ) -> anyhow::Result<(ConcreteStateMutationBatch, Vec<ConcreteStorageSlot>)> {
        let native = self.dpos_snapshot_at_finalized_block(0.into())?;
        let account_map = self.account_snapshot_map_at_block(0.into())?;
        let addresses = account_map.keys().copied().collect::<BTreeSet<_>>();
        let accounts = account_map
            .into_iter()
            .map(|(address, account)| {
                let mut raw = rlp::RlpStream::new_list(5);
                raw.append(&account.nonce.to_bytes())
                    .append(account.balance.as_u256());
                raw.append_empty_data().append_empty_data().append(&0_u64);
                ConcreteAccountMutation::Upsert {
                    address,
                    record: ConcreteAccountRecord {
                        account: ConcreteAccount {
                            nonce: account.nonce,
                            balance: ConcreteAccountBalance::new(BigUint::from_bytes_be(
                                &account.balance.as_u256().to_big_endian(),
                            )),
                            storage_root: None,
                            code_hash: None,
                            code_size: 0,
                        },
                        physical_rlp: raw.out().to_vec(),
                    },
                }
            })
            .collect();
        let storage = canonical_concrete_precompile_storage(
            &native,
            self.rewards_config.magnolia_period.is_genesis(),
        )?
        .into_iter()
        .filter_map(|((address, key), values)| {
            let value = values.into_iter().next().unwrap();
            (addresses.contains(&address) && !value.is_empty()).then_some(ConcreteStorageMutation {
                address,
                key: ConcreteStorageKey(key),
                value: Some(value),
            })
        })
        .collect::<Vec<_>>();
        let mut catalog = storage
            .iter()
            .map(|row| ConcreteStorageSlot {
                address: row.address,
                key: row.key.0,
            })
            .collect::<Vec<_>>();
        catalog.sort_unstable();
        Ok((
            ConcreteStateMutationBatch {
                accounts,
                storage,
                ..Default::default()
            },
            catalog,
        ))
    }
}
