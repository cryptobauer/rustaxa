//! Exact account facts and ordered ordinary effects for staged native kernels.
//!
//! FinalChain's legacy native executor stores accounts with bounded `U256`
//! balances. The concrete EVM journal can carry signed, arbitrary-width
//! intermediate balances. This module keeps those domains separate: selected
//! DPoS kernels operate through a narrow account port, the legacy account map
//! implements that port with its existing checked arithmetic, and staged
//! execution records exact `BigInt` replacements without narrowing them.

use super::super::{Account, DPOS_CONTRACT_ADDRESS, FinalChainNonce, empty_account};
use anyhow::{Result, anyhow, bail};
use ethereum_types::U256;
use num_bigint::{BigInt, BigUint, Sign};
use std::collections::{BTreeMap, HashMap};

/// Exact current account facts supplied to one staged native invocation.
///
/// `balance` is signed because the reference permits negative intermediate
/// balances on an execution path even though terminal persistence is unsigned.
/// The nonce remains arbitrary width. Code and storage metadata are deliberately
/// absent: [`FinalChainNativeOrdinaryMutation::Touch`] delegates the full
/// EIP-161 emptiness decision to the authoritative execution journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalChainNativeAccount {
    /// Whether the account currently exists in the ordinary account lane.
    pub exists: bool,
    /// Current arbitrary-width nonce.
    pub nonce: FinalChainNonce,
    /// Current signed, arbitrary-width execution balance.
    pub balance: BigInt,
}

/// One ordered ordinary-account effect produced by a staged native kernel.
///
/// The execution journal validates and applies these operations sequentially.
/// They remain in the enclosing ordinary frame checkpoint and therefore revert
/// with that frame, independently from irreversible native raw mutations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalChainNativeOrdinaryMutation {
    /// Apply `SubBalance(0)`: create a missing account without dirtying an
    /// existing account.
    EnsureExists {
        /// Account whose existence is ensured.
        address: [u8; 20],
        /// Exact existence observed before this operation.
        expected_exists: bool,
    },
    /// Apply `AddBalance(0)`. The journal owns the complete emptiness and RIPEMD
    /// rules because they require code metadata absent from the native context.
    Touch {
        /// Account to touch under the reference rule.
        address: [u8; 20],
        /// Exact existence observed before this operation.
        expected_exists: bool,
    },
    /// Replace one full-width balance after validating the preceding working
    /// account facts.
    BalanceReplace {
        /// Account whose balance changes.
        address: [u8; 20],
        /// Exact existence observed before this operation.
        expected_exists: bool,
        /// Exact signed balance observed before this operation.
        expected: BigInt,
        /// Exact signed balance after this operation.
        replacement: BigInt,
    },
}

/// Minimal account behavior used by the selected DPoS and reward kernels.
///
/// Implementations must make each method observe all preceding operations. A
/// caller performs operation-specific affordability checks before subtraction;
/// this port preserves arithmetic width and account-lifecycle behavior.
pub(in crate::final_chain) trait DposAccountPort {
    /// Returns current working facts, materializing a legacy map account exactly
    /// where the former `entry(...).or_insert_with(empty_account)` read did so.
    fn account(&mut self, address: [u8; 20]) -> Result<FinalChainNativeAccount>;

    /// Applies the reference's `SubBalance` semantics for a non-negative amount.
    fn subtract_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()>;

    /// Applies the reference's `AddBalance` semantics for a non-negative amount.
    fn add_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()>;
}

/// Invocation-local full-width account overlay and ordered effect recorder.
///
/// Construct a new value from authoritative current journal facts for every
/// native invocation. Successful callers consume the mutations; failed replay
/// discards this scratch state. It must never be reused as the next invocation's
/// account source or as publication authority.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the next reviewed staged-dispatch phase")
)]
#[derive(Debug)]
pub(super) struct StagedDposAccountPort {
    accounts: BTreeMap<[u8; 20], FinalChainNativeAccount>,
    mutations: Vec<FinalChainNativeOrdinaryMutation>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "used by the next reviewed staged-dispatch phase")
)]
impl StagedDposAccountPort {
    /// Builds one invocation-local overlay.
    ///
    /// Duplicate addresses are rejected. An absent row must also carry the
    /// canonical zero nonce and balance because absence cannot authorize spendable
    /// or otherwise mutated account state.
    pub(super) fn new(
        accounts: impl IntoIterator<Item = ([u8; 20], FinalChainNativeAccount)>,
    ) -> Result<Self> {
        let mut indexed = BTreeMap::new();
        for (address, account) in accounts {
            if !account.exists && (!account.nonce.is_zero() || account.balance != BigInt::default())
            {
                bail!("absent staged native account has nonzero state: {address:?}");
            }
            if indexed.insert(address, account).is_some() {
                bail!("duplicate staged native account context: {address:?}");
            }
        }
        Ok(Self {
            accounts: indexed,
            mutations: Vec::new(),
        })
    }

    /// Consumes the scratch overlay and returns effects in reference call order.
    pub(super) fn into_mutations(self) -> Vec<FinalChainNativeOrdinaryMutation> {
        self.mutations
    }

    fn current_mut(&mut self, address: [u8; 20]) -> Result<&mut FinalChainNativeAccount> {
        self.accounts
            .get_mut(&address)
            .ok_or_else(|| anyhow!("staged native account context missing: {address:?}"))
    }
}

impl DposAccountPort for StagedDposAccountPort {
    fn account(&mut self, address: [u8; 20]) -> Result<FinalChainNativeAccount> {
        Ok(self.current_mut(address)?.clone())
    }

    fn subtract_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()> {
        let current = self.current_mut(address)?.clone();
        if amount == &BigUint::default() {
            self.mutations
                .push(FinalChainNativeOrdinaryMutation::EnsureExists {
                    address,
                    expected_exists: current.exists,
                });
            self.current_mut(address)?.exists = true;
            return Ok(());
        }
        let replacement = &current.balance - BigInt::from(amount.clone());
        self.mutations
            .push(FinalChainNativeOrdinaryMutation::BalanceReplace {
                address,
                expected_exists: current.exists,
                expected: current.balance,
                replacement: replacement.clone(),
            });
        let account = self.current_mut(address)?;
        account.exists = true;
        account.balance = replacement;
        Ok(())
    }

    fn add_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()> {
        let current = self.current_mut(address)?.clone();
        if amount == &BigUint::default() {
            self.mutations
                .push(FinalChainNativeOrdinaryMutation::Touch {
                    address,
                    expected_exists: current.exists,
                });
            self.current_mut(address)?.exists = true;
            return Ok(());
        }
        let replacement = &current.balance + BigInt::from(amount.clone());
        self.mutations
            .push(FinalChainNativeOrdinaryMutation::BalanceReplace {
                address,
                expected_exists: current.exists,
                expected: current.balance,
                replacement: replacement.clone(),
            });
        let account = self.current_mut(address)?;
        account.exists = true;
        account.balance = replacement;
        Ok(())
    }
}

impl DposAccountPort for HashMap<[u8; 20], Account> {
    fn account(&mut self, address: [u8; 20]) -> Result<FinalChainNativeAccount> {
        let existed = self.contains_key(&address);
        let account = self.entry(address).or_insert_with(empty_account);
        Ok(FinalChainNativeAccount {
            exists: existed,
            nonce: account.nonce.clone(),
            balance: BigInt::from_bytes_be(
                Sign::Plus,
                &super::super::u256_to_big_endian(*account.balance.as_u256()),
            ),
        })
    }

    fn subtract_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()> {
        let amount = bounded_u256(amount, "legacy DPoS account subtraction")?;
        let account = self.entry(address).or_insert_with(empty_account);
        let replacement = account
            .balance
            .as_u256()
            .checked_sub(amount)
            .ok_or_else(|| anyhow!("legacy DPoS account subtraction underflow"))?;
        // The legacy HashMap kernel always replaced after confirm, even for a
        // zero amount, which canonicalizes Fixed32 provenance to Minimal.
        account.balance.replace_after_mutation(replacement);
        Ok(())
    }

    fn add_balance(&mut self, address: [u8; 20], amount: &BigUint) -> Result<()> {
        let amount = bounded_u256(amount, "legacy DPoS account addition")?;
        let account = self.entry(address).or_insert_with(empty_account);
        let replacement = account
            .balance
            .as_u256()
            .checked_add(amount)
            .ok_or_else(|| anyhow!("legacy DPoS account addition overflow"))?;
        // Preserve the old bounded-kernel encoding transition on zero. Staged
        // execution uses the distinct EnsureExists/Touch operations above.
        account.balance.replace_after_mutation(replacement);
        Ok(())
    }
}

/// Transfers contract custody through sequential subtract/add operations.
///
/// The source comparison uses the complete signed balance. Self-transfers retain
/// both operations, with the addition observing the post-subtraction value.
pub(in crate::final_chain) fn transfer_dpos_contract_balance(
    accounts: &mut (impl DposAccountPort + ?Sized),
    recipient: [u8; 20],
    amount: &BigUint,
    insufficient_message: &'static str,
) -> Result<()> {
    let source = accounts.account(DPOS_CONTRACT_ADDRESS)?;
    if source.balance < BigInt::from(amount.clone()) {
        bail!(insufficient_message);
    }
    accounts.subtract_balance(DPOS_CONTRACT_ADDRESS, amount)?;
    accounts.add_balance(recipient, amount)
}

fn bounded_u256(value: &BigUint, operation: &str) -> Result<U256> {
    if value.bits() > 256 {
        bail!("{operation} exceeds U256");
    }
    Ok(U256::from_big_endian(&value.to_bytes_be()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRACT: [u8; 20] = DPOS_CONTRACT_ADDRESS;
    const RECIPIENT: [u8; 20] = [0x44; 20];

    fn fact(exists: bool, nonce: u64, balance: BigInt) -> FinalChainNativeAccount {
        FinalChainNativeAccount {
            exists,
            nonce: FinalChainNonce::from_u64(nonce),
            balance,
        }
    }

    fn legacy_account(balance: u64) -> Account {
        let mut account = empty_account();
        account.balance.replace_after_mutation(U256::from(balance));
        account
    }

    #[test]
    fn staged_transfer_preserves_full_signed_balances_and_order() {
        let wide = (BigInt::from(1_u8) << 320_usize) + BigInt::from(9_u8);
        let amount = BigUint::from(7_u8);
        let mut accounts = StagedDposAccountPort::new([
            (CONTRACT, fact(true, 3, wide.clone())),
            (RECIPIENT, fact(true, 4, BigInt::from(-11))),
        ])
        .unwrap();

        transfer_dpos_contract_balance(&mut accounts, RECIPIENT, &amount, "insufficient").unwrap();

        assert_eq!(
            accounts.into_mutations(),
            vec![
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: CONTRACT,
                    expected_exists: true,
                    expected: wide.clone(),
                    replacement: &wide - BigInt::from(7_u8),
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: RECIPIENT,
                    expected_exists: true,
                    expected: BigInt::from(-11),
                    replacement: BigInt::from(-4),
                },
            ]
        );
    }

    #[test]
    fn staged_transfer_checks_complete_negative_source_before_effects() {
        let mut accounts = StagedDposAccountPort::new([
            (CONTRACT, fact(true, 0, BigInt::from(-1))),
            (RECIPIENT, fact(false, 0, BigInt::default())),
        ])
        .unwrap();

        assert_eq!(
            transfer_dpos_contract_balance(
                &mut accounts,
                RECIPIENT,
                &BigUint::default(),
                "insufficient exact balance",
            )
            .unwrap_err()
            .to_string(),
            "insufficient exact balance"
        );
        assert!(accounts.into_mutations().is_empty());
    }

    #[test]
    fn staged_context_rejects_noncanonical_absence_but_accepts_existing_negative_balance() {
        assert!(
            StagedDposAccountPort::new([(CONTRACT, fact(false, 1, BigInt::default()),)])
                .unwrap_err()
                .to_string()
                .contains("absent staged native account has nonzero state")
        );
        assert!(
            StagedDposAccountPort::new([(CONTRACT, fact(false, 0, BigInt::from(-1)),)])
                .unwrap_err()
                .to_string()
                .contains("absent staged native account has nonzero state")
        );
        assert!(StagedDposAccountPort::new([(CONTRACT, fact(true, 0, BigInt::from(-1)),)]).is_ok());
        assert!(
            StagedDposAccountPort::new([
                (CONTRACT, fact(true, 0, BigInt::default())),
                (CONTRACT, fact(true, 0, BigInt::default())),
            ])
            .unwrap_err()
            .to_string()
            .contains("duplicate staged native account context")
        );
    }

    #[test]
    fn zero_transfer_retains_distinct_ensure_and_touch_operations() {
        let mut accounts = StagedDposAccountPort::new([
            (CONTRACT, fact(false, 0, BigInt::default())),
            (RECIPIENT, fact(true, 0, BigInt::default())),
        ])
        .unwrap();

        transfer_dpos_contract_balance(
            &mut accounts,
            RECIPIENT,
            &BigUint::default(),
            "insufficient",
        )
        .unwrap();

        assert_eq!(
            accounts.into_mutations(),
            vec![
                FinalChainNativeOrdinaryMutation::EnsureExists {
                    address: CONTRACT,
                    expected_exists: false,
                },
                FinalChainNativeOrdinaryMutation::Touch {
                    address: RECIPIENT,
                    expected_exists: true,
                },
            ]
        );
    }

    #[test]
    fn self_transfer_retains_two_sequential_balance_replacements() {
        let mut accounts =
            StagedDposAccountPort::new([(CONTRACT, fact(true, 0, BigInt::from(10_u8)))]).unwrap();

        transfer_dpos_contract_balance(
            &mut accounts,
            CONTRACT,
            &BigUint::from(4_u8),
            "insufficient",
        )
        .unwrap();

        assert_eq!(
            accounts.into_mutations(),
            vec![
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: CONTRACT,
                    expected_exists: true,
                    expected: BigInt::from(10_u8),
                    replacement: BigInt::from(6_u8),
                },
                FinalChainNativeOrdinaryMutation::BalanceReplace {
                    address: CONTRACT,
                    expected_exists: true,
                    expected: BigInt::from(6_u8),
                    replacement: BigInt::from(10_u8),
                },
            ]
        );
    }

    #[test]
    fn legacy_map_and_staged_port_agree_for_bounded_transfer() {
        let mut legacy = HashMap::from([
            (CONTRACT, legacy_account(10)),
            (RECIPIENT, legacy_account(2)),
        ]);
        let mut staged = StagedDposAccountPort::new([
            (CONTRACT, fact(true, 0, BigInt::from(10_u8))),
            (RECIPIENT, fact(true, 0, BigInt::from(2_u8))),
        ])
        .unwrap();

        transfer_dpos_contract_balance(
            &mut legacy,
            RECIPIENT,
            &BigUint::from(3_u8),
            "insufficient",
        )
        .unwrap();
        transfer_dpos_contract_balance(
            &mut staged,
            RECIPIENT,
            &BigUint::from(3_u8),
            "insufficient",
        )
        .unwrap();

        assert_eq!(*legacy[&CONTRACT].balance.as_u256(), U256::from(7_u8));
        assert_eq!(*legacy[&RECIPIENT].balance.as_u256(), U256::from(5_u8));
        assert_eq!(
            staged.account(CONTRACT).unwrap().balance,
            BigInt::from(7_u8)
        );
        assert_eq!(
            staged.account(RECIPIENT).unwrap().balance,
            BigInt::from(5_u8)
        );
    }

    #[test]
    fn legacy_zero_transfer_retains_bounded_kernel_encoding_transition() {
        let mut contract = empty_account();
        contract.balance =
            rustaxa_types::FinalChainAccountBalance::from_cpp_genesis_bytes(&[0_u8; 32]).unwrap();
        let mut recipient = empty_account();
        recipient.balance =
            rustaxa_types::FinalChainAccountBalance::from_cpp_genesis_bytes(&[0_u8; 32]).unwrap();
        let mut accounts = HashMap::from([(CONTRACT, contract), (RECIPIENT, recipient)]);

        transfer_dpos_contract_balance(
            &mut accounts,
            RECIPIENT,
            &BigUint::default(),
            "insufficient",
        )
        .unwrap();

        assert!(accounts[&CONTRACT].balance.to_snapshot_bytes().is_empty());
        assert!(accounts[&RECIPIENT].balance.to_snapshot_bytes().is_empty());
    }
}
