mod application;
mod bot;
mod config;
mod domain;
mod infrastructure;
mod state;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use state::AppState;
use tracing::info;
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
    let pool = infrastructure::db::init_pool(&config.database_url)
        .await
        .context("connecting to database")?;

    infrastructure::db::apply_schema(&pool, std::path::Path::new("schema"))
        .await
        .context("applying local schema files")?;

    let state = Arc::new(AppState::new(config, pool));

    let _ingestion_handles =
        infrastructure::ingestion::spawn_ingestion_workers(state.clone(), state.config.ingestion_workers);
    let _bot_handle = bot::telegram_bot::spawn_bot(state.clone());

    info!("services started; keeping process alive");

    // The bot runs in its own thread and the ingestion workers loop forever, so
    // main only needs to stay alive.
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}