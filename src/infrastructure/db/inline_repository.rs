use sqlx::PgPool;

/// Records a Telegram inline query and its outcome for observability.
#[allow(clippy::too_many_arguments)]
pub async fn log_inline_query(
    pool: &PgPool,
    inline_query_id: &str,
    user_id: i64,
    user_username: Option<&str>,
    chat_type: Option<&str>,
    query_text: &str,
    response_text: Option<&str>,
    success: bool,
    error_message: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO telegram_inline_requests
        (inline_query_id, user_id, user_username, chat_type, query_text, response_text, success, error_message)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
    )
    .bind(inline_query_id)
    .bind(user_id)
    .bind(user_username)
    .bind(chat_type)
    .bind(query_text)
    .bind(response_text)
    .bind(success)
    .bind(error_message)
    .execute(pool)
    .await?;

    Ok(())
}