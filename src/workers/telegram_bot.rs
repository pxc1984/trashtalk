use std::thread;

use teloxide::{
    prelude::*,
    types::{
        InlineQuery, InlineQueryResult, InlineQueryResultArticle, InputMessageContent,
        InputMessageContentText,
    },
};
use tracing::{debug, error, info, warn};

use crate::{grpc::generator::GeneratorService, state::SharedState};

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

    let handler = Update::filter_inline_query().endpoint(move |bot: Bot, query: InlineQuery| {
        let state = state.clone();
        async move {
            if query.query.trim().is_empty() {
                debug!("ignoring empty inline query");
                return Ok::<_, teloxide::RequestError>(());
            }

            debug!(user_id = %query.from.id, query = %query.query, "inline query received");

            let generator = GeneratorService::new(state.clone());
            let response = generator
                .generate_text_for_prefix(query.query.clone(), max_tokens)
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
                        format!("{}-0", query.id.clone().0),
                        "Generated message",
                        content,
                    )
                    .description(preview(&reply_text, 128));

                    if let Err(err) = bot
                        .answer_inline_query(query.id, vec![InlineQueryResult::Article(article)])
                        .cache_time(0)
                        .send()
                        .await
                    {
                        warn!(error = ?err, "failed to answer inline query");
                    }
                }
                Err(err) => {
                    error!(error = ?err, "generation failed for inline query");
                    let article = InlineQueryResultArticle::new(
                        format!("{}-err", query.id.clone().0),
                        "Generation failed",
                        InputMessageContent::Text(InputMessageContentText::new(
                            "Generation failed. Try again later.",
                        )),
                    )
                    .description("Generation failed. Try again later.");

                    if let Err(send_err) = bot
                        .answer_inline_query(query.id, vec![InlineQueryResult::Article(article)])
                        .cache_time(0)
                        .is_personal(true)
                        .send()
                        .await
                    {
                        warn!(error = ?send_err, "failed to send error response for inline query");
                    }
                }
            }

            Ok(())
        }
    });

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

fn preview(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max_len).collect::<String>())
}
