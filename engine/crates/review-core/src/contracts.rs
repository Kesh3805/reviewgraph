//! Shapes shared with the TypeScript control plane (ADR-002).
//!
//! Types registered in `review-cli`'s contract registry are exported as JSON Schema and turned
//! into TypeScript by `packages/contracts`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bumped whenever a contract field is removed or renamed. Additive changes keep the version.
pub const CONTRACTS_VERSION: u32 = 1;

/// Describes the set of exported contracts. Seed type that keeps the pipeline testable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaInfo {
    /// Version of the contract set this description belongs to.
    pub contracts_version: u32,
    /// Names of the exported contract types.
    pub types: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_info_rejects_unknown_fields() {
        let bad = r#"{"contracts_version":1,"types":[],"extra":1}"#;
        assert!(serde_json::from_str::<SchemaInfo>(bad).is_err());
    }
}
