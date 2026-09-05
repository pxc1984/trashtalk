use chrono::Utc;
use sqlx::PgPool;

use crate::infrastructure::telegram_export::{NormalizedMessage, ParsedExport};

/// Returns the status of the latest ingestion run for an export path,
/// if any. Used to skip exports that already succeeded or are in flight.
pub async fn export_run_status(pool: &PgPool, export_path: &str) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT status FROM ingestion_runs WHERE export_path = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(export_path)
    .fetch_optional(pool)
    .await?)
}

/// Opens a new ingestion run and returns its id.
pub async fn start_ingestion_run(pool: &PgPool, export_path: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "INSERT INTO ingestion_runs (export_path, status, started_at) VALUES ($1, 'running', now()) RETURNING id",
    )
    .bind(export_path)
    .fetch_one(pool)
    .await?)
}

pub async fn complete_ingestion_run(
    pool: &PgPool,
    run_id: i64,
    messages: i64,
    tokens: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"UPDATE ingestion_runs
        SET status = 'completed', completed_at = now(), messages_ingested = $2, tokens_ingested = $3
        WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(messages)
    .bind(tokens)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_ingestion_run_error(
    pool: &PgPool,
    run_id: i64,
    message: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"UPDATE ingestion_runs
        SET status = 'error', completed_at = now(), error = $2
        WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Upserts a chat row from an export.
pub async fn upsert_chat(pool: &PgPool, parsed: &ParsedExport) -> anyhow::Result<()> {
    let last_at = parsed
        .messages
        .last()
        .map(|m| m.sent_at)
        .unwrap_or_else(Utc::now);

    sqlx::query(
        r#"INSERT INTO chats (chat_id, title, chat_type, last_message_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (chat_id) DO UPDATE SET
            title = COALESCE(EXCLUDED.title, chats.title),
            chat_type = COALESCE(EXCLUDED.chat_type, chats.chat_type),
            last_message_at = GREATEST(COALESCE(chats.last_message_at, EXCLUDED.last_message_at), EXCLUDED.last_message_at)"#,
    )
    .bind(parsed.chat_id)
    .bind(&parsed.chat_title)
    .bind(&parsed.chat_type)
    .bind(last_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upserts a raw message and returns its id.
pub async fn upsert_raw_message(
    pool: &PgPool,
    msg: &NormalizedMessage,
    ingestion_run_id: i64,
) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO raw_messages (chat_id, message_id, ingestion_run_id, from_id, sent_at, raw_json)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (chat_id, message_id)
        DO UPDATE SET raw_json = EXCLUDED.raw_json, sent_at = EXCLUDED.sent_at
        RETURNING id"#,
    )
    .bind(msg.chat_id)
    .bind(msg.message_id)
    .bind(ingestion_run_id)
    .bind(&msg.from_id)
    .bind(msg.sent_at)
    .bind(&msg.raw)
    .fetch_one(pool)
    .await?)
}

/// Upserts a normalized message and returns its id.
pub async fn upsert_message(
    pool: &PgPool,
    msg: &NormalizedMessage,
    ingestion_run_id: i64,
    raw_message_id: i64,
) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"INSERT INTO messages (chat_id, message_id, from_id, sent_at, ingestion_run_id, raw_message_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (chat_id, message_id)
        DO UPDATE SET sent_at = EXCLUDED.sent_at, from_id = EXCLUDED.from_id, ingestion_run_id = EXCLUDED.ingestion_run_id, raw_message_id = EXCLUDED.raw_message_id
        RETURNING id"#,
    )
    .bind(msg.chat_id)
    .bind(msg.message_id)
    .bind(&msg.from_id)
    .bind(msg.sent_at)
    .bind(ingestion_run_id)
    .bind(raw_message_id)
    .fetch_one(pool)
    .await?)
}

pub async fn insert_message_token(
    pool: &PgPool,
    message_id: i64,
    position: i32,
    token_id: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO message_tokens (message_id, position, token_id)
        VALUES ($1, $2, $3)
        ON CONFLICT (message_id, position) DO NOTHING"#,
    )
    .bind(message_id)
    .bind(position)
    .bind(token_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Advances the high-water mark of the last ingested message per chat.
pub async fn upsert_ingestion_offset(
    pool: &PgPool,
    chat_id: i64,
    message_id: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO ingestion_offsets (chat_id, last_message_id, updated_at)
        VALUES ($1, $2, now())
        ON CONFLICT (chat_id) DO UPDATE SET last_message_id = GREATEST(ingestion_offsets.last_message_id, EXCLUDED.last_message_id), updated_at = now()"#,
    )
    .bind(chat_id)
    .bind(message_id)
    .execute(pool)
    .await?;
    Ok(())
}