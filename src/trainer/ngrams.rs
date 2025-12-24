use sqlx::PgPool;

pub async fn update_ngrams(pool: &PgPool, token_ids: &[i64], n: usize) -> anyhow::Result<()> {
    if n < 2 || token_ids.is_empty() {
        return Ok(());
    }

    for window_end in 0..token_ids.len() {
        if window_end + 1 < n {
            continue;
        }
        let start = window_end + 1 - n;
        let prefix: Vec<i64> = token_ids[start..window_end].to_vec();
        let next_token = token_ids[window_end];

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
    }

    Ok(())
}
