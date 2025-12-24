use std::{env, net::SocketAddr, time::Duration};

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub grpc_addr: SocketAddr,
    pub exports_dir: String,
    pub ngram_size: usize,
    pub max_generation_length: usize,
    pub ingestion_interval: Duration,
    pub ingestion_workers: usize,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = env::var("DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("DATABASE_URL is required to connect to PostgreSQL"))?;

        let grpc_addr: SocketAddr = env::var("GRPC_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:50051".to_string())
            .parse()
            .map_err(|_| anyhow::anyhow!("GRPC_ADDR must be a valid socket address"))?;

        let exports_dir = env::var("EXPORTS_DIR").unwrap_or_else(|_| "exports".to_string());
        let ngram_size = env::var("NGRAM_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3);
        let max_generation_length = env::var("MAX_GENERATION_LENGTH")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(64);
        let ingestion_interval_secs = env::var("INGESTION_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let ingestion_workers = env::var("INGESTION_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        Ok(Self {
            database_url,
            grpc_addr,
            exports_dir,
            ngram_size: ngram_size.max(2),
            max_generation_length,
            ingestion_interval: Duration::from_secs(ingestion_interval_secs),
            ingestion_workers: ingestion_workers.max(1),
        })
    }
}
