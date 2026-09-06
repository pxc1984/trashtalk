mod memory;
mod pg;

pub use memory::InMemoryStore;
pub use pg::PgStore;

use std::sync::Arc;

use crate::domain::token::{Token, TokenKind};
use crate::infrastructure::db::ngram_repository::NgramCandidate;
use crate::infrastructure::db::token_repository::TokenRecord;
use crate::infrastructure::telegram_export::{NormalizedMessage, ParsedExport};

/// The store backing the app: the token vocabulary, the n-gram statistics, and
/// the ingestion/bookkeeping rows.
///
/// Two backends exist:
/// - [`Store::Pg`] — PostgreSQL, the default. Durable and shared.
/// - [`Store::InMemory`] — everything lives in the process's RAM, selected with
///   `USE_INMEMORY_STORE=true`. Fast to start, no schema, but all data is lost
///   on shutdown.
///
/// The enum is always shared behind [`SharedStore`] (`Arc`), never moved by
/// value, so the size of the largest variant is irrelevant.
#[allow(clippy::large_enum_variant)]
pub enum Store {
    Pg(PgStore),
    InMemory(InMemoryStore),
}

impl Store {
    /// Ensures a custom emoji row exists and returns its id.
    pub async fn ensure_custom_emoji(
        &self,
        document_id: &str,
        emoji_code: Option<&str>,
        description: Option<&str>,
    ) -> anyhow::Result<i64> {
        match self {
            Store::Pg(s) => s.ensure_custom_emoji(document_id, emoji_code, description).await,
            Store::InMemory(s) => s.ensure_custom_emoji(document_id, emoji_code, description).await,
        }
    }

    /// Ensures a token — resolving a custom emoji first — and returns its id.
    pub async fn ensure_token(&self, token: &Token) -> anyhow::Result<i64> {
        let emoji_id = if token.kind == TokenKind::CustomEmoji {
            match token.emoji_document_id.as_deref() {
                Some(doc_id) => Some(self.ensure_custom_emoji(doc_id, token.value.as_deref(), None).await?),
                None => None,
            }
        } else {
            None
        };
        self.ensure_token_record(token.kind.as_str(), token.value.as_deref(), emoji_id)
            .await
    }

    /// Ensures a token_vocabulary row for `(token_type, token_value, emoji_id)`
    /// exists and returns its id.
    pub async fn ensure_token_record(
        &self,
        token_type: &str,
        token_value: Option<&str>,
        emoji_id: Option<i64>,
    ) -> anyhow::Result<i64> {
        match self {
            Store::Pg(s) => s.ensure_token_record(token_type, token_value, emoji_id).await,
            Store::InMemory(s) => s.ensure_token_record(token_type, token_value, emoji_id).await,
        }
    }

    /// Returns the full records for the given token ids.
    pub async fn fetch_tokens(&self, ids: &[i64]) -> anyhow::Result<Vec<TokenRecord>> {
        match self {
            Store::Pg(s) => s.fetch_tokens(ids).await,
            Store::InMemory(s) => s.fetch_tokens(ids).await,
        }
    }

    /// Fetches a single token record by id.
    pub async fn fetch_token(&self, id: i64) -> anyhow::Result<Option<TokenRecord>> {
        Ok(self.fetch_tokens(&[id]).await?.into_iter().next())
    }

    /// Increments the observed count for the transition
    /// `(chat_id, n, prefix) -> next`.
    pub async fn upsert_ngram(
        &self,
        chat_id: i64,
        n: usize,
        prefix: &[i64],
        next_token: i64,
    ) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.upsert_ngram(chat_id, n, prefix, next_token).await,
            Store::InMemory(s) => s.upsert_ngram(chat_id, n, prefix, next_token).await,
        }
    }

    /// Candidate continuations and their counts for a prefix at a given order.
    /// `None` for `chat_id` queries across all chats; `Some(chat_id)` restricts
    /// to that chat's statistics.
    pub async fn query_next_candidates(
        &self,
        chat_id: Option<i64>,
        n: usize,
        prefix: &[i64],
    ) -> anyhow::Result<Vec<NgramCandidate>> {
        match self {
            Store::Pg(s) => s.query_next_candidates(chat_id, n, prefix).await,
            Store::InMemory(s) => s.query_next_candidates(chat_id, n, prefix).await,
        }
    }

    /// Status of the latest ingestion run for an export path, if any.
    pub async fn export_run_status(&self, export_path: &str) -> anyhow::Result<Option<String>> {
        match self {
            Store::Pg(s) => s.export_run_status(export_path).await,
            Store::InMemory(s) => s.export_run_status(export_path).await,
        }
    }

    pub async fn start_ingestion_run(&self, export_path: &str) -> anyhow::Result<i64> {
        match self {
            Store::Pg(s) => s.start_ingestion_run(export_path).await,
            Store::InMemory(s) => s.start_ingestion_run(export_path).await,
        }
    }

    pub async fn complete_ingestion_run(
        &self,
        run_id: i64,
        messages: i64,
        tokens: i64,
    ) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.complete_ingestion_run(run_id, messages, tokens).await,
            Store::InMemory(s) => s.complete_ingestion_run(run_id, messages, tokens).await,
        }
    }

    pub async fn mark_ingestion_run_error(&self, run_id: i64, message: &str) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.mark_ingestion_run_error(run_id, message).await,
            Store::InMemory(s) => s.mark_ingestion_run_error(run_id, message).await,
        }
    }

    pub async fn upsert_chat(&self, parsed: &ParsedExport) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.upsert_chat(parsed).await,
            Store::InMemory(s) => s.upsert_chat(parsed).await,
        }
    }

    pub async fn upsert_raw_message(
        &self,
        msg: &NormalizedMessage,
        run_id: i64,
    ) -> anyhow::Result<i64> {
        match self {
            Store::Pg(s) => s.upsert_raw_message(msg, run_id).await,
            Store::InMemory(s) => s.upsert_raw_message(msg, run_id).await,
        }
    }

    pub async fn upsert_message(
        &self,
        msg: &NormalizedMessage,
        run_id: i64,
        raw_message_id: i64,
    ) -> anyhow::Result<i64> {
        match self {
            Store::Pg(s) => s.upsert_message(msg, run_id, raw_message_id).await,
            Store::InMemory(s) => s.upsert_message(msg, run_id, raw_message_id).await,
        }
    }

    pub async fn insert_message_token(
        &self,
        message_id: i64,
        position: i32,
        token_id: i64,
    ) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.insert_message_token(message_id, position, token_id).await,
            Store::InMemory(s) => s.insert_message_token(message_id, position, token_id).await,
        }
    }

    pub async fn upsert_ingestion_offset(
        &self,
        chat_id: i64,
        message_id: i64,
    ) -> anyhow::Result<()> {
        match self {
            Store::Pg(s) => s.upsert_ingestion_offset(chat_id, message_id).await,
            Store::InMemory(s) => s.upsert_ingestion_offset(chat_id, message_id).await,
        }
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
        match self {
            Store::Pg(s) => {
                s.log_inline_query(
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
            Store::InMemory(s) => {
                s.log_inline_query(
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
    }
}

/// Convenience alias for the store shared across async tasks.
pub type SharedStore = Arc<Store>;