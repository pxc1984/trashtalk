CREATE TABLE IF NOT EXISTS chats (
    id bigserial PRIMARY KEY,
    chat_id bigint NOT NULL UNIQUE,
    title text,
    chat_type text,
    last_message_at timestamptz
);

CREATE TABLE IF NOT EXISTS custom_emojis (
    id bigserial PRIMARY KEY,
    document_id text NOT NULL UNIQUE,
    emoji_code text,
    description text
);

CREATE TABLE IF NOT EXISTS token_vocabulary (
    id bigserial PRIMARY KEY,
    token_type text NOT NULL,
    token_value text,
    emoji_id bigint REFERENCES custom_emojis(id),
    UNIQUE (token_type, token_value, emoji_id)
);

CREATE INDEX IF NOT EXISTS idx_token_vocab_type ON token_vocabulary (token_type);
