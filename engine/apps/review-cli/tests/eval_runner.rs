#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
//! EVAL-003: the runner.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use review_cli::eval::case::{load_corpus, LoadedCase};
use review_cli::eval::runner::{
    confirm_live, replay_gateway, run_cases, Budgets, CaseFailure, CaseRun, EvalGatewayMode,
    ReviewExecutor, ReviewerExecutor, RunConfig,
};

use common::paths;

fn config(repetitions: u32, max_parallel: usize) -> RunConfig {
    RunConfig {
        name: "test".into(),
        gateway_mode: EvalGatewayMode::Replay,
        routing_override: None,
        reviewers: vec!["correctness".into()],
        verification_version: "none".into(),
        repetitions,
        max_parallel_cases: max_parallel,
        budgets: Budgets {
            max_model_calls: 40,
            max_model_tokens: 400_000,
        },
    }
}

fn cases(filter: &str) -> Vec<LoadedCase> {
    let (cases, errors) = load_corpus(&paths(), Some(filter)).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    cases
}

#[tokio::test]
async fn runner_executes_example_case_under_replay() {
    let gw = replay_gateway(&paths().fixtures()).unwrap();
    let exec = Arc::new(ReviewerExecutor::new(gw, &["correctness".to_owned()]).unwrap());
    let rows = run_cases(&cases("example-000-template"), exec, &config(1, 1), "run").await;
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r.case_error, None, "{:?}", r.replay_misses);
    assert!(r.matches.is_some());
    assert!(r.findings.is_empty());
    assert_eq!(r.structured.success, 1);
    assert!(!r.context_symbols.is_empty());
}

#[tokio::test]
async fn runner_records_replay_miss_as_case_error() {
    let empty = tempfile::tempdir().unwrap();
    let gw = replay_gateway(empty.path()).unwrap();
    let exec = Arc::new(ReviewerExecutor::new(gw, &["correctness".to_owned()]).unwrap());
    let rows = run_cases(&cases("example-000-template"), exec, &config(1, 1), "run").await;
    assert_eq!(rows[0].case_error.as_deref(), Some("replay_miss"));
    assert!(!rows[0].replay_misses.is_empty());
}

struct Panicky;

#[async_trait]
impl ReviewExecutor for Panicky {
    async fn execute(&self, case: &LoadedCase, _rep: u32) -> Result<CaseRun, CaseFailure> {
        if case.case.id.starts_with("corr-001") {
            panic!("executor bug");
        }
        Ok(CaseRun::default())
    }
}

#[tokio::test]
async fn runner_isolates_case_panics() {
    let rows = run_cases(&cases("corr-00[12]-*"), Arc::new(Panicky), &config(1, 2), "run").await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].case_error.as_deref(), Some("panic"));
    assert_eq!(rows[1].case_error, None);
}

#[derive(Default)]
struct Counting {
    current: AtomicUsize,
    max: AtomicUsize,
}

#[async_trait]
impl ReviewExecutor for Counting {
    async fn execute(&self, _case: &LoadedCase, _rep: u32) -> Result<CaseRun, CaseFailure> {
        let now = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.current.fetch_sub(1, Ordering::SeqCst);
        Ok(CaseRun::default())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runner_respects_max_parallel_cases() {
    let exec = Arc::new(Counting::default());
    let rows = run_cases(&cases("corr-*"), exec.clone(), &config(2, 2), "run").await;
    assert_eq!(rows.len(), 10);
    let max = exec.max.load(Ordering::SeqCst);
    assert!((1..=2).contains(&max), "{max}");
}

#[tokio::test]
async fn repetitions_produce_distinct_rows() {
    let rows = run_cases(
        &cases("example-000-template"),
        Arc::new(Counting::default()),
        &config(3, 1),
        "run",
    )
    .await;
    let reps: Vec<u32> = rows.iter().map(|r| r.repetition).collect();
    assert_eq!(reps, [0, 1, 2]);
}

#[test]
fn live_mode_requires_confirmation_above_threshold() {
    assert!(confirm_live(6.0, false).is_err());
    assert!(confirm_live(6.0, true).is_ok());
    assert!(confirm_live(4.99, false).is_ok());
}
