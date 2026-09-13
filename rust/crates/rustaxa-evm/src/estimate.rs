//! Gas estimation by repeated probes against one immutable execution identity.
//!
//! The RPC layer owns transaction preparation and block selection. This module
//! owns only the bounded search kernel and deliberately receives a fresh probe
//! callback for each candidate gas limit. A caller must execute every probe in
//! an isolated, discarded journal over the same immutable state identity; a
//! probe must never share a mutable journal with another probe.

/// Outcome of one isolated execution probe.
///
/// `gas_used` is the execution result's consumed gas when the candidate limit
/// is sufficient. Consensus failures retain the exact error string supplied by
/// the execution boundary and terminate estimation. A code failure at a
/// midpoint means that the lower bound must be raised.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EstimateProbe {
    /// The candidate gas limit completed successfully.
    Success { gas_used: u64 },
    /// A consensus-level failure that cannot be repaired by increasing gas.
    ConsensusFailure { error: String },
    /// A code or execution failure used to search for a sufficient limit.
    CodeFailure { error: String },
}

/// Failure returned by [`estimate_gas`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EstimateError<E> {
    /// The probe callback failed before producing an execution result.
    Callback(E),
    /// The execution boundary returned an exact consensus failure string.
    ConsensusFailure { error: String },
    /// The execution boundary returned an exact code failure string.
    CodeFailure { error: String },
    /// The successful probe consumed more gas than the supplied upper bound.
    OutOfGas,
    /// A midpoint rounded to the current lower bound after a code failure.
    ///
    /// The legacy C++ loop would probe this same midpoint forever. This is an
    /// explicit refusal for an inconsistent/non-progressing probe outside the
    /// ordinary transaction domain, whose intrinsic gas is at least 21,000.
    NonProgress { low: u64, high: u64 },
}

/// Finds the smallest gas limit accepted within the legacy five-percent band.
///
/// The callback is invoked first with `gas_cap`, then with binary-search
/// midpoints. Each invocation must perform a fresh isolated execution over the
/// same immutable state identity and discard all effects before returning. A
/// successful probe records its `gas_used`; a midpoint code failure raises the
/// known lower bound, while a consensus failure terminates immediately with
/// its exact string. Callback errors are propagated as
/// [`EstimateError::Callback`]. Initial code failures terminate because there
/// is no successful lower-bound probe to search from.
///
/// The initial successful probe must not consume more than `gas_cap`; such a
/// result is [`EstimateError::OutOfGas`]. The search stops when the interval is
/// no wider than five percent of its upper bound, matching the legacy RPC
/// algorithm. For a one-unit interval whose midpoint equals the lower bound,
/// a code failure returns [`EstimateError::NonProgress`]. The legacy C++ code
/// would loop forever in that case; this explicit error is outside the valid
/// ordinary transaction domain and avoids inventing a successful estimate.
pub fn estimate_gas<F, E>(gas_cap: u64, mut probe: F) -> Result<u64, EstimateError<E>>
where
    F: FnMut(u64) -> Result<EstimateProbe, E>,
{
    let initial = probe(gas_cap).map_err(EstimateError::Callback)?;
    let mut low = match initial {
        EstimateProbe::Success { gas_used } => {
            if gas_used > gas_cap {
                return Err(EstimateError::OutOfGas);
            }
            gas_used
        }
        EstimateProbe::ConsensusFailure { error } => {
            return Err(EstimateError::ConsensusFailure { error });
        }
        EstimateProbe::CodeFailure { error } => {
            return Err(EstimateError::CodeFailure { error });
        }
    };
    let mut high = gas_cap;

    while high - low > high / 20 {
        let midpoint = low + (high - low) / 2;
        match probe(midpoint).map_err(EstimateError::Callback)? {
            EstimateProbe::Success { .. } => high = midpoint,
            EstimateProbe::CodeFailure { .. } if midpoint == low => {
                return Err(EstimateError::NonProgress { low, high });
            }
            EstimateProbe::CodeFailure { .. } => low = midpoint,
            EstimateProbe::ConsensusFailure { error } => {
                return Err(EstimateError::ConsensusFailure { error });
            }
        }
    }

    Ok(high)
}
