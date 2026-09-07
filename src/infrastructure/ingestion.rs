use std::{path::PathBuf, thread};

use tracing::{error, info, warn};

use crate::application::trainer::train_token_ids;
use crate::domain::token::tokenize_fragments;
use crate::infrastructure::telegram_export::read_export;
use crate::state::SharedState;

pub fn spawn_ingestion_workers(state: SharedState, workers: usize) -> Vec<thread::JoinHandle<()>> {
    let worker_count = workers.max(1);
    let mut handles = Vec::with_capacity(worker_count);

    for idx in 0..worker_count {
        let worker_state = state.clone();
        let handle = thread::spawn(move || {
            info!(worker = idx, "ingestion worker started");
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

            train_token_ids(
                state.store.as_ref(),
                message.chat_id,
                &token_ids,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::application::generator::GeneratorService;
    use crate::config::Config;
    use crate::infrastructure::store::{InMemoryStore, Store};
    use crate::state::AppState;

    /// Minimal, greedy config so generation is deterministic enough to assert on.
    fn test_config() -> Config {
        Config {
            database_url: String::new(),
            exports_dir: "exports".into(),
            ngram_size: 2,
            min_ngram_size: 2,
            max_generation_length: 64,
            generation_temperature: 0.0, // greedy
            generation_top_k: 0,
            generation_top_p: 1.0,
            repetition_penalty: 1.0,
            num_candidates: 1,
            min_generation_tokens: 1,
            reply_chance: 0.0,
            always_reply_to_username: String::new(),
            sticker_chance: 0.0,
            ingestion_interval: Duration::from_secs(3600),
            ingestion_workers: 1,
            heartbeat_interval: Duration::from_secs(3600),
            bot_token: None,
            use_inmemory_store: true,
            use_hybrid_mode: false,
        }
    }

    /// The bot must learn from a chat export, associate the learned data with
    /// the `chat_id` declared in `result.json` ("id"), and use it when
    /// generating for that group.
    #[tokio::test]
    async fn learns_from_chatexport_scoped_to_export_chat_id() {
        let dir =
            std::env::temp_dir().join(format!("trashtalk_ingest_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let json = r#"{
            "id": 42,
            "name": "Test Group",
            "type": "private_supergroup",
            "messages": [
                {"id": 1, "type": "message", "date": "2024-01-01 10:00:00", "from_id": "u1", "text": "привет мир"},
                {"id": 2, "type": "message", "date": "2024-01-01 10:01:00", "from_id": "u2", "text": "как дела"}
            ]
        }"#;
        std::fs::write(dir.join("result.json"), json).unwrap();

        let store = Arc::new(Store::InMemory(InMemoryStore::new()));
        let state = Arc::new(AppState::new(test_config(), store.clone()));

        // Real ingestion path: read_export + train, scoped to the export's chat.
        super::process_export(state.clone(), dir.clone()).await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // The learned data is associated with the export's chat id (42): chat 42
        // has tokens, an unrelated chat (99) has none.
        let chat42 = store.query_next_candidates(Some(42), 1, &[]).await.unwrap();
        let chat99 = store.query_next_candidates(Some(99), 1, &[]).await.unwrap();
        assert!(!chat42.is_empty(), "chat 42 should have learned tokens from the export");
        assert!(chat99.is_empty(), "chat 99 should have no data from the export");

        // Generating for that group uses the export-learned words.
        let svc = GeneratorService::new(state);
        let output = svc
            .generate_text_for_prefix(String::new(), 32, Some(42))
            .await
            .unwrap();
        assert!(
            ["привет", "мир", "как", "дела"].iter().any(|w| output.text.contains(w)),
            "generation for chat 42 should use export-learned words, got: {:?}",
            output.text
        );
    }
}