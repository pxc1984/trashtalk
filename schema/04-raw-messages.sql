CREATE TABLE IF NOT EXISTS raw_messages (
    id bigserial PRIMARY KEY,
    chat_id bigint NOT NULL,
    message_id bigint NOT NULL,
    ingestion_run_id bigint REFERENCES ingestion_runs(id),
    from_id text,
    sent_at timestamptz,
    raw_json jsonb NOT NULL,
    UNIQUE (chat_id, message_id)
);

CREATE INDEX IF NOT EXISTS idx_raw_messages_chat_msg ON raw_messages (chat_id, message_id);
CREATE INDEX IF NOT EXISTS idx_raw_messages_run ON raw_messages (ingestion_run_id);
