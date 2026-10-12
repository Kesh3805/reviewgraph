//! Building typed evidence (DOM-007) for deterministic results.

use review_core::evidence::{
    Evidence, EvidenceKind, EvidenceOrigin, EvidenceStrength, EvidenceVerification, SymbolRef,
};
use review_core::ids::SymbolId;
use review_core::location::{DiffSide, LineRange, RepoPath, SourceLocation};

const MAX_CLAIM_CHARS: usize = 500;

/// A head-side location of lines `start..=end` (clamped to at least line 1).
pub fn location(path: &str, start: u32, end: u32) -> Option<SourceLocation> {
    let start = start.max(1);
    Some(SourceLocation {
        path: RepoPath::new(path).ok()?,
        side: DiffSide::Head,
        lines: LineRange::new(start, end.max(start)).ok()?,
        range: None,
    })
}

pub fn symbol_ref(id: &str, display: &str) -> Option<SymbolRef> {
    let display: String = display.chars().take(200).collect();
    SymbolRef::new(SymbolId::from_canonical_unchecked(id), display).ok()
}

fn clip(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= MAX_CLAIM_CHARS {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(MAX_CLAIM_CHARS - 1).collect();
    out.push('…');
    out
}

/// A strong, deterministic, unverified evidence item. `tool` names the producer
/// (`policy:forbidden_dependency`, `precheck:tests_gap`, ...).
pub fn deterministic(
    tool: &str,
    kind: EvidenceKind,
    claim: &str,
    location: Option<SourceLocation>,
    symbols: Vec<SymbolRef>,
) -> Option<Evidence> {
    let claim = clip(claim);
    if claim.is_empty() {
        return None;
    }
    Evidence {
        kind,
        claimed_strength: EvidenceStrength::Strong,
        origin: EvidenceOrigin::Deterministic {
            tool: tool.to_owned(),
        },
        verification: EvidenceVerification::Unverified,
        claim,
        location,
        symbols,
        relation: None,
    }
    .validated()
    .ok()
}
