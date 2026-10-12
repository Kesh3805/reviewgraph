//! Line selection and segment extraction (CTX-007). Pure functions over file text.

use std::collections::BTreeSet;

use review_core::ids::CommitSha;
use review_core::location::{DiffSide, RepoPath};

use crate::model::{CompressionLevel, ElidedLines, Segment, SourceLocation};

/// Context lines around a changed line in [`CompressionLevel::ChangedHunks`].
pub const HUNK_CONTEXT: u32 = 3;
/// Leading lines kept by [`CompressionLevel::BodyTrimmed`].
pub const TRIM_HEAD_LINES: u32 = 15;
/// Trailing lines kept by [`CompressionLevel::BodyTrimmed`].
pub const TRIM_TAIL_LINES: u32 = 5;
/// Lines kept after a seed call (the guard or throw that follows it).
pub const CALL_TRAILING_LINES: u32 = 2;
const MAX_SIGNATURE_LINES: u32 = 5;

/// Where the interesting lines of a symbol are.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hints<'a> {
    /// Lines changed on the side being rendered.
    pub changed_lines: &'a [u32],
    /// Lines containing a call to a seed symbol (`CALLS` edge locations).
    pub seed_call_lines: &'a [u32],
}

/// What to render: a line range of a file at one side and commit.
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    pub file: &'a RepoPath,
    pub start_line: u32,
    pub end_line: u32,
    pub side: DiffSide,
    pub commit: &'a CommitSha,
    /// A class: [`CompressionLevel::SignatureOnly`] keeps the header and member signatures.
    pub is_class: bool,
}

fn signature_end(lines: &[&str], start: u32, end: u32) -> u32 {
    let limit = end.min(start + MAX_SIGNATURE_LINES - 1);
    for n in start..=limit {
        let Some(line) = usize::try_from(n - 1).ok().and_then(|i| lines.get(i)) else {
            break;
        };
        let t = line.trim_end();
        if t.contains('{') || t.ends_with(';') || t.ends_with(':') {
            return n;
        }
    }
    limit
}

fn class_signature_lines(lines: &[&str], start: u32, end: u32) -> BTreeSet<u32> {
    let mut keep: BTreeSet<u32> = (start..=signature_end(lines, start, end)).collect();
    let mut depth: i32 = 0;
    for n in start..=end {
        let Some(line) = usize::try_from(n - 1).ok().and_then(|i| lines.get(i)) else {
            break;
        };
        let trimmed = line.trim_start();
        if depth == 1
            && line.contains('(')
            && !trimmed.starts_with("//")
            && !trimmed.starts_with('*')
        {
            keep.insert(n);
        }
        let opens = i32::try_from(line.matches('{').count()).unwrap_or(0);
        let closes = i32::try_from(line.matches('}').count()).unwrap_or(0);
        depth += opens - closes;
    }
    keep
}

/// The 1-based lines kept for `level`.
pub fn select_lines(
    lines: &[&str],
    target: &Target<'_>,
    level: CompressionLevel,
    hints: &Hints<'_>,
) -> BTreeSet<u32> {
    let start = target.start_line;
    let end = target.end_line;
    let sig_end = signature_end(lines, start, end);
    let signature = || (start..=sig_end);
    match level {
        CompressionLevel::Full => (start..=end).collect(),
        CompressionLevel::Reference => BTreeSet::new(),
        CompressionLevel::SignatureOnly => {
            if target.is_class {
                class_signature_lines(lines, start, end)
            } else {
                signature().collect()
            }
        }
        CompressionLevel::BodyTrimmed => {
            let mut keep: BTreeSet<u32> = signature().collect();
            keep.extend(start..=end.min(start + TRIM_HEAD_LINES - 1));
            keep.extend(end.saturating_sub(TRIM_TAIL_LINES - 1).max(start)..=end);
            for &call in hints
                .seed_call_lines
                .iter()
                .filter(|l| (start..=end).contains(l))
            {
                keep.extend(call..=end.min(call + CALL_TRAILING_LINES));
            }
            keep
        }
        CompressionLevel::ChangedHunks => {
            let mut keep: BTreeSet<u32> = signature().collect();
            for &line in hints
                .changed_lines
                .iter()
                .filter(|l| (start..=end).contains(l))
            {
                keep.extend(
                    line.saturating_sub(HUNK_CONTEXT).max(start)..=end.min(line + HUNK_CONTEXT),
                );
            }
            keep
        }
    }
}

/// Renders the kept lines as segments (one per contiguous run). `None` when the range lies
/// outside the file.
pub fn extract(
    text: &str,
    target: &Target<'_>,
    level: CompressionLevel,
    hints: &Hints<'_>,
) -> Option<Vec<Segment>> {
    let lines: Vec<&str> = text.lines().collect();
    let total = u32::try_from(lines.len()).ok()?;
    if target.start_line == 0 || target.start_line > total || target.end_line < target.start_line {
        return None;
    }
    let clamped = Target {
        end_line: target.end_line.min(total),
        ..*target
    };
    let keep = select_lines(&lines, &clamped, level, hints);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for n in keep {
        if let Some(last) = runs.last_mut() {
            if last.1 + 1 == n {
                last.1 = n;
                continue;
            }
        }
        runs.push((n, n));
    }
    let mut segments = Vec::with_capacity(runs.len());
    let mut prev_end: Option<u32> = None;
    for (s, e) in runs {
        let slice = lines.get(usize::try_from(s - 1).ok()?..usize::try_from(e).ok()?)?;
        let elided_before = prev_end.map(|p| ElidedLines {
            start: p + 1,
            end: s - 1,
        });
        segments.push(Segment {
            location: SourceLocation {
                file: target.file.clone(),
                start_line: s,
                end_line: e,
                side: target.side,
                commit: target.commit.clone(),
            },
            text: slice.join("\n"),
            elided_before,
        });
        prev_end = Some(e);
    }
    Some(segments)
}
