//! First complete mixed-workload period through the existing FinalChain owner.
//!
//! This finite fixture creates both databases exclusively, hydrates the Rust
//! genesis snapshot from the independently generated concrete rows, executes
//! four periods through the real EVM driver and bound native rewards
//! session, and commits via ordered concrete lifecycle phases. It then closes
//! and reopens both databases and compares receipts, roots, catalog values and CF1--CF5 to
//! the pinned Go observer. This is test composition, not production routing or
//! imported-state qualification.

#[path = "support/mixed_genesis.rs"]
mod mixed_genesis;
#[path = "support/mixed_native.rs"]
mod mixed_native;
#[path = "support/state_api_epoch.rs"]
mod state_api_epoch;

use anyhow::{Context, Result, bail, ensure};
use ethereum_types::{H160, H256, U256};
use k256::ecdsa::SigningKey;
use num_bigint::BigUint;
use revm::primitives::keccak256;
use rlp::{Rlp, RlpStream};
use rustaxa_consensus::concrete_state_projection::*;
use rustaxa_consensus::final_chain_execution::*;
use rustaxa_consensus::{
    ConsensusExecutionPort, FinalChain, PillarAnchorStateReport, PillarAnchorStateRequest,
    RewardCertVoteFact,
    native_projection_context::{
        FinalChainNativeInvocationContext, FinalChainNativeProjectionContext,
        FinalChainNativeRewardsContext,
    },
};
use rustaxa_evm::{
    contracts::{
        BlockHashRead, BlockHashReadError, CodeExecutionError, CodeExecutionStatus,
        ConsensusNativeDisposition, ConsensusNativeObservation, ExecutionBlockContext,
        ExecutionTransactionKind, NativeCallKind, NativeOutcome, NativeRawOperation,
        TransactionExecutionResult,
    },
    driver::{
        NativeAddressClassifier, PeriodConsensusSequence, execute_top_level_call_with_native,
        execute_top_level_create_with_native,
    },
    envelope::EnvelopeRules,
    input::{LegacyInputKind, decode_legacy_input},
    journal::{ExecutionJournal, JournalAccountOperation},
    profile::TaraxaProfile,
};
use rustaxa_storage::{
    ConcreteAccountMutation, ConcreteCodeInsertion, ConcreteStateReader, ConcreteStorageMutation,
    Storage,
};
use rustaxa_storage::{
    ConcreteCommitApproval, ConcreteObserverPhaseDelta, ConcreteObserverPhaseOutput,
    ConcreteStateLifecycle, PreparedConcreteView,
};
use rustaxa_types::codec::rlp::concrete_lifecycle::{
    decode_concrete_storage_catalog, encode_concrete_storage_catalog,
};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::concrete_lifecycle::ConcreteStorageSlot;
use rustaxa_types::concrete_state::*;
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainNonce, FinalizationDagBlock, FinalizationTransaction,
    LegacyTransactionEnvelope, StoredFinalChainBlockHeader,
};
use serde_json::Value;
use state_api_epoch::StateApiEpoch;
use std::{
    cell::RefCell,
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use mixed_genesis::{duration_active_rules, fresh_concrete, historical_gas_rules, open_chain};
use mixed_native::MixedNativeExecutionPort;

#[path = "support/era_retention.rs"]
mod era_retention;
#[path = "support/mainnet_era.rs"]
mod mainnet_era;

#[test]
fn duration_historical_rules_pin_active_gas_and_fork() {
    let fixture = serde_json::json!({"profile":"synthetic_mainnet_era", "scenario":"magnolia_duration",
        "configuration":{"block_gas_limit":2100000000_u64,"hardforks":{"cornus":{"block":15610000_u64,"pbft_gas_limit":2100000000_u64}}}});
    for period in [5_729_994, 5_729_995, 5_730_002, 5_893_461, 5_893_468] {
        assert_eq!(
            duration_active_rules(&fixture, period).unwrap(),
            Some((315_000_000, false))
        );
    }
    assert_eq!(
        historical_gas_rules(15_609_999, 15_610_000, 2_100_000_000),
        (315_000_000, false)
    );
    assert_eq!(
        historical_gas_rules(15_610_000, 15_610_000, 2_100_000_000),
        (2_100_000_000, true)
    );
    assert!(duration_active_rules(&fixture, 15_610_000).is_err());
    let mut other = fixture.clone();
    other["scenario"] = serde_json::json!("unrelated");
    assert_eq!(duration_active_rules(&other, 15_610_000).unwrap(), None);
}

#[test]
fn duration_request_uses_historical_block_limit() {
    let fixture = serde_json::json!({"profile":"synthetic_mainnet_era", "scenario":"magnolia_duration",
        "configuration":{"block_gas_limit":2100000000_u64,"hardforks":{"cornus":{"block":15610000_u64,"pbft_gas_limit":2100000000_u64}}}});
    let period = serde_json::json!({"number":5729995_u64,"transactions":[],
        "planner_facts":{"dag_blocks":[],"certificate_votes":[]}, "reward_input":{"blocks_per_year":210240_u64}});
    let request = request(&fixture, &period).unwrap();
    assert_eq!(request.block_gas_limit.as_u64(), 315_000_000);
}

#[test]
fn duration_pre_cornus_intrinsic_failure_keeps_nonce() {
    use rustaxa_evm::contracts::{
        ExecutionGasPrice, ExecutionTransaction, ExecutionTransactionKind, ExecutionValue,
    };
    use rustaxa_evm::envelope::{EnvelopeAdmission, EnvelopeRules, IntrinsicGasSchedule, admit};
    use rustaxa_types::{FinalChainGas, FinalChainTransactionPosition};
    let fixture = serde_json::json!({"profile":"synthetic_mainnet_era", "scenario":"magnolia_duration",
        "configuration":{"block_gas_limit":2100000000_u64,"hardforks":{"cornus":{"block":15610000_u64,"pbft_gas_limit":2100000000_u64}}}});
    let sender = [0x71; 20];
    let tx = ExecutionTransaction {
        position: FinalChainTransactionPosition::from(0_u32),
        hash: [0x11; 32],
        sender,
        receiver: Some([0xbb; 20]),
        nonce: FinalChainNonce::from_u64(7),
        gas_price: ExecutionGasPrice::new(BigUint::default()),
        gas_limit: FinalChainGas::new(20_000),
        value: ExecutionValue::default(),
        input: Vec::new(),
        canonical_rlp: None,
        kind: ExecutionTransactionKind::Call,
    };
    let mut journal = ExecutionJournal::new(FeeJournalRead {
        author: sender,
        present: true,
        unavailable: false,
    });
    let rules = duration_active_rules(&fixture, 5_730_001).unwrap().unwrap();
    let admission = admit(
        &mut journal,
        &tx,
        EnvelopeRules { cornus: rules.1 },
        IntrinsicGasSchedule::PINNED,
    )
    .unwrap();
    let EnvelopeAdmission::Rejected(failure) = admission else {
        panic!("expected intrinsic rejection")
    };
    assert_eq!(
        failure.error,
        rustaxa_evm::contracts::ConsensusFailure::IntrinsicGas
    );
    assert_eq!(
        journal.account(sender).unwrap().nonce,
        FinalChainNonce::from_u64(7)
    );
    let mut after_cornus = ExecutionJournal::new(FeeJournalRead {
        author: sender,
        present: true,
        unavailable: false,
    });
    let admission = admit(
        &mut after_cornus,
        &tx,
        EnvelopeRules { cornus: true },
        IntrinsicGasSchedule::PINNED,
    )
    .unwrap();
    assert!(matches!(admission, EnvelopeAdmission::Rejected(_)));
    assert_eq!(
        after_cornus.account(sender).unwrap().nonce,
        FinalChainNonce::from_u64(8)
    );
}

fn fixture_execution_profile(period: &Value) -> TaraxaProfile {
    use rustaxa_evm::profile::TaraxaPhase;
    TaraxaProfile::for_phase(if period["rules"]["cacti"] == true {
        TaraxaPhase::Cacti
    } else if period["rules"]["ficus"] == true {
        TaraxaPhase::Ficus
    } else {
        TaraxaPhase::Californicum
    })
}

fn bytes(value: &Value) -> Vec<u8> {
    hex::decode(value.as_str().expect("fixture hex string")).unwrap()
}
fn fixed<const N: usize>(value: &Value) -> [u8; N] {
    bytes(value).try_into().unwrap()
}
fn number(value: &Value) -> u64 {
    value.as_u64().expect("fixture integer")
}
fn string(value: &Value) -> &str {
    value.as_str().expect("fixture string")
}
fn record(
    nonce: FinalChainNonce,
    balance: ConcreteAccountBalance,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
) -> ConcreteAccountRecord {
    let mut raw = RlpStream::new_list(5);
    raw.append(&nonce.to_bytes());
    let balance_bytes = if balance.value() == &BigUint::default() {
        vec![]
    } else {
        balance.value().to_bytes_be()
    };
    raw.append(&balance_bytes);
    raw.append_empty_data();
    match code_hash {
        Some(hash) => {
            raw.append(&hash.as_slice());
        }
        None => {
            raw.append_empty_data();
        }
    }
    raw.append(&code_size);
    ConcreteAccountRecord {
        account: ConcreteAccount {
            nonce,
            balance,
            storage_root: None,
            code_hash,
            code_size,
        },
        physical_rlp: raw.out().to_vec(),
    }
}

/// A fixed durable or unpublished base; unpublished views expose execution
/// reads only and cannot impersonate a committed descriptor.
enum FixturePrior<'a> {
    Lifecycle(&'a ConcreteStateLifecycle),
    Committed(&'a ConcreteStateReader),
    Prepared(PreparedConcreteView<'a>),
}
thread_local! {
    static PRIOR_PROOF_EVENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PRIOR_PROOF_NONMEMBER_RECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static COMMITTED_PROOF_NONMEMBER_RECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static PREPARED_PROOF_NONMEMBER_RECORDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn record_prior_proof(
    provider_kind: &str,
    address: [u8; 20],
    key: ConcreteStorageKey,
    provider: ConcreteStateIdentity,
    view: ConcreteStateIdentity,
    unavailable: ConcreteStateIdentity,
    proof: &Result<rustaxa_storage::ConcreteStoragePath, ConcreteReadError>,
) {
    let (kind, length) = match proof {
        Ok(rustaxa_storage::ConcreteStoragePath::NonMember) => {
            let seen = match provider_kind {
                "lifecycle_prior" => PRIOR_PROOF_NONMEMBER_RECORDED.with(|seen| seen.replace(true)),
                "committed" => COMMITTED_PROOF_NONMEMBER_RECORDED.with(|seen| seen.replace(true)),
                "prepared" => PREPARED_PROOF_NONMEMBER_RECORDED.with(|seen| seen.replace(true)),
                _ => panic!("unsupported proof provider"),
            };
            if seen {
                return;
            }
            ("nonmember", None)
        }
        Ok(rustaxa_storage::ConcreteStoragePath::Member(bytes)) => ("member", Some(bytes.len())),
        Err(ConcreteReadError::Pruned(_)) => ("error_pruned", None),
        Err(ConcreteReadError::HistoryUnavailable(_)) => ("error_history_unavailable", None),
        Err(ConcreteReadError::FuturePeriod { .. }) => ("error_future_period", None),
        Err(ConcreteReadError::IdentityMismatch { .. }) => ("error_identity_mismatch", None),
        Err(ConcreteReadError::Corrupt(_)) => ("error_corrupt", None),
        Err(ConcreteReadError::Io(_)) => ("error_io", None),
    };
    PRIOR_PROOF_EVENTS.with(|count| {
        let next = count.get();
        if next >= 64 {
            eprintln!("FIXTURE_PRIOR_PROOF_OVERFLOW limit=64");
            panic!("prior proof diagnostic limit exceeded");
        }
        count.set(next + 1);
    });
    eprintln!(
        "FIXTURE_PRIOR_PROOF provider={provider_kind} address={address:?} key={key:?} provider_identity={provider:?} view_identity={view:?} unavailable_identity={unavailable:?} kind={kind} member_length={length:?}"
    );
}

/// Immutable per-transaction view over the fixture's proof-checked state.
/// The ordered path bypasses the cumulative adapter's account/slot/code caches.
struct FixtureView<'a> {
    prior: FixturePrior<'a>,
    identity: ConcreteStateIdentity,
}
impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for FixtureView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        let result = match &self.prior {
            FixturePrior::Lifecycle(owner) => owner.prior_reader().account(address),
            FixturePrior::Committed(prior) => ConcreteStateRead::account(*prior, address),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                    prior, address,
                )
            }
        };
        #[cfg(feature = "synthetic-fixtures")]
        if native_integration::cleanup_requested()
            && rustaxa_consensus::native_session::cleanup_diagnostic::is_active()
            && address[..19] == [0_u8; 19]
            && address[19] == 0xee
        {
            use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
            let (source, actual_identity) = match &self.prior {
                FixturePrior::Lifecycle(owner) => {
                    ("lifecycle_prior", owner.prior_reader().identity())
                }
                FixturePrior::Committed(prior) => {
                    ("committed", ConcreteStateRead::identity(*prior))
                }
                FixturePrior::Prepared(prior) => (
                    "prepared",
                    rustaxa_types::concrete_state::execution::ConcreteExecutionRead::identity(
                        prior,
                    ),
                ),
            };
            match &result {
                Ok(value) => diagnostic::event_fmt(
                    "fixture_account_original",
                    format_args!(
                        "identity={:?} actual_identity={:?} source={} variant={} phase=original time={:?} authority=UNKNOWN",
                        self.identity,
                        actual_identity,
                        source,
                        match value {
                            ConcreteRead::Present(_) => "Present",
                            ConcreteRead::Absent => "Absent",
                            ConcreteRead::Tombstone => "Tombstone",
                        },
                        std::time::SystemTime::now()
                    ),
                ),
                Err(error) => diagnostic::event_fmt(
                    "fixture_account_original",
                    format_args!(
                        "identity={:?} actual_identity={:?} source={} error={:?} phase=original time={:?} authority=UNKNOWN",
                        self.identity,
                        actual_identity,
                        source,
                        error,
                        std::time::SystemTime::now()
                    ),
                ),
            }
        }
        if rustaxa_evm::journal::duration_journal_is_armed()
            && let Ok(read) = &result
        {
            let mut value = Vec::new();
            match read {
                ConcreteRead::Absent => value.push(0),
                ConcreteRead::Tombstone => value.push(1),
                ConcreteRead::Present(record) => {
                    value.push(2);
                    value.extend_from_slice(&record.physical_rlp);
                }
            }
            rustaxa_evm::journal::duration_journal_plan_event(
                "source_account",
                address,
                &self.identity.state_root,
                &value,
            );
        }
        result
    }
    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let result = match &self.prior {
            FixturePrior::Lifecycle(owner) => owner.prior_reader().storage(address, key),
            FixturePrior::Committed(prior) => ConcreteStateRead::storage(*prior, address, key),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                    prior, address, key,
                )
            }
        };
        #[cfg(feature = "synthetic-fixtures")]
        if native_integration::cleanup_requested()
            && rustaxa_consensus::native_session::cleanup_diagnostic::is_active()
            && address[..19] == [0_u8; 19]
            && address[19] == 0xee
            && key.0[..31] == [0_u8; 31]
            && key.0[31] == 2
        {
            use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
            let too_large = match &result {
                Ok(ConcreteRead::Present(value)) => value.len() > 128,
                Err(ConcreteReadError::Corrupt(value) | ConcreteReadError::Io(value)) => {
                    value.len() > 128
                }
                _ => false,
            };
            if too_large {
                diagnostic::overflow();
            } else {
                diagnostic::event_fmt(
                    "fixture_original",
                    format_args!(
                        "identity={:?} view={} result={:?} authority=original_result",
                        self.identity,
                        match &self.prior {
                            FixturePrior::Lifecycle(_) => "lifecycle_prior",
                            FixturePrior::Committed(_) => "committed",
                            FixturePrior::Prepared(_) => "prepared",
                        },
                        result
                    ),
                );
            }
        }
        let mapped = match result {
            Err(ConcreteReadError::HistoryUnavailable(unavailable)) => {
                let identity = match &self.prior {
                    FixturePrior::Lifecycle(owner) => owner.prior_reader().identity(),
                    FixturePrior::Committed(prior) => ConcreteStateRead::identity(*prior),
                    FixturePrior::Prepared(prior) => {
                        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::identity(
                            prior,
                        )
                    }
                };
                if unavailable != identity || identity != self.identity {
                    Err(ConcreteReadError::HistoryUnavailable(unavailable))
                } else if let FixturePrior::Lifecycle(owner) = &self.prior {
                    let proof = owner.verify_prior_storage_path(address, key);
                    record_prior_proof(
                        "lifecycle_prior",
                        address,
                        key,
                        identity,
                        self.identity,
                        unavailable,
                        &proof,
                    );
                    match proof {
                        Ok(rustaxa_storage::ConcreteStoragePath::NonMember) => {
                            Ok(ConcreteRead::Absent)
                        }
                        _ => Err(ConcreteReadError::HistoryUnavailable(unavailable)),
                    }
                } else if let FixturePrior::Prepared(view) = &self.prior {
                    let proof = view.verify_storage_path(address, key);
                    record_prior_proof(
                        "prepared",
                        address,
                        key,
                        identity,
                        self.identity,
                        unavailable,
                        &proof,
                    );
                    match proof {
                        Ok(rustaxa_storage::ConcreteStoragePath::NonMember) => {
                            Ok(ConcreteRead::Absent)
                        }
                        _ => Err(ConcreteReadError::HistoryUnavailable(unavailable)),
                    }
                } else if let FixturePrior::Committed(reader) = &self.prior {
                    let proof = reader.verify_storage_path(address, key);
                    record_prior_proof(
                        "committed",
                        address,
                        key,
                        identity,
                        self.identity,
                        unavailable,
                        &proof,
                    );
                    match proof {
                        Ok(rustaxa_storage::ConcreteStoragePath::NonMember) => {
                            Ok(ConcreteRead::Absent)
                        }
                        _ => Err(ConcreteReadError::HistoryUnavailable(unavailable)),
                    }
                } else {
                    unreachable!()
                }
            }
            result => result,
        };
        #[cfg(feature = "synthetic-fixtures")]
        if native_integration::cleanup_requested()
            && rustaxa_consensus::native_session::cleanup_diagnostic::is_active()
            && address[..19] == [0_u8; 19]
            && address[19] == 0xee
            && key.0[..31] == [0_u8; 31]
            && key.0[31] == 2
        {
            rustaxa_consensus::native_session::cleanup_diagnostic::event_fmt(
                "fixture_mapped",
                format_args!(
                    "identity={:?} result={:?} authority=proof_or_original",
                    self.identity, mapped
                ),
            );
        }
        if rustaxa_evm::journal::duration_journal_is_armed()
            && let Ok(read) = &mapped
        {
            let mut value = Vec::new();
            match read {
                ConcreteRead::Absent => value.push(0),
                ConcreteRead::Tombstone => value.push(1),
                ConcreteRead::Present(bytes) => {
                    value.push(2);
                    value.extend_from_slice(bytes);
                }
            }
            let mut identity = self.identity.state_root.to_vec();
            identity.extend_from_slice(&key.0);
            rustaxa_evm::journal::duration_journal_plan_event(
                "source_storage",
                address,
                &identity,
                &value,
            );
        }
        mapped
    }
    fn code(&self, hash: [u8; 32]) -> Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let result = match &self.prior {
            FixturePrior::Lifecycle(owner) => owner.prior_reader().code(hash),
            FixturePrior::Committed(prior) => ConcreteStateRead::code(*prior, hash),
            FixturePrior::Prepared(prior) => {
                rustaxa_types::concrete_state::execution::ConcreteExecutionRead::code(prior, hash)
            }
        };
        if rustaxa_evm::journal::duration_journal_is_armed()
            && let Ok(read) = &result
        {
            let mut value = Vec::new();
            match read {
                ConcreteRead::Absent => value.push(0),
                ConcreteRead::Tombstone => value.push(1),
                ConcreteRead::Present(bytes) => {
                    value.push(2);
                    value.extend_from_slice(bytes);
                }
            }
            let mut identity = self.identity.state_root.to_vec();
            identity.extend_from_slice(&hash);
            rustaxa_evm::journal::duration_journal_plan_event(
                "source_code",
                [0; 20],
                &identity,
                &value,
            );
        }
        result
    }
}
#[test]
fn lifecycle_prior_fixture_view_proves_unknown_slot_without_changing_identity() -> Result<()> {
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!("writer-prior-view-{}", std::process::id())),
        false,
    );
    let address = [0x41; 20];
    let present = ConcreteStorageKey([1; 32]);
    let unknown = ConcreteStorageKey([2; 32]);
    let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
        &path,
        [0x77; 32],
        rustaxa_storage::ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: record(
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::default()),
                    None,
                    0,
                ),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key: present,
                value: Some(vec![1]),
            }],
            ..Default::default()
        },
        vec![ConcreteStorageSlot {
            address,
            key: present.0,
        }],
    )?;
    closeout_test_scratch::created(&path);
    let identity = lifecycle.prior_reader().identity();
    ensure!(
        lifecycle.prior_reader().storage(address, unknown)
            == Err(ConcreteReadError::HistoryUnavailable(identity))
    );
    ensure!(
        lifecycle.verify_prior_storage_path(address, unknown)?
            == rustaxa_storage::ConcreteStoragePath::NonMember
    );
    ensure!(
        lifecycle.verify_prior_storage_path(address, present)?
            == rustaxa_storage::ConcreteStoragePath::Member(vec![1])
    );
    let view = FixtureView {
        prior: FixturePrior::Lifecycle(&lifecycle),
        identity,
    };
    ensure!(
        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            &view, address, unknown
        )? == ConcreteRead::Absent
    );
    ensure!(
        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            &view, address, present
        )? == ConcreteRead::Present(vec![1])
    );
    let wrong = FixtureView {
        prior: FixturePrior::Lifecycle(&lifecycle),
        identity: ConcreteStateIdentity {
            period: identity.period,
            state_root: [3; 32],
        },
    };
    ensure!(
        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            &wrong, address, unknown
        ) == Err(ConcreteReadError::HistoryUnavailable(identity))
    );
    drop(lifecycle);
    closeout_test_scratch::dispose(&path)?;
    Ok(())
}

#[test]
fn committed_fixture_view_proves_slots_and_preserves_proof_errors() -> Result<()> {
    use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!("writer-committed-proof-{}", std::process::id())),
        false,
    );
    let address = [0x41; 20];
    let present = ConcreteStorageKey([1; 32]);
    let unknown = ConcreteStorageKey([2; 32]);
    let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
        &path,
        [0x77; 32],
        rustaxa_storage::ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: record(
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::default()),
                    None,
                    0,
                ),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key: present,
                value: Some(vec![1]),
            }],
            ..Default::default()
        },
        vec![ConcreteStorageSlot {
            address,
            key: present.0,
        }],
    )?;
    closeout_test_scratch::created(&path);
    let identity = ConcreteStateRead::identity(lifecycle.prior_reader());
    let storage_root = match ConcreteStateRead::account(lifecycle.prior_reader(), address)? {
        ConcreteRead::Present(row) => row.account.storage_root.context("storage root")?,
        _ => bail!("missing account"),
    };
    drop(lifecycle);
    {
        let reader = ConcreteStateReader::open_read_only(&path, identity)?;
        ensure!(
            ConcreteStateRead::storage(&reader, address, unknown)
                == Err(ConcreteReadError::HistoryUnavailable(identity))
        );
        ensure!(
            reader.verify_storage_path(address, unknown)?
                == rustaxa_storage::ConcreteStoragePath::NonMember
        );
        ensure!(
            reader.verify_storage_path(address, present)?
                == rustaxa_storage::ConcreteStoragePath::Member(vec![1])
        );
        let view = FixtureView {
            prior: FixturePrior::Committed(&reader),
            identity,
        };
        ensure!(view.storage(address, unknown)? == ConcreteRead::Absent);
        ensure!(view.storage(address, present)? == ConcreteRead::Present(vec![1]));
        let wrong = FixtureView {
            prior: FixturePrior::Committed(&reader),
            identity: ConcreteStateIdentity {
                period: identity.period,
                state_root: [3; 32],
            },
        };
        ensure!(
            wrong.storage(address, unknown) == Err(ConcreteReadError::HistoryUnavailable(identity))
        );
    }
    for (column, root) in [("4", storage_root), ("2", identity.state_root)] {
        let options = rocksdb::Options::default();
        let names = rocksdb::DB::list_cf(&options, &path)?;
        let descriptors = names
            .iter()
            .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
        let db = rocksdb::DB::open_cf_descriptors(&options, &path, descriptors)?;
        let cf = db.cf_handle(column).context("proof column")?;
        let bytes = db.get_cf(cf, root)?.context("proof root before fault")?;
        db.delete_cf(cf, root)?;
        drop(db);
        {
            let reader = ConcreteStateReader::open_read_only(&path, identity)?;
            ensure!(reader.verify_storage_path(address, unknown).is_err());
            let before = PRIOR_PROOF_EVENTS.with(|count| count.get());
            let view = FixtureView {
                prior: FixturePrior::Committed(&reader),
                identity,
            };
            ensure!(
                view.storage(address, unknown)
                    == Err(ConcreteReadError::HistoryUnavailable(identity))
            );
            ensure!(PRIOR_PROOF_EVENTS.with(|count| count.get()) == before + 1);
        }
        let db = rocksdb::DB::open_cf_descriptors(
            &options,
            &path,
            names.iter().map(|name| {
                rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default())
            }),
        )?;
        let cf = db.cf_handle(column).context("proof column")?;
        db.put_cf(cf, root, bytes)?;
        drop(db);
    }
    closeout_test_scratch::dispose(&path)?;
    Ok(())
}

#[test]
fn prepared_fixture_view_proves_unknown_slot_at_unpublished_root() -> Result<()> {
    use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!("writer-prepared-view-{}", std::process::id())),
        false,
    );
    let address = [0x41; 20];
    let member = ConcreteStorageKey([1; 32]);
    let unknown = ConcreteStorageKey([2; 32]);
    let mut lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
        &path,
        [0x77; 32],
        rustaxa_storage::ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: record(
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::default()),
                    None,
                    0,
                ),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key: member,
                value: Some(vec![1]),
            }],
            ..Default::default()
        },
        vec![ConcreteStorageSlot {
            address,
            key: member.0,
        }],
    )?;
    closeout_test_scratch::created(&path);
    let prior = lifecycle.observation()?;
    let marker = rustaxa_types::concrete_lifecycle::FinalChainConcreteExecutionMarker {
        identity: prior.identity,
        generation: prior.generation + 1,
        plan_hash: [0x44; 32],
        period: prior.committed.period.as_u64() + 1,
        prior_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
            period: prior.committed.period.as_u64(),
            root: prior.committed.state_root,
        },
        transactions_hash: [0x55; 32],
        rewards_hash: [0x66; 32],
    };
    let marker_rlp =
        rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker(&marker);
    lifecycle.stage_execution(&marker_rlp)?;
    let output = lifecycle.apply_observer_phase(ConcreteObserverPhaseDelta::default())?;
    let before = lifecycle.observation()?;
    let prepared = lifecycle.prepared_view(&output)?;
    let identity = prepared.identity();
    ensure!(
        prepared.storage(address, unknown) == Err(ConcreteReadError::HistoryUnavailable(identity))
    );
    ensure!(
        prepared.verify_storage_path(address, unknown)?
            == rustaxa_storage::ConcreteStoragePath::NonMember
    );
    ensure!(
        prepared.verify_storage_path(address, member)?
            == rustaxa_storage::ConcreteStoragePath::Member(vec![1])
    );
    let view = FixtureView {
        prior: FixturePrior::Prepared(prepared),
        identity,
    };
    ensure!(view.storage(address, unknown)? == ConcreteRead::Absent);
    ensure!(view.storage(address, member)? == ConcreteRead::Present(vec![1]));
    let wrong = FixtureView {
        prior: FixturePrior::Prepared(lifecycle.prepared_view(&output)?),
        identity: ConcreteStateIdentity {
            period: identity.period,
            state_root: [3; 32],
        },
    };
    ensure!(
        wrong.storage(address, unknown) == Err(ConcreteReadError::HistoryUnavailable(identity))
    );
    ensure!(lifecycle.observation()? == before);
    drop(lifecycle);
    closeout_test_scratch::dispose(&path)?;
    Ok(())
}

#[test]
fn prepared_fixture_view_preserves_unavailable_when_prior_proof_node_is_missing() -> Result<()> {
    use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
    let path = closeout_test_scratch::new_path(
        std::env::temp_dir().join(format!(
            "writer-prepared-missing-node-{}",
            std::process::id()
        )),
        false,
    );
    let address = [0x41; 20];
    let member = ConcreteStorageKey([1; 32]);
    let unknown = ConcreteStorageKey([2; 32]);
    let lifecycle = ConcreteStateLifecycle::create_fresh_exclusive(
        &path,
        [0x77; 32],
        rustaxa_storage::ConcreteStateMutationBatch {
            accounts: vec![ConcreteAccountMutation::Upsert {
                address,
                record: record(
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::default()),
                    None,
                    0,
                ),
            }],
            storage: vec![ConcreteStorageMutation {
                address,
                key: member,
                value: Some(vec![1]),
            }],
            ..Default::default()
        },
        vec![ConcreteStorageSlot {
            address,
            key: member.0,
        }],
    )?;
    closeout_test_scratch::created(&path);
    let prior = lifecycle.observation()?;
    let root = match ConcreteStateRead::account(lifecycle.prior_reader(), address)? {
        ConcreteRead::Present(record) => {
            record.account.storage_root.context("prior storage root")?
        }
        _ => bail!("prior account missing"),
    };
    drop(lifecycle);
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, &path)?;
    let descriptors = columns
        .iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
    let db = rocksdb::DB::open_cf_descriptors(&options, &path, descriptors)?;
    let cf4 = db.cf_handle("4").context("CF4")?;
    ensure!(db.get_cf(cf4, root)?.is_some(), "actual prior proof root");
    db.delete_cf(cf4, root)?;
    drop(db);
    let mut lifecycle = ConcreteStateLifecycle::open(&path, [0x77; 32], prior.committed)?;
    let marker = rustaxa_types::concrete_lifecycle::FinalChainConcreteExecutionMarker {
        identity: prior.identity,
        generation: prior.generation + 1,
        plan_hash: [0x44; 32],
        period: prior.committed.period.as_u64() + 1,
        prior_state: rustaxa_types::concrete_lifecycle::FinalChainConcreteState {
            period: prior.committed.period.as_u64(),
            root: prior.committed.state_root,
        },
        transactions_hash: [0x55; 32],
        rewards_hash: [0x66; 32],
    };
    lifecycle.stage_execution(
        &rustaxa_types::codec::rlp::concrete_lifecycle::encode_concrete_execution_marker(&marker),
    )?;
    let output = lifecycle.apply_observer_phase(ConcreteObserverPhaseDelta::default())?;
    let prepared = lifecycle.prepared_view(&output)?;
    let identity = prepared.identity();
    ensure!(
        prepared.storage(address, unknown) == Err(ConcreteReadError::HistoryUnavailable(identity))
    );
    ensure!(
        prepared.verify_storage_path(address, unknown).is_err(),
        "missing prior proof must fail"
    );
    let before = PRIOR_PROOF_EVENTS.with(|count| count.get());
    let view = FixtureView {
        prior: FixturePrior::Prepared(prepared),
        identity,
    };
    ensure!(view.storage(address, unknown) == Err(ConcreteReadError::HistoryUnavailable(identity)));
    ensure!(
        PRIOR_PROOF_EVENTS.with(|count| count.get()) == before + 1,
        "secondary proof error was recorded"
    );
    drop(lifecycle);
    closeout_test_scratch::dispose(&path)?;
    Ok(())
}

struct ChainHashes<'a>(&'a FinalChain);
impl BlockHashRead for ChainHashes<'_> {
    fn block_hash(&self, number: FinalChainBlockNumber) -> Result<[u8; 32], BlockHashReadError> {
        self.0
            .block_hash(number)
            .map_err(|e| BlockHashReadError::Io(e.to_string()))?
            .ok_or(BlockHashReadError::HistoryUnavailable(number))?
            .try_into()
            .map_err(|_| BlockHashReadError::Corrupt("block hash width".into()))
    }
}
/// Fixture classifier selects the same stateless registry as its driver.
struct AllNatives<'a>(TaraxaProfile, &'a FinalChain);
impl NativeAddressClassifier for AllNatives<'_> {
    fn is_native_address(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        use rustaxa_evm::bls::{BlsPrecompile, BlsRegistry};
        self.1.native_contract_kind(period, address).is_some()
            || (address[..19] == [0; 19]
                && (matches!(address[19], 1..=8) || (self.0.ficus() && address[19] == 9)))
            || (self.0.ficus()
                && BlsPrecompile::at_address(
                    if self.0.cacti() {
                        BlsRegistry::Cacti
                    } else {
                        BlsRegistry::Ficus
                    },
                    address,
                )
                .is_some())
            || (self.0.cacti()
                && matches!(
                    address,
                    rustaxa_evm::p256::P256_VERIFY_ADDRESS
                        | rustaxa_evm::falcon::FALCON_VERIFY_ADDRESS
                ))
    }
}
/// Consensus-owned subset; stateless addresses are handled directly by the driver.
struct ConsensusNatives<'a>(&'a FinalChain);
impl NativeAddressClassifier for ConsensusNatives<'_> {
    fn is_native_address(&self, period: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.0.native_contract_kind(period, address).is_some()
    }
}
#[path = "support/ordinary_observer.rs"]
mod ordinary_observer;
type OrdinaryObserver = std::rc::Rc<RefCell<ordinary_observer::KnownOrdinaryStorageIdentities>>;

fn is_second_distribution_scenario(fixture: &Value) -> bool {
    matches!(
        fixture["scenario"].as_str(),
        Some(
            "mainnet_second_distribution_followthrough"
                | "mainnet_second_distribution_reopen"
                | "mainnet_second_distribution_followthrough_recovery"
        )
    )
}

struct Staged {
    prepared: ConcreteObserverPhaseOutput,
    system_accounts: BTreeSet<[u8; 20]>,
    projection: FinalChainConcreteStateProjection,
    provenance: Vec<u8>,
    catalog_rlp: Vec<u8>,
}

struct NativeContextParts {
    request_id: [u8; 32],
    projection_hash: [u8; 32],
    authority: rustaxa_consensus::native_projection_context::FinalChainLifecycleAuthority,
    delayed: std::rc::Rc<rustaxa_consensus::native_session::frozen_eligibility::FrozenEligibility>,
    lifecycle: Vec<rustaxa_consensus::native_projection_context::FinalChainLifecycleReceipt>,
    invocations: Vec<FinalChainNativeInvocationContext>,
    rewards: FinalChainNativeRewardsContext,
    raw_slots: BTreeSet<ConcreteStorageSlot>,
    raw_read_slots: BTreeSet<ConcreteStorageSlot>,
}

fn raw_read_slots<'a>(
    reads: impl Iterator<
        Item = &'a rustaxa_consensus::native_projection_context::FinalChainNativeRawRead,
    >,
) -> BTreeSet<ConcreteStorageSlot> {
    reads
        .map(|read| ConcreteStorageSlot {
            address: read.address,
            key: read.key.0,
        })
        .collect()
}

/// Keep replay reads separate from storage mutations. A read proves an input
/// to native execution; it does not itself add a physical catalog identity.
fn native_raw_observations(
    contexts: &[FinalChainNativeInvocationContext],
) -> (BTreeSet<ConcreteStorageSlot>, BTreeSet<ConcreteStorageSlot>) {
    let reads = raw_read_slots(contexts.iter().flat_map(|context| context.raw_reads.iter()));
    let writes = contexts
        .iter()
        .flat_map(|context| context.raw_mutations.iter())
        .map(|mutation| ConcreteStorageSlot {
            address: mutation.address,
            key: mutation.key.0,
        })
        .collect();
    (reads, writes)
}

#[test]
fn native_read_only_slots_remain_replay_evidence_without_catalog_promotion() {
    use rustaxa_consensus::native_projection_context::FinalChainNativeRawRead;
    use rustaxa_consensus::native_session::{
        FinalChainNativeCallKind, FinalChainNativeInvocationId, FinalChainNativeRawMutation,
        FinalChainNativeRawOperation, FinalChainNativeRawValue, FinalChainNativeRequest,
        FinalChainNativeValue,
    };
    let address = [0xfe; 20];
    let absent = ConcreteStorageKey([1; 32]);
    let present = ConcreteStorageKey([2; 32]);
    let absent_then_put = ConcreteStorageKey([9; 32]);
    let deleted = ConcreteStorageKey([3; 32]);
    let noop = ConcreteStorageKey([6; 32]);
    let empty = ConcreteStorageKey([8; 32]);
    let prior = ConcreteStorageSlot {
        address,
        key: [4; 32],
    };
    let probe = ConcreteStorageSlot {
        address,
        key: [5; 32],
    };
    let request = FinalChainNativeRequest {
        id: FinalChainNativeInvocationId {
            transaction: 0.into(),
            sequence: 0,
        },
        period: 1.into(),
        depth: 1,
        kind: FinalChainNativeCallKind::Call,
        is_static: false,
        caller: [6; 20],
        contract: address,
        state_address: address,
        value: FinalChainNativeValue::default(),
        input: Vec::new(),
        supplied_gas: 100_000.into(),
    };
    let context = FinalChainNativeInvocationContext {
        request,
        accounts: Vec::new(),
        raw_reads: vec![
            FinalChainNativeRawRead {
                address,
                key: absent,
                value: ConcreteRead::Absent,
            },
            FinalChainNativeRawRead {
                address,
                key: present,
                value: ConcreteRead::Present(vec![7]),
            },
            FinalChainNativeRawRead {
                address,
                key: absent_then_put,
                value: ConcreteRead::Absent,
            },
        ],
        ordinary_mutations: Vec::new(),
        raw_mutations: vec![
            FinalChainNativeRawMutation {
                address,
                key: absent_then_put,
                expected: ConcreteRead::Absent,
                operation: FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(vec![10]).unwrap(),
                ),
            },
            FinalChainNativeRawMutation {
                address,
                key: deleted,
                expected: ConcreteRead::Present(vec![8]),
                operation: FinalChainNativeRawOperation::Delete,
            },
            FinalChainNativeRawMutation {
                address,
                key: deleted,
                expected: ConcreteRead::Tombstone,
                operation: FinalChainNativeRawOperation::Delete,
            },
            FinalChainNativeRawMutation {
                address,
                key: noop,
                expected: ConcreteRead::Present(vec![9]),
                operation: FinalChainNativeRawOperation::Put(
                    FinalChainNativeRawValue::new(vec![9]).unwrap(),
                ),
            },
            FinalChainNativeRawMutation {
                address,
                key: empty,
                expected: ConcreteRead::Present(Vec::new()),
                operation: FinalChainNativeRawOperation::Delete,
            },
        ],
    };
    let original = context.clone();
    let (reads, writes) = native_raw_observations(std::slice::from_ref(&context));
    assert_eq!(
        context, original,
        "replay inputs and ordered effects remain intact"
    );
    assert_eq!(
        reads,
        BTreeSet::from([
            ConcreteStorageSlot {
                address,
                key: absent.0
            },
            ConcreteStorageSlot {
                address,
                key: present.0
            },
            ConcreteStorageSlot {
                address,
                key: absent_then_put.0
            },
        ])
    );
    assert_eq!(
        writes,
        BTreeSet::from([
            ConcreteStorageSlot {
                address,
                key: absent_then_put.0
            },
            ConcreteStorageSlot {
                address,
                key: deleted.0
            },
            ConcreteStorageSlot {
                address,
                key: noop.0
            },
            ConcreteStorageSlot {
                address,
                key: empty.0
            }
        ])
    );
    assert_eq!(context.raw_reads[2].key, absent_then_put);
    assert_eq!(context.raw_reads[2].value, ConcreteRead::Absent);
    assert_eq!(context.raw_mutations[0].key, absent_then_put);
    assert_eq!(context.raw_mutations[0].expected, ConcreteRead::Absent);
    assert_eq!(
        context.raw_mutations[0].operation,
        FinalChainNativeRawOperation::Put(FinalChainNativeRawValue::new(vec![10]).unwrap(),)
    );
    let written = ConcreteStorageSlot {
        address,
        key: absent_then_put.0,
    };
    assert!(
        reads.contains(&written),
        "absent pre-write read remains replay evidence"
    );
    assert!(
        writes.contains(&written),
        "subsequent put produces a catalog identity"
    );
    let reward_key = ConcreteStorageKey([7; 32]);
    let reward = FinalChainNativeRewardsContext {
        raw_reads: vec![FinalChainNativeRawRead {
            address,
            key: reward_key,
            value: ConcreteRead::Absent,
        }],
        ..Default::default()
    };
    let reward_before = reward.clone();
    let reward_reads = raw_read_slots(reward.raw_reads.iter());
    assert_eq!(reward, reward_before, "reward replay facts remain intact");
    assert_eq!(
        reward_reads,
        BTreeSet::from([ConcreteStorageSlot {
            address,
            key: reward_key.0
        }])
    );
    let mut catalog = BTreeSet::from([prior, probe]);
    catalog.extend(writes);
    assert_eq!(
        catalog,
        BTreeSet::from([
            prior,
            probe,
            ConcreteStorageSlot {
                address,
                key: absent_then_put.0
            },
            ConcreteStorageSlot {
                address,
                key: deleted.0
            },
            ConcreteStorageSlot {
                address,
                key: noop.0
            },
            ConcreteStorageSlot {
                address,
                key: empty.0
            }
        ])
    );
    assert!(
        catalog.contains(&written),
        "absent-then-put remains catalogued"
    );
    assert!(!catalog.contains(&ConcreteStorageSlot {
        address,
        key: absent.0
    }));
    assert!(!catalog.contains(&ConcreteStorageSlot {
        address,
        key: present.0
    }));
    assert!(!catalog.contains(&ConcreteStorageSlot {
        address,
        key: reward_key.0
    }));
}

/// Exact C bridge fee product (uint256), followed by wide ordinary addition.
/// The owner selects the phase; standalone driver/query paths never call this.
fn c_composed_transaction_fee(gas_price: &BigUint, gas_used: u64) -> BigUint {
    let modulus = BigUint::from(1_u8) << 256_usize;
    ((gas_price % &modulus) * BigUint::from(gas_used)) % modulus
}

fn credit_c_composed_transaction_fee<
    R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead,
>(
    journal: &mut ExecutionJournal<R>,
    author: [u8; 20],
    gas_price: &BigUint,
    gas_used: u64,
) -> Result<()> {
    // Even a zero fee follows the journal's actual touch/create/prune path.
    journal.add_balance(author, &c_composed_transaction_fee(gas_price, gas_used))?;
    Ok(())
}

#[derive(Clone, Copy)]
enum AccountNodePublication {
    ObserverHistory,
    FinalPeriod {
        limits: rustaxa_storage::ConcreteStorageInventoryLimits,
    },
}

// The duration owner arms this recorder only for one synchronous dispatch.
#[derive(Default)]
struct DurationBeginObservation {
    completion: Option<([u8; 32], u64, u64, [u8; 32])>,
    updates: Vec<(ConcreteObserverPhaseDelta, [u8; 32], bool)>,
}
thread_local! {
    static DURATION_BEGIN: RefCell<Option<DurationBeginObservation>> = const { RefCell::new(None) };
}
fn duration_begin_arm() -> Result<()> {
    DURATION_BEGIN.with(|slot| {
        let mut slot = slot.borrow_mut();
        ensure!(slot.is_none(), "duration begin recorder already armed");
        *slot = Some(DurationBeginObservation::default());
        Ok(())
    })
}
fn duration_begin_take() -> Result<DurationBeginObservation> {
    DURATION_BEGIN.with(|slot| {
        slot.borrow_mut()
            .take()
            .context("duration begin recorder not armed")
    })
}
fn duration_begin_dispatch(request: &FinalChainEvmExecutionRequest) -> Result<()> {
    DURATION_BEGIN.with(|slot| {
        if let Some(observation) = slot.borrow_mut().as_mut() {
            ensure!(observation.completion.is_none(), "duplicate begin dispatch");
            observation.completion = Some((
                request.request_id,
                request.period.as_u64(),
                request.prior_state.period.as_u64(),
                request.prior_state.state_root,
            ));
        }
        Ok(())
    })
}
fn duration_begin_update(
    delta: ConcreteObserverPhaseDelta,
    root: [u8; 32],
    created: bool,
) -> Result<()> {
    DURATION_BEGIN.with(|slot| {
        if let Some(observation) = slot.borrow_mut().as_mut() {
            ensure!(
                observation.completion.is_some(),
                "begin update before dispatch"
            );
            observation.updates.push((delta, root, created));
        }
        Ok(())
    })
}

fn duration_capture_plan(
    delta: &ConcreteObserverPhaseDelta,
    root: [u8; 32],
    role: u8,
    position: u32,
) {
    if !rustaxa_evm::journal::duration_journal_is_armed() {
        return;
    }
    let mut stage = vec![role];
    stage.extend_from_slice(&position.to_be_bytes());
    rustaxa_evm::journal::duration_journal_plan_event("plan_begin", [0; 20], &root, &stage);
    for item in &delta.accounts {
        match item {
            ConcreteAccountMutation::Upsert { address, record } => {
                let mut row = vec![2];
                row.extend_from_slice(&record.physical_rlp);
                rustaxa_evm::journal::duration_journal_plan_event(
                    "plan_account",
                    *address,
                    &root,
                    &row,
                );
            }
            ConcreteAccountMutation::Delete { address } => {
                rustaxa_evm::journal::duration_journal_plan_event(
                    "plan_account",
                    *address,
                    &root,
                    &[1],
                )
            }
        }
    }
    for item in &delta.storage {
        let mut row = vec![u8::from(item.value.is_some())];
        if let Some(value) = &item.value {
            row.extend_from_slice(value);
        }
        rustaxa_evm::journal::duration_journal_plan_event(
            "plan_storage",
            item.address,
            &item.key.0,
            &row,
        );
    }
    for item in &delta.code {
        rustaxa_evm::journal::duration_journal_plan_event(
            "plan_code",
            [0; 20],
            &item.code_hash,
            &item.code,
        );
    }
    rustaxa_evm::journal::duration_journal_plan_event("plan_end", [0; 20], &root, &stage);
}

fn duration_capture_prepared(
    concrete: &ConcreteStateLifecycle,
    output: &ConcreteObserverPhaseOutput,
    delta: &ConcreteObserverPhaseDelta,
) -> Result<()> {
    use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
    if !rustaxa_evm::journal::duration_journal_is_armed() {
        return Ok(());
    }
    let root = output.identity().state_root;
    let view = concrete.prepared_view(output)?;
    let mut owners = BTreeSet::new();
    for item in &delta.accounts {
        match item {
            ConcreteAccountMutation::Upsert { address, .. }
            | ConcreteAccountMutation::Delete { address } => {
                owners.insert(*address);
            }
        }
    }
    for item in &delta.storage {
        owners.insert(item.address);
    }
    for address in owners {
        let read = view.account(address)?;
        let mut value = Vec::new();
        match read {
            ConcreteRead::Absent => value.push(0),
            ConcreteRead::Tombstone => value.push(1),
            ConcreteRead::Present(record) => {
                value.push(2);
                value.extend_from_slice(&record.physical_rlp);
            }
        }
        rustaxa_evm::journal::duration_journal_plan_event(
            "prepared_plan_account",
            address,
            &root,
            &value,
        );
    }
    for item in &delta.storage {
        let read = view.storage(item.address, item.key)?;
        let mut value = Vec::new();
        match read {
            ConcreteRead::Absent => value.push(0),
            ConcreteRead::Tombstone => value.push(1),
            ConcreteRead::Present(bytes) => {
                value.push(2);
                value.extend_from_slice(&bytes);
            }
        }
        rustaxa_evm::journal::duration_journal_plan_event(
            "prepared_storage",
            item.address,
            &item.key.0,
            &value,
        );
    }
    for item in &delta.code {
        let read = view.code(item.code_hash)?;
        let mut value = Vec::new();
        match read {
            ConcreteRead::Absent => value.push(0),
            ConcreteRead::Tombstone => value.push(1),
            ConcreteRead::Present(bytes) => {
                value.push(2);
                value.extend_from_slice(&bytes);
            }
        }
        rustaxa_evm::journal::duration_journal_plan_event(
            "prepared_code",
            [0; 20],
            &item.code_hash,
            &value,
        );
    }
    for change in output.changed_accounts() {
        match &change.account {
            ConcreteRead::Absent => rustaxa_evm::journal::duration_journal_prepared_account(
                change.address,
                root,
                0,
                &[],
            ),
            ConcreteRead::Tombstone => rustaxa_evm::journal::duration_journal_prepared_account(
                change.address,
                root,
                1,
                &[],
            ),
            ConcreteRead::Present(record) => {
                rustaxa_evm::journal::duration_journal_prepared_account(
                    change.address,
                    root,
                    2,
                    &record.physical_rlp,
                )
            }
        }
    }
    rustaxa_evm::journal::duration_journal_prepared_stage(false, root);
    Ok(())
}

struct Adapter<'a> {
    chain: &'a FinalChain,
    application: &'a Storage,
    concrete: RefCell<Option<ConcreteStateLifecycle>>,
    staged: RefCell<Option<Staged>>,
    native: RefCell<Option<MixedNativeExecutionPort<'a>>>,
    native_context: RefCell<Option<NativeContextParts>>,
    physical_receipt:
        Option<std::rc::Rc<RefCell<Option<rustaxa_storage::ConcreteCommittedReceipt>>>>,
    state_api_epoch: StateApiEpoch,
    ordinary_observer: Option<OrdinaryObserver>,
    fixture: &'a Value,
    account_node_publication: AccountNodePublication,
}
impl ConsensusExecutionPort for Adapter<'_> {
    fn load_final_chain_committed_state(
        &self,
        request: &FinalChainExternalEvmPreflightRequest,
    ) -> Result<FinalChainExternalEvmPreflightReport> {
        let concrete = self.concrete.borrow();
        let observed = concrete
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable after commit"))?
            .observation()?;
        ensure!(
            observed.identity.chain_id == request.concrete_chain_identity,
            "fixture chain identity"
        );
        Ok(FinalChainExternalEvmPreflightReport {
            request_id: request.request_id,
            state_api_epoch: self.state_api_epoch.current(),
            committed: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            concrete_provenance_rlp: observed.provenance_rlp,
            pending_concrete_marker_rlp: observed.pending_marker_rlp,
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_system_transaction_facts(
        &self,
        request: &FinalChainSystemTransactionFactsRequest,
    ) -> Result<FinalChainSystemTransactionPlanFact> {
        let concrete = self.concrete.borrow();
        let account = concrete
            .as_ref()
            .unwrap()
            .prior_reader()
            .account(request.bridge_contract_address)?;
        ensure!(
            matches!(account, ConcreteRead::Absent | ConcreteRead::Tombstone),
            "fixture bridge is not absent"
        );
        Ok(FinalChainSystemTransactionPlanFact {
            request_id: request.request_id,
            period: request.period,
            is_pillar_block_period: request.is_pillar_block_period,
            bridge_contract_address: request.bridge_contract_address,
            bridge_contract_found: false,
            bridge_contract_has_code: false,
            should_finalize_epoch: false,
            system_account_nonce: FinalChainNonce::zero(),
            block_gas_limit: request.block_gas_limit,
        })
    }
    fn execute_final_chain_transactions(
        &self,
        request: &FinalChainEvmExecutionRequest,
    ) -> Result<FinalChainEvmExecutionReport> {
        self.state_api_epoch.validate(request.state_api_epoch)?;
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable"))?;
        concrete.stage_execution(&request.concrete_marker_rlp)?;
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        let observed = concrete.observation()?;
        ensure!(
            marker.identity == observed.identity && marker.generation == observed.generation + 1,
            "fixture delayed owner marker mismatch"
        );
        if let Some(observer) = &self.ordinary_observer {
            let observed = concrete.observation()?;
            ensure!(
                marker.identity == observed.identity
                    && marker.generation == observed.generation + 1
            );
            observer
                .borrow_mut()
                .bind_marker(&request.concrete_marker_rlp)?;
        }
        let expected = self.fixture["transactions"]
            .as_array()
            .context("period transactions")?;
        ensure!(
            request.transactions.len() == expected.len(),
            "fixture transaction count"
        );

        let selected = self
            .chain
            .pending_delayed_identity(request.period, request.prior_state.period)?;
        let reader = std::sync::Arc::new(concrete.open_retained_checkpoint(selected).map_err(|error| {
            #[cfg(feature = "synthetic-fixtures")]
            if std::env::var_os("RUST_LIB_BACKTRACE").as_deref() == Some(std::ffi::OsStr::new("1")) {
                eprintln!("sst_diagnostic checkpoint_error selected={selected:?} committed_before_open={:?} marker_generation={:?} writer_live_before_open=true view_live_before_open=false error={error:?}", observed.committed, marker.generation);
            }
            error
        })?);
        let delayed = self.chain.bind_pending_delayed_view(
            request.request_id,
            request.state_api_epoch,
            marker.generation,
            request.period,
            request.prior_state.period,
            reader,
        )?;
        let mut session = self.chain.begin_native_session_bound_at_state_api_epoch(
            request.request_id,
            request.state_api_epoch,
            request.period,
            request.prior_state.period,
        )?;
        // Constructor completion occurs before any observer phase or transaction.
        duration_begin_dispatch(request)?;
        session.install_pending_delayed_view(std::rc::Rc::clone(&delayed))?;
        let mut native = MixedNativeExecutionPort::new(session);
        let mut sequence = PeriodConsensusSequence::new(request.period);
        let mut prepared: Option<ConcreteObserverPhaseOutput> = None;
        let mut identity = concrete.observation()?.committed;
        // Protocol code changes precede every transaction, including periods
        // without transactions. The owner selects rules from chain configuration;
        // fixture output is only a later comparison oracle.
        let dpos_address = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
        ];
        let dpos_code_empty = match concrete.prior_reader().account(dpos_address)? {
            ConcreteRead::Present(record) => record.account.code_size == 0,
            ConcreteRead::Absent | ConcreteRead::Tombstone => true,
        };
        let updates = self
            .chain
            .native_code_updates(request.period, dpos_code_empty);
        let mut system_accounts = BTreeSet::new();
        // Apply one replacement at a time to preserve Aspen then Cornus when
        // both are configured for the same period.
        for update in updates {
            let prior = match prepared.as_ref() {
                Some(output) => {
                    rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                        &concrete.prepared_view(output)?,
                        update.address,
                    )?
                }
                None => concrete.prior_reader().account(update.address)?,
            };
            let created = matches!(&prior, ConcreteRead::Absent | ConcreteRead::Tombstone);
            let (nonce, balance) = match prior {
                ConcreteRead::Present(record) => (record.account.nonce, record.account.balance),
                ConcreteRead::Absent | ConcreteRead::Tombstone => (
                    FinalChainNonce::zero(),
                    ConcreteAccountBalance::new(BigUint::default()),
                ),
            };
            let hash = keccak256(update.code).0;
            let delta = ConcreteObserverPhaseDelta {
                accounts: vec![ConcreteAccountMutation::Upsert {
                    address: update.address,
                    record: record(nonce, balance, Some(hash), update.code.len() as u64),
                }],
                storage: Vec::new(),
                code: vec![ConcreteCodeInsertion {
                    code_hash: hash,
                    code: update.code.to_vec(),
                }],
            };
            duration_capture_plan(&delta, identity.state_root, 0, 0);
            rustaxa_evm::journal::duration_journal_prepared_stage(true, identity.state_root);
            let next = concrete.apply_observer_phase(delta.clone())?;
            duration_capture_prepared(concrete, &next, &delta)?;
            duration_begin_update(delta, next.identity().state_root, created)?;
            system_accounts.insert(update.address);
            identity = next.identity();
            prepared = Some(next);
        }
        let mut raw_catalog =
            decode_concrete_storage_catalog(&concrete.observation()?.catalog_rlp)?
                .into_iter()
                .collect::<BTreeSet<_>>();
        let mut native_contexts = Vec::new();
        let mut effects = Vec::new();
        let mut results = Vec::new();
        let mut cumulative = 0_u64;

        #[cfg(feature = "synthetic-fixtures")]
        let mut origin_report = None;
        #[cfg(feature = "synthetic-fixtures")]
        let origin_scope = std::env::var_os("RUSTAXA_NATIVE_LINEAGE_DIAGNOSTIC").as_deref()
            == Some(std::ffi::OsStr::new("invalid_signature_a"))
            && hex::encode(request.request_id)
                == "2fda07600a98939e6c86070d81711d6b93c5417aface40e5e378c25a7073d388"
            && request.period.as_u64() == 5
            && marker.generation == 5
            && marker.prior_state.period == 4
            && hex::encode(marker.prior_state.root)
                == "b934d70f36b020f10434857a9512ab00832370bfef875438cdb6e7ee58dd2b8e";

        for (transaction, expected) in request.transactions.iter().zip(expected) {
            ensure!(
                transaction.rlp == bytes(&expected["signed_rlp"]),
                "fixture signed input"
            );
            ensure!(!transaction.is_system, "fixture unexpected system input");
            let decoded = decode_legacy_input(
                transaction.position,
                &transaction.rlp,
                LegacyInputKind::Signed,
            )?;
            ensure!(
                decoded.hash == transaction.hash && decoded.sender == transaction.sender,
                "selected transaction identity"
            );
            let view = FixtureView {
                prior: match prepared.as_ref() {
                    Some(output) => FixturePrior::Prepared(concrete.prepared_view(output)?),
                    None => FixturePrior::Lifecycle(concrete),
                },
                identity,
            };
            #[cfg(feature = "synthetic-fixtures")]
            let origin = if origin_scope && transaction.position.as_u32() == 5 {
                use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
                diagnostic::start().ok().and_then(|guard| {
                    rustaxa_evm::journal::cleanup_diagnostic_observer::install(
                        diagnostic::is_active,
                        diagnostic::event,
                        diagnostic::overflow,
                        true,
                    )
                    .ok()
                    .map(|observer| (guard, observer))
                })
            } else {
                None
            };
            let mut journal = ExecutionJournal::new(view);
            let expected_receipt_token = journal.begin_lifecycle_receipt()?;
            let block = ExecutionBlockContext {
                period: request.period,
                author: request.block_author,
                timestamp: request.timestamp,
                gas_limit: request.block_gas_limit,
                chain_id: number(&expected["chain_id"]),
                difficulty: BigUint::default(),
            };
            let result = match decoded.kind {
                ExecutionTransactionKind::Call => execute_top_level_call_with_native(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &AllNatives(fixture_execution_profile(self.fixture), self.chain),
                    &ConsensusNatives(self.chain),
                    &mut native,
                    &mut sequence,
                    &block,
                    &decoded,
                    EnvelopeRules {
                        cornus: self.fixture["rules"]["cornus"].as_bool().unwrap_or(true),
                    },
                    fixture_execution_profile(self.fixture),
                )
                .with_context(|| {
                    format!(
                        "fixture transaction {} ({}) at period {}",
                        string(&expected["name"]),
                        hex::encode(decoded.hash),
                        request.period.as_u64(),
                    )
                })?,
                ExecutionTransactionKind::Create => execute_top_level_create_with_native(
                    &mut journal,
                    &ChainHashes(self.chain),
                    &AllNatives(fixture_execution_profile(self.fixture), self.chain),
                    &ConsensusNatives(self.chain),
                    &mut native,
                    &mut sequence,
                    &block,
                    &decoded,
                    EnvelopeRules {
                        cornus: self.fixture["rules"]["cornus"].as_bool().unwrap_or(true),
                    },
                    fixture_execution_profile(self.fixture),
                )
                .with_context(|| {
                    format!(
                        "fixture transaction {} ({}) at period {}",
                        string(&expected["name"]),
                        hex::encode(decoded.hash),
                        request.period.as_u64(),
                    )
                })?,
                other => bail!("fixture unsupported transaction kind: {other:?}"),
            };
            let TransactionExecutionResult::Executed(result) = result else {
                bail!("fixture consensus failure")
            };
            if request.period.as_u64() == 4 {
                eprintln!(
                    "CRYPTO_TX_DIAGNOSTIC {}",
                    serde_json::json!({
                        "period": request.period.as_u64(), "position": transaction.position.as_u32(),
                        "name": expected["name"], "transaction_hash": hex::encode(transaction.hash),
                        "actual_error": code_error(&result.status), "expected_error": expected["execution_error"],
                        "actual_output": hex::encode(&result.output), "expected_output": expected["output"],
                        "actual_gas_used": result.gas_used.as_u64(), "expected_gas_used": expected["gas_used"],
                        "actual_refund": result.refund_applied.as_u64(), "expected_refund": expected["gas_refund"]["applied"],
                        "prior_phase_period": identity.period.as_u64(), "prior_phase_root": hex::encode(identity.state_root),
                        "expected_observer_root": expected["observer"]["root"],
                        "expected_final_root": self.fixture["final"]["root"],
                        "actual_published_root": "NOT_OBSERVED", "published_receipts": "NOT_OBSERVED"
                    })
                );
            }
            let expected_error = string(&expected["execution_error"]);
            ensure!(
                code_error(&result.status) == expected_error,
                "Go execution status for {}: actual {:?}, expected {}",
                string(&expected["name"]),
                code_error(&result.status),
                expected_error
            );
            ensure!(
                result.output == bytes(&expected["output"]),
                "Go execution output for {} input {}: actual {}, expected {}",
                string(&expected["name"]),
                hex::encode(&decoded.input),
                hex::encode(&result.output),
                string(&expected["output"])
            );
            ensure!(
                result.attempted_contract_address.unwrap_or([0; 20])
                    == fixed_or_zero(&expected["created"]),
                "Go created address"
            );
            ensure!(
                result.gas_used.as_u64() == number(&expected["gas_used"]),
                "Go gas used"
            );
            ensure!(
                result.refund_applied.as_u64() == number(&expected["gas_refund"]["applied"]),
                "Go applied refund"
            );
            if self.chain.pre_magnolia_fee_reward_period(request.period) {
                credit_c_composed_transaction_fee(
                    &mut journal,
                    request.block_author,
                    decoded.gas_price.value(),
                    result.gas_used.as_u64(),
                )?;
            }
            let settled = journal.settle_transaction()?;
            native.retain_receipt(
                expected_receipt_token,
                settled
                    .lifecycle_receipt
                    .clone()
                    .context("missing executor lifecycle receipt")?,
            )?;
            #[cfg(feature = "synthetic-fixtures")]
            if origin.is_some() {
                use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
                let mut address = [0_u8; 20];
                address[19] = 0xee;
                let mut key = [0_u8; 32];
                key[31] = 2;
                let selected = settled
                    .writes
                    .raw_storage
                    .iter()
                    .find(|row| row.address == address && row.key.0 == key);
                let account = settled
                    .writes
                    .accounts
                    .iter()
                    .find(|row| row.address == address);
                diagnostic::event_fmt(
                    "origin_settled",
                    format_args!(
                        "tx=5 selected={selected:?} account={account:?} later_effects=NOT_RETAINED"
                    ),
                );
            }
            drop(journal);
            ensure!(
                settled.logs == result.logs,
                "settled logs differ from result"
            );
            ensure!(
                settled.refund == number(&expected["gas_refund"]["counter"]),
                "Go refund counter"
            );

            let call_contexts = native.take_contexts();
            ensure!(
                call_contexts.len() == settled.native_invocations.len(),
                "native context/observation count"
            );
            #[cfg(feature = "synthetic-fixtures")]
            if origin.is_some() {
                use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
                let mut address = [0_u8; 20];
                address[19] = 0xee;
                let mut key = [0_u8; 32];
                key[31] = 2;
                diagnostic::event_fmt(
                    "origin_transaction",
                    format_args!("tx=5 status={:?}", result.status),
                );
                for (context_index, (context, observation)) in call_contexts
                    .iter()
                    .zip(&settled.native_invocations)
                    .enumerate()
                {
                    let selected = context
                        .raw_mutations
                        .iter()
                        .any(|row| row.address == address && row.key.0 == key);
                    if context.request.contract == address
                        || observation.invocation.contract == address
                        || selected
                    {
                        diagnostic::event_fmt(
                            "origin_invocation",
                            format_args!(
                                "context_index={context_index} context_id={:?} observed_id={:?} contract={:?} status={:?} disposition={:?} setter_link=AMBIGUOUS",
                                context.request.id,
                                observation.invocation.id,
                                observation.invocation.contract,
                                observation.status,
                                observation.disposition
                            ),
                        );
                    }
                    for (mutation_index, mutation) in context.raw_mutations.iter().enumerate() {
                        if mutation.address == address && mutation.key.0 == key {
                            diagnostic::event_fmt(
                                "origin_mutation",
                                format_args!(
                                    "context_index={context_index} mutation_index={mutation_index} context_id={:?} expected={:?} operation={:?}",
                                    context.request.id, mutation.expected, mutation.operation
                                ),
                            );
                        }
                    }
                }
            }

            compare_raw_trace(&call_contexts, &expected["ordered_raw_writes"]).with_context(
                || {
                    format!(
                        "transaction {} native raw writes",
                        string(&expected["name"])
                    )
                },
            )?;
            if request.period.as_u64() == 4 {
                for (context_index, context) in call_contexts.iter().enumerate() {
                    for (index, row) in context.raw_mutations.iter().enumerate() {
                        let operation = match &row.operation {
                            rustaxa_consensus::native_session::FinalChainNativeRawOperation::Put(value) => serde_json::json!({"type":"put","value":hex::encode(value.as_bytes())}),
                            rustaxa_consensus::native_session::FinalChainNativeRawOperation::Delete => serde_json::json!({"type":"delete"}),
                        };
                        let before = match &row.expected {
                            ConcreteRead::Present(value) => {
                                serde_json::json!({"state":"present","value":hex::encode(value)})
                            }
                            ConcreteRead::Absent => serde_json::json!({"state":"absent"}),
                            ConcreteRead::Tombstone => serde_json::json!({"state":"tombstone"}),
                        };
                        eprintln!(
                            "CRYPTO_NATIVE_DIAGNOSTIC {}",
                            serde_json::json!({"period":4,"position":transaction.position.as_u32(),"name":expected["name"],"context_index":context_index,"index":index,"address":hex::encode(row.address),"key":hex::encode(row.key.0),"before":before,"operation":operation})
                        );
                    }
                }
            }
            native_contexts.extend(call_contexts);

            cumulative = cumulative
                .checked_add(result.gas_used.as_u64())
                .context("cumulative gas overflow")?;
            ensure!(
                cumulative == number(&expected["cumulative_gas_used"]),
                "Go cumulative gas"
            );
            let mut report = FinalChainEvmTransactionResult {
                position: transaction.position,
                hash: transaction.hash,
                status: u8::from(result.status == CodeExecutionStatus::Success),
                gas_used: result.gas_used,
                cumulative_gas_used: cumulative.into(),
                receipt_rlp: Vec::new(),
                logs: result.logs.iter().map(project_log).collect(),
                new_contract_address: result.attempted_contract_address,
                output: result.output.clone(),
                code_error: expected_error.to_owned(),
                consensus_error: String::new(),
            };
            report.receipt_rlp = encode_external_evm_receipt(&report);
            ensure!(
                report.receipt_rlp == bytes(&expected["receipt_rlp"]),
                "Go receipt bytes"
            );
            ensure!(
                encode_concrete_evm_transaction(transaction)
                    == bytes(&expected["state_api_transaction_rlp"]),
                "Go StateAPI transaction bytes"
            );
            ensure!(
                encode_concrete_execution_result(&report)
                    == bytes(&expected["state_api_execution_result_rlp"]),
                "Go StateAPI execution-result bytes"
            );

            let mut changed = BTreeSet::new();
            let mut accounts = Vec::new();
            let mut storage = Vec::new();
            let mut code = Vec::new();
            for write in settled.writes.accounts {
                changed.insert(write.address);
                accounts.push(match write.operation {
                    JournalAccountOperation::Upsert {
                        nonce,
                        balance,
                        code_hash,
                        code_size,
                    } => ConcreteAccountMutation::Upsert {
                        address: write.address,
                        record: record(nonce, balance, code_hash, code_size),
                    },
                    JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                        address: write.address,
                    },
                });
            }
            for write in settled.writes.ordinary_storage {
                changed.insert(write.address);
                if let Some(observer) = &self.ordinary_observer {
                    observer.borrow_mut().record_settled(ConcreteStorageSlot {
                        address: write.address,
                        key: write.key.0,
                    })?;
                }
                storage.push(ConcreteStorageMutation {
                    address: write.address,
                    key: write.key,
                    value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
                });
            }
            let mut raw_touched = BTreeSet::new();
            for write in settled.writes.raw_storage {
                changed.insert(write.address);
                raw_touched.insert((write.address, write.key));
                raw_catalog.insert(ConcreteStorageSlot {
                    address: write.address,
                    key: write.key.0,
                });
                storage.push(ConcreteStorageMutation {
                    address: write.address,
                    key: write.key,
                    value: raw_operation_value(&write.operation),
                });
            }
            for write in settled.writes.code {
                code.push(ConcreteCodeInsertion {
                    code_hash: write.code_hash,
                    code: write.code,
                });
            }
            #[cfg(feature = "synthetic-fixtures")]
            if let Some((guard, observer)) = origin {
                use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
                let mut address = [0_u8; 20];
                address[19] = 0xee;
                let mut key = [0_u8; 32];
                key[31] = 2;
                let mut count = 0_usize;
                for (index, row) in storage.iter().enumerate() {
                    if row.address == address && row.key.0 == key {
                        diagnostic::event_fmt(
                            "origin_delta",
                            format_args!("tx=5 index={index} selected={row:?}"),
                        );
                        count += 1;
                    }
                }
                diagnostic::event_fmt("origin_delta", format_args!("tx=5 matching_count={count}"));
                drop(observer);
                origin_report = Some(guard.finish());
            }
            let delta = ConcreteObserverPhaseDelta {
                accounts,
                storage,
                code,
            };
            duration_capture_plan(
                &delta,
                identity.state_root,
                1,
                transaction.position.as_u32(),
            );
            rustaxa_evm::journal::duration_journal_prepared_stage(true, identity.state_root);
            let next = concrete.apply_observer_phase(delta.clone())?;
            duration_capture_prepared(concrete, &next, &delta)?;
            identity = next.identity();
            if request.period.as_u64() == 4 {
                eprintln!(
                    "CRYPTO_ROOT_DIAGNOSTIC {}",
                    serde_json::json!({"period":4,"position":transaction.position.as_u32(),"name":expected["name"],"actual":hex::encode(identity.state_root),"expected":expected["observer"]["root"]})
                );
            }
            ensure!(
                identity.state_root == fixed(&expected["observer"]["root"]),
                "Go intermediate root"
            );
            let observed_changed = next
                .changed_accounts()
                .iter()
                .map(|entry| entry.address)
                .collect::<BTreeSet<_>>();
            ensure!(
                observed_changed.is_subset(&changed),
                "writer reported an account absent from the settled delta period={} tx={}: observed={observed_changed:?} settled={changed:?}",
                request.period.as_u64(),
                transaction.position.as_u32(),
            );
            let view = FixtureView {
                prior: FixturePrior::Prepared(concrete.prepared_view(&next)?),
                identity: next.identity(),
            };
            let effect_accounts = changed
                .iter()
                .map(|address| {
                    Ok(FinalChainConcreteAccountProjection {
                        address: *address,
                        raw_account_rlp: account_bytes(
                            &rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                                &view, *address,
                            ).with_context(|| format!("prepared transaction account period={} tx={} identity={:?} address={} fixture=periods[].transactions[].observer expected=projection", request.period.as_u64(), transaction.position.as_u32(), view.identity, hex::encode(address)))?,
                        ),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let effect_storage = raw_touched
                .into_iter()
                .map(|(address, key)| {
                    Ok(FinalChainConcreteStorageProjection {
                        contract: address,
                        key: key.0,
                        value: read_raw_projection(&view, address, key).with_context(|| format!("prepared transaction storage period={} tx={} identity={:?} address={} key={} fixture=periods[].transactions[].ordered_raw_writes expected=projection", request.period.as_u64(), transaction.position.as_u32(), view.identity, hex::encode(address), hex::encode(key.0)))?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let invocations = settled
                .native_invocations
                .iter()
                .map(project_invocation)
                .collect::<Result<Vec<_>>>()?;
            effects.push(FinalChainConcreteTransactionEffect {
                index: u64::from(transaction.position.as_u32()),
                transaction_rlp: encode_concrete_evm_transaction(transaction),
                execution_result_rlp: encode_concrete_execution_result(&report),
                intermediate_state: FinalChainConcreteState {
                    period: request.period.as_u64(),
                    root: identity.state_root,
                },
                accounts: effect_accounts,
                storage: effect_storage,
                invocations,
            });
            prepared = Some(next);
            results.push(report);
        }

        let prepared = match prepared {
            Some(prepared) => prepared,
            None => {
                let next = concrete.apply_observer_phase(ConcreteObserverPhaseDelta::default())?;
                identity = next.identity();
                next
            }
        };
        let projection = FinalChainConcreteStateProjection {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            prior_state: marker.prior_state,
            post_transaction_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            post_rewards_state: FinalChainConcreteState {
                period: request.period.as_u64(),
                root: identity.state_root,
            },
            transaction_effects: effects,
            accounts: Vec::new(),
            storage: Vec::new(),
            invocations: Vec::new(),
            rewards_input: Vec::new(),
            catalog_hash: concrete_storage_catalog_hash(&[]),
        };
        #[cfg(feature = "synthetic-fixtures")]
        if origin_scope
            && hex::encode(identity.state_root)
                == "5caad406f2d49cc61580c81a9cfaab801bb1147f654117af9a016b5a8e3f33bd"
        {
            use std::io::Write;
            if let Some(report) = origin_report {
                let mut output = std::io::stderr().lock();
                let mut written = true;
                let mut output_overflow = false;
                let mut bytes = 0_usize;
                for (index, event) in report.events.iter().enumerate() {
                    if !(event.contains("branch=origin_")
                        || event.starts_with("origin_settled ")
                        || event.starts_with("origin_delta ")
                        || event.starts_with("origin_transaction ")
                        || event.starts_with("origin_invocation ")
                        || event.starts_with("origin_mutation "))
                    {
                        continue;
                    }
                    let line = format!(
                        "ORIGIN_PARTIAL index={index} tx=5 sequence=NOT_BOUND fixture_body_sha256_external_pin=b4d3532791ffb4270bf253ae36c537b10b40227db2405c5f4ba854ba2409b9b9 {event}\n"
                    );
                    if line.len() > 65_000_usize.saturating_sub(bytes) {
                        output_overflow = true;
                        break;
                    }
                    bytes += line.len();
                    if output.write_all(line.as_bytes()).is_err() {
                        written = false;
                        break;
                    }
                }
                let _ = writeln!(
                    output,
                    "ORIGIN_PARTIAL_COMPLETE diagnostic_complete=false later_effects=NOT_RETAINED overflow={} output_overflow={output_overflow} writes_ok={written}",
                    report.overflow
                );
            }
        }
        let (period_raw_read_slots, period_raw_slots) = native_raw_observations(&native_contexts);
        let receipts = native.lifecycle_receipts(&request.transactions)?;
        let authority =
            rustaxa_consensus::native_projection_context::FinalChainLifecycleAuthority {
                request_id: request.request_id,
                state_api_epoch: request.state_api_epoch,
                projection_hash: [0; 32],
                receipts: receipts.clone(),
            };
        *self.native.borrow_mut() = Some(native);
        *self.native_context.borrow_mut() = Some(NativeContextParts {
            request_id: request.request_id,
            projection_hash: [0; 32],
            authority,
            delayed,
            lifecycle: receipts,
            invocations: native_contexts,
            rewards: FinalChainNativeRewardsContext::default(),
            raw_slots: period_raw_slots,
            raw_read_slots: period_raw_read_slots,
        });
        *self.staged.borrow_mut() = Some(Staged {
            prepared,
            system_accounts,
            projection,
            provenance: Vec::new(),
            catalog_rlp: encode_concrete_storage_catalog(raw_catalog.iter().copied()),
        });
        Ok(FinalChainEvmExecutionReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            status: FINAL_CHAIN_EVM_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            post_transaction_state_root: identity.state_root,
            cumulative_gas_used: cumulative.into(),
            results,
        })
    }
    fn distribute_final_chain_rewards(
        &self,
        _request: &FinalChainEvmRewardsRequest,
    ) -> Result<FinalChainEvmRewardsReport> {
        bail!("mixed fixture requires the opaque native rewards plan")
    }

    fn distribute_final_chain_rewards_with_native_plan(
        &self,
        request: &FinalChainEvmRewardsRequest,
        plan: &FinalChainPreparedExternalEvmRewardsStatsPlan,
    ) -> Result<FinalChainEvmRewardsReport> {
        self.state_api_epoch.validate(request.state_api_epoch)?;
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .context("fixture concrete handle unavailable")?;
        let mut staged = self.staged.borrow_mut();
        let staged = staged.as_mut().context("fixture execution is not staged")?;
        ensure!(
            staged.projection.post_transaction_state.root == request.post_transaction_state_root,
            "rewards prior root"
        );
        let prior_identity = staged.prepared.identity();
        let view = FixtureView {
            prior: FixturePrior::Prepared(concrete.prepared_view(&staged.prepared)?),
            identity: prior_identity,
        };
        let mut journal = ExecutionJournal::new(view);
        let mut native = self.native.borrow_mut();
        let native = native
            .as_mut()
            .context("bound native session unavailable")?;
        #[cfg(feature = "synthetic-fixtures")]
        let diagnostic = if native_integration::cleanup_requested() && request.period.as_u64() == 5
        {
            let guard = rustaxa_consensus::native_session::cleanup_diagnostic::start()
                .map_err(anyhow::Error::msg)?;
            let observer = rustaxa_evm::journal::cleanup_diagnostic_observer::install(
                rustaxa_consensus::native_session::cleanup_diagnostic::is_active,
                rustaxa_consensus::native_session::cleanup_diagnostic::event,
                rustaxa_consensus::native_session::cleanup_diagnostic::overflow,
                false,
            )
            .map_err(anyhow::Error::msg)?;
            rustaxa_consensus::native_session::cleanup_diagnostic::event_fmt(
                "binding",
                format_args!(
                    "request={:?} pending={:?} parent={:?} prepared_identity={:?} generation={} authority=UNKNOWN case=invalid_signature_a",
                    request.request_id,
                    request.period,
                    staged.projection.prior_state,
                    prior_identity,
                    staged.projection.generation
                ),
            );
            Some((guard, observer))
        } else {
            None
        };
        rustaxa_evm::journal::duration_journal_reward_boundary(true, request.period.as_u64());
        let native_result = native.finish(plan, &journal);
        #[cfg(feature = "synthetic-fixtures")]
        if let Some((guard, observer)) = diagnostic {
            use rustaxa_consensus::native_session::cleanup_diagnostic as diagnostic;
            use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
            use std::io::Write;
            if matches!(&native_result,
                Err(rustaxa_evm::contracts::NativePortError::Domain(message))
                    if message == "FinalChain native session: RawIntegrity(\"slashing jailed validators raw/domain facts disagree\")")
            {
                let address = [
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xee,
                ];
                let mut key = [0_u8; 32];
                key[31] = 2;
                let expected = [
                    213, 148, 88, 218, 153, 10, 143, 74, 58, 108, 167, 203, 99, 21, 214, 138, 20,
                    1, 5, 145, 115, 82,
                ];
                diagnostic::event_fmt(
                    "post_error_probe_start",
                    format_args!(
                        "label=prepared_account_and_raw phase=after_native_error request={:?} generation={} parent={:?} identity={:?} time={:?} authority=UNKNOWN",
                        request.request_id,
                        staged.projection.generation,
                        staged.projection.prior_state,
                        prior_identity,
                        std::time::SystemTime::now()
                    ),
                );
                let reader = journal.reader();
                match &reader.prior {
                    FixturePrior::Prepared(prepared)
                        if reader.identity == prior_identity
                            && prepared.identity() == prior_identity
                            && prior_identity.period.as_u64() == 5
                            && prior_identity.state_root
                                == [
                                    0x5c, 0xaa, 0xd4, 0x06, 0xf2, 0xd4, 0x9c, 0xc6, 0x15, 0x80,
                                    0xc8, 0x1a, 0x9c, 0xfa, 0xab, 0x80, 0x1b, 0xb1, 0x14, 0x7f,
                                    0x65, 0x41, 0x17, 0xaf, 0x9a, 0x01, 0x6b, 0x5a, 0x8e, 0x3f,
                                    0x33, 0xbd,
                                ] =>
                    {
                        let account = prepared.account(address);
                        match &account {
                            Ok(value) => diagnostic::event_fmt(
                                "post_error_account",
                                format_args!(
                                    "identity={:?} source=prepared variant={} time={:?}",
                                    prior_identity,
                                    match value {
                                        ConcreteRead::Present(_) => "Present",
                                        ConcreteRead::Absent => "Absent",
                                        ConcreteRead::Tombstone => "Tombstone",
                                    },
                                    std::time::SystemTime::now()
                                ),
                            ),
                            Err(error) => diagnostic::event_fmt(
                                "post_error_account",
                                format_args!(
                                    "identity={:?} source=prepared error={:?} time={:?}",
                                    prior_identity,
                                    error,
                                    std::time::SystemTime::now()
                                ),
                            ),
                        }
                        let storage = prepared.storage(address, ConcreteStorageKey(key));
                        if matches!(&storage, Ok(ConcreteRead::Present(bytes)) if bytes.len() > 128)
                        {
                            diagnostic::overflow();
                        } else {
                            let length = match &storage {
                                Ok(ConcreteRead::Present(bytes)) => Some(bytes.len()),
                                _ => None,
                            };
                            let equal = match &storage {
                                Ok(ConcreteRead::Present(bytes)) => {
                                    Some(bytes.as_slice() == expected)
                                }
                                _ => None,
                            };
                            diagnostic::event_fmt(
                                "post_error_storage",
                                format_args!(
                                    "identity={:?} source=prepared result={:?} length={:?} expected={:?} exact_match={:?} time={:?} authority=UNKNOWN",
                                    prior_identity,
                                    storage,
                                    length,
                                    expected,
                                    equal,
                                    std::time::SystemTime::now()
                                ),
                            );
                        }
                        diagnostic::event_fmt(
                            "post_error_probe_end",
                            format_args!(
                                "label=prepared_account_and_raw status=complete time={:?}",
                                std::time::SystemTime::now()
                            ),
                        );
                    }
                    _ => diagnostic::event_fmt(
                        "post_error_probe_end",
                        format_args!(
                            "label=prepared_account_and_raw status=unavailable_identity_or_reader time={:?}",
                            std::time::SystemTime::now()
                        ),
                    ),
                }
            }
            drop(observer);
            let report = guard.finish();
            let required = [
                "binding ",
                "decision ",
                "scheduler ",
                "plan ",
                "state_counts role=snapshot ",
                "state_counts role=visible ",
                "state_counts role=eligibility ",
                "state_counts role=session ",
                "compare ",
                "validation ",
                "finish_after ",
            ];
            let trace_reader = report
                .events
                .iter()
                .any(|row| row.starts_with("trace_current ") && row.contains("branch=reader"));
            let journal = report
                .events
                .iter()
                .find(|row| row.starts_with("evm_raw_storage "));
            let journal_bound = journal.is_some_and(|row| {
                !row.contains("branch=backing")
                    || (report
                        .events
                        .iter()
                        .any(|item| item.starts_with("fixture_original "))
                        && report
                            .events
                            .iter()
                            .any(|item| item.starts_with("fixture_mapped ")))
            });
            let complete = !report.overflow
                && required
                    .iter()
                    .all(|prefix| report.events.iter().any(|row| row.starts_with(prefix)))
                && trace_reader
                && journal_bound;
            let mut output = std::io::stderr().lock();
            let mut written = true;
            for row in &report.events {
                written &= writeln!(output, "CLEANUP_DIAGNOSTIC {row}").is_ok();
            }
            let _ = writeln!(
                output,
                "CLEANUP_DIAGNOSTIC_COMPLETE complete={} overflow={} events={} writes_ok={} authority=UNKNOWN",
                complete && written,
                report.overflow,
                report.events.len(),
                written
            );
        }
        let (native_rewards, projected, reward_context) = native_result?;
        ensure!(projected.status == rustaxa_evm::contracts::NativeStatus::Success);
        rustaxa_evm::journal::duration_native_outcome_boundary(true);
        for mutation in &projected.account_mutations {
            rustaxa_evm::journal::duration_native_account_event("outcome_account", mutation);
        }
        for mutation in &projected.raw_mutations {
            rustaxa_evm::journal::duration_native_raw_event("outcome_raw", mutation);
        }
        rustaxa_evm::journal::duration_native_outcome_boundary(false);
        apply_native_outcome(&mut journal, &projected)?;
        rustaxa_evm::journal::duration_journal_reward_boundary(false, request.period.as_u64());
        let settled = journal.settle_transaction()?;
        drop(journal);
        ensure!(settled.logs.is_empty() && settled.native_invocations.is_empty());
        ensure!(
            native_rewards.total_reward.as_u256()
                == decimal_u256(&self.fixture["reward_output"]["minted_reward"]),
            "Go minted reward"
        );
        if self.fixture.get("distributions").is_some() {
            mainnet_era::compare_distribution_writes(&reward_context, self.fixture)?;
        } else if self.fixture["reward_input"]["validators"]
            .as_array()
            .context("reward validators")?
            .len()
            > 1
        {
            if number(&self.fixture["number"]) == 5 {
                let cell = |value: &ConcreteRead<Vec<u8>>| match value {
                    ConcreteRead::Present(bytes) => {
                        serde_json::json!({"state":"present","value":hex::encode(bytes)})
                    }
                    ConcreteRead::Absent => serde_json::json!({"state":"absent"}),
                    ConcreteRead::Tombstone => serde_json::json!({"state":"tombstone"}),
                };
                let mutations = reward_context.raw_mutations.iter().enumerate().map(|(index, row)| {
                    let operation = match &row.operation {
                        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Put(value) => serde_json::json!({"type":"put","value":hex::encode(value.as_bytes())}),
                        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Delete => serde_json::json!({"type":"delete"}),
                    };
                    serde_json::json!({"index":index,"address":hex::encode(row.address),"key":hex::encode(row.key.0),"expected":cell(&row.expected),"operation":operation})
                }).collect::<Vec<_>>();
                let reads = reward_context.raw_reads.iter().enumerate().map(|(index, row)| {
                    serde_json::json!({"index":index,"address":hex::encode(row.address),"key":hex::encode(row.key.0),"value":cell(&row.value)})
                }).collect::<Vec<_>>();
                eprintln!(
                    "RECOVERY_REWARD_DIAGNOSTIC {}",
                    serde_json::json!({
                        "pending_period": number(&self.fixture["number"]),
                        "parent_period": staged.projection.prior_state.period,
                        "parent_root": hex::encode(staged.projection.prior_state.root),
                        "post_transaction_period": prior_identity.period.as_u64(),
                        "post_transaction_root": hex::encode(prior_identity.state_root),
                        "expected_final_root": self.fixture["final"]["root"],
                        "raw_mutations": mutations, "raw_reads": reads,
                        "expected_rewards": self.fixture["ordered_raw_writes"]["rewards"],
                        "expected_end_block": self.fixture["ordered_raw_writes"]["end_block"],
                        "reward_input": self.fixture["reward_input"],
                        "actual_minted_reward": native_rewards.total_reward.as_u256().to_string(),
                        "expected_reward_output": self.fixture["reward_output"],
                        "timer": "UNKNOWN", "native_transaction_context": "NOT_RETAINED",
                        "transaction_results": "NOT_RETAINED", "published_receipts": "NOT_OBSERVED",
                        "published_period_root": "NOT_OBSERVED",
                    })
                );
            }
            native_integration::compare_reward_map(
                &reward_context,
                &self.fixture["ordered_raw_writes"]["rewards"],
                &self.fixture["ordered_raw_writes"]["end_block"],
                &self.fixture["reward_input"],
            )?;
        } else {
            compare_reward_raw_trace(
                &reward_context,
                &self.fixture["ordered_raw_writes"]["rewards"],
                &self.fixture["ordered_raw_writes"]["end_block"],
            )?;
        }

        let mut changed = BTreeSet::new();
        let mut accounts = Vec::new();
        let mut storage = Vec::new();
        let mut code = Vec::new();
        for write in settled.writes.accounts {
            changed.insert(write.address);
            accounts.push(match write.operation {
                JournalAccountOperation::Upsert {
                    nonce,
                    balance,
                    code_hash,
                    code_size,
                } => ConcreteAccountMutation::Upsert {
                    address: write.address,
                    record: record(nonce, balance, code_hash, code_size),
                },
                JournalAccountOperation::Delete => ConcreteAccountMutation::Delete {
                    address: write.address,
                },
            });
        }
        for write in settled.writes.ordinary_storage {
            changed.insert(write.address);
            if let Some(observer) = &self.ordinary_observer {
                observer.borrow_mut().record_settled(ConcreteStorageSlot {
                    address: write.address,
                    key: write.key.0,
                })?;
            }
            storage.push(ConcreteStorageMutation {
                address: write.address,
                key: write.key,
                value: (write.value != BigUint::default()).then(|| write.value.to_bytes_be()),
            });
        }
        for write in settled.writes.raw_storage {
            changed.insert(write.address);
            storage.push(ConcreteStorageMutation {
                address: write.address,
                key: write.key,
                value: raw_operation_value(&write.operation),
            });
        }
        for write in settled.writes.code {
            code.push(ConcreteCodeInsertion {
                code_hash: write.code_hash,
                code: write.code,
            });
        }
        let delta = ConcreteObserverPhaseDelta {
            accounts,
            storage,
            code,
        };
        duration_capture_plan(&delta, prior_identity.state_root, 2, 0);
        rustaxa_evm::journal::duration_journal_prepared_stage(true, prior_identity.state_root);
        let final_output = concrete.apply_observer_phase(delta.clone())?;
        duration_capture_prepared(concrete, &final_output, &delta)?;
        ensure!(
            final_output.identity().state_root == fixed(&self.fixture["final"]["root"]),
            "Go post-rewards root"
        );
        ensure!(
            final_output
                .changed_accounts()
                .iter()
                .map(|entry| entry.address)
                .collect::<BTreeSet<_>>()
                == changed,
            "reward changed-account set"
        );

        let mut account_identities = staged
            .projection
            .transaction_effects
            .iter()
            .flat_map(|effect| effect.accounts.iter().map(|entry| entry.address))
            .collect::<BTreeSet<_>>();
        account_identities.extend(changed);
        account_identities.extend(staged.system_accounts.iter().copied());
        // The actual local Go RecordConcreteRewardsProjection hook records
        // DPoS even for a non-distribution period with an unchanged root. Its
        // touched-account catalog is distinct from changed account rows.
        account_identities.insert([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xfe,
        ]);
        let expected_accounts =
            catalog_account_identities(&self.fixture["period_catalog_identities"]["accounts"])?;
        ensure!(
            account_identities == expected_accounts,
            "Go period account catalog period={}: actual={account_identities:?} expected={expected_accounts:?}",
            request.period.as_u64(),
        );

        let mut catalog = decode_concrete_storage_catalog(&staged.catalog_rlp)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut period_raw_slots = self
            .native_context
            .borrow()
            .as_ref()
            .context("native context accumulator unavailable")?
            .raw_slots
            .clone();
        let mut period_raw_read_slots = self
            .native_context
            .borrow()
            .as_ref()
            .context("native context accumulator unavailable")?
            .raw_read_slots
            .clone();
        period_raw_read_slots.extend(raw_read_slots(reward_context.raw_reads.iter()));
        for mutation in &native_rewards.raw_mutations {
            let slot = ConcreteStorageSlot {
                address: mutation.address,
                key: mutation.key.0,
            };
            catalog.insert(slot);
            period_raw_slots.insert(slot);
        }
        let expected_slots =
            catalog_storage_identities(&self.fixture["period_catalog_identities"]["slots"])?;
        let declared_absence_probes = catalog_storage_identities(
            &self.fixture["final"]["native_catalog"]["coverage"]["absent_slot_identities"],
        )?;
        let mut ordinary_slots = BTreeSet::new();
        if let Some(observer) = &self.ordinary_observer {
            let view = concrete.prepared_view(&final_output)?;
            observer
                .borrow_mut()
                .finish_prepared(final_output.identity(), |slot| {
                    Ok(
                        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                            &view,
                            slot.address,
                            ConcreteStorageKey(slot.key),
                        )?,
                    )
                })?;
            let borrowed = observer.borrow();
            ordinary_slots = borrowed.staged_slots()?;
            for (slot, actual) in borrowed.staged_values()? {
                let rows = self.fixture["final"]["native_catalog"]["slots"]
                    .as_array()
                    .context("full observed slot rows")?;
                let row = rows
                    .iter()
                    .find(|row| {
                        fixed::<20>(&row["address"]) == slot.address
                            && fixed::<32>(&row["key"]) == slot.key
                    })
                    .context("ordinary observed Go slot missing")?;
                if row["present"] == true {
                    ensure!(
                        *actual == ConcreteRead::Present(bytes(&row["value"])),
                        "ordinary prepared value differs"
                    );
                } else {
                    ensure!(
                        matches!(actual, ConcreteRead::Absent | ConcreteRead::Tombstone)
                            || *actual == ConcreteRead::Present(Vec::new()),
                        "ordinary prepared absence differs"
                    );
                }
            }
        }
        let native_absence_probes = if self.ordinary_observer.is_some() {
            let mut native = BTreeSet::new();
            let view = concrete.prepared_view(&final_output)?;
            for slot in &declared_absence_probes {
                if ordinary_observer::is_native_storage_domain(slot.address) {
                    native.insert(*slot);
                } else {
                    ensure!(
                        ordinary_slots.contains(slot),
                        "ordinary absence probe has no observed provenance"
                    );
                    ensure!(
                        matches!(
                        rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                            &view, slot.address, ConcreteStorageKey(slot.key))?,
                        ConcreteRead::Absent | ConcreteRead::Tombstone
                    ),
                        "ordinary absence probe is present or unavailable"
                    );
                }
            }
            native
        } else {
            declared_absence_probes.clone()
        };
        // Native declared probes retain their established coverage treatment.
        // Ordinary identities come only from actual prior or settled writes.
        let mut explained_period_slots = catalog.clone();
        explained_period_slots.extend(period_raw_slots.iter().copied());
        explained_period_slots.extend(period_raw_read_slots.iter().copied());
        explained_period_slots.extend(native_absence_probes.iter().copied());
        explained_period_slots.extend(ordinary_slots.iter().copied());
        ensure!(
            expected_slots.is_subset(&explained_period_slots),
            "Go period catalog identity is neither prior, consumed, mutated nor a declared absence probe; period={} unexplained={:?}",
            request.period.as_u64(),
            expected_slots
                .difference(&explained_period_slots)
                .collect::<Vec<_>>()
        );
        catalog.extend(native_absence_probes.iter().copied());
        let expected_catalog =
            catalog_storage_identities(&self.fixture["final"]["native_catalog"]["slots"])?;
        let mut full_catalog = catalog.clone();
        full_catalog.extend(ordinary_slots.iter().copied());
        ensure!(
            full_catalog == expected_catalog,
            "Go full raw catalog period={} missing={:?} extra={:?}",
            request.period.as_u64(),
            expected_catalog
                .difference(&full_catalog)
                .collect::<Vec<_>>(),
            full_catalog
                .difference(&expected_catalog)
                .collect::<Vec<_>>()
        );
        let catalog_rlp = encode_concrete_storage_catalog(catalog.iter().copied());
        // Replace the temporary pre-rewards catalog hash only after every raw
        // key is known and every final value can be read from one prepared view.
        let view = FixtureView {
            prior: FixturePrior::Prepared(concrete.prepared_view(&final_output)?),
            identity: final_output.identity(),
        };
        for slot in &declared_absence_probes {
            ensure!(
                matches!(
                    rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
                        &view,
                        slot.address,
                        ConcreteStorageKey(slot.key),
                    ).with_context(|| format!("prepared reward absence period={} identity={:?} address={} key={} fixture=periods[].final.native_catalog.slots expected=absent", request.period.as_u64(), view.identity, hex::encode(slot.address), hex::encode(slot.key)))?,
                    ConcreteRead::Absent | ConcreteRead::Tombstone
                ),
                "fixture-declared raw absence probe is present"
            );
        }
        let projection_accounts = account_identities
            .iter()
            .map(|address| {
                Ok(FinalChainConcreteAccountProjection {
                    address: *address,
                    raw_account_rlp: account_bytes(
                        &rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
                            &view, *address,
                        ).with_context(|| format!("prepared reward account period={} identity={:?} address={} fixture=periods[].period_catalog_identities.accounts expected=projection", request.period.as_u64(), view.identity, hex::encode(address)))?,
                    ),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let projection_storage = catalog
            .iter()
            .map(|slot| {
                let key = ConcreteStorageKey(slot.key);
                Ok(FinalChainConcreteStorageProjection {
                    contract: slot.address,
                    key: slot.key,
                    value: read_raw_projection(&view, slot.address, key).with_context(|| format!("prepared reward storage period={} identity={:?} address={} key={} fixture=periods[].final.native_catalog.slots expected=projection", request.period.as_u64(), view.identity, hex::encode(slot.address), hex::encode(key.0)))?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let catalog_hash = concrete_storage_catalog_hash(&projection_storage);
        let projection_invocations = staged
            .projection
            .transaction_effects
            .iter()
            .flat_map(|effect| effect.invocations.iter().cloned())
            .collect();
        staged.projection.post_rewards_state = FinalChainConcreteState {
            period: request.period.as_u64(),
            root: final_output.identity().state_root,
        };
        staged.projection.accounts = projection_accounts;
        staged.projection.storage = projection_storage;
        staged.projection.invocations = projection_invocations;
        staged.projection.rewards_input =
            encode_concrete_rewards_input(&request.distribution_stats);
        staged.projection.catalog_hash = catalog_hash;
        staged.prepared = final_output;
        staged.catalog_rlp = catalog_rlp;

        let projection = encode_concrete_state_projection(&staged.projection);
        let projection_hash = concrete_state_bytes_digest(&projection);
        let marker = decode_concrete_execution_marker(&request.concrete_marker_rlp)?;
        staged.provenance = encode_concrete_state_provenance(&FinalChainConcreteStateProvenance {
            identity: marker.identity,
            generation: marker.generation,
            plan_hash: marker.plan_hash,
            committed_state: staged.projection.post_rewards_state,
            transactions_hash: marker.transactions_hash,
            rewards_hash: marker.rewards_hash,
            projection_hash,
            catalog_hash,
        });
        {
            let mut stored = self.native_context.borrow_mut();
            let stored = stored
                .as_mut()
                .context("native context accumulator unavailable")?;
            stored.rewards = reward_context;
            stored.projection_hash = projection_hash;
            stored.authority.projection_hash = projection_hash;
        }
        let mut wrong_request = request.request_id;
        wrong_request[0] ^= 1;
        ensure!(
            ConsensusExecutionPort::final_chain_native_projection_context(
                self,
                wrong_request,
                projection_hash,
            )
            .is_err(),
            "wrong native context request identity was accepted"
        );
        let mut wrong_projection = projection_hash;
        wrong_projection[0] ^= 1;
        ensure!(
            ConsensusExecutionPort::final_chain_native_projection_context(
                self,
                request.request_id,
                wrong_projection,
            )
            .is_err(),
            "wrong native context projection hash was accepted"
        );
        Ok(FinalChainEvmRewardsReport {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            period: request.period,
            status: FINAL_CHAIN_EVM_REWARDS_REPORT_STATUS_SUCCESS,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: staged.projection.post_rewards_state.root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_plan_hash: request.concrete_plan_hash,
            transactions_hash: request.transactions_hash,
            rewards_hash: request.rewards_hash,
            concrete_projection_rlp: projection,
            concrete_projection_hash: projection_hash,
            concrete_provenance_rlp: staged.provenance.clone(),
            total_reward: minimal_u256(native_rewards.total_reward.as_u256()),
        })
    }

    fn final_chain_native_lifecycle_authority(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<rustaxa_consensus::native_projection_context::FinalChainLifecycleAuthority>>
    {
        let stored = self.native_context.borrow();
        let parts = stored.as_ref().context("native authority unavailable")?;
        ensure!(
            parts.authority.request_id == request_id
                && parts.authority.projection_hash == projection_hash,
            "native authority binding mismatch"
        );
        Ok(Some(parts.authority.clone()))
    }

    fn final_chain_native_delayed_view(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<
        Option<
            std::rc::Rc<rustaxa_consensus::native_session::frozen_eligibility::FrozenEligibility>,
        >,
    > {
        let stored = self.native_context.borrow();
        let parts = stored
            .as_ref()
            .context("native delayed authority unavailable")?;
        ensure!(
            parts.authority.request_id == request_id
                && parts.authority.projection_hash == projection_hash,
            "native delayed authority binding mismatch"
        );
        Ok(Some(std::rc::Rc::clone(&parts.delayed)))
    }

    fn final_chain_native_projection_context(
        &self,
        request_id: [u8; 32],
        projection_hash: [u8; 32],
    ) -> Result<Option<FinalChainNativeProjectionContext>> {
        let mut stored = self.native_context.borrow_mut();
        let parts = stored
            .as_ref()
            .context("native context requested before rewards")?;
        ensure!(
            parts.request_id == request_id,
            "native context request identity mismatch"
        );
        ensure!(
            parts.projection_hash == projection_hash,
            "native context projection hash mismatch"
        );
        let parts = stored.take().expect("validated stored native context");
        Ok(Some(FinalChainNativeProjectionContext {
            request_id: parts.request_id,
            projection_hash: parts.projection_hash,
            invocations: parts.invocations,
            lifecycle: parts.lifecycle,
            rewards: parts.rewards,
        }))
    }
    fn commit_final_chain_state(
        &self,
        request: &FinalChainExternalEvmStateCommitIntent,
    ) -> Result<FinalChainExternalEvmStateCommitResult> {
        self.state_api_epoch.validate(request.state_api_epoch)?;
        ensure!(
            self.chain.last_block_number_typed()? == request.prior_state.period,
            "application published before concrete commit"
        );
        ensure!(
            self.application
                .final_chain()
                .external_evm_pending_publication_raw()?
                .is_some(),
            "application intent must be durable first"
        );
        self.chain.validate_pending_external_evm_commit(request)?;
        let mut missing_epoch = request.clone();
        missing_epoch.state_api_epoch = 0;
        ensure!(
            self.chain
                .validate_pending_external_evm_commit(&missing_epoch)
                .is_err(),
            "zero-epoch intent accepted"
        );
        let mut foreign = request.clone();
        foreign.concrete_projection_hash[0] ^= 1;
        ensure!(
            self.chain
                .validate_pending_external_evm_commit(&foreign)
                .is_err(),
            "foreign intent accepted"
        );
        let staged = self
            .staged
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("fixture execution is not staged"))?;
        ensure!(
            encode_concrete_state_projection(&staged.projection) == request.concrete_projection_rlp
                && staged.provenance == request.concrete_provenance_rlp,
            "accepted projection/provenance differs"
        );
        let concrete = self
            .concrete
            .borrow_mut()
            .take()
            .ok_or_else(|| anyhow::anyhow!("fixture concrete handle unavailable"))?;
        if let Some(observer) = &self.ordinary_observer {
            let prior = concrete.observation()?;
            observer.borrow().validate_pending_commit(
                prior.identity,
                prior.committed,
                prior.generation,
                &request.concrete_marker_rlp,
                staged.prepared.identity(),
            )?;
        }
        let approval = ConcreteCommitApproval {
            marker_rlp: request.concrete_marker_rlp.clone(),
            provenance_rlp: request.concrete_provenance_rlp.clone(),
            catalog_rlp: staged.catalog_rlp,
            projection_hash: request.concrete_projection_hash,
            catalog_hash: staged.projection.catalog_hash,
        };
        let observed = if let Some(capture) = &self.physical_receipt {
            ensure!(capture.borrow().is_none(), "unconsumed physical receipt");
            let (observation, receipt) = match self.account_node_publication {
                AccountNodePublication::FinalPeriod { limits } => concrete
                    .commit_observer_final_trie_nodes_with_receipt(
                        staged.prepared,
                        approval,
                        limits,
                    )?,
                AccountNodePublication::ObserverHistory => {
                    concrete.commit_observer_approved_with_receipt(staged.prepared, approval)?
                }
            };
            *capture.borrow_mut() = Some(receipt);
            observation
        } else {
            match self.account_node_publication {
                AccountNodePublication::FinalPeriod { limits } => {
                    concrete.commit_observer_final_trie_nodes(staged.prepared, approval, limits)?
                }
                AccountNodePublication::ObserverHistory => {
                    concrete.commit_observer_approved(staged.prepared, approval)?
                }
            }
        };
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.provenance_rlp == request.concrete_provenance_rlp,
            "concrete commit observation"
        );
        if let Some(observer) = &self.ordinary_observer {
            observer.borrow_mut().concrete_commit(
                observed.identity,
                &request.concrete_marker_rlp,
                observed.committed,
                observed.generation,
            )?;
        }
        Ok(FinalChainExternalEvmStateCommitResult {
            request_id: request.request_id,
            state_api_epoch: request.state_api_epoch,
            plan_id: request.plan_id,
            period: request.period,
            publication_block_hash: request.publication_block_hash,
            prior_state: request.prior_state,
            post_transaction_state_root: request.post_transaction_state_root,
            post_rewards_state_root: request.post_rewards_state_root,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            concrete_projection_rlp: request.concrete_projection_rlp.clone(),
            concrete_projection_hash: request.concrete_projection_hash,
            concrete_provenance_rlp: request.concrete_provenance_rlp.clone(),
            committed_state: Some(FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            }),
            status: FINAL_CHAIN_EVM_LIFECYCLE_STATUS_COMMITTED,
            error_code: String::new(),
        })
    }
    fn discard_final_chain_state(
        &self,
        request: &FinalChainExternalEvmDiscardRequest,
    ) -> Result<FinalChainExternalEvmDiscardReport> {
        self.state_api_epoch
            .validate(request.expected_state_api_epoch)?;
        self.staged.borrow_mut().take();
        let mut concrete = self.concrete.borrow_mut();
        let concrete = concrete
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("fixture ambiguous consumed commit"))?;
        ensure!(
            concrete_state_bytes_digest(&request.concrete_marker_rlp) == request.marker_hash,
            "fixture discard marker hash"
        );
        concrete.discard_execution(&request.concrete_marker_rlp)?;
        let observed = concrete.observation()?;
        ensure!(
            observed.pending_marker_rlp.is_empty()
                && observed.committed.period == request.prior_state.period
                && observed.committed.state_root == request.prior_state.state_root,
            "fixture discard reopened prior descriptor"
        );
        if let Some(observer) = &self.ordinary_observer {
            observer.borrow_mut().discard_from_owner(
                observed.identity,
                observed.committed,
                observed.generation,
            )?;
        }
        let (previous_state_api_epoch, state_api_epoch) = self
            .state_api_epoch
            .replace_after_discard(request.expected_state_api_epoch)?;
        Ok(FinalChainExternalEvmDiscardReport {
            request_id: request.request_id,
            previous_state_api_epoch,
            state_api_epoch,
            period: request.period,
            concrete_marker_rlp: request.concrete_marker_rlp.clone(),
            marker_hash: request.marker_hash,
            prior_state: request.prior_state,
            committed_state: FinalChainExternalEvmCommittedStateDescriptor {
                period: observed.committed.period,
                state_root: observed.committed.state_root,
            },
            succeeded: true,
            error_code: String::new(),
        })
    }
    fn load_pillar_anchor_state(
        &self,
        _: &PillarAnchorStateRequest,
    ) -> Result<PillarAnchorStateReport> {
        bail!("fixture has no pillar operation")
    }
}

fn project_log(log: &rustaxa_evm::contracts::ExecutionLog) -> FinalChainEvmLog {
    FinalChainEvmLog {
        address: log.address,
        topics: log
            .topics
            .iter()
            .map(|topic| FinalChainEvmLogTopic { topic: *topic })
            .collect(),
        data: log.data.clone(),
    }
}

fn project_invocation(
    observation: &ConsensusNativeObservation,
) -> Result<FinalChainConcreteInvocation> {
    let error = code_error(&observation.status);
    let call_type = match observation.invocation.kind {
        NativeCallKind::Call => 0,
        NativeCallKind::CallCode => 1,
        NativeCallKind::DelegateCall => 2,
        NativeCallKind::StaticCall => 3,
    };
    let disposition = match observation.disposition {
        ConsensusNativeDisposition::Normal => FINAL_CHAIN_CONCRETE_INVOCATION_NORMAL,
        ConsensusNativeDisposition::OwnFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_OWN_FRAME_REVERTED
        }
        ConsensusNativeDisposition::OuterFrameReverted => {
            FINAL_CHAIN_CONCRETE_INVOCATION_PARENT_FRAME_REVERTED
        }
    };
    Ok(FinalChainConcreteInvocation {
        transaction_index: u64::from(observation.invocation.id.transaction.as_u32()),
        sequence: observation.invocation.id.sequence,
        depth: observation.invocation.depth,
        call_type,
        caller: observation.invocation.caller,
        contract: observation.invocation.contract,
        value: observation.invocation.value.value().to_bytes_be(),
        input: observation.invocation.input.clone(),
        output: observation.output.clone(),
        supplied_gas: observation.invocation.supplied_gas.as_u64(),
        required_gas: observation.required_gas.as_u64(),
        gas_used: observation.gas_used.as_u64(),
        error,
        logs: observation.logs.iter().map(project_log).collect(),
        disposition,
    })
}

fn code_error(status: &CodeExecutionStatus) -> String {
    match status {
        CodeExecutionStatus::Success => String::new(),
        CodeExecutionStatus::Failure(CodeExecutionError::Revert) => "execution reverted".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::OutOfGas) => "out of gas".into(),
        CodeExecutionStatus::Failure(CodeExecutionError::InvalidOpcode(opcode)) => {
            format!("invalid opcode 0x{opcode:02x}")
        }
        CodeExecutionStatus::Failure(CodeExecutionError::Native(error)) => error.error.clone(),
        CodeExecutionStatus::Failure(other) => format!("{other:?}"),
    }
}

fn raw_operation_value(operation: &NativeRawOperation) -> Option<Vec<u8>> {
    match operation {
        NativeRawOperation::Put(value) => Some(value.as_bytes().to_vec()),
        NativeRawOperation::Delete => None,
    }
}

fn apply_native_outcome<R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead>(
    journal: &mut ExecutionJournal<R>,
    outcome: &NativeOutcome,
) -> Result<()> {
    for mutation in &outcome.account_mutations {
        rustaxa_evm::journal::duration_native_account_event("reward_apply_account_begin", mutation);
        journal.apply_native_account_mutations(std::slice::from_ref(mutation))?;
        rustaxa_evm::journal::duration_native_account_event("reward_apply_account_end", mutation);
    }
    for mutation in &outcome.raw_mutations {
        rustaxa_evm::journal::duration_native_raw_event("reward_apply_raw_begin", mutation);
        ensure!(
            journal.raw_storage(mutation.address, mutation.key)? == mutation.expected,
            "reward raw mutation expected classification"
        );
        journal.set_raw_storage(mutation.address, mutation.key, mutation.operation.clone())?;
        rustaxa_evm::journal::duration_native_raw_event("applied_raw", mutation);
        rustaxa_evm::journal::duration_native_raw_event("reward_apply_raw_end", mutation);
    }
    Ok(())
}

fn account_bytes(read: &ConcreteRead<ConcreteAccountRecord>) -> Vec<u8> {
    match read {
        ConcreteRead::Present(record) => record.physical_rlp.clone(),
        ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
    }
}

fn read_raw_projection<R: rustaxa_types::concrete_state::execution::ConcreteExecutionRead>(
    view: &R,
    address: [u8; 20],
    key: ConcreteStorageKey,
) -> Result<Vec<u8>> {
    Ok(
        match rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            view, address, key,
        )? {
            ConcreteRead::Present(value) => value,
            ConcreteRead::Absent | ConcreteRead::Tombstone => Vec::new(),
        },
    )
}

fn compare_raw_trace(
    contexts: &[FinalChainNativeInvocationContext],
    expected: &Value,
) -> Result<()> {
    let actual = contexts
        .iter()
        .flat_map(|context| context.raw_mutations.iter());
    compare_raw_mutations(actual, expected)
}

fn compare_reward_raw_trace(
    context: &FinalChainNativeRewardsContext,
    rewards: &Value,
    end_block: &Value,
) -> Result<()> {
    let expected = rewards
        .as_array()
        .context("reward raw trace")?
        .iter()
        .chain(end_block.as_array().context("end-block raw trace")?);
    let actual = context.raw_mutations.iter();
    ensure!(
        actual.len() == expected.clone().count(),
        "reward raw trace count"
    );
    for (actual, expected) in actual.zip(expected) {
        compare_raw_mutation(actual, expected)?;
    }
    Ok(())
}

fn compare_raw_mutations<'a>(
    actual: impl Iterator<Item = &'a rustaxa_consensus::native_session::FinalChainNativeRawMutation>,
    expected: &Value,
) -> Result<()> {
    let expected = expected.as_array().context("transaction raw trace")?;
    let actual = actual.collect::<Vec<_>>();
    ensure!(
        actual.len() == expected.len(),
        "transaction raw trace count"
    );
    for (index, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        compare_raw_mutation(actual, expected).with_context(|| {
            format!(
                "raw occurrence {index}: actual key {}, expected key {}",
                hex::encode(actual.key.0),
                string(&expected["key"])
            )
        })?;
    }
    Ok(())
}

fn compare_raw_mutation(
    actual: &rustaxa_consensus::native_session::FinalChainNativeRawMutation,
    expected: &Value,
) -> Result<()> {
    ensure!(
        actual.address == fixed(&expected["address"]),
        "raw trace address"
    );
    ensure!(actual.key.0 == fixed(&expected["key"]), "raw trace key");
    let value = bytes(&expected["value"]);
    match &actual.operation {
        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Put(actual) => {
            ensure!(
                actual.as_bytes() == value,
                "raw trace put bytes: key={} actual={} expected={}",
                string(&expected["key"]),
                hex::encode(actual.as_bytes()),
                hex::encode(&value)
            )
        }
        rustaxa_consensus::native_session::FinalChainNativeRawOperation::Delete => {
            ensure!(value.is_empty(), "raw trace delete bytes")
        }
    }
    Ok(())
}

fn catalog_account_identities(value: &Value) -> Result<BTreeSet<[u8; 20]>> {
    value
        .as_array()
        .context("account catalog")?
        .iter()
        .map(|value| fixed_prefixed(value.as_str().context("account identity")?))
        .collect()
}

fn catalog_storage_identities(value: &Value) -> Result<BTreeSet<ConcreteStorageSlot>> {
    value
        .as_array()
        .context("storage catalog")?
        .iter()
        .map(|entry| {
            Ok(ConcreteStorageSlot {
                address: fixed_prefixed(string(&entry["address"]))?,
                key: fixed_prefixed(string(&entry["key"]))?,
            })
        })
        .collect()
}

fn fixed_prefixed<const N: usize>(value: &str) -> Result<[u8; N]> {
    let bytes = hex::decode(value.strip_prefix("0x").unwrap_or(value))?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| anyhow::anyhow!("expected {N} bytes, got {}", bytes.len()))
}

fn decimal_u256(value: &Value) -> U256 {
    U256::from_dec_str(string(value)).expect("fixture uint256")
}

fn minimal_u256(value: U256) -> Vec<u8> {
    value
        .to_big_endian()
        .into_iter()
        .skip_while(|byte| *byte == 0)
        .collect()
}

fn fixed_or_zero<const N: usize>(value: &Value) -> [u8; N] {
    let value = string(value);
    if value.is_empty() || value.chars().all(|character| character == '0') {
        [0; N]
    } else {
        fixed_prefixed(value).expect("fixture fixed-width hex")
    }
}

fn pbft(period: u64) -> Vec<u8> {
    pbft_with_key(period, &[9; 32])
}

fn pbft_with_key(period: u64, private_key: &[u8; 32]) -> Vec<u8> {
    fn fields(out: &mut RlpStream, period: u64) {
        for value in 10..14 {
            out.append(&H256::from_low_u64_be(value));
        }
        out.append(&period).append(&(1234 + period));
        out.begin_list(0);
    }
    let mut unsigned = RlpStream::new_list(7);
    fields(&mut unsigned, period);
    let key = SigningKey::from_slice(private_key).expect("fixture signing key");
    let (signature, recovery) = key
        .sign_prehash_recoverable(&keccak256(unsigned.out()).0)
        .expect("fixture PBFT signature");
    let mut signature = signature.to_bytes().to_vec();
    signature.push(recovery.to_byte());
    let mut signed = RlpStream::new_list(8);
    fields(&mut signed, period);
    signed.append(&signature);
    signed.out().to_vec()
}

fn period_data(request: &FinalChainExecutionRequest) -> Vec<u8> {
    let mut stream = RlpStream::new_list(4);
    stream.append_raw(&request.pbft_block_rlp, 1);
    stream.begin_list(0);
    stream.begin_list(0);
    stream.begin_list(request.transactions.len());
    for transaction in &request.transactions {
        stream.append_raw(&transaction.rlp, 1);
    }
    stream.out().to_vec()
}

fn request(root: &Value, period: &Value) -> Result<FinalChainExecutionRequest> {
    let transactions = period["transactions"]
        .as_array()
        .context("period transactions")?
        .iter()
        .map(|transaction| {
            let raw = bytes(&transaction["signed_rlp"]);
            let decoded = LegacyTransactionEnvelope::decode(&raw)?;
            ensure!(decoded.signature_valid, "fixture signature");
            Ok(FinalizationTransaction {
                hash: decoded.hash.0,
                sender: decoded.sender.context("signed sender")?.0,
                receiver: decoded.receiver.map(|address| address.0),
                nonce: FinalChainNonce::from_bytes(
                    &decoded
                        .nonce
                        .to_big_endian()
                        .into_iter()
                        .skip_while(|byte| *byte == 0)
                        .collect::<Vec<_>>(),
                )?,
                value: decoded.value.into(),
                gas_price: decoded.gas_price.into(),
                gas_limit: decoded.gas.into(),
                data: decoded.data,
                rlp: raw,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let planner = &period["planner_facts"];
    let dag = planner["dag_blocks"]
        .as_array()
        .context("planner DAG blocks")?
        .iter()
        .map(|block| {
            Ok(FinalizationDagBlock {
                author: fixed(&block["author"]),
                difficulty: number(&block["difficulty"])
                    .try_into()
                    .context("DAG difficulty fits u16")?,
                transaction_hashes: block["transaction_hashes"]
                    .as_array()
                    .context("DAG transaction hashes")?
                    .iter()
                    .map(fixed)
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        dag.iter()
            .flat_map(|block| block.transaction_hashes.iter())
            .copied()
            .collect::<Vec<_>>()
            == transactions
                .iter()
                .map(|transaction| transaction.hash)
                .collect::<Vec<_>>(),
        "planner DAG transaction order"
    );
    let cert_votes = planner["certificate_votes"]
        .as_array()
        .context("certificate votes")?
        .iter()
        .map(|vote| RewardCertVoteFact {
            voter: H160(fixed(&vote["validator"])),
            weight: number(&vote["weight"]),
            period: number(&vote["period"]),
        })
        .collect();
    Ok(FinalChainExecutionRequest {
        pbft_block_rlp: if matches!(
            root["profile"].as_str(),
            Some("synthetic_current" | "synthetic_mainnet_era")
        ) {
            pbft_with_key(number(&period["number"]), &[10; 32])
        } else {
            pbft(number(&period["number"]))
        },
        transactions,
        finalized_dag_blocks: dag,
        blocks_per_year: number(if root["profile"] == "synthetic_mainnet_era" {
            &period["reward_input"]["blocks_per_year"]
        } else {
            &root["configuration"]["dpos"]["blocks_per_year"]
        })
        .try_into()
        .context("blocks per year fits u32")?,
        cert_votes,
        block_gas_limit: duration_active_rules(root, number(&period["number"]))?
            .map_or_else(
                || number(&root["configuration"]["block_gas_limit"]),
                |rules| rules.0,
            )
            .into(),
        mode: FINAL_CHAIN_EXECUTION_MODE_EXTERNAL_EVM_ALLOWED,
    })
}

fn load_fixture() -> Result<Value> {
    let path = std::env::var_os("RUSTAXA_MIXED_FIXTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                "../../../experiments/evm_feasibility/fixtures/mixed_workload_local_observer.json",
            )
        });
    serde_json::from_str(&std::fs::read_to_string(path)?).context("mixed workload fixture")
}

fn verify_receipts(storage: &Storage, period: &Value) -> Result<()> {
    for transaction in period["transactions"].as_array().context("transactions")? {
        ensure!(
            storage
                .final_chain()
                .receipt_by_trx_hash(H256(fixed(&transaction["hash"])))?
                == Some(bytes(&transaction["receipt_rlp"])),
            "durable receipt differs from Go"
        );
    }
    Ok(())
}

fn verify_physical_cf5(path: &Path, expected: &Value) -> Result<()> {
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, path)?;
    let descriptors = columns
        .iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    let handle = db.cf_handle("5").context("concrete CF5")?;
    let actual = db
        .iterator_cf(handle, rocksdb::IteratorMode::Start)
        .map(|row| row.map(|(key, value)| (hex::encode(key), Value::String(hex::encode(value)))))
        .collect::<Result<serde_json::Map<String, Value>, _>>()?;
    ensure!(
        Value::Object(actual) == *expected,
        "Go physical CF5 rows differ"
    );
    Ok(())
}

fn verify_physical_rows(path: &Path, expected: &Value) -> Result<()> {
    let options = rocksdb::Options::default();
    let columns = rocksdb::DB::list_cf(&options, path)?;
    let descriptors = columns
        .iter()
        .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
    let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
    for column in 1..=5 {
        let handle = db.cf_handle(&column.to_string()).context("concrete CF")?;
        let actual = db
            .iterator_cf(handle, rocksdb::IteratorMode::Start)
            .map(|row| {
                row.map(|(key, value)| (hex::encode(key), Value::String(hex::encode(value))))
            })
            .collect::<Result<serde_json::Map<String, Value>, _>>()?;
        ensure!(
            Value::Object(actual) == expected[column - 1],
            "Go physical CF{column} rows differ"
        );
    }
    Ok(())
}

fn verify_final_reader(reader: &ConcreteStateReader, final_state: &Value) -> Result<()> {
    let view = FixtureView {
        prior: FixturePrior::Committed(reader),
        identity: ConcreteStateRead::identity(reader),
    };
    verify_strict_final_view(&view, final_state)
}

fn verify_complete_final_reader(
    reader: &dyn rustaxa_types::concrete_state::execution::ConcreteExecutionRead,
    final_state: &Value,
) -> Result<()> {
    verify_strict_final_view(reader, final_state)
}

fn verify_strict_final_view(
    view: &dyn rustaxa_types::concrete_state::execution::ConcreteExecutionRead,
    final_state: &Value,
) -> Result<()> {
    for account in final_state["accounts"]
        .as_array()
        .context("final accounts")?
    {
        let actual = rustaxa_types::concrete_state::execution::ConcreteExecutionRead::account(
            view,
            fixed(&account["address"]),
        ).with_context(|| format!("final committed account identity={:?} address={} fixture=final.accounts expected_present={} expected_raw={}", view.identity(), account["address"], account["present"], account["raw_account"]))?;
        if account["present"] == Value::Bool(true) {
            let ConcreteRead::Present(actual) = actual else {
                bail!("Go-present final account is missing")
            };
            ensure!(
                actual.physical_rlp == bytes(&account["raw_account"]),
                "final account RLP"
            );
            if number(&account["code_size"]) != 0 {
                ensure!(
                    view.code(fixed(&account["code_hash"])).with_context(|| format!("final committed code identity={:?} address={} code_hash={} fixture=final.accounts.code expected_present=true expected={}", view.identity(), account["address"], account["code_hash"], account["code"]))?
                        == ConcreteRead::Present(bytes(&account["code"])),
                    "final code bytes"
                );
            }
        } else {
            ensure!(matches!(
                actual,
                ConcreteRead::Absent | ConcreteRead::Tombstone
            ));
        }
    }
    for slot in final_state["native_catalog"]["slots"]
        .as_array()
        .context("final native slots")?
    {
        let actual = rustaxa_types::concrete_state::execution::ConcreteExecutionRead::storage(
            view,
            fixed(&slot["address"]),
            ConcreteStorageKey(fixed(&slot["key"])),
        ).with_context(|| format!("final committed storage identity={:?} address={} key={} fixture=final.native_catalog.slots expected_present={} expected_value={}", view.identity(), slot["address"], slot["key"], slot["present"], slot["value"]))?;
        if slot["present"] == Value::Bool(true) {
            ensure!(
                actual == ConcreteRead::Present(bytes(&slot["value"])),
                "final raw slot"
            );
        } else {
            ensure!(
                matches!(actual, ConcreteRead::Absent | ConcreteRead::Tombstone)
                    || actual == ConcreteRead::Present(Vec::new()),
                "final raw deletion"
            );
        }
    }
    Ok(())
}

fn verify_historical_account(reader: &ConcreteStateReader, expected: &Value) -> Result<()> {
    let actual = reader.account(fixed(&expected["address"])).with_context(|| format!("historical account identity={:?} address={} fixture=historical.accounts expected_present={} expected_raw={}", ConcreteStateRead::identity(reader), expected["address"], expected["present"], expected["raw_account"]))?;
    if expected["present"] == Value::Bool(true) {
        let ConcreteRead::Present(actual) = actual else {
            bail!("Go-present historical account is missing")
        };
        ensure!(
            actual.physical_rlp == bytes(&expected["raw_account"]),
            "historical account RLP"
        );
        if number(&expected["code_size"]) != 0 {
            ensure!(
                reader.code(fixed(&expected["code_hash"])).with_context(|| format!("historical code identity={:?} address={} code_hash={} fixture=historical.accounts.code expected_present=true expected={}", ConcreteStateRead::identity(reader), expected["address"], expected["code_hash"], expected["code"]))?
                    == ConcreteRead::Present(bytes(&expected["code"])),
                "historical code bytes"
            );
        }
    } else {
        ensure!(
            matches!(actual, ConcreteRead::Absent | ConcreteRead::Tombstone),
            "Go-absent historical account is present"
        );
    }
    Ok(())
}

fn verify_historical_slot(reader: &ConcreteStateReader, expected: &Value) -> Result<()> {
    let actual = reader.storage(
        fixed(&expected["address"]),
        ConcreteStorageKey(fixed(&expected["key"])),
    ).with_context(|| format!("historical storage identity={:?} address={} key={} fixture=historical.storage expected_present=true expected={}", ConcreteStateRead::identity(reader), expected["address"], expected["key"], expected["raw_value"]))?;
    ensure!(
        actual == ConcreteRead::Present(bytes(&expected["raw_value"])),
        "historical physical slot"
    );
    Ok(())
}

fn historical_reader(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
    period: usize,
) -> Result<ConcreteStateReader> {
    if fixture["profile"] == "synthetic_mainnet_era" {
        let state = fixture["periods"]
            .as_array()
            .context("periods")?
            .iter()
            .find(|row| number(&row["number"]) == period as u64)
            .context("retained fixture period")?;
        return Ok(ConcreteStateReader::open_historical_read_only(
            path,
            committed,
            ConcreteStateIdentity {
                period: FinalChainBlockNumber::new(period as u64),
                state_root: fixed(&state["final"]["root"]),
            },
        )?);
    }
    ConcreteStateReader::open_historical_read_only(
        path,
        committed,
        ConcreteStateIdentity {
            period: FinalChainBlockNumber::new(period as u64),
            state_root: fixed(&fixture["periods"][period - 1]["final"]["root"]),
        },
    )
    .map_err(Into::into)
}

fn verify_historical_reads(
    path: &Path,
    committed: ConcreteStateIdentity,
    fixture: &Value,
) -> Result<()> {
    if fixture["profile"] == "synthetic_current" {
        return native_integration::verify_retained(path, committed, fixture);
    }
    let head = committed.period.as_u64();
    let expected = &fixture["historical_reads"];
    let period_one = historical_reader(path, committed, fixture, 1)?;
    verify_historical_account(&period_one, &expected["period_1_lifecycle_account"])?;
    verify_historical_slot(&period_one, &expected["period_1_lifecycle_slot_0"])?;
    verify_historical_slot(
        &period_one,
        &expected["restored_slot_contract"]["period_1_slot_0"],
    )?;
    verify_historical_slot(
        &period_one,
        &expected["restored_slot_contract"]["period_1_slot_1"],
    )?;
    verify_historical_account(&period_one, &expected["reverted_selfdestruct"]["child"])?;
    verify_historical_account(
        &period_one,
        &expected["reverted_selfdestruct"]["beneficiary"],
    )?;
    drop(period_one);

    if head >= 2 {
        let period_two = historical_reader(path, committed, fixture, 2)?;
        verify_historical_account(&period_two, &expected["period_2_lifecycle_account"])?;
        // Raw physical history deliberately survives after the account is
        // deleted and its logical trie path is absent.
        verify_historical_slot(
            &period_two,
            &expected["period_2_orphaned_lifecycle_storage_row"],
        )?;
        verify_historical_account(
            &period_two,
            &expected["native_custody"]["dispatcher_period_2"],
        )?;
        drop(period_two);

        let retained = &expected["retained_period_1_lifecycle_cf5"];
        ensure!(
            bytes(&retained["physical_key"])
                == [
                    bytes(&retained["logical_key"]),
                    number(&retained["period"]).to_be_bytes().to_vec(),
                ]
                .concat(),
            "retained CF5 physical-key layout"
        );
        let options = rocksdb::Options::default();
        let columns = rocksdb::DB::list_cf(&options, path)?;
        let descriptors = columns
            .iter()
            .map(|name| rocksdb::ColumnFamilyDescriptor::new(name, rocksdb::Options::default()));
        let db = rocksdb::DB::open_cf_descriptors_read_only(&options, path, descriptors, false)?;
        let cf5 = db.cf_handle("5").context("concrete CF5")?;
        ensure!(
            db.get_cf(cf5, bytes(&retained["physical_key"]))? == Some(bytes(&retained["value"])),
            "retained period-one CF5 row"
        );
        drop(db);
    }

    if head >= 4 {
        let period_four = historical_reader(path, committed, fixture, 4)?;
        verify_historical_slot(
            &period_four,
            &expected["restored_slot_contract"]["period_4_slot_0"],
        )?;
        verify_historical_slot(
            &period_four,
            &expected["restored_slot_contract"]["period_4_slot_1"],
        )?;
        verify_historical_account(
            &period_four,
            &expected["native_custody"]["dispatcher_period_4"],
        )?;
    }
    Ok(())
}

fn run_period(
    fixture: &Value,
    period: &Value,
    application_path: &Path,
    concrete_path: &Path,
    fresh: bool,
    expected_chain_identity: Option<[u8; 32]>,
) -> Result<[u8; 32]> {
    let (application, mut chain) = open_chain(application_path, fixture)?;
    ensure!(
        application
            .final_chain()
            .external_evm_pending_publication_raw()?
            .is_none(),
        "application unexpectedly needs recovery"
    );
    let chain_identity = chain.concrete_chain_identity()?;
    if let Some(expected) = expected_chain_identity {
        ensure!(
            chain_identity == expected,
            "reopened concrete chain identity"
        );
    }
    let concrete = if fresh {
        let concrete = fresh_concrete(concrete_path, &chain, fixture)?;
        chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
        concrete
    } else {
        let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, chain_identity)?;
        let genesis_identity = ConcreteStateIdentity {
            period: FinalChainBlockNumber::GENESIS,
            state_root: fixed(&fixture["genesis"]["root"]),
        };
        let genesis_reader = ConcreteStateReader::open_historical_read_only(
            concrete_path,
            observed.committed,
            genesis_identity,
        )?;
        chain.hydrate_concrete_genesis_accounts(&genesis_reader)?;
        drop(genesis_reader);
        ConcreteStateLifecycle::open(concrete_path, chain_identity, observed.committed)?
    };
    let adapter = Adapter {
        chain: &chain,
        application: &application,
        concrete: RefCell::new(Some(concrete)),
        staged: RefCell::new(None),
        native: RefCell::new(None),
        native_context: RefCell::new(None),
        physical_receipt: None,
        state_api_epoch: StateApiEpoch::new(),
        ordinary_observer: None,
        fixture: period,
        account_node_publication: AccountNodePublication::ObserverHistory,
    };
    initialize_and_recover_final_chain_application_state_at_joint_startup(&chain, &adapter)?;
    let author = fixed(&period["reward_input"]["block_author"]);
    let execution_request = request(fixture, period)?;
    chain.ensure_period_data(
        FinalChainBlockNumber::new(number(&period["number"])),
        &period_data(&execution_request),
    )?;
    let report = execute_final_chain_application_task(
        &chain,
        execution_request,
        FinalChainProposalPeriodDagLevelUpdate::default(),
        false,
        fixed(&fixture["configuration"]["hardforks"]["ficus"]["bridge_contract_address"]),
        &adapter,
    )?;
    let period_number = FinalChainBlockNumber::new(number(&period["number"]));
    ensure!(
        period["planner_facts"]["is_pillar_boundary"] == Value::Bool(false),
        "mixed period unexpectedly configured as a pillar boundary"
    );
    ensure!(report.period == period_number);
    ensure!(author == fixed(&period["planner_facts"]["dag_blocks"][0]["author"]));
    let header = application
        .final_chain()
        .block_header_raw(period_number.as_u64())?
        .context("published mixed-period header")?;
    let header = StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&header))?;
    let public_header = chain
        .block_header(period_number)?
        .context("published materialized mixed-period header")?;
    ensure!(
        Rlp::new(&public_header).val_at::<H160>(2)?.0 == author,
        "published validator author"
    );
    ensure!(
        header.state_root.0 == fixed(&period["final"]["root"]),
        "published root"
    );
    ensure!(
        header.gas_used.as_u64()
            == period["transactions"]
                .as_array()
                .context("transactions")?
                .iter()
                .map(|transaction| number(&transaction["gas_used"]))
                .sum::<u64>(),
        "published gas"
    );
    ensure!(
        header.total_reward.as_u256() == decimal_u256(&period["reward_output"]["minted_reward"]),
        "published reward"
    );
    verify_receipts(&application, period)?;
    drop(adapter);
    drop(chain);
    drop(application);

    verify_physical_rows(concrete_path, &period["final"]["rows"])?;
    let (application, mut chain) = open_chain(application_path, fixture)?;
    let observed = ConcreteStateLifecycle::inspect_existing(concrete_path, chain_identity)?;
    ensure!(observed.committed.period == period_number);
    ensure!(observed.committed.state_root == fixed(&period["final"]["root"]));
    ensure!(observed.pending_marker_rlp.is_empty());
    let genesis_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::GENESIS,
        state_root: fixed(&fixture["genesis"]["root"]),
    };
    let genesis_reader = ConcreteStateReader::open_historical_read_only(
        concrete_path,
        observed.committed,
        genesis_identity,
    )?;
    chain.hydrate_concrete_genesis_accounts(&genesis_reader)?;
    drop(genesis_reader);
    let concrete = ConcreteStateLifecycle::open(concrete_path, chain_identity, observed.committed)?;
    ensure!(chain.last_block_number_typed()? == period_number);
    let validator = if period["planner_facts"]["certificate_votes"]
        .as_array()
        .context("certificate votes")?
        .is_empty()
    {
        fixed(&period["reward_input"]["block_author"])
    } else {
        fixed(&period["planner_facts"]["certificate_votes"][0]["validator"])
    };
    ensure!(
        chain.dpos_eligible_vote_count(period_number, validator)?
            == number(&period["planner_facts"]["validator_eligible_vote_count"]),
        "delayed validator vote count"
    );
    ensure!(
        chain.dpos_eligible_total_vote_count(period_number)?
            == number(&period["planner_facts"]["total_eligible_vote_count"]),
        "delayed total vote count"
    );
    verify_receipts(&application, period)?;
    drop(concrete);
    let reader = ConcreteStateReader::open_read_only(concrete_path, observed.committed)?;
    verify_final_reader(&reader, &period["final"])?;
    drop(reader);
    verify_historical_reads(concrete_path, observed.committed, fixture)?;
    drop(chain);
    drop(application);
    verify_physical_rows(concrete_path, &period["final"]["rows"])?;
    Ok(chain_identity)
}

#[test]
fn four_mixed_periods_commit_and_reopen_exactly() -> Result<()> {
    let fixture = load_fixture()?;
    ensure!(
        fixture["execution_mode"] == "observer",
        "observer fixture required"
    );
    ensure!(
        number(&fixture["configuration"]["chain_id"]) == 841,
        "fixture chain id"
    );
    let periods = fixture["periods"].as_array().context("fixture periods")?;
    ensure!(periods.len() == 4, "four-period fixture required");
    let expected_invocations = [0, 11, 4, 3];
    for (index, (period, count)) in periods.iter().zip(expected_invocations).enumerate() {
        ensure!(
            number(&period["number"]) == (index + 1) as u64,
            "consecutive mixed period"
        );
        ensure!(
            period["period_catalog_identities"]["invocations"]
                .as_array()
                .context("period invocation identities")?
                .len()
                == count,
            "frozen native invocation count"
        );
    }

    let root =
        std::env::temp_dir().join(format!("rustaxa-evm-mixed-periods-{}", std::process::id()));
    let mut scratch = era_retention::register(&root, &fixture)?;
    std::fs::create_dir(&root)?;
    era_retention::created(&mut scratch)?;
    let application_path = root.join("application");
    let concrete_path = root.join("state_db");
    let mut chain_identity = None;
    for (index, period) in periods.iter().enumerate() {
        chain_identity = Some(run_period(
            &fixture,
            period,
            &application_path,
            &concrete_path,
            index == 0,
            chain_identity,
        )?);
    }
    if scratch.is_some() {
        era_retention::finish(
            scratch,
            era_retention::final_evidence(
                &root,
                &fixture,
                "four mixed periods,all assertions,physical reopen/native/history/receipts; owners closed",
            )?,
            false,
        )?;
    } else {
        std::fs::remove_dir_all(root)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DurationRecoveryAuthority {
    FreshPrefix,
    RehearsalSeed([u8; 32]),
}

// A recovery copy is created only after the checked prefix and both closed-store manifests agree.
struct DurationVerifiedRecoveryCopy {
    authority: DurationRecoveryAuthority,
    mode: &'static str,
    source_manifest: [u8; 32],
    contract_sha256: [u8; 32],
    rust_binary_sha256: [u8; 32],
    prefix_owner_sha256: [u8; 32],
    source_path: std::path::PathBuf,
    seed: rustaxa_types::concrete_state::ConcreteStateIdentity,
    completed_count: u64,
    prefix_rolling: [u8; 32],
    effects_rolling: [u8; 32],
    target: std::path::PathBuf,
    identity: rustaxa_types::concrete_lifecycle::FinalChainConcreteIdentity,
    committed: rustaxa_types::concrete_state::ConcreteStateIdentity,
    generation: u64,
    historical: Vec<rustaxa_types::concrete_state::ConcreteStateIdentity>,
    selected: Vec<serde_json::Value>,
    prior_public_slots: serde_json::Value,
    copy_manifest_sha256: [u8; 32],
}

#[path = "support/mixed_recovery.rs"]
mod mixed_recovery;

#[path = "support/native_integration.rs"]
mod native_integration;

#[path = "../../rustaxa-storage/src/closeout_test_scratch.rs"]
pub mod closeout_test_scratch;

#[test]
fn c_composed_fee_product_wraps256_before_wide_addition() {
    let high = BigUint::from(1_u8) << 255_usize;
    assert_eq!(c_composed_transaction_fee(&high, 3), high);
    assert_eq!(
        c_composed_transaction_fee(&(BigUint::from(1_u8) << 256_usize), 7),
        BigUint::default()
    );
    assert_eq!(
        c_composed_transaction_fee(&BigUint::from(1_000_000_000_u64), 21000),
        BigUint::from(21_000_000_000_000_u64)
    );
}

#[derive(Clone)]
struct FeeJournalRead {
    author: [u8; 20],
    present: bool,
    unavailable: bool,
}
impl rustaxa_types::concrete_state::execution::ConcreteExecutionRead for FeeJournalRead {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateIdentity {
            period: 1.into(),
            state_root: [0; 32],
        }
    }
    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if address != self.author {
            return Ok(ConcreteRead::Absent);
        }
        if self.unavailable {
            return Err(ConcreteReadError::HistoryUnavailable(self.identity()));
        }
        Ok(if self.present {
            ConcreteRead::Present(record(
                7.into(),
                ConcreteAccountBalance::new(BigUint::from(50_u64)),
                Some([0x55; 32]),
                2,
            ))
        } else {
            ConcreteRead::Absent
        })
    }
    fn storage(
        &self,
        _: [u8; 20],
        _: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
    fn code(&self, _: [u8; 32]) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
}
#[test]
fn c_composed_fee_journal_preserves_metadata_and_uses_current_self_author_balance() {
    let author = [0x71; 20];
    let mut journal = ExecutionJournal::new(FeeJournalRead {
        author,
        present: true,
        unavailable: false,
    });
    // Model the real outer envelope's net debit/refund before the fee credit.
    journal
        .subtract_balance(author, &BigUint::from(9_u64))
        .unwrap();
    credit_c_composed_transaction_fee(&mut journal, author, &BigUint::from(3_u64), 3).unwrap();
    let account = journal.account(author).unwrap();
    assert_eq!(account.nonce, FinalChainNonce::from_u64(7));
    assert_eq!(account.balance.value(), &num_bigint::BigInt::from(50));
    let metadata = journal.account_metadata(author).unwrap();
    assert_eq!(metadata.code_hash, Some([0x55; 32]));
    assert_eq!(metadata.code_size, 2);
    let settled = journal.settle_transaction().unwrap();
    assert!(settled.writes.ordinary_storage.is_empty() && settled.writes.raw_storage.is_empty());
}
#[test]
fn c_composed_fee_journal_absent_zero_and_unavailable_author_are_isolated() {
    let author = [0x71; 20];
    for fee in [0_u64, 21] {
        let mut journal = ExecutionJournal::new(FeeJournalRead {
            author,
            present: false,
            unavailable: false,
        });
        credit_c_composed_transaction_fee(&mut journal, author, &BigUint::from(fee), 1).unwrap();
        let account = journal.account(author).unwrap();
        assert!(account.exists);
        assert_eq!(account.balance.value(), &num_bigint::BigInt::from(fee));
        let settled = journal.settle_transaction().unwrap();
        if fee == 0 {
            assert!(
                settled
                    .writes
                    .accounts
                    .iter()
                    .any(|row| row.address == author
                        && matches!(row.operation, JournalAccountOperation::Delete))
            );
        }
    }
    let mut failed = ExecutionJournal::new(FeeJournalRead {
        author,
        present: false,
        unavailable: true,
    });
    assert!(
        credit_c_composed_transaction_fee(&mut failed, author, &BigUint::from(21_u64), 1).is_err()
    );
    assert!(
        failed
            .settle_transaction()
            .unwrap()
            .writes
            .accounts
            .is_empty()
    );
    let mut retry = ExecutionJournal::new(FeeJournalRead {
        author,
        present: false,
        unavailable: false,
    });
    credit_c_composed_transaction_fee(&mut retry, author, &BigUint::from(21_u64), 1).unwrap();
    assert_eq!(
        retry.account(author).unwrap().balance.value(),
        &num_bigint::BigInt::from(21)
    );
}

#[test]
fn c_composed_fee_credit_survives_reverted_inner_frame_and_remains_wide() {
    let author = [0x71; 20];
    let mut journal = ExecutionJournal::new(FeeJournalRead {
        author,
        present: true,
        unavailable: false,
    });
    let checkpoint = journal.checkpoint();
    journal
        .subtract_balance(author, &BigUint::from(13_u64))
        .unwrap();
    journal.revert_checkpoint(checkpoint).unwrap();
    let high = BigUint::from(1_u8) << 255_usize;
    credit_c_composed_transaction_fee(&mut journal, author, &high, 1).unwrap();
    credit_c_composed_transaction_fee(&mut journal, author, &high, 1).unwrap();
    assert_eq!(
        journal.account(author).unwrap().balance.value(),
        &num_bigint::BigInt::from((BigUint::from(1_u8) << 256_usize) + BigUint::from(50_u64))
    );
    assert!(
        journal
            .settle_transaction()
            .unwrap()
            .writes
            .raw_storage
            .is_empty()
    );
}

// Actual transaction and real owner EndBlock component. Root parity requires
// captured cross-language comparison; no Go post-writes are used as inputs.
#[path = "support/zero_fee_checkpoint_component.rs"]
mod zero_fee_checkpoint_component;
