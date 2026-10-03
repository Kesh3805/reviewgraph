#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! JSON Schemas of the request/response shapes live in `packages/contracts/model-gateway/` for
//! a future gRPC move (ADR-009). This test checks them; `UPDATE_SCHEMAS=1` rewrites them.

use std::path::PathBuf;

use model_gateway::{ModelRequest, ModelResponse, RouteDecision, Usage};
use schemars::JsonSchema;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../packages/contracts/model-gateway")
}

fn render<T: JsonSchema>() -> String {
    let schema = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let value = serde_json::to_value(schema).expect("schema serialises");
    let mut text = serde_json::to_string_pretty(&value).expect("render");
    text.push('\n');
    text
}

fn check<T: JsonSchema>(name: &str) {
    let path = dir().join(format!("{name}.schema.json"));
    let want = render::<T>();
    if std::env::var("UPDATE_SCHEMAS").is_ok() {
        std::fs::create_dir_all(dir()).expect("mkdir");
        std::fs::write(&path, &want).expect("write schema");
        return;
    }
    let have = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("missing {}; run with UPDATE_SCHEMAS=1", path.display()));
    assert_eq!(
        have, want,
        "{name} schema drifted; run with UPDATE_SCHEMAS=1"
    );
}

#[test]
fn model_gateway_schemas_match_checked_in_files() {
    check::<ModelRequest>("ModelRequest");
    check::<ModelResponse>("ModelResponse");
    check::<RouteDecision>("RouteDecision");
    check::<Usage>("Usage");
}
