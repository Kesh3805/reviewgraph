//! Shared `schemars` helpers for the wire enums of this crate.
//!
//! The graph taxonomy serializes as the PRD spelling (`"APIEndpoint"`, `"CALLS"`, …) rather
//! than the Rust variant spelling, so the derived `JsonSchema` is assembled from the same
//! `as_str` table that [`serde`](serde) uses. One table, one source of truth.

use schemars::schema::{Metadata, Schema, SchemaObject};

/// Builds a closed string-enum schema whose values are exactly `values`, in order.
pub(crate) fn string_enum_schema(name: &str, values: &[&str]) -> Schema {
    let mut object = SchemaObject {
        instance_type: Some(schemars::schema::InstanceType::String.into()),
        ..SchemaObject::default()
    };
    object.metadata = Some(Box::new(Metadata {
        title: Some(name.to_owned()),
        ..Metadata::default()
    }));
    object.enum_values = Some(
        values
            .iter()
            .map(|value| serde_json::Value::String((*value).to_owned()))
            .collect(),
    );
    Schema::Object(object)
}
