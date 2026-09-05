use std::collections::HashMap;
use std::sync::Mutex;

use tracing::debug;

use crate::infrastructure::db::ngram_repository::NgramCandidate;
use crate::infrastructure::db::token_repository::TokenRecord;
use crate::infrastructure::telegram_export::{NormalizedMessage, ParsedExport};

/// A `Token`-keyed identity used to deduplicate the in-memory vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TokenKey {
    token_type: String,
    token_value: Option<String>,
    emoji_id: Option<i64>,
}

/// A single ingestion run, mirroring the PostgreSQL `ingestion_runs` row.
#[derive(Debug, Default)]
struct RunRecord {
    export_path: String,
    status: String,
    error: Option<String>,
    messages: i64,
    tokens: i64,
}

/// All mutable state behind the store's lock.
#[derive(Debug, Default)]
struct Data {
    tokens: HashMap<TokenKey, i64>,
    token_records: HashMap<i64, TokenRecord>,
    next_token_id: i64,
    emoji_ids: HashMap<String, i64>,
    next_emoji_id: i64,
    ngrams: HashMap<(usize, Vec<i64>), HashMap<i64, i64>>,
    export_status: HashMap<String, String>,
    runs: HashMap<i64, RunRecord>,
    next_run_id: i64,
    next_raw_message_id: i64,
    next_message_id: i64,
}

/// Store backend that lives entirely in the process's RAM.
///
/// Selected with `USE_INMEMORY_STORE=true`. It keeps the same shape as the
/// PostgreSQL backend — token vocabulary, n-gram statistics, ingestion-run
/// status — so the app behaves identically, but nothing is persisted and all
/// data vanishes on shutdown. Message/chat rows and inline-query logs are
/// bookkeeping for observability, so the in-memory backend only tracks what it
/// needs to keep re-ingestion idempotent (run status).
pub struct InMemoryStore {
    data: Mutex<Data>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(Data::default()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryStore {
    pub async fn ensure_custom_emoji(
        &self,
        document_id: &str,
        _emoji_code: Option<&str>,
        _description: Option<&str>,
    ) -> anyhow::Result<i64> {
        let mut data = self.data.lock().unwrap();
        if let Some(id) = data.emoji_ids.get(document_id) {
            return Ok(*id);
        }
        data.next_emoji_id += 1;
        let id = data.next_emoji_id;
        data.emoji_ids.insert(document_id.to_string(), id);
        Ok(id)
    }

    pub async fn ensure_token_record(
        &self,
        token_type: &str,
        token_value: Option<&str>,
        emoji_id: Option<i64>,
    ) -> anyhow::Result<i64> {
        let key = TokenKey {
            token_type: token_type.to_string(),
            token_value: token_value.map(str::to_string),
            emoji_id,
        };

        let mut data = self.data.lock().unwrap();
        if let Some(id) = data.tokens.get(&key) {
            return Ok(*id);
        }

        data.next_token_id += 1;
        let id = data.next_token_id;

        let record = TokenRecord {
            id,
            token_type: key.token_type.clone(),
            token_value: key.token_value.clone(),
            emoji_id,
        };

        data.tokens.insert(key, id);
        data.token_records.insert(id, record);
        Ok(id)
    }

    pub async fn fetch_tokens(&self, ids: &[i64]) -> anyhow::Result<Vec<TokenRecord>> {
        let data = self.data.lock().unwrap();
        Ok(ids
            .iter()
            .filter_map(|id| data.token_records.get(id).cloned())
            .collect())
    }

    pub async fn upsert_ngram(&self, n: usize, prefix: &[i64], next_token: i64) -> anyhow::Result<()> {
        let mut data = self.data.lock().unwrap();
        let bucket = data.ngrams.entry((n, prefix.to_vec())).or_default();
        *bucket.entry(next_token).or_insert(0) += 1;
        Ok(())
    }

    pub async fn query_next_candidates(
        &self,
        n: usize,
        prefix: &[i64],
    ) -> anyhow::Result<Vec<NgramCandidate>> {
        let data = self.data.lock().unwrap();
        Ok(data
            .ngrams
            .get(&(n, prefix.to_vec()))
            .map(|bucket| {
                bucket
                    .iter()
                    .map(|(&token, &count)| (token, count))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn export_run_status(&self, export_path: &str) -> anyhow::Result<Option<String>> {
        let data = self.data.lock().unwrap();
        Ok(data.export_status.get(export_path).cloned())
    }

    pub async fn start_ingestion_run(&self, export_path: &str) -> anyhow::Result<i64> {
        let mut data = self.data.lock().unwrap();
        data.next_run_id += 1;
        let id = data.next_run_id;
        data.export_status
            .insert(export_path.to_string(), "running".to_string());
        data.runs.insert(
            id,
            RunRecord {
                export_path: export_path.to_string(),
                status: "running".to_string(),
                ..RunRecord::default()
            },
        );
        Ok(id)
    }

    pub async fn complete_ingestion_run(
        &self,
        run_id: i64,
        messages: i64,
        tokens: i64,
    ) -> anyhow::Result<()> {
        let mut data = self.data.lock().unwrap();
        let completed_path = match data.runs.get_mut(&run_id) {
            Some(run) => {
                run.status = "completed".to_string();
                run.messages = messages;
                run.tokens = tokens;
                Some(run.export_path.clone())
            }
            None => None,
        };
        if let Some(path) = completed_path {
            data.export_status.insert(path, "completed".to_string());
        }
        Ok(())
    }

    pub async fn mark_ingestion_run_error(&self, run_id: i64, message: &str) -> anyhow::Result<()> {
        let mut data = self.data.lock().unwrap();
        let error_path = match data.runs.get_mut(&run_id) {
            Some(run) => {
                run.status = "error".to_string();
                run.error = Some(message.to_string());
                Some(run.export_path.clone())
            }
            None => None,
        };
        if let Some(path) = error_path {
            data.export_status.insert(path, "error".to_string());
        }
        Ok(())
    }

    pub async fn upsert_chat(&self, _parsed: &ParsedExport) -> anyhow::Result<()> {
        Ok(())
    }

    pub async fn upsert_raw_message(
        &self,
        _msg: &NormalizedMessage,
        _run_id: i64,
    ) -> anyhow::Result<i64> {
        let mut data = self.data.lock().unwrap();
        data.next_raw_message_id += 1;
        Ok(data.next_raw_message_id)
    }

    pub async fn upsert_message(
        &self,
        _msg: &NormalizedMessage,
        _run_id: i64,
        _raw_message_id: i64,
    ) -> anyhow::Result<i64> {
        let mut data = self.data.lock().unwrap();
        data.next_message_id += 1;
        Ok(data.next_message_id)
    }

    pub async fn insert_message_token(
        &self,
        _message_id: i64,
        _position: i32,
        _token_id: i64,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    pub async fn upsert_ingestion_offset(&self, _chat_id: i64, _message_id: i64) -> anyhow::Result<()> {
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn log_inline_query(
        &self,
        inline_query_id: &str,
        _user_id: i64,
        _user_username: Option<&str>,
        _chat_type: Option<&str>,
        _query_text: &str,
        _response_text: Option<&str>,
        _success: bool,
        _error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        debug!(inline_query_id, "in-memory store: inline query logging is a no-op");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::token::{Token, TokenKind};
    use crate::infrastructure::store::Store;

    fn store() -> Store {
        Store::InMemory(InMemoryStore::new())
    }

    #[tokio::test]
    pub async fn token_is_deduplicated_by_type_value_emoji() {
        let s = store();
        let token = Token::new(TokenKind::Word, Some("hello".to_string()));
        let a = s.ensure_token(&token).await.unwrap();
        let b = s.ensure_token(&token).await.unwrap();
        assert_eq!(a, b);

        let other = Token::new(TokenKind::Word, Some("world".to_string()));
        let c = s.ensure_token(&other).await.unwrap();
        assert_ne!(a, c);
    }

    #[tokio::test]
    pub async fn custom_emoji_resolves_to_same_id() {
        let s = store();
        let emoji = Token::custom_emoji("doc-1".to_string(), None);
        let a = s.ensure_token(&emoji).await.unwrap();
        let b = s.ensure_token(&emoji).await.unwrap();
        assert_eq!(a, b);

        let fetched = s
            .fetch_tokens(&[a])
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(fetched.token_type, "CustomEmoji");
        // The token record references the custom-emoji row by its own id.
        let emoji_id = s.ensure_custom_emoji("doc-1", None, None).await.unwrap();
        assert_eq!(fetched.emoji_id, Some(emoji_id));
    }

    #[tokio::test]
    pub async fn ngram_counts_accumulate_and_query_returns_candidates() {
        let s = store();
        s.upsert_ngram(2, &[1], 2).await.unwrap();
        s.upsert_ngram(2, &[1], 2).await.unwrap();
        s.upsert_ngram(2, &[1], 3).await.unwrap();

        let candidates = s.query_next_candidates(2, &[1]).await.unwrap();
        assert_eq!(candidates.len(), 2);
        let count_of_2 = candidates.iter().find(|(t, _)| *t == 2).unwrap().1;
        assert_eq!(count_of_2, 2);
    }

    #[tokio::test]
    pub async fn unknown_prefix_returns_no_candidates() {
        let s = store();
        assert!(s.query_next_candidates(2, &[99]).await.unwrap().is_empty());
    }

    #[tokio::test]
    pub async fn completed_export_is_not_reprocessed() {
        let s = store();
        assert_eq!(s.export_run_status("/exports/a").await.unwrap(), None);

        let run = s.start_ingestion_run("/exports/a").await.unwrap();
        s.complete_ingestion_run(run, 10, 20).await.unwrap();

        assert_eq!(
            s.export_run_status("/exports/a").await.unwrap().as_deref(),
            Some("completed")
        );
    }

    #[tokio::test]
    pub async fn fetch_missing_token_returns_none() {
        let s = store();
        assert!(s.fetch_tokens(&[4242]).await.unwrap().is_empty());
    }
}