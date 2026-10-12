//! Role classification of graph nodes (PROF-002).
//!
//! Precedence, strongest first: a framework fact (the node kind the framework mapper refined it
//! to, confidence 0.95), then a file-name suffix such as `*.controller.ts` (0.85), then a
//! directory segment such as `controllers/` (0.7). Anything else has no role.

use codegraph::{NodeFlags, NodeKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Confidence of a role taken from a framework fact.
pub const FRAMEWORK_CONFIDENCE: f32 = 0.95;
/// Confidence of a role taken from a file-name suffix.
pub const SUFFIX_CONFIDENCE: f32 = 0.85;
/// Confidence of a role taken from a directory segment.
pub const DIRECTORY_CONFIDENCE: f32 = 0.7;

/// The architectural role of a class or file.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Controller,
    Service,
    Repository,
    Entity,
    Dto,
    Guard,
    Processor,
    Module,
    Config,
    Util,
    Test,
}

impl Role {
    pub const ALL: [Role; 11] = [
        Role::Controller,
        Role::Service,
        Role::Repository,
        Role::Entity,
        Role::Dto,
        Role::Guard,
        Role::Processor,
        Role::Module,
        Role::Config,
        Role::Util,
        Role::Test,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Controller => "controller",
            Role::Service => "service",
            Role::Repository => "repository",
            Role::Entity => "entity",
            Role::Dto => "dto",
            Role::Guard => "guard",
            Role::Processor => "processor",
            Role::Module => "module",
            Role::Config => "config",
            Role::Util => "util",
            Role::Test => "test",
        }
    }
}

/// Where a role came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RoleBasis {
    Framework,
    Suffix,
    Directory,
}

/// A role with its confidence and provenance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoleAssignment {
    pub role: Role,
    pub confidence: f32,
    pub basis: RoleBasis,
}

/// `(file-name tag, role)`: `users.controller.ts` has the tag `controller`.
const SUFFIXES: &[(&str, Role)] = &[
    ("controller", Role::Controller),
    ("service", Role::Service),
    ("repository", Role::Repository),
    ("entity", Role::Entity),
    ("dto", Role::Dto),
    ("guard", Role::Guard),
    ("processor", Role::Processor),
    ("consumer", Role::Processor),
    ("module", Role::Module),
    ("config", Role::Config),
    ("spec", Role::Test),
    ("test", Role::Test),
    ("e2e-spec", Role::Test),
    ("util", Role::Util),
    ("utils", Role::Util),
    ("helper", Role::Util),
];

/// `(directory segment, role)`. `modules/` is deliberately absent: it groups feature modules, it
/// is not a layer.
const DIRECTORIES: &[(&str, Role)] = &[
    ("controllers", Role::Controller),
    ("controller", Role::Controller),
    ("services", Role::Service),
    ("service", Role::Service),
    ("repositories", Role::Repository),
    ("repository", Role::Repository),
    ("entities", Role::Entity),
    ("entity", Role::Entity),
    ("dto", Role::Dto),
    ("dtos", Role::Dto),
    ("guards", Role::Guard),
    ("guard", Role::Guard),
    ("processors", Role::Processor),
    ("consumers", Role::Processor),
    ("config", Role::Config),
    ("utils", Role::Util),
    ("util", Role::Util),
    ("helpers", Role::Util),
    ("test", Role::Test),
    ("tests", Role::Test),
    ("__tests__", Role::Test),
    ("e2e", Role::Test),
];

/// The role a framework-refined node kind implies, if any.
pub fn framework_role(kind: NodeKind, flags: NodeFlags) -> Option<Role> {
    if flags.contains(NodeFlags::TEST) {
        return Some(Role::Test);
    }
    match kind {
        NodeKind::Controller => Some(Role::Controller),
        NodeKind::Middleware => Some(Role::Guard),
        NodeKind::DatabaseEntity => Some(Role::Entity),
        NodeKind::QueueConsumer | NodeKind::JobHandler | NodeKind::Worker => Some(Role::Processor),
        NodeKind::TestSuite | NodeKind::TestCase | NodeKind::Fixture => Some(Role::Test),
        NodeKind::Configuration => Some(Role::Config),
        NodeKind::Module => Some(Role::Module),
        _ => None,
    }
}

/// The `(tag, role)` a file name implies, from the `.tag.` segments before the extension.
pub fn suffix_match(path: &str) -> Option<(&'static str, Role)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut parts: Vec<&str> = name.split('.').collect();
    // Drop the base name and the extension; what remains are the tags.
    if parts.len() < 3 {
        return None;
    }
    parts.remove(0);
    parts.pop();
    // The tag nearest the extension wins (`a.service.spec.ts` is a test).
    parts
        .iter()
        .rev()
        .find_map(|tag| SUFFIXES.iter().find(|(t, _)| t == tag).copied())
}

/// The role a file name implies.
pub fn suffix_role(path: &str) -> Option<Role> {
    suffix_match(path).map(|(_, role)| role)
}

/// The `(segment, role)` of the deepest matching directory segment.
pub fn directory_match(path: &str) -> Option<(&'static str, Role)> {
    let mut segments: Vec<&str> = path.split('/').collect();
    segments.pop();
    segments
        .iter()
        .rev()
        .find_map(|seg| DIRECTORIES.iter().find(|(d, _)| d == seg).copied())
}

/// The role the deepest matching directory segment implies.
pub fn directory_role(path: &str) -> Option<Role> {
    directory_match(path).map(|(_, role)| role)
}

/// Classifies a node by its kind, flags and owning file. `None` when nothing recognizes it.
pub fn classify(kind: NodeKind, flags: NodeFlags, path: Option<&str>) -> Option<RoleAssignment> {
    if let Some(role) = framework_role(kind, flags) {
        return Some(RoleAssignment {
            role,
            confidence: FRAMEWORK_CONFIDENCE,
            basis: RoleBasis::Framework,
        });
    }
    let path = path?;
    if let Some(role) = suffix_role(path) {
        return Some(RoleAssignment {
            role,
            confidence: SUFFIX_CONFIDENCE,
            basis: RoleBasis::Suffix,
        });
    }
    directory_role(path).map(|role| RoleAssignment {
        role,
        confidence: DIRECTORY_CONFIDENCE,
        basis: RoleBasis::Directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_from_nest_decorators() {
        let a = classify(
            NodeKind::Controller,
            NodeFlags::EMPTY,
            Some("src/x/anything.ts"),
        )
        .unwrap();
        assert_eq!((a.role, a.basis), (Role::Controller, RoleBasis::Framework));
        assert_eq!(a.confidence, 0.95);
        let e = classify(NodeKind::DatabaseEntity, NodeFlags::EMPTY, None).unwrap();
        assert_eq!(e.role, Role::Entity);
        let g = classify(NodeKind::Middleware, NodeFlags::EMPTY, None).unwrap();
        assert_eq!(g.role, Role::Guard);
        let t = classify(NodeKind::Class, NodeFlags::TEST, Some("src/a.ts")).unwrap();
        assert_eq!(t.role, Role::Test);
    }

    #[test]
    fn roles_from_suffix_fallback() {
        let a = classify(
            NodeKind::File,
            NodeFlags::EMPTY,
            Some("src/users/users.service.ts"),
        )
        .unwrap();
        assert_eq!(
            (a.role, a.basis, a.confidence),
            (Role::Service, RoleBasis::Suffix, 0.85)
        );
        assert_eq!(suffix_role("src/a/b.repository.ts"), Some(Role::Repository));
        assert_eq!(suffix_role("src/a/b.service.spec.ts"), Some(Role::Test));
        assert_eq!(suffix_role("src/a/b.ts"), None);
        assert_eq!(suffix_role("src/a/service.ts"), None);
    }

    #[test]
    fn roles_from_directory_fallback() {
        let a = classify(
            NodeKind::File,
            NodeFlags::EMPTY,
            Some("src/controllers/users.ts"),
        )
        .unwrap();
        assert_eq!(
            (a.role, a.basis, a.confidence),
            (Role::Controller, RoleBasis::Directory, 0.7)
        );
        assert!(classify(
            NodeKind::File,
            NodeFlags::EMPTY,
            Some("src/modules/users/thing.ts")
        )
        .is_none());
        assert!(classify(NodeKind::File, NodeFlags::EMPTY, None).is_none());
    }

    #[test]
    fn precedence_framework_over_suffix_over_directory() {
        let a = classify(
            NodeKind::Controller,
            NodeFlags::EMPTY,
            Some("src/services/a.service.ts"),
        )
        .unwrap();
        assert_eq!(a.role, Role::Controller);
        let b = classify(
            NodeKind::File,
            NodeFlags::EMPTY,
            Some("src/controllers/a.service.ts"),
        )
        .unwrap();
        assert_eq!(b.role, Role::Service);
    }
}
