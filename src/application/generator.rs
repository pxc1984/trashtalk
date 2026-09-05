use std::collections::HashMap;

use rand::distr::{Distribution, weighted::WeightedIndex};
use rand::rng;
use tracing::{debug, info, warn};

use crate::domain::generation::{GenerationParams, is_sentence_boundary_str, score_generation};
use crate::domain::token::{TextFragment, bos_token, eos_token, tokenize_fragments};
use crate::infrastructure::db::{
    ngram_repository,
    token_repository::{TokenRecord, ensure_token, fetch_token, fetch_tokens},
};
use crate::state::SharedState;

/// Orchestrates text generation from the stored n-gram statistics.
///
/// Generation is intentionally richer than a plain Markov chain over a single
/// order:
/// - **Backoff**: if the exact high-order continuation is unknown, fall back to
///   lower-order prefixes instead of stopping.
/// - **Diversity**: temperature, top-k and top-p (nucleus) sampling avoid the
///   generic, repetitive continuations that raw frequency weighting produces.
/// - **Natural stopping**: once a minimum length is reached, generation prefers
///   to stop at a sentence boundary rather than mid-word.
pub struct GeneratorService {
    state: SharedState,
}

impl GeneratorService {
    pub fn new(state: SharedState) -> Self {
        Self { state }
    }

    pub async fn generate_text_for_prefix(
        &self,
        prefix: String,
        max_tokens: usize,
    ) -> anyhow::Result<(String, usize)> {
        info!(
            max_tokens,
            prefix_len = prefix.len(),
            "generate_text_for_prefix invoked"
        );

        let max_tokens = max_tokens.max(1).min(self.state.config.max_generation_length);
        let params = self.generation_params();
        let num_candidates = self.state.config.num_candidates.max(1);

        let bos_id = ensure_token(&self.state.pool, &bos_token()).await?;
        let eos_id = ensure_token(&self.state.pool, &eos_token()).await?;

        let prefix_tokens = tokenize_fragments(&[TextFragment::Text(prefix)]);

        debug!(token_count = prefix_tokens.len(), "prefix tokenized");

        let bos_padding = self.state.ngram_size.saturating_sub(1).max(1);

        let mut base = Vec::with_capacity(prefix_tokens.len() + bos_padding);
        base.extend(std::iter::repeat_n(bos_id, bos_padding));
        for token in &prefix_tokens {
            let id = ensure_token(&self.state.pool, token).await?;
            base.push(id);
        }

        // BOS-only seed used when the prefix cannot be continued at all.
        let bos_only: Vec<i64> = base[..bos_padding].to_vec();

        let (best_ids, generated) = self
            .best_of_candidates(&base, &bos_only, bos_id, eos_id, max_tokens, params, num_candidates)
            .await?;

        let text = self.render_text(&best_ids, bos_id, eos_id).await?;

        info!(generated, final_len = text.len(), "text generation completed");

        Ok((text, generated))
    }

    fn generation_params(&self) -> GenerationParams {
        GenerationParams {
            temperature: self.state.config.generation_temperature,
            top_k: self.state.config.generation_top_k,
            top_p: self.state.config.generation_top_p,
            repetition_penalty: self.state.config.repetition_penalty,
            min_stop_tokens: self.state.config.min_generation_tokens,
        }
    }

    /// Samples `num_candidates` continuations from `base` and keeps the one
    /// with the best quality score. A candidate that cannot continue the
    /// prefix is retried from the BOS-only seed.
    #[allow(clippy::too_many_arguments)]
    async fn best_of_candidates(
        &self,
        base: &[i64],
        bos_only: &[i64],
        bos_id: i64,
        eos_id: i64,
        max_tokens: usize,
        params: GenerationParams,
        num_candidates: usize,
    ) -> anyhow::Result<(Vec<i64>, usize)> {
        let mut best_ids = base.to_vec();
        let mut best_generated = 0usize;
        let mut best_score = f64::NEG_INFINITY;

        for _ in 0..num_candidates {
            let mut cand = base.to_vec();
            let mut generated =
                self.generate_from_model(&mut cand, max_tokens, eos_id, params).await?;

            if generated == 0 && base.len() > bos_only.len() {
                info!("candidate failed to continue, retrying from BOS-only prefix");
                cand = bos_only.to_vec();
                generated = self.generate_from_model(&mut cand, max_tokens, eos_id, params).await?;
            }

            let text = self.render_text(&cand, bos_id, eos_id).await?;
            let is_truncated = generated >= max_tokens;
            let score = score_generation(&text, is_truncated);

            debug!(score, generated, "candidate scored");

            if score > best_score {
                best_score = score;
                best_ids = cand;
                best_generated = generated;
            }
        }

        Ok((best_ids, best_generated))
    }

    async fn render_text(
        &self,
        token_ids: &[i64],
        bos_id: i64,
        eos_id: i64,
    ) -> anyhow::Result<String> {
        debug!(token_count = token_ids.len(), "render_text called");

        let records = fetch_tokens(&self.state.pool, token_ids).await?;

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

    /// Runs the autoregressive loop: backoff search over n-gram orders, then a
    /// temperature/top-k/top-p sample, stopping early at a sentence boundary.
    async fn generate_from_model(
        &self,
        token_ids: &mut Vec<i64>,
        max_tokens: usize,
        eos_id: i64,
        params: GenerationParams,
    ) -> anyhow::Result<usize> {
        let mut generated = 0usize;

        for step in 0..max_tokens {
            let Some(next_token) = self.sample_next(token_ids, params).await? else {
                debug!(step, "no continuation found at any n-gram order");
                break;
            };

            debug!(step, next_token, "sampled next token");

            token_ids.push(next_token);
            generated += 1;

            if next_token == eos_id {
                debug!(step, "EOS reached, stopping generation");
                break;
            }

            if generated >= params.min_stop_tokens && self.is_boundary_token(next_token).await? {
                debug!(step, "stopped at sentence boundary");
                break;
            }
        }

        info!(generated, "generation completed");
        Ok(generated)
    }

    /// Searches n-gram orders from highest to lowest and returns the sampled
    /// continuation for the first order that has any observed candidates.
    async fn sample_next(
        &self,
        token_ids: &[i64],
        params: GenerationParams,
    ) -> anyhow::Result<Option<i64>> {
        let max_order = self.state.ngram_size.min(token_ids.len() + 1);
        let seen = token_counts(token_ids);

        for order in (self.state.min_ngram_size..=max_order).rev() {
            let prefix_len = order - 1;
            if prefix_len > token_ids.len() {
                continue;
            }
            let start = token_ids.len() - prefix_len;
            let prefix = &token_ids[start..];

            debug!(order, prefix = ?prefix, "querying n-gram statistics");

            let candidates =
                ngram_repository::query_next_candidates(&self.state.pool, order, prefix).await?;

            if candidates.is_empty() {
                debug!(order, prefix = ?prefix, "no matches at this order, backing off");
                continue;
            }

            let next = sample_from_candidates(&candidates, &seen, &params)?;
            return Ok(Some(next));
        }

        Ok(None)
    }

    /// Whether the given token is a natural end-of-sentence marker.
    async fn is_boundary_token(&self, token_id: i64) -> anyhow::Result<bool> {
        Ok(match fetch_token(&self.state.pool, token_id).await? {
            Some(record) => {
                is_sentence_boundary_str(&record.token_type, record.token_value.as_deref())
            }
            None => false,
        })
    }
}

/// Counts token occurrences in a sequence, used to apply the repetition
/// penalty during sampling.
fn token_counts(ids: &[i64]) -> HashMap<i64, u32> {
    let mut counts = HashMap::new();
    for id in ids {
        *counts.entry(*id).or_insert(0) += 1;
    }
    counts
}

/// Temperature / top-k / top-p / repetition-penalty sampling over n-gram
/// candidates. `seen` is the count of each token already in the generated
/// sequence; candidates that repeat it are down-weighted to break loops.
fn sample_from_candidates(
    candidates: &[(i64, i64)],
    seen: &HashMap<i64, u32>,
    params: &GenerationParams,
) -> anyhow::Result<i64> {
    let temp = params.temperature.max(0.0);

    // Temperature: shape the distribution away from pure frequency weighting.
    let mut weighted: Vec<(usize, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, (_, count))| {
            let weight = if temp <= 0.0 {
                *count as f64
            } else {
                (*count as f64).powf(1.0 / temp as f64)
            };
            (i, weight)
        })
        .collect();

    // Repetition penalty: down-weight candidates already used in the sequence.
    if params.repetition_penalty > 1.0 {
        let penalty = params.repetition_penalty as f64;
        for (i, w) in weighted.iter_mut() {
            if let Some(&cnt) = seen.get(&candidates[*i].0) {
                *w *= penalty.powf(-(cnt as f64));
            }
        }
    }

    // Top-k: keep only the k most likely continuations.
    if params.top_k > 0 {
        weighted.sort_by(|a, b| b.1.total_cmp(&a.1));
        weighted.truncate(params.top_k);
    }

    // Top-p (nucleus): keep the smallest set whose cumulative mass >= top_p.
    if params.top_p < 1.0 {
        weighted.sort_by(|a, b| b.1.total_cmp(&a.1));
        let total: f64 = weighted.iter().map(|(_, w)| w).sum();
        if total > 0.0 {
            let mut cum = 0.0;
            let mut keep = 0usize;
            for (_, w) in weighted.iter() {
                cum += w / total;
                keep += 1;
                if cum >= params.top_p as f64 {
                    break;
                }
            }
            weighted.truncate(keep.max(1));
        }
    }

    let best = weighted
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| *i)
        .ok_or_else(|| anyhow::anyhow!("no candidates to sample from"))?;

    // temperature <= 0 means fully greedy.
    if temp <= 0.0 {
        return Ok(candidates[best].0);
    }

    let weights: Vec<f64> = weighted.iter().map(|(_, w)| *w).collect();
    let dist = WeightedIndex::new(weights)
        .map_err(|e| anyhow::anyhow!("invalid n-gram weights: {e}"))?;

    let mut rng = rng();
    let chosen = dist.sample(&mut rng);
    Ok(candidates[weighted[chosen].0].0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::generation::GenerationParams;

    fn params() -> GenerationParams {
        GenerationParams::default()
    }

    fn no_seen() -> HashMap<i64, u32> {
        HashMap::new()
    }

    #[test]
    fn greedy_picks_highest_count() {
        let candidates = [(1, 5), (2, 50), (3, 1)];
        let mut p = params();
        p.temperature = 0.0;
        for _ in 0..50 {
            assert_eq!(sample_from_candidates(&candidates, &no_seen(), &p).unwrap(), 2);
        }
    }

    #[test]
    fn greedy_ignores_top_k_when_one_dominant() {
        let candidates = [(1, 5), (2, 50), (3, 1)];
        let mut p = params();
        p.temperature = 0.0;
        p.top_k = 1;
        for _ in 0..50 {
            assert_eq!(sample_from_candidates(&candidates, &no_seen(), &p).unwrap(), 2);
        }
    }

    #[test]
    fn greedy_respects_top_p() {
        // With temperature 0 the largest-count candidate always wins, but the
        // nucleus cutoff must still resolve to a valid candidate.
        let candidates = [(1, 1), (2, 1), (3, 100)];
        let mut p = params();
        p.temperature = 0.0;
        p.top_p = 0.5;
        for _ in 0..50 {
            assert_eq!(sample_from_candidates(&candidates, &no_seen(), &p).unwrap(), 3);
        }
    }

    #[test]
    fn empty_candidates_are_an_error() {
        assert!(sample_from_candidates(&[], &no_seen(), &params()).is_err());
    }

    #[test]
    fn repetition_penalty_discourages_reusing_seen_token_in_greedy() {
        // Token 1 has the highest count but is already heavily used; the
        // penalty must push a greedy sample towards the unused token 2.
        let candidates = [(1, 100), (2, 10)];
        let mut p = params();
        p.temperature = 0.0;
        p.repetition_penalty = 4.0;

        let mut seen = HashMap::new();
        seen.insert(1, 5); // 1 already appeared 5 times
        for _ in 0..50 {
            assert_eq!(sample_from_candidates(&candidates, &seen, &p).unwrap(), 2);
        }

        // Without repetition, token 1 would be chosen greedily.
        for _ in 0..50 {
            assert_eq!(sample_from_candidates(&candidates, &no_seen(), &p).unwrap(), 1);
        }
    }

    #[test]
    fn token_counts_aggregate_occurrences() {
        let counts = token_counts(&[7, 7, 9, 7, 10]);
        assert_eq!(counts.get(&7), Some(&3));
        assert_eq!(counts.get(&9), Some(&1));
        assert_eq!(counts.get(&10), Some(&1));
    }
}