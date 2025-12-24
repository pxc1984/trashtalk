use std::collections::HashMap;

use rand::distr::{Distribution, weighted::WeightedIndex};
use rand::rng;
use sqlx::Row;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::{
    db::{self, TokenRecord},
    grpc::proto::{
        GenerateTextRequest, GenerateTextResponse, HealthCheckRequest, HealthCheckResponse,
        HealthStatus, ReloadTrainingDataRequest, ReloadTrainingDataResponse, Token,
        TokenizePreviewRequest, TokenizePreviewResponse, text_generator_server::TextGenerator,
    },
    state::SharedState,
    tokenizer::{
        self,
        telegram::TextFragment,
        tokens::{bos_token, eos_token},
    },
    workers,
};

pub struct GeneratorService {
    state: SharedState,
}

impl GeneratorService {
    pub fn new(state: SharedState) -> Self {
        info!("GeneratorService initialized");
        Self { state }
    }

    pub async fn generate_text_for_prefix(
        &self,
        prefix: String,
        max_tokens: usize,
    ) -> Result<(String, usize), Status> {
        info!(
            max_tokens,
            prefix_len = prefix.len(),
            "generate_text_for_prefix invoked"
        );

        let max_tokens = max_tokens.max(1);
        let bos_id = db::ensure_token(&self.state.pool, &bos_token())
            .await
            .map_err(internal_error)?;
        let eos_id = db::ensure_token(&self.state.pool, &eos_token())
            .await
            .map_err(internal_error)?;

        let prefix_tokens = tokenizer::tokenize_fragments(&[TextFragment::Text(prefix)]);

        debug!(token_count = prefix_tokens.len(), "prefix tokenized");

        let bos_padding = self.state.ngram_size.saturating_sub(1).max(1);

        let mut token_ids = Vec::with_capacity(prefix_tokens.len() + bos_padding);
        token_ids.extend(std::iter::repeat(bos_id).take(bos_padding));

        for token in &prefix_tokens {
            let id = db::ensure_token(&self.state.pool, token)
                .await
                .map_err(internal_error)?;
            token_ids.push(id);
        }

        let max_tokens = max_tokens.min(self.state.config.max_generation_length);

        let mut generated =
            self.generate_from_model(&mut token_ids, max_tokens, eos_id).await?;

        if generated == 0 && token_ids.len() > bos_padding {
            info!("prefix failed to continue, falling back to BOS-only generation");
            token_ids.truncate(bos_padding);
            generated = self.generate_from_model(&mut token_ids, max_tokens, eos_id).await?;
        }

        let text = self.render_text(&token_ids, bos_id, eos_id).await?;

        info!(generated, final_len = text.len(), "text generation completed");

        Ok((text, generated))
    }

    async fn render_text(&self, token_ids: &[i64], bos_id: i64, eos_id: i64) -> Result<String, Status> {
        debug!(token_count = token_ids.len(), "render_text called");

        let records = db::fetch_tokens(&self.state.pool, token_ids)
            .await
            .map_err(internal_error)?;

        let by_id: HashMap<i64, TokenRecord> = records.into_iter().map(|r| (r.id, r)).collect();

        let mut text = String::new();

        for id in token_ids {
            if *id == bos_id {
                continue;
            }
            if *id == eos_id {
                break;
            }

            let Some(token) = by_id.get(id) else {
                warn!(token_id = id, "missing token record during rendering");
                continue;
            };

            match token.token_type.as_str() {
                "Word" => {
                    if let Some(value) = &token.token_value {
                        text.push_str(value);
                    }
                }
                "Whitespace" => text.push_str(token.token_value.as_deref().unwrap_or(" ")),
                "Newline" => text.push('\n'),
                "Punctuation" => text.push_str(token.token_value.as_deref().unwrap_or("")),
                "CustomEmoji" => {
                    if let Some(value) = token.token_value.as_deref() {
                        text.push_str(value);
                    } else if let Some(emoji_id) = token.emoji_id {
                        text.push_str(&format!("<emoji:{}>", emoji_id));
                    } else {
                        warn!("custom emoji token missing both value and emoji_id");
                        text.push_str("[emoji]");
                    }
                }
                "Special" => {} // skip BOS/EOS when rendering user-facing text
                other => {
                    warn!(token_type = other, "unknown token type during rendering");
                }
            }
        }

        debug!(output_len = text.len(), "render_text completed");
        Ok(text)
    }

    async fn generate_from_model(
        &self,
        token_ids: &mut Vec<i64>,
        max_tokens: usize,
        eos_id: i64,
    ) -> Result<usize, Status> {
        let context_len = self.state.ngram_size.saturating_sub(1);
        let mut generated = 0usize;

        debug!(
            initial_tokens = token_ids.len(),
            max_tokens, context_len, "generation started"
        );

        for step in 0..max_tokens {
            if token_ids.len() < context_len || context_len == 0 {
                debug!("not enough context to continue generation");
                break;
            }

            let start = token_ids.len() - context_len;
            let prefix: Vec<i64> = token_ids[start..].to_vec();

            debug!(
                step,
                prefix = ?prefix,
                "querying n-gram statistics"
            );

            let rows = sqlx::query(
                "SELECT next_token_id, count
                 FROM ngram_statistics
                 WHERE n = $1 AND prefix_tokens = $2",
            )
            .bind(self.state.ngram_size as i16)
            .bind(&prefix)
            .fetch_all(&self.state.pool)
            .await
            .map_err(internal_error)?;

            if rows.is_empty() {
                warn!(prefix = ?prefix, "no n-gram matches found");
                break;
            }

            let mut choices = Vec::with_capacity(rows.len());
            let mut weights = Vec::with_capacity(rows.len());

            for row in rows {
                let next: i64 = row.get("next_token_id");
                let count: i64 = row.get("count");
                choices.push(next);
                weights.push(count.max(1) as u64);
            }

            let dist = WeightedIndex::new(&weights)
                .map_err(|_| Status::internal("invalid n-gram weights"))?;

            let mut rng = rng();
            let idx = dist.sample(&mut rng);
            let next_token = choices[idx];

            debug!(
                step,
                next_token,
                weight = weights[idx],
                "sampled next token"
            );

            token_ids.push(next_token);
            generated += 1;

            if next_token == eos_id {
                debug!(step, "EOS reached, stopping generation");
                break;
            }
        }

        info!(generated, "generation completed");
        Ok(generated)
    }
}

#[tonic::async_trait]
impl TextGenerator for GeneratorService {
    async fn generate_text(
        &self,
        request: Request<GenerateTextRequest>,
    ) -> Result<Response<GenerateTextResponse>, Status> {
        let req = request.into_inner();

        info!(
            max_tokens = req.max_tokens,
            prefix_len = req.prefix.len(),
            "GenerateText request received"
        );

        let (text, generated) = self
            .generate_text_for_prefix(req.prefix, req.max_tokens as usize)
            .await?;

        Ok(Response::new(GenerateTextResponse {
            text,
            tokens_generated: generated as u32,
        }))
    }

    async fn tokenize_preview(
        &self,
        request: Request<TokenizePreviewRequest>,
    ) -> Result<Response<TokenizePreviewResponse>, Status> {
        let req = request.into_inner();

        debug!(input_len = req.text.len(), "TokenizePreview request");

        let tokens = tokenizer::tokenize_fragments(&[TextFragment::Text(req.text)]);

        debug!(token_count = tokens.len(), "tokenization completed");

        let rendered_tokens = tokens
            .into_iter()
            .map(|token| Token {
                token_type: token.kind.as_str().to_string(),
                token_value: token.value.unwrap_or_default(),
                emoji_document_id: token.emoji_document_id.unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(TokenizePreviewResponse {
            tokens: rendered_tokens,
        }))
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        debug!("health_check invoked");

        let status = match sqlx::query("SELECT 1").execute(&self.state.pool).await {
            Ok(_) => {
                debug!("health_check OK");
                HealthStatus::Ok
            }
            Err(err) => {
                error!(error = ?err, "health check failed");
                HealthStatus::Error
            }
        };

        Ok(Response::new(HealthCheckResponse {
            status: status as i32,
            message: "ok".to_string(),
        }))
    }

    async fn reload_training_data(
        &self,
        _request: Request<ReloadTrainingDataRequest>,
    ) -> Result<Response<ReloadTrainingDataResponse>, Status> {
        info!("manual ReloadTrainingData requested");

        let result = workers::ingestion::run_ingestion_cycle(self.state.clone()).await;

        let (reloaded, message) = match result {
            Ok(_) => {
                info!("training data reload completed");
                (true, "reload triggered".to_string())
            }
            Err(err) => {
                error!(error = ?err, "manual reload failed");
                (false, err.to_string())
            }
        };

        Ok(Response::new(ReloadTrainingDataResponse {
            reloaded,
            message,
        }))
    }
}

fn internal_error<E: ToString>(err: E) -> Status {
    error!(error = %err.to_string(), "internal error");
    Status::internal(err.to_string())
}
