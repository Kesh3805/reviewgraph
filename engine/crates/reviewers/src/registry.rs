//! The shipped reviewers (REV-S-002, REV-T-001, REV-A-001, REV-P-001, REV-M-001).

use std::sync::Arc;

use crate::architecture::architecture_reviewer;
use crate::correctness::CorrectnessReviewer;
use crate::error::ReviewerError;
use crate::maintainability::maintainability_reviewer;
use crate::performance::performance_reviewer;
use crate::reviewer::Reviewer;
use crate::security::security_reviewer;
use crate::tests_reviewer::tests_reviewer;

/// Every reviewer, built with its locked prompt and schema.
pub fn build_reviewers() -> Result<Vec<Arc<dyn Reviewer>>, ReviewerError> {
    let all: Vec<Arc<dyn Reviewer>> = vec![
        Arc::new(CorrectnessReviewer::new()?),
        Arc::new(security_reviewer()?),
        Arc::new(tests_reviewer()?),
        Arc::new(architecture_reviewer()?),
        Arc::new(performance_reviewer()?),
        Arc::new(maintainability_reviewer()?),
    ];
    Ok(all)
}
