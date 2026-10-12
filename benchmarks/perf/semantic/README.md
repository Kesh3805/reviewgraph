# Semantic retrieval benchmark (SEM-009)

Validates ADR-008's single-collection-with-payload-filters design: filtered recall and latency of
Qdrant retrieval for a skewed multi-tenant corpus.

- **Corpus:** 20 organizations x 5 repositories, seeded synthetic identifier text embedded with the
  hash provider; one large organization, the others small; kinds 60% `code_chunk`, 30%
  `symbol_summary`, 10% `doc`. `CorpusSpec::small()` (CI: 20k-point large org, 500 points per other
  org, 256 dims, 100 queries) and `CorpusSpec::full()` (1M-point large org, 20k per other org,
  768 dims, 500 queries).
- **Scenarios:** (a) large organization, one repository; (b) small organization (selective filter);
  (c) large organization, `kind = doc` only. Every query goes through the tenant filter.
- **Ground truth:** brute-force cosine over the filtered subset, computed in Rust.
- **Metrics:** recall@10, recall@50, p50/p95/p99 latency at `hnsw_ef` 64/128/256 with 1 and 8
  concurrent clients.
- **Targets:** recall@10 >= 0.95 for every scenario at ef=128; p95 < 100 ms at 1M points on the
  reference VM. A scenario below target opens the ADR-008 revisit.

## Running

```sh
# needs a running Qdrant; uses and deletes the collection rg_bench_filtered
QDRANT_URL=http://127.0.0.1:6333 SEM_BENCH_REPORT_DIR=../benchmarks/perf/semantic/reports \
  cargo bench -p semantic --bench qdrant_filtered          # CI-sized corpus
SEM_BENCH_FULL=1 QDRANT_URL=... cargo bench -p semantic --bench qdrant_filtered   # 1M points
```

The harness (`semantic::bench`) refuses any collection outside the `rg_bench_` prefix and a
collection that already exists, so it never touches real data. The CI integration job runs the
small corpus as the test `semantic_benchmark_small_report` and prints the markdown report between
`SEMANTIC_BENCH_REPORT_BEGIN` and `SEMANTIC_BENCH_REPORT_END`. Reports are committed under
`reports/`.
