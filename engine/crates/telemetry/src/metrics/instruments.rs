//! The metric catalog: every instrument, its type, unit, allowed labels and meaning. This table
//! is the single definition; the registry, `docs/operations/metrics-catalog.md` (rendered by
//! `cargo run -p xtask -- metrics-catalog`) and the TypeScript twin all follow it.

/// Instrument type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Counter,
    Histogram,
    Gauge,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Counter => "counter",
            Kind::Histogram => "histogram",
            Kind::Gauge => "gauge",
        }
    }
}

/// One instrument definition.
#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub name: &'static str,
    pub kind: Kind,
    /// UCUM-style unit: `1` for counts, `s`, `ratio`, `usd_micros`.
    pub unit: &'static str,
    /// The only labels this instrument accepts (a subset of `labels::ALLOWED`).
    pub labels: &'static [&'static str],
    pub description: &'static str,
}

const fn c(name: &'static str, labels: &'static [&'static str], description: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Counter,
        unit: "1",
        labels,
        description,
    }
}

const fn h(name: &'static str, labels: &'static [&'static str], description: &'static str) -> Spec {
    Spec {
        name,
        kind: Kind::Histogram,
        unit: "s",
        labels,
        description,
    }
}

const fn g(
    name: &'static str,
    unit: &'static str,
    labels: &'static [&'static str],
    description: &'static str,
) -> Spec {
    Spec {
        name,
        kind: Kind::Gauge,
        unit,
        labels,
        description,
    }
}

const LLM: &[&str] = &["provider", "model", "task", "tier"];
const FINDING: &[&str] = &["reviewer_type", "category", "severity"];

/// Every metric, in documentation order.
pub const CATALOG: &[Spec] = &[
    h(
        "review_duration_seconds",
        &["outcome"],
        "Wall time of one review run",
    ),
    h(
        "incremental_index_duration_seconds",
        &["language"],
        "Wall time of an incremental graph update",
    ),
    c(
        "files_reparsed_total",
        &["language"],
        "Files re-parsed by incremental indexing",
    ),
    c(
        "symbols_changed_total",
        &["language"],
        "Symbols added, removed or changed",
    ),
    g(
        "graph_nodes_total",
        "1",
        &["language"],
        "Graph nodes in the latest snapshot",
    ),
    g(
        "graph_edges_total",
        "1",
        &["language"],
        "Graph edges in the latest snapshot",
    ),
    c(
        "graph_invalidations_total",
        &["stage"],
        "Cached graph facts invalidated",
    ),
    c(
        "context_tokens_total",
        &["reviewer_type"],
        "Tokens of context selected for reviewers",
    ),
    c(
        "context_symbols_total",
        &["reviewer_type"],
        "Symbols included in selected context",
    ),
    c(
        "qdrant_queries_total",
        &["kind", "outcome"],
        "Qdrant requests",
    ),
    h(
        "qdrant_query_duration_seconds",
        &["kind"],
        "Qdrant request latency",
    ),
    Spec {
        name: "llm_requests_total",
        kind: Kind::Counter,
        unit: "1",
        labels: &["provider", "model", "task", "tier", "outcome"],
        description: "Model requests",
    },
    Spec {
        name: "llm_input_tokens_total",
        kind: Kind::Counter,
        unit: "1",
        labels: LLM,
        description: "Input tokens sent",
    },
    Spec {
        name: "llm_output_tokens_total",
        kind: Kind::Counter,
        unit: "1",
        labels: LLM,
        description: "Output tokens received",
    },
    Spec {
        name: "llm_cached_tokens_total",
        kind: Kind::Counter,
        unit: "1",
        labels: LLM,
        description: "Input tokens served from the provider cache",
    },
    Spec {
        name: "llm_cost_estimate",
        kind: Kind::Counter,
        unit: "usd_micros",
        labels: LLM,
        description: "Estimated model spend in millionths of a US dollar",
    },
    Spec {
        name: "candidate_findings_total",
        kind: Kind::Counter,
        unit: "1",
        labels: FINDING,
        description: "Candidate findings generated",
    },
    Spec {
        name: "verified_findings_total",
        kind: Kind::Counter,
        unit: "1",
        labels: FINDING,
        description: "Findings that passed verification",
    },
    Spec {
        name: "published_findings_total",
        kind: Kind::Counter,
        unit: "1",
        labels: FINDING,
        description: "Findings published to the pull request",
    },
    Spec {
        name: "suppressed_findings_total",
        kind: Kind::Counter,
        unit: "1",
        labels: &["reviewer_type", "category", "severity", "reason"],
        description: "Findings suppressed before publication",
    },
    g(
        "finding_acceptance_rate",
        "ratio",
        &["reviewer_type"],
        "Trailing 7-day acceptance ratio",
    ),
    g(
        "finding_false_positive_rate",
        "ratio",
        &["reviewer_type"],
        "Trailing 7-day false-positive ratio",
    ),
    g(
        "queue_depth",
        "1",
        &["queue", "state"],
        "Jobs per queue and state",
    ),
    h(
        "queue_wait_seconds",
        &["queue"],
        "Time a job waited before being claimed",
    ),
    h(
        "worker_duration_seconds",
        &["queue", "outcome"],
        "Time a worker spent on one job",
    ),
    c(
        "telemetry_export_failures_total",
        &[],
        "Failed OTLP exports",
    ),
    c(
        "webhook_signature_failures_total",
        &[],
        "Webhook deliveries with an invalid signature",
    ),
    c(
        "qdrant_scope_violation_total",
        &[],
        "Qdrant requests rejected for a missing tenant scope",
    ),
    c(
        "tenancy_denied_total",
        &[],
        "Requests denied by the tenancy guard",
    ),
    c(
        "trace_context_missing_total",
        &["queue"],
        "Jobs claimed without a valid traceparent",
    ),
    c(
        "log_redactions_total",
        &["pattern"],
        "Secret matches redacted from logs and span attributes",
    ),
    c(
        "metrics_refresh_failures_total",
        &[],
        "Failed background gauge refresh queries",
    ),
    c(
        "secret_findings_total",
        &["pattern"],
        "Likely secrets detected while scanning repositories",
    ),
    c(
        "secret_scan_skipped_total",
        &["reason"],
        "Files whose secret scan was skipped",
    ),
    c(
        "llm_redactions_total",
        &["pattern"],
        "Secret matches redacted before a model call",
    ),
    c(
        "llm_redaction_tripwire_total",
        &[],
        "Outgoing model requests that still matched a secret pattern",
    ),
    c(
        "llm_blocked_requests_total",
        &["reason"],
        "Model requests blocked by policy or the tripwire",
    ),
    c(
        "github_token_mint_total",
        &["outcome"],
        "GitHub installation token mint attempts",
    ),
    c(
        "clone_credentials_issued_total",
        &[],
        "Short-lived clone credentials issued to workers",
    ),
    c(
        "token_cache_decrypt_failures_total",
        &[],
        "Token cache entries that failed to decrypt",
    ),
    c(
        "retention_objects_deleted_total",
        &["policy"],
        "Objects deleted by the retention sweeper",
    ),
    h(
        "retention_sweep_duration_seconds",
        &["outcome"],
        "Wall time of one retention sweep",
    ),
    c(
        "checkout_wipe_failures_total",
        &[],
        "Checkout directories that could not be removed",
    ),
    c(
        "object_store_requests_total",
        &["op", "outcome"],
        "Object store requests",
    ),
    c(
        "object_store_denied_total",
        &[],
        "Object store requests denied or hidden by tenant scoping",
    ),
];

/// The spec named `name`.
pub fn find(name: &str) -> Option<&'static Spec> {
    CATALOG.iter().find(|s| s.name == name)
}

/// Metrics shared between the Rust and TypeScript registries; `apps/api` owns the TS side of
/// these names and a test asserts the two stay equal.
pub const SHARED_WITH_TYPESCRIPT: &[&str] = &[
    "queue_depth",
    "queue_wait_seconds",
    "published_findings_total",
    "finding_acceptance_rate",
    "finding_false_positive_rate",
    "webhook_signature_failures_total",
    "tenancy_denied_total",
    "trace_context_missing_total",
    "log_redactions_total",
    "github_token_mint_total",
    "clone_credentials_issued_total",
    "token_cache_decrypt_failures_total",
    "object_store_requests_total",
    "object_store_denied_total",
];
