use sqlx::PgPool;

use crate::infrastructure::db::ngram_repository::NgramCandidate;
use crate::infrastructure::db::token_repository::TokenRecord;
use crate::infrastructure::db::{
    emoji_repository, inline_repository, message_repository, ngram_repository, token_repository,
};
use crate::infrastructure::telegram_export::{NormalizedMessage, ParsedExport};

/// PostgreSQL-backed store backend. Used by the pure-PostgreSQL mode (default)
/// where everything — message history, token vocabulary and n-gram statistics —
/// is durable in the database. Also provides the durable history half of the
/// hybrid backend.
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl PgStore {
    pub async fn ensure_custom_emoji(
        &self,
        document_id: &str,
        emoji_code: Option<&str>,
        description: Option<&str>,
    ) -> anyhow::Result<i64> {
        emoji_repository::ensure_custom_emoji(&self.pool, document_id, emoji_code, description).await
    }

    pub async fn ensure_token_record(
        &self,
        token_type: &str,
        token_value: Option<&str>,
        emoji_id: Option<i64>,
    ) -> anyhow::Result<i64> {
        token_repository::ensure_token_record(&self.pool, token_type, token_value, emoji_id).await
    }

    pub async fn fetch_tokens(&self, ids: &[i64]) -> anyhow::Result<Vec<TokenRecord>> {
        token_repository::fetch_tokens(&self.pool, ids).await
    }

    pub async fn upsert_ngram(
        &self,
        chat_id: i64,
        n: usize,
        prefix: &[i64],
        next_token: i64,
    ) -> anyhow::Result<()> {
        ngram_repository::upsert_ngram(&self.pool, chat_id, n, prefix, next_token).await
    }

    pub async fn query_next_candidates(
        &self,
        chat_id: Option<i64>,
        n: usize,
        prefix: &[i64],
    ) -> anyhow::Result<Vec<NgramCandidate>> {
        ngram_repository::query_next_candidates(&self.pool, chat_id, n, prefix).await
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

    pub async fn insert_message_token(
        &self,
        message_id: i64,
        position: i32,
        token_id: i64,
    ) -> anyhow::Result<()> {
        message_repository::insert_message_token(&self.pool, message_id, position, token_id).await
    }

    pub async fn upsert_ingestion_offset(&self, chat_id: i64, message_id: i64) -> anyhow::Result<()> {
        message_repository::upsert_ingestion_offset(&self.pool, chat_id, message_id).await
    }

    pub async fn fetch_messages_for_rebuild(
        &self,
    ) -> anyhow::Result<Vec<message_repository::StoredMessage>> {
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