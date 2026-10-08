//! Symbol summary units: a deterministic structural description (names and structure only).

use review_core::ids::{RepositoryId, SnapshotId, SymbolId, SymbolKey};
use review_core::language::Language;
use review_core::location::RepoPath;
use review_core::symbol::{ModulePath, SymbolKind};

use super::{finish, truncate_chars, EmbeddingUnit, UnitKind, UnitMeta};

/// Cap on summary text.
pub const SUMMARY_MAX_CHARS: usize = 1_500;
const DOC_CHARS: usize = 300;
const TOP_CALLEES: usize = 10;
const TOP_CALLERS: usize = 5;

/// What the unit builders need to know about one symbol. Filled by the composition root from
/// the IR (ranges, decorators, doc, body text) and the code graph (callers, callees).
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInput {
    pub symbol_id: SymbolId,
    pub kind: SymbolKind,
    /// `AuthService.authorize`.
    pub qualified_name: String,
    /// Declaration line(s) without the body.
    pub signature: Option<String>,
    pub file_path: RepoPath,
    pub language: Language,
    /// 1-based, inclusive.
    pub start_line: u32,
    pub end_line: u32,
    pub decorators: Vec<String>,
    /// Callee names with edge confidence.
    pub callees: Vec<(String, f32)>,
    pub callers: Vec<String>,
    pub doc: Option<String>,
    /// Source text of the whole declaration; `None` after a degraded parse.
    pub body: Option<String>,
    pub is_private: bool,
    /// File is generated or vendored (INIT-008).
    pub is_generated: bool,
    /// File carries a secrets signal at index time (SEC-003).
    pub has_secrets: bool,
}

impl SymbolInput {
    pub fn symbol_key(&self) -> SymbolKey {
        SymbolKey::of(&self.symbol_id)
    }

    pub fn line_count(&self) -> u32 {
        self.end_line.saturating_sub(self.start_line) + 1
    }

    pub(crate) fn meta(
        &self,
        kind: UnitKind,
        key: String,
        repository_id: RepositoryId,
        snapshot_id: SnapshotId,
    ) -> UnitMeta {
        UnitMeta {
            kind,
            key,
            repository_id,
            snapshot_id,
            language: Some(self.language),
            module: Some(ModulePath::of(&self.file_path).as_str().to_owned()),
            file_path: Some(self.file_path.clone()),
            start_line: Some(self.start_line),
            end_line: Some(self.end_line),
            symbol_key: Some(self.symbol_key()),
        }
    }
}

fn summarized(kind: SymbolKind) -> bool {
    matches!(
        kind,
        SymbolKind::Function
            | SymbolKind::Method
            | SymbolKind::Constructor
            | SymbolKind::Getter
            | SymbolKind::Setter
            | SymbolKind::Class
            | SymbolKind::Interface
    )
}

fn top_callees(callees: &[(String, f32)]) -> Vec<&str> {
    let mut sorted: Vec<&(String, f32)> = callees.iter().collect();
    sorted.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut out: Vec<&str> = Vec::new();
    for (name, _) in sorted {
        if !out.contains(&name.as_str()) {
            out.push(name);
        }
        if out.len() == TOP_CALLEES {
            break;
        }
    }
    out
}

fn top_callers(callers: &[String]) -> Vec<&str> {
    let mut sorted: Vec<&str> = callers.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.truncate(TOP_CALLERS);
    sorted
}

fn list(items: &[&str]) -> String {
    if items.is_empty() {
        "-".to_owned()
    } else {
        items.join(", ")
    }
}

/// The summary text before redaction and truncation.
pub(crate) fn summary_text(sym: &SymbolInput) -> String {
    let decorators: Vec<&str> = sym.decorators.iter().map(String::as_str).collect();
    let doc = sym
        .doc
        .as_deref()
        .map(|d| {
            truncate_chars(
                &d.split_whitespace().collect::<Vec<_>>().join(" "),
                DOC_CHARS,
            )
        })
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| "-".to_owned());
    let signature = sym
        .signature
        .as_deref()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| "-".to_owned());
    format!(
        "{kind} {name}\nsignature: {signature}\nmodule: {module}\ndecorators: {decorators}\n\
         calls: {calls}\ncalled by: {callers}\ndoc: {doc}",
        kind = sym.kind.as_id_str(),
        name = sym.qualified_name,
        module = ModulePath::of(&sym.file_path),
        decorators = list(&decorators),
        calls = list(&top_callees(&sym.callees)),
        callers = list(&top_callers(&sym.callers)),
    )
}

/// The symbol-summary unit, or `None` for kinds that are not summarized, generated files and
/// private trivial accessors (< 3 lines).
pub fn symbol_summary(
    sym: &SymbolInput,
    repository_id: RepositoryId,
    snapshot_id: SnapshotId,
) -> Option<EmbeddingUnit> {
    if !summarized(sym.kind) || sym.is_generated {
        return None;
    }
    let accessor = matches!(sym.kind, SymbolKind::Getter | SymbolKind::Setter);
    if accessor && sym.is_private && sym.line_count() < 3 {
        return None;
    }
    let meta = sym.meta(
        UnitKind::SymbolSummary,
        sym.symbol_key().to_string(),
        repository_id,
        snapshot_id,
    );
    Some(finish(meta, &summary_text(sym), SUMMARY_MAX_CHARS))
}
