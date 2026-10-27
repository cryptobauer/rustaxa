//! The signed registration bootstrap through the existing native EVM adapter.
#[path = "mixed_genesis.rs"]
mod genesis;
use anyhow::{Result, ensure};
use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::*,
    driver::{
        NativeAddressClassifier, PeriodConsensusSequence, execute_top_level_call_with_native,
    },
    envelope::EnvelopeRules,
    journal::ExecutionJournal,
    profile::TaraxaProfile,
    simulation::simulate_with_native,
};
use rustaxa_types::{FinalChainBlockNumber, FinalChainNonce, concrete_state::*};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
fn bytes(v: &Value) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}
fn fixed<const N: usize>(v: &Value) -> [u8; N] {
    bytes(v).try_into().unwrap()
}
fn amount(v: &Value) -> BigUint {
    BigUint::parse_bytes(v.as_str().unwrap().as_bytes(), 10).unwrap()
}
fn fixture(stream: &str) -> Result<Value> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../experiments/evm_feasibility/fixtures/native_integration");
    let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("manifest.json"))?)?;
    let pin = &manifest["streams"][stream];
    let stored = std::fs::read(root.join(pin["file"].as_str().unwrap()))?;
    let mut body = Vec::new();
    if pin["encoding"] == "gzip" {
        flate2::read::GzDecoder::new(stored.as_slice()).read_to_end(&mut body)?;
    } else {
        body = stored;
    }
    ensure!(body.len() as u64 == pin["bytes"].as_u64().unwrap());
    ensure!(
        hex::encode(
            revm::precompile::hash::sha256_run(&body, u64::MAX)
                .unwrap()
                .bytes
        ) == pin["sha256"]
    );
    Ok(serde_json::from_slice(&body)?)
}
#[derive(Clone)]
struct State {
    identity: ConcreteStateIdentity,
    accounts: BTreeMap<[u8; 20], ConcreteAccountRecord>,
    slots: BTreeMap<([u8; 20], ConcreteStorageKey), Vec<u8>>,
}
impl ConcreteStateRead for State {
    fn identity(&self) -> ConcreteStateIdentity {
        self.identity
    }
    fn account(
        &self,
        a: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        Ok(self
            .accounts
            .get(&a)
            .cloned()
            .map(ConcreteRead::Present)
            .unwrap_or(ConcreteRead::Absent))
    }
    fn storage(
        &self,
        a: [u8; 20],
        k: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(self
            .slots
            .get(&(a, k))
            .cloned()
            .map(ConcreteRead::Present)
            .unwrap_or(ConcreteRead::Absent))
    }
    fn code(&self, _: [u8; 32]) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        Ok(ConcreteRead::Absent)
    }
}
struct Natives;
impl NativeAddressClassifier for Natives {
    fn is_native_address(&self, _: FinalChainBlockNumber, a: [u8; 20]) -> bool {
        a[..19] == [0; 19] && matches!(a[19], 0xfe | 0xee)
    }
}
struct NoHashes;
impl BlockHashRead for NoHashes {
    fn block_hash(
        &self,
        p: FinalChainBlockNumber,
    ) -> std::result::Result<[u8; 32], BlockHashReadError> {
        Err(BlockHashReadError::HistoryUnavailable(p))
    }
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        crate::closeout_test_scratch::dispose(&self.0).unwrap();
    }
}
pub(super) fn run(simulation: bool) -> Result<()> {
    let f = fixture("local.observer.traced")?;
    let base = std::env::var_os("RUN_ARTIFACT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let path = Scratch(base.join(format!(
            "native-bootstrap-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )));
    crate::closeout_test_scratch::register(&path.0);
    std::fs::create_dir(&path.0)?;
    crate::closeout_test_scratch::created(&path.0);
    let (_storage, mut chain) = genesis::open_chain(&path.0.join("application"), &f)?;
    let concrete = genesis::fresh_concrete(&path.0.join("concrete"), &chain, &f)?;
    chain.hydrate_concrete_genesis_accounts(concrete.prior_reader())?;
    let catalog = &f["genesis"]["native_catalog"];
    let mut state = State {
        identity: ConcreteStateIdentity {
            period: 0.into(),
            state_root: fixed(&f["genesis"]["root"]),
        },
        accounts: BTreeMap::new(),
        slots: BTreeMap::new(),
    };
    for row in catalog["accounts"].as_array().unwrap() {
        if row["present"] == true {
            let a = fixed(&row["address"]);
            let ConcreteRead::Present(account) = concrete.prior_reader().account(a)? else {
                anyhow::bail!("genesis account")
            };
            state.accounts.insert(a, account);
        }
    }
    for row in catalog["slots"].as_array().unwrap() {
        if row["present"] == true {
            state.slots.insert(
                (
                    fixed(&row["address"]),
                    ConcreteStorageKey(fixed(&row["key"])),
                ),
                bytes(&row["value"]),
            );
        }
    }
    let block = ExecutionBlockContext {
        period: 1.into(),
        author: fixed(&f["periods"][0]["reward_input"]["block_author"]),
        timestamp: 0,
        gas_limit: 10_000_000.into(),
        chain_id: 666,
        difficulty: BigUint::default(),
    };
    if simulation {
        let api = fixture("local.observer.traced.api")?;
        let case = &api["cases"][0];
        let row = &case["request"];
        let req = ExecutionTransaction {
            position: 0.into(),
            hash: [0; 32],
            sender: fixed(&row["sender"]),
            receiver: Some(fixed(&row["to"])),
            nonce: FinalChainNonce::from_bytes(&amount(&row["supplied_nonce"]).to_bytes_be())?,
            gas_price: ExecutionGasPrice::new(1u8.into()),
            gas_limit: 500_000.into(),
            value: ExecutionValue::new(amount(&row["value"])),
            input: bytes(&row["input"]),
            canonical_rlp: None,
            kind: ExecutionTransactionKind::Call,
        };
        for _ in 0..2 {
            let mut historical = block.clone();
            historical.period = 0.into();
            let out = simulate_with_native(
                &state,
                &NoHashes,
                &Natives,
                &Natives,
                |_| {
                    Ok(super::mixed_native::SimulationNativeExecutionPort::new(
                        chain.begin_native_simulation(0.into()).unwrap(),
                    ))
                },
                &historical,
                &req,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            compare(&out.execution, &case["call"]["output"])?;
        }
    } else {
        let mut sequence = PeriodConsensusSequence::new(1.into());
        let mut port = super::mixed_native::MixedNativeExecutionPort::new(
            chain.begin_native_session(1.into(), 0.into())?,
        );
        for (i, row) in f["periods"][0]["transactions"].as_array().unwrap()[..3]
            .iter()
            .enumerate()
        {
            let mut journal = ExecutionJournal::new(state.clone());
            let req = ExecutionTransaction {
                position: (i as u32).into(),
                hash: fixed(&row["hash"]),
                sender: fixed(&row["sender"]),
                receiver: Some(fixed(&row["to"])),
                nonce: FinalChainNonce::zero(),
                gas_price: ExecutionGasPrice::new(1u8.into()),
                gas_limit: 500_000.into(),
                value: ExecutionValue::new(amount(&row["value"])),
                input: bytes(&row["input"]),
                canonical_rlp: Some(bytes(&row["signed_rlp"])),
                kind: ExecutionTransactionKind::Call,
            };
            let result = execute_top_level_call_with_native(
                &mut journal,
                &NoHashes,
                &Natives,
                &Natives,
                &mut port,
                &mut sequence,
                &block,
                &req,
                EnvelopeRules { cornus: true },
                TaraxaProfile::new(false),
            )?;
            compare(&result, row)?;
            let settled = journal.settle_transaction()?;
            for write in settled.writes.raw_storage {
                state.slots.insert(
                    (write.address, write.key),
                    match write.operation {
                        NativeRawOperation::Put(v) => v.as_bytes().to_vec(),
                        NativeRawOperation::Delete => Vec::new(),
                    },
                );
            }
            for write in settled.writes.accounts {
                match write.operation {
                    rustaxa_evm::journal::JournalAccountOperation::Delete => {
                        state.accounts.remove(&write.address);
                    }
                    rustaxa_evm::journal::JournalAccountOperation::Upsert {
                        nonce,
                        balance,
                        code_hash,
                        code_size,
                    } => {
                        let mut raw = rlp::RlpStream::new_list(5);
                        raw.append(&nonce.to_bytes())
                            .append(&balance.value().to_bytes_be())
                            .append_empty_data();
                        if let Some(hash) = code_hash {
                            raw.append(&hash.as_slice());
                        } else {
                            raw.append_empty_data();
                        }
                        raw.append(&code_size);
                        state.accounts.insert(
                            write.address,
                            ConcreteAccountRecord {
                                account: ConcreteAccount {
                                    nonce,
                                    balance,
                                    storage_root: None,
                                    code_hash,
                                    code_size,
                                },
                                physical_rlp: raw.out().to_vec(),
                            },
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
fn compare(result: &TransactionExecutionResult, row: &Value) -> Result<()> {
    let TransactionExecutionResult::Executed(out) = result else {
        anyhow::bail!("registration admission")
    };
    ensure!(out.status == CodeExecutionStatus::Success);
    ensure!(out.gas_used.as_u64() == row["gas_used"].as_u64().unwrap());
    ensure!(out.output == bytes(&row["output"]));
    let logs=out.logs.iter().map(|l|serde_json::json!({"address":hex::encode(l.address),"topics":l.topics.iter().map(hex::encode).collect::<Vec<_>>(),"data":hex::encode(&l.data)})).collect::<Vec<_>>();
    ensure!(logs == *row["logs"].as_array().unwrap());
    Ok(())
}
