use sqlx::PgPool;

use crate::infrastructure::db::inline_repository;
use crate::infrastructure::db::message_repository::{self, StoredMessage};
use crate::infrastructure::telegram_export::{NormalizedMessage, ParsedExport};

/// PostgreSQL-backed store of durable message history for the hybrid backend.
///
/// It persists chats, raw/normalized messages, ingestion runs and inline-query
/// logs, but not the token vocabulary or n-gram statistics — those live in the
/// in-memory cache and are rebuilt from [`StoredMessage`]s on startup.
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn export_run_status(&self, export_path: &str) -> anyhow::Result<Option<String>> {
        message_repository::export_run_status(&self.pool, export_path).await
    }

    pub async fn start_ingestion_run(&self, export_path: &str) -> anyhow::Result<i64> {
        message_repository::start_ingestion_run(&self.pool, export_path).await
    }

    pub async fn complete_ingestion_run(
        &self,
        run_id: i64,
        messages: i64,
        tokens: i64,
    ) -> anyhow::Result<()> {
        message_repository::complete_ingestion_run(&self.pool, run_id, messages, tokens).await
    }

    pub async fn mark_ingestion_run_error(&self, run_id: i64, message: &str) -> anyhow::Result<()> {
        message_repository::mark_ingestion_run_error(&self.pool, run_id, message).await
    }

    pub async fn upsert_chat(&self, parsed: &ParsedExport) -> anyhow::Result<()> {
        message_repository::upsert_chat(&self.pool, parsed).await
    }

    pub async fn upsert_raw_message(
        &self,
        msg: &NormalizedMessage,
        run_id: i64,
    ) -> anyhow::Result<i64> {
        message_repository::upsert_raw_message(&self.pool, msg, run_id).await
    }

    pub async fn upsert_message(
        &self,
        msg: &NormalizedMessage,
        run_id: i64,
        raw_message_id: i64,
    ) -> anyhow::Result<i64> {
        message_repository::upsert_message(&self.pool, msg, run_id, raw_message_id).await
    }

    pub async fn upsert_ingestion_offset(&self, chat_id: i64, message_id: i64) -> anyhow::Result<()> {
        message_repository::upsert_ingestion_offset(&self.pool, chat_id, message_id).await
    }

    pub async fn fetch_messages_for_rebuild(&self) -> anyhow::Result<Vec<StoredMessage>> {
        message_repository::fetch_messages_for_rebuild(&self.pool).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_inline_query(
        &self,
        inline_query_id: &str,
        user_id: i64,
        user_username: Option<&str>,
        chat_type: Option<&str>,
        query_text: &str,
        response_text: Option<&str>,
        success: bool,
        error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        inline_repository::log_inline_query(
            &self.pool,
            inline_query_id,
            user_id,
            user_username,
            chat_type,
            query_text,
            response_text,
            success,
            error_message,
        )
        .await
    }
}