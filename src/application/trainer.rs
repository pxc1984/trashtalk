use crate::domain::token::{TextFragment, bos_token, eos_token, tokenize_fragments};
use crate::infrastructure::store::Store;

/// Tokenizes a live chat message and records its n-gram statistics, scoped to
/// `chat_id`. Used by the bot to keep learning from the messages it sees in a
/// chat (privacy mode off).
pub async fn train_text(
    store: &Store,
    chat_id: i64,
    text: &str,
    max_n: usize,
    min_n: usize,
) -> anyhow::Result<()> {
    let tokens = tokenize_fragments(&[TextFragment::Text(text.to_string())]);
    let mut token_ids = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let id = store.ensure_token(token).await?;
        token_ids.push(id);
    }
    train_token_ids(store, chat_id, &token_ids, max_n, min_n).await
}

/// Wraps `token_ids` with BOS/EOS padding and records n-gram statistics for
/// the chat.
pub async fn train_token_ids(
    store: &Store,
    chat_id: i64,
    token_ids: &[i64],
    max_n: usize,
    min_n: usize,
) -> anyhow::Result<()> {
    let bos_id = store.ensure_token(&bos_token()).await?;
    let eos_id = store.ensure_token(&eos_token()).await?;
    let padding = max_n.saturating_sub(1).max(1);
    let mut training_ids = Vec::with_capacity(token_ids.len() + padding + 1);
    training_ids.extend(std::iter::repeat_n(bos_id, padding));
    training_ids.extend_from_slice(token_ids);
    training_ids.push(eos_id);
    update_ngrams(store, chat_id, &training_ids, max_n, min_n).await
}

/// Records n-gram statistics for every order in `min_n..=max_n`, scoped to the
/// chat the message came from.
///
/// Storing multiple orders enables backoff during generation: if the exact
/// higher-order continuation is unknown, the generator can fall back to a
/// shorter prefix instead of dead-ending.
pub async fn update_ngrams(
    store: &Store,
    chat_id: i64,
    token_ids: &[i64],
    max_n: usize,
    min_n: usize,
) -> anyhow::Result<()> {
    let min_n = min_n.max(2);
    for n in min_n..=max_n {
        update_order(store, chat_id, token_ids, n).await?;
    }
    Ok(())
}

async fn update_order(
    store: &Store,
    chat_id: i64,
    token_ids: &[i64],
    n: usize,
) -> anyhow::Result<()> {
    if n < 2 || token_ids.is_empty() {
        return Ok(());
    }

    for window_end in 0..token_ids.len() {
        if window_end + 1 < n {
            continue;
        }
        let start = window_end + 1 - n;
        let prefix = &token_ids[start..window_end];
        let next_token = token_ids[window_end];

        store.upsert_ngram(chat_id, n, prefix, next_token).await?;
    }

    Ok(())
}