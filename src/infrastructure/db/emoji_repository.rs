use sqlx::PgPool;

pub async fn ensure_custom_emoji(
    pool: &PgPool,
    document_id: &str,
    emoji_code: Option<&str>,
    description: Option<&str>,
) -> anyhow::Result<i64> {
    if let Some(existing) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM custom_emojis WHERE document_id = $1 LIMIT 1")
            .bind(document_id)
            .fetch_optional(pool)
            .await?
    {
        return Ok(existing);
    }

    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO custom_emojis (document_id, emoji_code, description) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(document_id)
    .bind(emoji_code)
    .bind(description)
    .fetch_one(pool)
    .await?;

    Ok(id)
}