//! Review budget manager (PIPE-006, PRD §90). Exhaustion is degradation, not failure.

mod limits;
mod manager;
pub mod persist;

pub use limits::{BudgetKind, ReviewBudget};
pub use manager::{BudgetEvent, BudgetExceeded, BudgetManager, BudgetUsage, Reservation};
