use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::domain::token::TextFragment;

#[derive(Debug, Clone)]
pub struct NormalizedMessage {
    pub chat_id: i64,
    pub message_id: i64,
    pub from_id: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub fragments: Vec<TextFragment>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ParsedExport {
    pub chat_id: i64,
    pub chat_title: Option<String>,
    pub chat_type: Option<String>,
    pub messages: Vec<NormalizedMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    #[serde(rename = "type")]
    message_type: String,
    id: i64,
    from_id: Option<String>,
    date: String,
    date_unixtime: Option<String>,
    text: Option<serde_json::Value>,
}

/// Reads a Telegram chat export directory and normalizes its messages.
pub fn read_export(export_dir: &Path) -> anyhow::Result<ParsedExport> {
    let content = std::fs::read_to_string(export_dir.join("result.json"))
        .with_context(|| format!("reading export file in {}", export_dir.display()))?;
    let root: serde_json::Value =
        serde_json::from_str(&content).context("parsing Telegram export JSON")?;

    let chat_id = root.get("id").and_then(|v| v.as_i64()).unwrap_or_default();
    let chat_title = root
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let chat_type = root
        .get("type")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let messages_value = root
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut normalized = Vec::new();
    for value in messages_value {
        let msg: TelegramMessage = match serde_json::from_value(value.clone()) {
            Ok(msg) => msg,
            Err(_) => continue,
        };

        if msg.message_type != "message" {
            continue;
        }

        let sent_at = parse_date(&msg).unwrap_or_else(|_| Utc::now());
        let fragments = fragments_from_text(&msg.text);

        normalized.push(NormalizedMessage {
            chat_id,
            message_id: msg.id,
            from_id: msg.from_id,
            sent_at,
            fragments,
            raw: value,
        });
    }

    Ok(ParsedExport {
        chat_id,
        chat_title,
        chat_type,
        messages: normalized,
    })
}

fn parse_date(msg: &TelegramMessage) -> anyhow::Result<DateTime<Utc>> {
    if let Some(unix) = msg.date_unixtime.as_deref().and_then(|v| v.parse::<i64>().ok())
        && let Some(dt) = DateTime::<Utc>::from_timestamp(unix, 0)
    {
        return Ok(dt);
    }

    let parsed = DateTime::parse_from_rfc3339(&msg.date)
        .or_else(|_| DateTime::parse_from_str(&msg.date, "%Y-%m-%d %H:%M:%S%z"))
        .context("parsing message date")?;
    Ok(parsed.with_timezone(&Utc))
}

fn fragments_from_text(text: &Option<serde_json::Value>) -> Vec<TextFragment> {
    match text {
        Some(serde_json::Value::String(s)) => vec![TextFragment::Text(s.clone())],
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                serde_json::Value::String(s) => Some(TextFragment::Text(s.clone())),
                serde_json::Value::Object(map) => {
                    let kind = map.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                    match kind {
                        "custom_emoji" => {
                            map.get("document_id")
                                .and_then(|id| id.as_str())
                                .map(|document_id| TextFragment::CustomEmoji {
                                    document_id: document_id.to_string(),
                                    text: map
                                        .get("text")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string()),
                                })
                        }
                        _ => map
                            .get("text")
                            .and_then(|v| v.as_str())
                            .map(|s| TextFragment::Text(s.to_string())),
                    }
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}