use std::thread;

use teloxide::{
    prelude::*,
    types::{
        ChatType, InlineQuery, InlineQueryResult, InlineQueryResultArticle, InputMessageContent,
        InputMessageContentText, Message,
    },
};
use tracing::{debug, error, info, warn};

use crate::application::generator::GeneratorService;
use crate::application::trainer::train_text;
use crate::state::SharedState;

pub fn spawn_bot(state: SharedState) -> Option<thread::JoinHandle<()>> {
    let Some(token) = state.config.bot_token.clone() else {
        info!("BOT_TOKEN not provided; Telegram inline bot disabled");
        return None;
    };

    let max_tokens = state.config.max_generation_length;

    Some(thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("telegram bot runtime");
        if let Err(err) = rt.block_on(run_bot(state, token, max_tokens)) {
            error!(error = ?err, "telegram bot exited with error");
        }
    }))
}

async fn run_bot(state: SharedState, token: String, max_tokens: usize) -> anyhow::Result<()> {
    info!("starting Telegram inline bot");
    let bot = Bot::new(token);

    let inline_handler = Update::filter_inline_query().endpoint({
        let state = state.clone();
        move |bot: Bot, query: InlineQuery| {
            let state = state.clone();
            async move { handle_inline_query(bot, query, state, max_tokens).await }
        }
    });

    let message_handler = Update::filter_message().endpoint({
        let state = state.clone();
        move |_bot: Bot, msg: Message| {
            let state = state.clone();
            async move { handle_message(msg, state).await }
        }
    });

    let handler = dptree::entry().branch(inline_handler).branch(message_handler);

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_inline_query(
    bot: Bot,
    query: InlineQuery,
    state: SharedState,
    max_tokens: usize,
) -> ResponseResult<()> {
    if query.query.trim().is_empty() {
        debug!("ignoring empty inline query");
        return Ok(());
    }

    debug!(user_id = %query.from.id, query = %query.query, "inline query received");

    let generator = GeneratorService::new(state.clone());
    let inline_query_id = query.id.clone().0;
    let user_id = i64::try_from(query.from.id.0).unwrap_or(-1);
    let username = query.from.username.clone();
    let chat_type = query.chat_type.as_ref().map(chat_type_as_str);
    // Inline queries carry no chat in this teloxide version, so generation
    // uses the global statistics (all messages).
    let response = generator
        .generate_text_for_prefix(query.query.clone(), max_tokens, None)
        .await;

    match response {
        Ok((text, _)) => {
            let reply_text = if text.trim().is_empty() {
                "Could not generate text for this prompt.".to_string()
            } else {
                text
            };

            let content = InputMessageContent::Text(InputMessageContentText::new(
                reply_text.clone(),
            ));

            let article = InlineQueryResultArticle::new(
                format!("{}-0", inline_query_id),
                "Generated message",
                content,
            )
            .description(preview(&reply_text, 128));

            if let Err(err) = bot
                .answer_inline_query(query.id.clone(), vec![InlineQueryResult::Article(article)])
                .cache_time(0)
                .send()
                .await
            {
                warn!(error = ?err, "failed to answer inline query");
            }

            if let Err(err) = state
                .store
                .log_inline_query(
                    &inline_query_id,
                    user_id,
                    username.as_deref(),
                    chat_type,
                    &query.query,
                    Some(&reply_text),
                    true,
                    None,
                )
                .await
            {
                warn!(error = ?err, "failed to log successful inline query");
            }
        }
        Err(err) => {
            error!(error = ?err, "generation failed for inline query");
            let article = InlineQueryResultArticle::new(
                format!("{}-err", inline_query_id),
                "Generation failed",
                InputMessageContent::Text(InputMessageContentText::new(
                    "Generation failed. Try again later.",
                )),
            )
            .description("Generation failed. Try again later.");

            if let Err(send_err) = bot
                .answer_inline_query(query.id.clone(), vec![InlineQueryResult::Article(article)])
                .cache_time(0)
                .is_personal(true)
                .send()
                .await
            {
                warn!(error = ?send_err, "failed to send error response for inline query");
            }

            if let Err(log_err) = state
                .store
                .log_inline_query(
                    &inline_query_id,
                    user_id,
                    username.as_deref(),
                    chat_type,
                    &query.query,
                    None,
                    false,
                    Some(&err.to_string()),
                )
                .await
            {
                warn!(error = ?log_err, "failed to log failed inline query");
            }
        }
    }

    Ok(())
}

/// Trains the model on a message received in a chat. With privacy mode off the
/// bot sees every message, so each one is folded into that chat's n-gram
/// statistics, keeping the model in sync with the conversations it watches.
async fn handle_message(msg: Message, state: SharedState) -> ResponseResult<()> {
    // Skip messages sent via an inline bot (the "via @cutalkbot" marker): these
    // are generated text, not something the model should learn from.
    if let Some(via) = &msg.via_bot {
        debug!(via_bot = ?via.username, msg_id = msg.id.0, "ignoring inline-bot message");
        return Ok(());
    }

    let Some(text) = msg.text() else {
        return Ok(());
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('/') {
        return Ok(());
    }

    let chat_id = msg.chat.id.0;
    debug!(chat_id, msg_id = msg.id.0, "training on chat message");

    if let Err(err) = train_text(
        &state.store,
        chat_id,
        text,
        state.ngram_size,
        state.min_ngram_size,
    )
    .await
    {
        warn!(error = ?err, "failed to train on chat message");
    }

    Ok(())
}

fn chat_type_as_str(chat_type: &ChatType) -> &str {
    match chat_type {
        ChatType::Sender => "sender",
        ChatType::Private => "private",
        ChatType::Group => "group",
        ChatType::Supergroup => "supergroup",
        ChatType::Channel => "channel",
    }
}

fn preview(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max_len).collect::<String>())
}