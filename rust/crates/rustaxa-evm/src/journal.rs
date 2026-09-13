//! Transaction-local concrete execution state and rollback rules.
//!
//! The journal keeps ordinary account/storage changes, native raw bytes and
//! transient state in distinct lanes because the pinned Taraxa EVM gives them
//! different read, rollback and persistence behavior. It reads immutable state
//! through [`ConcreteStateRead`] and emits a typed write plan; it never writes a
//! database or publishes a FinalChain generation.

use std::collections::BTreeMap;

use num_bigint::{BigInt, BigUint};
use rustaxa_types::{
    FinalChainNonce,
    concrete_state::{
        ConcreteAccountBalance, ConcreteRead, ConcreteReadError, ConcreteStateRead,
        ConcreteStorageKey,
    },
};

use crate::contracts::{
    BalanceConversionError, ExecutionBalance, ExecutionLog, NativeJournalAccount,
    NativeJournalRead, NativeJournalReadError, NativeOrdinaryAccountMutation, NativeRawOperation,
};

/// Unhashed EVM account address.
pub type JournalAddress = [u8; 20];

/// Opaque token for the current top frame checkpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JournalCheckpoint(u64);

/// Failure to read, mutate, settle or flush journal state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalError {
    /// Immutable concrete state could not be read safely.
    State(ConcreteReadError),
    /// A checkpoint was committed/reverted out of stack order.
    CheckpointOrder,
    /// Transaction settlement was requested with open frame checkpoints.
    OpenCheckpoints,
    /// A settled account balance cannot cross the unsigned persistence boundary.
    Balance(BalanceConversionError),
    /// Refund arithmetic exceeded its unsigned domain.
    RefundOverflow,
    /// Native ordinary output was not based on the current journal state.
    NativeExpectation {
        /// Account whose expected state was stale.
        address: JournalAddress,
        /// Field whose expectation did not match.
        field: NativeAccountField,
    },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "execution journal: {self:?}")
    }
}

impl std::error::Error for JournalError {}

impl From<ConcreteReadError> for JournalError {
    fn from(error: ConcreteReadError) -> Self {
        Self::State(error)
    }
}

/// Account field used to classify a stale native-kernel mutation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeAccountField {
    /// Account existence/touch state.
    Existence,
    /// Signed execution balance.
    Balance,
    /// Arbitrary-width nonce.
    Nonce,
}

/// Final ordinary account operation produced by transaction settlement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalAccountOperation {
    /// Persist the settled account fields; storage/code roots remain sink-owned.
    Upsert {
        /// Arbitrary-width account nonce.
        nonce: FinalChainNonce,
        /// Settled non-negative balance.
        balance: ConcreteAccountBalance,
        /// Reachable code hash, if the account has code.
        code_hash: Option<[u8; 32]>,
        /// Exact reference code size.
        code_size: u64,
    },
    /// Remove an empty account created by this transaction.
    Delete,
}

/// One deterministic account write in a transaction write plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalAccountWrite {
    /// Account receiving the operation.
    pub address: JournalAddress,
    /// Upsert or deletion selected by lifecycle settlement.
    pub operation: JournalAccountOperation,
}

/// One ordinary semantic storage write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalStorageWrite {
    /// Account whose storage trie owns the slot.
    pub address: JournalAddress,
    /// Logical unhashed slot key.
    pub key: ConcreteStorageKey,
    /// Arbitrary-width reference value; zero is deleted by the sink.
    pub value: BigUint,
}

/// One native raw write applied after all ordinary storage writes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalRawWrite {
    /// Account whose storage trie owns the slot.
    pub address: JournalAddress,
    /// Logical unhashed slot key.
    pub key: ConcreteStorageKey,
    /// Exact put/delete operation.
    pub operation: NativeRawOperation,
}

/// Immutable code bytes installed by a successful creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JournalCodeWrite {
    /// Hash under which the unversioned code row is stored.
    pub code_hash: [u8; 32],
    /// Exact runtime code bytes.
    pub code: Vec<u8>,
}

/// Ordered persistence inputs emitted by a settled transaction.
///
/// A sink must apply `ordinary_storage` first and `raw_storage` second. This
/// makes raw bytes win when both lanes touch the same logical key. This plan is
/// a single-transaction contract and does not define same-block cache behavior.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct JournalWritePlan {
    /// Deterministic account operations.
    pub accounts: Vec<JournalAccountWrite>,
    /// Deterministic ordinary slot operations.
    pub ordinary_storage: Vec<JournalStorageWrite>,
    /// Deterministic raw slot operations, applied last.
    pub raw_storage: Vec<JournalRawWrite>,
    /// Immutable code rows referenced by account upserts.
    pub code: Vec<JournalCodeWrite>,
}

/// Receipt facts plus persistence inputs returned at a transaction boundary.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettledTransaction {
    /// Logs surviving all frame checkpoints, in emission order.
    pub logs: Vec<ExecutionLog>,
    /// Final transaction refund before the caller applies its gas cap rule.
    pub refund: u64,
    /// Ordinary-then-raw persistence plan.
    pub writes: JournalWritePlan,
}

#[derive(Clone, Debug)]
struct JournalAccount {
    exists: bool,
    created: bool,
    dirty: bool,
    nonce: FinalChainNonce,
    balance: ExecutionBalance,
    code_hash: Option<[u8; 32]>,
    code_size: u64,
    code: Option<Vec<u8>>,
}

impl JournalAccount {
    fn empty(&self) -> bool {
        self.nonce.is_zero() && self.balance.value() == &BigInt::default() && self.code_size == 0
    }
}

#[derive(Clone, Debug)]
struct StorageCell {
    original: BigUint,
    current: BigUint,
    dirty: bool,
}

#[derive(Clone, Debug)]
enum Undo {
    Account {
        address: JournalAddress,
        previous: Option<JournalAccount>,
    },
    Storage {
        address: JournalAddress,
        key: ConcreteStorageKey,
        previous: Option<StorageCell>,
    },
}

#[derive(Clone, Copy, Debug)]
struct CheckpointState {
    id: JournalCheckpoint,
    undo_len: usize,
    logs_len: usize,
    refund: u64,
}

/// Concrete execution journal bound to one immutable reader generation.
pub struct ExecutionJournal<R> {
    reader: R,
    accounts: BTreeMap<JournalAddress, JournalAccount>,
    ordinary: BTreeMap<(JournalAddress, ConcreteStorageKey), StorageCell>,
    raw: BTreeMap<(JournalAddress, ConcreteStorageKey), NativeRawOperation>,
    transient: BTreeMap<(JournalAddress, ConcreteStorageKey), [u8; 32]>,
    logs: Vec<ExecutionLog>,
    refund: u64,
    undo: Vec<Undo>,
    checkpoints: Vec<CheckpointState>,
    next_checkpoint: u64,
}

impl<R: ConcreteStateRead> ExecutionJournal<R> {
    /// Creates an empty transaction journal over one immutable concrete reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            accounts: BTreeMap::new(),
            ordinary: BTreeMap::new(),
            raw: BTreeMap::new(),
            transient: BTreeMap::new(),
            logs: Vec::new(),
            refund: 0,
            undo: Vec::new(),
            checkpoints: Vec::new(),
            next_checkpoint: 0,
        }
    }

    /// Borrows the immutable reader fixed for this journal.
    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// Starts a nested frame checkpoint.
    pub fn checkpoint(&mut self) -> JournalCheckpoint {
        let id = JournalCheckpoint(self.next_checkpoint);
        self.next_checkpoint = self.next_checkpoint.wrapping_add(1);
        self.checkpoints.push(CheckpointState {
            id,
            undo_len: self.undo.len(),
            logs_len: self.logs.len(),
            refund: self.refund,
        });
        id
    }

    /// Commits the current frame while retaining undo records for its parent.
    pub fn commit_checkpoint(&mut self, checkpoint: JournalCheckpoint) -> Result<(), JournalError> {
        self.take_top(checkpoint)?;
        if self.checkpoints.is_empty() {
            self.undo.clear();
        }
        Ok(())
    }

    /// Reverts ordinary account/storage/log/refund changes in the current frame.
    ///
    /// Native raw and transient writes normally survive. Reverting creation of
    /// an absent account also removes that account's raw overlay, matching the
    /// reference account-lifecycle undo callback.
    pub fn revert_checkpoint(&mut self, checkpoint: JournalCheckpoint) -> Result<(), JournalError> {
        let state = self.take_top(checkpoint)?;
        while self.undo.len() > state.undo_len {
            match self.undo.pop().expect("undo length checked") {
                Undo::Account { address, previous } => {
                    let removed_creation = self
                        .accounts
                        .get(&address)
                        .is_some_and(|current| current.exists)
                        && previous.as_ref().is_none_or(|prior| !prior.exists);
                    match previous {
                        Some(account) => {
                            self.accounts.insert(address, account);
                        }
                        None => {
                            self.accounts.remove(&address);
                        }
                    }
                    if removed_creation {
                        self.raw.retain(|(owner, _), _| owner != &address);
                    }
                }
                Undo::Storage {
                    address,
                    key,
                    previous,
                } => match previous {
                    Some(cell) => {
                        self.ordinary.insert((address, key), cell);
                    }
                    None => {
                        self.ordinary.remove(&(address, key));
                    }
                },
            }
        }
        self.logs.truncate(state.logs_len);
        self.refund = state.refund;
        if self.checkpoints.is_empty() {
            self.undo.clear();
        }
        Ok(())
    }

    /// Reads current account existence, nonce and signed balance.
    pub fn account(&self, address: JournalAddress) -> Result<NativeJournalAccount, JournalError> {
        self.current_account(address).map_err(JournalError::from)
    }

    /// Creates/touches an account through the ordinary rollback lane.
    pub fn touch_account(&mut self, address: JournalAddress) -> Result<(), JournalError> {
        let previous = self.load_account(address)?;
        if previous.exists {
            return Ok(());
        }
        self.undo.push(Undo::Account {
            address,
            previous: self.accounts.get(&address).cloned(),
        });
        self.accounts.insert(
            address,
            JournalAccount {
                exists: true,
                created: true,
                dirty: true,
                ..previous
            },
        );
        Ok(())
    }

    /// Replaces a nonce through the ordinary rollback lane.
    pub fn set_nonce(
        &mut self,
        address: JournalAddress,
        nonce: FinalChainNonce,
    ) -> Result<(), JournalError> {
        self.touch_account(address)?;
        let previous = self.accounts.get(&address).cloned();
        self.undo.push(Undo::Account { address, previous });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was touched");
        account.nonce = nonce;
        account.dirty = true;
        Ok(())
    }

    /// Replaces a signed balance through the ordinary rollback lane.
    pub fn set_balance(
        &mut self,
        address: JournalAddress,
        balance: ExecutionBalance,
    ) -> Result<(), JournalError> {
        self.touch_account(address)?;
        let previous = self.accounts.get(&address).cloned();
        self.undo.push(Undo::Account { address, previous });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was touched");
        account.balance = balance;
        account.dirty = true;
        Ok(())
    }

    /// Installs runtime code through the ordinary rollback lane.
    ///
    /// The caller supplies the already verified hash. Code bytes are emitted as
    /// an immutable sink row only if the containing transaction settles.
    pub fn set_code(
        &mut self,
        address: JournalAddress,
        code_hash: [u8; 32],
        code: Vec<u8>,
    ) -> Result<(), JournalError> {
        self.touch_account(address)?;
        let previous = self.accounts.get(&address).cloned();
        self.undo.push(Undo::Account { address, previous });
        let account = self
            .accounts
            .get_mut(&address)
            .expect("account was touched");
        account.code_hash = Some(code_hash);
        account.code_size = code.len() as u64;
        account.code = Some(code);
        account.dirty = true;
        Ok(())
    }

    /// Deletes an account through the ordinary rollback lane.
    pub fn delete_account(&mut self, address: JournalAddress) -> Result<(), JournalError> {
        let current = self.load_account(address)?;
        if !current.exists {
            return Ok(());
        }
        let previous = self.accounts.get(&address).cloned();
        self.undo.push(Undo::Account { address, previous });
        let account = self.accounts.get_mut(&address).expect("account was loaded");
        account.exists = false;
        account.dirty = true;
        Ok(())
    }

    /// Validates and applies ordered native ordinary-account effects.
    ///
    /// Each effect observes all prior effects in the slice. A mismatch aborts
    /// the pending period; callers must not reuse the partially advanced
    /// journal. `Touch` can establish existence before a first nonce change.
    pub fn apply_native_account_mutations(
        &mut self,
        mutations: &[NativeOrdinaryAccountMutation],
    ) -> Result<(), JournalError> {
        for mutation in mutations {
            match mutation {
                NativeOrdinaryAccountMutation::Touch {
                    address,
                    expected_exists,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.touch_account(*address)?;
                }
                NativeOrdinaryAccountMutation::Balance {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.expect_native(
                        *address,
                        account.balance == *expected,
                        NativeAccountField::Balance,
                    )?;
                    self.set_balance(*address, replacement.clone())?;
                }
                NativeOrdinaryAccountMutation::Nonce {
                    address,
                    expected_exists,
                    expected,
                    replacement,
                } => {
                    let account = self.account(*address)?;
                    self.expect_native(
                        *address,
                        account.exists == *expected_exists,
                        NativeAccountField::Existence,
                    )?;
                    self.expect_native(
                        *address,
                        account.nonce == *expected,
                        NativeAccountField::Nonce,
                    )?;
                    self.set_nonce(*address, replacement.clone())?;
                }
            }
        }
        Ok(())
    }

    /// Reads original and current ordinary storage without consulting raw dirty bytes.
    pub fn ordinary_storage(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<(BigUint, BigUint), JournalError> {
        if !self.current_account(address)?.exists {
            return Ok((BigUint::default(), BigUint::default()));
        }
        if let Some(cell) = self.ordinary.get(&(address, key)) {
            return Ok((cell.original.clone(), cell.current.clone()));
        }
        let value = read_storage_integer(self.reader.storage(address, key)?);
        Ok((value.clone(), value))
    }

    /// Replaces current ordinary storage while retaining its first-read original value.
    pub fn set_ordinary_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        value: BigUint,
    ) -> Result<(), JournalError> {
        self.touch_account(address)?;
        let map_key = (address, key);
        let previous = self.ordinary.get(&map_key).cloned();
        let mut cell = match previous.clone() {
            Some(cell) => cell,
            None => {
                let original = read_storage_integer(self.reader.storage(address, key)?);
                StorageCell {
                    original: original.clone(),
                    current: original,
                    dirty: false,
                }
            }
        };
        self.undo.push(Undo::Storage {
            address,
            key,
            previous,
        });
        cell.current = value;
        cell.dirty = true;
        self.ordinary.insert(map_key, cell);
        Ok(())
    }

    /// Reads exact native raw bytes, independently from ordinary dirty storage.
    pub fn raw_storage(
        &self,
        address: JournalAddress,
        key: ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, JournalError> {
        if !self.current_account(address)?.exists {
            return Ok(ConcreteRead::Absent);
        }
        if let Some(operation) = self.raw.get(&(address, key)) {
            return Ok(match operation {
                NativeRawOperation::Put(value) => ConcreteRead::Present(value.as_bytes().to_vec()),
                NativeRawOperation::Delete => ConcreteRead::Present(Vec::new()),
            });
        }
        self.reader
            .storage(address, key)
            .map_err(JournalError::from)
    }

    /// Applies an irreversible native raw operation for an existing account.
    pub fn set_raw_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        operation: NativeRawOperation,
    ) -> Result<(), JournalError> {
        self.touch_account(address)?;
        self.raw.insert((address, key), operation);
        Ok(())
    }

    /// Reads transaction-local transient storage; missing values are zero.
    pub fn transient_storage(&self, address: JournalAddress, key: ConcreteStorageKey) -> [u8; 32] {
        self.transient
            .get(&(address, key))
            .copied()
            .unwrap_or([0_u8; 32])
    }

    /// Writes transaction-local transient storage, which survives frame revert.
    pub fn set_transient_storage(
        &mut self,
        address: JournalAddress,
        key: ConcreteStorageKey,
        value: [u8; 32],
    ) {
        if value == [0_u8; 32] {
            self.transient.remove(&(address, key));
        } else {
            self.transient.insert((address, key), value);
        }
    }

    /// Appends a log through the ordinary frame rollback lane.
    pub fn push_log(&mut self, log: ExecutionLog) {
        self.logs.push(log);
    }

    /// Borrows logs currently visible to the transaction.
    pub fn logs(&self) -> &[ExecutionLog] {
        &self.logs
    }

    /// Returns the current refund counter.
    pub fn refund(&self) -> u64 {
        self.refund
    }

    /// Adds to the refund counter with checked arithmetic.
    pub fn add_refund(&mut self, refund: u64) -> Result<(), JournalError> {
        self.refund = self
            .refund
            .checked_add(refund)
            .ok_or(JournalError::RefundOverflow)?;
        Ok(())
    }

    /// Settles one transaction and resets transaction-local facts.
    ///
    /// Empty accounts created in this transaction are deleted and contribute no
    /// storage writes. Ordinary values are emitted before raw values. Negative
    /// balances fail explicitly at this unsupported persistence boundary.
    pub fn settle_transaction(&mut self) -> Result<SettledTransaction, JournalError> {
        if !self.checkpoints.is_empty() {
            return Err(JournalError::OpenCheckpoints);
        }

        let mut writes = JournalWritePlan::default();
        let mut discarded = Vec::new();
        for (address, account) in &self.accounts {
            if !account.exists || (account.created && account.empty()) {
                discarded.push(*address);
                writes.accounts.push(JournalAccountWrite {
                    address: *address,
                    operation: JournalAccountOperation::Delete,
                });
            } else if account.dirty {
                writes.accounts.push(JournalAccountWrite {
                    address: *address,
                    operation: JournalAccountOperation::Upsert {
                        nonce: account.nonce.clone(),
                        balance: account
                            .balance
                            .try_to_persisted()
                            .map_err(JournalError::Balance)?,
                        code_hash: account.code_hash,
                        code_size: account.code_size,
                    },
                });
                if let (Some(code_hash), Some(code)) = (account.code_hash, account.code.as_ref()) {
                    writes.code.push(JournalCodeWrite {
                        code_hash,
                        code: code.clone(),
                    });
                }
            }
        }

        for ((address, key), cell) in &self.ordinary {
            if cell.dirty && !discarded.contains(address) {
                writes.ordinary_storage.push(JournalStorageWrite {
                    address: *address,
                    key: *key,
                    value: cell.current.clone(),
                });
            }
        }
        for ((address, key), operation) in &self.raw {
            if !discarded.contains(address) {
                writes.raw_storage.push(JournalRawWrite {
                    address: *address,
                    key: *key,
                    operation: operation.clone(),
                });
            }
        }

        let settled = SettledTransaction {
            logs: std::mem::take(&mut self.logs),
            refund: std::mem::take(&mut self.refund),
            writes,
        };
        self.accounts.clear();
        self.ordinary.clear();
        self.raw.clear();
        self.transient.clear();
        self.undo.clear();
        Ok(settled)
    }

    fn take_top(&mut self, checkpoint: JournalCheckpoint) -> Result<CheckpointState, JournalError> {
        if self
            .checkpoints
            .last()
            .is_none_or(|state| state.id != checkpoint)
        {
            return Err(JournalError::CheckpointOrder);
        }
        Ok(self.checkpoints.pop().expect("top checkpoint checked"))
    }

    fn expect_native(
        &self,
        address: JournalAddress,
        matches: bool,
        field: NativeAccountField,
    ) -> Result<(), JournalError> {
        if matches {
            Ok(())
        } else {
            Err(JournalError::NativeExpectation { address, field })
        }
    }

    fn load_account(&mut self, address: JournalAddress) -> Result<JournalAccount, JournalError> {
        if let Some(account) = self.accounts.get(&address) {
            return Ok(account.clone());
        }
        let account = read_account(self.reader.account(address)?);
        self.accounts.insert(address, account.clone());
        Ok(account)
    }

    fn current_account(
        &self,
        address: JournalAddress,
    ) -> Result<NativeJournalAccount, ConcreteReadError> {
        let account = match self.accounts.get(&address) {
            Some(account) => account.clone(),
            None => read_account(self.reader.account(address)?),
        };
        Ok(NativeJournalAccount {
            exists: account.exists,
            nonce: account.nonce,
            balance: account.balance,
        })
    }
}

impl<R: ConcreteStateRead> NativeJournalRead for ExecutionJournal<R> {
    fn account(
        &self,
        address: JournalAddress,
    ) -> Result<NativeJournalAccount, NativeJournalReadError> {
        self.current_account(address)
            .map_err(NativeJournalReadError::State)
    }

    fn raw_storage(
        &self,
        address: JournalAddress,
        key: &ConcreteStorageKey,
    ) -> Result<ConcreteRead<Vec<u8>>, NativeJournalReadError> {
        ExecutionJournal::raw_storage(self, address, *key).map_err(|error| match error {
            JournalError::State(error) => NativeJournalReadError::State(error),
            other => NativeJournalReadError::Invariant(other.to_string()),
        })
    }
}

fn read_account(
    read: ConcreteRead<rustaxa_types::concrete_state::ConcreteAccountRecord>,
) -> JournalAccount {
    match read {
        ConcreteRead::Present(record) => JournalAccount {
            exists: true,
            created: false,
            dirty: false,
            nonce: record.account.nonce,
            balance: ExecutionBalance::from_persisted(&record.account.balance),
            code_hash: record.account.code_hash,
            code_size: record.account.code_size,
            code: None,
        },
        ConcreteRead::Absent | ConcreteRead::Tombstone => JournalAccount {
            exists: false,
            created: false,
            dirty: false,
            nonce: FinalChainNonce::zero(),
            balance: ExecutionBalance::default(),
            code_hash: None,
            code_size: 0,
            code: None,
        },
    }
}

fn read_storage_integer(read: ConcreteRead<Vec<u8>>) -> BigUint {
    match read {
        ConcreteRead::Present(bytes) => BigUint::from_bytes_be(&bytes),
        ConcreteRead::Absent | ConcreteRead::Tombstone => BigUint::default(),
    }
}
