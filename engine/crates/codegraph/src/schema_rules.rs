//! The allowed endpoint-kind matrix and the direction conventions for every [`EdgeKind`]
//! (CG-002).
//!
//! The matrix is enforced by the CG-012 validator as a **warning**, never as a build error:
//! a language extension (a new node kind, a new edge produced by an analyzer) must be able to
//! enter the graph without a `codegraph` release, and a warning is what tells the operator the
//! taxonomy disagrees with the conventions.
//!
//! [`convention`] states, in one sentence, which endpoint is which for a kind — the same text
//! `docs/graph-schema/edge-kinds.md` documents and the CG-002 acceptance reads.

use crate::edge_kind::{EdgeKind, EdgeKindSet};
use crate::node_kind::{NodeCategory, NodeKind};

type Pair = (NodeCategory, NodeCategory);

/// Any kind may name a type.
const ANY_TO_TYPE: [Pair; 12] = [
    (NodeCategory::Structural, NodeCategory::Type),
    (NodeCategory::Type, NodeCategory::Type),
    (NodeCategory::Callable, NodeCategory::Type),
    (NodeCategory::Framework, NodeCategory::Type),
    (NodeCategory::Database, NodeCategory::Type),
    (NodeCategory::Messaging, NodeCategory::Type),
    (NodeCategory::Config, NodeCategory::Type),
    (NodeCategory::Test, NodeCategory::Type),
    (NodeCategory::External, NodeCategory::Type),
    (NodeCategory::Build, NodeCategory::Type),
    (NodeCategory::Governance, NodeCategory::Type),
    (NodeCategory::Data, NodeCategory::Type),
];

/// A handler, a test or a coverage run must originate from something that can hold or run
/// code. Data stores, configuration, dependencies and loose values cannot: \Database\ as a
/// source of \HANDLED_BY\ is exactly the kind of taxonomy disagreement CG-012 warns about.
const HANDLED: [Pair; 8] = [
    (NodeCategory::Structural, NodeCategory::Callable),
    (NodeCategory::Type, NodeCategory::Callable),
    (NodeCategory::Callable, NodeCategory::Callable),
    (NodeCategory::Framework, NodeCategory::Callable),
    (NodeCategory::Messaging, NodeCategory::Callable),
    (NodeCategory::Test, NodeCategory::Callable),
    (NodeCategory::Build, NodeCategory::Callable),
    (NodeCategory::Governance, NodeCategory::Callable),
];

/// Any kind may reach a queue.
const ANY_TO_MESSAGING: [Pair; 12] = [
    (NodeCategory::Structural, NodeCategory::Messaging),
    (NodeCategory::Type, NodeCategory::Messaging),
    (NodeCategory::Callable, NodeCategory::Messaging),
    (NodeCategory::Framework, NodeCategory::Messaging),
    (NodeCategory::Database, NodeCategory::Messaging),
    (NodeCategory::Messaging, NodeCategory::Messaging),
    (NodeCategory::Config, NodeCategory::Messaging),
    (NodeCategory::Test, NodeCategory::Messaging),
    (NodeCategory::External, NodeCategory::Messaging),
    (NodeCategory::Build, NodeCategory::Messaging),
    (NodeCategory::Governance, NodeCategory::Messaging),
    (NodeCategory::Data, NodeCategory::Messaging),
];

/// Any kind may read or write configuration.
const ANY_TO_CONFIG: [Pair; 12] = [
    (NodeCategory::Structural, NodeCategory::Config),
    (NodeCategory::Type, NodeCategory::Config),
    (NodeCategory::Callable, NodeCategory::Config),
    (NodeCategory::Framework, NodeCategory::Config),
    (NodeCategory::Database, NodeCategory::Config),
    (NodeCategory::Messaging, NodeCategory::Config),
    (NodeCategory::Config, NodeCategory::Config),
    (NodeCategory::Test, NodeCategory::Config),
    (NodeCategory::External, NodeCategory::Config),
    (NodeCategory::Build, NodeCategory::Config),
    (NodeCategory::Governance, NodeCategory::Config),
    (NodeCategory::Data, NodeCategory::Config),
];

/// Any kind may touch a table.
const ANY_TO_DATABASE: [Pair; 12] = [
    (NodeCategory::Structural, NodeCategory::Database),
    (NodeCategory::Type, NodeCategory::Database),
    (NodeCategory::Callable, NodeCategory::Database),
    (NodeCategory::Framework, NodeCategory::Database),
    (NodeCategory::Database, NodeCategory::Database),
    (NodeCategory::Messaging, NodeCategory::Database),
    (NodeCategory::Config, NodeCategory::Database),
    (NodeCategory::Test, NodeCategory::Database),
    (NodeCategory::External, NodeCategory::Database),
    (NodeCategory::Build, NodeCategory::Database),
    (NodeCategory::Governance, NodeCategory::Database),
    (NodeCategory::Data, NodeCategory::Database),
];

/// Containers may hold their contents: a directory holds files, a file holds top-level
/// symbols, a class holds members, an entity holds columns, a suite holds cases.
const DECLARED: [Pair; 25] = [
    (NodeCategory::Structural, NodeCategory::Structural),
    (NodeCategory::Structural, NodeCategory::Type),
    (NodeCategory::Structural, NodeCategory::Callable),
    (NodeCategory::Structural, NodeCategory::Data),
    (NodeCategory::Structural, NodeCategory::Framework),
    (NodeCategory::Structural, NodeCategory::Database),
    (NodeCategory::Structural, NodeCategory::Messaging),
    (NodeCategory::Structural, NodeCategory::Config),
    (NodeCategory::Structural, NodeCategory::Test),
    (NodeCategory::Structural, NodeCategory::External),
    (NodeCategory::Structural, NodeCategory::Build),
    (NodeCategory::Structural, NodeCategory::Governance),
    (NodeCategory::Type, NodeCategory::Type),
    (NodeCategory::Type, NodeCategory::Callable),
    (NodeCategory::Type, NodeCategory::Data),
    (NodeCategory::Framework, NodeCategory::Type),
    (NodeCategory::Framework, NodeCategory::Callable),
    (NodeCategory::Framework, NodeCategory::Data),
    (NodeCategory::Database, NodeCategory::Database),
    (NodeCategory::Database, NodeCategory::Data),
    (NodeCategory::Messaging, NodeCategory::Callable),
    (NodeCategory::Messaging, NodeCategory::Data),
    (NodeCategory::Test, NodeCategory::Callable),
    (NodeCategory::Test, NodeCategory::Data),
    (NodeCategory::Build, NodeCategory::Callable),
];

/// Nominal types may be implemented or extended only by another nominal-ish kind.
const NOMINAL: [Pair; 7] = [
    (NodeCategory::Type, NodeCategory::Type),
    (NodeCategory::Framework, NodeCategory::Type),
    (NodeCategory::Messaging, NodeCategory::Type),
    (NodeCategory::Database, NodeCategory::Type),
    (NodeCategory::Build, NodeCategory::Type),
    (NodeCategory::Test, NodeCategory::Type),
    (NodeCategory::Governance, NodeCategory::Type),
];

/// Any kind may name a type.
const USES_TYPE: [Pair; 12] = [
    (NodeCategory::Structural, NodeCategory::Type),
    (NodeCategory::Type, NodeCategory::Type),
    (NodeCategory::Callable, NodeCategory::Type),
    (NodeCategory::Data, NodeCategory::Type),
    (NodeCategory::Framework, NodeCategory::Type),
    (NodeCategory::Database, NodeCategory::Type),
    (NodeCategory::Messaging, NodeCategory::Type),
    (NodeCategory::Config, NodeCategory::Type),
    (NodeCategory::Test, NodeCategory::Type),
    (NodeCategory::External, NodeCategory::Type),
    (NodeCategory::Build, NodeCategory::Type),
    (NodeCategory::Governance, NodeCategory::Type),
];

/// Callers: anything whose body can run.
const CALLERS: [Pair; 7] = [
    (NodeCategory::Callable, NodeCategory::Callable),
    (NodeCategory::Framework, NodeCategory::Callable),
    (NodeCategory::Messaging, NodeCategory::Callable),
    (NodeCategory::Test, NodeCategory::Callable),
    (NodeCategory::Build, NodeCategory::Callable),
    (NodeCategory::Structural, NodeCategory::Callable),
    (NodeCategory::Governance, NodeCategory::Callable),
];

/// Value readers and writers.
const VALUE_IO: [Pair; 6] = [
    (NodeCategory::Callable, NodeCategory::Data),
    (NodeCategory::Framework, NodeCategory::Data),
    (NodeCategory::Structural, NodeCategory::Data),
    (NodeCategory::Test, NodeCategory::Data),
    (NodeCategory::Messaging, NodeCategory::Data),
    (NodeCategory::Build, NodeCategory::Data),
];

/// General references: anything naming a value, a type or a table.
const REFERENCES: [Pair; 8] = [
    (NodeCategory::Callable, NodeCategory::Type),
    (NodeCategory::Callable, NodeCategory::Data),
    (NodeCategory::Type, NodeCategory::Type),
    (NodeCategory::Data, NodeCategory::Type),
    (NodeCategory::Structural, NodeCategory::External),
    (NodeCategory::Database, NodeCategory::Database),
    (NodeCategory::Framework, NodeCategory::Database),
    (NodeCategory::Config, NodeCategory::Data),
];

/// The allowed `(source, target)` category pairs of `kind`.
pub fn allowed(kind: EdgeKind) -> &'static [Pair] {
    match kind {
        EdgeKind::Contains => &DECLARED,
        EdgeKind::Declares => &DECLARED,
        EdgeKind::Exports => &DECLARED,
        EdgeKind::Imports => &[
            (NodeCategory::Structural, NodeCategory::Structural),
            (NodeCategory::Structural, NodeCategory::External),
            (NodeCategory::Build, NodeCategory::External),
        ],
        EdgeKind::Calls => &CALLERS,
        EdgeKind::Reads => &VALUE_IO,
        EdgeKind::Writes => &VALUE_IO,
        EdgeKind::Implements => &NOMINAL,
        EdgeKind::Extends => &NOMINAL,
        EdgeKind::Overrides => &[(NodeCategory::Callable, NodeCategory::Callable)],
        EdgeKind::References => &REFERENCES,
        EdgeKind::UsesType => &USES_TYPE,
        EdgeKind::ReturnsType => &ANY_TO_TYPE,
        EdgeKind::AcceptsType => &ANY_TO_TYPE,
        EdgeKind::RoutesTo => &[(NodeCategory::Framework, NodeCategory::Framework)],
        EdgeKind::HandledBy => &HANDLED,
        EdgeKind::Tests => &HANDLED,
        EdgeKind::Covers => &HANDLED,
        EdgeKind::ProducesJob => &ANY_TO_MESSAGING,
        EdgeKind::ConsumesJob => &[
            (NodeCategory::Messaging, NodeCategory::Messaging),
            (NodeCategory::Callable, NodeCategory::Messaging),
            (NodeCategory::Framework, NodeCategory::Messaging),
        ],
        EdgeKind::ReadsConfig => &ANY_TO_CONFIG,
        EdgeKind::WritesConfig => &ANY_TO_CONFIG,
        EdgeKind::ReadsTable => &ANY_TO_DATABASE,
        EdgeKind::WritesTable => &ANY_TO_DATABASE,
        EdgeKind::DependsOn => &[
            (NodeCategory::Structural, NodeCategory::Structural),
            (NodeCategory::Structural, NodeCategory::External),
            (NodeCategory::Build, NodeCategory::External),
            (NodeCategory::Structural, NodeCategory::Build),
        ],
        EdgeKind::Throws => &ANY_TO_TYPE,
        EdgeKind::Catches => &ANY_TO_TYPE,
        EdgeKind::Serializes => &ANY_TO_TYPE,
        EdgeKind::Deserializes => &ANY_TO_TYPE,
        EdgeKind::Validates => &[
            (NodeCategory::Callable, NodeCategory::Data),
            (NodeCategory::Callable, NodeCategory::Type),
            (NodeCategory::Framework, NodeCategory::Data),
            (NodeCategory::Test, NodeCategory::Data),
        ],
        EdgeKind::Authorizes => &[(NodeCategory::Framework, NodeCategory::Framework)],
        EdgeKind::Publishes => &ANY_TO_MESSAGING,
        EdgeKind::Subscribes => &ANY_TO_MESSAGING,
    }
}

/// True when `source` may point at `target` for `kind`.
pub fn is_allowed(kind: EdgeKind, source: NodeCategory, target: NodeCategory) -> bool {
    allowed(kind).contains(&(source, target))
}

/// The direction convention of `kind`, in the wording `docs/graph-schema/edge-kinds.md` uses.
pub fn convention(kind: EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Contains => "parent -> child (Directory -> File/Directory, File -> top-level symbol, Class -> member)",
        EdgeKind::Declares => "File -> symbol declared at top level",
        EdgeKind::Imports => "File -> File, or File -> ExternalDependency",
        EdgeKind::Exports => "File -> exported symbol",
        EdgeKind::Calls => "caller -> callee",
        EdgeKind::Reads => "reader -> value read",
        EdgeKind::Writes => "writer -> value written",
        EdgeKind::Implements => "sub -> super",
        EdgeKind::Extends => "sub -> super",
        EdgeKind::Overrides => "sub method -> super method",
        EdgeKind::References => "referencing symbol -> referenced symbol",
        EdgeKind::UsesType => "user of a type -> type",
        EdgeKind::ReturnsType => "callable -> returned type",
        EdgeKind::AcceptsType => "callable -> accepted type",
        EdgeKind::RoutesTo => "ApiEndpoint -> Controller",
        EdgeKind::HandledBy => "ApiEndpoint -> handler (Method/Function)",
        EdgeKind::Tests => "TestCase -> symbol under test",
        EdgeKind::Covers => "TestCase -> covered symbol",
        EdgeKind::ProducesJob => "producer -> Queue",
        EdgeKind::ConsumesJob => "JobHandler -> Queue",
        EdgeKind::ReadsConfig => "callable -> EnvironmentVariable|Configuration",
        EdgeKind::WritesConfig => "callable -> EnvironmentVariable|Configuration",
        EdgeKind::ReadsTable => "callable -> DatabaseTable",
        EdgeKind::WritesTable => "callable -> DatabaseTable",
        EdgeKind::DependsOn => "File|Package -> Package|ExternalDependency",
        EdgeKind::Throws => "throwing callable -> thrown type",
        EdgeKind::Catches => "catching callable -> caught type",
        EdgeKind::Serializes => "serializing callable -> type serialized",
        EdgeKind::Deserializes => "deserializing callable -> type deserialized",
        EdgeKind::Validates => "validating callable -> validated value/type",
        EdgeKind::Authorizes => "Middleware -> ApiEndpoint",
        EdgeKind::Publishes => "publisher -> Queue",
        EdgeKind::Subscribes => "subscriber -> Queue",
    }
}

/// The minimum node category a kind may originate from, for the quick checks in tests.
pub fn source_categories(kind: EdgeKind) -> Vec<NodeCategory> {
    let mut seen: Vec<NodeCategory> = allowed(kind).iter().map(|(s, _)| *s).collect();
    seen.sort_by_key(|c| *c as u8);
    seen.dedup();
    seen
}

/// The minimum node category a kind may point at.
pub fn target_categories(kind: EdgeKind) -> Vec<NodeCategory> {
    let mut seen: Vec<NodeCategory> = allowed(kind).iter().map(|(_, t)| *t).collect();
    seen.sort_by_key(|c| *c as u8);
    seen.dedup();
    seen
}

/// True when a node of `kind` may be either endpoint of `edge` (used by the validator's
/// orphan and kind-rule warnings before an edge is even inspected).
pub fn endpoint_possible(edge: EdgeKind, node: NodeKind) -> bool {
    let category = node.category();
    allowed(edge)
        .iter()
        .any(|(source, target)| *source == category || *target == category)
}

/// Edge kinds that always have a framework-produced counterpart (CG-006).
pub const FRAMEWORK_EDGE_KINDS: EdgeKindSet = EdgeKindSet::FRAMEWORK;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::node_kind::ALL_NODE_KINDS;

    /// CG-002 acceptance: every stored kind declares at least one allowed endpoint pair and
    /// a direction convention.
    #[test]
    fn schema_rules_cover_every_edge_kind() {
        let mut covered = std::collections::HashSet::new();
        for kind in EdgeKind::ALL {
            let pairs = allowed(kind);
            assert!(!pairs.is_empty(), "{kind} has no allowed endpoint pairs");
            assert!(!convention(kind).is_empty(), "{kind} has no convention");
            covered.insert(kind);
            for (source, target) in pairs {
                assert!(
                    NodeCategory::ALL.contains(source) && NodeCategory::ALL.contains(target),
                    "{kind} pair {source:?}->{target:?} is not a NodeCategory"
                );
            }
        }
        assert_eq!(covered.len(), 33);
        assert_eq!(EdgeKind::ALL.len(), 33);
    }

    #[test]
    fn direction_conventions_follow_the_prd_contract() {
        assert_eq!(
            convention(EdgeKind::Calls),
            "caller -> callee",
            "CALLS caller -> callee"
        );
        assert_eq!(
            convention(EdgeKind::HandledBy),
            "ApiEndpoint -> handler (Method/Function)"
        );
        assert_eq!(convention(EdgeKind::RoutesTo), "ApiEndpoint -> Controller");
        assert_eq!(
            convention(EdgeKind::Authorizes),
            "Middleware -> ApiEndpoint"
        );
        assert_eq!(convention(EdgeKind::ProducesJob), "producer -> Queue");
        assert_eq!(convention(EdgeKind::ConsumesJob), "JobHandler -> Queue");
        assert_eq!(
            convention(EdgeKind::ReadsTable),
            "callable -> DatabaseTable"
        );
        assert_eq!(
            convention(EdgeKind::ReadsConfig),
            "callable -> EnvironmentVariable|Configuration"
        );
        assert_eq!(convention(EdgeKind::Tests), "TestCase -> symbol under test");
        assert_eq!(
            convention(EdgeKind::DependsOn),
            "File|Package -> Package|ExternalDependency"
        );
        assert_eq!(convention(EdgeKind::Extends), "sub -> super");
        assert_eq!(
            convention(EdgeKind::Overrides),
            "sub method -> super method"
        );
        for kind in EdgeKind::ALL {
            assert!(convention(kind).contains("->"), "{kind} has no arrow");
        }
    }

    #[test]
    fn allowed_pairs_reject_wrong_endpoints() {
        assert!(is_allowed(
            EdgeKind::HandledBy,
            NodeCategory::Framework,
            NodeCategory::Callable
        ));
        assert!(!is_allowed(
            EdgeKind::HandledBy,
            NodeCategory::Database,
            NodeCategory::Callable
        ));
        assert!(is_allowed(
            EdgeKind::RoutesTo,
            NodeCategory::Framework,
            NodeCategory::Framework
        ));
        assert!(!is_allowed(
            EdgeKind::RoutesTo,
            NodeCategory::Callable,
            NodeCategory::Framework
        ));
        assert!(is_allowed(
            EdgeKind::Extends,
            NodeCategory::Type,
            NodeCategory::Type
        ));
        assert!(!is_allowed(
            EdgeKind::Extends,
            NodeCategory::Data,
            NodeCategory::Type
        ));
        assert!(is_allowed(
            EdgeKind::Overrides,
            NodeCategory::Callable,
            NodeCategory::Callable
        ));
        assert!(is_allowed(
            EdgeKind::Imports,
            NodeCategory::Structural,
            NodeCategory::External
        ));
        assert!(is_allowed(
            EdgeKind::DependsOn,
            NodeCategory::Structural,
            NodeCategory::External
        ));
        assert!(!is_allowed(
            EdgeKind::DependsOn,
            NodeCategory::Callable,
            NodeCategory::Callable
        ));
        assert!(is_allowed(
            EdgeKind::ReadsTable,
            NodeCategory::Callable,
            NodeCategory::Database
        ));
        assert!(!is_allowed(
            EdgeKind::ReadsTable,
            NodeCategory::Callable,
            NodeCategory::Messaging
        ));
        assert!(is_allowed(
            EdgeKind::Authorizes,
            NodeCategory::Framework,
            NodeCategory::Framework
        ));
        assert!(!is_allowed(
            EdgeKind::Authorizes,
            NodeCategory::Test,
            NodeCategory::Framework
        ));
    }

    #[test]
    fn source_and_target_categories_are_deduplicated_and_sorted() {
        let sources = source_categories(EdgeKind::Contains);
        assert!(sources.contains(&NodeCategory::Structural));
        assert!(sources.contains(&NodeCategory::Type));
        let mut sorted = sources.clone();
        sorted.sort_by_key(|c| *c as u8);
        assert_eq!(sources, sorted);
        assert!(target_categories(EdgeKind::HandledBy).contains(&NodeCategory::Callable));
        assert_eq!(FRAMEWORK_EDGE_KINDS, EdgeKindSet::FRAMEWORK);
    }

    #[test]
    fn endpoint_possible_accepts_plausible_nodes() {
        assert!(endpoint_possible(
            EdgeKind::HandledBy,
            NodeKind::ApiEndpoint
        ));
        assert!(endpoint_possible(EdgeKind::HandledBy, NodeKind::Method));
        assert!(!endpoint_possible(
            EdgeKind::HandledBy,
            NodeKind::DatabaseTable
        ));
        assert!(endpoint_possible(EdgeKind::ReadsTable, NodeKind::Function));
        for kind in EdgeKind::ALL {
            let sources = source_categories(kind);
            assert!(!sources.is_empty(), "{kind} has no source category");
            for node in ALL_NODE_KINDS {
                let _ = endpoint_possible(kind, node);
            }
        }
    }

    #[test]
    fn every_category_is_reachable_from_some_edge() {
        let mut seen = std::collections::HashSet::new();
        for kind in EdgeKind::ALL {
            for category in source_categories(kind) {
                seen.insert(category);
            }
            for category in target_categories(kind) {
                seen.insert(category);
            }
        }
        assert_eq!(
            seen.len(),
            NodeCategory::ALL.len(),
            "every category appears in the matrix"
        );
    }
}
