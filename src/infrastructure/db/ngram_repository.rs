use sqlx::{PgPool, Row};

/// A candidate next token paired with the observed count for an n-gram prefix.
pub type NgramCandidate = (i64, i64);

/// Increments the count for `(n, prefix_tokens) -> next_token` by one.
pub async fn upsert_ngram(
    pool: &PgPool,
    n: usize,
    prefix: &[i64],
    next_token: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO ngram_statistics (n, prefix_tokens, next_token_id, count)
        VALUES ($1, $2, $3, 1)
        ON CONFLICT (n, prefix_tokens, next_token_id)
        DO UPDATE SET count = ngram_statistics.count + 1"#,
    )
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
pub async fn query_next_candidates(
    pool: &PgPool,
    n: usize,
    prefix: &[i64],
) -> anyhow::Result<Vec<NgramCandidate>> {
    let rows = sqlx::query(
        "SELECT next_token_id, count
         FROM ngram_statistics
         WHERE n = $1 AND prefix_tokens = $2",
    )
    .bind(n as i16)
    .bind(prefix)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let next: i64 = row.get("next_token_id");
            let count: i64 = row.get("count");
            (next, count.max(1))
        })
        .collect())
}