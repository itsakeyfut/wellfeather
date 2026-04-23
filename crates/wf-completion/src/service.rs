//! `CompletionService` — pure async compute wrapper over parser + engine + cache.
//!
//! The 300 ms debounce and the Slint timer logic live in `app/src/ui/mod.rs`
//! (the only crate that depends on Slint).  This crate provides only the
//! metadata lookup and candidate generation.

use wf_db::models::DbMetadata;

use crate::{
    CompletionItem, cache::MetadataCache, engine::CompletionEngine, parser::parse_context,
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
        CompletionEngine::complete(context, &metadata, &prefix)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
    use wf_db::models::{ColumnInfo, TableInfo};

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
        let dir = tempfile::tempdir().unwrap();
        let cache = MetadataCache::new(dir.path().join("m.db"));
        let svc = CompletionService::new(cache);
        let items = svc.complete("conn-1", "SEL", 3).await;
        let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"SELECT"), "expected SELECT in {labels:?}");
    }

    #[tokio::test]
    async fn complete_should_return_table_candidates_from_cached_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let cache = MetadataCache::new(dir.path().join("m.db"));
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
        let dir = tempfile::tempdir().unwrap();
        let cache = MetadataCache::new(dir.path().join("m.db"));
        let svc = CompletionService::new(cache);
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
}
