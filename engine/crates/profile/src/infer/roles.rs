//! Role classification of files and symbols (PROF-002).
//!
//! Precedence: a framework fact on the symbol or file (0.95), then the file-name suffix
//! (`*.controller.ts`, 0.85), then a directory segment (`controllers/`, 0.7), else unknown. The
//! role names are generic; no consumer-specific layer name exists here.

use std::collections::{BTreeMap, HashMap};

use codegraph::{GraphQuery, NodeFlags, NodeKey, NodeKind, NodeRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
        Self::Controller,
        Self::Service,
        Self::Repository,
        Self::Entity,
        Self::Dto,
        Self::Guard,
        Self::Processor,
        Self::Module,
        Self::Config,
        Self::Util,
        Self::Test,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Service => "service",
            Self::Repository => "repository",
            Self::Entity => "entity",
            Self::Dto => "dto",
            Self::Guard => "guard",
            Self::Processor => "processor",
            Self::Module => "module",
            Self::Config => "config",
            Self::Util => "util",
            Self::Test => "test",
        }
    }

    /// Resolves a layer name used in config (`controllers`, `repository`, ...) to a role.
    pub fn from_layer_name(name: &str) -> Option<Role> {
        let lowered = name.trim().to_ascii_lowercase();
        let singular = match lowered.as_str() {
            "repositories" => "repository",
            "entities" => "entity",
            "utils" | "utilities" | "helpers" => "util",
            other => other.strip_suffix('s').unwrap_or(other),
        };
        Self::ALL.into_iter().find(|r| r.as_str() == singular)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleSource {
    Framework,
    Suffix,
    Directory,
}

impl RoleSource {
    pub const fn confidence(self) -> f32 {
        match self {
            Self::Framework => 0.95,
            Self::Suffix => 0.85,
            Self::Directory => 0.7,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleAssignment {
    pub role: Role,
    pub source: RoleSource,
    /// The pattern that matched: a suffix glob, a directory glob, or the node kind.
    pub pattern: String,
}

impl RoleAssignment {
    pub fn confidence(&self) -> f32 {
        self.source.confidence()
    }
}

const SUFFIXES: &[(&str, Role)] = &[
    ("controller", Role::Controller),
    ("resolver", Role::Controller),
    ("service", Role::Service),
    ("repository", Role::Repository),
    ("repo", Role::Repository),
    ("entity", Role::Entity),
    ("model", Role::Entity),
    ("dto", Role::Dto),
    ("guard", Role::Guard),
    ("processor", Role::Processor),
    ("consumer", Role::Processor),
    ("worker", Role::Processor),
    ("module", Role::Module),
    ("config", Role::Config),
    ("util", Role::Util),
    ("utils", Role::Util),
    ("helper", Role::Util),
    ("helpers", Role::Util),
];

const DIRECTORIES: &[(&str, Role)] = &[
    ("controllers", Role::Controller),
    ("services", Role::Service),
    ("repositories", Role::Repository),
    ("entities", Role::Entity),
    ("models", Role::Entity),
    ("dto", Role::Dto),
    ("dtos", Role::Dto),
    ("guards", Role::Guard),
    ("processors", Role::Processor),
    ("consumers", Role::Processor),
    ("workers", Role::Processor),
    ("jobs", Role::Processor),
    ("config", Role::Config),
    ("configs", Role::Config),
    ("utils", Role::Util),
    ("util", Role::Util),
    ("helpers", Role::Util),
    ("test", Role::Test),
    ("tests", Role::Test),
    ("__tests__", Role::Test),
];

const TEST_MARKERS: &[&str] = &["spec", "test", "e2e-spec", "e2e", "int-spec"];

/// The file name without its directory.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether the path names a test file by convention (`*.spec.ts`, `*.test.ts`, `__tests__/`).
pub fn is_test_path(path: &str) -> bool {
    let name = file_name(path);
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 3 && parts[1..parts.len() - 1].iter().any(|p| TEST_MARKERS.contains(p)) {
        return true;
    }
    path.split('/').any(|seg| seg == "__tests__")
}

/// Suffix rule, then directory rule.
pub fn role_from_path(path: &str) -> Option<RoleAssignment> {
    if is_test_path(path) {
        return Some(RoleAssignment {
            role: Role::Test,
            source: RoleSource::Suffix,
            pattern: test_pattern(path),
        });
    }
    let name = file_name(path);
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 3 {
        let suffix = parts[parts.len() - 2].to_ascii_lowercase();
        if let Some((_, role)) = SUFFIXES.iter().find(|(s, _)| *s == suffix) {
            let ext = parts[parts.len() - 1];
            return Some(RoleAssignment {
                role: *role,
                source: RoleSource::Suffix,
                pattern: format!("**/*.{suffix}.{ext}"),
            });
        }
    }
    let segments: Vec<&str> = path.split('/').collect();
    let dirs = &segments[..segments.len().saturating_sub(1)];
    for dir in dirs.iter().rev() {
        let lowered = dir.to_ascii_lowercase();
        if let Some((d, role)) = DIRECTORIES.iter().find(|(d, _)| *d == lowered) {
            return Some(RoleAssignment {
                role: *role,
                source: RoleSource::Directory,
                pattern: format!("**/{d}/**"),
            });
        }
    }
    None
}

fn test_pattern(path: &str) -> String {
    let name = file_name(path);
    let parts: Vec<&str> = name.split('.').collect();
    if parts.len() >= 3 {
        let marker = parts[1..parts.len() - 1]
            .iter()
            .rev()
            .find(|p| TEST_MARKERS.contains(p))
            .copied()
            .unwrap_or("spec");
        format!("**/*.{marker}.{}", parts[parts.len() - 1])
    } else {
        "**/__tests__/**".to_owned()
    }
}

/// The framework role of a node, from its refined kind or flags.
pub fn framework_role(node: &NodeRef<'_>) -> Option<Role> {
    match node.kind {
        NodeKind::Controller => Some(Role::Controller),
        NodeKind::Middleware => Some(Role::Guard),
        NodeKind::DatabaseEntity => Some(Role::Entity),
        NodeKind::JobHandler | NodeKind::QueueConsumer | NodeKind::Worker => Some(Role::Processor),
        NodeKind::TestSuite | NodeKind::TestCase | NodeKind::Fixture => Some(Role::Test),
        _ if node.attrs.flags.contains(NodeFlags::TEST) => Some(Role::Test),
        _ => None,
    }
}

/// Priority when a file holds framework nodes of several roles.
fn framework_priority(role: Role) -> u8 {
    match role {
        Role::Controller => 0,
        Role::Processor => 1,
        Role::Guard => 2,
        Role::Entity => 3,
        Role::Module => 4,
        Role::Test => 5,
        _ => 6,
    }
}

/// Roles of every file and of every framework-refined node of a graph.
#[derive(Debug, Clone, Default)]
pub struct RoleIndex {
    files: BTreeMap<String, RoleAssignment>,
    nodes: HashMap<NodeKey, Role>,
}

impl RoleIndex {
    /// Classifies every file that owns a node. `module_files` are files holding a framework
    /// module declaration.
    pub fn build(graph: &dyn GraphQuery, module_files: &[String]) -> Self {
        let mut framework: BTreeMap<String, Role> = BTreeMap::new();
        let mut nodes = HashMap::new();
        let mut paths: Vec<String> = Vec::new();
        graph.for_each_node(&mut |node| {
            let Some(path) = node.file else {
                return;
            };
            paths.push(path.to_owned());
            if let Some(role) = framework_role(&node) {
                nodes.insert(node.key, role);
                let slot = framework.entry(path.to_owned()).or_insert(role);
                if framework_priority(role) < framework_priority(*slot) {
                    *slot = role;
                }
            }
        });
        for file in module_files {
            framework.entry(file.clone()).or_insert(Role::Module);
            paths.push(file.clone());
        }
        paths.sort();
        paths.dedup();
        let mut files = BTreeMap::new();
        for path in paths {
            let by_path = role_from_path(&path);
            let assignment = match framework.get(&path) {
                Some(role) => Some(RoleAssignment {
                    role: *role,
                    source: RoleSource::Framework,
                    pattern: by_path
                        .filter(|a| a.role == *role)
                        .map(|a| a.pattern)
                        .unwrap_or_else(|| dir_glob(&path)),
                }),
                None => by_path,
            };
            if let Some(assignment) = assignment {
                files.insert(path, assignment);
            }
        }
        Self { files, nodes }
    }

    pub fn file_role(&self, path: &str) -> Option<&RoleAssignment> {
        self.files.get(path)
    }

    /// The node's own framework role, else its file's role.
    pub fn node_role(&self, node: &NodeRef<'_>) -> Option<Role> {
        self.nodes
            .get(&node.key)
            .copied()
            .or_else(|| node.file.and_then(|f| self.file_role(f)).map(|a| a.role))
    }

    pub fn role_of_key(&self, graph: &dyn GraphQuery, key: NodeKey) -> Option<Role> {
        graph.node(key).and_then(|n| self.node_role(&n))
    }

    /// Every classified file, in path order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &RoleAssignment)> {
        self.files.iter().map(|(p, a)| (p.as_str(), a))
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

fn dir_glob(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/**"),
        None => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_and_directory_rules() {
        let a = role_from_path("src/users/users.controller.ts").unwrap();
        assert_eq!((a.role, a.source), (Role::Controller, RoleSource::Suffix));
        assert_eq!(a.pattern, "**/*.controller.ts");
        let b = role_from_path("src/repositories/users.ts").unwrap();
        assert_eq!((b.role, b.source), (Role::Repository, RoleSource::Directory));
        assert_eq!(role_from_path("src/users/users.service.spec.ts").unwrap().role, Role::Test);
        assert!(role_from_path("src/main.ts").is_none());
    }

    #[test]
    fn layer_names_resolve_plurals() {
        assert_eq!(Role::from_layer_name("controllers"), Some(Role::Controller));
        assert_eq!(Role::from_layer_name("repositories"), Some(Role::Repository));
        assert_eq!(Role::from_layer_name("entities"), Some(Role::Entity));
        assert_eq!(Role::from_layer_name("dto"), Some(Role::Dto));
        assert_eq!(Role::from_layer_name("web"), None);
    }
}
