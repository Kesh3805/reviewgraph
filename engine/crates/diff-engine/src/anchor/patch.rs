//! Structural parser for provider patch text (DIFF-005).
//!
//! Provider patches are untrusted data: they are parsed structurally into line sets, never
//! interpreted, never logged, and bounded in size. Any inconsistency (bad header numbers, a body
//! that does not match its header counts, unknown line prefixes) rejects the whole patch.

use review_core::change::Hunk;

use super::lineset::LineSet;

/// Patches with more lines than this are rejected.
pub const MAX_PATCH_LINES: usize = 200_000;

/// The anchorable lines a provider patch defines.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedPatch {
    /// Hunk headers in order.
    pub hunks: Vec<Hunk>,
    /// New-side lines a comment may target (context + added).
    pub right: LineSet,
    /// Old-side lines a comment may target (context + deleted).
    pub left: LineSet,
    /// Added lines.
    pub additions: u32,
    /// Deleted lines.
    pub deletions: u32,
}

/// Why a patch was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    /// More than [`MAX_PATCH_LINES`] lines.
    #[error("patch exceeds {MAX_PATCH_LINES} lines")]
    TooLarge,
    /// A hunk header could not be parsed or has impossible numbers.
    #[error("malformed hunk header at patch line {0}")]
    BadHeader(usize),
    /// A body line has an unknown prefix or the body disagrees with its header counts.
    #[error("malformed hunk body at patch line {0}")]
    BadBody(usize),
}

/// Parse `@@ -a,b +c,d @@` hunks and their bodies. Lines before the first hunk (`diff --git`,
/// `index`, `---`, `+++`) are skipped.
pub fn parse_patch(text: &str) -> Result<ParsedPatch, PatchError> {
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.len() > MAX_PATCH_LINES {
        return Err(PatchError::TooLarge);
    }
    let mut out = ParsedPatch::default();
    let mut right: Vec<u32> = Vec::new();
    let mut left: Vec<u32> = Vec::new();
    let mut i = 0usize;
    // Skip the preamble.
    while i < lines.len() && !lines[i].starts_with("@@") {
        i += 1;
    }
    while i < lines.len() {
        let line = lines[i];
        if line.is_empty() && i + 1 == lines.len() {
            break; // trailing newline of the text
        }
        let header = parse_header(line).ok_or(PatchError::BadHeader(i + 1))?;
        out.hunks.push(header);
        i += 1;
        let (mut old_left, mut new_left) = (header.old_lines, header.new_lines);
        let (mut o, mut n) = (header.old_start.max(1), header.new_start.max(1));
        while old_left > 0 || new_left > 0 {
            let Some(body) = lines.get(i) else {
                return Err(PatchError::BadBody(i + 1));
            };
            let last = i + 1 == lines.len();
            let tag = match body.as_bytes().first() {
                Some(b'+') => "+",
                Some(b'-') => "-",
                Some(b' ') => " ",
                Some(b'\\') => "\\",
                // The final empty element is the text's trailing newline, not a line.
                None if !last => "",
                _ => return Err(PatchError::BadBody(i + 1)),
            };
            match tag {
                "+" => {
                    if new_left == 0 {
                        return Err(PatchError::BadBody(i + 1));
                    }
                    right.push(n);
                    n += 1;
                    new_left -= 1;
                    out.additions += 1;
                }
                "-" => {
                    if old_left == 0 {
                        return Err(PatchError::BadBody(i + 1));
                    }
                    left.push(o);
                    o += 1;
                    old_left -= 1;
                    out.deletions += 1;
                }
                // An empty line is a context line whose single space was stripped.
                " " | "" => {
                    if old_left == 0 || new_left == 0 {
                        return Err(PatchError::BadBody(i + 1));
                    }
                    left.push(o);
                    right.push(n);
                    o += 1;
                    n += 1;
                    old_left -= 1;
                    new_left -= 1;
                }
                "\\" => {}
                _ => return Err(PatchError::BadBody(i + 1)),
            }
            i += 1;
        }
        // `\ No newline at end of file` markers after the last body line.
        while lines.get(i).is_some_and(|l| l.starts_with('\\')) {
            i += 1;
        }
        // Anything that is neither a new hunk nor the end is malformed.
        if let Some(next) = lines.get(i) {
            if !next.starts_with("@@") && !(next.is_empty() && i + 1 == lines.len()) {
                return Err(PatchError::BadBody(i + 1));
            }
        }
    }
    out.right = LineSet::from_lines(right);
    out.left = LineSet::from_lines(left);
    Ok(out)
}

/// `@@ -a[,b] +c[,d] @@[ context]`.
fn parse_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@ -")?;
    let end = rest.find(" @@")?;
    let ranges = &rest[..end];
    let (old, new) = ranges.split_once(" +")?;
    let (old_start, old_lines) = parse_range(old)?;
    let (new_start, new_lines) = parse_range(new)?;
    // A non-empty range starts at line 1 or later.
    if (old_lines > 0 && old_start == 0) || (new_lines > 0 && new_start == 0) {
        return None;
    }
    let limit = MAX_PATCH_LINES as u32 * 16;
    if old_start > limit || new_start > limit || old_lines > limit || new_lines > limit {
        return None;
    }
    Some(Hunk {
        old_start,
        old_lines,
        new_start,
        new_lines,
    })
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    let (start, count) = match s.split_once(',') {
        Some((a, b)) => (a, b),
        None => (s, "1"),
    };
    let digits = |t: &str| !t.is_empty() && t.len() <= 9 && t.bytes().all(|b| b.is_ascii_digit());
    if !digits(start) || !digits(count) {
        return None;
    }
    Some((start.parse().ok()?, count.parse().ok()?))
}

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_style_patch() {
        let patch = "@@ -7,6 +7,6 @@ export class A {\n   a\n \n   b\n-    old\n+    new\n   }\n }";
        let parsed = parse_patch(patch);
        assert!(parsed.is_ok(), "{parsed:?}");
        let parsed = parsed.unwrap_or_default();
        assert_eq!(parsed.right.ranges(), &[7..13]);
        assert_eq!(parsed.left.ranges(), &[7..13]);
        assert_eq!((parsed.additions, parsed.deletions), (1, 1));
    }

    #[test]
    fn rejects_count_mismatch() {
        assert!(parse_patch("@@ -1,2 +1,2 @@\n a\n").is_err());
        assert!(parse_patch("@@ -x +1 @@\n a\n").is_err());
    }
}
