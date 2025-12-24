CREATE TABLE IF NOT EXISTS messages (
    id bigserial primary key,
    chat_id bigint not null,
    from_id text,
    date timestamptz
);
CREATE TABLE IF NOT EXISTS message_tokens (
    message_id bigint references messages(id),
    position int not null,
    token_type text not null,
    token_value text,
    emoji_id bigint,
    primary key (message_id, position)
);