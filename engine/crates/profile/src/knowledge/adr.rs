//! The `adr` adapter: `ADR-*.md` architecture decision records.
//!
//! Frontmatter wins when present (same keys as the vault). Otherwise the status is read from a
//! `Status:` line (`Status: Accepted`, `**Status:** Superseded by ADR-007`, `- Status: proposed`)
//! or from a `## Status` section's first line, and the title from the first heading. Every ADR
//! is a `decision`.

use super::frontmatter::{self, first_heading};
use super::{
    body_hash, file_stem, slug, DocKind, DocStatus, DocumentationRule, KnowledgeDiagnostic,
    KnowledgeFile, KnowledgeSourceAdapter,
};
use crate::config::{KnowledgeSourceConfig, KnowledgeSourceKind};

#[derive(Debug, Clone, Copy, Default)]
pub struct AdrSource;

/// Whether a file name is an ADR (`ADR-001-title.md`, `adr-0001.md`, `0001-title.md`).
pub fn is_adr_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    if !(name.ends_with(".md") || name.ends_with(".markdown")) {
        return false;
    }
    name.starts_with("adr-")
        || name.starts_with("adr_")
        || (name.len() > 4 && name.chars().take(4).all(|c| c.is_ascii_digit()))
}

/// The status from a `Status:` line or a `## Status` section.
pub fn status_from_body(body: &str) -> Option<DocStatus> {
    let lines: Vec<&str> = body.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw
            .trim()
            .trim_start_matches(['-', '*', ' '])
            .replace("**", "");
        let lowered = line.to_ascii_lowercase();
        if let Some(rest) = lowered.strip_prefix("status:") {
            return DocStatus::parse(rest);
        }
        if lowered.trim_start_matches('#').trim() == "status" && line.starts_with('#') {
            return lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .and_then(DocStatus::parse);
        }
    }
    None
}

impl KnowledgeSourceAdapter for AdrSource {
    fn kind(&self) -> KnowledgeSourceKind {
        KnowledgeSourceKind::Adr
    }

    fn ingest(
        &self,
        source: &KnowledgeSourceConfig,
        files: &[KnowledgeFile],
    ) -> (Vec<DocumentationRule>, Vec<KnowledgeDiagnostic>) {
        let mut rules = Vec::new();
        let mut diagnostics = Vec::new();
        for file in files.iter().filter(|f| is_adr_file(&f.path)) {
            let parsed = match frontmatter::parse(&file.content) {
                Ok(parsed) => parsed,
                Err(message) => {
                    diagnostics.push(KnowledgeDiagnostic {
                        source_id: source.id.clone(),
                        path: file.path.clone(),
                        message,
                    });
                    continue;
                }
            };
            let stem = file_stem(&file.path);
            let fm = parsed.frontmatter.as_ref();
            let status = fm
                .and_then(|fm| frontmatter::string(fm, "status"))
                .and_then(|s| DocStatus::parse(&s))
                .or_else(|| status_from_body(&parsed.body));
            let Some(status) = status else {
                diagnostics.push(KnowledgeDiagnostic {
                    source_id: source.id.clone(),
                    path: file.path.clone(),
                    message: "no Status line; the ADR was indexed as proposed".to_owned(),
                });
                rules.push(build(source, file, &parsed, &stem, DocStatus::Proposed));
                continue;
            };
            if !status.is_active() {
                continue;
            }
            rules.push(build(source, file, &parsed, &stem, status));
        }
        (rules, diagnostics)
    }
}

fn build(
    source: &KnowledgeSourceConfig,
    file: &KnowledgeFile,
    parsed: &frontmatter::Parsed,
    stem: &str,
    status: DocStatus,
) -> DocumentationRule {
    let fm = parsed.frontmatter.as_ref();
    let id = fm
        .and_then(|fm| frontmatter::string(fm, "id"))
        .unwrap_or_else(|| stem.to_owned());
    let rule_id = slug(&id);
    DocumentationRule {
        node_id: format!("doc:{}/{rule_id}", source.id),
        source_id: source.id.clone(),
        rule_id,
        title: fm
            .and_then(|fm| frontmatter::string(fm, "title"))
            .or_else(|| first_heading(&parsed.body))
            .unwrap_or_else(|| stem.to_owned()),
        kind: DocKind::Decision,
        severity: fm.and_then(|fm| frontmatter::string(fm, "severity")),
        status,
        body_hash: body_hash(&parsed.body),
        path: file.path.clone(),
        line_range: (
            parsed.body_start_line,
            parsed.total_lines.max(parsed.body_start_line),
        ),
        applies_to: fm
            .map(|fm| frontmatter::strings(fm, "applies_to"))
            .unwrap_or_default(),
        policy: fm.and_then(super::DocPolicy::from_frontmatter),
    }
}
