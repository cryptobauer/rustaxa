//! Owner-bound logical proof reads for pending-period eligibility queries.
use anyhow::{Result, ensure};
use ethereum_types::{H160, U256};
use rlp::Rlp;
use rustaxa_storage::{ConcreteCheckpointReaders, ConcreteStoragePath};
use rustaxa_types::concrete_state::{
    ConcreteAccountRecord, ConcreteRead, ConcreteStateIdentity, ConcreteStorageKey,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;
use tiny_keccak::{Hasher, Keccak};

const FE: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
];
const EE: [u8; 20] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
];

type VerifiedPaths = BTreeMap<([u8; 20], [u8; 32]), VerifiedPath>;

/// Only the owning session may create this from a header-bound checkpoint.
pub struct FrozenEligibility {
    reader: Arc<ConcreteCheckpointReaders>,
    pub(super) identity: ConcreteStateIdentity,
    pub(super) request_id: [u8; 32],
    pub(super) epoch: u64,
    pub(super) generation: u64,
    pub(super) parent: u64,
    pub(super) pending: u64,
    pub(super) basis: super::reward_scheduler::FinalChainRewardSchedulerBasis,
    cache: RefCell<VerifiedPaths>,
}

#[derive(Clone)]
struct VerifiedPath {
    account: ConcreteRead<ConcreteAccountRecord>,
    path: ConcreteStoragePath,
}

impl FrozenEligibility {
    pub(crate) fn validate_binding(
        &self,
        request_id: [u8; 32],
        epoch: u64,
        generation: u64,
        parent: u64,
        pending: u64,
    ) -> Result<()> {
        ensure!(
            self.request_id == request_id
                && self.epoch == epoch
                && self.generation == generation
                && self.parent == parent
                && self.pending == pending,
            "FINAL_CHAIN_DELAYED_VIEW_BINDING_MISMATCH"
        );
        Ok(())
    }
    pub(super) fn from_owner(
        reader: Arc<ConcreteCheckpointReaders>,
        identity: ConcreteStateIdentity,
        request_id: [u8; 32],
        epoch_generation: (u64, u64),
        parent: u64,
        pending: u64,
        basis: super::reward_scheduler::FinalChainRewardSchedulerBasis,
    ) -> Result<Self> {
        let (epoch, generation) = epoch_generation;
        ensure!(
            reader.retained_identities().contains(&identity),
            "FINAL_CHAIN_DELAYED_IDENTITY_MISSING"
        );
        Ok(Self {
            reader,
            identity,
            request_id,
            epoch,
            generation,
            parent,
            pending,
            basis,
            cache: RefCell::new(BTreeMap::new()),
        })
    }

    fn slot(&self, address: [u8; 20], key: [u8; 32]) -> Result<Option<Vec<u8>>> {
        if let Some(value) = self.cache.borrow().get(&(address, key)) {
            return value.logical();
        }
        let value = VerifiedPath {
            account: self.reader.account_at(self.identity, address)?,
            path: self.reader.verify_storage_path_at(
                self.identity,
                address,
                ConcreteStorageKey(key),
            )?,
        };
        let logical = value.logical()?;
        self.cache.borrow_mut().insert((address, key), value);
        Ok(logical)
    }

    pub(crate) fn jail_block(&self, validator: [u8; 20]) -> Result<Option<u64>> {
        self.slot(EE, hash(&[&[0], &validator]))?
            .map(|bytes| Ok(exact(&bytes)?.as_val::<u64>()?))
            .transpose()
    }

    pub(crate) fn jailed_validators(&self) -> Result<Vec<[u8; 20]>> {
        let mut key = [0; 32];
        key[31] = 2;
        match self.slot(EE, key)? {
            None => Ok(Vec::new()),
            Some(bytes) => decode_jailed_validators(&bytes),
        }
    }

    fn stake(&self, validator: [u8; 20]) -> Result<U256> {
        let Some(bytes) = self.slot(FE, hash(&[&[0, 0], &validator]))? else {
            return Ok(U256::zero());
        };
        ensure!(
            bytes.len() <= 4096,
            "FINAL_CHAIN_DELAYED_VALIDATOR_SIZE_LIMIT"
        );
        let row = exact(&bytes)?;
        let base = match row.item_count()? {
            2 => {
                row.val_at::<u16>(1)?;
                row.at(0)?
            }
            4 => row,
            _ => anyhow::bail!("FINAL_CHAIN_DELAYED_VALIDATOR_LAYOUT"),
        };
        ensure!(
            base.item_count()? == 4,
            "FINAL_CHAIN_DELAYED_VALIDATOR_LAYOUT"
        );
        base.val_at::<u16>(1)?;
        base.val_at::<u64>(2)?;
        base.val_at::<u64>(3)?;
        Ok(base.val_at(0)?)
    }

    fn jail(&self, validator: [u8; 20], period: u64, cacti: bool, magnolia: bool) -> Result<bool> {
        if !cacti || !magnolia {
            return Ok(false);
        }
        let Some(bytes) = self.slot(EE, hash(&[&[0], &validator]))? else {
            return Ok(false);
        };
        Ok(exact(&bytes)?.as_val::<u64>()? >= period)
    }

    pub(crate) fn eligible(&self, validator: [u8; 20], policy: &EligibilityPolicy) -> Result<bool> {
        if self.jail(validator, policy.period, policy.cacti, policy.magnolia)? {
            return Ok(false);
        }
        Ok(self.stake(validator)? >= policy.threshold)
    }

    pub(crate) fn validator_votes(
        &self,
        validator: [u8; 20],
        policy: &EligibilityPolicy,
    ) -> Result<u64> {
        if self.jail(validator, policy.period, policy.cacti, policy.magnolia)? {
            return Ok(0);
        }
        votes(self.stake(validator)?, policy)
    }

    pub(crate) fn total_votes(&self, policy: &EligibilityPolicy) -> Result<u64> {
        let raw = self.slot(FE, hash(&[&[4]]))?.unwrap_or_default();
        ensure!(raw.len() <= 8, "FINAL_CHAIN_DELAYED_TOTAL_WIDTH");
        let mut total = raw.iter().fold(0_u64, |n, b| (n << 8) | u64::from(*b));
        let mut key = [0; 32];
        key[31] = 2;
        if let Some(list) = self.slot(EE, key)? {
            ensure!(
                list.len() <= 1_048_576,
                "FINAL_CHAIN_DELAYED_JAIL_LIST_SIZE_LIMIT"
            );
            let list = exact(&list)?;
            ensure!(list.is_list(), "FINAL_CHAIN_DELAYED_JAIL_LIST");
            let count = list.item_count()?;
            ensure!(count <= 10_000, "FINAL_CHAIN_DELAYED_JAIL_LIST_LIMIT");
            for i in 0..count {
                let address = list.val_at::<H160>(i)?.0;
                total = total
                    .checked_sub(votes(self.stake(address)?, policy)?)
                    .ok_or_else(|| anyhow::anyhow!("FINAL_CHAIN_DELAYED_TOTAL_UNDERFLOW"))?;
            }
        }
        Ok(total)
    }
}

impl VerifiedPath {
    fn logical(&self) -> Result<Option<Vec<u8>>> {
        match (&self.account, &self.path) {
            (ConcreteRead::Present(_), ConcreteStoragePath::Member(bytes)) => {
                Ok(Some(bytes.clone()))
            }
            (_, ConcreteStoragePath::NonMember) => Ok(None),
            _ => anyhow::bail!("FINAL_CHAIN_DELAYED_ACCOUNT_PROOF_MISMATCH"),
        }
    }
}

pub(crate) struct EligibilityPolicy {
    pub period: u64,
    pub cacti: bool,
    pub magnolia: bool,
    pub aspen: bool,
    pub threshold: U256,
    pub step: U256,
    pub maximum: U256,
}

fn votes(stake: U256, policy: &EligibilityPolicy) -> Result<u64> {
    ensure!(
        !policy.step.is_zero() && (policy.maximum.is_zero() || policy.threshold <= policy.maximum),
        "FINAL_CHAIN_DELAYED_POLICY"
    );
    ensure!(
        policy.maximum.is_zero() || stake <= policy.maximum,
        "FINAL_CHAIN_DELAYED_STAKE_EXCEEDS_MAXIMUM"
    );
    let mut value = U256::zero();
    if policy.aspen && stake >= policy.maximum {
        value = policy.maximum / policy.step;
    }
    if stake >= policy.threshold {
        value = stake / policy.step;
    }
    ensure!(
        value <= U256::from(u64::MAX),
        "FINAL_CHAIN_DELAYED_VOTE_OVERFLOW"
    );
    Ok(value.as_u64())
}

fn decode_jailed_validators(bytes: &[u8]) -> Result<Vec<[u8; 20]>> {
    ensure!(
        bytes.len() <= 1_048_576,
        "FINAL_CHAIN_DELAYED_JAIL_LIST_SIZE_LIMIT"
    );
    let list = exact(bytes)?;
    ensure!(list.is_list(), "FINAL_CHAIN_DELAYED_JAIL_LIST");
    let count = list.item_count()?;
    ensure!(count <= 10_000, "FINAL_CHAIN_DELAYED_JAIL_LIST_LIMIT");
    (0..count).map(|i| Ok(list.val_at::<H160>(i)?.0)).collect()
}

fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Keccak::v256();
    for part in parts {
        h.update(part);
    }
    let mut result = [0; 32];
    h.finalize(&mut result);
    result
}

fn exact(bytes: &[u8]) -> Result<Rlp<'_>> {
    let rlp = Rlp::new(bytes);
    ensure!(
        rlp.payload_info()?.total() == bytes.len(),
        "FINAL_CHAIN_DELAYED_RLP_TRAILING"
    );
    Ok(rlp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_proof_preserves_account_absence_and_tombstone() {
        for account in [ConcreteRead::Absent, ConcreteRead::Tombstone] {
            let path = VerifiedPath {
                account: account.clone(),
                path: ConcreteStoragePath::NonMember,
            };
            assert_eq!(path.logical().unwrap(), None);
            let forged = VerifiedPath {
                account,
                path: ConcreteStoragePath::Member(vec![1]),
            };
            assert!(forged.logical().is_err());
        }
    }

    #[test]
    fn physical_vote_rule_keeps_threshold_rounding_and_aspen_assignment() {
        let p = EligibilityPolicy {
            period: 5,
            cacti: true,
            magnolia: true,
            aspen: true,
            threshold: 100.into(),
            step: 30.into(),
            maximum: 150.into(),
        };
        assert_eq!(votes(99.into(), &p).unwrap(), 0);
        assert_eq!(votes(100.into(), &p).unwrap(), 3);
        assert_eq!(votes(120.into(), &p).unwrap(), 4);
        assert_eq!(votes(150.into(), &p).unwrap(), 5);
        assert!(votes(180.into(), &p).is_err());
    }

    #[test]
    fn physical_vote_rule_rejects_bad_policy_and_overflow() {
        let mut p = EligibilityPolicy {
            period: 0,
            cacti: false,
            magnolia: false,
            aspen: false,
            threshold: 1.into(),
            step: 1.into(),
            maximum: U256::MAX,
        };
        assert!(votes(U256::MAX, &p).is_err());
        p.step = U256::zero();
        assert!(votes(1.into(), &p).is_err());
        p.step = 1.into();
        p.threshold = 2.into();
        p.maximum = 1.into();
        assert!(votes(2.into(), &p).is_err());
    }

    #[test]
    fn checkpoint_nonmembership_cache_and_reopen_keep_exact_identity() -> Result<()> {
        use rustaxa_storage::{
            ConcreteCheckpointReaders, ConcreteStateLifecycle, ConcreteStateMutationBatch,
        };
        let path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!("frozen-eligibility-{}", std::process::id())),
            false,
        );
        let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path,
            [7; 32],
            ConcreteStateMutationBatch::default(),
            vec![],
        )?;
        crate::closeout_test_scratch::created(&path);
        let identity = lifecycle.prior_reader().identity();
        let reader = Arc::new(lifecycle.open_retained_checkpoint(identity)?);
        let mut scheduler =
            super::super::reward_scheduler::FinalChainRewardSchedulerRuntime::new(0.into());
        scheduler.authorize_joint_startup(3, 0.into())?;
        scheduler.complete_recovery(3)?;
        let basis = scheduler.basis(3, 0.into(), 1.into(), [1; 32])?;
        let bad = ConcreteStateIdentity {
            period: identity.period,
            state_root: [9; 32],
        };
        assert!(
            FrozenEligibility::from_owner(Arc::clone(&reader), bad, [1; 32], (3, 1), 0, 1, basis)
                .is_err()
        );
        let view = FrozenEligibility::from_owner(
            Arc::clone(&reader),
            identity,
            [1; 32],
            (3, 1),
            0,
            1,
            basis,
        )?;
        let mut unavailable = FrozenEligibility::from_owner(
            Arc::clone(&reader),
            identity,
            [1; 32],
            (3, 1),
            0,
            1,
            basis,
        )?;
        unavailable.identity = bad;
        assert!(unavailable.jail_block([8; 20]).is_err());
        assert!(unavailable.jailed_validators().is_err());
        drop(unavailable);
        let policy = EligibilityPolicy {
            period: 0,
            cacti: true,
            magnolia: true,
            aspen: false,
            threshold: 100.into(),
            step: 10.into(),
            maximum: 1000.into(),
        };
        assert!(!view.eligible([8; 20], &policy)?);
        assert_eq!(view.jail_block([8; 20])?, None);
        assert!(view.jailed_validators()?.is_empty());
        let count = view.cache.borrow().len();
        assert!(!view.eligible([8; 20], &policy)?);
        assert_eq!(view.cache.borrow().len(), count);
        assert_eq!(view.total_votes(&policy)?, 0);
        assert!(view.validate_binding([2; 32], 3, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 4, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 2, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 1, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 0, 2).is_err());
        drop(view);
        drop(reader);
        drop(lifecycle);
        let reader = Arc::new(ConcreteCheckpointReaders::open_read_only(
            &path,
            identity,
            [identity],
        )?);
        let reopened =
            FrozenEligibility::from_owner(reader, identity, [1; 32], (3, 1), 0, 1, basis)?;
        assert!(!reopened.eligible([8; 20], &policy)?);
        assert_eq!(reopened.jail_block([8; 20])?, None);
        assert!(reopened.jailed_validators()?.is_empty());
        drop(reopened);
        crate::closeout_test_scratch::dispose(path)?;
        Ok(())
    }

    #[test]
    fn jail_getter_list_preserves_duplicates_and_rejects_invalid_rlp() {
        let mut list = rlp::RlpStream::new_list(3);
        list.append(&H160([8; 20]));
        list.append(&H160([9; 20]));
        list.append(&H160([8; 20]));
        let bytes = list.out().to_vec();
        assert_eq!(
            decode_jailed_validators(&bytes).unwrap(),
            vec![[8; 20], [9; 20], [8; 20]]
        );
        assert!(decode_jailed_validators(&[0xc0]).unwrap().is_empty());
        assert!(decode_jailed_validators(&[0xc0, 0]).is_err());
        assert!(decode_jailed_validators(&[0xc1, 0x01]).is_err());
        assert!(decode_jailed_validators(&vec![0; 1_048_577]).is_err());
    }

    #[test]
    fn checkpoint_total_keeps_expired_duplicate_list_entries() -> Result<()> {
        use rustaxa_storage::{
            ConcreteAccountMutation, ConcreteStateLifecycle, ConcreteStateMutationBatch,
            ConcreteStorageMutation,
        };
        use rustaxa_types::FinalChainNonce;
        use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
        use rustaxa_types::concrete_state::{
            ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord,
        };
        let path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!("frozen-eligibility-total-{}", std::process::id())),
            false,
        );
        let record = || {
            let mut r = rlp::RlpStream::new_list(5);
            r.append_empty_data();
            r.append_empty_data();
            r.append_empty_data();
            r.append_empty_data();
            r.append(&0_u64);
            ConcreteAccountRecord {
                account: ConcreteAccount {
                    nonce: FinalChainNonce::zero(),
                    balance: ConcreteAccountBalance::new(Default::default()),
                    storage_root: None,
                    code_hash: None,
                    code_size: 0,
                },
                physical_rlp: r.out().to_vec(),
            }
        };
        let validator = [8; 20];
        let stake_key = ConcreteStorageKey(hash(&[&[0, 0], &validator]));
        let jail_key = ConcreteStorageKey(hash(&[&[0], &validator]));
        let total_key = ConcreteStorageKey(hash(&[&[4]]));
        let mut list_key = [0; 32];
        list_key[31] = 2;
        let list_key = ConcreteStorageKey(list_key);
        let mut stake = rlp::RlpStream::new_list(4);
        stake.append(&U256::from(150));
        stake.append(&0_u16);
        stake.append(&0_u64);
        stake.append(&0_u64);
        let mut list = rlp::RlpStream::new_list(2);
        list.append(&H160(validator));
        list.append(&H160(validator));
        let rows = vec![
            (FE, stake_key, stake.out().to_vec()),
            (FE, total_key, vec![10]),
            (EE, jail_key, rlp::encode(&1_u64).to_vec()),
            (
                EE,
                ConcreteStorageKey(hash(&[&[0], &[9; 20]])),
                rlp::encode(&0_u64).to_vec(),
            ),
            (EE, list_key, list.out().to_vec()),
        ];
        let mut catalog: Vec<ConcreteStorageSlot> = rows
            .iter()
            .map(|(address, key, _)| ConcreteStorageSlot {
                address: *address,
                key: key.0,
            })
            .collect();
        catalog.sort_unstable();
        assert_eq!(catalog.len(), 5);
        assert!(catalog.windows(2).all(|pair| pair[0] < pair[1]));
        let storage = rows
            .into_iter()
            .map(|(address, key, value)| ConcreteStorageMutation {
                address,
                key,
                value: Some(value),
            })
            .collect();
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path,
            [7; 32],
            ConcreteStateMutationBatch {
                accounts: vec![
                    ConcreteAccountMutation::Upsert {
                        address: FE,
                        record: record(),
                    },
                    ConcreteAccountMutation::Upsert {
                        address: EE,
                        record: record(),
                    },
                ],
                storage,
                ..Default::default()
            },
            catalog.clone(),
        )?;
        crate::closeout_test_scratch::created(&path);
        let identity = lifecycle.prior_reader().identity();
        let reader = Arc::new(lifecycle.open_retained_checkpoint(identity)?);
        let mut scheduler =
            super::super::reward_scheduler::FinalChainRewardSchedulerRuntime::new(0.into());
        scheduler.authorize_joint_startup(3, 0.into())?;
        scheduler.complete_recovery(3)?;
        let basis = scheduler.basis(3, 0.into(), 1.into(), [1; 32])?;
        let view = FrozenEligibility::from_owner(reader, identity, [1; 32], (3, 1), 0, 1, basis)?;
        let mut policy = EligibilityPolicy {
            period: 5,
            cacti: true,
            magnolia: true,
            aspen: true,
            threshold: 100.into(),
            step: 30.into(),
            maximum: 1000.into(),
        };
        assert_eq!(view.jail_block(validator)?, Some(1));
        assert_eq!(view.jail_block([9; 20])?, Some(0));
        assert_eq!(view.jail_block([10; 20])?, None);
        assert_eq!(view.jailed_validators()?, vec![validator, validator]);
        assert_eq!(view.jail_block(validator)?, Some(1));
        assert_eq!(view.jailed_validators()?, vec![validator, validator]);
        assert!(view.validate_binding([1; 32], 3, 1, 0, 1).is_ok());
        assert!(view.validate_binding([2; 32], 3, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 4, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 2, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 1, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 0, 2).is_err());
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.validator_votes(validator, &policy)?, 5);
        assert_eq!(view.total_votes(&policy)?, 0);
        let cached_path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!("cached-eligibility-total-{}", std::process::id())),
            false,
        );
        let cached_storage = Arc::new(rustaxa_storage::Storage::new(
            rustaxa_storage::Config::new(cached_path.clone()),
        )?);
        crate::closeout_test_scratch::created(&cached_path);
        let cached = crate::final_chain::FinalChain::new_with_rewards_config(
            Arc::clone(&cached_storage),
            1_000_000.into(),
            0,
            vec![],
            vec![],
            rustaxa_types::GenesisDposConfig::default(),
            rustaxa_types::FinalChainRewardsConfig::default(),
        )?;
        let mut cached_snapshot = cached.dpos_snapshot(0.into())?;
        cached_snapshot.total_vote_count = 10;
        cached_snapshot.vote_counts.insert(validator, 5);
        cached_snapshot.slashing_jail_blocks.insert(validator, 1);
        cached_snapshot.slashing_jailed_validators = vec![validator, validator];
        assert_eq!(
            cached.dpos_effective_total_vote_count(&cached_snapshot, policy.period.into())?,
            view.total_votes(&policy)?,
        );
        drop(cached);
        drop(cached_storage);
        let _ = crate::closeout_test_scratch::dispose(cached_path);
        assert!(!view.eligible([9; 20], &policy)?);
        assert_eq!(view.validator_votes([9; 20], &policy)?, 0);
        policy.period = 0;
        assert!(!view.eligible(validator, &policy)?);
        policy.period = 1;
        assert!(!view.eligible(validator, &policy)?);
        assert_eq!(view.validator_votes(validator, &policy)?, 0);
        assert_eq!(view.total_votes(&policy)?, 0);
        policy.cacti = false;
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.total_votes(&policy)?, 0);
        policy.cacti = true;
        policy.magnolia = false;
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.total_votes(&policy)?, 0);
        policy.step = 10.into();
        assert!(view.total_votes(&policy).is_err());
        let observed = lifecycle.observation()?;
        let marker = rustaxa_types::concrete_lifecycle::FinalChainConcreteExecutionMarker {
            identity: observed.identity,
            generation: 1,
            plan_hash: [4; 32],
            period: 1,
            prior_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
                period: identity.period.as_u64(),
                root: identity.state_root,
            },
            transactions_hash: [5; 32],
            rewards_hash: [6; 32],
        };
        let marker_rlp =
            rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker(
                &marker,
            );
        lifecycle.stage_execution(&marker_rlp)?;
        let prepared = lifecycle.prepare(
            1.into(),
            ConcreteStateMutationBatch {
                accounts: vec![ConcreteAccountMutation::Delete { address: EE }],
                ..Default::default()
            },
        )?;
        assert_eq!(view.jail_block(validator)?, Some(1));
        assert_eq!(view.jailed_validators()?, vec![validator, validator]);
        use rustaxa_storage::ConcreteCommitApproval;
        use rustaxa_types::codec::rlp::concrete_lifecycle::{
            concrete_storage_slot_catalog_hash, encode_concrete_state_provenance,
            encode_concrete_storage_catalog,
        };
        use rustaxa_types::concrete_lifecycle::FinalChainConcreteStateProvenance;
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let projection_hash = [8; 32];
        let next = prepared.next_identity();
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
                period: next.period.as_u64(),
                root: next.state_root,
            },
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        let approval = ConcreteCommitApproval {
            marker_rlp,
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp: encode_concrete_storage_catalog(catalog.iter().copied()),
            projection_hash,
            catalog_hash,
        };
        let committed = lifecycle.commit_approved(prepared, approval)?;
        assert_eq!(view.jail_block(validator)?, Some(1));
        drop(view);
        let reopened = ConcreteStateLifecycle::open(&path, [7; 32], committed.committed)?;
        let next_reader = Arc::new(reopened.open_retained_checkpoint(committed.committed)?);
        let mut restarted =
            super::super::reward_scheduler::FinalChainRewardSchedulerRuntime::new(1.into());
        restarted.authorize_joint_startup(4, 1.into())?;
        restarted.complete_recovery(4)?;
        let next_basis = restarted.basis(4, 1.into(), 2.into(), [2; 32])?;
        assert_ne!(basis, next_basis);
        let next_view = FrozenEligibility::from_owner(
            next_reader,
            committed.committed,
            [2; 32],
            (4, 2),
            1,
            2,
            next_basis,
        )?;
        assert_eq!(next_view.jail_block(validator)?, None);
        assert!(next_view.jailed_validators()?.is_empty());
        assert!(next_view.validate_binding([1; 32], 3, 1, 0, 1).is_err());
        assert!(next_view.validate_binding([2; 32], 4, 2, 1, 2).is_ok());
        drop(next_view);
        drop(reopened);
        crate::closeout_test_scratch::dispose(path)?;
        Ok(())
    }

    #[test]
    fn checkpoint_expiry_list_cleanup_reopens_with_scalar_and_proof() -> Result<()> {
        use rustaxa_storage::{
            ConcreteAccountMutation, ConcreteStateLifecycle, ConcreteStateMutationBatch,
            ConcreteStorageMutation,
        };
        use rustaxa_types::FinalChainNonce;
        use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
        use rustaxa_types::concrete_state::{
            ConcreteAccount, ConcreteAccountBalance, ConcreteAccountRecord,
        };
        let path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!(
                "frozen-eligibility-expiry-reopen-{}",
                std::process::id()
            )),
            false,
        );
        let record = || {
            let mut r = rlp::RlpStream::new_list(5);
            r.append_empty_data();
            r.append_empty_data();
            r.append_empty_data();
            r.append_empty_data();
            r.append(&0_u64);
            ConcreteAccountRecord {
                account: ConcreteAccount {
                    nonce: FinalChainNonce::zero(),
                    balance: ConcreteAccountBalance::new(Default::default()),
                    storage_root: None,
                    code_hash: None,
                    code_size: 0,
                },
                physical_rlp: r.out().to_vec(),
            }
        };
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../rustaxa-evm/tests/support/actual-source108-aggregate-boundary.json"
        ))?;
        let decode = |s: &str| -> Vec<u8> {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect()
        };
        let validator: [u8; 20] = decode(
            fixture["rows"][0]["planner_facts"]["certificate_votes"][0]["validator"]
                .as_str()
                .unwrap(),
        )
        .try_into()
        .unwrap();
        let proof_key = ConcreteStorageKey(
            decode(fixture["proof_marker_slot"]["key"].as_str().unwrap())
                .try_into()
                .unwrap(),
        );
        let proof_value = decode(fixture["proof_marker_slot"]["value"].as_str().unwrap());
        let scalar_value = decode(fixture["jail_scalar_slot"]["value"].as_str().unwrap());
        let jail = fixture["jail_scalar_slot"]["period"].as_u64().unwrap();
        assert_eq!(rlp::Rlp::new(&scalar_value).as_val::<u64>()?, jail);
        let nonempty = decode(fixture["list_slot"]["nonempty"].as_str().unwrap());
        let empty = decode(fixture["list_slot"]["cleaned"].as_str().unwrap());
        let stake_key = ConcreteStorageKey(hash(&[&[0, 0], &validator]));
        let jail_key = ConcreteStorageKey(hash(&[&[0], &validator]));
        assert_eq!(
            jail_key.0.to_vec(),
            decode(fixture["jail_scalar_slot"]["key"].as_str().unwrap())
        );
        let total_key = ConcreteStorageKey(hash(&[&[4]]));
        let mut list_key = [0; 32];
        list_key[31] = 2;
        let list_key = ConcreteStorageKey(list_key);
        assert_eq!(
            list_key.0.to_vec(),
            decode(fixture["list_slot"]["key"].as_str().unwrap())
        );
        let mut stake = rlp::RlpStream::new_list(4);
        stake.append(&U256::from(150));
        stake.append(&0_u16);
        stake.append(&0_u64);
        stake.append(&0_u64);
        let mut list = rlp::RlpStream::new_list(1);
        list.append(&H160(validator));
        let list_bytes = list.out().to_vec();
        assert_eq!(list_bytes, nonempty);
        let rows = vec![
            (FE, stake_key, stake.out().to_vec()),
            (FE, total_key, vec![10]),
            (EE, jail_key, scalar_value.clone()),
            (EE, proof_key, proof_value.clone()),
            (
                EE,
                ConcreteStorageKey(hash(&[&[0], &[9; 20]])),
                rlp::encode(&0_u64).to_vec(),
            ),
            (EE, list_key, list_bytes),
        ];
        let mut catalog: Vec<ConcreteStorageSlot> = rows
            .iter()
            .map(|(address, key, _)| ConcreteStorageSlot {
                address: *address,
                key: key.0,
            })
            .collect();
        catalog.sort_unstable();
        assert_eq!(catalog.len(), 6);
        assert!(catalog.windows(2).all(|pair| pair[0] < pair[1]));
        let storage = rows
            .into_iter()
            .map(|(address, key, value)| ConcreteStorageMutation {
                address,
                key,
                value: Some(value),
            })
            .collect();
        let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
            &path,
            [7; 32],
            ConcreteStateMutationBatch {
                accounts: vec![
                    ConcreteAccountMutation::Upsert {
                        address: FE,
                        record: record(),
                    },
                    ConcreteAccountMutation::Upsert {
                        address: EE,
                        record: record(),
                    },
                ],
                storage,
                ..Default::default()
            },
            catalog.clone(),
        )?;
        crate::closeout_test_scratch::created(&path);
        let identity = lifecycle.prior_reader().identity();
        let reader = Arc::new(lifecycle.open_retained_checkpoint(identity)?);
        let mut scheduler =
            super::super::reward_scheduler::FinalChainRewardSchedulerRuntime::new(0.into());
        scheduler.authorize_joint_startup(3, 0.into())?;
        scheduler.complete_recovery(3)?;
        let basis = scheduler.basis(3, 0.into(), 1.into(), [1; 32])?;
        let view = FrozenEligibility::from_owner(reader, identity, [1; 32], (3, 1), 0, 1, basis)?;
        let mut policy = EligibilityPolicy {
            period: jail + 1,
            cacti: true,
            magnolia: true,
            aspen: true,
            threshold: 100.into(),
            step: 30.into(),
            maximum: 1000.into(),
        };
        assert_eq!(view.jail_block(validator)?, Some(jail));
        assert_eq!(view.jail_block([9; 20])?, Some(0));
        assert_eq!(view.jail_block([10; 20])?, None);
        assert_eq!(view.jailed_validators()?, vec![validator]);
        assert_eq!(view.jail_block(validator)?, Some(jail));
        assert_eq!(view.jailed_validators()?, vec![validator]);
        assert!(view.validate_binding([1; 32], 3, 1, 0, 1).is_ok());
        assert!(view.validate_binding([2; 32], 3, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 4, 1, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 2, 0, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 1, 1).is_err());
        assert!(view.validate_binding([1; 32], 3, 1, 0, 2).is_err());
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.validator_votes(validator, &policy)?, 5);
        assert_eq!(view.total_votes(&policy)?, 5);
        let cached_path = crate::closeout_test_scratch::new_path(
            std::env::temp_dir().join(format!(
                "cached-eligibility-expiry-reopen-{}",
                std::process::id()
            )),
            false,
        );
        let cached_storage = Arc::new(rustaxa_storage::Storage::new(
            rustaxa_storage::Config::new(cached_path.clone()),
        )?);
        crate::closeout_test_scratch::created(&cached_path);
        let cached = crate::final_chain::FinalChain::new_with_rewards_config(
            Arc::clone(&cached_storage),
            1_000_000.into(),
            0,
            vec![],
            vec![],
            rustaxa_types::GenesisDposConfig::default(),
            rustaxa_types::FinalChainRewardsConfig::default(),
        )?;
        let mut cached_snapshot = cached.dpos_snapshot(0.into())?;
        cached_snapshot.total_vote_count = 10;
        cached_snapshot.vote_counts.insert(validator, 5);
        cached_snapshot.slashing_jail_blocks.insert(validator, jail);
        cached_snapshot.slashing_jailed_validators = vec![validator];
        assert_eq!(
            cached.dpos_effective_total_vote_count(&cached_snapshot, policy.period.into())?,
            view.total_votes(&policy)?,
        );
        drop(cached);
        drop(cached_storage);
        let _ = crate::closeout_test_scratch::dispose(cached_path);
        assert!(!view.eligible([9; 20], &policy)?);
        assert_eq!(view.validator_votes([9; 20], &policy)?, 0);
        policy.period = 0;
        assert!(!view.eligible(validator, &policy)?);
        policy.period = 1;
        assert!(!view.eligible(validator, &policy)?);
        assert_eq!(view.validator_votes(validator, &policy)?, 0);
        assert_eq!(view.total_votes(&policy)?, 5);
        policy.cacti = false;
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.total_votes(&policy)?, 5);
        policy.cacti = true;
        policy.magnolia = false;
        assert!(view.eligible(validator, &policy)?);
        assert_eq!(view.total_votes(&policy)?, 5);
        policy.step = 10.into();
        assert!(view.total_votes(&policy).is_err());
        policy.step = 30.into();
        policy.magnolia = true;
        for head in (jail - 1)..=(jail + 5) {
            policy.period = head;
            assert_eq!(view.jail_block(validator)?, Some(jail));
            assert_eq!(view.slot(EE, proof_key.0)?, Some(proof_value.clone()));
            assert_eq!(view.slot(EE, list_key.0)?, Some(nonempty.clone()));
            assert_eq!(view.total_votes(&policy)?, 5);
        }
        let observed = lifecycle.observation()?;
        let marker = rustaxa_types::concrete_lifecycle::FinalChainConcreteExecutionMarker {
            identity: observed.identity,
            generation: 1,
            plan_hash: [4; 32],
            period: 1,
            prior_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
                period: identity.period.as_u64(),
                root: identity.state_root,
            },
            transactions_hash: [5; 32],
            rewards_hash: [6; 32],
        };
        let marker_rlp =
            rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker(
                &marker,
            );
        lifecycle.stage_execution(&marker_rlp)?;
        let prepared = lifecycle.prepare(
            1.into(),
            ConcreteStateMutationBatch {
                storage: vec![ConcreteStorageMutation {
                    address: EE,
                    key: list_key,
                    value: Some(empty.clone()),
                }],
                ..Default::default()
            },
        )?;
        assert_eq!(view.jail_block(validator)?, Some(jail));
        assert_eq!(view.jailed_validators()?, vec![validator]);
        use rustaxa_storage::ConcreteCommitApproval;
        use rustaxa_types::codec::rlp::concrete_lifecycle::{
            concrete_storage_slot_catalog_hash, encode_concrete_state_provenance,
            encode_concrete_storage_catalog,
        };
        use rustaxa_types::concrete_lifecycle::FinalChainConcreteStateProvenance;
        let catalog_hash = concrete_storage_slot_catalog_hash(catalog.iter().copied());
        let projection_hash = [8; 32];
        let next = prepared.next_identity();
        let provenance = FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
                period: next.period.as_u64(),
                root: next.state_root,
            },
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        };
        let approval = ConcreteCommitApproval {
            marker_rlp,
            provenance_rlp: encode_concrete_state_provenance(&provenance),
            catalog_rlp: encode_concrete_storage_catalog(catalog.iter().copied()),
            projection_hash,
            catalog_hash,
        };
        let committed = lifecycle.commit_approved(prepared, approval)?;
        assert_eq!(view.jail_block(validator)?, Some(jail));
        assert_eq!(view.jailed_validators()?, vec![validator]);
        assert_eq!(view.slot(EE, proof_key.0)?, Some(proof_value.clone()));
        assert!(view.validate_binding([2; 32], 4, 2, 1, 2).is_err());
        drop(view);
        let reopened = ConcreteStateLifecycle::open(&path, [7; 32], committed.committed)?;
        let next_reader = Arc::new(reopened.open_retained_checkpoint(committed.committed)?);
        let mut restarted =
            super::super::reward_scheduler::FinalChainRewardSchedulerRuntime::new(1.into());
        restarted.authorize_joint_startup(4, 1.into())?;
        restarted.complete_recovery(4)?;
        let next_basis = restarted.basis(4, 1.into(), 2.into(), [2; 32])?;
        assert_ne!(basis, next_basis);
        let next_view = FrozenEligibility::from_owner(
            next_reader,
            committed.committed,
            [2; 32],
            (4, 2),
            1,
            2,
            next_basis,
        )?;
        assert_eq!(next_view.jail_block(validator)?, Some(jail));
        assert!(next_view.jailed_validators()?.is_empty());
        assert_eq!(next_view.slot(EE, proof_key.0)?, Some(proof_value.clone()));
        policy.step = 30.into();
        policy.magnolia = true;
        policy.period = jail + 5;
        for head in (jail - 1)..=(jail + 5) {
            policy.period = head;
            assert_eq!(next_view.jail_block(validator)?, Some(jail));
            assert_eq!(next_view.slot(EE, proof_key.0)?, Some(proof_value.clone()));
            assert_eq!(next_view.slot(EE, list_key.0)?, Some(empty.clone()));
            assert_eq!(next_view.total_votes(&policy)?, 10);
        }
        assert_eq!(next_view.total_votes(&policy)?, 10);
        assert!(next_view.validate_binding([1; 32], 3, 1, 0, 1).is_err());
        assert!(next_view.validate_binding([2; 32], 4, 2, 1, 2).is_ok());
        drop(next_view);
        drop(reopened);
        let reader = Arc::new(ConcreteCheckpointReaders::open_read_only(
            &path,
            committed.committed,
            [committed.committed],
        )?);
        let read_only = FrozenEligibility::from_owner(
            reader,
            committed.committed,
            [2; 32],
            (4, 2),
            1,
            2,
            next_basis,
        )?;
        assert_eq!(read_only.jail_block(validator)?, Some(jail));
        assert!(read_only.jailed_validators()?.is_empty());
        assert_eq!(read_only.slot(EE, proof_key.0)?, Some(proof_value.clone()));
        assert_eq!(read_only.total_votes(&policy)?, 10);
        drop(read_only);
        crate::closeout_test_scratch::dispose(path)?;
        Ok(())
    }
}
