//! Operation-specific historical block selection for the isolated Rust backend.
//!
//! This pure policy preserves the current external StateAPI owner's defaults and
//! future-period branches. Inputs are one caller-supplied coherent observation of
//! FinalChain and concrete heads; the policy neither authenticates that observation
//! nor opens state. Readability checks, historical retention, header availability,
//! leaf errors and RPC parsing remain the composition owner's responsibility.

use rustaxa_types::FinalChainBlockNumber;

/// Historical operation after RPC block parsing, before selecting a state reader.
///
/// `None` preserves each operation's existing default. Trace requires an explicit
/// execution period. Direct DPoS calls use the existing native query boundary;
/// they must not be silently treated as ordinary EVM calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoricalQuery {
    /// Account lookup; defaults to the FinalChain head, then clamps to concrete.
    Account(Option<FinalChainBlockNumber>),
    /// Storage lookup; defaults to concrete head and returns zero above it.
    Storage(Option<FinalChainBlockNumber>),
    /// Code lookup; defaults to concrete head and returns empty bytes above it.
    Code(Option<FinalChainBlockNumber>),
    /// Ordinary call; uses the selected concrete period's header and state.
    OrdinaryCall(Option<FinalChainBlockNumber>),
    /// Direct DPoS query; preserves the requested or default FinalChain period.
    NativeDposCall(Option<FinalChainBlockNumber>),
    /// Trace execution period; the TraceRunner separately selects preceding state.
    Trace(FinalChainBlockNumber),
}

/// Selection result, with no state access or publication authority.
///
/// Reader-bearing variants retain operation identity because leaf error handling
/// differs. `OrdinaryCall` and `Trace` periods select headers; missing headers
/// remain errors. Trace execution at B starts from state max(B-1,0) in the runner,
/// not from a reader at B. Constant results apply only to future storage/code
/// requests, never to missing retained data or failed reads at a selected period.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HistoricalQueryPlan {
    /// Load the account at this concrete period.
    Account(FinalChainBlockNumber),
    /// Load storage at this concrete period.
    Storage(FinalChainBlockNumber),
    /// Load code at this concrete period.
    Code(FinalChainBlockNumber),
    /// Execute against this period's header and concrete state.
    OrdinaryCall(FinalChainBlockNumber),
    /// Invoke the native query client at this unmodified period.
    NativeDposCall(FinalChainBlockNumber),
    /// Trace using this execution header and separately selected prior state.
    Trace(FinalChainBlockNumber),
    /// Future storage request has the existing all-zero word result.
    ZeroStorage,
    /// Future code request has the existing empty-byte result.
    EmptyCode,
}

/// Trace rejects future concrete periods before any header or state lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceAboveConcreteHead {
    /// Requested execution period.
    pub requested: FinalChainBlockNumber,
    /// Concrete head observed by the caller.
    pub concrete_head: FinalChainBlockNumber,
}

impl std::fmt::Display for TraceAboveConcreteHead {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FinalChain::trace is not implemented above EVM head")
    }
}

impl std::error::Error for TraceAboveConcreteHead {}

/// Resolves an operation using existing StateAPI block-selection semantics.
///
/// Both heads may differ, including during native-only history. This function
/// does not assert their ordering or substitute synthetic roots. Explicit zero
/// remains genesis, and u64 boundary values require no arithmetic. All returned
/// read periods still require authenticated history; an unavailable dependency
/// must not be converted into a future-request constant result.
pub fn select_historical_query(
    query: HistoricalQuery,
    final_chain_head: FinalChainBlockNumber,
    concrete_head: FinalChainBlockNumber,
) -> Result<HistoricalQueryPlan, TraceAboveConcreteHead> {
    use HistoricalQuery as Q;
    use HistoricalQueryPlan as P;
    Ok(match query {
        Q::Account(requested) => {
            P::Account(requested.unwrap_or(final_chain_head).min(concrete_head))
        }
        Q::Storage(requested) => {
            let requested = requested.unwrap_or(concrete_head);
            if requested > concrete_head {
                P::ZeroStorage
            } else {
                P::Storage(requested)
            }
        }
        Q::Code(requested) => {
            let requested = requested.unwrap_or(concrete_head);
            if requested > concrete_head {
                P::EmptyCode
            } else {
                P::Code(requested)
            }
        }
        Q::OrdinaryCall(requested) => {
            P::OrdinaryCall(requested.unwrap_or(final_chain_head).min(concrete_head))
        }
        Q::NativeDposCall(requested) => P::NativeDposCall(requested.unwrap_or(final_chain_head)),
        Q::Trace(requested) => {
            if requested > concrete_head {
                return Err(TraceAboveConcreteHead {
                    requested,
                    concrete_head,
                });
            }
            P::Trace(requested)
        }
    })
}
