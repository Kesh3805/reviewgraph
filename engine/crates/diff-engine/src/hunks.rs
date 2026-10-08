//! Line-diff model types (DIFF-003 computes hunks; DIFF-002 only defines the shapes that
//! [`crate::model::FileDiff`] embeds).

use std::ops::Range;

use review_core::change::Hunk;

/// Line statistics of one diffed file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineStats {
    /// Added lines.
    pub additions: u32,
    /// Deleted lines.
    pub deletions: u32,
}

/// Kind of a line within a [`DiffHunk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Present on both sides.
    Context,
    /// Present only on the new side.
    Add,
    /// Present only on the old side.
    Del,
}

/// One line inside a hunk. `span` addresses the side's own buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// A unified-diff hunk with full line detail (the header mirrors the DOM-005 [`Hunk`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHunk {
    /// Unified header semantics: `@@ -old_start,old_lines +new_start,new_lines @@`.
    pub header: Hunk,
    /// Lines of the hunk in side order.
    pub lines: Vec<HunkLine>,
}
