#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! REV-S-001: security prompt v1 and the `security.v1` output schema.

mod common;

use model_gateway::{check_strict_compatible, SchemaValidators};
use reviewers::prompts::{sha_of, PROMPTS};
use reviewers::security::{
    security_input, security_output_schema, security_prompt, SecurityCandidateExtension,
    SecurityCategory,
};
use serde_json::{json, Value};

use common::{auth_bypass_context, golden_output};

fn security_finding(category: &str, entry_points: Value) -> Value {
    let mut f = golden_output()["findings"][0].clone();
    f["category"] = category.into();
    f["entry_points"] = entry_points;
    f["trust_boundary"] = json!({"source": "http_body", "sink_symbol_key": "N1"});
    f["missing_control"] = "ownership_check".into();
    json!({"no_findings_reason": null, "findings": [f]})
}

#[test]
fn security_schema_is_valid_json_schema() {
    let schema = security_output_schema().unwrap();
    SchemaValidators::new().compile(&schema).expect("compiles");
    check_strict_compatible(&schema.schema).expect("strict compatible");
    let ok = security_finding(
        "authz_bypass",
        json!([{"endpoint_node_id": "N2", "path_symbol_keys": ["N2", "N1", "S1"]}]),
    );
    let errors = SchemaValidators::new().validate(&schema, &ok).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let ext = SecurityCandidateExtension::from_raw(&ok["findings"][0]).expect("extension");
    assert_eq!(ext.category, SecurityCategory::AuthzBypass);
}

#[test]
fn security_prompt_version_is_stable_hash() {
    let a = security_prompt().unwrap();
    let b = security_prompt().unwrap();
    assert_eq!(a.reference, b.reference);
    let file = PROMPTS.iter().find(|p| p.kind == "security").unwrap();
    assert_eq!(a.reference.sha, sha_of(file.text));
    assert_eq!(
        a.reference.prompt_version(),
        format!("security:v1:{}", &a.reference.sha[..8])
    );
}

#[test]
fn security_category_roundtrip_snake_case() {
    for c in SecurityCategory::ALL {
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(s, format!("\"{}\"", c.as_str()));
        assert_eq!(serde_json::from_str::<SecurityCategory>(&s).unwrap(), c);
        assert_eq!(SecurityCategory::parse(c.as_str()), Some(c));
    }
    assert_eq!(
        serde_json::to_string(&SecurityCategory::Ssrf).unwrap(),
        "\"ssrf\""
    );
    // Every category is in the schema enum.
    let schema: Value = serde_json::from_str(reviewers::security::SECURITY_SCHEMA_JSON).unwrap();
    let enum_values =
        schema["properties"]["findings"]["items"]["properties"]["category"]["enum"].clone();
    assert_eq!(enum_values.as_array().unwrap().len(), 12);
}

#[test]
fn security_schema_rejects_missing_entry_point_for_authz_bypass() {
    let schema = security_output_schema().unwrap();
    let v = SchemaValidators::new();
    let missing = security_finding("authz_bypass", json!([]));
    assert!(!v.validate(&schema, &missing).unwrap().is_empty());
    // Other categories may omit entry points.
    let other = security_finding("data_exposure", json!([]));
    assert!(v.validate(&schema, &other).unwrap().is_empty());
}

#[test]
fn security_prompt_render_auth_bypass() {
    let p = security_prompt().unwrap();
    let system_text = p.render(&[]);
    insta::assert_snapshot!(system_text.trim_end());
    // The auth-bypass input carries the untrusted `security_patterns` section last.
    let input = security_input(&auth_bypass_context(), &[]);
    let last = input.sections.last().unwrap();
    assert_eq!(last.name, "security_patterns");
    assert_eq!(last.content["untrusted_data"], true);
    for f in PROMPTS {
        assert!(!f.text.to_lowercase().contains("reference"));
    }
}
