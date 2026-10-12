//! Compression (CTX-007): structured excerpts with preserved source locations.
//!
//! Text is read from blobs at the commit of the item's side, never from a working tree. A blob
//! that cannot be read downgrades the item to a reference; secret-bearing excerpts drop the
//! whole item.

pub mod excerpt;
pub mod levels;
pub mod secrets;

use review_core::ids::CommitSha;
use review_core::location::RepoPath;

pub use excerpt::{Hints, Target};
pub use levels::{cheaper, default_level};
pub use secrets::contains_secret;

use crate::budget::estimate_tokens;
use crate::candidates::{Candidate, LineSpan};
use crate::model::{CompressionLevel, ItemContent, ItemKind, SourceLocation};

/// Reads file contents at a commit (object database blobs; the composition root implements it
/// over the repository adapter).
pub trait BlobSource {
    fn read(&self, commit: &CommitSha, path: &RepoPath) -> Option<String>;
}

/// The outcome of rendering one candidate at one level.
#[derive(Debug, Clone, PartialEq)]
pub enum Rendered {
    Content {
        content: ItemContent,
        locations: Vec<SourceLocation>,
        /// The blob could not be read; the item was downgraded to a reference.
        downgraded: bool,
    },
    SecretBearing,
}

impl Rendered {
    /// Token cost of the rendered form, including the title.
    pub fn tokens(&self, title: &str) -> u32 {
        match self {
            Self::Content { content, .. } => {
                estimate_tokens(&format!("{title}\n{}", content.text()))
            }
            Self::SecretBearing => 0,
        }
    }
}

fn reference(span: Option<&LineSpan>, commit: &CommitSha, downgraded: bool) -> Rendered {
    let locations = span
        .map(|s| {
            vec![SourceLocation {
                file: s.file.clone(),
                start_line: s.start_line,
                end_line: s.end_line,
                side: s.side,
                commit: commit.clone(),
            }]
        })
        .unwrap_or_default();
    Rendered::Content {
        content: ItemContent::Code {
            level: CompressionLevel::Reference,
            segments: Vec::new(),
        },
        locations,
        downgraded,
    }
}

/// Number of lines a candidate spans (1 when unknown).
pub fn span_lines(span: Option<&LineSpan>) -> u32 {
    span.map_or(1, |s| s.end_line.saturating_sub(s.start_line) + 1)
}

/// Renders `cand` at `level`. `commit` is the commit of the candidate's side.
pub fn render(
    cand: &Candidate,
    level: CompressionLevel,
    commit: &CommitSha,
    blobs: &dyn BlobSource,
    hints: &Hints<'_>,
) -> Rendered {
    let span = cand.span.as_ref();
    if let Some(value) = &cand.structured {
        if contains_secret(&value.to_string()) {
            return Rendered::SecretBearing;
        }
        let locations = span
            .map(|s| {
                vec![SourceLocation {
                    file: s.file.clone(),
                    start_line: s.start_line,
                    end_line: s.end_line,
                    side: s.side,
                    commit: commit.clone(),
                }]
            })
            .unwrap_or_default();
        return Rendered::Content {
            content: ItemContent::Structured {
                value: value.clone(),
            },
            locations,
            downgraded: false,
        };
    }
    let Some(span) = span else {
        return Rendered::Content {
            content: ItemContent::Empty,
            locations: Vec::new(),
            downgraded: false,
        };
    };
    if level == CompressionLevel::Reference {
        return reference(Some(span), commit, false);
    }
    let Some(text) = blobs.read(commit, &span.file) else {
        return reference(Some(span), commit, true);
    };
    let target = Target {
        file: &span.file,
        start_line: span.start_line,
        end_line: span.end_line,
        side: span.side,
        commit,
        is_class: cand.kind == ItemKind::ClassSignature,
    };
    let Some(segments) = excerpt::extract(&text, &target, level, hints) else {
        return reference(Some(span), commit, true);
    };
    if segments.iter().any(|s| contains_secret(&s.text)) {
        return Rendered::SecretBearing;
    }
    let locations = segments.iter().map(|s| s.location.clone()).collect();
    Rendered::Content {
        content: ItemContent::Code { level, segments },
        locations,
        downgraded: false,
    }
}
