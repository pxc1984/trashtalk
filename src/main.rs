mod config;
mod db;
mod grpc;
mod state;
mod tokenizer;
mod trainer;
mod workers;

use std::sync::Arc;

use anyhow::Context;
use state::AppState;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_target(false)
        .init();

    let config = config::Config::from_env().context("loading configuration")?;
    let pool = db::init_pool(&config.database_url)
        .await
        .context("connecting to database")?;

    db::apply_schema(&pool, std::path::Path::new("schema"))
        .await
        .context("applying local schema files")?;

    let state = Arc::new(AppState::new(config, pool));

    let _ingestion_handles =
        workers::ingestion::spawn_ingestion_workers(state.clone(), state.config.ingestion_workers);
    let _bot_handle = workers::telegram_bot::spawn_bot(state.clone());

    workers::grpc_server::run_server(state.clone())
        .await
        .context("running gRPC server")
}
