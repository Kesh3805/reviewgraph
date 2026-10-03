//! Replay fixture format and store (GW-005).
//!
//! Path: `{root}/{task}/{hash[0..2]}/{hash}.{provider}.{model}.json`. Synthetic fixtures use
//! `any` for provider and model and match every route.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, PermanentKind};
use crate::types::{FinishReason, ModelOutput, TaskType, Usage};

pub const FIXTURE_VERSION: u32 = 1;
/// Wildcard provider/model used by synthetic fixtures.
pub const ANY: &str = "any";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FixtureResponse {
    pub output: ModelOutput,
    pub usage: Usage,
    pub finish_reason: FinishReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Fixture {
    pub fixture_version: u32,
    pub request_hash: String,
    pub provider: String,
    pub model: String,
    pub task: String,
    pub prompt_id: String,
    pub prompt_version: String,
    pub schema_hash: Option<String>,
    pub synthetic: bool,
    pub recorded_at: String,
    pub engine_git_sha: Option<String>,
    pub response: FixtureResponse,
    pub latency_ms: u32,
}

fn clean(part: &str) -> String {
    part.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Location of a fixture file under `root`.
pub fn fixture_path(
    root: &Path,
    task: TaskType,
    hash: &str,
    provider: &str,
    model: &str,
) -> PathBuf {
    let shard: String = hash.chars().take(2).collect();
    root.join(task.as_str()).join(shard).join(format!(
        "{hash}.{}.{}.json",
        clean(provider),
        clean(model)
    ))
}

fn corrupt(path: &Path, why: &str) -> GatewayError {
    GatewayError::Permanent {
        kind: PermanentKind::Unknown,
        provider: None,
        detail: format!("corrupt replay fixture {}: {why}", path.display())
            .chars()
            .take(500)
            .collect(),
    }
}

/// Reads fixtures from disk with a memo, and writes new ones atomically.
#[derive(Debug)]
pub struct FixtureStore {
    root: PathBuf,
    memo: DashMap<PathBuf, Arc<Fixture>>,
}

impl FixtureStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            memo: DashMap::new(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    async fn read(&self, path: PathBuf) -> Result<Option<Arc<Fixture>>, GatewayError> {
        if let Some(f) = self.memo.get(&path) {
            return Ok(Some(Arc::clone(f.value())));
        }
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(corrupt(&path, &e.to_string())),
        };
        let fixture: Fixture =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(&path, &e.to_string()))?;
        if fixture.fixture_version != FIXTURE_VERSION {
            return Err(corrupt(&path, "unsupported fixture_version"));
        }
        let fixture = Arc::new(fixture);
        self.memo.insert(path, Arc::clone(&fixture));
        Ok(Some(fixture))
    }

    /// Exact `(hash, provider, model)` first, then `(hash, any, any)`.
    pub async fn lookup(
        &self,
        task: TaskType,
        hash: &str,
        provider: &str,
        model: &str,
    ) -> Result<Option<Arc<Fixture>>, GatewayError> {
        let exact = fixture_path(&self.root, task, hash, provider, model);
        if let Some(f) = self.read(exact).await? {
            return Ok(Some(f));
        }
        self.read(fixture_path(&self.root, task, hash, ANY, ANY))
            .await
    }

    /// Writes `fixture` atomically (temp file + link). Never overwrites unless `overwrite`.
    /// Returns `true` when a file was written; losing a race to another writer is not an error.
    pub async fn write(
        &self,
        task: TaskType,
        fixture: &Fixture,
        overwrite: bool,
    ) -> std::io::Result<bool> {
        let path = fixture_path(
            &self.root,
            task,
            &fixture.request_hash,
            &fixture.provider,
            &fixture.model,
        );
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.clone());
        tokio::fs::create_dir_all(&dir).await?;
        let mut text = serde_json::to_string_pretty(fixture)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        text.push('\n');
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp = dir.join(format!(".tmp.{}.{nonce}", std::process::id()));
        tokio::fs::write(&tmp, text).await?;
        let result = if overwrite {
            tokio::fs::rename(&tmp, &path).await.map(|()| true)
        } else {
            let linked = tokio::fs::hard_link(&tmp, &path).await;
            let _ = tokio::fs::remove_file(&tmp).await;
            match linked {
                Ok(()) => Ok(true),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
                Err(e) => Err(e),
            }
        };
        if matches!(result, Ok(true)) {
            self.memo.remove(&path);
        }
        result
    }
}
