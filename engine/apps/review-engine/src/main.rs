use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use review_engine::auth::ServiceKeyRing;
use review_engine::backend::PgBackend;
use review_engine::state::DEFAULT_GRAPH_CACHE;
use review_engine::{router, AppState, EngineSettings};
use sqlx::postgres::PgPoolOptions;

/// Time the server gets to finish in-flight requests after a shutdown signal.
const DRAIN: Duration = Duration::from_secs(10);

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// `SERVICE_JWT_KEYS` (`kid:base64,...`), or `SERVICE_JWT_SECRET` as kid `default`: the same
/// precedence the API uses to sign.
fn key_ring() -> anyhow::Result<ServiceKeyRing> {
    if let Some(spec) = env("SERVICE_JWT_KEYS") {
        return ServiceKeyRing::parse(&spec).map_err(anyhow::Error::msg);
    }
    let secret =
        env("SERVICE_JWT_SECRET").context("SERVICE_JWT_KEYS or SERVICE_JWT_SECRET must be set")?;
    ServiceKeyRing::single("default", secret.as_bytes()).map_err(anyhow::Error::msg)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _telemetry = telemetry::init(telemetry::TelemetryConfig::from_env("review-engine")?)?;

    let database_url = env("DATABASE_URL").context("DATABASE_URL must be set")?;
    let bind: SocketAddr = env("ENGINE_BIND")
        .unwrap_or_else(|| "0.0.0.0:8081".to_owned())
        .parse()
        .context("ENGINE_BIND must be host:port")?;
    let graph_cache = env("ENGINE_GRAPH_CACHE")
        .map(|raw| raw.parse::<usize>())
        .transpose()
        .context("ENGINE_GRAPH_CACHE must be a number")?
        .unwrap_or(DEFAULT_GRAPH_CACHE);
    let mirror_root = env("ENGINE_MIRROR_DIR").map(PathBuf::from);

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .context("connecting to the database")?;
    let settings = EngineSettings {
        graph_cache,
        ..EngineSettings::default()
    };
    let state = AppState::new(
        key_ring()?,
        Arc::new(PgBackend::new(pool, mirror_root)),
        settings,
    );

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("binding {bind}"))?;
    tracing::info!(%bind, graph_cache, "review-engine listening");

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = axum::serve(listener, router(state)).with_graceful_shutdown(async move {
        let _ = stop_rx.await;
    });
    let mut serving = tokio::spawn(async move { server.await });

    tokio::select! {
        result = &mut serving => {
            // The server stopped on its own: surface why.
            return result.context("server task")?.context("server error");
        }
        () = shutdown_signal() => {}
    }
    tracing::info!("shutdown requested, draining");
    let _ = stop_tx.send(());
    match tokio::time::timeout(DRAIN, &mut serving).await {
        Ok(result) => result.context("server task")?.context("server error"),
        Err(_) => {
            tracing::warn!("drain timed out, exiting");
            serving.abort();
            Ok(())
        }
    }
}
