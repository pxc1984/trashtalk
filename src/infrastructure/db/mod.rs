pub mod emoji_repository;
pub mod inline_repository;
pub mod message_repository;
pub mod ngram_repository;
pub mod token_repository;

use anyhow::Context;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::path::Path;
use std::time::Duration;

pub async fn init_pool(database_url: &str) -> anyhow::Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(20)
        .min_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(database_url)
        .await
        .context("failed to connect to PostgreSQL")
}

pub async fn apply_schema(pool: &PgPool, schema_dir: &Path) -> anyhow::Result<()> {
    if !schema_dir.exists() {
        return Ok(());
    }

    let mut files: Vec<_> = std::fs::read_dir(schema_dir)
        .context("reading schema directory")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .collect();

    files.sort_by_key(|entry| entry.file_name());

    for file in files {
        let sql = std::fs::read_to_string(file.path())
            .with_context(|| format!("reading schema file {}", file.path().display()))?;

        for statement in sql.split(';') {
            let stmt = statement.trim();
            if stmt.is_empty() {
                continue;
            }

            sqlx::query(stmt)
                .execute(pool)
                .await
                .with_context(|| format!("applying statement from {}", file.path().display()))?;
        }
    }

    Ok(())
}