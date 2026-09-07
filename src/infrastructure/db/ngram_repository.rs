/// A candidate next token paired with the observed count for an n-gram prefix.
///
/// n-grams are kept in the in-memory cache (not PostgreSQL), so this module
/// only carries the shared result type used by the store backends.
pub type NgramCandidate = (i64, i64);