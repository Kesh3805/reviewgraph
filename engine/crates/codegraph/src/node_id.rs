//! Deterministic synthetic and structural node IDs (CG-001).
//!
//! Every constructor is a pure function of its inputs, so a full build and an incremental
//! build produce byte-identical IDs — the property the INC-012 oracle depends on. Normalizers
//! are idempotent: `normalize(normalize(x)) == normalize(x)`.
//!
//! [`NodeKey`] is the 128-bit `blake3` prefix of the ID string, the same hash SID-001 uses
//! for `SymbolId` (ADR-005), so symbol nodes and synthetic nodes share one keyspace.

use std::fmt;

use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use review_core::ids::{SymbolId, SymbolKey};
use review_core::location::RepoPath;

/// The storage key of a node: `blake3(NodeId string)[..16]`, 32 lowercase hex characters on
/// the wire (ADR-005).
pub type NodeKey = SymbolKey;

/// Why a synthetic node ID could not be built. Every constructor that can receive invalid
/// input returns this instead of panicking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NodeIdError {
    #[error("name is empty")]
    EmptyName,
    #[error("environment variable name {0:?} does not match [A-Za-z_][A-Za-z0-9_]*")]
    InvalidEnvName(String),
    #[error("HTTP method {0:?} is not a plain alphabetic token")]
    InvalidHttpMethod(String),
    #[error("path {0:?} is outside the repository")]
    PathOutsideRepository(String),
}

/// A canonical node ID string, for example `http:GET /users/{}` or
/// `ts:src/auth/auth.service#AuthService.authorize/method`.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(
    description = "Canonical node id, e.g. `http:GET /users/{}` or `ts:src/a#B/c/function`."
)]
pub struct NodeId(String);

impl NodeId {
    /// Wraps an already canonical ID. Synthetic constructors below are the sanctioned way to
    /// build one; this exists for decoding persisted rows.
    pub fn from_canonical(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The 128-bit storage key: the first 16 bytes of `blake3(id)`.
    pub fn key(&self) -> NodeKey {
        NodeKey::of(&SymbolId::from_canonical_unchecked(&self.0))
    }

    /// The repository singleton, `repo:/`. The repository itself is a namespace column rather
    /// than part of any ID (ADR-005).
    pub fn repository() -> Self {
        Self("repo:/".to_owned())
    }

    /// The repository root directory, `dir:.`.
    pub fn root_directory() -> Self {
        Self("dir:.".to_owned())
    }

    /// `dir:{path}` for a validated repository-relative path.
    pub fn directory(path: &RepoPath) -> Self {
        Self(format!("dir:{}", path.as_str()))
    }

    /// `dir:{path}` for a raw string; anything that is not a valid [`RepoPath`] is rejected
    /// as [`NodeIdError::PathOutsideRepository`].
    pub fn directory_str(path: &str) -> Result<Self, NodeIdError> {
        let path =
            RepoPath::new(path).map_err(|_| NodeIdError::PathOutsideRepository(path.to_owned()))?;
        Ok(Self::directory(&path))
    }

    /// The directory that contains `path`, walking to [`Self::root_directory`] at the top.
    pub fn containing_directory(path: &RepoPath) -> Self {
        match path.as_str().rsplit_once('/') {
            Some((parent, _)) if !parent.is_empty() => Self(format!("dir:{parent}")),
            _ => Self::root_directory(),
        }
    }

    /// Every directory from the root down to the one that contains `path`, outermost first.
    pub fn ancestor_directories(path: &RepoPath) -> Vec<Self> {
        let mut out = vec![Self::root_directory()];
        if let Some((parent, _)) = path.as_str().rsplit_once('/') {
            if !parent.is_empty() {
                let mut prefix = String::new();
                for segment in parent.split('/') {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(segment);
                    out.push(Self(format!("dir:{prefix}")));
                }
            }
        }
        out
    }

    /// `file:{path}` for a validated repository-relative path. The extension is kept.
    pub fn file(path: &RepoPath) -> Self {
        Self(format!("file:{}", path.as_str()))
    }

    /// [`Self::file`] for a raw string, rejecting anything outside the repository.
    pub fn file_str(path: &str) -> Result<Self, NodeIdError> {
        let path =
            RepoPath::new(path).map_err(|_| NodeIdError::PathOutsideRepository(path.to_owned()))?;
        Ok(Self::file(&path))
    }

    /// `package:{dir}` for a workspace package directory.
    pub fn workspace_package(dir: &RepoPath) -> Self {
        Self(format!("package:{}", dir.as_str()))
    }

    /// `http:{METHOD} {normalized_path}`.
    ///
    /// The method is upper-cased (`ALL` is allowed); the path gets a leading `/`, collapsed
    /// `//`, no trailing `/` (except the root), the query string removed and every path
    /// parameter (`:id`, `{id}`, `[id]`, `*`) collapsed to `{}`. Case is preserved.
    pub fn http(method: &str, path: &str) -> Result<Self, NodeIdError> {
        let method = normalize_http_method(method)?;
        Ok(Self(format!("http:{method} {}", normalize_http_path(path))))
    }

    /// `queue:{name}`, trimmed, case preserved.
    pub fn queue(name: &str) -> Result<Self, NodeIdError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        Ok(Self(format!("queue:{name}")))
    }

    /// `db:{schema}.{table}`. Unquoted identifiers are lower-cased (PostgreSQL folds them),
    /// double-quoted identifiers keep their case and are re-emitted quoted, and a missing
    /// schema defaults to `public`.
    pub fn table(schema: Option<&str>, table: &str) -> Result<Self, NodeIdError> {
        let table = table.trim();
        if table.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        let schema = normalize_db_schema(schema);
        Ok(Self(format!("db:{schema}.{}", normalize_db_ident(table))))
    }

    /// `env:{NAME}`. The name must match `^[A-Za-z_][A-Za-z0-9_]*$`.
    ///
    /// Only the variable *name* is ever recorded; values are never read.
    pub fn env(name: &str) -> Result<Self, NodeIdError> {
        if name.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        if !is_env_name(name) {
            return Err(NodeIdError::InvalidEnvName(name.to_owned()));
        }
        Ok(Self(format!("env:{name}")))
    }

    /// `pkg:{ecosystem}/{name}`.
    ///
    /// npm deep imports (`lodash/fp`) reduce to the package, `@scope/name` keeps its scope,
    /// version suffixes are dropped and `node:` builtins become ecosystem `node`. No version
    /// is ever part of the ID.
    pub fn package(ecosystem: &str, spec: &str) -> Result<Self, NodeIdError> {
        let (ecosystem, name) = normalize_package_spec(ecosystem, spec)?;
        Ok(Self(format!("pkg:{ecosystem}/{name}")))
    }

    /// `test:{file}#{suite path} › {name}`. Suites are joined by ` › ` (U+203A) and
    /// whitespace inside every component is collapsed.
    pub fn test(file: &RepoPath, suite_path: &str, name: &str) -> Result<Self, NodeIdError> {
        Self::test_ordinal(file, suite_path, name, 0)
    }

    /// [`Self::test`] with the duplicate ordinal SID-003 assigns in source order: `0` adds
    /// nothing, `n >= 1` appends `~n`.
    pub fn test_ordinal(
        file: &RepoPath,
        suite_path: &str,
        name: &str,
        ordinal: u32,
    ) -> Result<Self, NodeIdError> {
        let name = normalize_test_component(name);
        if name.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        let suite = normalize_test_component(suite_path);
        let mut full = String::new();
        if !suite.is_empty() {
            full.push_str(&suite);
            full.push_str(" \u{203a} ");
        }
        full.push_str(&name);
        if ordinal > 0 {
            full.push('~');
            full.push_str(&ordinal.to_string());
        }
        Ok(Self(format!("test:{}#{full}", file.as_str())))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NodeId({})", self.0)
    }
}

impl std::str::FromStr for NodeId {
    type Err = NodeIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        Ok(Self(s.to_owned()))
    }
}

impl<'de> Deserialize<'de> for NodeId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw.is_empty() {
            return Err(D::Error::custom("node id is empty"));
        }
        Ok(Self(raw))
    }
}

/// Prefixes that belong to the synthetic ID schemes. SID-001 language prefixes (`ts`, `js`,
/// …) must never collide with these; a unit test asserts it for every [`Language`](review_core::language::Language).
pub const RESERVED_PREFIXES: &[&str] = &[
    "repo", "dir", "file", "package", "http", "queue", "db", "env", "pkg", "test",
];

/// Upper-cases and validates an HTTP method token. `ALL` is a legal method in this scheme.
pub fn normalize_http_method(method: &str) -> Result<String, NodeIdError> {
    let method = method.trim().to_ascii_uppercase();
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return Err(NodeIdError::InvalidHttpMethod(method));
    }
    Ok(method)
}

/// Normalizes an HTTP route path (idempotent): strips the query/fragment, collapses path
/// parameters to `{}`, ensures a leading `/`, collapses `//` and drops the trailing slash.
pub fn normalize_http_path(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut joined = String::with_capacity(path.len() + 1);
    for (index, segment) in path.split('/').enumerate() {
        if index > 0 {
            joined.push('/');
        }
        if is_http_param_segment(segment) {
            joined.push_str("{}");
        } else {
            joined.push_str(segment);
        }
    }
    let mut out = String::with_capacity(joined.len() + 1);
    if !joined.starts_with('/') {
        out.push('/');
    }
    out.push_str(&joined);
    let mut collapsed = String::with_capacity(out.len());
    let mut previous_was_slash = false;
    for ch in out.chars() {
        if ch == '/' {
            if !previous_was_slash {
                collapsed.push(ch);
            }
            previous_was_slash = true;
        } else {
            collapsed.push(ch);
            previous_was_slash = false;
        }
    }
    while collapsed.len() > 1 && collapsed.ends_with('/') {
        collapsed.pop();
    }
    collapsed
}

fn is_http_param_segment(segment: &str) -> bool {
    if segment.starts_with(':') || segment.starts_with('*') {
        return true;
    }
    let wrapped = (segment.starts_with('{') && segment.ends_with('}'))
        || (segment.starts_with('[') && segment.ends_with(']'));
    wrapped && segment.len() >= 3
}

/// Lower-cases an unquoted identifier; a double-quoted one keeps its case and is re-emitted
/// with quotes so that normalization stays idempotent.
pub fn normalize_db_ident(ident: &str) -> String {
    let ident = ident.trim();
    if ident.len() >= 2 && ident.starts_with('"') && ident.ends_with('"') {
        let inner = &ident[1..ident.len() - 1];
        let unescaped = inner.replace("\"\"", "\"");
        return format!("\"{unescaped}\"");
    }
    ident.to_ascii_lowercase()
}

/// The schema half of a `db:` ID; `None` and empty mean `public`.
pub fn normalize_db_schema(schema: Option<&str>) -> String {
    match schema.map(str::trim) {
        Some(s) if !s.is_empty() => normalize_db_ident(s),
        _ => "public".to_owned(),
    }
}

/// Splits a package specifier into `(ecosystem, name)` with the version and subpath removed
/// (idempotent).
pub fn normalize_package_spec(
    ecosystem: &str,
    spec: &str,
) -> Result<(String, String), NodeIdError> {
    let mut ecosystem = ecosystem.trim().to_ascii_lowercase();
    if ecosystem.is_empty() {
        return Err(NodeIdError::EmptyName);
    }
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(NodeIdError::EmptyName);
    }
    if let Some(module) = spec.strip_prefix("node:") {
        ecosystem = "node".to_owned();
        let module = module.trim();
        // Same package-identity rule as the npm branch: a subpath is not part of the
        // package name, so `node:fs/promises` and `node:fs` are the same package.
        let module = module.split('/').next().unwrap_or("").trim();
        if module.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        return Ok((ecosystem, module.to_owned()));
    }
    let name = if let Some(rest) = spec.strip_prefix('@') {
        let mut parts = rest.splitn(2, '/');
        let scope = parts.next().unwrap_or("");
        let remainder = parts.next().unwrap_or("");
        if scope.is_empty() || remainder.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        let head = remainder.split('/').next().unwrap_or("");
        let head = strip_version(head);
        if head.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        format!("@{scope}/{head}")
    } else {
        let head = spec.split('/').next().unwrap_or("");
        let head = strip_version(head);
        if head.is_empty() {
            return Err(NodeIdError::EmptyName);
        }
        head.to_owned()
    };
    Ok((ecosystem, name))
}

fn strip_version(name: &str) -> &str {
    match name.split_once('@') {
        Some((base, _)) if !base.is_empty() => base,
        _ => name,
    }
}

/// Collapses whitespace runs and trims (idempotent).
pub fn normalize_test_component(component: &str) -> String {
    component.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `^[A-Za-z_][A-Za-z0-9_]*$` without pulling in a regex engine.
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn path(s: &str) -> RepoPath {
        RepoPath::new(s).unwrap()
    }

    #[test]
    fn http_id_normalizes_params_and_slashes() {
        assert_eq!(
            NodeId::http("get", "/users/:id/").unwrap().as_str(),
            "http:GET /users/{}"
        );
        assert_eq!(
            NodeId::http("GET", "/users//users/{userId}")
                .unwrap()
                .as_str(),
            "http:GET /users/users/{}"
        );
        assert_eq!(
            NodeId::http("GET", "/a/{x}/b/[y]/c/*").unwrap().as_str(),
            "http:GET /a/{}/b/{}/c/{}"
        );
        assert_eq!(
            NodeId::http("GET", "users?page=2#top").unwrap().as_str(),
            "http:GET /users"
        );
        assert_eq!(NodeId::http("ALL", "/").unwrap().as_str(), "http:ALL /");
        assert_eq!(NodeId::http("get", "///").unwrap().as_str(), "http:GET /");
        assert_eq!(
            NodeId::http("post", "/users/:id?").unwrap().as_str(),
            "http:POST /users/{}"
        );
        assert_eq!(
            NodeId::http("get", "/users/:id").unwrap(),
            NodeId::http("GET", "/users/:id").unwrap()
        );
    }

    #[test]
    fn node_http_rejects_bad_methods() {
        assert_eq!(
            NodeId::http("", "/x"),
            Err(NodeIdError::InvalidHttpMethod(String::new()))
        );
        assert_eq!(
            NodeId::http("GET /x", "/x"),
            Err(NodeIdError::InvalidHttpMethod("GET /X".to_owned()))
        );
        assert!(NodeId::http("GET /x", "/x").is_err());
    }

    #[test]
    fn db_id_lowercases_unquoted_and_defaults_public() {
        assert_eq!(
            NodeId::table(None, "Users").unwrap().as_str(),
            "db:public.users"
        );
        assert_eq!(
            NodeId::table(Some("App"), "Users").unwrap().as_str(),
            "db:app.users"
        );
        assert_eq!(
            NodeId::table(Some("\"App\""), "\"MyTable\"")
                .unwrap()
                .as_str(),
            "db:\"App\".\"MyTable\""
        );
        assert_eq!(
            NodeId::table(Some("  "), "orders").unwrap().as_str(),
            "db:public.orders"
        );
        assert_eq!(
            NodeId::table(None, "  Users  ").unwrap(),
            NodeId::table(None, "Users").unwrap()
        );
        assert_eq!(NodeId::table(None, " "), Err(NodeIdError::EmptyName));
    }

    #[test]
    fn pkg_id_reduces_deep_imports_and_scopes() {
        assert_eq!(
            NodeId::package("npm", "@babel/core/lib/index.js")
                .unwrap()
                .as_str(),
            "pkg:npm/@babel/core"
        );
        assert_eq!(
            NodeId::package("npm", "lodash/fp").unwrap().as_str(),
            "pkg:npm/lodash"
        );
        assert_eq!(
            NodeId::package("npm", "lodash@4.17.21").unwrap().as_str(),
            "pkg:npm/lodash"
        );
        assert_eq!(
            NodeId::package("npm", "node:fs/promises").unwrap().as_str(),
            "pkg:node/fs"
        );
        assert_eq!(
            NodeId::package("NPM", "@scope/name@1.2.3/sub")
                .unwrap()
                .as_str(),
            "pkg:npm/@scope/name"
        );
        assert_eq!(
            NodeId::package("npm", "node:fs").unwrap(),
            NodeId::package("node", "fs").unwrap()
        );
        assert_eq!(NodeId::package("", "x"), Err(NodeIdError::EmptyName));
        assert_eq!(NodeId::package("npm", "/"), Err(NodeIdError::EmptyName));
    }

    #[test]
    fn test_id_joins_suite_path_and_orders_duplicates() {
        let file = path("src/users/users.service.spec.ts");
        assert_eq!(
            NodeId::test(&file, "UsersService", "findOne")
                .unwrap()
                .as_str(),
            "test:src/users/users.service.spec.ts#UsersService \u{203a} findOne"
        );
        assert_eq!(
            NodeId::test(&file, "outer  inner", "  a   b  ")
                .unwrap()
                .as_str(),
            "test:src/users/users.service.spec.ts#outer inner \u{203a} a b"
        );
        let first = NodeId::test_ordinal(&file, "S", "dup", 0).unwrap();
        let second = NodeId::test_ordinal(&file, "S", "dup", 1).unwrap();
        let third = NodeId::test_ordinal(&file, "S", "dup", 2).unwrap();
        assert_eq!(
            first.as_str(),
            "test:src/users/users.service.spec.ts#S \u{203a} dup"
        );
        assert_eq!(
            second.as_str(),
            "test:src/users/users.service.spec.ts#S \u{203a} dup~1"
        );
        assert_eq!(
            third.as_str(),
            "test:src/users/users.service.spec.ts#S \u{203a} dup~2"
        );
        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(
            NodeId::test(&file, "", "root").unwrap().as_str(),
            "test:src/users/users.service.spec.ts#root"
        );
        assert_eq!(NodeId::test(&file, "S", "   "), Err(NodeIdError::EmptyName));
    }

    #[test]
    fn env_id_rejects_invalid_names() {
        assert_eq!(NodeId::env("API_KEY").unwrap().as_str(), "env:API_KEY");
        assert_eq!(NodeId::env("_private").unwrap().as_str(), "env:_private");
        assert_eq!(NodeId::env("a1_b2").unwrap().as_str(), "env:a1_b2");
        assert_eq!(NodeId::env(""), Err(NodeIdError::EmptyName));
        assert_eq!(
            NodeId::env("1BAD"),
            Err(NodeIdError::InvalidEnvName("1BAD".to_owned()))
        );
        assert_eq!(
            NodeId::env("A-B"),
            Err(NodeIdError::InvalidEnvName("A-B".to_owned()))
        );
        assert_eq!(
            NodeId::env("A B"),
            Err(NodeIdError::InvalidEnvName("A B".to_owned()))
        );
        assert_eq!(
            NodeId::env("Ä"),
            Err(NodeIdError::InvalidEnvName("\u{c4}".to_owned()))
        );
    }

    #[test]
    fn node_structural_ids_and_path_guards() {
        assert_eq!(NodeId::repository().as_str(), "repo:/");
        assert_eq!(NodeId::root_directory().as_str(), "dir:.");
        assert_eq!(
            NodeId::directory(&path("src/users")).as_str(),
            "dir:src/users"
        );
        assert_eq!(NodeId::file(&path("src/a.ts")).as_str(), "file:src/a.ts");
        assert_eq!(
            NodeId::workspace_package(&path("packages/api")).as_str(),
            "package:packages/api"
        );
        assert_eq!(
            NodeId::containing_directory(&path("src/users/users.service.ts")).as_str(),
            "dir:src/users"
        );
        assert_eq!(
            NodeId::containing_directory(&path("a.ts")).as_str(),
            "dir:."
        );
        assert_eq!(
            NodeId::ancestor_directories(&path("src/users/users.service.ts")),
            vec![
                NodeId::root_directory(),
                NodeId::from_canonical("dir:src".to_owned()),
                NodeId::from_canonical("dir:src/users".to_owned()),
            ]
        );
        assert_eq!(
            NodeId::ancestor_directories(&path("a.ts")),
            vec![NodeId::root_directory()]
        );
        assert_eq!(
            NodeId::file_str("../x"),
            Err(NodeIdError::PathOutsideRepository("../x".to_owned()))
        );
        assert_eq!(
            NodeId::directory_str("/abs"),
            Err(NodeIdError::PathOutsideRepository("/abs".to_owned()))
        );
        assert_eq!(
            NodeId::file_str("src/a.ts").unwrap(),
            NodeId::file(&path("src/a.ts"))
        );
    }

    #[test]
    fn reserved_prefixes_do_not_collide_with_language_prefixes() {
        for prefix in RESERVED_PREFIXES {
            for lang in review_core::language::Language::ALL {
                assert_ne!(
                    *prefix,
                    lang.id_prefix(),
                    "language prefix collides with a reserved synthetic prefix"
                );
            }
        }
        assert_eq!(
            RESERVED_PREFIXES,
            ["repo", "dir", "file", "package", "http", "queue", "db", "env", "pkg", "test"]
        );
    }

    #[test]
    fn node_key_equals_blake3_prefix_of_id() {
        let id = NodeId::http("GET", "/users/:id").unwrap();
        let hash = blake3::hash(id.as_str().as_bytes());
        let mut expected = [0u8; 16];
        expected.copy_from_slice(&hash.as_bytes()[..16]);
        assert_eq!(*id.key().as_bytes(), expected);

        let symbol = NodeId::from_canonical(
            "ts:src/auth/auth.service#AuthService.authorize/method".to_owned(),
        );
        let hash = blake3::hash(symbol.as_str().as_bytes());
        let mut expected = [0u8; 16];
        expected.copy_from_slice(&hash.as_bytes()[..16]);
        assert_eq!(*symbol.key().as_bytes(), expected);
        assert_eq!(
            symbol.key(),
            NodeKey::of(&SymbolId::from_canonical_unchecked(symbol.as_str()))
        );
        assert_eq!(symbol.key().to_string().len(), 32);
    }

    #[test]
    fn node_id_serde_and_display_roundtrip() {
        let id = NodeId::queue("email").unwrap();
        assert_eq!(id.to_string(), "queue:email");
        assert_eq!(format!("{id:?}"), "NodeId(queue:email)".to_owned());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"queue:email\"");
        assert_eq!(serde_json::from_str::<NodeId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<NodeId>("\"\"").is_err());
        assert_eq!("queue:email".parse::<NodeId>().unwrap(), id);
        assert_eq!("".parse::<NodeId>(), Err(NodeIdError::EmptyName));
    }

    #[test]
    fn node_schemes_produce_disjoint_keyspaces() {
        let ids = [
            NodeId::repository(),
            NodeId::root_directory(),
            NodeId::file(&path("a.ts")),
            NodeId::workspace_package(&path("a")),
            NodeId::http("GET", "/").unwrap(),
            NodeId::queue("q").unwrap(),
            NodeId::table(None, "t").unwrap(),
            NodeId::env("E").unwrap(),
            NodeId::package("npm", "x").unwrap(),
            NodeId::test(&path("a.spec.ts"), "S", "n").unwrap(),
        ];
        let keys: std::collections::HashSet<NodeKey> = ids.iter().map(NodeId::key).collect();
        assert_eq!(keys.len(), ids.len());
        for id in ids {
            assert!(id.as_str().contains(':'), "{}", id.as_str());
            let prefix = id.as_str().split(':').next().unwrap_or("");
            assert!(RESERVED_PREFIXES.contains(&prefix), "{prefix}");
        }
    }

    proptest! {
        #[test]
        fn http_normalization_is_idempotent(path in ".{0,40}") {
            let once = normalize_http_path(&path);
            prop_assert_eq!(normalize_http_path(&once), once);
        }

        #[test]
        fn http_normalization_is_idempotent_for_route_shapes(
            prefix in "[a-z]{0,6}",
            param in "[A-Za-z0-9_]{0,8}",
            depth in 0usize..4,
        ) {
            let mut raw = String::from("/");
            raw.push_str(&prefix);
            if !param.is_empty() {
                raw.push('/');
                raw.push(':');
                raw.push_str(&param);
            }
            for _ in 0..depth {
                raw.push_str("//x/{id}");
            }
            let once = normalize_http_path(&raw);
            prop_assert_eq!(normalize_http_path(&once), once.clone());
            prop_assert!(!once.contains("//"));
            prop_assert!(!once.ends_with('/') || once == "/");
        }

        #[test]
        fn db_normalization_is_idempotent(ident in ".{0,32}") {
            let once = normalize_db_ident(&ident);
            prop_assert_eq!(normalize_db_ident(&once), once);
            let schema = normalize_db_schema(Some(&ident));
            prop_assert_eq!(normalize_db_schema(Some(&schema)), schema);
            prop_assert_eq!(normalize_db_schema(None), "public".to_owned());
        }

        #[test]
        fn pkg_normalization_is_idempotent(
            ecosystem in "[A-Za-z]{1,8}",
            spec in "[@A-Za-z0-9/._-]{1,24}",
        ) {
            if let Ok((eco, name)) = normalize_package_spec(&ecosystem, &spec) {
                match normalize_package_spec(&eco, &name) {
                    Ok(second) => prop_assert_eq!(second, (eco, name)),
                    Err(_) => prop_assert!(false, "a normalized pair must normalize again"),
                }
            }
        }

        #[test]
        fn test_normalization_is_idempotent(component in ".{0,40}") {
            let once = normalize_test_component(&component);
            prop_assert_eq!(normalize_test_component(&once), once);
        }
    }
}
