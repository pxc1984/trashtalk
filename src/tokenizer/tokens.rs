use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenKind {
    Word,
    Punctuation,
    Whitespace,
    Newline,
    CustomEmoji,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub value: Option<String>,
    pub emoji_document_id: Option<String>,
}

impl TokenKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TokenKind::Word => "Word",
            TokenKind::Punctuation => "Punctuation",
            TokenKind::Whitespace => "Whitespace",
            TokenKind::Newline => "Newline",
            TokenKind::CustomEmoji => "CustomEmoji",
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
