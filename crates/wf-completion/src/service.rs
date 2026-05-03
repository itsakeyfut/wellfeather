//! `CompletionService` — pure async compute wrapper over parser + engine + cache.
//!
//! The 300 ms debounce and the Slint timer logic live in `app/src/ui/mod.rs`
//! (the only crate that depends on Slint).  This crate provides only the
//! metadata lookup and candidate generation.

use wf_db::models::DbMetadata;

use crate::{
    CompletionItem,
    cache::MetadataCache,
    engine::CompletionEngine,
    parser::{CompletionContext, is_sql_keyword, parse_context},
};

// ---------------------------------------------------------------------------
// CompletionService
// ---------------------------------------------------------------------------

/// Wraps [`MetadataCache`] + [`CompletionEngine`] to compute candidates for
/// a single SQL text + cursor position.
///
/// The caller is responsible for debounce and for sending the result to the UI.
#[derive(Clone)]
pub struct CompletionService {
    cache: MetadataCache,
}

impl CompletionService {
    /// Create a service backed by the given metadata cache.
    pub fn new(cache: MetadataCache) -> Self {
        Self { cache }
    }

    /// Return completion candidates for `sql` at byte offset `cursor_pos`.
    ///
    /// Looks up `DbMetadata` for `conn_id` in the cache; falls back to an
    /// empty metadata set when the connection is not yet cached.
    pub async fn complete(
        &self,
        conn_id: &str,
        sql: &str,
        cursor_pos: usize,
    ) -> Vec<CompletionItem> {
        let metadata: DbMetadata = self.cache.load(conn_id).await.unwrap_or_default();
        let prefix = extract_prefix(sql, cursor_pos);
        let context = parse_context(sql, cursor_pos);

        // When the cursor sits at the end of a name that exactly matches a known
        // table or view, the user has finished typing the table name and is
        // waiting to see what comes next.  Simulate a trailing space so that
        // parse_context returns the correct "next-context" (NextClause or JoinOn)
        // and the popup appears without requiring the user to press Space first.
        if matches!(&context, CompletionContext::TableName) && !prefix.is_empty() {
            let exact_match = metadata
                .tables
                .iter()
                .chain(metadata.views.iter())
                .any(|t| t.name.eq_ignore_ascii_case(&prefix));
            if exact_match {
                let virtual_sql = format!("{} ", &sql[..cursor_pos.min(sql.len())]);
                let next_ctx = parse_context(&virtual_sql, virtual_sql.len());
                if matches!(
                    next_ctx,
                    CompletionContext::NextClause | CompletionContext::JoinOn
                ) {
                    return CompletionEngine::complete(next_ctx, &metadata, "");
                }
            }
        }

        // Same idea for ColumnName: if the prefix exactly matches a known column name,
        // return Operator candidates (`=`, `!=`, `IS NULL`, …) so the next popup
        // appears without requiring a Space keypress.
        if let CompletionContext::ColumnName { table: ref tbl } = context
            && !prefix.is_empty()
        {
            let table_filter = tbl.as_deref();
            let exact_col = metadata
                .tables
                .iter()
                .chain(metadata.views.iter())
                .filter(|t| table_filter.is_none_or(|tn| t.name.eq_ignore_ascii_case(tn)))
                .any(|t| {
                    t.columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(&prefix))
                });
            if exact_col {
                let virtual_sql = format!("{} ", &sql[..cursor_pos.min(sql.len())]);
                let next_ctx = parse_context(&virtual_sql, virtual_sql.len());
                if matches!(next_ctx, CompletionContext::Operator) {
                    return CompletionEngine::complete(next_ctx, &metadata, "");
                }
            }
        }

        // For ColumnName { table: None } with a space before the current word, try a
        // compound prefix that spans two words (e.g. "name u" for "name (users)").
        // This lets users narrow column disambiguation by typing part of the table name
        // with a space separator, not just without (e.g. "nameuse").
        // Two-pass: compound result wins if non-empty; falls back to simple prefix.
        if matches!(&context, CompletionContext::ColumnName { table: None })
            && let Some(compound) = extract_compound_prefix(sql, cursor_pos)
        {
            let compound_items = CompletionEngine::complete(context.clone(), &metadata, &compound);
            if !compound_items.is_empty() {
                return compound_items;
            }
        }

        CompletionEngine::complete(context, &metadata, &prefix)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a compound prefix `"prev_word current_word"` when the cursor is at
/// `"prev_word<space>current_word|"` and `prev_word` is not a SQL keyword.
///
/// Returns `None` when the compound prefix would be the same as the simple
/// prefix (no previous word, previous word is a keyword, or immediately preceded
/// by a non-space non-word character like `.`).
fn extract_compound_prefix(sql: &str, cursor_pos: usize) -> Option<String> {
    let pos = cursor_pos.min(sql.len());
    let before = &sql[..pos];

    // Current word (scan back past alphanumeric / underscore).
    let current_word_len = before
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    let current_word_start = pos - current_word_len;
    let current_word = &before[current_word_start..];

    // There must be at least one space immediately before the current word.
    let before_word = &before[..current_word_start];
    if !before_word.ends_with(' ') && !before_word.ends_with('\t') {
        return None;
    }

    // Scan back past that whitespace to find the previous word.
    let trimmed = before_word.trim_end_matches([' ', '\t']);
    if trimmed.is_empty() {
        return None;
    }

    let prev_word_len = trimmed
        .bytes()
        .rev()
        .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
        .count();
    if prev_word_len == 0 {
        return None;
    }

    let prev_word_start = trimmed.len() - prev_word_len;

    // Reject if the character before the previous word is `.` or alphanumeric/`_`
    // (means the previous word is part of a qualified name like `users.name`).
    if prev_word_start > 0 {
        let ch = trimmed.as_bytes()[prev_word_start - 1];
        if ch == b'.' || ch.is_ascii_alphanumeric() || ch == b'_' {
            return None;
        }
    }

    let prev_word = &trimmed[prev_word_start..];

    // Reject if the previous word is a SQL keyword (e.g. SELECT, FROM, WHERE …).
    if is_sql_keyword(prev_word) {
        return None;
    }

    Some(format!("{} {}", prev_word, current_word))
}

/// Extract the typed prefix at `cursor_pos`.
///
/// Scans backward from the cursor and returns the last run of word characters
/// (`[A-Za-z0-9_]`).  In dot-notation (`alias.col|`), only the segment after
/// the last `.` is returned.
pub(crate) fn extract_prefix(sql: &str, cursor_pos: usize) -> String {
    let pos = cursor_pos.min(sql.len());
    let before = &sql[..pos];
    let search_start = before.rfind('.').map(|i| i + 1).unwrap_or(0);
    let segment = &before[search_start..];
    segment
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;
    use wf_db::models::{ColumnInfo, TableInfo};

    async fn open_memory_cache() -> MetadataCache {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        MetadataCache::new(pool).await.unwrap()
    }

    fn make_meta() -> DbMetadata {
        DbMetadata {
            tables: vec![TableInfo {
                name: "users".to_string(),
                columns: vec![
                    ColumnInfo {
                        name: "id".to_string(),
                        data_type: "integer".to_string(),
                        nullable: false,
                    },
                    ColumnInfo {
                        name: "email".to_string(),
                        data_type: "varchar".to_string(),
                        nullable: false,
                    },
                ],
            }],
            views: vec![],
            stored_procs: vec![],
            indexes: vec![],
        }
    }

    #[tokio::test]
    async fn complete_should_return_keyword_candidates_without_metadata() {
        let svc = CompletionService::new(open_memory_cache().await);
        let items = svc.complete("conn-1", "SEL", 3).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"SELECT"), "expected SELECT in {labels:?}");
    }

    #[tokio::test]
    async fn complete_should_return_table_candidates_from_cached_metadata() {
        let cache = open_memory_cache().await;
        cache.store("conn-1", make_meta()).await.unwrap();
        let svc = CompletionService::new(cache);
        // TableName context: "SELECT * FROM "
        let sql = "SELECT * FROM ";
        let items = svc.complete("conn-1", sql, sql.len()).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"users"), "expected 'users' in {labels:?}");
    }

    #[tokio::test]
    async fn complete_should_return_empty_column_candidates_when_no_metadata() {
        let svc = CompletionService::new(open_memory_cache().await);
        // No metadata stored — ColumnName { Some("users") } → no columns
        let sql = "SELECT id FROM users WHERE ";
        let items = svc.complete("conn-x", sql, sql.len()).await;
        assert!(
            items.iter().all(|i| i.label != "id"),
            "expected no column items without metadata"
        );
    }

    #[test]
    fn extract_prefix_should_return_typed_word_before_cursor() {
        assert_eq!(extract_prefix("SELECT sel", 10), "sel");
        assert_eq!(extract_prefix("FROM ord", 8), "ord");
    }

    #[test]
    fn extract_prefix_should_return_empty_after_space() {
        assert_eq!(extract_prefix("SELECT ", 7), "");
        assert_eq!(extract_prefix("SELECT * FROM ", 14), "");
    }

    #[test]
    fn extract_prefix_should_return_segment_after_dot() {
        assert_eq!(extract_prefix("u.em", 4), "em");
        assert_eq!(extract_prefix("u.", 2), "");
        assert_eq!(extract_prefix("alias.col_name", 14), "col_name");
    }

    #[tokio::test]
    async fn complete_should_return_next_clause_when_exact_table_name_at_cursor_end() {
        let cache = open_memory_cache().await;
        cache.store("conn-1", make_meta()).await.unwrap();
        let svc = CompletionService::new(cache);
        // Cursor at end of "users" — exact table match → NextClause candidates
        let sql = "SELECT * FROM users";
        let items = svc.complete("conn-1", sql, sql.len()).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels
                .iter()
                .any(|l| l.contains("WHERE") || l.contains("ORDER")),
            "expected NextClause candidates (WHERE / ORDER BY) in {labels:?}"
        );
    }

    #[tokio::test]
    async fn complete_should_return_operator_when_exact_column_name_at_cursor_end() {
        let cache = open_memory_cache().await;
        cache.store("conn-1", make_meta()).await.unwrap();
        let svc = CompletionService::new(cache);
        // Cursor at end of "id" — exact column match → Operator candidates
        let sql = "SELECT * FROM users WHERE id";
        let items = svc.complete("conn-1", sql, sql.len()).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| *l == "=" || l.contains("IS NULL")),
            "expected Operator candidates (=, IS NULL, …) in {labels:?}"
        );
    }

    #[tokio::test]
    async fn complete_should_not_promote_partial_table_name_to_next_clause() {
        let cache = open_memory_cache().await;
        cache.store("conn-1", make_meta()).await.unwrap();
        let svc = CompletionService::new(cache);
        // "use" is a prefix of "users" but not an exact match — must stay TableName
        let sql = "SELECT * FROM use";
        let items = svc.complete("conn-1", sql, sql.len()).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"users"),
            "expected TableName candidates (users) in {labels:?}"
        );
        assert!(
            !labels.iter().any(|l| l.contains("WHERE")),
            "must not contain NextClause candidates: {labels:?}"
        );
    }
}
