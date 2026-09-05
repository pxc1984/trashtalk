use std::{path::PathBuf, thread};

use tracing::{error, info, warn};

use crate::application::trainer::update_ngrams;
use crate::domain::token::{bos_token, eos_token, tokenize_fragments};
use crate::infrastructure::telegram_export::read_export;
use crate::state::SharedState;

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

    if let Some(status) = state.store.export_run_status(&export_path).await? {
        if status == "running" {
            warn!(export = %export_path, "ingestion already running for export");
            return Ok(());
        }

        if status == "completed" {
            return Ok(());
        }
    }

    let run_id = state.store.start_ingestion_run(&export_path).await?;

    let parsed = match read_export(&export_dir) {
        Ok(parsed) => parsed,
        Err(err) => {
            state.store.mark_ingestion_run_error(run_id, &err.to_string()).await?;
            return Err(err);
        }
    };
    state.store.upsert_chat(&parsed).await?;

    let mut total_messages = 0i64;
    let mut total_tokens = 0i64;

    let process_result: anyhow::Result<()> = (|| async {
        for message in parsed.messages {
            let raw_id = state.store.upsert_raw_message(&message, run_id).await?;

            let message_id = state.store.upsert_message(&message, run_id, raw_id).await?;

            let tokens = tokenize_fragments(&message.fragments);
            let mut token_ids = Vec::with_capacity(tokens.len());

            for (idx, token) in tokens.iter().enumerate() {
                let token_id = state.store.ensure_token(token).await?;
                token_ids.push(token_id);

                state
                    .store
                    .insert_message_token(message_id, idx as i32, token_id)
                    .await?;
            }

            let bos_id = state.store.ensure_token(&bos_token()).await?;
            let eos_id = state.store.ensure_token(&eos_token()).await?;

            let bos_padding = state.ngram_size.saturating_sub(1).max(1);

            let mut training_ids = Vec::with_capacity(token_ids.len() + bos_padding + 1);
            training_ids.extend(std::iter::repeat_n(bos_id, bos_padding));
            training_ids.extend_from_slice(&token_ids);
            training_ids.push(eos_id);

            update_ngrams(
                state.store.as_ref(),
                &training_ids,
                state.ngram_size,
                state.min_ngram_size,
            )
            .await?;

            state.store.upsert_ingestion_offset(message.chat_id, message.message_id).await?;

            total_messages += 1;
            total_tokens += token_ids.len() as i64;
        }
        Ok(())
    })()
    .await;

    if let Err(err) = process_result {
        state.store.mark_ingestion_run_error(run_id, &err.to_string()).await?;
        return Err(err);
    }

    state.store.complete_ingestion_run(run_id, total_messages, total_tokens).await?;

    info!(
        export = %export_path,
        messages = total_messages,
        tokens = total_tokens,
        "ingestion completed"
    );

    Ok(())
}