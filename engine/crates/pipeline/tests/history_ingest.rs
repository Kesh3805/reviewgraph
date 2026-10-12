//! HIST-001 against a fake provider and an in-memory store.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use pipeline::history::{
    run_ingest, AuthorKind, CursorState, HistoryComment, HistoryPr, HistoryProvider, HistoryStore,
    IngestConfig, IngestMode, IngestOutcome, IngestStatus, LinkStatus, NormalizedPage, PageCounts,
    ProviderFailure, ProviderPage, RawComment, RawPullRequest, SizeStats, StoreError,
};
use uuid::Uuid;

const SECRET: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).single().unwrap()
}

fn fixture(count: usize) -> Vec<RawPullRequest> {
    (0..count)
        .map(|i| {
            let closed = now() - ChronoDuration::hours(i as i64 + 1);
            let comment = |k: usize| RawComment {
                provider_comment_id: format!("c{i}-{k}"),
                thread_id: Some(format!("t{i}-{k}")),
                path: Some("src/a.ts".into()),
                line: Some(10 + k as i32),
                commit_sha: Some("a".repeat(40)),
                author_login: format!("Reviewer{}", i % 7),
                author_is_bot: k == 1,
                body: format!("please look at this, token {SECRET}"),
                resolved: k == 0,
                resolved_by_login: (k == 0).then(|| "maintainer".to_owned()),
                resolved_by_is_bot: false,
                created_at: closed - ChronoDuration::minutes(5),
            };
            RawPullRequest {
                provider_pr_id: format!("pr-{i}"),
                number: i as i32 + 1,
                title: format!("change {i}"),
                author_login: format!("Author{}", i % 5),
                merged: i % 3 != 0,
                merge_commit: (i % 3 != 0).then(|| "b".repeat(40)),
                opened_at: closed - ChronoDuration::hours(2),
                closed_at: Some(closed),
                updated_at: closed,
                size: SizeStats {
                    additions: 10,
                    deletions: 2,
                    changed_files: 1,
                },
                comments: vec![comment(0), comment(1)],
            }
        })
        .collect()
}

struct FakeProvider {
    prs: Vec<RawPullRequest>,
    /// `(page number, failure)`: fails once when that page is first requested.
    failure: Mutex<Option<(usize, ProviderFailure)>>,
}

impl FakeProvider {
    fn new(prs: Vec<RawPullRequest>) -> Self {
        Self {
            prs,
            failure: Mutex::new(None),
        }
    }

    fn failing_once(self, page: usize, failure: ProviderFailure) -> Self {
        *self.failure.lock().unwrap() = Some((page, failure));
        self
    }
}

#[async_trait]
impl HistoryProvider for FakeProvider {
    async fn list_closed_prs(
        &self,
        cursor: Option<&str>,
        per_page: u32,
    ) -> Result<ProviderPage, ProviderFailure> {
        let offset: usize = cursor.map_or(0, |c| c.parse().unwrap());
        let per_page = per_page as usize;
        let page_no = offset / per_page;
        {
            let mut guard = self.failure.lock().unwrap();
            if guard.as_ref().is_some_and(|(p, _)| *p == page_no) {
                if let Some((_, failure)) = guard.take() {
                    return Err(failure);
                }
            }
        }
        let end = (offset + per_page).min(self.prs.len());
        Ok(ProviderPage {
            prs: self.prs[offset..end].to_vec(),
            next_cursor: (end < self.prs.len()).then(|| end.to_string()),
        })
    }
}

#[derive(Default)]
struct Inner {
    cursor: CursorState,
    prs: BTreeMap<String, HistoryPr>,
    comments: BTreeMap<(String, String), HistoryComment>,
    findings: BTreeMap<String, Uuid>,
    status: Option<(IngestStatus, Option<String>)>,
    commits: usize,
}

#[derive(Default)]
struct FakeStore {
    inner: Mutex<Inner>,
    /// Fails the commit with this 1-based index (simulates a killed worker).
    kill_at_commit: Mutex<Option<usize>>,
}

impl FakeStore {
    fn snapshot(
        &self,
    ) -> (
        BTreeMap<String, HistoryPr>,
        BTreeMap<(String, String), HistoryComment>,
    ) {
        let inner = self.inner.lock().unwrap();
        (inner.prs.clone(), inner.comments.clone())
    }
}

#[async_trait]
impl HistoryStore for FakeStore {
    async fn load_cursor(&self) -> Result<CursorState, StoreError> {
        Ok(self.inner.lock().unwrap().cursor.clone())
    }

    async fn published_finding_for_comment(
        &self,
        provider_comment_id: &str,
    ) -> Result<Option<Uuid>, StoreError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .findings
            .get(provider_comment_id)
            .copied())
    }

    async fn commit_page(
        &self,
        page: &NormalizedPage,
        next: &CursorState,
    ) -> Result<PageCounts, StoreError> {
        let mut inner = self.inner.lock().unwrap();
        inner.commits += 1;
        if *self.kill_at_commit.lock().unwrap() == Some(inner.commits) {
            return Err(StoreError("killed".into()));
        }
        let mut counts = PageCounts::default();
        for entry in &page.prs {
            let key = entry.pr.provider_pr_id.clone();
            if inner.prs.get(&key) != Some(&entry.pr) {
                inner.prs.insert(key.clone(), entry.pr.clone());
                counts.prs_changed += 1;
            }
            for c in &entry.comments {
                let ck = (key.clone(), c.provider_comment_id.clone());
                if inner.comments.get(&ck) != Some(c) {
                    inner.comments.insert(ck, c.clone());
                    counts.comments_changed += 1;
                }
            }
        }
        inner.cursor = next.clone();
        Ok(counts)
    }

    async fn set_status(
        &self,
        status: IngestStatus,
        reason: Option<&str>,
    ) -> Result<(), StoreError> {
        self.inner.lock().unwrap().status = Some((status, reason.map(str::to_owned)));
        Ok(())
    }
}

fn cfg(store_bodies: bool) -> IngestConfig {
    let mut cfg = IngestConfig::new("org-salt", store_bodies);
    cfg.page_size = 50;
    cfg
}

async fn run(
    provider: &FakeProvider,
    store: &FakeStore,
    cfg: &IngestConfig,
    mode: IngestMode,
) -> pipeline::history::IngestReport {
    run_ingest(provider, store, None, cfg, mode, now())
        .await
        .unwrap()
}

#[tokio::test]
async fn ingest_500_prs_has_exact_row_counts_and_second_run_changes_nothing() {
    let provider = FakeProvider::new(fixture(500));
    let store = FakeStore::default();
    let first = run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(first.outcome, IngestOutcome::Completed);
    assert_eq!(first.prs_ingested, 500);
    assert_eq!(first.comments_ingested, 1000);
    assert_eq!(first.rows_changed, 1500);
    let (prs, comments) = store.snapshot();
    assert_eq!((prs.len(), comments.len()), (500, 1000));

    let second = run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(second.outcome, IngestOutcome::Completed);
    assert_eq!(
        second.rows_changed, 0,
        "re-running a backfill changes nothing"
    );
    assert_eq!(store.snapshot(), (prs, comments));
}

#[tokio::test]
async fn reingest_is_idempotent_incrementally_too() {
    let provider = FakeProvider::new(fixture(120));
    let store = FakeStore::default();
    run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    let before = store.snapshot();
    let report = run(&provider, &store, &cfg(true), IngestMode::Incremental).await;
    assert_eq!(report.rows_changed, 0);
    assert_eq!(store.snapshot(), before);
}

#[tokio::test]
async fn backfill_resumes_from_cursor_after_failure() {
    let provider = FakeProvider::new(fixture(500));
    let clean = FakeStore::default();
    run(&provider, &clean, &cfg(true), IngestMode::Backfill).await;

    let store = FakeStore::default();
    *store.kill_at_commit.lock().unwrap() = Some(4);
    let killed = run_ingest(
        &provider,
        &store,
        None,
        &cfg(true),
        IngestMode::Backfill,
        now(),
    )
    .await;
    assert!(killed.is_err());
    let cursor = store.load_cursor().await.unwrap();
    assert_eq!(
        cursor.prs_ingested, 150,
        "cursor stays at the last committed page"
    );
    assert_eq!(cursor.cursor.as_deref(), Some("150"));

    *store.kill_at_commit.lock().unwrap() = None;
    let resumed = run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(resumed.outcome, IngestOutcome::Completed);
    assert_eq!(
        resumed.prs_ingested, 350,
        "only the remaining pages are fetched"
    );
    assert_eq!(store.snapshot(), clean.snapshot());
}

#[tokio::test]
async fn rate_limit_pauses_and_resumes() {
    let provider = FakeProvider::new(fixture(200)).failing_once(
        2,
        ProviderFailure::RateLimited {
            retry_after: Duration::from_secs(30),
        },
    );
    let store = FakeStore::default();
    let paused = run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(
        paused.outcome,
        IngestOutcome::Paused {
            retry_after: Duration::from_secs(30)
        }
    );
    assert_eq!(paused.prs_ingested, 100);
    let resumed = run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(resumed.outcome, IngestOutcome::Completed);
    assert_eq!(resumed.prs_ingested, 100);
    assert_eq!(store.snapshot().0.len(), 200);
}

#[tokio::test]
async fn author_identity_hashed_not_stored_raw() {
    let provider = FakeProvider::new(fixture(10));
    let store = FakeStore::default();
    run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    let (prs, comments) = store.snapshot();
    for pr in prs.values() {
        let text = format!("{pr:?}");
        assert!(!text.contains("Author"), "{text}");
        assert!(pr.author_ref_hash.len() == 64);
    }
    for c in comments.values() {
        let text = format!("{c:?}");
        assert!(
            !text.contains("Reviewer") && !text.contains("maintainer"),
            "{text}"
        );
    }
    // Same person, same hash; a different salt gives a different hash.
    let a = &comments[&("pr-0".to_owned(), "c0-0".to_owned())];
    let b = &comments[&("pr-7".to_owned(), "c7-0".to_owned())];
    assert_eq!(a.author_ref_hash, b.author_ref_hash);
}

#[tokio::test]
async fn bot_comment_linked_to_published_finding() {
    let provider = FakeProvider::new(fixture(3));
    let store = FakeStore::default();
    let finding = Uuid::new_v4();
    store
        .inner
        .lock()
        .unwrap()
        .findings
        .insert("c1-1".to_owned(), finding);
    run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    let (_, comments) = store.snapshot();
    let own = &comments[&("pr-1".to_owned(), "c1-1".to_owned())];
    assert_eq!(own.author_kind, AuthorKind::ReviewGraph);
    assert_eq!(own.published_finding_id, Some(finding));
    let other_bot = &comments[&("pr-2".to_owned(), "c2-1".to_owned())];
    assert_eq!(other_bot.author_kind, AuthorKind::Bot);
    let human = &comments[&("pr-2".to_owned(), "c2-0".to_owned())];
    assert_eq!(human.author_kind, AuthorKind::Human);
    assert_eq!(human.link_status, LinkStatus::PathOnly);
}

#[tokio::test]
async fn opt_in_required_for_comment_bodies() {
    let provider = FakeProvider::new(fixture(5));
    let off = FakeStore::default();
    run(&provider, &off, &cfg(false), IngestMode::Backfill).await;
    assert!(off
        .snapshot()
        .1
        .values()
        .all(|c| c.body_text_redacted.is_none()));

    let on = FakeStore::default();
    run(&provider, &on, &cfg(true), IngestMode::Backfill).await;
    for c in on.snapshot().1.values() {
        let body = c.body_text_redacted.as_deref().unwrap();
        assert!(!body.contains(SECRET), "bodies pass the redaction layer");
        assert!(body.contains("redacted"));
    }
}

#[tokio::test]
async fn bodies_are_capped_at_4_kib() {
    let mut prs = fixture(1);
    prs[0].comments[0].body = "é".repeat(5000);
    let provider = FakeProvider::new(prs);
    let store = FakeStore::default();
    run(&provider, &store, &cfg(true), IngestMode::Backfill).await;
    let body = store.snapshot().1[&("pr-0".to_owned(), "c0-0".to_owned())]
        .body_text_redacted
        .clone()
        .unwrap();
    assert!(body.len() <= 4096);
}

#[tokio::test]
async fn window_and_pr_cap_bound_the_backfill() {
    let provider = FakeProvider::new(fixture(300));
    let store = FakeStore::default();
    let mut capped = cfg(true);
    capped.max_prs = 120;
    let report = run(&provider, &store, &capped, IngestMode::Backfill).await;
    assert_eq!(report.prs_ingested, 120);

    let windowed = FakeStore::default();
    let mut narrow = cfg(true);
    narrow.window_days = 1; // fixture PRs close 1..300 hours before `now`
    let report = run(&provider, &windowed, &narrow, IngestMode::Backfill).await;
    assert_eq!(report.prs_ingested, 24);
}

#[tokio::test]
async fn forbidden_skips_cleanly_and_permanent_failure_marks_failed() {
    let store = FakeStore::default();
    let forbidden = FakeProvider::new(fixture(3)).failing_once(0, ProviderFailure::Forbidden);
    let report = run(&forbidden, &store, &cfg(true), IngestMode::Backfill).await;
    assert!(matches!(report.outcome, IngestOutcome::Skipped { .. }));
    assert_eq!(
        store.inner.lock().unwrap().status.as_ref().map(|s| s.0),
        Some(IngestStatus::Skipped)
    );

    let gone = FakeProvider::new(fixture(3)).failing_once(
        0,
        ProviderFailure::Permanent {
            reason: "repository_not_found".into(),
        },
    );
    let report = run(&gone, &store, &cfg(true), IngestMode::Backfill).await;
    assert_eq!(
        report.outcome,
        IngestOutcome::Failed {
            reason: "repository_not_found".into()
        }
    );
    assert_eq!(
        store.inner.lock().unwrap().status.clone(),
        Some((IngestStatus::Failed, Some("repository_not_found".into())))
    );
}
