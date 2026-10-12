//! `review eval fixture new` (REV-C-004 authoring helper): builds the reviewer request of a case
//! exactly as the runner does, prints its request hash and writes a synthetic template fixture
//! (`provider: any`, `model: any`, empty findings) for the developer to fill in.

use std::path::{Path, PathBuf};

use anyhow::bail;
use model_gateway::fixture::{fixture_path, Fixture, FixtureResponse, ANY, FIXTURE_VERSION};
use model_gateway::{
    request_hash, DefaultRedactor, FinishReason, ModelOutput, ModelRequest, PreSendRedactor,
    TaskType, Usage,
};
use reviewers::{CorrectnessReviewer, Reviewer};
use serde_json::{json, Value};

use super::case::LoadedCase;
use super::runner::CaseInputs;

/// The correctness request of a case, redacted like the gateway does before hashing.
pub fn case_request(case: &LoadedCase) -> anyhow::Result<ModelRequest> {
    let Some(inputs) = CaseInputs::of(case) else {
        bail!("{}: no context.json", case.case.id);
    };
    let reviewer = CorrectnessReviewer::new()?;
    let (mut req, _) = reviewer.build_request(&inputs.request(), reviewer.tier(&inputs.risk))?;
    DefaultRedactor::new().redact(&mut req.input);
    Ok(req)
}

/// Where the case's correctness fixture lives, and its request hash.
pub fn expected_fixture(
    fixtures_root: &Path,
    case: &LoadedCase,
) -> anyhow::Result<(String, PathBuf)> {
    let req = case_request(case)?;
    let hash = request_hash(&req).0;
    let path = fixture_path(fixtures_root, TaskType::CorrectnessReview, &hash, ANY, ANY);
    Ok((hash, path))
}

/// A synthetic fixture for `req` with `output`.
pub fn synthetic_fixture(req: &ModelRequest, output: Value) -> Fixture {
    Fixture {
        fixture_version: FIXTURE_VERSION,
        request_hash: request_hash(req).0,
        provider: ANY.into(),
        model: ANY.into(),
        task: req.task.as_str().to_owned(),
        prompt_id: req.input.system.prompt_id.clone(),
        prompt_version: req.input.system.prompt_version.clone(),
        schema_hash: req.output_schema.as_ref().map(|s| s.hash.clone()),
        synthetic: true,
        recorded_at: "2026-10-08T00:00:00Z".into(),
        engine_git_sha: None,
        response: FixtureResponse {
            output: ModelOutput::Json(output),
            usage: Usage::default(),
            finish_reason: FinishReason::Complete,
        },
        latency_ms: 0,
    }
}

/// Writes a template fixture (empty findings) unless one exists. Returns its path.
pub fn write_template(fixtures_root: &Path, case: &LoadedCase) -> anyhow::Result<PathBuf> {
    let req = case_request(case)?;
    let hash = request_hash(&req).0;
    let path = fixture_path(fixtures_root, TaskType::CorrectnessReview, &hash, ANY, ANY);
    if path.exists() {
        return Ok(path);
    }
    let fixture = synthetic_fixture(
        &req,
        json!({"findings": [], "no_findings_reason": "template: fill in the plausible model output"}),
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string_pretty(&fixture)?;
    text.push('\n');
    std::fs::write(&path, text)?;
    Ok(path)
}
