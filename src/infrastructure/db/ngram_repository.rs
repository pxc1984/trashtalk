use sqlx::{PgPool, Row};

/// A candidate next token paired with the observed count for an n-gram prefix.
pub type NgramCandidate = (i64, i64);

/// Increments the count for `(chat_id, n, prefix_tokens) -> next_token` by one.
///
/// n-grams are scoped per chat so generation can favour the messages of the
/// chat the query came from.
pub async fn upsert_ngram(
    pool: &PgPool,
    chat_id: i64,
    n: usize,
    prefix: &[i64],
    next_token: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO ngram_statistics (chat_id, n, prefix_tokens, next_token_id, count)
        VALUES ($1, $2, $3, $4, 1)
        ON CONFLICT (chat_id, n, prefix_tokens, next_token_id)
        DO UPDATE SET count = ngram_statistics.count + 1"#,
    )
    .bind(chat_id)
    .bind(n as i16)
    .bind(prefix)
    .bind(next_token)
    .execute(pool)
    .await?;

    Ok(())
}

/// Returns the candidate next tokens and their counts for a specific n-gram
/// order and prefix. Empty when no continuation has ever been observed for the
/// given prefix at this order.
///
/// When `chat_id` is `Some`, only that chat's statistics are considered; when
/// `None`, statistics across all chats are returned (the no-chat-context
/// fallback).
pub async fn query_next_candidates(
    pool: &PgPool,
    chat_id: Option<i64>,
    n: usize,
    prefix: &[i64],
) -> anyhow::Result<Vec<NgramCandidate>> {
    let rows = match chat_id {
        Some(chat_id) => {
            sqlx::query(
                "SELECT next_token_id, count
                 FROM ngram_statistics
                 WHERE n = $1 AND prefix_tokens = $2 AND chat_id = $3",
            )
            .bind(n as i16)
            .bind(prefix)
            .bind(chat_id)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query(
                "SELECT next_token_id, count
                 FROM ngram_statistics
                 WHERE n = $1 AND prefix_tokens = $2",
            )
            .bind(n as i16)
            .bind(prefix)
            .fetch_all(pool)
            .await?
        }
    };

    Ok(rows
        .into_iter()
        .map(|row| {
            let next: i64 = row.get("next_token_id");
            let count: i64 = row.get("count");
            (next, count.max(1))
        })
        .collect())
}