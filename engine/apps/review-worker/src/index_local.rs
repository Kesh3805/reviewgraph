//! `review-worker index-local`: index a checkout from the command line (IDX-001).
//!
//! Development driver used by tests, parity runs and benchmarks. The store is either
//! in-memory (`mem`, a dry run that reports statistics only) or Postgres (`pg`). The file store
//! of the CLI (GS-006) does not exist yet and is refused explicitly.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Context};
use graph_storage::mem::MemGraphStore;
use graph_storage::pg::PgGraphStore;
use graph_storage::types::SnapshotPurpose;
use graph_storage::GraphStore;
use pipeline::index::{
    typescript_registry, FullIndexRequest, FullIndexer, IndexConfig, IndexStats, NoopProgress,
    SourceSpec,
};
use repository::store::RepoScope;
use review_core::ids::{CommitSha, OrganizationId, RepositoryId};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// Where graphs are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StoreKind {
    Mem,
    Pg,
    File,
}

/// Arguments of the subcommand.
#[derive(Debug, Clone, clap::Args)]
pub struct IndexLocalArgs {
    /// Repository checkout to index.
    #[arg(long)]
    pub repo: PathBuf,
    /// Commit recorded on the snapshot (40 or 64 hex characters). Defaults to a digest of the
    /// repository path, which marks a working-tree index.
    #[arg(long, conflicts_with = "working_tree")]
    pub commit: Option<String>,
    /// Index the working tree as it is on disk (the default and only source).
    #[arg(long)]
    pub working_tree: bool,
    #[arg(long, value_enum, default_value = "mem")]
    pub store: StoreKind,
    /// Postgres connection string for `--store pg`.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    pub database_url: Option<String>,
    /// Tenant of the snapshot (required for `--store pg`).
    #[arg(long)]
    pub organization_id: Option<Uuid>,
    /// Repository of the snapshot (required for `--store pg`).
    #[arg(long)]
    pub repository_id: Option<Uuid>,
    /// Write the run statistics as JSON to this file.
    #[arg(long)]
    pub json_stats: Option<PathBuf>,
    /// Parse threads (default: available parallelism minus one).
    #[arg(long)]
    pub threads: Option<usize>,
}

fn working_tree_commit(root: &std::path::Path) -> anyhow::Result<CommitSha> {
    let canonical = std::fs::canonicalize(root).context("resolving --repo")?;
    let digest = blake3::hash(canonical.to_string_lossy().as_bytes());
    digest.to_hex().as_str()[..40]
        .parse::<CommitSha>()
        .context("deriving a commit marker")
}

/// Runs the index and returns its statistics.
pub async fn run(args: IndexLocalArgs) -> anyhow::Result<IndexStats> {
    let commit = match &args.commit {
        Some(sha) => sha.parse::<CommitSha>().context("--commit")?,
        None => working_tree_commit(&args.repo)?,
    };
    let (store, scope): (Arc<dyn GraphStore>, RepoScope) = match args.store {
        StoreKind::Mem => (
            Arc::new(MemGraphStore::new()),
            RepoScope {
                organization_id: OrganizationId::new(),
                repository_id: RepositoryId::new(),
            },
        ),
        StoreKind::Pg => {
            let url = args
                .database_url
                .as_deref()
                .context("--store pg needs --database-url or DATABASE_URL")?;
            let org = args
                .organization_id
                .context("--store pg needs --organization-id")?;
            let repo = args
                .repository_id
                .context("--store pg needs --repository-id")?;
            let pool = PgPoolOptions::new()
                .max_connections(4)
                .connect(url)
                .await
                .context("connecting to the database")?;
            (
                Arc::new(PgGraphStore::new(pool)),
                RepoScope {
                    organization_id: OrganizationId::from(org),
                    repository_id: RepositoryId::from(repo),
                },
            )
        }
        StoreKind::File => bail!("the file store is not available yet (GS-006); use mem or pg"),
    };

    let config = IndexConfig::default();
    let threads = args.threads.unwrap_or(config.parse_threads);
    let (registry, parsers) = typescript_registry();
    let indexer = FullIndexer::new(store, registry, threads)?.with_parser_versions(parsers);
    let mut request = FullIndexRequest::new(
        scope,
        commit,
        SourceSpec::WorkingTree {
            root: args.repo.clone(),
        },
        SnapshotPurpose::Local,
    );
    request.config = config;
    request.repository_name = args.repo.file_name().map_or_else(
        || "repository".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let outcome = indexer.run(request, &NoopProgress).await?;
    if let Some(path) = &args.json_stats {
        let json = serde_json::to_string_pretty(&outcome.stats)?;
        std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(outcome.stats)
}
