use std::sync::Arc;

use crate::config::Config;
use crate::infrastructure::store::SharedStore;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub store: SharedStore,
    pub ngram_size: usize,
    pub min_ngram_size: usize,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(config: Config, store: SharedStore) -> Self {
        Self {
            ngram_size: config.ngram_size,
            min_ngram_size: config.min_ngram_size,
            config,
            store,
        }
    }
}