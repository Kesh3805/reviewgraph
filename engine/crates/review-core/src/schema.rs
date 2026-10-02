//! Small helpers for hand-written JSON Schemas.

use schemars::schema::{InstanceType, Metadata, Schema, SchemaObject, StringValidation};

/// A `string` schema restricted by a regular expression.
pub(crate) fn string_pattern(title: &str, pattern: &str) -> Schema {
    SchemaObject {
        instance_type: Some(InstanceType::String.into()),
        metadata: Some(Box::new(Metadata {
            title: Some(title.to_owned()),
            ..Default::default()
        })),
        string: Some(Box::new(StringValidation {
            pattern: Some(pattern.to_owned()),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}
