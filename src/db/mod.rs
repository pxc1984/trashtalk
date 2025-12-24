use std::path::Path;
use std::time::Duration;
use anyhow::Context;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

use crate::tokenizer::tokens::{Token, TokenKind};

pub async fn init_pool(database_url: &str) -> anyhow::Result<PgPool> {
    sqlx::postgres::PgPoolOptions::new()
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

pub async fn ensure_custom_emoji(
    pool: &PgPool,
    document_id: &str,
    emoji_code: Option<&str>,
    description: Option<&str>,
) -> anyhow::Result<i64> {
    if let Some(existing) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM custom_emojis WHERE document_id = $1 LIMIT 1")
            .bind(document_id)
            .fetch_optional(pool)
            .await?
    {
        return Ok(existing);
    }

    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO custom_emojis (document_id, emoji_code, description) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(document_id)
    .bind(emoji_code)
    .bind(description)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

pub async fn ensure_token(pool: &PgPool, token: &Token) -> anyhow::Result<i64> {
    let emoji_id = if token.kind == TokenKind::CustomEmoji {
        if let Some(doc_id) = token.emoji_document_id.as_deref() {
            Some(ensure_custom_emoji(pool, doc_id, token.value.as_deref(), None).await?)
        } else {
            None
        }
    } else {
        None
    };

    let existing = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM token_vocabulary WHERE token_type = $1 AND token_value IS NOT DISTINCT FROM $2 AND emoji_id IS NOT DISTINCT FROM $3 LIMIT 1",
    )
    .bind(token.kind.as_str())
    .bind(&token.value)
    .bind(emoji_id)
    .fetch_optional(pool)
    .await?;

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO token_vocabulary (token_type, token_value, emoji_id) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(token.kind.as_str())
    .bind(&token.value)
    .bind(emoji_id)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub id: i64,
    pub token_type: String,
    pub token_value: Option<String>,
    pub emoji_id: Option<i64>,
}

pub async fn fetch_tokens(pool: &PgPool, ids: &[i64]) -> anyhow::Result<Vec<TokenRecord>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let rows = sqlx::query(
        r#"SELECT id, token_type, token_value, emoji_id FROM token_vocabulary WHERE id = ANY($1)"#,
    )
    .bind(ids)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| TokenRecord {
            id: r.get("id"),
            token_type: r.get("token_type"),
            token_value: r.get("token_value"),
            emoji_id: r.get("emoji_id"),
        })
        .collect())
}
