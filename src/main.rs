mod application;
mod bot;
mod config;
mod domain;
mod infrastructure;
mod state;

use std::sync::Arc;

use anyhow::Context;
use state::AppState;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // stderr is unbuffered and always captured by Docker, so these markers
    // survive even an abrupt exit that would lose buffered stdout.
    eprintln!("[trashtalk] booting pid={}", std::process::id());

    install_panic_hook();
    debug!("panic hook installed");

    dotenvy::dotenv().ok();
    debug!("loaded .env (if present)");

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("debug")),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    info!("tracing initialized");

    info!("stage: loading configuration");
    let config = config::Config::from_env().context("loading configuration")?;
    debug!(
        use_inmemory_store = config.use_inmemory_store,
        database_url_set = !config.database_url.is_empty(),
        "configuration loaded"
    );

    info!("stage: initializing store");
    let store: infrastructure::store::SharedStore = if config.use_inmemory_store {
        info!("USE_INMEMORY_STORE=true; using in-memory (RAM) store");
        Arc::new(infrastructure::store::Store::InMemory(
            infrastructure::store::InMemoryStore::new(),
        ))
    } else {
        info!("stage: connecting to PostgreSQL");
        let pool = infrastructure::db::init_pool(&config.database_url)
            .await
            .context("connecting to database")?;
        debug!("connected to PostgreSQL");

        info!("stage: applying schema");
        infrastructure::db::apply_schema(&pool, std::path::Path::new("schema"))
            .await
            .context("applying local schema files")?;
        debug!("schema applied");

        // Hybrid backend: message history stays in PostgreSQL, while the token
        // vocabulary and n-gram statistics live in the in-memory cache.
        let store = infrastructure::store::Store::Hybrid(
            infrastructure::store::HybridStore::new(pool),
        );
        info!("stage: rebuilding in-memory cache from DB history");
        store
            .rebuild_cache(config.ngram_size, config.min_ngram_size)
            .await
            .context("rebuilding in-memory token/n-gram cache")?;
        debug!("in-memory cache rebuilt");
        Arc::new(store)
    };
    info!("store ready");

    let state = Arc::new(AppState::new(config, store));
    debug!("app state created");

    info!(
        workers = state.config.ingestion_workers,
        "stage: spawning ingestion workers"
    );
    let ingestion_handles =
        infrastructure::ingestion::spawn_ingestion_workers(state.clone(), state.config.ingestion_workers);
    debug!(count = ingestion_handles.len(), "ingestion workers spawned");

    info!("stage: spawning Telegram bot");
    let bot_handle = bot::telegram_bot::spawn_bot(state.clone());
    debug!(
        bot = if bot_handle.is_some() { "running" } else { "disabled" },
        "bot startup done"
    );

    info!("stage: entering main keep-alive loop");
    let heartbeat = state.config.heartbeat_interval;
    debug!(?heartbeat, "keep-alive loop started");
    eprintln!("[trashtalk] entering keep-alive loop (pid={})", std::process::id());

    // Heartbeat: proves the process is alive and surfaces worker/bot threads
    // that silently died. Without it a code-0 exit or a dead worker thread
    // looks identical to "nothing happened".
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