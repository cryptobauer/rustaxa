//! Bounded CREATE and frame-settlement checks against pinned reference vectors.

use num_bigint::BigUint;
use rustaxa_evm::{
    contracts::CodeExecutionError,
    frame::{
        ChildFrameResult, ChildFrameStatus, CodeDepositResult, CreateScheme, create_address,
        settle_code_deposit, settle_create_child,
    },
};
use rustaxa_types::{FinalChainGas, FinalChainNonce};
use serde_json::Value;

const PARENT: [u8; 20] = {
    let mut address = [0_u8; 20];
    address[19] = 0xbb;
    address
};

#[test]
fn create_addresses_match_all_arbitrary_nonce_reference_cases() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .unwrap();
    let rows = fixture["creation_frames"].as_array().unwrap();
    let mut checked = 0;
    for row in rows {
        if row["case"].as_str().unwrap().starts_with("create2-") {
            continue;
        }
        let nonce = number(row["parent_nonce"].as_str().unwrap());
        let address = create_address(
            PARENT,
            &CreateScheme::Create {
                nonce: nonce_type(nonce),
            },
            &[],
        );
        assert_eq!(hex::encode(address), row["child"].as_str().unwrap());
        checked += 1;
    }
    assert_eq!(checked, 12);
}

#[test]
fn create2_addresses_match_success_and_revert_reference_vectors() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/local.json"
    ))
    .unwrap();
    for row in fixture["creation_frames"].as_array().unwrap() {
        let (salt, init_code) = match row["case"].as_str().unwrap() {
            "create2-u256-success" | "create2-second-collision" | "create2-parent-revert" => {
                (1, vec![0])
            }
            "create2-child-revert" => (1, hex::decode("60006000fd").unwrap()),
            _ => continue,
        };
        let mut salt_word = [0_u8; 32];
        salt_word[31] = salt;
        assert_eq!(
            hex::encode(create_address(
                PARENT,
                &CreateScheme::Create2 { salt: salt_word },
                &init_code
            )),
            row["child"].as_str().unwrap()
        );
    }
}

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

fn number(value: &str) -> BigUint {
    let digits = value.strip_prefix("0x").unwrap_or(value);
    BigUint::parse_bytes(digits.as_bytes(), 16).unwrap()
}

fn nonce_type(value: BigUint) -> FinalChainNonce {
    if value == BigUint::default() {
        FinalChainNonce::zero()
    } else {
        FinalChainNonce::from_bytes(&value.to_bytes_be()).unwrap()
    }
}
