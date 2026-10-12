//! Knowledge sources: documented rules become `DocumentationRule` nodes (PROF-007).
//!
//! A source is configured in `.review/config.yaml` `knowledge_sources:` and read by an adapter:
//! [`markdown_vault`] (frontmatter-driven) or [`adr`] (`ADR-*.md`, `Status:` lines). Bodies are
//! never stored: a rule carries its path, line range and body hash, and context loads the body
//! with redaction. `applies_to` globs resolve against the snapshot's files into governs edges,
//! capped at 500 per rule; above the cap the rule attaches to the matched files' common
//! directory instead. See `docs/reviewers/knowledge-sources.md`.

pub mod adr;
pub mod frontmatter;
pub mod markdown_vault;

use std::collections::BTreeSet;
use std::path::Path;

use codegraph::{
    confidence_of, Edge, EdgeKind, NodeId, NodeInput, NodeInputAttrs, NodeKind, Provenance,
    ResolvedBy,
};
use globset::{Glob, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::config::{KnowledgeSourceConfig, KnowledgeSourceKind, ReviewConfigV1};
use crate::model::DocumentationRuleRef;

/// Most governs edges one rule gets before it attaches to a directory instead.
pub const MAX_EDGES_PER_RULE: usize = 500;
/// Largest document read.
pub const MAX_DOCUMENT_BYTES: usize = 512 * 1024;

/// One markdown file of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeFile {
    /// Repository-relative path.
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Rule,
    Decision,
    Guide,
}

impl DocKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "rule" => Some(Self::Rule),
            "decision" | "adr" => Some(Self::Decision),
            "guide" | "note" => Some(Self::Guide),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Decision => "decision",
            Self::Guide => "guide",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocStatus {
    Accepted,
    Proposed,
    Superseded,
    Deprecated,
}

impl DocStatus {
    /// Reads `accepted`, `Accepted (2024-01-02)`, `Superseded by ADR-007`, ...
    pub fn parse(raw: &str) -> Option<Self> {
        let lowered = raw.trim().to_ascii_lowercase();
        let word = lowered
            .split(|c: char| !c.is_ascii_alphabetic())
            .find(|w| !w.is_empty())?;
        match word {
            "accepted" | "active" | "approved" => Some(Self::Accepted),
            "proposed" | "draft" => Some(Self::Proposed),
            "superseded" | "replaced" => Some(Self::Superseded),
            "deprecated" | "rejected" | "retired" => Some(Self::Deprecated),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Proposed => "proposed",
            Self::Superseded => "superseded",
            Self::Deprecated => "deprecated",
        }
    }

    /// Superseded and deprecated documents are not rules any more.
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Accepted | Self::Proposed)
    }
}

/// A documented rule (one `DocumentationRule` node).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentationRule {
    /// `doc:{source_id}/{rule_id}`.
    pub node_id: String,
    pub source_id: String,
    pub rule_id: String,
    pub title: String,
    pub kind: DocKind,
    pub severity: Option<String>,
    pub status: DocStatus,
    /// blake3 of the body, hex.
    pub body_hash: String,
    pub path: String,
    /// 1-based inclusive line range of the body.
    pub line_range: (u32, u32),
    /// Globs or symbol ids. Empty for a guide without structure.
    pub applies_to: Vec<String>,
    /// The machine-readable policy the document states, if its frontmatter declares one
    /// (`topic`, `decision`, and `from`/`to` for a layer dependency). POL-004 ranks it as
    /// documented policy.
    pub policy: Option<DocPolicy>,
}

/// A documented policy statement (frontmatter `topic`, `decision`, `from`, `to`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocPolicy {
    pub topic: String,
    pub decision: String,
    pub from: Option<String>,
    pub to: Option<String>,
}

impl DocPolicy {
    /// Reads the policy keys of a frontmatter block.
    pub fn from_frontmatter(
        fm: &std::collections::BTreeMap<String, serde_yaml::Value>,
    ) -> Option<Self> {
        Some(Self {
            topic: frontmatter::string(fm, "topic")?,
            decision: frontmatter::string(fm, "decision").unwrap_or_else(|| "required".to_owned()),
            from: frontmatter::string(fm, "from"),
            to: frontmatter::string(fm, "to"),
        })
    }
}

impl DocumentationRule {
    pub fn to_ref(&self) -> DocumentationRuleRef {
        DocumentationRuleRef {
            id: self.node_id.clone(),
            source_id: self.source_id.clone(),
            title: self.title.clone(),
            kind: self.kind.as_str().to_owned(),
            severity: self.severity.clone(),
            path: self.path.clone(),
            applies_to: self.applies_to.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum GovernedTarget {
    File(String),
    /// The matched files' common directory (edge cap exceeded).
    Directory(String),
    Symbol(String),
}

/// `DocumentationRule -[GOVERNS]-> target`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GovernsEdge {
    pub rule: String,
    pub target: GovernedTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeDiagnostic {
    pub source_id: String,
    pub path: String,
    pub message: String,
}

/// The output of ingesting every configured source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnowledgeIngest {
    pub rules: Vec<DocumentationRule>,
    pub edges: Vec<GovernsEdge>,
    pub diagnostics: Vec<KnowledgeDiagnostic>,
}

impl KnowledgeIngest {
    pub fn refs(&self) -> Vec<DocumentationRuleRef> {
        self.rules.iter().map(DocumentationRule::to_ref).collect()
    }

    pub fn rule(&self, node_id: &str) -> Option<&DocumentationRule> {
        self.rules.iter().find(|r| r.node_id == node_id)
    }

    /// The synthetic nodes and edges to add to the graph. Edges use provenance `POLICY`; until
    /// the graph schema has a dedicated `GOVERNS` kind they are `REFERENCES` edges from the rule.
    pub fn to_graph_inputs(&self) -> (Vec<NodeInput>, Vec<Edge>) {
        let nodes = self
            .rules
            .iter()
            .map(|r| {
                let attrs = NodeInputAttrs {
                    extra: vec![
                        ("kind".to_owned(), r.kind.as_str().to_owned()),
                        ("severity".to_owned(), r.severity.clone().unwrap_or_default()),
                        ("status".to_owned(), r.status.as_str().to_owned()),
                        ("body_hash".to_owned(), r.body_hash.clone()),
                        ("path".to_owned(), r.path.clone()),
                        (
                            "line_range".to_owned(),
                            format!("{}-{}", r.line_range.0, r.line_range.1),
                        ),
                    ],
                    ..NodeInputAttrs::default()
                };
                NodeInput::new(
                    NodeId::from_canonical(r.node_id.clone()),
                    NodeKind::DocumentationRule,
                    r.title.clone(),
                )
                .qualified_name(r.node_id.clone())
                .with_attrs(attrs)
            })
            .collect();
        let edges = self
            .edges
            .iter()
            .map(|e| {
                let target = match &e.target {
                    GovernedTarget::File(path) => format!("file:{path}"),
                    GovernedTarget::Directory(dir) => format!("dir:{dir}"),
                    GovernedTarget::Symbol(id) => id.clone(),
                };
                Edge::new(
                    EdgeKind::References,
                    NodeId::from_canonical(e.rule.clone()).key(),
                    NodeId::from_canonical(target).key(),
                    confidence_of(ResolvedBy::Structural),
                    ResolvedBy::Structural,
                    Provenance::Policy,
                )
            })
            .collect();
        (nodes, edges)
    }
}

/// A knowledge-source adapter.
pub trait KnowledgeSourceAdapter {
    fn kind(&self) -> KnowledgeSourceKind;
    /// Parses the source's documents. Superseded documents are dropped; malformed ones become
    /// diagnostics.
    fn ingest(
        &self,
        source: &KnowledgeSourceConfig,
        files: &[KnowledgeFile],
    ) -> (Vec<DocumentationRule>, Vec<KnowledgeDiagnostic>);
}

pub(crate) fn body_hash(body: &str) -> String {
    blake3::hash(body.as_bytes()).to_hex().to_string()
}

pub(crate) fn file_stem(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md")
        .or_else(|| name.strip_suffix(".markdown"))
        .unwrap_or(name)
        .to_owned()
}

/// A rule id as used in a node id: lower-case, `[a-z0-9._-]` only.
pub(crate) fn slug(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// Whether an `applies_to` entry is a symbol id rather than a glob.
fn is_symbol_id(entry: &str) -> bool {
    entry.contains('#') || entry.starts_with("ts:") || entry.starts_with("http:")
}

fn common_dir(paths: &[&str]) -> String {
    let mut prefix: Vec<&str> = match paths.first() {
        Some(first) => {
            let mut segments: Vec<&str> = first.split('/').collect();
            segments.pop();
            segments
        }
        None => return String::new(),
    };
    for path in &paths[1..] {
        let segments: Vec<&str> = path.split('/').collect();
        let shared = prefix
            .iter()
            .zip(segments.iter())
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(shared);
    }
    prefix.join("/")
}

/// Resolves a rule's `applies_to` against the snapshot's files.
pub fn resolve_edges(rule: &DocumentationRule, files: &[String]) -> Vec<GovernsEdge> {
    let mut out = BTreeSet::new();
    let mut builder = GlobSetBuilder::new();
    let mut any_glob = false;
    for entry in &rule.applies_to {
        if is_symbol_id(entry) {
            out.insert(GovernsEdge {
                rule: rule.node_id.clone(),
                target: GovernedTarget::Symbol(entry.clone()),
            });
        } else if let Ok(glob) = Glob::new(entry) {
            builder.add(glob);
            any_glob = true;
        }
    }
    if any_glob {
        if let Ok(set) = builder.build() {
            let matched: Vec<&str> = files
                .iter()
                .map(String::as_str)
                .filter(|f| set.is_match(f))
                .collect();
            if matched.len() > MAX_EDGES_PER_RULE {
                out.insert(GovernsEdge {
                    rule: rule.node_id.clone(),
                    target: GovernedTarget::Directory(common_dir(&matched)),
                });
            } else {
                for path in matched {
                    out.insert(GovernsEdge {
                        rule: rule.node_id.clone(),
                        target: GovernedTarget::File(path.to_owned()),
                    });
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Ingests every configured source. `read` returns a source directory's markdown files (`None`
/// when the directory does not exist in the snapshot); `files` is the snapshot's file list.
pub fn ingest_sources(
    config: &ReviewConfigV1,
    read: &dyn Fn(&str) -> Option<Vec<KnowledgeFile>>,
    files: &[String],
) -> KnowledgeIngest {
    let span = tracing::info_span!("knowledge_source_ingest", sources = config.knowledge_sources.len());
    let _guard = span.enter();
    let mut ingest = KnowledgeIngest::default();
    for source in &config.knowledge_sources {
        let Some(documents) = read(&source.path) else {
            ingest.diagnostics.push(KnowledgeDiagnostic {
                source_id: source.id.clone(),
                path: source.path.clone(),
                message: format!("knowledge source `{}` path does not exist", source.id),
            });
            continue;
        };
        let (rules, diagnostics) = match source.kind {
            KnowledgeSourceKind::MarkdownVault => markdown_vault::MarkdownVault.ingest(source, &documents),
            KnowledgeSourceKind::Adr => adr::AdrSource.ingest(source, &documents),
        };
        crate::metrics::documentation_rules(&source.id, rules.len());
        for rule in &rules {
            ingest.edges.extend(resolve_edges(rule, files));
        }
        ingest.rules.extend(rules);
        ingest.diagnostics.extend(diagnostics);
    }
    ingest.rules.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    ingest.rules.dedup_by(|a, b| a.node_id == b.node_id);
    ingest.edges.sort();
    ingest.edges.dedup();
    ingest
}

/// Reads the markdown files under `root/rel` (recursively, sorted). `None` when the directory
/// does not exist.
pub fn read_markdown_dir(root: &Path, rel: &str) -> Option<Vec<KnowledgeFile>> {
    let base = root.join(rel);
    if !base.is_dir() {
        return None;
    }
    let mut out = Vec::new();
    let mut stack = vec![base];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e == "md" || e == "markdown")
            {
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.len() as usize > MAX_DOCUMENT_BYTES {
                    continue;
                }
                let Ok(content) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(relative) = path.strip_prefix(root) else {
                    continue;
                };
                let rel_path = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.push(KnowledgeFile {
                    path: rel_path,
                    content,
                });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Some(out)
}
