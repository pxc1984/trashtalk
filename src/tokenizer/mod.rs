pub mod telegram;
pub mod tokens;

use tokens::{Token, tokenize_text};

use crate::tokenizer::telegram::TextFragment;

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
        }
    }

    tokens
}
