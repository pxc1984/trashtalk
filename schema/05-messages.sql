CREATE TABLE IF NOT EXISTS messages (
    id bigserial PRIMARY KEY,
    chat_id bigint NOT NULL,
    message_id bigint NOT NULL,
    from_id text,
    sent_at timestamptz,
    ingestion_run_id bigint REFERENCES ingestion_runs(id),
    raw_message_id bigint REFERENCES raw_messages(id),
    UNIQUE (chat_id, message_id)
);

CREATE TABLE IF NOT EXISTS message_tokens (
    message_id bigint REFERENCES messages(id) ON DELETE CASCADE,
    position int NOT NULL,
    token_id bigint NOT NULL REFERENCES token_vocabulary(id),
    PRIMARY KEY (message_id, position)
);

CREATE INDEX IF NOT EXISTS idx_messages_chat_msg ON messages (chat_id, message_id);
CREATE INDEX IF NOT EXISTS idx_message_tokens_token ON message_tokens (token_id);
