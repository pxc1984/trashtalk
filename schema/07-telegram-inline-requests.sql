CREATE TABLE IF NOT EXISTS telegram_inline_requests (
    id bigserial PRIMARY KEY,
    inline_query_id text NOT NULL,
    user_id bigint,
    user_username text,
    query_text text NOT NULL,
    response_text text,
    success boolean NOT NULL DEFAULT false,
    error_message text,
    chat_type text,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_telegram_inline_requests_created_at ON telegram_inline_requests (created_at DESC);