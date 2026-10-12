//! Filtered-recall and latency benchmark harness (SEM-009).
//!
//! Builds a seeded multi-tenant corpus of hash-provider vectors in a dedicated `rg_bench_*`
//! collection, computes exact top-k per query by brute force over the filtered subset, and
//! measures recall@10 / recall@50 and latency percentiles at several `hnsw_ef` values.
//! It refuses any collection outside the `rg_bench_` prefix and deletes its collection at the end.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use review_core::ids::{OrganizationId, RepositoryId};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::embedding::hash::embed_text;
use crate::error::{Error, Result};
use crate::filter::{fields, Cond};
use crate::qdrant::types::PointUpsert;
use crate::qdrant::{HnswCfg, Payload, QdrantClient, QdrantConfig};
use crate::tenant::{scoped, ExtraFilter, NonEmptyVec, TenantScope};

/// Every benchmark collection starts with this.
pub const BENCH_PREFIX: &str = "rg_bench_";

/// Rejects names outside the benchmark prefix (never touch real collections).
pub fn check_bench_collection(name: &str) -> Result<()> {
    if name.starts_with(BENCH_PREFIX) && name.len() > BENCH_PREFIX.len() {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "benchmark collections must start with {BENCH_PREFIX}, got {name:?}"
        )))
    }
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

/// Exact cosine top-`k` (vectors are unit length, so dot = cosine). Ties break by index.
pub fn brute_force_top_k(corpus: &[&[f32]], query: &[f32], k: usize) -> Vec<usize> {
    let mut scored: Vec<(usize, f64)> = corpus
        .iter()
        .enumerate()
        .map(|(i, v)| (i, dot(v, query)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().take(k).map(|(i, _)| i).collect()
}

/// `|truth[..k] ∩ got[..k]| / min(k, |truth|)`; 1.0 when the truth is empty.
pub fn recall_at_k<T: Eq + std::hash::Hash>(truth: &[T], got: &[T], k: usize) -> f64 {
    let truth: HashSet<&T> = truth.iter().take(k).collect();
    if truth.is_empty() {
        return 1.0;
    }
    let hit = got.iter().take(k).filter(|g| truth.contains(g)).count();
    hit as f64 / truth.len() as f64
}

/// Nearest-rank percentile of `values` (sorted in place); 0 for an empty slice.
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * values.len() as f64).ceil() as usize;
    values[rank.clamp(1, values.len()) - 1]
}

/// splitmix64: seeded, reproducible corpus generation.
#[derive(Debug, Clone)]
pub struct SplitMix(u64);

impl SplitMix {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

const WORDS: [&str; 48] = [
    "user",
    "auth",
    "token",
    "session",
    "order",
    "payment",
    "invoice",
    "report",
    "admin",
    "permission",
    "role",
    "cache",
    "queue",
    "job",
    "event",
    "handler",
    "controller",
    "service",
    "repository",
    "entity",
    "config",
    "parse",
    "validate",
    "format",
    "render",
    "load",
    "save",
    "update",
    "delete",
    "create",
    "find",
    "list",
    "check",
    "authorize",
    "notify",
    "email",
    "upload",
    "file",
    "stream",
    "batch",
    "retry",
    "lock",
    "audit",
    "metric",
    "trace",
    "graph",
    "node",
    "edge",
];

/// A synthetic identifier-heavy code text.
pub fn synthetic_text(rng: &mut SplitMix) -> String {
    let n = 6 + rng.below(10);
    let mut parts = Vec::with_capacity(n);
    for _ in 0..n {
        let a = WORDS[rng.below(WORDS.len())];
        let b = WORDS[rng.below(WORDS.len())];
        parts.push(format!("{a}{}{}", b[..1].to_uppercase(), &b[1..]));
    }
    parts.join(" ")
}

/// Corpus shape.
#[derive(Debug, Clone, Serialize)]
pub struct CorpusSpec {
    pub orgs: usize,
    pub repos_per_org: usize,
    /// Points of the first (large) organization.
    pub large_org_points: usize,
    /// Points of every other organization.
    pub small_org_points: usize,
    pub dims: u16,
    pub queries: usize,
    pub seed: u64,
}

impl CorpusSpec {
    /// CI-sized corpus.
    pub fn small() -> Self {
        Self {
            orgs: 20,
            repos_per_org: 5,
            large_org_points: 20_000,
            small_org_points: 500,
            dims: 256,
            queries: 100,
            seed: 0x5345_4d30_3039,
        }
    }

    /// The full SEM-009 corpus (1M-point large org).
    pub fn full() -> Self {
        Self {
            large_org_points: 1_000_000,
            small_org_points: 20_000,
            dims: 768,
            queries: 500,
            ..Self::small()
        }
    }
}

struct BenchPoint {
    org: usize,
    repo: usize,
    kind: &'static str,
    vector: Vec<f32>,
}

/// One scenario result.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub scenario: String,
    pub hnsw_ef: u32,
    pub concurrency: usize,
    pub filtered_points: usize,
    pub recall_at_10: f64,
    pub recall_at_50: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
}

/// Full benchmark output.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    pub corpus: CorpusSpec,
    pub total_points: usize,
    pub load_seconds: f64,
    pub results: Vec<ScenarioResult>,
}

impl BenchReport {
    /// Markdown table for `benchmarks/perf/semantic/reports/`.
    pub fn markdown(&self) -> String {
        let mut s = format!(
            "# Semantic retrieval benchmark\n\nCorpus: {} orgs x {} repos, large org {} points, \
             other orgs {} points each, {} dims, {} queries per scenario, {} points total \
             (load {:.1} s).\n\n| scenario | ef | clients | filtered points | recall@10 | \
             recall@50 | p50 ms | p95 ms | p99 ms |\n|---|---|---|---|---|---|---|---|---|\n",
            self.corpus.orgs,
            self.corpus.repos_per_org,
            self.corpus.large_org_points,
            self.corpus.small_org_points,
            self.corpus.dims,
            self.corpus.queries,
            self.total_points,
            self.load_seconds
        );
        for r in &self.results {
            s.push_str(&format!(
                "| {} | {} | {} | {} | {:.3} | {:.3} | {:.1} | {:.1} | {:.1} |\n",
                r.scenario,
                r.hnsw_ef,
                r.concurrency,
                r.filtered_points,
                r.recall_at_10,
                r.recall_at_50,
                r.p50_ms,
                r.p95_ms,
                r.p99_ms
            ));
        }
        s
    }
}

/// Benchmark run against a live Qdrant.
#[derive(Debug)]
pub struct Bench {
    client: QdrantClient,
    collection: String,
}

impl Bench {
    pub fn new(cfg: QdrantConfig, collection: impl Into<String>) -> Result<Self> {
        let collection = collection.into();
        check_bench_collection(&collection)?;
        Ok(Self {
            client: QdrantClient::new(cfg)?,
            collection,
        })
    }

    /// Loads the corpus, runs every scenario, deletes the collection.
    pub async fn run(&self, spec: &CorpusSpec) -> Result<BenchReport> {
        let result = self.run_inner(spec).await;
        let cleanup = self.client.delete_collection(&self.collection).await;
        let report = result?;
        cleanup?;
        Ok(report)
    }

    async fn run_inner(&self, spec: &CorpusSpec) -> Result<BenchReport> {
        let state = self
            .client
            .ensure_collection(&self.collection, spec.dims, HnswCfg::default())
            .await?;
        if !state.created || state.points_count.unwrap_or(0) > 0 {
            return Err(Error::InvalidInput(format!(
                "benchmark collection {} must not exist before the run",
                self.collection
            )));
        }
        for (field, schema) in fields::INDEXED {
            self.client
                .ensure_payload_index(
                    &self.collection,
                    field,
                    schema,
                    field == fields::ORGANIZATION_ID,
                )
                .await?;
        }
        let orgs: Vec<OrganizationId> = (0..spec.orgs).map(|_| OrganizationId::new()).collect();
        let repos: Vec<Vec<RepositoryId>> = (0..spec.orgs)
            .map(|_| {
                (0..spec.repos_per_org)
                    .map(|_| RepositoryId::new())
                    .collect()
            })
            .collect();
        let mut rng = SplitMix::new(spec.seed);
        let mut points: Vec<BenchPoint> = Vec::new();
        for org in 0..spec.orgs {
            let n = if org == 0 {
                spec.large_org_points
            } else {
                spec.small_org_points
            };
            for _ in 0..n {
                let roll = rng.below(10);
                let kind = match roll {
                    0..=5 => "code_chunk",
                    6..=8 => "symbol_summary",
                    _ => "doc",
                };
                points.push(BenchPoint {
                    org,
                    repo: rng.below(spec.repos_per_org),
                    kind,
                    vector: embed_text(&synthetic_text(&mut rng), spec.dims),
                });
            }
        }
        let started = Instant::now();
        let client = Arc::new(self.client.clone());
        let mut tasks = tokio::task::JoinSet::new();
        for (batch_no, batch) in points.chunks(1024).enumerate() {
            let upserts: Vec<PointUpsert> = batch
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut payload = Payload::new();
                    payload.insert(
                        fields::ORGANIZATION_ID.into(),
                        json!(orgs[p.org].to_string()),
                    );
                    payload.insert(
                        fields::REPOSITORY_ID.into(),
                        json!(repos[p.org][p.repo].to_string()),
                    );
                    payload.insert(fields::KIND.into(), json!(p.kind));
                    PointUpsert {
                        id: Uuid::from_u128((batch_no * 1024 + i) as u128 + 1),
                        vector: p.vector.clone(),
                        payload,
                    }
                })
                .collect();
            let c = Arc::clone(&client);
            let name = self.collection.clone();
            tasks.spawn(async move { c.upsert(&name, &upserts).await });
            while tasks.len() >= 8 {
                if let Some(r) = tasks.join_next().await {
                    r.map_err(|e| Error::InvalidInput(format!("load task: {e}")))??;
                }
            }
        }
        while let Some(r) = tasks.join_next().await {
            r.map_err(|e| Error::InvalidInput(format!("load task: {e}")))??;
        }
        let load_seconds = started.elapsed().as_secs_f64();

        // (name, scope, kind filter, member predicate)
        type Member = Box<dyn Fn(&BenchPoint) -> bool>;
        let large_repo = repos[0][0];
        let small_org = spec.orgs.saturating_sub(1);
        let scenarios: Vec<(&str, TenantScope, Option<&str>, Member)> = vec![
            (
                "large_org_one_repo",
                TenantScope::single(orgs[0], large_repo),
                None,
                Box::new(|p: &BenchPoint| p.org == 0 && p.repo == 0) as Member,
            ),
            (
                "small_org_all_repos",
                TenantScope::new(
                    orgs[small_org],
                    NonEmptyVec::new(repos[small_org].clone())
                        .ok_or_else(|| Error::InvalidInput("no repositories".into()))?,
                ),
                None,
                Box::new(move |p: &BenchPoint| p.org == small_org) as Member,
            ),
            (
                "large_org_kind_doc",
                TenantScope::new(
                    orgs[0],
                    NonEmptyVec::new(repos[0].clone())
                        .ok_or_else(|| Error::InvalidInput("no repositories".into()))?,
                ),
                Some("doc"),
                Box::new(|p: &BenchPoint| p.org == 0 && p.kind == "doc") as Member,
            ),
        ];
        let mut results = Vec::new();
        for (name, scope, kind, member) in &scenarios {
            let subset: Vec<&BenchPoint> = points.iter().filter(|p| member(*p)).collect();
            let vectors: Vec<&[f32]> = subset.iter().map(|p| p.vector.as_slice()).collect();
            let mut extra = ExtraFilter::new();
            if let Some(k) = kind {
                extra = extra.must(Cond::keyword(fields::KIND, k))?;
            }
            let filter = scoped(scope, &extra);
            let queries: Vec<Vec<f32>> = (0..spec.queries)
                .map(|_| embed_text(&synthetic_text(&mut rng), spec.dims))
                .collect();
            let truths: Vec<Vec<usize>> = queries
                .iter()
                .map(|q| brute_force_top_k(&vectors, q, 50))
                .collect();
            // Map point id -> subset index for comparing results.
            let ids: Vec<Uuid> = points
                .iter()
                .enumerate()
                .filter(|(_, p)| member(*p))
                .map(|(i, _)| Uuid::from_u128(i as u128 + 1))
                .collect();
            for ef in [64u32, 128, 256] {
                for concurrency in [1usize, 8] {
                    let mut latencies = Vec::with_capacity(queries.len());
                    let (mut r10, mut r50) = (0.0, 0.0);
                    for (chunk_no, chunk) in queries.chunks(concurrency).enumerate() {
                        let futs = chunk.iter().map(|q| {
                            let filter = &filter;
                            async move {
                                let t = Instant::now();
                                let hits = self
                                    .client
                                    .search(&self.collection, q, filter, 50, None, Some(ef))
                                    .await?;
                                Ok::<_, Error>((t.elapsed().as_secs_f64() * 1000.0, hits))
                            }
                        });
                        let outs = futures::future::try_join_all(futs).await?;
                        for (j, (ms, hits)) in outs.into_iter().enumerate() {
                            latencies.push(ms);
                            let truth: Vec<Uuid> = truths[chunk_no * concurrency + j]
                                .iter()
                                .map(|i| ids[*i])
                                .collect();
                            let got: Vec<Uuid> = hits.iter().map(|h| h.id).collect();
                            r10 += recall_at_k(&truth, &got, 10);
                            r50 += recall_at_k(&truth, &got, 50);
                        }
                    }
                    let n = queries.len().max(1) as f64;
                    results.push(ScenarioResult {
                        scenario: (*name).to_owned(),
                        hnsw_ef: ef,
                        concurrency,
                        filtered_points: subset.len(),
                        recall_at_10: r10 / n,
                        recall_at_50: r50 / n,
                        p50_ms: percentile(&mut latencies, 50.0),
                        p95_ms: percentile(&mut latencies, 95.0),
                        p99_ms: percentile(&mut latencies, 99.0),
                    });
                }
            }
        }
        Ok(BenchReport {
            corpus: spec.clone(),
            total_points: points.len(),
            load_seconds,
            results,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ground_truth_bruteforce_small_corpus() {
        let a = [1.0f32, 0.0];
        let b = [0.0f32, 1.0];
        let c = [0.6f32, 0.8];
        let corpus: Vec<&[f32]> = vec![&a, &b, &c];
        assert_eq!(brute_force_top_k(&corpus, &[1.0, 0.0], 2), vec![0, 2]);
        assert_eq!(brute_force_top_k(&corpus, &[0.0, 1.0], 3), vec![1, 2, 0]);
        assert!((recall_at_k(&[0, 2], &[2, 1], 2) - 0.5).abs() < 1e-9);
        assert!((recall_at_k::<u8>(&[], &[1], 10) - 1.0).abs() < 1e-9);
        let mut lat = vec![5.0, 1.0, 3.0, 2.0, 4.0];
        assert_eq!(percentile(&mut lat, 50.0), 3.0);
        assert_eq!(percentile(&mut lat, 95.0), 5.0);
    }

    #[test]
    fn bench_collection_prefix_enforced() {
        assert!(check_bench_collection("rg_bench_semantic").is_ok());
        assert!(check_bench_collection("rg_hash_fh768_768_v1").is_err());
        assert!(check_bench_collection("rg_bench_").is_err());
        assert!(Bench::new(QdrantConfig::new("http://127.0.0.1:1"), "rg_openai_x_v1").is_err());
    }

    #[test]
    fn synthetic_corpus_is_seeded() {
        let mut a = SplitMix::new(7);
        let mut b = SplitMix::new(7);
        assert_eq!(synthetic_text(&mut a), synthetic_text(&mut b));
    }
}
