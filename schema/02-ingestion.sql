CREATE TABLE IF NOT EXISTS ingestion_runs (
    id bigserial PRIMARY KEY,
    export_path text NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    status text NOT NULL DEFAULT 'running',
    error text,
    messages_ingested bigint NOT NULL DEFAULT 0,
    tokens_ingested bigint NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS ingestion_offsets (
    id bigserial PRIMARY KEY,
    chat_id bigint NOT NULL,
    last_message_id bigint,
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (chat_id)
);
