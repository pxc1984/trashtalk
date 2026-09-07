-- Hybrid store: normalized message fragments are persisted here so the
-- in-memory n-gram/token cache can be rebuilt from DB history on startup
-- without needing the original export files.
ALTER TABLE messages ADD COLUMN IF NOT EXISTS fragments jsonb;