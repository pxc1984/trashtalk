use sqlx::PgPool;

use crate::infrastructure::db::ngram_repository::upsert_ngram;

/// Records n-gram statistics for every order in `min_n..=max_n`.
///
/// Storing multiple orders enables backoff during generation: if the exact
/// higher-order continuation is unknown, the generator can fall back to a
/// shorter prefix instead of dead-ending.
pub async fn update_ngrams(
    pool: &PgPool,
    token_ids: &[i64],
    max_n: usize,
    min_n: usize,
) -> anyhow::Result<()> {
    let min_n = min_n.max(2);
    for n in min_n..=max_n {
        update_order(pool, token_ids, n).await?;
    }
    Ok(())
}

async fn update_order(pool: &PgPool, token_ids: &[i64], n: usize) -> anyhow::Result<()> {
    if n < 2 || token_ids.is_empty() {
        return Ok(());
    }

    for window_end in 0..token_ids.len() {
        if window_end + 1 < n {
            continue;
        }
        let start = window_end + 1 - n;
        let prefix = &token_ids[start..window_end];
        let next_token = token_ids[window_end];

        upsert_ngram(pool, n, prefix, next_token).await?;
    }

    Ok(())
}