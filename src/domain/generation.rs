use std::collections::HashSet;

/// String variant used when the token kind comes from storage rather than a
/// typed `TokenKind`.
pub fn is_sentence_boundary_str(kind: &str, value: Option<&str>) -> bool {
    match kind {
        "Newline" => true,
        "Punctuation" => matches!(value, Some(".") | Some("!") | Some("?") | Some("…")),
        _ => false,
    }
}

/// Tunable parameters that shape the generated text.
///
/// These are pure domain values: the application layer turns them into a
/// sampling strategy over the stored n-gram statistics.
#[derive(Debug, Clone, Copy)]
pub struct GenerationParams {
    /// Sampling temperature. `1.0` samples proportionally to observed counts,
    /// lower values sharpen towards the most frequent continuation, higher
    /// values flatten the distribution and increase variety.
    pub temperature: f32,
    /// Keep only the `top_k` most likely candidates before sampling.
    /// `0` disables the limit.
    pub top_k: usize,
    /// Nucleus sampling: keep the smallest set of candidates whose cumulative
    /// probability reaches `top_p`. `1.0` disables the cutoff.
    pub top_p: f32,
    /// Multiplicative penalty applied to candidates that already appear in the
    /// generated sequence. Values above `1.0` break the repetition loops a
    /// Markov chain tends to fall into (e.g. "ло ло ло").
    pub repetition_penalty: f32,
    /// Minimum number of tokens to generate before stopping early at a natural
    /// sentence boundary (punctuation / newline / EOS).
    pub min_stop_tokens: usize,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            temperature: 1.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.3,
            min_stop_tokens: 4,
        }
    }
}

/// Heuristic quality score for a generated candidate. Higher is better.
///
/// Rewards vocabulary richness and penalizes repetitiveness, adjacent word
/// loops and hard truncation. Used to pick the best of several sampled
/// continuations (best-of-N reranking).
pub fn score_generation(text: &str, is_truncated: bool) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return f64::NEG_INFINITY;
    }

    let words: Vec<&str> = trimmed
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return -100.0;
    }

    let total = words.len() as f64;
    let unique = words.iter().copied().collect::<HashSet<_>>().len() as f64;

    // Vocabulary richness: reward many distinct words.
    let content = unique;
    // Repetitiveness: 1.0 when every word repeats, 0.0 when all unique.
    let repetitiveness = 1.0 - unique / total;
    // Repeated adjacent words (e.g. "ло ло ло") are a strong degeneracy signal.
    let adjacent_repeats = words.windows(2).filter(|w| w[0] == w[1]).count() as f64;

    let mut score = content - repetitiveness * total * 0.5 - adjacent_repeats * 3.0;
    if is_truncated {
        // Reached the hard cap without a natural ending: slight penalty.
        score -= 2.0;
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_end_punctuation_is_a_boundary() {
        assert!(is_sentence_boundary_str("Punctuation", Some(".")));
        assert!(is_sentence_boundary_str("Punctuation", Some("!")));
        assert!(is_sentence_boundary_str("Punctuation", Some("?")));
        assert!(is_sentence_boundary_str("Punctuation", Some("…")));
    }

    #[test]
    fn newline_is_a_boundary() {
        assert!(is_sentence_boundary_str("Newline", None));
    }

    #[test]
    fn non_boundary_tokens_are_not_boundaries() {
        assert!(!is_sentence_boundary_str("Punctuation", Some(",")));
        assert!(!is_sentence_boundary_str("Punctuation", Some(";")));
        assert!(!is_sentence_boundary_str("Word", Some(".")));
        assert!(!is_sentence_boundary_str("Whitespace", Some(" ")));
    }

    #[test]
    fn empty_text_scores_worst() {
        assert!(score_generation("", false) <= score_generation("hello world", false));
    }

    #[test]
    fn more_unique_words_scores_higher() {
        let diverse = score_generation("один два три четыре пять", false);
        let repetitive = score_generation("один один один", false);
        assert!(diverse > repetitive);
    }

    #[test]
    fn adjacent_repetition_is_penalized() {
        let no_repeat = score_generation("ло ха ло ха", false);
        let looped = score_generation("ло ло ло ло", false);
        assert!(no_repeat > looped);
    }

    #[test]
    fn truncation_is_penalized() {
        let a = score_generation("один два три", false);
        let b = score_generation("один два три", true);
        assert!(a > b);
    }
}