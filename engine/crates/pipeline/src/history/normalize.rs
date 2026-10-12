//! Normalization of raw provider history (HIST-001).
//!
//! Author identities become salted hashes, so analytics tables never hold a login. Comment bodies
//! are redacted first and capped afterwards (a secret cut in half by the cap would otherwise
//! escape the redactor), and are kept only when the repository opted in.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::link::{link_comment, LinkStatus, SymbolResolver};

/// Stored comment bodies are capped at 4 KiB.
pub const MAX_BODY_BYTES: usize = 4096;

/// Who wrote a comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    Human,
    Bot,
    /// ReviewGraph's own comment, linked to a `published_findings` row by provider comment id.
    ReviewGraph,
}

impl AuthorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Bot => "bot",
            Self::ReviewGraph => "reviewgraph",
        }
    }
}

/// Pull request state in history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

/// Size statistics of a pull request (stored as `size_stats jsonb`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeStats {
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
}

/// A pull request as the provider reports it, with its review comments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPullRequest {
    pub provider_pr_id: String,
    pub number: i32,
    pub title: String,
    pub author_login: String,
    pub merged: bool,
    pub merge_commit: Option<String>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub size: SizeStats,
    pub comments: Vec<RawComment>,
}

/// A review comment as the provider reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawComment {
    pub provider_comment_id: String,
    pub thread_id: Option<String>,
    pub path: Option<String>,
    pub line: Option<i32>,
    pub commit_sha: Option<String>,
    pub author_login: String,
    pub author_is_bot: bool,
    pub body: String,
    pub resolved: bool,
    pub resolved_by_login: Option<String>,
    pub resolved_by_is_bot: bool,
    pub created_at: DateTime<Utc>,
}

/// A pull request in its stored shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPr {
    pub provider_pr_id: String,
    pub number: i32,
    pub title_hash: String,
    pub author_ref_hash: String,
    pub state: PrState,
    pub merged: bool,
    pub merge_commit: Option<String>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub size: SizeStats,
}

/// A review comment in its stored shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryComment {
    pub provider_comment_id: String,
    pub thread_id: Option<String>,
    pub path: Option<String>,
    pub line: Option<i32>,
    pub commit_sha: Option<String>,
    pub author_kind: AuthorKind,
    pub author_ref_hash: String,
    /// `None` unless the repository opted in.
    pub body_text_redacted: Option<String>,
    pub resolved: bool,
    pub resolved_by_kind: Option<AuthorKind>,
    pub link_status: LinkStatus,
    pub symbol_key: Option<String>,
    pub published_finding_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// Salted hash of a provider identity. The salt is per organization; the value is lower-cased so
/// the hash is stable across case differences.
pub fn hash_identity(salt: &str, value: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(salt.as_bytes());
    hasher.update(&[0]);
    hasher.update(value.to_lowercase().as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Cuts `text` to at most `max` bytes on a character boundary.
pub fn truncate_utf8(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The mandatory redaction layer (SEC-003) used for comment bodies.
pub fn default_redact(text: &str) -> String {
    model_gateway::redact::scrub_text(text)
}

/// Normalization settings of one repository.
#[derive(Debug, Clone, Copy)]
pub struct NormalizeSettings<'a> {
    pub salt: &'a str,
    /// `history.enabled` in the repository config; bodies are dropped when false.
    pub store_bodies: bool,
    pub redact: fn(&str) -> String,
}

pub fn normalize_pr(settings: &NormalizeSettings<'_>, raw: &RawPullRequest) -> HistoryPr {
    let state = if raw.merged {
        PrState::Merged
    } else if raw.closed_at.is_some() {
        PrState::Closed
    } else {
        PrState::Open
    };
    HistoryPr {
        provider_pr_id: raw.provider_pr_id.clone(),
        number: raw.number,
        title_hash: hash_identity(settings.salt, &raw.title),
        author_ref_hash: hash_identity(settings.salt, &raw.author_login),
        state,
        merged: raw.merged,
        merge_commit: raw.merge_commit.clone(),
        opened_at: raw.opened_at,
        closed_at: raw.closed_at,
        updated_at: raw.updated_at,
        size: raw.size,
    }
}

/// Normalizes one comment. `published_finding_id` is the finding published with this comment id
/// (ReviewGraph's own comment); the symbol link comes from `resolver` when the commit is indexed.
pub fn normalize_comment(
    settings: &NormalizeSettings<'_>,
    raw: &RawComment,
    published_finding_id: Option<Uuid>,
    resolver: Option<&dyn SymbolResolver>,
) -> HistoryComment {
    let author_kind = if published_finding_id.is_some() {
        AuthorKind::ReviewGraph
    } else if raw.author_is_bot {
        AuthorKind::Bot
    } else {
        AuthorKind::Human
    };
    let body_text_redacted = settings.store_bodies.then(|| {
        let redacted = (settings.redact)(&raw.body);
        truncate_utf8(&redacted, MAX_BODY_BYTES).to_owned()
    });
    let (link_status, symbol_key) = link_comment(resolver, raw);
    HistoryComment {
        provider_comment_id: raw.provider_comment_id.clone(),
        thread_id: raw.thread_id.clone(),
        path: raw.path.clone(),
        line: raw.line,
        commit_sha: raw.commit_sha.clone(),
        author_kind,
        author_ref_hash: hash_identity(settings.salt, &raw.author_login),
        body_text_redacted,
        resolved: raw.resolved,
        resolved_by_kind: raw.resolved_by_login.as_ref().map(|_| {
            if raw.resolved_by_is_bot {
                AuthorKind::Bot
            } else {
                AuthorKind::Human
            }
        }),
        link_status,
        symbol_key,
        published_finding_id,
        created_at: raw.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_salted_and_case_insensitive() {
        assert_eq!(hash_identity("s", "Alice"), hash_identity("s", "alice"));
        assert_ne!(hash_identity("s1", "alice"), hash_identity("s2", "alice"));
        assert!(!hash_identity("s", "alice").contains("alice"));
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        let text = "é".repeat(10);
        let cut = truncate_utf8(&text, 5);
        assert_eq!(cut.len(), 4);
        assert_eq!(truncate_utf8("abc", 10), "abc");
    }
}
