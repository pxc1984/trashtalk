use sqlx::{PgPool, Row};

use crate::domain::token::{Token, TokenKind};
use crate::infrastructure::db::emoji_repository::ensure_custom_emoji;

#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub id: i64,
    pub token_type: String,
    pub token_value: Option<String>,
    pub emoji_id: Option<i64>,
}

/// Looks up an existing token by its type/value/emoji, creating it if absent.
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

/// Fetches full token records for the given ids in one round-trip.
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

/// Fetches a single token record, used to inspect a just-sampled token for
/// sentence-boundary detection.
pub async fn fetch_token(pool: &PgPool, id: i64) -> anyhow::Result<Option<TokenRecord>> {
    Ok(fetch_tokens(pool, &[id]).await?.into_iter().next())
}