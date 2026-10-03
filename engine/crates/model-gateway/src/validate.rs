//! JSON Schema validation of structured output (GW-005 recorder, GW-009 gateway flow).
//!
//! Error summaries carry only the instance path, the failing keyword and a generic message:
//! outputs can contain customer code, so instance values are never echoed.

use std::sync::Arc;

use dashmap::DashMap;
use jsonschema::Validator;
use serde_json::Value;

use crate::error::{GatewayError, PermanentKind};
use crate::types::{OutputSchema, SchemaErrorSummary};

/// Maximum number of errors returned.
pub const MAX_ERRORS: usize = 20;
const MAX_MESSAGE_CHARS: usize = 200;

/// Compiled schemas, cached by schema hash. Read-mostly and concurrent.
#[derive(Default)]
pub struct SchemaValidators {
    compiled: DashMap<String, Arc<Validator>>,
}

impl std::fmt::Debug for SchemaValidators {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaValidators")
            .field("compiled", &self.compiled.len())
            .finish()
    }
}

impl SchemaValidators {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compiles (or fetches) the validator. A schema that fails to compile is
    /// `Permanent(InvalidRequest)`.
    pub fn compile(&self, schema: &OutputSchema) -> Result<Arc<Validator>, GatewayError> {
        if let Some(v) = self.compiled.get(&schema.hash) {
            return Ok(Arc::clone(v.value()));
        }
        let validator =
            jsonschema::validator_for(&schema.schema).map_err(|e| GatewayError::Permanent {
                kind: PermanentKind::InvalidRequest,
                provider: None,
                detail: crate::classify::sanitize_detail(&format!(
                    "schema `{}` does not compile: {e}",
                    schema.name
                )),
            })?;
        let validator = Arc::new(validator);
        self.compiled
            .insert(schema.hash.clone(), Arc::clone(&validator));
        Ok(validator)
    }

    /// Validates `output`; an empty vector means valid.
    pub fn validate(
        &self,
        schema: &OutputSchema,
        output: &Value,
    ) -> Result<Vec<SchemaErrorSummary>, GatewayError> {
        let validator = self.compile(schema)?;
        Ok(summarize(&validator, output))
    }
}

fn keyword_of(schema_path: &str) -> String {
    schema_path
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("schema")
        .to_owned()
}

/// Collects up to [`MAX_ERRORS`] summaries without instance values.
pub fn summarize(validator: &Validator, output: &Value) -> Vec<SchemaErrorSummary> {
    validator
        .iter_errors(output)
        .take(MAX_ERRORS)
        .map(|e| {
            let keyword = keyword_of(&e.schema_path().to_string());
            let message = match e.kind() {
                jsonschema::error::ValidationErrorKind::AdditionalProperties { unexpected } => {
                    format!("unexpected properties: {}", unexpected.join(", "))
                }
                _ => format!("value violates keyword `{keyword}`"),
            };
            SchemaErrorSummary {
                instance_path: e.instance_path().to_string(),
                keyword,
                message: message.chars().take(MAX_MESSAGE_CHARS).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn schema() -> OutputSchema {
        OutputSchema::new(
            "t",
            "1",
            json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"], "additionalProperties": false}),
        )
    }

    #[test]
    fn valid_and_invalid_outputs() {
        let v = SchemaValidators::new();
        assert!(v.validate(&schema(), &json!({"n": 1})).unwrap().is_empty());
        let errs = v
            .validate(&schema(), &json!({"n": "SECRET-VALUE"}))
            .unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].instance_path, "/n");
        assert_eq!(errs[0].keyword, "type");
        assert!(!errs[0].message.contains("SECRET-VALUE"));
    }

    #[test]
    fn compiled_validators_are_cached() {
        let v = SchemaValidators::new();
        let a = v.compile(&schema()).unwrap();
        let b = v.compile(&schema()).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn uncompilable_schema_is_invalid_request() {
        let bad = OutputSchema::new("bad", "1", json!({"type": 5}));
        let err = SchemaValidators::new().compile(&bad).unwrap_err();
        assert!(matches!(
            err,
            GatewayError::Permanent {
                kind: PermanentKind::InvalidRequest,
                ..
            }
        ));
    }
}
