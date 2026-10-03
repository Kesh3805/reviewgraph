//! Test doubles for crates that consume the gateway (`testing` feature).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::builder::ModelGateway;
use crate::error::{BudgetKind, GatewayError};
use crate::request_hash::request_hash;
use crate::types::{
    FinishReason, ModelOutput, ModelRequest, ModelResponse, ModelTier, ProviderId, RouteDecision,
    ServedFrom, TaskType, Usage,
};

type ErrorFactory = Box<dyn Fn() -> GatewayError + Send + Sync>;

enum Scripted {
    Output(ModelOutput),
    Error(ErrorFactory),
    Pending,
}

/// A gateway that answers from a script keyed by [`TaskType`]. Responses are consumed in
/// order; the last one repeats.
#[derive(Default)]
pub struct FakeGateway {
    script: Mutex<HashMap<TaskType, VecDeque<Scripted>>>,
    calls: Mutex<Vec<(TaskType, String)>>,
}

impl std::fmt::Debug for FakeGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeGateway").finish_non_exhaustive()
    }
}

impl FakeGateway {
    pub fn new() -> Self {
        Self::default()
    }

    fn push(self, task: TaskType, s: Scripted) -> Self {
        if let Ok(mut m) = self.script.lock() {
            m.entry(task).or_default().push_back(s);
        }
        self
    }

    pub fn on_json(self, task: TaskType, value: serde_json::Value) -> Self {
        self.push(task, Scripted::Output(ModelOutput::Json(value)))
    }

    pub fn on_text(self, task: TaskType, text: impl Into<String>) -> Self {
        self.push(task, Scripted::Output(ModelOutput::Text(text.into())))
    }

    pub fn on_error(
        self,
        task: TaskType,
        make: impl Fn() -> GatewayError + Send + Sync + 'static,
    ) -> Self {
        self.push(task, Scripted::Error(Box::new(make)))
    }

    /// The call never completes (until cancelled).
    pub fn on_pending(self, task: TaskType) -> Self {
        self.push(task, Scripted::Pending)
    }

    /// `(task, request_hash)` of every call received so far.
    pub fn calls(&self) -> Vec<(TaskType, String)> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

enum Step {
    Output(ModelOutput),
    Error(GatewayError),
    Pending,
}

impl FakeGateway {
    fn next(&self, task: TaskType) -> Option<Step> {
        let mut m = self.script.lock().ok()?;
        let q = m.get_mut(&task)?;
        let take = |s: &Scripted| match s {
            Scripted::Output(o) => Step::Output(o.clone()),
            Scripted::Error(f) => Step::Error(f()),
            Scripted::Pending => Step::Pending,
        };
        if q.len() > 1 {
            q.pop_front().map(|s| take(&s))
        } else {
            q.front().map(take)
        }
    }
}

#[async_trait]
impl ModelGateway for FakeGateway {
    async fn call(
        &self,
        req: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<ModelResponse, GatewayError> {
        if req.budget.deadline <= Instant::now() {
            return Err(GatewayError::BudgetExceeded {
                kind: BudgetKind::Deadline,
            });
        }
        let hash = request_hash(&req);
        if let Ok(mut c) = self.calls.lock() {
            c.push((req.task, hash.0.clone()));
        }
        let step = self.next(req.task).ok_or_else(|| GatewayError::Permanent {
            kind: crate::error::PermanentKind::ReplayMiss,
            provider: None,
            detail: format!("no scripted response for {}", req.task.as_str()),
        })?;
        match step {
            Step::Error(e) => Err(e),
            Step::Pending => {
                cancel.cancelled().await;
                Err(GatewayError::Cancelled)
            }
            Step::Output(output) => Ok(ModelResponse {
                output,
                usage: Usage::default(),
                latency_ms: 0,
                provider: ProviderId::new("fake"),
                model: "fake".into(),
                cost_usd_micros: Some(0),
                finish_reason: FinishReason::Complete,
                request_hash: hash,
                route: RouteDecision {
                    requested_tier: req.tier,
                    effective_tier: req.tier,
                    candidates: Vec::new(),
                    downgraded: None,
                    table_hash: "fake".into(),
                    attempted: Vec::new(),
                },
                attempts: 1,
                served_from: ServedFrom::Replay,
            }),
        }
    }
}

/// Convenience tier used by the fakes' own tests.
pub const FAKE_TIER: ModelTier = ModelTier::FastReasoner;
