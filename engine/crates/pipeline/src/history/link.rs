//! Linking historical comments to symbols (HIST-001).
//!
//! A comment on `(path, line, commit_sha)` links to a symbol when the nearest indexed snapshot of
//! that commit has one at the line; otherwise only the path is kept and `link_status` says why.

use std::collections::BTreeMap;

use super::normalize::RawComment;

/// How far a comment could be linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// Mapped to a symbol key.
    Linked,
    /// The commit is not indexable or no symbol covers the line; the path is kept.
    PathOnly,
    /// A comment on the pull request as a whole: no path.
    Unlinked,
}

impl LinkStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::PathOnly => "path_only",
            Self::Unlinked => "unlinked",
        }
    }
}

/// Resolves a location to a symbol key using the nearest indexed snapshot.
pub trait SymbolResolver: Send + Sync {
    fn resolve(&self, path: &str, line: i32, commit_sha: &str) -> Option<String>;
}

pub fn link_comment(
    resolver: Option<&dyn SymbolResolver>,
    comment: &RawComment,
) -> (LinkStatus, Option<String>) {
    let Some(path) = comment.path.as_deref() else {
        return (LinkStatus::Unlinked, None);
    };
    let (Some(line), Some(sha), Some(resolver)) =
        (comment.line, comment.commit_sha.as_deref(), resolver)
    else {
        return (LinkStatus::PathOnly, None);
    };
    match resolver.resolve(path, line, sha) {
        Some(key) => (LinkStatus::Linked, Some(key)),
        None => (LinkStatus::PathOnly, None),
    }
}

/// A symbol's line range in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRange {
    pub symbol_key: String,
    pub start_line: i32,
    pub end_line: i32,
}

/// A resolver over symbol ranges loaded per `(commit, path)`; the narrowest range covering the
/// line wins (a method beats its class).
#[derive(Debug, Default, Clone)]
pub struct RangeSymbolResolver {
    ranges: BTreeMap<(String, String), Vec<SymbolRange>>,
}

impl RangeSymbolResolver {
    pub fn insert(&mut self, commit_sha: &str, path: &str, ranges: Vec<SymbolRange>) {
        self.ranges
            .insert((commit_sha.to_owned(), path.to_owned()), ranges);
    }
}

impl SymbolResolver for RangeSymbolResolver {
    fn resolve(&self, path: &str, line: i32, commit_sha: &str) -> Option<String> {
        self.ranges
            .get(&(commit_sha.to_owned(), path.to_owned()))?
            .iter()
            .filter(|r| r.start_line <= line && line <= r.end_line)
            .min_by_key(|r| (r.end_line - r.start_line, r.symbol_key.clone()))
            .map(|r| r.symbol_key.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn comment(path: Option<&str>, line: Option<i32>, sha: Option<&str>) -> RawComment {
        RawComment {
            provider_comment_id: "c1".into(),
            thread_id: None,
            path: path.map(str::to_owned),
            line,
            commit_sha: sha.map(str::to_owned),
            author_login: "a".into(),
            author_is_bot: false,
            body: String::new(),
            resolved: false,
            resolved_by_login: None,
            resolved_by_is_bot: false,
            created_at: Utc.timestamp_opt(0, 0).single().unwrap_or_default(),
        }
    }

    #[test]
    fn narrowest_symbol_wins_and_missing_index_keeps_path() {
        let mut r = RangeSymbolResolver::default();
        r.insert(
            "sha",
            "a.ts",
            vec![
                SymbolRange {
                    symbol_key: "Class".into(),
                    start_line: 1,
                    end_line: 50,
                },
                SymbolRange {
                    symbol_key: "Class.method".into(),
                    start_line: 10,
                    end_line: 20,
                },
            ],
        );
        let linked = link_comment(Some(&r), &comment(Some("a.ts"), Some(12), Some("sha")));
        assert_eq!(linked, (LinkStatus::Linked, Some("Class.method".into())));
        let other = link_comment(Some(&r), &comment(Some("a.ts"), Some(12), Some("other")));
        assert_eq!(other, (LinkStatus::PathOnly, None));
        let pr_level = link_comment(Some(&r), &comment(None, None, None));
        assert_eq!(pr_level, (LinkStatus::Unlinked, None));
        let no_resolver = link_comment(None, &comment(Some("a.ts"), Some(1), Some("sha")));
        assert_eq!(no_resolver, (LinkStatus::PathOnly, None));
    }
}
