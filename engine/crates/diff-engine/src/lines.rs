//! Line splitting for the in-process line diff (DIFF-003).
//!
//! Lines are split on `\n` only. A trailing `\r` stays part of the line, so a CRLF <-> LF
//! conversion is a real change unless the caller asks to ignore line endings. The last line of
//! a buffer without a trailing newline is flagged `no_eol` and compares unequal to the same text
//! followed by a newline, exactly like git.

use std::ops::Range;

/// One line of a buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Byte range of the line in its buffer, including the terminator.
    pub span: Range<u32>,
    /// The buffer ended without a newline after this line.
    pub no_eol: bool,
}

/// Every line of `buf`, in order. An empty buffer has no lines.
pub fn split_lines(buf: &[u8]) -> Vec<Line> {
    let mut out = Vec::with_capacity(buf.len() / 32 + 1);
    let mut start = 0usize;
    for (i, byte) in buf.iter().enumerate() {
        if *byte == b'\n' {
            out.push(Line {
                span: to_u32(start)..to_u32(i + 1),
                no_eol: false,
            });
            start = i + 1;
        }
    }
    if start < buf.len() {
        out.push(Line {
            span: to_u32(start)..to_u32(buf.len()),
            no_eol: true,
        });
    }
    out
}

/// Number of lines `split_lines` would return, without allocating.
pub fn count_lines(buf: &[u8]) -> u32 {
    let newlines = buf.iter().filter(|b| **b == b'\n').count();
    let trailing = usize::from(!buf.is_empty() && buf.last() != Some(&b'\n'));
    to_u32(newlines + trailing)
}

/// The comparison token of one line: its text without the terminator (and without a `\r`
/// before it when line endings are ignored), plus whether a terminator was present.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(crate) struct LineToken<'a> {
    pub text: &'a [u8],
    pub eol: bool,
}

impl AsRef<[u8]> for LineToken<'_> {
    fn as_ref(&self) -> &[u8] {
        self.text
    }
}

/// The token of `line` inside `buf`.
pub(crate) fn token<'a>(buf: &'a [u8], line: &Line, ignore_eol: bool) -> LineToken<'a> {
    let start = line.span.start as usize;
    let mut end = (line.span.end as usize).min(buf.len());
    let start = start.min(end);
    let eol = !line.no_eol;
    if eol && end > start && buf[end - 1] == b'\n' {
        end -= 1;
        if ignore_eol && end > start && buf[end - 1] == b'\r' {
            end -= 1;
        }
    }
    LineToken {
        text: &buf[start..end],
        eol,
    }
}

/// Whether a line is blank or looks like a comment (`//`, `/*`, `*`, `*/`, `#`, `<!--`).
///
/// This is a language-agnostic hint used to flag comment-only hunks early (DIFF-006
/// `touches_code`); the authoritative cosmetic decision compares symbol body hashes.
pub fn is_trivia(text: &[u8]) -> bool {
    let trimmed = trim_ascii(text);
    trimmed.is_empty()
        || trimmed.starts_with(b"//")
        || trimmed.starts_with(b"/*")
        || trimmed.starts_with(b"*")
        || trimmed.starts_with(b"#")
        || trimmed.starts_with(b"<!--")
}

fn trim_ascii(text: &[u8]) -> &[u8] {
    let start = text
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(text.len());
    let end = text
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &text[start..end.max(start)]
}

pub(crate) fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_newline_and_flags_missing_eol() {
        let lines = split_lines(b"a\nb\r\nc");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].span, 0..2);
        assert_eq!(lines[1].span, 2..5);
        assert_eq!(lines[2].span, 5..6);
        assert!(lines[2].no_eol);
        assert!(!lines[1].no_eol);
        assert_eq!(count_lines(b"a\nb\r\nc"), 3);
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"a\n"), 1);
    }

    #[test]
    fn tokens_respect_eol_mode() {
        let buf = b"a\r\na\n";
        let lines = split_lines(buf);
        assert_ne!(token(buf, &lines[0], false), token(buf, &lines[1], false));
        assert_eq!(token(buf, &lines[0], true), token(buf, &lines[1], true));
        let tail = b"a";
        let tail_lines = split_lines(tail);
        assert_ne!(
            token(tail, &tail_lines[0], true),
            token(buf, &lines[1], true)
        );
    }

    #[test]
    fn trivia_detection() {
        assert!(is_trivia(b"   // note\n"));
        assert!(is_trivia(b"\n"));
        assert!(is_trivia(b" * doc\n"));
        assert!(!is_trivia(b"  return x;\n"));
    }
}
