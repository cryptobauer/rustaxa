//! Compare pure Rust selection with exact C++ methods and recording leaves.
//! The corpus does not establish historical retention, execution or RPC parity.
use rustaxa_evm::query_policy::{HistoricalQuery, HistoricalQueryPlan, select_historical_query};
use serde_json::Value;

#[test]
fn historical_selection_matches_extracted_cpp_methods() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../experiments/evm_feasibility/fixtures/query_policy_reference.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 188);
    for case in cases {
        let requested = case["requested"].as_u64().map(Into::into);
        let query = match case["op"].as_u64().unwrap() {
            0 => HistoricalQuery::Account(requested),
            1 => HistoricalQuery::Storage(requested),
            2 => HistoricalQuery::Code(requested),
            3 => HistoricalQuery::OrdinaryCall(requested),
            4 => HistoricalQuery::NativeDposCall(requested),
            5 => HistoricalQuery::Trace(requested.unwrap()),
            _ => panic!("unknown reference operation"),
        };
        let concrete_head = case["concrete_head"].as_u64().unwrap().into();
        let result = select_historical_query(
            query,
            case["app_head"].as_u64().unwrap().into(),
            concrete_head,
        );
        if let Some(expected) = case["error"].as_str() {
            let error = result.unwrap_err();
            assert_eq!(error.to_string(), expected, "{case}");
            assert_eq!(error.requested, requested.unwrap());
            assert_eq!(error.concrete_head, concrete_head);
            continue;
        }
        let (kind, period) = match result.unwrap() {
            HistoricalQueryPlan::Account(p) => ("account", Some(p)),
            HistoricalQueryPlan::Storage(p) => ("storage", Some(p)),
            HistoricalQueryPlan::Code(p) => ("code", Some(p)),
            HistoricalQueryPlan::OrdinaryCall(p) => ("call", Some(p)),
            HistoricalQueryPlan::NativeDposCall(p) => ("native", Some(p)),
            HistoricalQueryPlan::Trace(p) => ("trace", Some(p)),
            HistoricalQueryPlan::ZeroStorage => ("zero_storage", None),
            HistoricalQueryPlan::EmptyCode => ("empty_code", None),
        };
        assert_eq!(case["kind"], kind, "{case}");
        assert_eq!(
            case["period"].as_u64(),
            period.map(|p| p.as_u64()),
            "{case}"
        );
    }
}
