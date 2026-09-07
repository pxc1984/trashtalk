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
        let bin =
            std::path::Path::new(&config.exports_dir).join(infrastructure::store::TRASHTALK_BIN);
        let mem = match infrastructure::store::InMemoryStore::load_from_file(&bin) {
            Ok(Some(m)) => {
                info!(path = %bin.display(), "loaded in-memory store from trashtalk.bin");
                m
            }
            Ok(None) => infrastructure::store::InMemoryStore::new(),
            Err(err) => {
                warn!(
                    error = ?err,
                    path = %bin.display(),
                    "failed to load in-memory store; starting empty"
                );
                infrastructure::store::InMemoryStore::new()
            }
        };
        Arc::new(infrastructure::store::Store::InMemory(mem))
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

        if config.use_hybrid_mode {
            info!("USE_HYBRID_MODE=true; history in PostgreSQL, tokens/n-grams in RAM");
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
        } else {
            info!("using pure PostgreSQL store");
            Arc::new(infrastructure::store::Store::Pg(infrastructure::store::PgStore::new(pool)))
        }
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
    let heartbeat_interval = state.config.heartbeat_interval;
    debug!(?heartbeat_interval, "keep-alive loop started");
    eprintln!("[trashtalk] entering keep-alive loop (pid={})", std::process::id());

    // Heartbeat: proves the process is alive and surfaces worker/bot threads
    // that silently died. Runs as a background task; it also snapshots the
    // in-memory store every tick, and the main flow waits for a shutdown
    // signal so it can persist the store one last time.
    let heartbeat_store = state.store.clone();
    let heartbeat_exports_dir = state.config.exports_dir.clone();
    let heartbeat = tokio::spawn(async move {
        run_heartbeat_loop(
            heartbeat_store,
            heartbeat_exports_dir,
            ingestion_handles,
            bot_handle,
            heartbeat_interval,
        )
        .await;
    });

    info!("stage: awaiting shutdown signal (Ctrl+C / SIGTERM)");
    shutdown_signal().await;

    info!("stage: graceful shutdown");
    if state.config.use_inmemory_store {
        state
            .store
            .save_to_disk(&state.config.exports_dir)
            .await
            .context("saving in-memory store on shutdown")?;
        info!("in-memory store saved to exports dir");
    }

    heartbeat.abort();
    info!("shutdown complete");
    Ok(())
}

/// Logs a liveness heartbeat every `interval`, flagging any worker/bot threads
/// that exited. Also snapshots the in-memory store (`trashtalk.bin`) on every
/// tick so a crash loses at most one interval of training. Runs forever.
async fn run_heartbeat_loop(
    store: infrastructure::store::SharedStore,
    exports_dir: String,
    ingestion_handles: Vec<std::thread::JoinHandle<()>>,
    bot_handle: Option<std::thread::JoinHandle<()>>,
    interval: Duration,
) {
    loop {
        tokio::time::sleep(interval).await;

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

        // Dump the in-memory store every heartbeat so a crash loses at most
        // one interval. A no-op for the PostgreSQL and hybrid backends.
        if let Err(err) = store.save_to_disk(&exports_dir).await {
            warn!(error = ?err, "failed to dump in-memory store on heartbeat");
        }
    }
}

/// Resolves when the process should shut down gracefully: Ctrl+C or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("installing Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => info!("received Ctrl+C"),
        _ = terminate => info!("received SIGTERM"),
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