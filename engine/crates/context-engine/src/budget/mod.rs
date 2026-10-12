//! Per-reviewer budgets and greedy selection (CTX-006).

pub mod resolve;
pub mod select;
pub mod tokens;

pub use resolve::{resolve, BudgetError, BudgetLimits, HARD_MAX_TOKENS, RESERVE_TOKENS};
pub use select::{
    fill_remaining, select, semantic_fill_needed, semantic_item_cap, Candidate, Selection,
    SEMANTIC_MIN_REMAINING_TOKENS,
};
pub use tokens::estimate_tokens;
