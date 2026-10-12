//! Correctness prompt v1 and its binding to `reviewer_output.v1` (REV-C-001).

use std::sync::Arc;

use model_gateway::SystemPrompt;

use crate::error::ReviewerError;
use crate::focus::FocusProfile;
use crate::prompts::{prompt, Prompt};

pub const PROMPT_KIND: &str = "correctness";
pub const PROMPT_VERSION: u32 = 1;

/// The correctness prompt, v1.
pub fn correctness_prompt() -> Result<Prompt, ReviewerError> {
    prompt(PROMPT_KIND, PROMPT_VERSION)
}

/// The system prompt for a set of active focus profiles. The text is static per focus
/// combination (at most 2^3 variants), so it stays cacheable.
pub fn system_prompt(p: &Prompt, focus: &[FocusProfile]) -> SystemPrompt {
    SystemPrompt {
        prompt_id: p.reference.kind.clone(),
        prompt_version: p.reference.prompt_version(),
        prompt_sha: p.reference.sha.clone(),
        text: Arc::from(p.render(focus)),
    }
}
