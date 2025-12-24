use std::{path::PathBuf, thread};

use chrono::Utc;
use tracing::{error, info, warn};

use crate::{
    db,
    state::SharedState,
    tokenizer::{
        self,
        telegram::read_export,
        tokens::{bos_token, eos_token},
    },
    trainer,
};

pub fn spawn_ingestion_workers(state: SharedState, workers: usize) -> Vec<thread::JoinHandle<()>> {
    let worker_count = workers.max(1);
    let mut handles = Vec::with_capacity(worker_count);

    for idx in 0..worker_count {
        let worker_state = state.clone();
        let handle = thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("ingestion runtime");
            let _guard = rt.enter();
            loop {
                if let Err(err) = rt.block_on(run_ingestion_cycle(worker_state.clone())) {
                    error!(error = ?err, worker = idx, "ingestion cycle failed");
                }
                rt.block_on(tokio::time::sleep(worker_state.config.ingestion_interval));
            }
        });
        handles.push(handle);
    }

    handles
}

pub async fn run_ingestion_cycle(state: SharedState) -> anyhow::Result<()> {
    let exports = discover_exports(&state.config.exports_dir)?;
    for export_dir in exports {
        if let Err(err) = process_export(state.clone(), export_dir.clone()).await {
            error!(
                error = ?err,
                export = %export_dir.display(),
                "failed to ingest export"
            );
        }
    }
    Ok(())
}

fn discover_exports(root: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut exports = Vec::new();
    let root_path = PathBuf::from(root);
    if !root_path.exists() {
        std::fs::create_dir_all(&root_path).ok();
        return Ok(exports);
    }

    for entry in std::fs::read_dir(root_path)? {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() && path.join("result.json").exists() {
            exports.push(path);
        }
    }

    exports.sort();
    Ok(exports)
}

async fn process_export(state: SharedState, export_dir: PathBuf) -> anyhow::Result<()> {
    let export_path = export_dir.to_string_lossy().to_string();

    if let Some(status) = sqlx::query_scalar::<_, String>(
        "SELECT status FROM ingestion_runs WHERE export_path = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(&export_path)
    .fetch_optional(&state.pool)
    .await?
    {
        if status == "running" {
            warn!(export = %export_path, "ingestion already running for export");
            return Ok(());
        }

        if status == "completed" {
            return Ok(());
        }
    }

    let run_id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO ingestion_runs (export_path, status, started_at) VALUES ($1, 'running', now()) RETURNING id",
    )
    .bind(&export_path)
    .fetch_one(&state.pool)
    .await?;

    let parsed = match read_export(&export_dir) {
        Ok(parsed) => parsed,
        Err(err) => {
            mark_run_error(&state, run_id, &err.to_string()).await?;
            return Err(err);
        }
    };
    upsert_chat(&state, &parsed).await?;

    let mut total_messages = 0i64;
    let mut total_tokens = 0i64;

    let process_result: anyhow::Result<()> = (|| async {
        for message in parsed.messages {
            let raw_id = sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO raw_messages (chat_id, message_id, ingestion_run_id, from_id, sent_at, raw_json)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (chat_id, message_id)
                DO UPDATE SET raw_json = EXCLUDED.raw_json, sent_at = EXCLUDED.sent_at
                RETURNING id"#,
            )
            .bind(message.chat_id)
            .bind(message.message_id)
            .bind(run_id)
            .bind(&message.from_id)
            .bind(message.sent_at)
            .bind(&message.raw)
            .fetch_one(&state.pool)
            .await?;

            let message_id = sqlx::query_scalar::<_, i64>(
                r#"INSERT INTO messages (chat_id, message_id, from_id, sent_at, ingestion_run_id, raw_message_id)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (chat_id, message_id)
                DO UPDATE SET sent_at = EXCLUDED.sent_at, from_id = EXCLUDED.from_id, ingestion_run_id = EXCLUDED.ingestion_run_id, raw_message_id = EXCLUDED.raw_message_id
                RETURNING id"#,
            )
            .bind(message.chat_id)
            .bind(message.message_id)
            .bind(&message.from_id)
            .bind(message.sent_at)
            .bind(run_id)
            .bind(raw_id)
            .fetch_one(&state.pool)
            .await?;

            let tokens = tokenizer::tokenize_fragments(&message.fragments);
            let mut token_ids = Vec::with_capacity(tokens.len());

            for (idx, token) in tokens.iter().enumerate() {
                let token_id = db::ensure_token(&state.pool, token).await?;
                token_ids.push(token_id);

                sqlx::query(
                    r#"INSERT INTO message_tokens (message_id, position, token_id)
                    VALUES ($1, $2, $3)
                    ON CONFLICT (message_id, position) DO NOTHING"#,
                )
                .bind(message_id)
                .bind(idx as i32)
                .bind(token_id)
                .execute(&state.pool)
                .await?;
            }

            let bos_id = db::ensure_token(&state.pool, &bos_token()).await?;
            let eos_id = db::ensure_token(&state.pool, &eos_token()).await?;

            let bos_padding = state.ngram_size.saturating_sub(1).max(1);

            let mut training_ids = Vec::with_capacity(token_ids.len() + bos_padding + 1);
            training_ids.extend(std::iter::repeat(bos_id).take(bos_padding));
            training_ids.extend_from_slice(&token_ids);
            training_ids.push(eos_id);

            trainer::ngrams::update_ngrams(&state.pool, &training_ids, state.ngram_size).await?;

            sqlx::query(
                r#"INSERT INTO ingestion_offsets (chat_id, last_message_id, updated_at)
                VALUES ($1, $2, now())
                ON CONFLICT (chat_id) DO UPDATE SET last_message_id = GREATEST(ingestion_offsets.last_message_id, EXCLUDED.last_message_id), updated_at = now()"#,
            )
            .bind(message.chat_id)
            .bind(message.message_id)
            .execute(&state.pool)
            .await?;

            total_messages += 1;
            total_tokens += token_ids.len() as i64;
        }
        Ok(())
    })()
    .await;

    if let Err(err) = process_result {
        mark_run_error(&state, run_id, &err.to_string()).await?;
        return Err(err);
    }

    sqlx::query(
        r#"UPDATE ingestion_runs
        SET status = 'completed', completed_at = now(), messages_ingested = $2, tokens_ingested = $3
        WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(total_messages)
    .bind(total_tokens)
    .execute(&state.pool)
    .await?;

    info!(
        export = %export_path,
        messages = total_messages,
        tokens = total_tokens,
        "ingestion completed"
    );

    Ok(())
}

async fn mark_run_error(state: &SharedState, run_id: i64, message: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"UPDATE ingestion_runs
        SET status = 'error', completed_at = now(), error = $2
        WHERE id = $1"#,
    )
    .bind(run_id)
    .bind(message)
    .execute(&state.pool)
    .await?;
    Ok(())
}

async fn upsert_chat(
    state: &SharedState,
    parsed: &crate::tokenizer::telegram::ParsedExport,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"INSERT INTO chats (chat_id, title, chat_type, last_message_at)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (chat_id) DO UPDATE SET
            title = COALESCE(EXCLUDED.title, chats.title),
            chat_type = COALESCE(EXCLUDED.chat_type, chats.chat_type),
            last_message_at = GREATEST(COALESCE(chats.last_message_at, EXCLUDED.last_message_at), EXCLUDED.last_message_at)"#,
    )
    .bind(parsed.chat_id)
    .bind(&parsed.chat_title)
    .bind(&parsed.chat_type)
    .bind(parsed.messages.last().map(|m| m.sent_at).unwrap_or_else(|| Utc::now()))
    .execute(&state.pool)
    .await?;

    Ok(())
}
