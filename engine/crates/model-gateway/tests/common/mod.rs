//! Shared builders for gateway tests.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use model_gateway::{
    CallBudget, InputSection, ModelRequest, ModelTier, OutputSchema, StructuredInput, SystemPrompt,
    TaskType, TenantScope,
};
use review_core::ids::{OrganizationId, RepositoryId};
use serde_json::json;

pub fn tenant() -> TenantScope {
    TenantScope {
        organization_id: OrganizationId::new(),
        repository_id: RepositoryId::new(),
    }
}

pub fn input() -> StructuredInput {
    StructuredInput::new(
        SystemPrompt {
            prompt_id: "correctness".into(),
            prompt_version: "v1".into(),
            prompt_sha: "sha-1".into(),
            text: Arc::from("You are a reviewer."),
        },
        vec![
            InputSection::new("rules", json!({"a": 1})).with_cache_breakpoint(),
            InputSection::new("diff", json!({"files": ["a.ts", "b.ts"]})),
        ],
    )
}

pub fn schema() -> OutputSchema {
    OutputSchema::new(
        "result",
        "1",
        json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"], "additionalProperties": false}),
    )
}

pub fn request() -> ModelRequest {
    ModelRequest::new(
        TaskType::CorrectnessReview,
        ModelTier::ReviewReasoner,
        input(),
        tenant(),
        CallBudget::within(Duration::from_secs(60)),
    )
    .with_schema(schema())
}
