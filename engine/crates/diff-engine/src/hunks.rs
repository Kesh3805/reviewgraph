//! Line-level diff of two blobs (DIFF-003).
//!
//! [`compute_hunks`] interns lines, runs the histogram algorithm of imara-diff (the copy vendored
//! by gix, which carries the indentation slider heuristic), derives the zero-context changed
//! ranges first and then expands them into unified-diff hunks with context. Nothing here reads
//! git configuration, attributes, environment variables or the locale: the result is a pure
//! function of the two byte buffers and the [`HunkOptions`].

use std::ops::Range;

use gix::diff::blob::{Algorithm, Diff, InternedInput};
use review_core::change::Hunk;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::lines::{is_trivia, split_lines, to_u32, token, Line, LineToken};

/// Line statistics of one diffed file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineStats {
    /// Added lines.
    pub additions: u32,
    /// Deleted lines.
    pub deletions: u32,
}

/// Kind of a line within a [`DiffHunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Present on both sides.
    Context,
    /// Present only on the new side.
    Add,
    /// Present only on the old side.
    Del,
}

/// One line inside a hunk. `span` addresses the buffer of the line's own side: the old buffer
/// for [`LineKind::Del`], the new buffer for [`LineKind::Add`] and [`LineKind::Context`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HunkLine {
    /// Which side the line belongs to.
    pub kind: LineKind,
    /// 1-based old-side line number (`None` for additions).
    pub old_no: Option<u32>,
    /// 1-based new-side line number (`None` for deletions).
    pub new_no: Option<u32>,
    /// Byte range of the line within its side's buffer, including the line terminator.
    pub span: Range<u32>,
    /// The line had no trailing newline at end of input.
    pub no_eol: bool,
    /// The line is blank or looks like a comment (a hint for DIFF-006 `touches_code`).
    #[serde(default)]
    pub trivia: bool,
}

/// A unified-diff hunk with full line detail (the header mirrors the DOM-005 [`Hunk`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DiffHunk {
    /// Unified header semantics: `@@ -old_start,old_lines +new_start,new_lines @@`.
    pub header: Hunk,
    /// Lines of the hunk in side order (within a change: deletions before additions).
    pub lines: Vec<HunkLine>,
}

/// Line-diff algorithm. Only histogram is used in production; Myers exists for experiments.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiffAlgo {
    /// Histogram diff (git `--histogram`).
    #[default]
    Histogram,
    /// Myers diff.
    Myers,
}

/// Options of [`compute_hunks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkOptions {
    /// Unchanged lines of context around each change (`3`, like `-U3`).
    pub context: u32,
    /// Diff algorithm (histogram).
    pub algorithm: DiffAlgo,
    /// Treat `\r\n` and `\n` as equal line endings (`false`).
    pub ignore_eol: bool,
    /// Inputs with more lines than this on either side fail with [`HunkError::TooLarge`].
    pub max_lines: u32,
}

impl Default for HunkOptions {
    fn default() -> Self {
        Self {
            context: 3,
            algorithm: DiffAlgo::Histogram,
            ignore_eol: false,
            max_lines: 200_000,
        }
    }
}

/// The full line diff of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HunkSet {
    /// Hunks with context, in file order.
    pub hunks: Vec<DiffHunk>,
    /// Zero-context changed old-side lines: 1-based, `[start, end)`, sorted and disjoint.
    pub changed_old: Vec<Range<u32>>,
    /// Zero-context changed new-side lines: 1-based, `[start, end)`, sorted and disjoint.
    pub changed_new: Vec<Range<u32>>,
    /// Added and deleted line counts.
    pub stats: LineStats,
}

/// Failures of [`compute_hunks`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HunkError {
    /// One side has more lines than [`HunkOptions::max_lines`]; DIFF-004 records the file as
    /// `TooLarge` instead of diffing it.
    #[error("input has {lines} lines, above the limit of {max}")]
    TooLarge {
        /// Line count of the larger side.
        lines: u32,
        /// The configured limit.
        max: u32,
    },
}

/// Diff `old` against `new` line by line.
pub fn compute_hunks(old: &[u8], new: &[u8], o: &HunkOptions) -> Result<HunkSet, HunkError> {
    let old_lines = split_lines(old);
    let new_lines = split_lines(new);
    let largest = to_u32(old_lines.len().max(new_lines.len()));
    if largest > o.max_lines {
        return Err(HunkError::TooLarge {
            lines: largest,
            max: o.max_lines,
        });
    }

    let changes = raw_changes(old, &old_lines, new, &new_lines, o);

    let mut set = HunkSet::default();
    for change in &changes {
        if !change.before.is_empty() {
            set.changed_old
                .push(change.before.start + 1..change.before.end + 1);
        }
        if !change.after.is_empty() {
            set.changed_new
                .push(change.after.start + 1..change.after.end + 1);
        }
        set.stats.deletions += change.before.end - change.before.start;
        set.stats.additions += change.after.end - change.after.start;
    }
    set.changed_old = merge_ranges(std::mem::take(&mut set.changed_old));
    set.changed_new = merge_ranges(std::mem::take(&mut set.changed_new));

    let sides = Sides {
        old,
        new,
        old_lines: &old_lines,
        new_lines: &new_lines,
    };
    set.hunks = group(&changes, o.context)
        .into_iter()
        .map(|g| build_hunk(&sides, g, o.context))
        .collect();
    Ok(set)
}

/// A change in 0-based line indices: `before` lines of old replaced by `after` lines of new.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Change {
    before: Range<u32>,
    after: Range<u32>,
}

fn raw_changes(
    old: &[u8],
    old_lines: &[Line],
    new: &[u8],
    new_lines: &[Line],
    o: &HunkOptions,
) -> Vec<Change> {
    let mut input: InternedInput<LineToken<'_>> = InternedInput::default();
    input.update_before(old_lines.iter().map(|l| token(old, l, o.ignore_eol)));
    input.update_after(new_lines.iter().map(|l| token(new, l, o.ignore_eol)));
    let algorithm = match o.algorithm {
        DiffAlgo::Histogram => Algorithm::Histogram,
        DiffAlgo::Myers => Algorithm::Myers,
    };
    let mut diff = Diff::compute(algorithm, &input);
    diff.postprocess_lines(&input);
    diff.hunks()
        .map(|h| Change {
            before: h.before,
            after: h.after,
        })
        .collect()
}

/// Merge touching or overlapping ranges of a sorted list.
fn merge_ranges(ranges: Vec<Range<u32>>) -> Vec<Range<u32>> {
    let mut out: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        if let Some(last) = out.last_mut() {
            if r.start <= last.end {
                last.end = last.end.max(r.end);
                continue;
            }
        }
        out.push(r);
    }
    out
}

/// Changes whose surrounding context overlaps (gap of at most `2 * context` unchanged lines)
/// belong to one hunk.
fn group(changes: &[Change], context: u32) -> Vec<&[Change]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for i in 1..changes.len() {
        let gap = changes[i]
            .before
            .start
            .saturating_sub(changes[i - 1].before.end);
        if gap > context.saturating_mul(2) {
            out.push(&changes[start..i]);
            start = i;
        }
    }
    if start < changes.len() {
        out.push(&changes[start..]);
    }
    out
}

struct Sides<'a> {
    old: &'a [u8],
    new: &'a [u8],
    old_lines: &'a [Line],
    new_lines: &'a [Line],
}

fn build_hunk(sides: &Sides<'_>, changes: &[Change], context: u32) -> DiffHunk {
    let (Some(first), Some(last)) = (changes.first(), changes.last()) else {
        return DiffHunk {
            header: Hunk {
                old_start: 0,
                old_lines: 0,
                new_start: 0,
                new_lines: 0,
            },
            lines: Vec::new(),
        };
    };
    let old_len = to_u32(sides.old_lines.len());
    let new_len = to_u32(sides.new_lines.len());
    let lead = context.min(first.before.start).min(first.after.start);
    let old_start0 = first.before.start - lead;
    let new_start0 = first.after.start - lead;
    let trail = context
        .min(old_len.saturating_sub(last.before.end))
        .min(new_len.saturating_sub(last.after.end));
    let old_end0 = last.before.end + trail;
    let new_end0 = last.after.end + trail;

    let mut lines = Vec::new();
    let (mut o, mut n) = (old_start0, new_start0);
    for change in changes {
        while o < change.before.start && n < change.after.start {
            lines.push(context_line(sides, o, n));
            o += 1;
            n += 1;
        }
        for i in change.before.clone() {
            lines.push(side_line(sides, LineKind::Del, i));
        }
        for i in change.after.clone() {
            lines.push(side_line(sides, LineKind::Add, i));
        }
        o = change.before.end;
        n = change.after.end;
    }
    while o < old_end0 && n < new_end0 {
        lines.push(context_line(sides, o, n));
        o += 1;
        n += 1;
    }

    let old_lines = old_end0 - old_start0;
    let new_lines = new_end0 - new_start0;
    DiffHunk {
        header: Hunk {
            old_start: if old_lines == 0 {
                old_start0
            } else {
                old_start0 + 1
            },
            old_lines,
            new_start: if new_lines == 0 {
                new_start0
            } else {
                new_start0 + 1
            },
            new_lines,
        },
        lines,
    }
}

fn context_line(sides: &Sides<'_>, o: u32, n: u32) -> HunkLine {
    let (span, no_eol, trivia) = line_info(sides.new, sides.new_lines, n);
    HunkLine {
        kind: LineKind::Context,
        old_no: Some(o + 1),
        new_no: Some(n + 1),
        span,
        no_eol,
        trivia,
    }
}

fn side_line(sides: &Sides<'_>, kind: LineKind, i: u32) -> HunkLine {
    let (buf, lines) = match kind {
        LineKind::Del => (sides.old, sides.old_lines),
        LineKind::Add | LineKind::Context => (sides.new, sides.new_lines),
    };
    let (span, no_eol, trivia) = line_info(buf, lines, i);
    HunkLine {
        kind,
        old_no: (kind == LineKind::Del).then_some(i + 1),
        new_no: (kind == LineKind::Add).then_some(i + 1),
        span,
        no_eol,
        trivia,
    }
}

fn line_info(buf: &[u8], lines: &[Line], i: u32) -> (Range<u32>, bool, bool) {
    match lines.get(i as usize) {
        Some(line) => {
            let start = (line.span.start as usize).min(buf.len());
            let end = (line.span.end as usize).min(buf.len());
            (
                line.span.clone(),
                line.no_eol,
                is_trivia(&buf[start..end.max(start)]),
            )
        }
        None => (0..0, false, true),
    }
}

/// The changed old-side and new-side lines of already computed hunks, as zero-context ranges.
/// Equivalent to [`HunkSet::changed_old`]/[`HunkSet::changed_new`] for the same diff.
pub fn changed_ranges(hunks: &[DiffHunk]) -> (Vec<Range<u32>>, Vec<Range<u32>>) {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for hunk in hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Del => {
                    if let Some(no) = line.old_no {
                        old.push(no..no + 1);
                    }
                }
                LineKind::Add => {
                    if let Some(no) = line.new_no {
                        new.push(no..no + 1);
                    }
                }
                LineKind::Context => {}
            }
        }
    }
    (merge_ranges(old), merge_ranges(new))
}

/// Apply `hunks` to `old`, producing the new buffer (used by the reconstruction property).
pub fn apply_hunks(old: &[u8], new: &[u8], hunks: &[DiffHunk]) -> Vec<u8> {
    let old_lines = split_lines(old);
    let mut out = Vec::with_capacity(new.len());
    let mut next_old = 0usize;
    for hunk in hunks {
        let hunk_old_start = if hunk.header.old_lines == 0 {
            hunk.header.old_start as usize
        } else {
            hunk.header.old_start as usize - 1
        };
        for line in old_lines.iter().take(hunk_old_start).skip(next_old) {
            out.extend_from_slice(slice(old, &line.span));
        }
        next_old = next_old.max(hunk_old_start);
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context | LineKind::Add => {
                    out.extend_from_slice(slice(new, &line.span));
                }
                LineKind::Del => {}
            }
            if line.kind != LineKind::Add {
                next_old += 1;
            }
        }
    }
    for line in old_lines.iter().skip(next_old) {
        out.extend_from_slice(slice(old, &line.span));
    }
    out
}

fn slice<'a>(buf: &'a [u8], span: &Range<u32>) -> &'a [u8] {
    let start = (span.start as usize).min(buf.len());
    let end = (span.end as usize).min(buf.len()).max(start);
    &buf[start..end]
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn merge_ranges_joins_touching() {
        assert_eq!(merge_ranges(vec![1..3, 3..5, 7..8]), vec![1..5, 7..8]);
    }

    #[test]
    fn identical_inputs_have_no_hunks() {
        let set = compute_hunks(b"a\nb\n", b"a\nb\n", &HunkOptions::default()).unwrap_or_default();
        assert!(set.hunks.is_empty());
        assert_eq!(set.stats, LineStats::default());
    }
}
