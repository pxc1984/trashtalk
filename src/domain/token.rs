use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Word,
    Punctuation,
    Whitespace,
    Newline,
    CustomEmoji,
    Sticker,
    Special,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: Option<String>,
    pub emoji_document_id: Option<String>,
}

/// A fragment of a raw message before tokenization. Produced by export/import
/// adapters (e.g. Telegram chat export) and reduced to tokens by
/// [`tokenize_fragments`].
#[derive(Debug, Clone)]
pub enum TextFragment {
    Text(String),
    CustomEmoji {
        document_id: String,
        text: Option<String>,
    },
    Sticker { file: String },
}

impl TokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenKind::Word => "Word",
            TokenKind::Punctuation => "Punctuation",
            TokenKind::Whitespace => "Whitespace",
            TokenKind::Newline => "Newline",
            TokenKind::CustomEmoji => "CustomEmoji",
            TokenKind::Sticker => "Sticker",
            TokenKind::Special => "Special",
        }
    }
}

impl Token {
    pub fn new(kind: TokenKind, value: Option<String>) -> Self {
        Self {
            kind,
            value,
            emoji_document_id: None,
        }
    }

    pub fn custom_emoji(document_id: String, value: Option<String>) -> Self {
        Self {
            kind: TokenKind::CustomEmoji,
            value,
            emoji_document_id: Some(document_id),
        }
    }

    /// A sticker token, keyed by the file that holds the sticker media (an
    /// absolute path for exported stickers, or a Telegram `file_id` for
    /// stickers seen live in a chat).
    pub fn sticker(file: String) -> Self {
        Self {
            kind: TokenKind::Sticker,
            value: Some(file),
            emoji_document_id: None,
        }
    }
}

pub const BOS_TOKEN_VALUE: &str = "<BOS>";
pub const EOS_TOKEN_VALUE: &str = "<EOS>";

pub fn bos_token() -> Token {
    Token::new(TokenKind::Special, Some(BOS_TOKEN_VALUE.to_string()))
}

pub fn eos_token() -> Token {
    Token::new(TokenKind::Special, Some(EOS_TOKEN_VALUE.to_string()))
}

pub fn tokenize_fragments(fragments: &[TextFragment]) -> Vec<Token> {
    let mut tokens = Vec::new();

    for fragment in fragments {
        match fragment {
            TextFragment::Text(text) => tokens.extend(tokenize_text(text)),
            TextFragment::CustomEmoji { document_id, text } => tokens.push(Token::custom_emoji(
                document_id.clone(),
                text.clone()
                    .or_else(|| Some(format!("<emoji:{}>", document_id))),
            )),
            TextFragment::Sticker { file } => tokens.push(Token::sticker(file.clone())),
        }
    }

    tokens
}

pub fn tokenize_text(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut current_word = String::new();

    for grapheme in text.graphemes(true) {
        let ch = grapheme.chars().next().unwrap_or(' ');

        if ch == '\n' {
            if !current_word.is_empty() {
                tokens.push(Token::new(
                    TokenKind::Word,
                    Some(std::mem::take(&mut current_word)),
                ));
            }
            tokens.push(Token::new(TokenKind::Newline, Some("\n".to_string())));
            continue;
        }

        if ch.is_whitespace() {
            if !current_word.is_empty() {
                tokens.push(Token::new(
                    TokenKind::Word,
                    Some(std::mem::take(&mut current_word)),
                ));
            }
            tokens.push(Token::new(
                TokenKind::Whitespace,
                Some(grapheme.to_string()),
            ));
            continue;
        }

        if ch.is_alphanumeric() || ch == '_' {
            current_word.push_str(grapheme);
            continue;
        }

        if !current_word.is_empty() {
            tokens.push(Token::new(
                TokenKind::Word,
                Some(std::mem::take(&mut current_word)),
            ));
        }

        tokens.push(Token::new(
            TokenKind::Punctuation,
            Some(grapheme.to_string()),
        ));
    }

    if !current_word.is_empty() {
        tokens.push(Token::new(
            TokenKind::Word,
            Some(std::mem::take(&mut current_word)),
        ));
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_words_and_punctuation() {
        let tokens = tokenize_text("Привет, мир!");
        let kinds: Vec<&str> = tokens.iter().map(|t| t.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec!["Word", "Punctuation", "Whitespace", "Word", "Punctuation"]
        );
        assert_eq!(tokens[0].value.as_deref(), Some("Привет"));
        assert_eq!(tokens[3].value.as_deref(), Some("мир"));
        assert_eq!(tokens[4].value.as_deref(), Some("!"));
    }

    #[test]
    fn tokenize_handles_newlines() {
        let tokens = tokenize_text("а\nб");
        let kinds: Vec<&str> = tokens.iter().map(|t| t.kind.as_str()).collect();
        assert_eq!(kinds, vec!["Word", "Newline", "Word"]);
    }

    #[test]
    fn custom_emoji_fragment_produces_emoji_token() {
        let tokens = tokenize_fragments(&[TextFragment::CustomEmoji {
            document_id: "123".to_string(),
            text: None,
        }]);
        assert_eq!(tokens[0].kind, TokenKind::CustomEmoji);
        assert_eq!(tokens[0].emoji_document_id.as_deref(), Some("123"));
    }

    #[test]
    fn sticker_fragment_produces_sticker_token() {
        let tokens = tokenize_fragments(&[TextFragment::Sticker {
            file: "stickers/foo.webp".to_string(),
        }]);
        assert_eq!(tokens[0].kind, TokenKind::Sticker);
        assert_eq!(tokens[0].value.as_deref(), Some("stickers/foo.webp"));
        assert_eq!(tokens[0].kind.as_str(), "Sticker");
    }
}