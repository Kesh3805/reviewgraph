//! `SemanticIndex`: the only public way to search or write vectors (SEM-005).
//!
//! Every method takes a [`TenantScope`]; filters are rendered by [`crate::tenant::scoped`], point
//! payload tenant keys come from the scope, and every returned point is re-checked against the
//! scope before it leaves the crate.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use review_core::ids::{RepositoryId, SnapshotId, SymbolKey};
use review_core::language::Language;
use review_core::location::RepoPath;
use serde_json::{json, Value};
use tracing::Instrument;
use uuid::Uuid;

use crate::embedding::{EmbedRequest, EmbeddingProvider, EmbeddingSpace, InputKind, TraceContext};
use crate::error::{Error, Result};
use crate::filter::{fields, Cond};
use crate::metrics;
use crate::point_id::point_id;
use crate::qdrant::types::{PointUpsert, WithPayload};
use crate::qdrant::{Payload, QdrantClient, QdrantConfig};
use crate::tenant::{payload_in_scope, scoped, ExtraFilter, TenantScope};
use crate::units::{EmbeddingUnit, UnitKind, UNIT_TEMPLATE_VERSION};

/// Upper bound on `SemanticQuery::limit`.
pub const MAX_LIMIT: u32 = 100;
const SCROLL_PAGE: u32 = 256;

/// Which collections reads and writes go to. During a model migration writes go to the active
/// collection **and** the one being built (dual-write); reads always use the active one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionTargets {
    pub read: String,
    /// Includes `read`.
    pub write: Vec<String>,
}

impl CollectionTargets {
    pub fn single(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            write: vec![name.clone()],
            read: name,
        }
    }
}

/// Query input: text (embedded as a query) or a ready vector of the index's space.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryVector {
    Text(String),
    Vector(Vec<f32>),
}

/// A semantic search. Only scoped through [`SemanticIndex::search`].
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticQuery {
    pub vector: QueryVector,
    /// Empty means every kind.
    pub kinds: Vec<UnitKind>,
    /// Empty means every language.
    pub languages: Vec<Language>,
    /// Restrict to points present in this snapshot.
    pub snapshot_id: Option<SnapshotId>,
    pub exclude_symbol_keys: Vec<SymbolKey>,
    /// `1..=100`.
    pub limit: u32,
    pub min_score: f32,
}

impl SemanticQuery {
    pub fn text(text: impl Into<String>, limit: u32) -> Self {
        Self {
            vector: QueryVector::Text(text.into()),
            kinds: Vec::new(),
            languages: Vec::new(),
            snapshot_id: None,
            exclude_symbol_keys: Vec::new(),
            limit,
            min_score: 0.0,
        }
    }

    pub fn vector(vector: Vec<f32>, limit: u32) -> Self {
        Self {
            vector: QueryVector::Vector(vector),
            ..Self::text(String::new(), limit)
        }
    }
}

/// One search result, already verified to be inside the caller's scope.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticHit {
    pub point_id: Uuid,
    pub score: f32,
    pub kind: UnitKind,
    pub key: String,
    pub repository_id: RepositoryId,
    pub symbol_key: Option<SymbolKey>,
    pub file_path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub language: Option<String>,
    pub content_hash: Option<String>,
}

/// Result of [`SemanticIndex::upsert_units`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpsertReport {
    pub upserted: usize,
    pub tokens: u64,
}

/// What [`SemanticIndex::delete_units`] removes inside the scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteSelector {
    /// Units of one kind by key, in every repository of the scope.
    Units { kind: UnitKind, keys: Vec<String> },
    /// Every unit (all kinds) of these symbols.
    SymbolKeys(Vec<SymbolKey>),
    /// Every unit of these files.
    FilePaths(Vec<RepoPath>),
    /// Garbage collection: points present in none of the live snapshots. The set must be
    /// non-empty (it always contains the default-branch head).
    NotInSnapshots(Vec<SnapshotId>),
    /// Everything in the scope.
    All,
}

/// Stored state of one point, as the sync needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingPoint {
    pub content_hash: Option<String>,
    pub snapshot_ids: Vec<String>,
}

/// Tenant-scoped vector index over one embedding space.
#[derive(Debug)]
pub struct SemanticIndex {
    client: QdrantClient,
    provider: Arc<dyn EmbeddingProvider>,
    collections: RwLock<Option<CollectionTargets>>,
    hnsw_ef: Option<u32>,
}

fn str_field(p: &Payload, key: &str) -> Option<String> {
    p.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn u32_field(p: &Payload, key: &str) -> Option<u32> {
    p.get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn report_violation(what: &str, id: Uuid) {
    metrics::scope_violation();
    tracing::error!(point_id = %id, what, "qdrant point outside the tenant scope dropped");
}

impl SemanticIndex {
    /// Connects lazily (no request is sent). Call [`crate::collections::bootstrap`] or
    /// [`SemanticIndex::set_targets`] before use.
    pub fn new(cfg: QdrantConfig, provider: Arc<dyn EmbeddingProvider>) -> Result<Self> {
        Ok(Self {
            client: QdrantClient::new(cfg)?,
            provider,
            collections: RwLock::new(None),
            hnsw_ef: None,
        })
    }

    pub fn with_targets(self, targets: CollectionTargets) -> Self {
        self.set_targets(targets);
        self
    }

    /// Search-time `hnsw_ef` (Qdrant default when `None`).
    pub fn with_hnsw_ef(mut self, ef: Option<u32>) -> Self {
        self.hnsw_ef = ef;
        self
    }

    /// Routes every request through `audit` (SEM-005 / SEC-002 test builds only).
    #[cfg(feature = "audit")]
    pub fn with_audit(mut self, audit: Arc<crate::audit::TenantAudit>) -> Self {
        self.client
            .set_observer(Arc::new(move |op, collection, body| {
                audit.observe(op, collection, body);
            }));
        self
    }

    pub fn provider(&self) -> &Arc<dyn EmbeddingProvider> {
        &self.provider
    }

    pub fn space(&self) -> &EmbeddingSpace {
        self.provider.space()
    }

    pub fn set_targets(&self, targets: CollectionTargets) {
        if let Ok(mut guard) = self.collections.write() {
            *guard = Some(targets);
        }
    }

    pub fn targets(&self) -> Result<CollectionTargets> {
        self.collections
            .read()
            .ok()
            .and_then(|g| g.clone())
            .ok_or_else(|| Error::Registry("semantic index has no active collection".into()))
    }

    pub(crate) fn client(&self) -> &QdrantClient {
        &self.client
    }

    /// Qdrant readiness.
    pub async fn ready(&self) -> Result<()> {
        Ok(self.client.ready().await?)
    }

    /// The deterministic point id of `unit` in this space.
    pub fn point_id_of(&self, scope: &TenantScope, unit: &EmbeddingUnit) -> Uuid {
        point_id(
            scope.organization_id(),
            unit.repository_id,
            unit.kind,
            &unit.key,
            &self.space().id(),
        )
    }

    /// Nearest units inside `scope`. Failures are reported, never panicked; callers degrade to
    /// structural-only context.
    pub async fn search(&self, scope: &TenantScope, q: &SemanticQuery) -> Result<Vec<SemanticHit>> {
        if q.limit == 0 || q.limit > MAX_LIMIT {
            return Err(Error::InvalidInput(format!(
                "limit must be in 1..={MAX_LIMIT}, got {}",
                q.limit
            )));
        }
        let targets = self.targets()?;
        let vector = match &q.vector {
            QueryVector::Text(t) => {
                let texts = [t.clone()];
                let mut resp = self
                    .provider
                    .embed(EmbedRequest {
                        kind: InputKind::Query,
                        texts: &texts,
                        trace: TraceContext::new().organization_id(scope.organization_id()),
                    })
                    .await?;
                resp.vectors.pop().ok_or_else(|| {
                    Error::InvalidInput("embedding provider returned no vector".into())
                })?
            }
            QueryVector::Vector(v) => {
                if v.len() != usize::from(self.space().dims) {
                    return Err(Error::InvalidInput(format!(
                        "query vector has {} dims, the space has {}",
                        v.len(),
                        self.space().dims
                    )));
                }
                v.clone()
            }
        };
        let mut extra = ExtraFilter::new();
        if !q.kinds.is_empty() {
            extra = extra.must(Cond::any(fields::KIND, q.kinds.iter().map(|k| k.as_str())))?;
        }
        if !q.languages.is_empty() {
            extra = extra.must(Cond::any(
                fields::LANGUAGE,
                q.languages.iter().map(|l| l.as_str()),
            ))?;
        }
        if let Some(s) = q.snapshot_id {
            extra = extra.must(Cond::keyword(fields::SNAPSHOT_IDS, s))?;
        }
        if !q.exclude_symbol_keys.is_empty() {
            extra = extra.must_not(Cond::any(fields::SYMBOL_KEY, &q.exclude_symbol_keys))?;
        }
        let filter = scoped(scope, &extra);
        let span = tracing::info_span!(
            "qdrant_search",
            organization_id = %scope.organization_id(),
            repositories = scope.repository_ids().len(),
            kinds = ?q.kinds,
            limit = q.limit,
            hits = tracing::field::Empty,
        );
        let threshold = (q.min_score > 0.0).then_some(q.min_score);
        let points = self
            .client
            .search(
                &targets.read,
                &vector,
                &filter,
                q.limit,
                threshold,
                self.hnsw_ef,
            )
            .instrument(span.clone())
            .await?;
        let mut hits = Vec::with_capacity(points.len());
        for p in points {
            if !payload_in_scope(scope, &p.payload) {
                report_violation("search hit", p.id);
                continue;
            }
            let Some(kind) =
                str_field(&p.payload, fields::KIND).and_then(|k| k.parse::<UnitKind>().ok())
            else {
                continue;
            };
            let Some(repository_id) = str_field(&p.payload, fields::REPOSITORY_ID)
                .and_then(|r| r.parse::<RepositoryId>().ok())
            else {
                continue;
            };
            hits.push(SemanticHit {
                point_id: p.id,
                score: p.score,
                kind,
                key: str_field(&p.payload, fields::UNIT_KEY).unwrap_or_default(),
                repository_id,
                symbol_key: str_field(&p.payload, fields::SYMBOL_KEY).and_then(|s| s.parse().ok()),
                file_path: str_field(&p.payload, fields::FILE_PATH),
                start_line: u32_field(&p.payload, fields::START_LINE),
                end_line: u32_field(&p.payload, fields::END_LINE),
                language: str_field(&p.payload, fields::LANGUAGE),
                content_hash: str_field(&p.payload, fields::CONTENT_HASH),
            });
        }
        span.record("hits", hits.len());
        Ok(hits)
    }

    /// Number of points inside `scope` matching `extra` in the read collection.
    pub async fn count(&self, scope: &TenantScope, extra: &ExtraFilter) -> Result<u64> {
        let targets = self.targets()?;
        Ok(self
            .client
            .count(&targets.read, &scoped(scope, extra))
            .await?)
    }

    /// Embeds and writes `units` unconditionally (no content-hash check; see
    /// [`crate::sync::sync`] for the incremental path). Every unit must belong to a repository
    /// of the scope, or nothing is written.
    pub async fn upsert_units(
        &self,
        scope: &TenantScope,
        units: &[EmbeddingUnit],
    ) -> Result<UpsertReport> {
        let targets = self.targets()?;
        for u in units {
            self.check_unit_scope(scope, u)?;
        }
        let refs: Vec<&EmbeddingUnit> = units.iter().collect();
        let (vectors, tokens) = self.embed_units(scope, &refs).await?;
        let mut points = Vec::with_capacity(units.len());
        for (u, vector) in units.iter().zip(vectors) {
            points.push(PointUpsert {
                id: self.point_id_of(scope, u),
                vector,
                payload: self.payload_for(scope, u, &[u.snapshot_id.to_string()])?,
            });
        }
        self.write_points(&targets, &points).await?;
        Ok(UpsertReport {
            upserted: points.len(),
            tokens,
        })
    }

    /// Deletes the selected units inside `scope` from every write collection.
    pub async fn delete_units(&self, scope: &TenantScope, sel: DeleteSelector) -> Result<()> {
        let targets = self.targets()?;
        let space_id = self.space().id();
        let extra = match sel {
            DeleteSelector::Units { kind, keys } => {
                if keys.is_empty() {
                    return Ok(());
                }
                let ids = scope
                    .repository_ids()
                    .iter()
                    .flat_map(|repo| {
                        keys.iter()
                            .map(|k| point_id(scope.organization_id(), *repo, kind, k, &space_id))
                    })
                    .collect();
                ExtraFilter::new().must(Cond::HasId(ids))?
            }
            DeleteSelector::SymbolKeys(keys) => {
                if keys.is_empty() {
                    return Ok(());
                }
                ExtraFilter::new().must(Cond::any(fields::SYMBOL_KEY, &keys))?
            }
            DeleteSelector::FilePaths(paths) => {
                if paths.is_empty() {
                    return Ok(());
                }
                ExtraFilter::new().must(Cond::any(fields::FILE_PATH, &paths))?
            }
            DeleteSelector::NotInSnapshots(live) => {
                if live.is_empty() {
                    return Err(Error::InvalidInput(
                        "garbage collection needs at least one live snapshot".into(),
                    ));
                }
                ExtraFilter::new().must_not(Cond::any(fields::SNAPSHOT_IDS, &live))?
            }
            DeleteSelector::All => ExtraFilter::new(),
        };
        let filter = scoped(scope, &extra);
        for c in &targets.write {
            self.client.delete_by_filter(c, &filter).await?;
        }
        Ok(())
    }

    // ---- crate-internal building blocks for sync (SEM-007) ----

    fn check_unit_scope(&self, scope: &TenantScope, unit: &EmbeddingUnit) -> Result<()> {
        if scope.contains(&unit.repository_id) {
            return Ok(());
        }
        metrics::scope_violation();
        tracing::error!(
            organization_id = %scope.organization_id(),
            repository_id = %unit.repository_id,
            "embedding unit outside the tenant scope rejected"
        );
        Err(Error::ScopeViolation(format!(
            "unit repository {} is not in the scope",
            unit.repository_id
        )))
    }

    /// Point payload. Tenant keys come from the scope; a unit outside it is rejected.
    pub(crate) fn payload_for(
        &self,
        scope: &TenantScope,
        unit: &EmbeddingUnit,
        snapshot_ids: &[String],
    ) -> Result<Payload> {
        self.check_unit_scope(scope, unit)?;
        let mut p = Payload::new();
        p.insert(
            fields::ORGANIZATION_ID.into(),
            json!(scope.organization_id().to_string()),
        );
        p.insert(
            fields::REPOSITORY_ID.into(),
            json!(unit.repository_id.to_string()),
        );
        p.insert(fields::KIND.into(), json!(unit.kind.as_str()));
        p.insert(fields::UNIT_KEY.into(), json!(unit.key));
        p.insert(
            fields::CONTENT_HASH.into(),
            json!(unit.content_hash.to_string()),
        );
        p.insert(fields::SNAPSHOT_IDS.into(), json!(snapshot_ids));
        p.insert(
            fields::EMBEDDING_VERSION.into(),
            json!(self.space().version),
        );
        p.insert("template_version".into(), json!(UNIT_TEMPLATE_VERSION));
        if matches!(unit.kind, UnitKind::CodeChunk | UnitKind::Doc) {
            p.insert(fields::CHUNK_KEY.into(), json!(unit.key));
        }
        if let Some(k) = &unit.symbol_key {
            p.insert(fields::SYMBOL_KEY.into(), json!(k.to_string()));
        }
        if let Some(l) = unit.language {
            p.insert(fields::LANGUAGE.into(), json!(l.as_str()));
        }
        if let Some(m) = &unit.module {
            p.insert(fields::MODULE.into(), json!(m));
        }
        if let Some(f) = &unit.file_path {
            p.insert(fields::FILE_PATH.into(), json!(f.as_str()));
        }
        if let Some(s) = unit.start_line {
            p.insert(fields::START_LINE.into(), json!(s));
        }
        if let Some(e) = unit.end_line {
            p.insert(fields::END_LINE.into(), json!(e));
        }
        Ok(p)
    }

    /// Embeds unit texts as documents in provider-sized batches.
    pub(crate) async fn embed_units(
        &self,
        scope: &TenantScope,
        units: &[&EmbeddingUnit],
    ) -> Result<(Vec<Vec<f32>>, u64)> {
        let mut vectors = Vec::with_capacity(units.len());
        let mut tokens: u64 = 0;
        let batch = self.provider.max_batch().max(1);
        for chunk in units.chunks(batch) {
            let texts: Vec<String> = chunk.iter().map(|u| u.text.clone()).collect();
            let resp = self
                .provider
                .embed(EmbedRequest {
                    kind: InputKind::Document,
                    texts: &texts,
                    trace: TraceContext::new().organization_id(scope.organization_id()),
                })
                .await?;
            tokens += u64::from(resp.usage_tokens);
            vectors.extend(resp.vectors);
        }
        Ok((vectors, tokens))
    }

    /// Upserts into every write collection. Payload tenant keys were set by [`Self::payload_for`].
    pub(crate) async fn write_points(
        &self,
        targets: &CollectionTargets,
        points: &[PointUpsert],
    ) -> Result<()> {
        if points.is_empty() {
            return Ok(());
        }
        for c in &targets.write {
            self.client.upsert(c, points).await?;
        }
        Ok(())
    }

    /// Content hash and snapshot ids of the given points in `collection`.
    pub(crate) async fn existing(
        &self,
        scope: &TenantScope,
        collection: &str,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ExistingPoint>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let filter = scoped(scope, &ExtraFilter::new().must(Cond::HasId(ids.to_vec()))?);
        const FIELDS: [&str; 4] = [
            fields::ORGANIZATION_ID,
            fields::REPOSITORY_ID,
            fields::CONTENT_HASH,
            fields::SNAPSHOT_IDS,
        ];
        let records = self
            .client
            .scroll_all(
                collection,
                &filter,
                WithPayload::Fields(&FIELDS),
                false,
                SCROLL_PAGE,
            )
            .await?;
        for r in records {
            if !payload_in_scope(scope, &r.payload) {
                report_violation("scrolled point", r.id);
                continue;
            }
            let snapshot_ids = r
                .payload
                .get(fields::SNAPSHOT_IDS)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            out.insert(
                r.id,
                ExistingPoint {
                    content_hash: str_field(&r.payload, fields::CONTENT_HASH),
                    snapshot_ids,
                },
            );
        }
        Ok(out)
    }

    /// Stored vectors (and payloads) of the given points in `collection`.
    pub(crate) async fn vectors(
        &self,
        scope: &TenantScope,
        collection: &str,
        ids: &[Uuid],
    ) -> Result<HashMap<Uuid, (Vec<f32>, Payload)>> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let filter = scoped(scope, &ExtraFilter::new().must(Cond::HasId(ids.to_vec()))?);
        let records = self
            .client
            .scroll_all(collection, &filter, WithPayload::All, true, SCROLL_PAGE)
            .await?;
        for r in records {
            if !payload_in_scope(scope, &r.payload) {
                report_violation("scrolled point", r.id);
                continue;
            }
            if let Some(v) = r.vector {
                out.insert(r.id, (v, r.payload));
            }
        }
        Ok(out)
    }

    /// Replaces `snapshot_ids` of the given points in `collection`.
    pub(crate) async fn set_snapshot_ids(
        &self,
        scope: &TenantScope,
        collection: &str,
        ids: &[Uuid],
        snapshot_ids: &[String],
    ) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let filter = scoped(scope, &ExtraFilter::new().must(Cond::HasId(ids.to_vec()))?);
        let mut payload = Payload::new();
        payload.insert(fields::SNAPSHOT_IDS.into(), json!(snapshot_ids));
        Ok(self
            .client
            .set_payload(collection, &filter, payload)
            .await?)
    }

    /// Deletes points by id inside `scope` from every write collection.
    pub(crate) async fn delete_ids(
        &self,
        scope: &TenantScope,
        targets: &CollectionTargets,
        ids: &[Uuid],
    ) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let filter = scoped(scope, &ExtraFilter::new().must(Cond::HasId(ids.to_vec()))?);
        for c in &targets.write {
            self.client.delete_by_filter(c, &filter).await?;
        }
        Ok(())
    }
}
