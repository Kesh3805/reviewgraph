//! The `markdown_vault` adapter: markdown files with frontmatter keys `id`, `title`,
//! `applies_to` (globs or symbol ids), `kind: rule|decision|guide`, `severity` and
//! `status: accepted|proposed|superseded|deprecated`.
//!
//! A document without frontmatter is indexed as a `guide` with no `applies_to`: retrievable, but
//! with no structural edges. Malformed frontmatter skips the document with a diagnostic.

use super::frontmatter::{self, first_heading};
use super::{
    body_hash, file_stem, slug, DocKind, DocStatus, DocumentationRule, KnowledgeDiagnostic,
    KnowledgeFile, KnowledgeSourceAdapter,
};
use crate::config::{KnowledgeSourceConfig, KnowledgeSourceKind};

#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownVault;

impl KnowledgeSourceAdapter for MarkdownVault {
    fn kind(&self) -> KnowledgeSourceKind {
        KnowledgeSourceKind::MarkdownVault
    }

    fn ingest(
        &self,
        source: &KnowledgeSourceConfig,
        files: &[KnowledgeFile],
    ) -> (Vec<DocumentationRule>, Vec<KnowledgeDiagnostic>) {
        let mut rules = Vec::new();
        let mut diagnostics = Vec::new();
        for file in files {
            let diagnostic = |message: String| KnowledgeDiagnostic {
                source_id: source.id.clone(),
                path: file.path.clone(),
                message,
            };
            let parsed = match frontmatter::parse(&file.content) {
                Ok(parsed) => parsed,
                Err(message) => {
                    diagnostics.push(diagnostic(message));
                    continue;
                }
            };
            let stem = file_stem(&file.path);
            let (id, title, kind, severity, status, applies_to) = match &parsed.frontmatter {
                None => (
                    stem.clone(),
                    first_heading(&parsed.body).unwrap_or_else(|| stem.clone()),
                    DocKind::Guide,
                    None,
                    DocStatus::Accepted,
                    Vec::new(),
                ),
                Some(fm) => {
                    let kind = match frontmatter::string(fm, "kind") {
                        None => DocKind::Guide,
                        Some(raw) => match DocKind::parse(&raw) {
                            Some(kind) => kind,
                            None => {
                                diagnostics.push(diagnostic(format!("unknown kind `{raw}`")));
                                continue;
                            }
                        },
                    };
                    let status = match frontmatter::string(fm, "status") {
                        None => DocStatus::Accepted,
                        Some(raw) => match DocStatus::parse(&raw) {
                            Some(status) => status,
                            None => {
                                diagnostics.push(diagnostic(format!("unknown status `{raw}`")));
                                continue;
                            }
                        },
                    };
                    (
                        frontmatter::string(fm, "id").unwrap_or_else(|| stem.clone()),
                        frontmatter::string(fm, "title")
                            .or_else(|| first_heading(&parsed.body))
                            .unwrap_or_else(|| stem.clone()),
                        kind,
                        frontmatter::string(fm, "severity"),
                        status,
                        frontmatter::strings(fm, "applies_to"),
                    )
                }
            };
            if !status.is_active() {
                continue;
            }
            let rule_id = slug(&id);
            if rule_id.is_empty() {
                diagnostics.push(diagnostic("document id is empty".to_owned()));
                continue;
            }
            rules.push(DocumentationRule {
                node_id: format!("doc:{}/{rule_id}", source.id),
                source_id: source.id.clone(),
                rule_id,
                title,
                kind,
                severity,
                status,
                body_hash: body_hash(&parsed.body),
                path: file.path.clone(),
                line_range: (parsed.body_start_line, parsed.total_lines.max(parsed.body_start_line)),
                applies_to,
                policy: parsed
                    .frontmatter
                    .as_ref()
                    .and_then(super::DocPolicy::from_frontmatter),
            });
        }
        (rules, diagnostics)
    }
}
