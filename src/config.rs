use std::{env, time::Duration};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub exports_dir: String,
    pub ngram_size: usize,
    /// Lowest n-gram order used for backoff during generation. Order 1
    /// (unigram) is always available as a final fallback, so this only
    /// controls the higher-order backoff depth.
    pub min_ngram_size: usize,
    pub max_generation_length: usize,
    pub generation_temperature: f32,
    pub generation_top_k: usize,
    pub generation_top_p: f32,
    /// Down-weighting of tokens already present in the generated sequence.
    pub repetition_penalty: f32,
    /// Number of candidate continuations sampled and reranked (best-of-N).
    pub num_candidates: usize,
    pub min_generation_tokens: usize,
    /// Probability (0.0–1.0) of replying to an incoming message with a random,
    /// chat-scoped message. Default 5%.
    pub reply_chance: f64,
    /// Username (without leading '@') of the bot whose every message is always
    /// answered, regardless of `reply_chance`. Default "cutalkshitbot"; an
    /// empty string disables the always-reply behavior.
    pub always_reply_to_username: String,
    /// Probability (0.0–1.0) that a reply is a sticker (picked to fit the
    /// message) instead of generated text. Default 10%.
    pub sticker_chance: f64,
    pub ingestion_interval: Duration,
    pub ingestion_workers: usize,
    /// How often the main loop logs a liveness heartbeat.
    pub heartbeat_interval: Duration,
    pub bot_token: Option<String>,
    /// When true, all store data lives in the process's RAM instead of
    /// PostgreSQL. Selected with `USE_INMEMORY_STORE=true`.
    pub use_inmemory_store: bool,
    /// When true (with a database), message history is stored in PostgreSQL
    /// while tokens and n-gram statistics live in an in-memory cache rebuilt
    /// from history on startup. Selected with `USE_HYBRID_MODE=true`. The
    /// default (no flags) is the pure-PostgreSQL store.
    pub use_hybrid_mode: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let use_inmemory_store = env::var("USE_INMEMORY_STORE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);
        let use_hybrid_mode = env::var("USE_HYBRID_MODE")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let database_url = if use_inmemory_store {
            String::new()
        } else {
            env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL is required to connect to PostgreSQL"))?
        };

        let exports_dir = env::var("EXPORTS_DIR").unwrap_or_else(|_| "exports".to_string());
        let ngram_size = env::var("NGRAM_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let min_ngram_size = env::var("MIN_NGRAM_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(2);
        let max_generation_length = env::var("MAX_GENERATION_LENGTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);
        let generation_temperature: f32 = env::var("GENERATION_TEMPERATURE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9);
        let generation_top_k = env::var("GENERATION_TOP_K")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let generation_top_p: f32 = env::var("GENERATION_TOP_P")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.9);
        let repetition_penalty: f32 = env::var("REPETITION_PENALTY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.3);
        let num_candidates = env::var("NUM_CANDIDATES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let min_generation_tokens = env::var("MIN_GENERATION_TOKENS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let reply_chance: f64 = env::var("REPLY_CHANCE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.05);
        // Username (without '@') of a bot to always answer. Unset defaults to
        // "cutalkshitbot"; an explicitly empty value disables the feature.
        let always_reply_to_username = env::var("ALWAYS_REPLY_TO_USERNAME")
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "cutalkshitbot".to_string());
        let sticker_chance: f64 = env::var("STICKER_CHANCE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.10);
        let ingestion_interval_secs = env::var("INGESTION_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let ingestion_workers = env::var("INGESTION_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        let heartbeat_interval_secs = env::var("HEARTBEAT_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(60);
        let bot_token = env::var("BOT_TOKEN").ok();

        Ok(Self {
            database_url,
            exports_dir,
            ngram_size: ngram_size.max(2),
            min_ngram_size: min_ngram_size.clamp(1, ngram_size.max(2)),
            max_generation_length,
            generation_temperature,
            generation_top_k,
            generation_top_p: generation_top_p.clamp(0.0, 1.0),
            repetition_penalty: repetition_penalty.max(1.0),
            num_candidates: num_candidates.max(1),
            min_generation_tokens,
            reply_chance: reply_chance.clamp(0.0, 1.0),
            always_reply_to_username,
            sticker_chance: sticker_chance.clamp(0.0, 1.0),
            ingestion_interval: Duration::from_secs(ingestion_interval_secs),
            ingestion_workers: ingestion_workers.max(1),
            heartbeat_interval: Duration::from_secs(heartbeat_interval_secs.max(5)),
            bot_token,
            use_inmemory_store,
            use_hybrid_mode,
        })
    }
}