//! Source-contract diagnostics shared by native and disposable API fault tests.
use super::support::decode;
use serde_json::Value;
fn keccak(bytes: impl AsRef<[u8]>) -> [u8; 32] {
    super::lifecycle_keccak(bytes.as_ref())
}
/// Exact infrastructure diagnostics for the reviewed lifecycle serializer calls.
/// The independently measured Go write order binds each source-contract label
/// to its consumed key; unknown operations, shapes or keys are rejected.
pub fn malformed_raw_error(row: &Value, key: &[u8; 32]) -> String {
    let writes = row["ordered_raw_writes"].as_array().unwrap();
    let input = decode(row["input"].as_str().unwrap());
    let mut labels = Vec::new();
    let confirm_v1 = keccak("confirmUndelegate(address)");
    let confirm_v2 = keccak("confirmUndelegateV2(address,uint64)");
    let commission = keccak("claimCommissionRewards(address)");
    if input[..4] == confirm_v1[..4] || input[..4] == confirm_v2[..4] {
        let v2 = input[..4] == confirm_v2[..4];
        labels.push(if v2 {
            "confirmed V2 undelegation"
        } else {
            "confirmed V1 undelegation"
        });
        for _ in 0..if v2 { 2 } else { 1 } {
            labels.extend([
                "iterable-map last item",
                "iterable-map last position",
                "iterable-map remove count",
            ]);
        }
        if writes[labels.len()]["value"] != "" {
            labels.push("validator");
        }
    } else {
        assert_eq!(
            input[..4],
            commission[..4],
            "unknown lifecycle fault operation"
        );
        if writes.len() == 1 {
            labels.push("validator rewards");
        }
    }
    if labels.len() < writes.len() {
        labels.extend([
            "deleted validator",
            "deleted validator info",
            "deleted validator owner",
            "deleted validator VRF key",
            "deleted validator rewards",
        ]);
        if writes[labels.len()]["value"] == "" {
            labels.extend(["iterable-map last item", "iterable-map last position"]);
        } else {
            labels.extend([
                "iterable-map moved position",
                "iterable-map replacement item",
                "iterable-map removed position",
                "iterable-map old last item",
            ]);
        }
        labels.extend([
            "iterable-map remove count",
            "decremented reward-reference node",
        ]);
    }
    assert_eq!(
        labels.len(),
        writes.len(),
        "unqualified lifecycle raw-write shape"
    );
    let index = writes
        .iter()
        .position(|row| decode(row["key"].as_str().unwrap()) == key)
        .expect("fault key must be consumed by a measured Go write");
    format!("{} raw/domain facts disagree", labels[index])
}
