//! Contract creation and frame settlement checks.

use rustaxa_evm::{
    contracts::CodeExecutionError,
    frame::{
        ChildFrameResult, ChildFrameStatus, ChildPreEntryError, CodeDepositResult,
        settle_code_deposit, settle_create_child,
    },
};
use rustaxa_types::FinalChainGas;

#[test]
fn create_child_and_code_deposit_settlement_keep_error_paths_distinct() {
    let address = [0x44; 20];
    let reverted = settle_create_child(
        address,
        ChildFrameResult {
            status: ChildFrameStatus::Revert,
            gas_remaining: FinalChainGas::new(7),
            output: vec![0xaa],
        },
    );
    assert_eq!(reverted.returned_gas, FinalChainGas::new(7));
    assert_eq!(reverted.return_data, vec![0xaa]);
    assert_eq!(reverted.created_address, None);

    let exceptional = settle_create_child(
        address,
        ChildFrameResult {
            status: ChildFrameStatus::Exceptional(CodeExecutionError::InvalidOpcode(0xfe)),
            gas_remaining: FinalChainGas::new(7),
            output: vec![0xbb],
        },
    );
    assert_eq!(exceptional.returned_gas, FinalChainGas::ZERO);
    assert!(exceptional.return_data.is_empty());

    for error in [
        ChildPreEntryError::Depth,
        ChildPreEntryError::InsufficientBalance,
    ] {
        let rejected = settle_create_child(
            address,
            ChildFrameResult {
                status: ChildFrameStatus::PreEntryRejected(error),
                gas_remaining: FinalChainGas::new(7),
                output: vec![0xcc],
            },
        );
        assert_eq!(rejected.returned_gas, FinalChainGas::new(7));
        assert!(rejected.return_data.is_empty());
        assert_eq!(rejected.created_address, None);
    }

    assert_eq!(
        settle_code_deposit(vec![0x60], FinalChainGas::new(199), 24_576, 200),
        CodeDepositResult {
            result: Err(CodeExecutionError::CodeDepositOutOfGas),
            gas_remaining: FinalChainGas::ZERO,
        }
    );
    assert_eq!(
        settle_code_deposit(vec![0x60], FinalChainGas::new(200), 24_576, 200),
        CodeDepositResult {
            result: Ok(vec![0x60]),
            gas_remaining: FinalChainGas::ZERO,
        }
    );
}
