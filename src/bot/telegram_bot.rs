use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use rand::Rng;
use teloxide::{
    prelude::*,
    types::{
        Chat, ChatId, ChatKind, ChatMemberStatus, ChatMemberUpdated, ChatType, InlineQuery,
        InlineQueryResult, InlineQueryResultArticle, InputMessageContent, InputMessageContentText,
        Message, PublicChatKind,
    },
};
use tracing::{debug, error, info, warn};

use crate::application::generator::GeneratorService;
use crate::application::trainer::train_text;
use crate::state::SharedState;

/// Per-chat schedule: chat_id -> when the next random message is due.
type BotChats = Arc<Mutex<HashMap<i64, Instant>>>;

/// Random-message interval bounds: 10 minutes to 12 hours.
const MIN_INTERVAL_SECS: u64 = 10 * 60;
const MAX_INTERVAL_SECS: u64 = 12 * 60 * 60;

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
    info!("starting Telegram bot");
    let bot = Bot::new(token);
    let chats: BotChats = Arc::new(Mutex::new(HashMap::new()));

    let inline_handler = Update::filter_inline_query().endpoint({
        let state = state.clone();
        move |bot: Bot, query: InlineQuery| {
            let state = state.clone();
            async move { handle_inline_query(bot, query, state, max_tokens).await }
        }
    });

    let message_handler = Update::filter_message().endpoint({
        let state = state.clone();
        let chats = chats.clone();
        move |_bot: Bot, msg: Message| {
            let state = state.clone();
            let chats = chats.clone();
            async move { handle_message(msg, state, chats).await }
        }
    });

    let member_handler = Update::filter_my_chat_member().endpoint({
        let chats = chats.clone();
        move |_bot: Bot, update: ChatMemberUpdated| {
            let chats = chats.clone();
            async move { handle_my_chat_member(update, chats).await }
        }
    });

    let handler = dptree::entry()
        .branch(inline_handler)
        .branch(message_handler)
        .branch(member_handler);

    // Background task that periodically sends a random, chat-scoped message to
    // each group the bot is a member of.
    tokio::spawn(chat_message_loop(bot.clone(), state.clone(), chats.clone()));

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
async fn handle_message(msg: Message, state: SharedState, chats: BotChats) -> ResponseResult<()> {
    // Skip messages sent via an inline bot (the "via @cutalkbot" marker): these
    // are generated text, not something the model should learn from.
    if let Some(via) = &msg.via_bot {
        debug!(via_bot = ?via.username, msg_id = msg.id.0, "ignoring inline-bot message");
        return Ok(());
    }

    let chat_id = msg.chat.id.0;

    // A group message confirms the bot is a member; make sure it is scheduled.
    if is_group(&msg.chat) {
        register_chat(&chats, chat_id);
    }

    let Some(text) = msg.text() else {
        return Ok(());
    };
    let text = text.trim();
    if text.is_empty() || text.starts_with('/') {
        return Ok(());
    }

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

/// Registers the chat when the bot is added to a group (or becomes an
/// administrator), scheduling its first random message.
async fn handle_my_chat_member(
    update: ChatMemberUpdated,
    chats: BotChats,
) -> ResponseResult<()> {
    let is_member = matches!(
        update.new_chat_member.status(),
        ChatMemberStatus::Member | ChatMemberStatus::Administrator
    );
    if !is_member || !is_group(&update.chat) {
        return Ok(());
    }

    register_chat(&chats, update.chat.id.0);
    info!(chat_id = update.chat.id.0, "bot added to a group; scheduling random messages");
    Ok(())
}

/// Periodically sends a random, chat-scoped message to every scheduled group,
/// each on its own random interval of 10 minutes to 12 hours.
async fn chat_message_loop(bot: Bot, state: SharedState, chats: BotChats) {
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        let now = Instant::now();

        let due: Vec<i64> = {
            let guard = chats.lock().unwrap();
            guard
                .iter()
                .filter(|(_, next)| **next <= now)
                .map(|(id, _)| *id)
                .collect()
        };

        for chat_id in due {
            if let Err(err) = send_random_message(&bot, &state, chat_id).await {
                warn!(chat_id, error = ?err, "failed to send random chat message");
            }

            let mut guard = chats.lock().unwrap();
            if let Some(next) = guard.get_mut(&chat_id) {
                *next = Instant::now() + random_interval();
            }
        }
    }
}

/// Generates a random message from a chat's own model and posts it to the chat.
async fn send_random_message(bot: &Bot, state: &SharedState, chat_id: i64) -> anyhow::Result<()> {
    let generator = GeneratorService::new(state.clone());
    let (text, _) = generator
        .generate_text_for_prefix(String::new(), state.config.max_generation_length, Some(chat_id))
        .await?;

    let text = text.trim();
    if text.is_empty() {
        debug!(chat_id, "empty random message, skipping send");
        return Ok(());
    }

    info!(chat_id, text = %text, "sending random chat message");
    bot.send_message(ChatId(chat_id), text).await?;
    Ok(())
}

/// Schedules a chat for random messages unless it already is, using the
/// current random delay.
fn register_chat(chats: &BotChats, chat_id: i64) {
    let mut guard = chats.lock().unwrap();
    guard
        .entry(chat_id)
        .or_insert_with(|| Instant::now() + random_interval());
}

/// A random delay between 10 minutes and 12 hours.
fn random_interval() -> Duration {
    let secs = rand::rng().random_range(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS);
    Duration::from_secs(secs)
}

fn is_group(chat: &Chat) -> bool {
    match &chat.kind {
        ChatKind::Public(public) => {
            matches!(public.kind, PublicChatKind::Group | PublicChatKind::Supergroup(_))
        }
        ChatKind::Private(_) => false,
    }
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