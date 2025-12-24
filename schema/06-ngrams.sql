CREATE TABLE IF NOT EXISTS ngram_statistics (
    id bigserial PRIMARY KEY,
    n smallint NOT NULL,
    prefix_tokens bigint[] NOT NULL,
    next_token_id bigint NOT NULL REFERENCES token_vocabulary(id),
    count bigint NOT NULL DEFAULT 1,
    UNIQUE (n, prefix_tokens, next_token_id)
);

CREATE INDEX IF NOT EXISTS idx_ngram_prefix ON ngram_statistics USING gin (prefix_tokens);
CREATE INDEX IF NOT EXISTS idx_ngram_next_token ON ngram_statistics (next_token_id);
