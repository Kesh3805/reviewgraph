//! Review runs and reviewer runs (DOM-008, PRD §108-109).
//!
//! [`state`] holds the authoritative run lifecycle table; [`run`] and [`reviewer_run`] hold the
//! entities. These types are *validators*: persisted transitions are compare-and-set in SQL
//! (PIPE-007).

pub mod reviewer_run;
pub mod run;
pub mod state;

pub use reviewer_run::{ReviewerRun, ReviewerRunState, TokenUsage};
pub use run::{ReviewRun, ReviewRunSpec, ReviewTrigger, RunFailure, TransitionInput};
pub use state::ReviewState;
