//! Minimal typed Qdrant REST client (SEM-003, ADR-008: REST via reqwest, no gRPC stack).
//!
//! `pub(crate)` on purpose: the only public way to read or write points is
//! [`crate::SemanticIndex`], which always renders the tenant filter (SEM-005).

use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::Method;
use serde_json::{json, Value};
use telemetry::Secret;
use tracing::Instrument;
use uuid::Uuid;

use super::error::QdrantError;
use super::types::{
    CollectionState, HnswCfg, Payload, PointUpsert, Record, ScoredPoint, ScrollPage, WithPayload,
    MAX_PAYLOAD_BYTES, UPSERT_BATCH,
};
use crate::filter::{Filter, IndexSchema};
use crate::metrics;

/// Connection settings.
#[derive(Clone)]
pub struct QdrantConfig {
    /// Base URL, for example `http://127.0.0.1:6333`.
    pub url: String,
    /// Sent as the `api-key` header. Never logged.
    pub api_key: Option<Secret>,
    pub request_timeout: Duration,
    /// Search requests fail fast: a failed search degrades context, never a review.
    pub search_timeout: Duration,
    pub max_retries: u32,
    pub retry_base: Duration,
    pub retry_cap: Duration,
}

impl std::fmt::Debug for QdrantConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantConfig")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("request_timeout", &self.request_timeout)
            .field("search_timeout", &self.search_timeout)
            .field("max_retries", &self.max_retries)
            .finish_non_exhaustive()
    }
}

impl QdrantConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into().trim_end_matches('/').to_owned(),
            api_key: None,
            request_timeout: Duration::from_secs(10),
            search_timeout: Duration::from_secs(2),
            max_retries: 3,
            retry_base: Duration::from_millis(200),
            retry_cap: Duration::from_secs(3),
        }
    }

    /// `QDRANT_URL` (required) and `QDRANT_API_KEY` (optional).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let url = lookup("QDRANT_URL").filter(|u| !u.trim().is_empty())?;
        let mut cfg = Self::new(url.trim());
        cfg.api_key = lookup("QDRANT_API_KEY")
            .filter(|k| !k.is_empty())
            .map(Secret::new);
        Some(cfg)
    }

    /// No waiting between retries (tests).
    pub fn without_backoff(mut self) -> Self {
        self.retry_base = Duration::ZERO;
        self.retry_cap = Duration::ZERO;
        self
    }
}

/// Called with `(op, collection, body)` before every request is sent. Used by the tenant audit.
pub(crate) type Observer = Arc<dyn Fn(&'static str, &str, &Value) + Send + Sync>;

#[derive(Clone)]
pub(crate) struct QdrantClient {
    http: reqwest::Client,
    cfg: QdrantConfig,
    observer: Option<Observer>,
}

impl std::fmt::Debug for QdrantClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QdrantClient")
            .field("cfg", &self.cfg)
            .field("observed", &self.observer.is_some())
            .finish()
    }
}

fn check_name(name: &str) -> Result<(), QdrantError> {
    let ok = !name.is_empty()
        && name.len() <= 255
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(QdrantError::BadRequest {
            body: format!("invalid collection name {name:?}"),
        })
    }
}

fn parse_id(v: &Value) -> Result<Uuid, QdrantError> {
    v.as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| QdrantError::Protocol(format!("point id is not a uuid: {v}")))
}

fn parse_payload(v: Option<&Value>) -> Payload {
    match v {
        Some(Value::Object(m)) => m.clone(),
        _ => Payload::new(),
    }
}

fn parse_vector(v: Option<&Value>) -> Option<Vec<f32>> {
    v?.as_array()?
        .iter()
        .map(|x| x.as_f64().map(|f| f as f32))
        .collect()
}

impl QdrantClient {
    pub(crate) fn new(cfg: QdrantConfig) -> Result<Self, QdrantError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("reviewgraph-semantic/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| QdrantError::Unavailable(format!("http client: {e}")))?;
        Ok(Self {
            http,
            cfg,
            observer: None,
        })
    }

    #[cfg_attr(not(feature = "audit"), allow(dead_code))]
    pub(crate) fn set_observer(&mut self, observer: Observer) {
        self.observer = Some(observer);
    }

    fn backoff(&self, attempt: u32) -> Duration {
        self.cfg
            .retry_base
            .saturating_mul(
                1u32.checked_shl(attempt.saturating_sub(1))
                    .unwrap_or(u32::MAX),
            )
            .min(self.cfg.retry_cap)
    }

    /// Sends one request (with retries) and returns the `result` member of the response.
    async fn request(
        &self,
        op: &'static str,
        collection: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, QdrantError> {
        if let (Some(obs), Some(b)) = (&self.observer, &body) {
            obs(op, collection, b);
        }
        let points = body
            .as_ref()
            .and_then(|b| b.get("points"))
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let span = tracing::debug_span!(
            "qdrant_request",
            op,
            collection,
            points,
            status = tracing::field::Empty
        );
        let url = format!("{}{}", self.cfg.url, path);
        let mut attempt: u32 = 0;
        loop {
            let started = Instant::now();
            let result = self
                .send_once(&method, &url, body.as_ref(), timeout)
                .instrument(span.clone())
                .await;
            metrics::qdrant_call(op, started.elapsed().as_secs_f64() * 1000.0);
            match result {
                Ok((status, value)) => {
                    span.record("status", status);
                    return Ok(value);
                }
                Err(e) => {
                    metrics::qdrant_error(op, e.kind());
                    if e.is_retryable() && attempt < self.cfg.max_retries {
                        attempt += 1;
                        let wait = self.backoff(attempt);
                        tracing::debug!(op, attempt, kind = e.kind(), "retrying qdrant request");
                        if !wait.is_zero() {
                            tokio::time::sleep(wait).await;
                        }
                        continue;
                    }
                    span.record("status", e.kind());
                    return Err(e);
                }
            }
        }
    }

    async fn send_once(
        &self,
        method: &Method,
        url: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<(u16, Value), QdrantError> {
        let mut req = self.http.request(method.clone(), url).timeout(timeout);
        if let Some(key) = &self.cfg.api_key {
            req = req.header("api-key", key.expose());
        }
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| {
            if e.is_timeout() {
                QdrantError::Timeout
            } else {
                QdrantError::Unavailable("connection failed".into())
            }
        })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            if e.is_timeout() {
                QdrantError::Timeout
            } else {
                QdrantError::Unavailable("response body could not be read".into())
            }
        })?;
        let code = status.as_u16();
        if status.is_success() {
            let parsed: Value = if text.trim().is_empty() {
                Value::Null
            } else {
                serde_json::from_str(&text).unwrap_or(Value::String(text))
            };
            let result = match parsed {
                Value::Object(mut m) => m.remove("result").unwrap_or(Value::Null),
                other => other,
            };
            return Ok((code, result));
        }
        let detail: String = text.chars().take(500).collect();
        Err(match code {
            404 => QdrantError::NotFound,
            409 => QdrantError::Conflict(detail),
            400 | 422 => QdrantError::BadRequest { body: detail },
            408 => QdrantError::Timeout,
            _ if status.is_server_error() => QdrantError::Unavailable(format!("http {code}")),
            _ => QdrantError::BadRequest { body: detail },
        })
    }

    /// `GET /readyz`.
    pub(crate) async fn ready(&self) -> Result<(), QdrantError> {
        self.request(
            "ready",
            "",
            Method::GET,
            "/readyz",
            None,
            self.cfg.request_timeout,
        )
        .await
        .map(|_| ())
    }

    /// `GET /collections/{c}`; `None` when it does not exist.
    pub(crate) async fn get_collection(
        &self,
        name: &str,
    ) -> Result<Option<CollectionState>, QdrantError> {
        check_name(name)?;
        let path = format!("/collections/{name}");
        let result = match self
            .request(
                "get_collection",
                name,
                Method::GET,
                &path,
                None,
                self.cfg.request_timeout,
            )
            .await
        {
            Ok(v) => v,
            Err(QdrantError::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let vectors = &result["config"]["params"]["vectors"];
        let dims = vectors
            .get("size")
            .and_then(Value::as_u64)
            .ok_or_else(|| QdrantError::Protocol("collection has no vector size".into()))?;
        let indexed_fields = result
            .get("payload_schema")
            .and_then(Value::as_object)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        Ok(Some(CollectionState {
            name: name.to_owned(),
            dims,
            created: false,
            points_count: result.get("points_count").and_then(Value::as_u64),
            indexed_fields,
        }))
    }

    /// Creates the collection (cosine distance) unless it exists. Never changes an existing one;
    /// the caller compares `dims`.
    pub(crate) async fn ensure_collection(
        &self,
        name: &str,
        dims: u16,
        hnsw: HnswCfg,
    ) -> Result<CollectionState, QdrantError> {
        if let Some(state) = self.get_collection(name).await? {
            return Ok(state);
        }
        let body = json!({
            "vectors": {"size": dims, "distance": "Cosine"},
            "hnsw_config": {"m": hnsw.m, "ef_construct": hnsw.ef_construct},
            "on_disk_payload": hnsw.on_disk_payload,
        });
        let path = format!("/collections/{name}");
        match self
            .request(
                "create_collection",
                name,
                Method::PUT,
                &path,
                Some(body),
                self.cfg.request_timeout,
            )
            .await
        {
            Ok(_) => {}
            // Another worker created it first.
            Err(QdrantError::Conflict(_)) => {}
            Err(QdrantError::BadRequest { body }) if body.contains("already exists") => {}
            Err(e) => return Err(e),
        }
        let mut state = self
            .get_collection(name)
            .await?
            .ok_or_else(|| QdrantError::Protocol("collection missing after create".into()))?;
        state.created = true;
        Ok(state)
    }

    /// `DELETE /collections/{c}`; deleting a missing collection succeeds.
    pub(crate) async fn delete_collection(&self, name: &str) -> Result<(), QdrantError> {
        check_name(name)?;
        let path = format!("/collections/{name}");
        match self
            .request(
                "delete_collection",
                name,
                Method::DELETE,
                &path,
                None,
                self.cfg.request_timeout,
            )
            .await
        {
            Ok(_) | Err(QdrantError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `PUT /collections/{c}/index`. Re-creating an existing index is accepted by Qdrant.
    pub(crate) async fn ensure_payload_index(
        &self,
        collection: &str,
        field: &str,
        schema: IndexSchema,
        is_tenant: bool,
    ) -> Result<(), QdrantError> {
        check_name(collection)?;
        let mut field_schema = json!({"type": schema.as_str()});
        if is_tenant {
            field_schema["is_tenant"] = json!(true);
        }
        let body = json!({"field_name": field, "field_schema": field_schema});
        let path = format!("/collections/{collection}/index?wait=true");
        self.request(
            "create_index",
            collection,
            Method::PUT,
            &path,
            Some(body),
            self.cfg.request_timeout,
        )
        .await
        .map(|_| ())
    }

    /// `PUT /collections/{c}/points?wait=true` in batches of 256 (read-after-write consistent).
    pub(crate) async fn upsert(
        &self,
        collection: &str,
        points: &[PointUpsert],
    ) -> Result<(), QdrantError> {
        check_name(collection)?;
        let path = format!("/collections/{collection}/points?wait=true");
        for batch in points.chunks(UPSERT_BATCH) {
            let mut items = Vec::with_capacity(batch.len());
            for p in batch {
                let payload = Value::Object(p.payload.clone());
                let size = serde_json::to_vec(&payload).map_or(usize::MAX, |v| v.len());
                if size > MAX_PAYLOAD_BYTES {
                    return Err(QdrantError::BadRequest {
                        body: format!("payload of point {} is {size} bytes (cap 8 KiB)", p.id),
                    });
                }
                items.push(json!({
                    "id": p.id.hyphenated().to_string(),
                    "vector": p.vector,
                    "payload": payload,
                }));
            }
            self.request(
                "upsert",
                collection,
                Method::PUT,
                &path,
                Some(json!({ "points": items })),
                self.cfg.request_timeout,
            )
            .await?;
        }
        Ok(())
    }

    /// `POST /collections/{c}/points/delete?wait=true` with a filter.
    pub(crate) async fn delete_by_filter(
        &self,
        collection: &str,
        filter: &Filter,
    ) -> Result<(), QdrantError> {
        check_name(collection)?;
        let path = format!("/collections/{collection}/points/delete?wait=true");
        self.request(
            "delete",
            collection,
            Method::POST,
            &path,
            Some(json!({"filter": filter.to_json()})),
            self.cfg.request_timeout,
        )
        .await
        .map(|_| ())
    }

    /// `POST /collections/{c}/points/query` (nearest neighbours with payload).
    pub(crate) async fn search(
        &self,
        collection: &str,
        vector: &[f32],
        filter: &Filter,
        limit: u32,
        score_threshold: Option<f32>,
        hnsw_ef: Option<u32>,
    ) -> Result<Vec<ScoredPoint>, QdrantError> {
        check_name(collection)?;
        let mut body = json!({
            "query": vector,
            "filter": filter.to_json(),
            "limit": limit,
            "with_payload": true,
        });
        if let Some(t) = score_threshold {
            body["score_threshold"] = json!(t);
        }
        if let Some(ef) = hnsw_ef {
            body["params"] = json!({"hnsw_ef": ef});
        }
        let path = format!("/collections/{collection}/points/query");
        let result = self
            .request(
                "search",
                collection,
                Method::POST,
                &path,
                Some(body),
                self.cfg.search_timeout,
            )
            .await?;
        let points = result
            .get("points")
            .and_then(Value::as_array)
            .ok_or_else(|| QdrantError::Protocol("query result has no points".into()))?;
        points
            .iter()
            .map(|p| {
                Ok(ScoredPoint {
                    id: parse_id(&p["id"])?,
                    score: p.get("score").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                    payload: parse_payload(p.get("payload")),
                })
            })
            .collect()
    }

    /// `POST /collections/{c}/points/scroll`: one page ordered by id.
    pub(crate) async fn scroll(
        &self,
        collection: &str,
        filter: &Filter,
        with_payload: WithPayload<'_>,
        with_vector: bool,
        page: u32,
        offset: Option<Uuid>,
    ) -> Result<ScrollPage, QdrantError> {
        check_name(collection)?;
        let mut body = json!({
            "filter": filter.to_json(),
            "limit": page,
            "with_payload": match with_payload {
                WithPayload::All => json!(true),
                WithPayload::Fields(f) => json!({"include": f}),
            },
            "with_vector": with_vector,
        });
        if let Some(o) = offset {
            body["offset"] = json!(o.hyphenated().to_string());
        }
        let path = format!("/collections/{collection}/points/scroll");
        let result = self
            .request(
                "scroll",
                collection,
                Method::POST,
                &path,
                Some(body),
                self.cfg.request_timeout,
            )
            .await?;
        let points = result
            .get("points")
            .and_then(Value::as_array)
            .ok_or_else(|| QdrantError::Protocol("scroll result has no points".into()))?
            .iter()
            .map(|p| {
                Ok(Record {
                    id: parse_id(&p["id"])?,
                    payload: parse_payload(p.get("payload")),
                    vector: parse_vector(p.get("vector")),
                })
            })
            .collect::<Result<Vec<_>, QdrantError>>()?;
        let next_offset = match result.get("next_page_offset") {
            None | Some(Value::Null) => None,
            Some(v) => Some(parse_id(v)?),
        };
        Ok(ScrollPage {
            points,
            next_offset,
        })
    }

    /// Every point matching `filter` (pages of `page`).
    pub(crate) async fn scroll_all(
        &self,
        collection: &str,
        filter: &Filter,
        with_payload: WithPayload<'_>,
        with_vector: bool,
        page: u32,
    ) -> Result<Vec<Record>, QdrantError> {
        let mut out = Vec::new();
        let mut offset = None;
        loop {
            let p = self
                .scroll(collection, filter, with_payload, with_vector, page, offset)
                .await?;
            out.extend(p.points);
            match p.next_offset {
                Some(next) => offset = Some(next),
                None => return Ok(out),
            }
        }
    }

    /// `POST /collections/{c}/points/payload?wait=true`: merges `payload` into every point
    /// matching `filter`.
    pub(crate) async fn set_payload(
        &self,
        collection: &str,
        filter: &Filter,
        payload: Payload,
    ) -> Result<(), QdrantError> {
        check_name(collection)?;
        let path = format!("/collections/{collection}/points/payload?wait=true");
        self.request(
            "set_payload",
            collection,
            Method::POST,
            &path,
            Some(json!({"payload": Value::Object(payload), "filter": filter.to_json()})),
            self.cfg.request_timeout,
        )
        .await
        .map(|_| ())
    }

    /// `POST /collections/{c}/points/count` (exact).
    pub(crate) async fn count(
        &self,
        collection: &str,
        filter: &Filter,
    ) -> Result<u64, QdrantError> {
        check_name(collection)?;
        let path = format!("/collections/{collection}/points/count");
        let result = self
            .request(
                "count",
                collection,
                Method::POST,
                &path,
                Some(json!({"filter": filter.to_json(), "exact": true})),
                self.cfg.request_timeout,
            )
            .await?;
        result
            .get("count")
            .and_then(Value::as_u64)
            .ok_or_else(|| QdrantError::Protocol("count result has no count".into()))
    }
}
