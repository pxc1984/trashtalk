/// A resolved token vocabulary row. The vocabulary itself lives in the
/// in-memory cache (not PostgreSQL), so this module only carries the shared
/// record type used by the store backends.
#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub id: i64,
    pub token_type: String,
    pub token_value: Option<String>,
    pub emoji_id: Option<i64>,
}