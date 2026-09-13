//! Read-only, demand-driven execution preflight for retained period 25,706,949.
//!
//! The probe opens an independent light-node snapshot copy, authenticates every
//! concrete state read through the historical Rust reader, and executes the
//! exact retained ordinary transactions in order. It carries settled account
//! values in a memory-only block overlay. It never prepares, writes, adopts, or
//! publishes concrete state and therefore makes no reproduced-root claim.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use num_bigint::BigUint;
use rlp::{Rlp, RlpStream};
use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options};
use rustaxa_evm::contracts::{
    BlockHashRead, BlockHashReadError, CodeExecutionStatus, ExecutionBlockContext,
    ExecutionTransactionKind, TransactionExecutionResult,
};
use rustaxa_evm::driver::{NativeAddressClassifier, execute_top_level_call};
use rustaxa_evm::envelope::EnvelopeRules;
use rustaxa_evm::input::{LegacyInputKind, decode_legacy_input};
use rustaxa_evm::journal::{ExecutionJournal, JournalAccountOperation, JournalWritePlan};
use rustaxa_evm::profile::TaraxaProfile;
use rustaxa_storage::{Column, ConcreteStateReader, FinalChainRepository, TransactionRepository};
use rustaxa_types::codec::rlp::final_chain::StoredBlockHeaderRlp;
use rustaxa_types::codec::rlp::pbft::SignedPbftBlockRlp;
use rustaxa_types::concrete_state::execution::ConcreteExecutionRead;
use rustaxa_types::concrete_state::{
    ConcreteAccount, ConcreteAccountRecord, ConcreteRead, ConcreteReadError, ConcreteStateIdentity,
    ConcreteStateRead, ConcreteStorageKey,
};
use rustaxa_types::{
    FinalChainBlockNumber, FinalChainGas, FinalChainTransactionPosition, LegacyTransactionEnvelope,
    PbftBlockMetadata, StoredFinalChainBlockHeader,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const SUPPLIED_EVIDENCE: &str = "/tmp/snapshot-litenode";
const TARGET_PERIOD: u64 = 25_706_949;
const CHAIN_ID: u64 = 841;
const CORNUS_PBFT_GAS_LIMIT: u64 = 0x7d2b7500;
const REQUIRED_COLUMNS: &[&str] = &[
    "period_data",
    "final_chain_meta",
    "final_chain_blk_by_number",
    "final_chain_blk_hash_by_number",
    "final_chain_receipt_by_period",
];
type Database = DBWithThreadMode<MultiThreaded>;

#[derive(Serialize)]
struct Report {
    schema: u32,
    tool_source_sha256: String,
    input_copy: String,
    open_mode: &'static str,
    period: u64,
    prior_period: u64,
    prior_root_hex: String,
    execution_context: ContextReport,
    envelope_classification: EnvelopeClassification,
    prior_state_dependencies: Dependencies,
    outcomes: Vec<Outcome>,
    receipt_row_sha256: String,
    receipt_row_exact_match: bool,
    total_gas_used: u64,
    header_gas_used: u64,
    qualification: Qualification,
    open_dependencies: Vec<&'static str>,
}

#[derive(Serialize)]
struct ContextReport {
    pbft_author_hex: String,
    pbft_timestamp: u64,
    chain_id: u64,
    block_gas_limit: u64,
    envelope_cornus: bool,
    profile_cacti: bool,
    block_context_observed_by_bytecode: bool,
    block_hash_reads: Vec<u64>,
    native_address_checks: Vec<String>,
}

#[derive(Serialize)]
struct EnvelopeClassification {
    expected_count: usize,
    exact_count: usize,
    every_signature_decoded: bool,
    every_chain_id_841: bool,
    every_call: bool,
    every_input_empty: bool,
    every_receiver_non_native: bool,
    every_receiver_has_no_code: bool,
    entries: Vec<EnvelopeEntry>,
}

#[derive(Serialize)]
struct EnvelopeEntry {
    position: usize,
    hash_hex: String,
    rlp_sha256: String,
    sender_hex: String,
    receiver_hex: Option<String>,
    nonce_hex: String,
    gas_price_hex: String,
    gas_limit: u64,
    chain_id: u64,
    value_hex: String,
    input_hex: String,
    kind: &'static str,
}

#[derive(Default, Serialize)]
struct Dependencies {
    accounts: Vec<AccountDependency>,
    code: Vec<CodeDependency>,
    slots: Vec<SlotDependency>,
}

#[derive(Clone, Serialize)]
struct AccountDependency {
    address_hex: String,
    result: &'static str,
    physical_rlp_sha256: Option<String>,
    physical_rlp_hex: Option<String>,
    nonce_hex: Option<String>,
    balance_hex: Option<String>,
    storage_root_hex: Option<String>,
    code_hash_hex: Option<String>,
    code_size: Option<u64>,
}

#[derive(Clone, Serialize)]
struct CodeDependency {
    code_hash_hex: String,
    result: &'static str,
    bytes: usize,
    sha256: Option<String>,
}

#[derive(Clone, Serialize)]
struct SlotDependency {
    address_hex: String,
    key_hex: String,
    result: &'static str,
    value_hex: Option<String>,
    sha256: Option<String>,
}

#[derive(Serialize)]
struct Outcome {
    position: usize,
    hash_hex: String,
    status: u8,
    gas_used: u64,
    cumulative_gas_used: u64,
    logs: usize,
    output_hex: String,
    expected_receipt_sha256: String,
    produced_receipt_sha256: String,
    exact_receipt_match: bool,
    account_writes: usize,
}

#[derive(Serialize)]
struct Qualification {
    exact_ordered_ordinary_execution: bool,
    demanded_prior_state_reads_authenticated: bool,
    exact_receipts_reproduced: bool,
    touched_read_closure_qualified: bool,
    reward_transition_qualified: bool,
    concrete_writer_bootstrap_qualified: bool,
    final_state_root_reproduced: bool,
    publication_authorized: bool,
}

#[derive(Default)]
struct Recorder {
    accounts: BTreeMap<[u8; 20], AccountDependency>,
    code: BTreeMap<[u8; 32], CodeDependency>,
    slots: BTreeMap<([u8; 20], [u8; 32]), SlotDependency>,
}

/// Memory-only sequential account overlay on one authenticated historical view.
struct ReplayView<'a> {
    base: &'a ConcreteStateReader,
    accounts: &'a BTreeMap<[u8; 20], ConcreteRead<ConcreteAccountRecord>>,
    recorder: &'a RefCell<Recorder>,
}

impl ConcreteExecutionRead for ReplayView<'_> {
    fn identity(&self) -> ConcreteStateIdentity {
        ConcreteStateRead::identity(self.base)
    }

    fn account(
        &self,
        address: [u8; 20],
    ) -> std::result::Result<ConcreteRead<ConcreteAccountRecord>, ConcreteReadError> {
        if let Some(value) = self.accounts.get(&address) {
            return Ok(value.clone());
        }
        let value = ConcreteStateRead::account(self.base, address)?;
        self.recorder
            .borrow_mut()
            .accounts
            .entry(address)
            .or_insert_with(|| account_dependency(address, &value));
        Ok(value)
    }

    fn storage(
        &self,
        address: [u8; 20],
        key: ConcreteStorageKey,
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let value = ConcreteStateRead::storage(self.base, address, key)?;
        self.recorder
            .borrow_mut()
            .slots
            .entry((address, key.0))
            .or_insert_with(|| slot_dependency(address, key, &value));
        Ok(value)
    }

    fn code(
        &self,
        code_hash: [u8; 32],
    ) -> std::result::Result<ConcreteRead<Vec<u8>>, ConcreteReadError> {
        let value = ConcreteStateRead::code(self.base, code_hash)?;
        self.recorder
            .borrow_mut()
            .code
            .entry(code_hash)
            .or_insert_with(|| code_dependency(code_hash, &value));
        Ok(value)
    }
}

struct AppBlockHashes<'a> {
    repository: &'a FinalChainRepository<Database>,
    reads: &'a RefCell<Vec<u64>>,
}

impl BlockHashRead for AppBlockHashes<'_> {
    fn block_hash(
        &self,
        number: FinalChainBlockNumber,
    ) -> std::result::Result<[u8; 32], BlockHashReadError> {
        self.reads.borrow_mut().push(number.as_u64());
        self.repository
            .block_hash_by_number(number.as_u64())
            .map_err(|error| BlockHashReadError::Io(error.to_string()))?
            .ok_or(BlockHashReadError::HistoryUnavailable(number))?
            .try_into()
            .map_err(|_| BlockHashReadError::Corrupt("block hash is not 32 bytes".into()))
    }
}

#[derive(Default)]
struct MainnetNativeClassifier {
    checks: RefCell<Vec<[u8; 20]>>,
}

impl NativeAddressClassifier for MainnetNativeClassifier {
    fn is_native_address(&self, _: FinalChainBlockNumber, address: [u8; 20]) -> bool {
        self.checks.borrow_mut().push(address);
        address[..19] == [0; 19] && matches!(address[19], 1..=9 | 0xee | 0xfe)
    }
}

fn main() -> Result<()> {
    let (input, output) = validated_paths()?;
    let app_path = canonical_child(&input, "db/db")?;
    let state_path = canonical_child(&input, "db/state_db")?;
    let application = Arc::new(open_application_read_only(&app_path)?);
    let transactions = TransactionRepository::new(application.clone());
    let final_chain = FinalChainRepository::new(application.clone());

    let head = exact_le_u64(
        &final_chain
            .meta_value(1)?
            .context("missing FinalChain head")?,
        "FinalChain head",
    )?;
    ensure!(head == TARGET_PERIOD, "unexpected snapshot head {head}");
    let prior_period = TARGET_PERIOD - 1;
    let current_header = decode_header(&final_chain, TARGET_PERIOD)?;
    let prior_header = decode_header(&final_chain, prior_period)?;
    let current_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(TARGET_PERIOD),
        state_root: current_header.state_root.0,
    };
    let current = ConcreteStateReader::open_read_only(&state_path, current_identity)?;
    let prior_identity = ConcreteStateIdentity {
        period: FinalChainBlockNumber::new(prior_period),
        state_root: prior_header.state_root.0,
    };
    let prior = ConcreteStateReader::open_historical_read_only(
        &state_path,
        ConcreteStateRead::identity(&current),
        prior_identity,
    )?;
    drop(current);

    let period_data =
        rustaxa_storage::PeriodRepository::new(application.clone()).data_raw(TARGET_PERIOD)?;
    let period_rlp = Rlp::new(&period_data);
    let pbft = PbftBlockMetadata::try_from(SignedPbftBlockRlp::new(period_rlp.at(0)?.as_raw()))?;
    ensure!(
        pbft.period == TARGET_PERIOD,
        "PBFT metadata period mismatch"
    );

    let count = usize::try_from(transactions.count(TARGET_PERIOD)?)?;
    ensure!(count == 19, "expected exactly 19 ordinary transactions");
    ensure!(
        transactions
            .period_system_hashes_rlp(TARGET_PERIOD)?
            .is_empty(),
        "period contains system transactions"
    );
    let receipts_rlp =
        rustaxa_storage::PeriodRepository::new(application.clone()).receipt(TARGET_PERIOD)?;
    let receipts = Rlp::new(&receipts_rlp);
    ensure!(receipts.item_count()? == count, "receipt count mismatch");

    let recorder = RefCell::new(Recorder::default());
    let block_hash_reads = RefCell::new(Vec::new());
    let hashes = AppBlockHashes {
        repository: &final_chain,
        reads: &block_hash_reads,
    };
    let natives = MainnetNativeClassifier::default();
    let block = ExecutionBlockContext {
        period: FinalChainBlockNumber::new(TARGET_PERIOD),
        author: pbft.author.0,
        timestamp: pbft.timestamp,
        gas_limit: FinalChainGas::new(CORNUS_PBFT_GAS_LIMIT),
        chain_id: CHAIN_ID,
        difficulty: BigUint::default(),
    };
    let mut overlay = BTreeMap::new();
    let mut envelopes = Vec::with_capacity(count);
    let mut outcomes = Vec::with_capacity(count);
    let mut cumulative = 0_u64;

    for position in 0..count {
        let rlp = transactions
            .by_period_position_rlp(TARGET_PERIOD, u32::try_from(position)?)?
            .with_context(|| format!("missing transaction {position}"))?;
        let tx = decode_legacy_input(
            FinalChainTransactionPosition::try_from(position)?,
            &rlp,
            LegacyInputKind::Signed,
        )?;
        let envelope = envelope_entry(position, &tx, &rlp)?;
        ensure!(
            envelope.chain_id == CHAIN_ID,
            "transaction {position} has unexpected chain ID {}",
            envelope.chain_id
        );
        envelopes.push(envelope);
        ensure!(
            tx.kind == ExecutionTransactionKind::Call,
            "transaction {position} is not a call"
        );
        ensure!(tx.input.is_empty(), "transaction {position} has calldata");
        ensure!(
            tx.receiver.is_some(),
            "transaction {position} has no receiver"
        );

        let view = ReplayView {
            base: &prior,
            accounts: &overlay,
            recorder: &recorder,
        };
        let receiver = tx.receiver.expect("checked above");
        ensure!(
            !natives.is_native_address(block.period, receiver),
            "transaction {position} targets a native address"
        );
        let receiver_record = view.account(receiver)?;
        ensure!(
            !matches!(&receiver_record, ConcreteRead::Present(record) if record.account.code_size != 0),
            "transaction {position} receiver has code"
        );

        let mut journal = ExecutionJournal::new(view);
        let result = execute_top_level_call(
            &mut journal,
            &hashes,
            &natives,
            &block,
            &tx,
            EnvelopeRules { cornus: true },
            TaraxaProfile::new(true),
        )?;
        let TransactionExecutionResult::Executed(executed) = result else {
            bail!("transaction {position} failed envelope admission: {result:?}")
        };
        ensure!(
            executed.status == CodeExecutionStatus::Success,
            "transaction {position} failed code execution"
        );
        let settled = journal.settle_transaction()?;
        ensure_transfer_writes(position, &settled.writes)?;
        apply_account_writes(&mut overlay, &settled.writes, &prior, &recorder)?;
        cumulative = cumulative
            .checked_add(executed.gas_used.as_u64())
            .context("cumulative gas overflow")?;
        let produced = encode_receipt(
            1,
            executed.gas_used.as_u64(),
            cumulative,
            &executed.logs,
            executed.attempted_contract_address,
        );
        let expected = receipts.at(position)?.as_raw().to_vec();
        outcomes.push(Outcome {
            position,
            hash_hex: hex::encode(tx.hash),
            status: 1,
            gas_used: executed.gas_used.as_u64(),
            cumulative_gas_used: cumulative,
            logs: executed.logs.len(),
            output_hex: hex::encode(&executed.output),
            expected_receipt_sha256: sha256_hex(&expected),
            produced_receipt_sha256: sha256_hex(&produced),
            exact_receipt_match: produced == expected,
            account_writes: settled.writes.accounts.len(),
        });
    }

    let all_receipts_match = outcomes.iter().all(|outcome| outcome.exact_receipt_match);
    let recorder = recorder.into_inner();
    let dependencies = Dependencies {
        accounts: recorder.accounts.into_values().collect(),
        code: recorder.code.into_values().collect(),
        slots: recorder.slots.into_values().collect(),
    };
    let block_hash_reads = block_hash_reads.into_inner();
    let native_checks = natives.checks.into_inner();
    let classification = EnvelopeClassification {
        expected_count: 19,
        exact_count: envelopes.len(),
        every_signature_decoded: true,
        every_chain_id_841: envelopes.iter().all(|entry| entry.chain_id == CHAIN_ID),
        every_call: true,
        every_input_empty: true,
        every_receiver_non_native: true,
        every_receiver_has_no_code: true,
        entries: envelopes,
    };
    let report = Report {
        schema: 1,
        tool_source_sha256: sha256_hex(include_bytes!("replay_preflight.rs")),
        input_copy: input.display().to_string(),
        open_mode: "RocksDB read-only repositories and ConcreteStateReader::open_historical_read_only",
        period: TARGET_PERIOD,
        prior_period,
        prior_root_hex: hex::encode(prior_identity.state_root),
        execution_context: ContextReport {
            pbft_author_hex: hex::encode(pbft.author),
            pbft_timestamp: pbft.timestamp,
            chain_id: CHAIN_ID,
            block_gas_limit: CORNUS_PBFT_GAS_LIMIT,
            envelope_cornus: true,
            profile_cacti: true,
            block_context_observed_by_bytecode: !block_hash_reads.is_empty(),
            block_hash_reads,
            native_address_checks: native_checks.into_iter().map(hex::encode).collect(),
        },
        envelope_classification: classification,
        prior_state_dependencies: dependencies,
        outcomes,
        receipt_row_sha256: sha256_hex(&receipts_rlp),
        receipt_row_exact_match: all_receipts_match,
        total_gas_used: cumulative,
        header_gas_used: current_header.gas_used.as_u64(),
        qualification: Qualification {
            exact_ordered_ordinary_execution: true,
            demanded_prior_state_reads_authenticated: true,
            exact_receipts_reproduced: all_receipts_match,
            touched_read_closure_qualified: all_receipts_match,
            reward_transition_qualified: false,
            concrete_writer_bootstrap_qualified: false,
            final_state_root_reproduced: false,
            publication_authorized: false,
        },
        open_dependencies: vec![
            "Reward transition still needs exact producer-compatible rewards inputs and prior weighted vote/certificate facts.",
            "ConcreteStateWriter opens only the committed current descriptor and prepares current+1; replay from this historical prior root needs an authorized disposable adoption or historical-preparation contract.",
            "The legacy snapshot is markerless for Rust lifecycle/provenance and already has a nonzero head; current lifecycle pairing accepts first application pairing only at head zero, so existing-head bootstrap remains unsupported.",
            "Current multi-validator Aspen2 native-session behavior remains unsupported and is not exercised by this transfer-only period.",
            "The memory overlay proves transaction sequencing and receipts but does not update storage roots, derive the final trie root, write a descriptor, or authorize publication.",
            "The candidate mainnet configuration bytes are qualified separately, while the exact producer binary remains unverified; these transfers do not execute bytecode that could observe block context.",
        ],
    };
    write_report(&output, &report)
}

fn decode_header(
    repository: &FinalChainRepository<Database>,
    period: u64,
) -> Result<StoredFinalChainBlockHeader> {
    let bytes = repository
        .block_header_raw(period)?
        .with_context(|| format!("missing FinalChain header {period}"))?;
    StoredFinalChainBlockHeader::try_from(StoredBlockHeaderRlp::new(&bytes))
}

fn envelope_entry(
    position: usize,
    tx: &rustaxa_evm::contracts::ExecutionTransaction,
    rlp: &[u8],
) -> Result<EnvelopeEntry> {
    let wire = LegacyTransactionEnvelope::decode(rlp)?;
    Ok(EnvelopeEntry {
        position,
        hash_hex: hex::encode(tx.hash),
        rlp_sha256: sha256_hex(rlp),
        sender_hex: hex::encode(tx.sender),
        receiver_hex: tx.receiver.map(hex::encode),
        nonce_hex: hex::encode(tx.nonce.to_bytes()),
        gas_price_hex: tx.gas_price.value().to_str_radix(16),
        gas_limit: tx.gas_limit.as_u64(),
        chain_id: wire.chain_id,
        value_hex: tx.value.value().to_str_radix(16),
        input_hex: hex::encode(&tx.input),
        kind: match tx.kind {
            ExecutionTransactionKind::Call => "call",
            ExecutionTransactionKind::Create => "create",
            ExecutionTransactionKind::System => "system",
        },
    })
}

fn ensure_transfer_writes(position: usize, writes: &JournalWritePlan) -> Result<()> {
    ensure!(
        writes.ordinary_storage.is_empty(),
        "transaction {position} emitted storage writes"
    );
    ensure!(
        writes.raw_storage.is_empty(),
        "transaction {position} emitted raw writes"
    );
    ensure!(
        writes.code.is_empty(),
        "transaction {position} emitted code writes"
    );
    Ok(())
}

fn apply_account_writes(
    overlay: &mut BTreeMap<[u8; 20], ConcreteRead<ConcreteAccountRecord>>,
    writes: &JournalWritePlan,
    prior: &ConcreteStateReader,
    recorder: &RefCell<Recorder>,
) -> Result<()> {
    for write in &writes.accounts {
        let previous = if let Some(value) = overlay.get(&write.address) {
            value.clone()
        } else {
            let value = ConcreteStateRead::account(prior, write.address)?;
            recorder
                .borrow_mut()
                .accounts
                .entry(write.address)
                .or_insert_with(|| account_dependency(write.address, &value));
            value
        };
        let next = match &write.operation {
            JournalAccountOperation::Delete => ConcreteRead::Tombstone,
            JournalAccountOperation::Upsert {
                nonce,
                balance,
                code_hash,
                code_size,
            } => {
                let (storage_root, physical_rlp) = match previous {
                    ConcreteRead::Present(record) => {
                        (record.account.storage_root, record.physical_rlp)
                    }
                    ConcreteRead::Absent | ConcreteRead::Tombstone => (None, Vec::new()),
                };
                ConcreteRead::Present(ConcreteAccountRecord {
                    account: ConcreteAccount {
                        nonce: nonce.clone(),
                        balance: balance.clone(),
                        storage_root,
                        code_hash: *code_hash,
                        code_size: *code_size,
                    },
                    physical_rlp,
                })
            }
        };
        overlay.insert(write.address, next);
    }
    Ok(())
}

fn encode_receipt(
    status: u8,
    gas_used: u64,
    cumulative: u64,
    logs: &[rustaxa_evm::contracts::ExecutionLog],
    created: Option<[u8; 20]>,
) -> Vec<u8> {
    let mut stream = RlpStream::new_list(5);
    stream.append(&status).append(&gas_used).append(&cumulative);
    stream.begin_list(logs.len());
    for log in logs {
        stream.begin_list(3).append(&log.address.as_slice());
        stream.begin_list(log.topics.len());
        for topic in &log.topics {
            stream.append(&topic.as_slice());
        }
        stream.append(&log.data.as_slice());
    }
    match created {
        Some(address) => stream.append(&address.as_slice()),
        None => stream.append(&0_u8),
    };
    stream.out().to_vec()
}

fn account_dependency(
    address: [u8; 20],
    value: &ConcreteRead<ConcreteAccountRecord>,
) -> AccountDependency {
    match value {
        ConcreteRead::Present(record) => AccountDependency {
            address_hex: hex::encode(address),
            result: "present",
            physical_rlp_sha256: Some(sha256_hex(&record.physical_rlp)),
            physical_rlp_hex: Some(hex::encode(&record.physical_rlp)),
            nonce_hex: Some(hex::encode(record.account.nonce.to_bytes())),
            balance_hex: Some(record.account.balance.value().to_str_radix(16)),
            storage_root_hex: record.account.storage_root.map(hex::encode),
            code_hash_hex: record.account.code_hash.map(hex::encode),
            code_size: Some(record.account.code_size),
        },
        ConcreteRead::Absent | ConcreteRead::Tombstone => AccountDependency {
            address_hex: hex::encode(address),
            result: if matches!(value, ConcreteRead::Absent) {
                "absent"
            } else {
                "tombstone"
            },
            physical_rlp_sha256: None,
            physical_rlp_hex: None,
            nonce_hex: None,
            balance_hex: None,
            storage_root_hex: None,
            code_hash_hex: None,
            code_size: None,
        },
    }
}

fn code_dependency(code_hash: [u8; 32], value: &ConcreteRead<Vec<u8>>) -> CodeDependency {
    let bytes = match value {
        ConcreteRead::Present(bytes) => Some(bytes),
        _ => None,
    };
    CodeDependency {
        code_hash_hex: hex::encode(code_hash),
        result: match value {
            ConcreteRead::Present(_) => "present",
            ConcreteRead::Absent => "absent",
            ConcreteRead::Tombstone => "tombstone",
        },
        bytes: bytes.map_or(0, Vec::len),
        sha256: bytes.map(|bytes| sha256_hex(bytes)),
    }
}

fn slot_dependency(
    address: [u8; 20],
    key: ConcreteStorageKey,
    value: &ConcreteRead<Vec<u8>>,
) -> SlotDependency {
    let bytes = match value {
        ConcreteRead::Present(bytes) => Some(bytes),
        _ => None,
    };
    SlotDependency {
        address_hex: hex::encode(address),
        key_hex: hex::encode(key.0),
        result: match value {
            ConcreteRead::Present(_) => "present",
            ConcreteRead::Absent => "absent",
            ConcreteRead::Tombstone => "tombstone",
        },
        value_hex: bytes.map(hex::encode),
        sha256: bytes.map(|bytes| sha256_hex(bytes)),
    }
}

fn open_application_read_only(path: &Path) -> Result<Database> {
    let mut options = Options::default();
    options.create_if_missing(false);
    options.create_missing_column_families(false);
    options.set_max_open_files(128);
    let columns = Database::list_cf(&options, path)?;
    for required in REQUIRED_COLUMNS {
        ensure!(
            columns.iter().any(|column| column == required),
            "missing column family {required}"
        );
    }
    let descriptors = columns.iter().map(|name| {
        Column::from_name(name).map_or_else(
            |_| ColumnFamilyDescriptor::new(name, Options::default()),
            |column| column.descriptor(&Options::default()),
        )
    });
    Ok(Database::open_cf_descriptors_read_only(
        &options,
        path,
        descriptors,
        false,
    )?)
}

fn validated_paths() -> Result<(PathBuf, PathBuf)> {
    let mut args = env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: replay_preflight SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    let output = PathBuf::from(
        args.next()
            .context("usage: replay_preflight SNAPSHOT_COPY OUTPUT_JSON")?,
    );
    ensure!(
        args.next().is_none(),
        "usage: replay_preflight SNAPSHOT_COPY OUTPUT_JSON"
    );
    let supplied = fs::canonicalize(SUPPLIED_EVIDENCE)?;
    let input = fs::canonicalize(input)?;
    ensure!(
        input != supplied && !input.starts_with(&supplied),
        "refusing supplied evidence snapshot"
    );
    ensure!(!output.exists(), "output already exists");
    let name = output.file_name().context("output path has no file name")?;
    let parent = fs::canonicalize(output.parent().unwrap_or_else(|| Path::new(".")))?;
    let output = parent.join(name);
    ensure!(
        !output.starts_with(&supplied) && !output.starts_with(&input),
        "output must be outside snapshot paths"
    );
    Ok((input, output))
}

fn canonical_child(input: &Path, relative: &str) -> Result<PathBuf> {
    let supplied = fs::canonicalize(SUPPLIED_EVIDENCE)?;
    let child = fs::canonicalize(input.join(relative))?;
    ensure!(
        child.starts_with(input) && !child.starts_with(supplied),
        "snapshot child escapes copy"
    );
    Ok(child)
}

fn write_report(path: &Path, report: &Report) -> Result<()> {
    let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
    serde_json::to_writer_pretty(&mut output, report)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    Ok(())
}

fn exact_le_u64(bytes: &[u8], label: &str) -> Result<u64> {
    Ok(u64::from_le_bytes(
        bytes
            .try_into()
            .with_context(|| format!("{label} is not eight bytes"))?,
    ))
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_transfer_receipt_has_reference_shape() {
        assert_eq!(
            hex::encode(encode_receipt(1, 21_000, 42_000, &[], None)),
            "c90182520882a410c080"
        );
    }

    #[test]
    fn native_classifier_is_exact_and_period_independent() {
        let classifier = MainnetNativeClassifier::default();
        assert!(classifier.is_native_address(
            1_u64.into(),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 9]
        ));
        assert!(!classifier.is_native_address(1_u64.into(), [0x11; 20]));
    }
}
