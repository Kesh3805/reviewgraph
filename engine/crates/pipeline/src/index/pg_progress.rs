//! `index_runs` progress sink (IDX-002).
//!
//! Phase changes are always written; file counters are throttled (at most once per second and
//! per 1,000 files) so progress never adds measurable database load. Every write failure is
//! logged and ignored: progress is advisory and must never fail an index.

use std::sync::Mutex;
use std::time::Instant;

use async_trait::async_trait;
use graph_storage::pg::PgIndexRuns;
use repository::store::RepoScope;
use review_core::ids::SnapshotId;
use uuid::Uuid;

use super::full::IndexStats;
use super::progress::{IndexPhase, ProgressSink, ProgressThrottle};

/// Writes index progress to `index_runs`.
#[derive(Debug)]
pub struct PgProgressSink {
    runs: PgIndexRuns,
    scope: RepoScope,
    job_id: Option<Uuid>,
    state: Mutex<State>,
    started: Instant,
}

#[derive(Debug)]
struct State {
    run: Option<Uuid>,
    throttle: ProgressThrottle,
}

impl PgProgressSink {
    pub fn new(runs: PgIndexRuns, scope: RepoScope, job_id: Option<Uuid>) -> Self {
        Self {
            runs,
            scope,
            job_id,
            state: Mutex::new(State {
                run: None,
                throttle: ProgressThrottle::standard(),
            }),
            started: Instant::now(),
        }
    }

    fn run_id(&self) -> Option<Uuid> {
        self.state.lock().ok().and_then(|s| s.run)
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

#[async_trait]
impl ProgressSink for PgProgressSink {
    async fn phase(&self, phase: IndexPhase) {
        let Some(run) = self.run_id() else {
            return;
        };
        if phase == IndexPhase::Done {
            // The final write, with the statistics, happens in `finish`.
            return;
        }
        if let Err(error) = self
            .runs
            .update(&self.scope, run, phase.as_str(), None)
            .await
        {
            tracing::warn!(%error, "index_runs phase update failed");
        }
    }

    async fn files(&self, done: u64, total: u64) {
        let Some(run) = self.run_id() else {
            return;
        };
        let due = {
            let now = self.now_ms();
            match self.state.lock() {
                Ok(mut state) => state.throttle.should_write(now, done, total),
                Err(_) => false,
            }
        };
        if !due {
            return;
        }
        if let Err(error) = self
            .runs
            .update(
                &self.scope,
                run,
                IndexPhase::Parsing.as_str(),
                Some((done, total)),
            )
            .await
        {
            tracing::warn!(%error, "index_runs file counter update failed");
        }
    }

    async fn started(&self, snapshot: SnapshotId) {
        match self
            .runs
            .start(&self.scope, snapshot, "full", self.job_id)
            .await
        {
            Ok(run) => {
                if let Ok(mut state) = self.state.lock() {
                    state.run = Some(run);
                }
            }
            Err(error) => tracing::warn!(%error, "index_runs start failed"),
        }
    }

    async fn failed(&self, message: &str) {
        let Some(run) = self.run_id() else {
            return;
        };
        let stats = serde_json::json!({});
        if let Err(error) = self
            .runs
            .finish(
                &self.scope,
                run,
                IndexPhase::Failed.as_str(),
                &stats,
                Some(message),
            )
            .await
        {
            tracing::warn!(%error, "index_runs finish failed");
        }
    }

    async fn finish(&self, stats: &IndexStats) {
        let Some(run) = self.run_id() else {
            return;
        };
        let json = serde_json::to_value(stats).unwrap_or_default();
        if let Err(error) = self
            .runs
            .finish(&self.scope, run, IndexPhase::Done.as_str(), &json, None)
            .await
        {
            tracing::warn!(%error, "index_runs finish failed");
        }
    }
}
