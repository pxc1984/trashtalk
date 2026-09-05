mod application;
mod bot;
mod config;
mod domain;
mod infrastructure;
mod state;

use std::sync::Arc;

use anyhow::Context;
use state::AppState;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    install_panic_hook();

    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_target(false)
        .init();

    let config = config::Config::from_env().context("loading configuration")?;

    let store: infrastructure::store::SharedStore = if config.use_inmemory_store {
        info!("USE_INMEMORY_STORE=true; using in-memory (RAM) store");
        Arc::new(infrastructure::store::Store::InMemory(
            infrastructure::store::InMemoryStore::new(),
        ))
    } else {
        let pool = infrastructure::db::init_pool(&config.database_url)
            .await
            .context("connecting to database")?;

        infrastructure::db::apply_schema(&pool, std::path::Path::new("schema"))
            .await
            .context("applying local schema files")?;

        Arc::new(infrastructure::store::Store::Pg(infrastructure::store::PgStore::new(pool)))
    };

    let state = Arc::new(AppState::new(config, store));

    let ingestion_handles =
        infrastructure::ingestion::spawn_ingestion_workers(state.clone(), state.config.ingestion_workers);
    let bot_handle = bot::telegram_bot::spawn_bot(state.clone());

    info!("services started; keeping process alive");

    // Heartbeat: proves the process is alive and surfaces worker/bot threads
    // that silently died. Without it a code-0 exit or a dead worker thread
    // looks identical to "nothing happened".
    let heartbeat = state.config.heartbeat_interval;
    loop {
        tokio::time::sleep(heartbeat).await;

        let mut dead: Vec<String> = ingestion_handles
            .iter()
            .enumerate()
            .filter(|(_, h)| h.is_finished())
            .map(|(i, _)| format!("ingestion[{i}]"))
            .collect();

        if let Some(bot) = &bot_handle
            && bot.is_finished()
        {
            dead.push("bot".to_string());
        }

        if dead.is_empty() {
            info!(
                ingestion_workers = ingestion_handles.len(),
                bot = if bot_handle.is_some() { "running" } else { "disabled" },
                "alive"
            );
        } else {
            error!(threads = ?dead, "worker thread(s) exited; process may be degrading");
        }
    }
}

/// Logs any panic from any thread so it is not silently swallowed. Docker
/// captures stderr, so this also lands in `docker compose logs`.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.payload();
        let msg = if let Some(s) = msg.downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = msg.downcast_ref::<String>() {
            s.clone()
        } else {
            format!("{msg:?}")
        };
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        eprintln!("PANIC at {location}: {msg}");
        warn!(location, msg, "thread panicked");
    }));
}