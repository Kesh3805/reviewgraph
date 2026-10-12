//! The history ingest driver (HIST-001).
//!
//! Pages through a repository's closed and merged pull requests via a [`HistoryProvider`],
//! normalizes them and commits one page at a time through a [`HistoryStore`] together with the
//! cursor, so a crash or a rate limit resumes from the last fully committed page. Every write is
//! an upsert keyed by provider ids: re-running changes nothing.
//!
//! Provider contract: pull requests are closed or merged, ordered by `closed_at` descending, and
//! each carries its review comments. A provider fetches comments with at most
//! [`MAX_PROVIDER_CONCURRENCY`] requests in flight.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use uuid::Uuid;

use super::link::SymbolResolver;
use super::normalize::{
    default_redact, normalize_comment, normalize_pr, HistoryComment, HistoryPr, NormalizeSettings,
    RawPullRequest,
};

/// Backfill window on onboarding (12 months).
pub const DEFAULT_WINDOW_DAYS: i64 = 365;
/// Backfill ceiling on onboarding.
pub const DEFAULT_MAX_PRS: u32 = 2000;
pub const DEFAULT_PAGE_SIZE: u32 = 50;
/// Bound on concurrent comment fetches inside a provider page.
pub const MAX_PROVIDER_CONCURRENCY: usize = 4;
/// Incremental runs re-read this many days before the last seen pull request, so late review
/// comments on recent pull requests are picked up (the upserts make the overlap free).
pub const INCREMENTAL_OVERLAP_DAYS: i64 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestMode {
    Backfill,
    Incremental,
}

impl IngestMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Backfill => "backfill",
            Self::Incremental => "incremental",
        }
    }
}

#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// Per-organization salt for identity hashes.
    pub salt: String,
    /// `history.enabled`: comment bodies are stored only when true.
    pub store_bodies: bool,
    pub redact: fn(&str) -> String,
    pub window_days: i64,
    pub max_prs: u32,
    pub page_size: u32,
}

impl IngestConfig {
    pub fn new(salt: impl Into<String>, store_bodies: bool) -> Self {
        Self {
            salt: salt.into(),
            store_bodies,
            redact: default_redact,
            window_days: DEFAULT_WINDOW_DAYS,
            max_prs: DEFAULT_MAX_PRS,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

/// Why the provider could not serve a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderFailure {
    RateLimited {
        retry_after: Duration,
    },
    /// The installation may not read history; the repository is skipped cleanly.
    Forbidden,
    /// 404, revoked access and the like: no retry.
    Permanent {
        reason: String,
    },
    Transient {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPage {
    pub prs: Vec<RawPullRequest>,
    /// Opaque token of the next page; `None` on the last page.
    pub next_cursor: Option<String>,
}

#[async_trait]
pub trait HistoryProvider: Send + Sync {
    async fn list_closed_prs(
        &self,
        cursor: Option<&str>,
        per_page: u32,
    ) -> Result<ProviderPage, ProviderFailure>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("history store: {0}")]
pub struct StoreError(pub String);

/// Persisted progress of one repository's ingest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorState {
    pub cursor: Option<String>,
    /// Newest `closed_at` seen.
    pub last_pr_updated_at: Option<DateTime<Utc>>,
    pub prs_ingested: u32,
    /// The last run reached the end of its window.
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPr {
    pub pr: HistoryPr,
    pub comments: Vec<HistoryComment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NormalizedPage {
    pub prs: Vec<NormalizedPr>,
}

/// Rows a committed page inserted or actually modified.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageCounts {
    pub prs_changed: u32,
    pub comments_changed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestStatus {
    Running,
    Completed,
    Failed,
    Skipped,
}

impl IngestStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// Storage of one repository's history (the repository is bound to the store instance).
#[async_trait]
pub trait HistoryStore: Send + Sync {
    async fn load_cursor(&self) -> Result<CursorState, StoreError>;

    /// The published finding that was posted as this provider comment, if any.
    async fn published_finding_for_comment(
        &self,
        provider_comment_id: &str,
    ) -> Result<Option<Uuid>, StoreError>;

    /// Upserts the page and stores `next` atomically.
    async fn commit_page(
        &self,
        page: &NormalizedPage,
        next: &CursorState,
    ) -> Result<PageCounts, StoreError>;

    async fn set_status(
        &self,
        status: IngestStatus,
        reason: Option<&str>,
    ) -> Result<(), StoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    Completed,
    /// Rate limited: resume from the stored cursor after `retry_after`.
    Paused {
        retry_after: Duration,
    },
    Transient {
        message: String,
    },
    Failed {
        reason: String,
    },
    Skipped {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestReport {
    pub outcome: IngestOutcome,
    pub prs_ingested: u32,
    pub comments_ingested: u32,
    pub rows_changed: u32,
}

/// The publication time of a pull request in history: its close time.
fn ordering_key(raw: &RawPullRequest) -> DateTime<Utc> {
    raw.closed_at.unwrap_or(raw.updated_at)
}

/// Runs (or resumes) one ingest. Provider failures become an [`IngestOutcome`]; only store
/// failures are errors.
pub async fn run_ingest(
    provider: &dyn HistoryProvider,
    store: &dyn HistoryStore,
    resolver: Option<&dyn SymbolResolver>,
    cfg: &IngestConfig,
    mode: IngestMode,
    now: DateTime<Utc>,
) -> Result<IngestReport, StoreError> {
    let cutoff = now - ChronoDuration::days(cfg.window_days);
    let mut state = store.load_cursor().await?;
    // An unfinished run resumes from its cursor whichever mode asked for it.
    let incremental = mode == IngestMode::Incremental && state.completed;
    let since = if incremental {
        state
            .last_pr_updated_at
            .map(|t| t - ChronoDuration::days(INCREMENTAL_OVERLAP_DAYS))
    } else {
        None
    };
    if state.completed {
        state.cursor = None;
        state.prs_ingested = 0;
        state.completed = false;
    }
    store.set_status(IngestStatus::Running, None).await?;

    let settings = NormalizeSettings {
        salt: &cfg.salt,
        store_bodies: cfg.store_bodies,
        redact: cfg.redact,
    };
    let mut report = IngestReport {
        outcome: IngestOutcome::Completed,
        prs_ingested: 0,
        comments_ingested: 0,
        rows_changed: 0,
    };

    loop {
        let page = match provider
            .list_closed_prs(state.cursor.as_deref(), cfg.page_size)
            .await
        {
            Ok(page) => page,
            Err(failure) => {
                report.outcome = match failure {
                    ProviderFailure::RateLimited { retry_after } => {
                        IngestOutcome::Paused { retry_after }
                    }
                    ProviderFailure::Transient { message } => IngestOutcome::Transient { message },
                    ProviderFailure::Forbidden => {
                        let reason = "no_history_permission".to_owned();
                        store
                            .set_status(IngestStatus::Skipped, Some(&reason))
                            .await?;
                        IngestOutcome::Skipped { reason }
                    }
                    ProviderFailure::Permanent { reason } => {
                        store
                            .set_status(IngestStatus::Failed, Some(&reason))
                            .await?;
                        IngestOutcome::Failed { reason }
                    }
                };
                return Ok(report);
            }
        };

        let mut reached_end = page.next_cursor.is_none();
        let mut newest = state.last_pr_updated_at;
        let mut normalized = Vec::with_capacity(page.prs.len());
        let mut ingested_after = state.prs_ingested;
        let mut comment_count = 0u32;
        for raw in &page.prs {
            let key = ordering_key(raw);
            let before_window = key < cutoff;
            let before_since = since.is_some_and(|s| key < s);
            if before_window || before_since || ingested_after >= cfg.max_prs {
                reached_end = true;
                break;
            }
            let mut comments = Vec::with_capacity(raw.comments.len());
            for c in &raw.comments {
                let finding = store
                    .published_finding_for_comment(&c.provider_comment_id)
                    .await?;
                comments.push(normalize_comment(&settings, c, finding, resolver));
            }
            comment_count += u32::try_from(comments.len()).unwrap_or(u32::MAX);
            normalized.push(NormalizedPr {
                pr: normalize_pr(&settings, raw),
                comments,
            });
            ingested_after += 1;
            newest = Some(newest.map_or(key, |n| n.max(key)));
        }

        let mut next = state.clone();
        next.prs_ingested = ingested_after;
        next.last_pr_updated_at = newest;
        if reached_end {
            next.cursor = None;
            next.completed = true;
        } else {
            next.cursor = page.next_cursor.clone();
        }
        let counts = store
            .commit_page(&NormalizedPage { prs: normalized }, &next)
            .await?;
        report.prs_ingested += ingested_after - state.prs_ingested;
        report.comments_ingested += comment_count;
        report.rows_changed += counts.prs_changed + counts.comments_changed;
        tracing::info!(
            prs = ingested_after - state.prs_ingested,
            comments = comment_count,
            mode = mode.as_str(),
            "history_ingest page committed"
        );
        state = next;
        if reached_end {
            break;
        }
    }
    store.set_status(IngestStatus::Completed, None).await?;
    Ok(report)
}
