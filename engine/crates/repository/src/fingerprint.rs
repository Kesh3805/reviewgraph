//! Repository fingerprint (INIT-012, ADR-015).
//!
//! One value that changes exactly when an input that affects derived intelligence changes. Every
//! field is written as `tag: u8`, `len: u64 LE`, `bytes`, so different inputs can never
//! concatenate to the same byte stream.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::str::FromStr;

use rayon::prelude::*;
use review_core::ids::{CommitSha, RepositoryId};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::version::AnalyzerVersion;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{InitError, InitWarning};
use crate::git::GitState;
use crate::read::BoundedReader;
use crate::walk::{FileClass, FileInventory};

/// Domain separator of the fingerprint hash.
pub const FINGERPRINT_DOMAIN: &[u8] = b"rg.fp.v1\0";
const LOCAL_ID_DOMAIN: &[u8] = b"rg.repo.local.v1\0";
const WORKTREE_DOMAIN: &[u8] = b"rg.worktree.v1\0";

/// Repository identity as the fingerprint sees it: a hosted UUID or a portable local id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum RepoIdentity {
    Hosted(RepositoryId),
    /// 32 lowercase hex characters derived from the normalized origin URL.
    Local(String),
}

impl RepoIdentity {
    fn canonical(&self) -> String {
        match self {
            Self::Hosted(id) => format!("hosted:{id}"),
            Self::Local(hex) => format!("local:{hex}"),
        }
    }
}

/// The commit input of the fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitComponent {
    Clean(CommitSha),
    Dirty {
        head: CommitSha,
        worktree_hash: [u8; 32],
    },
    NoGit {
        tree_hash: [u8; 32],
    },
}

impl CommitComponent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Clean(_) => "clean",
            Self::Dirty { .. } => "dirty",
            Self::NoGit { .. } => "nogit",
        }
    }

    fn canonical(&self) -> String {
        match self {
            Self::Clean(sha) => format!("clean:{sha}"),
            Self::Dirty {
                head,
                worktree_hash,
            } => format!("dirty:{head}:{}", hex::encode(worktree_hash)),
            Self::NoGit { tree_hash } => format!("nogit:{}", hex::encode(tree_hash)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FingerprintInputs<'a> {
    pub repository_id: &'a RepoIdentity,
    pub commit: &'a CommitComponent,
    pub analyzer_versions: &'a BTreeMap<Language, AnalyzerVersion>,
    pub graph_schema_version: u32,
    pub config_hash: [u8; 32],
    /// Sorted by name.
    pub parser_versions: &'a [(&'a str, &'a str)],
    pub profile_version: u32,
}

/// `fp1:` followed by 64 lowercase hex characters.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint([u8; 32]);

impl Fingerprint {
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fp1:{}", hex::encode(self.0))
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({self})")
    }
}

impl FromStr for Fingerprint {
    type Err = InitError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex_part = s
            .strip_prefix("fp1:")
            .filter(|h| h.len() == 64 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .ok_or_else(|| InitError::Serde("invalid fingerprint".to_owned()))?;
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(hex_part, &mut bytes).map_err(|e| InitError::Serde(e.to_string()))?;
        Ok(Self(bytes))
    }
}

fn field(hasher: &mut blake3::Hasher, tag: u8, bytes: &[u8]) {
    hasher.update(&[tag]);
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Computes the fingerprint. Tags, in fixed ADR-015 order: 1 repository id, 2 commit,
/// 3 analyzer versions, 4 graph schema version, 5 config hash, 6 parser versions,
/// 7 profile version.
pub fn compute_fingerprint(inputs: &FingerprintInputs<'_>) -> Fingerprint {
    let mut hasher = blake3::Hasher::new();
    hasher.update(FINGERPRINT_DOMAIN);
    field(&mut hasher, 1, inputs.repository_id.canonical().as_bytes());
    field(&mut hasher, 2, inputs.commit.canonical().as_bytes());

    let mut analyzers: Vec<(&str, String)> = inputs
        .analyzer_versions
        .iter()
        .map(|(lang, version)| (lang.id_prefix(), version.as_semver().to_string()))
        .collect();
    analyzers.sort();
    let analyzer_text: String = analyzers
        .iter()
        .map(|(lang, version)| format!("{lang}={version}\n"))
        .collect();
    field(&mut hasher, 3, analyzer_text.as_bytes());

    field(&mut hasher, 4, &inputs.graph_schema_version.to_le_bytes());
    field(&mut hasher, 5, &inputs.config_hash);

    let mut parsers: Vec<(&str, &str)> = inputs.parser_versions.to_vec();
    parsers.sort();
    let parser_text: String = parsers
        .iter()
        .map(|(name, version)| format!("{name}={version}\n"))
        .collect();
    field(&mut hasher, 6, parser_text.as_bytes());

    field(&mut hasher, 7, &inputs.profile_version.to_le_bytes());
    Fingerprint(*hasher.finalize().as_bytes())
}

/// Hash of the changed files of a dirty worktree: sorted `(path, blake3(content) | "deleted")`.
/// Sensitive files contribute only path and size, never content, so the hash cannot be used as
/// an oracle for secret values.
pub fn worktree_hash(
    root: &Path,
    inventory: &FileInventory,
    reader: &BoundedReader,
    changed: &[RepoPath],
) -> ([u8; 32], Vec<InitWarning>) {
    let mut sorted: Vec<&RepoPath> = changed.iter().collect();
    sorted.sort();
    sorted.dedup();
    let lines: Vec<(String, Option<InitWarning>)> = sorted
        .par_iter()
        .map(|path| {
            let mut warning = None;
            let state = match inventory.find(path.as_str()) {
                Some(entry) => match entry.class {
                    FileClass::Sensitive => format!("sensitive:{}", entry.size),
                    FileClass::Source => {
                        match reader.read_prefix(entry, usize::try_from(entry.size).unwrap_or(0)) {
                            Ok(bytes) => hex::encode(blake3::hash(&bytes).as_bytes()),
                            Err(_) => {
                                warning = Some(InitWarning::new(
                                    "unreadable_entry",
                                    Some((*path).clone()),
                                    "changed file could not be read for the worktree hash",
                                ));
                                format!("unreadable:{path}")
                            }
                        }
                    }
                    other => format!("{}:{}", other.as_str(), entry.size),
                },
                None => match std::fs::metadata(root.join(path.as_str())) {
                    Err(_) => "deleted".to_owned(),
                    Ok(meta) => format!("unlisted:{}", meta.len()),
                },
            };
            (format!("{path}\0{state}\n"), warning)
        })
        .collect();
    let mut hasher = blake3::Hasher::new();
    hasher.update(WORKTREE_DOMAIN);
    let mut warnings = Vec::new();
    for (line, warning) in lines {
        hasher.update(line.as_bytes());
        warnings.extend(warning);
    }
    (*hasher.finalize().as_bytes(), warnings)
}

/// Hash of the whole inventory, for directories that are not git repositories.
pub fn tree_hash(inventory: &FileInventory, reader: &BoundedReader) -> [u8; 32] {
    let all: Vec<RepoPath> = inventory.entries.iter().map(|e| e.path.clone()).collect();
    worktree_hash(&inventory.root, inventory, reader, &all).0
}

/// The commit component for a repository's current state.
pub fn commit_component(
    git: Option<&GitState>,
    root: &Path,
    inventory: &FileInventory,
    reader: &BoundedReader,
) -> (CommitComponent, Vec<InitWarning>) {
    match git.and_then(|g| g.head.sha().map(|sha| (g, sha))) {
        Some((state, sha)) if !state.dirty.is_dirty => {
            (CommitComponent::Clean(sha.clone()), Vec::new())
        }
        Some((state, sha)) => {
            let (hash, warnings) = worktree_hash(root, inventory, reader, &state.dirty.all_paths);
            (
                CommitComponent::Dirty {
                    head: sha.clone(),
                    worktree_hash: hash,
                },
                warnings,
            )
        }
        None => (
            CommitComponent::NoGit {
                tree_hash: tree_hash(inventory, reader),
            },
            Vec::new(),
        ),
    }
}

/// Portable local repository id: `blake3("rg.repo.local.v1\0" || normalized origin)[..32 hex]`.
/// Without an origin the canonical root path is used and a warning says the id is not portable.
pub fn local_repository_id(
    git: Option<&GitState>,
    root: &Path,
) -> (RepoIdentity, Option<InitWarning>) {
    let origin = git.and_then(|g| {
        g.remotes
            .iter()
            .find(|r| r.name == "origin")
            .or_else(|| g.remotes.first())
            .and_then(|r| r.slug.clone())
    });
    let (material, warning) = match origin {
        Some(slug) => (slug.to_ascii_lowercase(), None),
        None => (
            root.to_string_lossy().into_owned(),
            Some(InitWarning::new(
                "repository_id_from_path",
                None,
                "no remote is configured: the local repository id is derived from the path and is not portable",
            )),
        ),
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(LOCAL_ID_DOMAIN);
    hasher.update(material.as_bytes());
    let id = hex::encode(hasher.finalize().as_bytes())[..32].to_owned();
    (RepoIdentity::Local(id), warning)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without length prefixes `("ab", "c")` and `("a", "bc")` would hash identically.
    #[test]
    fn length_prefix_prevents_concat_ambiguity() {
        let hash = |first: &str, second: &str| {
            let mut hasher = blake3::Hasher::new();
            field(&mut hasher, 1, first.as_bytes());
            field(&mut hasher, 2, second.as_bytes());
            *hasher.finalize().as_bytes()
        };
        assert_ne!(hash("ab", "c"), hash("a", "bc"));
        let mut naive_a = blake3::Hasher::new();
        naive_a.update(b"ab");
        naive_a.update(b"c");
        let mut naive_b = blake3::Hasher::new();
        naive_b.update(b"a");
        naive_b.update(b"bc");
        assert_eq!(naive_a.finalize(), naive_b.finalize());
    }

    #[test]
    fn fingerprint_text_round_trips_and_rejects_garbage() {
        let fp = Fingerprint([9u8; 32]);
        assert_eq!(fp.to_string().parse::<Fingerprint>().unwrap(), fp);
        assert!("fp2:00".parse::<Fingerprint>().is_err());
        assert!("fp1:ZZ".parse::<Fingerprint>().is_err());
    }
}
