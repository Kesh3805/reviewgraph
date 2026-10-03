//! Source text helpers: BOM handling, lossy decoding and tree-sitter point conversion.
//!
//! tree-sitter works on bytes. Ranges in the IR use 1-based lines and 0-based UTF-8 byte
//! columns, and columns of the first line include the BOM so positions are positions in the
//! file, not in the parsed slice.

use std::borrow::Cow;

use review_core::location::{Position, SourceRange};
use tree_sitter::{Node, Point};

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The text handed to the parser plus what is needed to map back to file positions.
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    /// The parsed slice (BOM removed).
    pub bytes: &'a [u8],
    /// Length of the removed BOM (0 or 3).
    pub bom: u32,
}

impl<'a> Source<'a> {
    pub fn new(file: &'a [u8]) -> Self {
        match file.strip_prefix(BOM) {
            Some(rest) => Self {
                bytes: rest,
                bom: 3,
            },
            None => Self {
                bytes: file,
                bom: 0,
            },
        }
    }

    /// Whether the text is valid UTF-8. Invalid text is still analyzed (lossy decoding).
    pub fn is_valid_utf8(&self) -> bool {
        std::str::from_utf8(self.bytes).is_ok()
    }

    /// Number of newline characters plus one.
    pub fn line_count(&self) -> u32 {
        let newlines = self.bytes.iter().filter(|&&b| b == b'\n').count();
        u32::try_from(newlines + 1).unwrap_or(u32::MAX)
    }

    pub fn position(&self, point: Point) -> Position {
        let line = u32::try_from(point.row)
            .unwrap_or(u32::MAX - 1)
            .saturating_add(1);
        let mut column = u32::try_from(point.column).unwrap_or(u32::MAX);
        if point.row == 0 {
            column = column.saturating_add(self.bom);
        }
        Position { line, column }
    }

    pub fn range(&self, node: Node<'_>) -> SourceRange {
        let start = self.position(node.start_position());
        let end = self.position(node.end_position());
        // tree-sitter guarantees start <= end; fall back to an empty range otherwise.
        SourceRange::new(start, end).unwrap_or_else(|_| {
            SourceRange::new(start, start).unwrap_or_else(|_| single_point(start))
        })
    }

    /// Range of the whole text.
    pub fn whole(&self) -> SourceRange {
        let lines = self.line_count();
        let last_start = self
            .bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let mut column = u32::try_from(self.bytes.len() - last_start).unwrap_or(u32::MAX);
        if lines == 1 {
            column = column.saturating_add(self.bom);
        }
        let start = Position { line: 1, column: 0 };
        let end = Position {
            line: lines,
            column,
        };
        SourceRange::new(start, end).unwrap_or_else(|_| single_point(start))
    }

    /// The node's text, decoded lossily.
    pub fn text(&self, node: Node<'_>) -> Cow<'a, str> {
        self.slice(node.start_byte(), node.end_byte())
    }

    pub fn slice(&self, start: usize, end: usize) -> Cow<'a, str> {
        let end = end.min(self.bytes.len());
        let start = start.min(end);
        String::from_utf8_lossy(&self.bytes[start..end])
    }
}

fn single_point(at: Position) -> SourceRange {
    // `Position` pairs are always orderable, so this cannot fail; the helper exists to keep the
    // no-panic lints satisfied without an `expect`.
    SourceRange { start: at, end: at }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_is_stripped_and_counted() {
        let file = [0xEF, 0xBB, 0xBF, b'a', b'\n', b'b'];
        let src = Source::new(&file);
        assert_eq!(src.bom, 3);
        assert_eq!(src.bytes, b"a\nb");
        assert_eq!(src.line_count(), 2);
        let p = src.position(Point { row: 0, column: 1 });
        assert_eq!((p.line, p.column), (1, 4));
        let q = src.position(Point { row: 1, column: 0 });
        assert_eq!((q.line, q.column), (2, 0));
    }

    #[test]
    fn whole_range_covers_the_text() {
        let src = Source::new(b"ab\ncd\n");
        let r = src.whole();
        assert_eq!((r.start.line, r.end.line, r.end.column), (1, 3, 0));
        let empty = Source::new(b"");
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.whole().end.column, 0);
    }
}
