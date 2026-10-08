//! `review semantic collections|activate|retire` (SEM-004).

use clap::Subcommand;
use semantic::collections::{activate, parse_age, retire, PgRegistry};
use semantic::{CollectionRegistry, QdrantConfig};
use sqlx::postgres::PgPoolOptions;

#[derive(Debug, Subcommand)]
pub enum SemanticAction {
    /// List registered vector collections and their state.
    Collections,
    /// Make a building collection active; the previous active one starts retiring.
    Activate {
        /// Collection name, for example rg_voyage_voyage_code_3_1024_v2.
        name: String,
    },
    /// Delete retiring collections older than the given age from Qdrant.
    Retire {
        /// Age such as 7d, 12h, 30m.
        #[arg(long, default_value = "7d")]
        older_than: String,
    },
}

pub fn run(database_url: &str, action: SemanticAction) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(database_url)
            .await?;
        let registry = PgRegistry::new(pool);
        match action {
            SemanticAction::Collections => {
                for c in registry.list().await? {
                    println!(
                        "{}\t{}\t{}\t{} dims\tv{}",
                        c.name, c.state, c.space_id, c.dims, c.version
                    );
                }
            }
            SemanticAction::Activate { name } => {
                let previous = activate(&registry, &name).await?;
                match previous {
                    Some(p) => println!("{name} is active; {p} is retiring"),
                    None => println!("{name} is active"),
                }
            }
            SemanticAction::Retire { older_than } => {
                let cfg = QdrantConfig::from_lookup(|k| std::env::var(k).ok())
                    .ok_or_else(|| anyhow::anyhow!("QDRANT_URL is not set"))?;
                for name in retire(&cfg, &registry, parse_age(&older_than)?).await? {
                    println!("retired {name}");
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    })
}
