//! Doc units: markdown, ADRs, READMEs and rule documents split on H1-H3 headings.

use std::collections::BTreeMap;

use review_core::ids::{RepositoryId, SnapshotId};
use review_core::language::Language;
use review_core::location::RepoPath;

use super::{derived_key, finish, truncate_chars, EmbeddingUnit, UnitKind, UnitMeta};

/// Cap on doc unit text.
pub const DOC_MAX_CHARS: usize = 2_000;

/// A document discovered by INIT-010 or a `.review/` knowledge source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocInput {
    pub path: RepoPath,
    pub text: String,
}

#[derive(Debug)]
struct Section {
    heading_path: String,
    lines: Vec<(u32, String)>,
}

fn heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.bytes().take_while(|b| *b == b'#').count();
    if !(1..=3).contains(&level) {
        return None;
    }
    let rest = &trimmed[level..];
    if !rest.starts_with(' ') && !rest.is_empty() {
        return None;
    }
    let title = rest.trim().trim_end_matches('#').trim().to_owned();
    Some((level, title))
}

fn sections(text: &str) -> Vec<Section> {
    let mut out = vec![Section {
        heading_path: String::new(),
        lines: Vec::new(),
    }];
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut fence: Option<&str> = None;
    for (i, line) in text.lines().enumerate() {
        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let t = line.trim_start();
        let marker = ["```", "~~~"].into_iter().find(|m| t.starts_with(m));
        match (fence, marker) {
            (None, Some(m)) => fence = Some(m),
            (Some(open), Some(m)) if open == m => fence = None,
            _ => {}
        }
        if fence.is_none() && marker.is_none() {
            if let Some((level, title)) = heading(line) {
                stack.retain(|(l, _)| *l < level);
                stack.push((level, title));
                out.push(Section {
                    heading_path: stack
                        .iter()
                        .map(|(_, t)| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" > "),
                    lines: Vec::new(),
                });
                continue;
            }
        }
        if let Some(s) = out.last_mut() {
            s.lines.push((n, line.to_owned()));
        }
    }
    out
}

/// Splits a section's lines into pieces whose text (header included) fits the cap.
fn pieces(header: &str, lines: &[(u32, String)]) -> Vec<(u32, u32, String)> {
    let budget = DOC_MAX_CHARS
        .saturating_sub(header.chars().count() + 1)
        .max(1);
    let mut out = Vec::new();
    let mut current: Vec<&(u32, String)> = Vec::new();
    let mut size = 0usize;
    let flush = |current: &mut Vec<&(u32, String)>, out: &mut Vec<(u32, u32, String)>| {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            let body = current
                .iter()
                .map(|(_, l)| l.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            out.push((first.0, last.0, truncate_chars(&body, budget)));
        }
        current.clear();
    };
    for line in lines {
        let len = line.1.chars().count() + 1;
        if !current.is_empty() && size + len > budget {
            flush(&mut current, &mut out);
            size = 0;
        }
        current.push(line);
        size += len;
    }
    flush(&mut current, &mut out);
    out
}

/// Doc units of one document, in document order.
pub fn doc_units(
    doc: &DocInput,
    repository_id: RepositoryId,
    snapshot_id: SnapshotId,
) -> Vec<EmbeddingUnit> {
    let mut ordinals: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = Vec::new();
    for section in sections(&doc.text) {
        if section.lines.iter().all(|(_, l)| l.trim().is_empty()) {
            continue;
        }
        // Drop leading and trailing blank lines.
        let start = section
            .lines
            .iter()
            .position(|(_, l)| !l.trim().is_empty())
            .unwrap_or(0);
        let end = section
            .lines
            .iter()
            .rposition(|(_, l)| !l.trim().is_empty())
            .map_or(section.lines.len(), |i| i + 1);
        let lines = &section.lines[start..end];
        for (first, last, body) in pieces(&section.heading_path, lines) {
            let ordinal = ordinals.entry(section.heading_path.clone()).or_insert(0);
            let key = derived_key(
                "semantic-doc",
                &[
                    doc.path.as_str(),
                    &section.heading_path,
                    &ordinal.to_string(),
                ],
            );
            *ordinal += 1;
            let text = if section.heading_path.is_empty() {
                body
            } else {
                format!("{}\n{body}", section.heading_path)
            };
            let meta = UnitMeta {
                kind: UnitKind::Doc,
                key,
                repository_id,
                snapshot_id,
                language: Some(Language::Markdown),
                module: None,
                file_path: Some(doc.path.clone()),
                start_line: Some(first),
                end_line: Some(last),
                symbol_key: None,
            };
            out.push(finish(meta, &text, DOC_MAX_CHARS));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_inside_fences_are_ignored() {
        let s = sections("# A\n```\n# not a heading\n```\n## B\ntext");
        let paths: Vec<&str> = s.iter().map(|s| s.heading_path.as_str()).collect();
        assert_eq!(paths, vec!["", "A", "A > B"]);
    }

    #[test]
    fn h4_is_content() {
        assert!(heading("#### deep").is_none());
        assert_eq!(heading("## Title ##"), Some((2, "Title".to_owned())));
        assert!(heading("#hashtag").is_none());
    }
}
