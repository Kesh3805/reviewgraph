//! Schema versioning for the in-memory graph (CG-004) and its wire form (CG-011).
//!
//! One constant, one owner: `codegraph` decides when the persisted shape changes, and
//! `graph-storage`, `incremental` and every analyzer read it from here instead of keeping a
//! second copy that can drift. `docs/graph-schema/versioning.md` records the rules.

/// Version of the in-memory graph layout and of the encoded `Graph`/`Delta` payloads.
///
/// Bumped whenever a node kind, an edge kind, a struct layout or a codec field changes.
/// A reader must refuse a payload whose `schema_version` differs from its own
/// ([`crate::graph::GraphBuildError::SchemaVersionMismatch`], and the codec header check
/// in [`crate::codec`]).
pub const SCHEMA_VERSION: u32 = 1;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn schema_version_starts_at_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }
}
