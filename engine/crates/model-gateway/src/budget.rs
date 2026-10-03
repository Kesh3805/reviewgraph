//! Per-call budget pre-checks (GW-007). Both run before any I/O.
//!
//! The run-level ledger (PIPE-006) derives each call's [`crate::CallBudget`]; this only enforces
//! the numbers the call was given.

use crate::error::{BudgetKind, GatewayError};
use crate::types::{ModelRequest, ProviderId};

/// Worst-case price lookup, implemented by the price table (GW-008).
pub trait WorstCasePricer: Send + Sync {
    /// Upper-bound cost in micro-USD for `input` + `output` tokens on this model, or `None` when
    /// nothing is known about the provider.
    fn worst_case_micros(
        &self,
        provider: &ProviderId,
        model: &str,
        input: u32,
        output: u32,
    ) -> Option<u64>;
}

/// Token budget check: estimated input and requested output against the call budget.
pub fn precheck_tokens(req: &ModelRequest, est_input: u32) -> Result<(), GatewayError> {
    if est_input > req.budget.max_input_tokens {
        return Err(GatewayError::BudgetExceeded {
            kind: BudgetKind::InputTokens,
        });
    }
    if req.max_output_tokens > req.budget.max_output_tokens {
        return Err(GatewayError::BudgetExceeded {
            kind: BudgetKind::OutputTokens,
        });
    }
    Ok(())
}

/// Cost budget check for one candidate: `est_input*p_in + max_output*p_out` against
/// `max_cost_usd_micros`.
pub fn precheck_cost(
    req: &ModelRequest,
    est_input: u32,
    provider: &ProviderId,
    model: &str,
    pricer: Option<&dyn WorstCasePricer>,
) -> Result<(), GatewayError> {
    let (Some(max), Some(pricer)) = (req.budget.max_cost_usd_micros, pricer) else {
        return Ok(());
    };
    match pricer.worst_case_micros(provider, model, est_input, req.max_output_tokens) {
        Some(cost) if cost > max => Err(GatewayError::BudgetExceeded {
            kind: BudgetKind::Cost,
        }),
        _ => Ok(()),
    }
}
