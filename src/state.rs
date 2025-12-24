use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: PgPool,
    pub ngram_size: usize,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(config: Config, pool: PgPool) -> Self {
        Self {
            ngram_size: config.ngram_size,
            config,
            pool,
        }
    }
}
