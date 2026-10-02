use clap::{Parser, Subcommand};
use review_worker::migrate;

#[derive(Debug, Parser)]
#[command(name = "review-worker", about = "ReviewGraph pipeline worker")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Apply pending database migrations and exit.
    Migrate {
        /// Postgres connection string.
        #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
        database_url: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _telemetry = telemetry::init(telemetry::TelemetryConfig::from_env("review-worker")?)?;

    match Cli::parse().command {
        Command::Migrate { database_url } => {
            let applied = migrate::run_from_url(&database_url).await?;
            if applied.is_empty() {
                println!("database is up to date");
            } else {
                println!("applied {} migration(s)", applied.len());
            }
            Ok(())
        }
    }
}
