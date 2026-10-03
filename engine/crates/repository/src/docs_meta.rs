//! Rule-doc and architecture metadata discovery (INIT-010).
//!
//! Only Markdown prefixes and CODEOWNERS are read. Tool configuration inside `.claude/`,
//! `.obsidian/` and similar directories is never read.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use regex::Regex;
use review_core::location::RepoPath;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::codeowners::{parse_codeowners, CodeownersFact, CODEOWNERS_PATHS};
use crate::dirs::RepoDir;
use crate::error::InitWarning;
use crate::read::BoundedReader;
use crate::walk::{FileClass, FileEntry, FileInventory};

const RULE_DOC_MAX_BYTES: u64 = 512 * 1024;
const MAX_RULE_DOCS: usize = 200;
const ADR_DIRS: &[&str] = &[
    "docs/adr",
    "docs/adrs",
    "docs/decisions",
    "doc/adr",
    "adr",
    "architecture/decisions",
];
const RULE_KEYWORDS: &[&str] = &[
    "guideline",
    "convention",
    "standard",
    "rule",
    "style",
    "security",
    "testing",
    "coding",
    "review",
    "policy",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AdrFact {
    pub path: RepoPath,
    pub number: Option<u32>,
    pub title: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuleDocCandidate {
    pub path: RepoPath,
    pub score: f32,
    pub signals: Vec<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VaultKind {
    AgentVault,
    Obsidian,
    CursorRules,
    ClaudeDir,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KnowledgeVault {
    pub root: RepoDir,
    pub kind: VaultKind,
    pub markdown_files: u64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct DocsFacts {
    pub adrs: Vec<AdrFact>,
    pub architecture: Vec<RepoPath>,
    pub contributing: Vec<RepoPath>,
    pub security_policy: Vec<RepoPath>,
    pub pr_templates: Vec<RepoPath>,
    pub agent_instructions: Vec<RepoPath>,
    pub rule_docs: Vec<RuleDocCandidate>,
    pub codeowners: Option<CodeownersFact>,
    pub knowledge_vaults: Vec<KnowledgeVault>,
}

struct Regexes {
    adr_name: Regex,
    status: Regex,
    imperative: Regex,
}

fn regexes() -> &'static Option<Regexes> {
    static RE: OnceLock<Option<Regexes>> = OnceLock::new();
    RE.get_or_init(|| {
        Some(Regexes {
            adr_name: Regex::new(r"^(?:ADR[-_ ]?)?(\d{3,4})[-_].*\.md$").ok()?,
            status: Regex::new(r"(?i)^\**status:?\**\s*:?\s*(\w+)").ok()?,
            imperative: Regex::new(r"(?i)\b(must|never|always|do not)\b").ok()?,
        })
    })
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}

fn is_markdown(path: &str) -> bool {
    path.ends_with(".md") || path.ends_with(".mdx")
}

fn is_agent_instruction(path: &str) -> bool {
    let name = basename(path);
    matches!(
        name,
        "AGENTS.md" | "CLAUDE.md" | ".cursorrules" | ".windsurfrules" | "SKILL.md"
    ) || path == ".github/copilot-instructions.md"
        || (path.starts_with(".cursor/rules/") && path.ends_with(".mdc"))
}

/// Detects documentation and architecture metadata.
pub fn detect_docs(inv: &FileInventory, reader: &BoundedReader) -> (DocsFacts, Vec<InitWarning>) {
    let span = tracing::info_span!("init.docs");
    let _guard = span.enter();

    let mut warnings = Vec::new();
    let mut facts = DocsFacts::default();
    let Some(re) = regexes() else {
        return (facts, warnings);
    };
    let readable =
        |entry: &FileEntry| matches!(entry.class, FileClass::Source | FileClass::TooLarge);

    // ADR directories: named, or any directory with >= 2 numbered markdown files.
    let mut numbered: BTreeMap<&str, usize> = BTreeMap::new();
    for entry in &inv.entries {
        let path = entry.path.as_str();
        if re.adr_name.is_match(basename(path)) {
            *numbered.entry(parent(path)).or_insert(0) += 1;
        }
    }
    let adr_dirs: BTreeSet<&str> = inv
        .dirs
        .iter()
        .map(|d| d.as_str())
        .filter(|d| {
            ADR_DIRS
                .iter()
                .any(|n| d == n || d.ends_with(&format!("/{n}")))
        })
        .chain(numbered.iter().filter(|(_, n)| **n >= 2).map(|(d, _)| *d))
        .collect();
    for entry in &inv.entries {
        let path = entry.path.as_str();
        if !is_markdown(path) || !adr_dirs.contains(parent(path)) || !readable(entry) {
            continue;
        }
        let name = basename(path);
        if name.eq_ignore_ascii_case("README.md") {
            continue;
        }
        let number = re
            .adr_name
            .captures(name)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse().ok());
        let (title, status) = match reader.read_text(entry, 4096) {
            Ok(text) => {
                let title = text
                    .lines()
                    .find_map(|l| l.strip_prefix("# "))
                    .map(|t| t.trim().to_owned());
                let status = text
                    .lines()
                    .take(40)
                    .find_map(|l| re.status.captures(l.trim()))
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_owned());
                (title, status)
            }
            Err(_) => {
                warnings.push(InitWarning::new(
                    "unreadable_entry",
                    Some(entry.path.clone()),
                    "document could not be read",
                ));
                (None, None)
            }
        };
        facts.adrs.push(AdrFact {
            path: entry.path.clone(),
            number,
            title,
            status,
        });
    }
    facts
        .adrs
        .sort_by(|a, b| (a.number, &a.path).cmp(&(b.number, &b.path)));

    // Simple path-based lists.
    for entry in &inv.entries {
        let path = entry.path.as_str();
        let lower = path.to_ascii_lowercase();
        let name = basename(&lower);
        if name == "architecture.md"
            || (lower.starts_with("docs/architecture")
                && is_markdown(&lower)
                && !lower[5..].contains("/decisions"))
            || lower == "docs/system-overview.md"
            || (lower.starts_with("docs/design/") && is_markdown(&lower))
        {
            // ARCHITECTURE.md at any depth is architecture documentation.
            if name != "architecture.md"
                || parent(&lower).is_empty()
                || parent(&lower).starts_with("docs")
            {
                facts.architecture.push(entry.path.clone());
            }
        }
        if name == "contributing.md" && matches!(parent(&lower), "" | ".github" | "docs") {
            facts.contributing.push(entry.path.clone());
        }
        if name == "security.md" && matches!(parent(&lower), "" | ".github" | "docs") {
            facts.security_policy.push(entry.path.clone());
        }
        if (lower.starts_with(".github/pull_request_template")
            || lower == "docs/pull_request_template.md")
            && is_markdown(&lower)
        {
            facts.pr_templates.push(entry.path.clone());
        }
        if is_agent_instruction(path) {
            facts.agent_instructions.push(entry.path.clone());
        }
    }
    for list in [
        &mut facts.architecture,
        &mut facts.contributing,
        &mut facts.security_policy,
        &mut facts.pr_templates,
        &mut facts.agent_instructions,
    ] {
        list.sort();
        list.dedup();
    }

    // CODEOWNERS: first location wins.
    for candidate in CODEOWNERS_PATHS {
        if let Some(entry) = inv.find(candidate) {
            match reader.read_text(entry, 256 * 1024) {
                Ok(text) => {
                    facts.codeowners = Some(parse_codeowners(entry.path.clone(), &text));
                }
                Err(_) => warnings.push(InitWarning::new(
                    "unreadable_entry",
                    Some(entry.path.clone()),
                    "CODEOWNERS could not be read",
                )),
            }
            break;
        }
    }

    // Knowledge vaults.
    let md_under = |root: &RepoDir| -> u64 {
        inv.entries
            .iter()
            .filter(|e| is_markdown(e.path.as_str()) && root.contains(e.path.as_str()))
            .count() as u64
    };
    let mut vault_roots: BTreeMap<(RepoDir, VaultKind), ()> = BTreeMap::new();
    for dir in &inv.dirs {
        let d = dir.as_str();
        if let Some(rest) = d.strip_prefix(".agent/") {
            if !rest.contains('/') {
                if let Ok(root) = RepoDir::new(d) {
                    vault_roots.insert((root, VaultKind::AgentVault), ());
                }
            }
        }
        if basename(d) == ".obsidian" {
            if let Ok(root) = RepoDir::new(parent(d)) {
                vault_roots.insert((root, VaultKind::Obsidian), ());
            }
        }
        if d == ".cursor/rules" {
            if let Ok(root) = RepoDir::new(d) {
                vault_roots.insert((root, VaultKind::CursorRules), ());
            }
        }
        if d == ".claude" {
            if let Ok(root) = RepoDir::new(d) {
                vault_roots.insert((root, VaultKind::ClaudeDir), ());
            }
        }
    }
    for (root, kind) in vault_roots.into_keys() {
        let markdown_files = md_under(&root);
        facts.knowledge_vaults.push(KnowledgeVault {
            root,
            kind,
            markdown_files,
        });
    }
    facts
        .knowledge_vaults
        .sort_by(|a, b| (&a.root, a.kind).cmp(&(&b.root, b.kind)));

    // Rule-doc candidates.
    let vault_roots: Vec<&RepoDir> = facts.knowledge_vaults.iter().map(|v| &v.root).collect();
    let mut candidates = Vec::new();
    for entry in &inv.entries {
        let path = entry.path.as_str();
        let is_doc = is_markdown(path) || path.ends_with(".mdc") || is_agent_instruction(path);
        if !is_doc || !readable(entry) || entry.size > RULE_DOC_MAX_BYTES {
            continue;
        }
        let in_docs = path.starts_with("docs/") || path.starts_with("rules/");
        let at_root = !path.contains('/');
        let in_vault = vault_roots.iter().any(|v| !v.is_root() && v.contains(path));
        if !(in_docs || at_root || in_vault || is_agent_instruction(path)) {
            continue;
        }
        let name = basename(path).to_ascii_lowercase();
        let mut score = 0.0f32;
        let mut signals = Vec::new();
        if RULE_KEYWORDS.iter().any(|k| name.contains(k)) {
            score += 0.4;
            signals.push("basename_keyword".to_owned());
        }
        if in_docs {
            score += 0.2;
            signals.push("docs_or_rules_dir".to_owned());
        }
        if is_agent_instruction(path) {
            score += 0.5;
            signals.push("agent_instruction_file".to_owned());
        }
        if let Ok(text) = reader.read_text(entry, 4096) {
            if re.imperative.find_iter(&text).count() >= 3 {
                score += 0.2;
                signals.push("imperative_markers".to_owned());
            }
        }
        let score = (score.min(1.0) * 100.0).round() / 100.0;
        if score >= 0.4 {
            candidates.push(RuleDocCandidate {
                path: entry.path.clone(),
                score,
                signals,
            });
        }
    }
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path))
    });
    if candidates.len() > MAX_RULE_DOCS {
        candidates.truncate(MAX_RULE_DOCS);
        warnings.push(InitWarning::new(
            "list_truncated",
            None,
            "rule_docs was capped at 200 entries",
        ));
    }
    facts.rule_docs = candidates;
    warnings.sort();
    (facts, warnings)
}
