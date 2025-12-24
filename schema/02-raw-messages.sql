CREATE TABLE IF NOT EXISTS raw_messages (
    id bigserial primary key,
    chat_id bigint not null,
    message_id bigint not null,
    from_id text,
    date timestamptz,
    raw_json jsonb not null
);