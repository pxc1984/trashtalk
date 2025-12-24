use std::collections::HashMap;

use rand::distr::{Distribution, weighted::WeightedIndex};
use rand::rng;
use sqlx::Row;
use tonic::{Request, Response, Status};

use crate::{
    db::{self, TokenRecord},
    grpc::proto::{
        GenerateTextRequest, GenerateTextResponse, HealthCheckRequest, HealthCheckResponse,
        HealthStatus, ReloadTrainingDataRequest, ReloadTrainingDataResponse, Token,
        TokenizePreviewRequest, TokenizePreviewResponse, text_generator_server::TextGenerator,
    },
    state::SharedState,
    tokenizer::{self, telegram::TextFragment},
    workers,
};

pub struct GeneratorService {
    state: SharedState,
}

impl GeneratorService {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    async fn render_text(&self, token_ids: &[i64]) -> Result<String, Status> {
        let records = db::fetch_tokens(&self.state.pool, token_ids)
            .await
            .map_err(internal_error)?;
        let by_id: HashMap<i64, TokenRecord> = records.into_iter().map(|r| (r.id, r)).collect();

        let mut text = String::new();
        for id in token_ids {
            let Some(token) = by_id.get(id) else {
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
                        text.push_str("[emoji]");
                    }
                }
                _ => {}
            }
        }

        Ok(text)
    }

    async fn generate_from_model(
        &self,
        token_ids: &mut Vec<i64>,
        max_tokens: usize,
    ) -> Result<usize, Status> {
        let mut generated = 0usize;
        let context_len = self.state.ngram_size.saturating_sub(1);

        for _ in 0..max_tokens {
            if token_ids.len() < context_len || context_len == 0 {
                break;
            }

            let start = token_ids.len() - context_len;
            let prefix: Vec<i64> = token_ids[start..].to_vec();

            let rows = sqlx::query(
                "SELECT next_token_id, count FROM ngram_statistics WHERE n = $1 AND prefix_tokens = $2",
            )
            .bind(self.state.ngram_size as i16)
            .bind(&prefix)
            .fetch_all(&self.state.pool)
            .await
            .map_err(internal_error)?;

            if rows.is_empty() {
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

            let dist = WeightedIndex::new(weights)
                .map_err(|_| Status::internal("invalid n-gram weights"))?;
            let mut rng = rng();
            let idx = dist.sample(&mut rng);
            let next_token = choices[idx];
            token_ids.push(next_token);
            generated += 1;
        }

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
        let max_tokens = req.max_tokens.max(1) as usize;

        let prefix_tokens =
            tokenizer::tokenize_fragments(&[TextFragment::Text(req.prefix.clone())]);
        let mut token_ids = Vec::new();
        for token in prefix_tokens.iter() {
            let id = db::ensure_token(&self.state.pool, token)
                .await
                .map_err(internal_error)?;
            token_ids.push(id);
        }

        let generated = self
            .generate_from_model(
                &mut token_ids,
                max_tokens.min(self.state.config.max_generation_length),
            )
            .await?;
        let text = self.render_text(&token_ids).await?;

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
        let tokens = tokenizer::tokenize_fragments(&[TextFragment::Text(req.text)]);

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
        let status = match sqlx::query("SELECT 1").execute(&self.state.pool).await {
            Ok(_) => HealthStatus::Ok,
            Err(err) => {
                tracing::error!(error = ?err, "health check failed");
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
        let result = workers::ingestion::run_ingestion_cycle(self.state.clone()).await;
        let (reloaded, message) = match result {
            Ok(_) => (true, "reload triggered".to_string()),
            Err(err) => {
                tracing::error!(error = ?err, "manual reload failed");
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
    Status::internal(err.to_string())
}
