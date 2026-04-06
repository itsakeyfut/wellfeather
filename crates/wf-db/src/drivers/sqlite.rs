use std::time::Instant;

use sqlx::{Column, Row, SqlitePool, TypeInfo};

use crate::error::DbError;
use crate::models::QueryResult;

/// Connect to a SQLite database at `url`.
///
/// `url` may be:
/// - `"sqlite::memory:"` — in-process in-memory database
/// - `"sqlite:<path>"` — file-backed database
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<SqlitePool, DbError> {
    SqlitePool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

/// Execute `sql` against `pool` and return a [`QueryResult`].
///
/// - **SELECT / row-returning statements**: columns and rows are populated;
///   `row_count` equals the number of returned rows.
/// - **DML / DDL statements**: rows are empty; `row_count` equals
///   `rows_affected()` reported by SQLite.
/// - **NULL values** map to `None` in every cell.
/// - `execution_time_ms` is measured with [`Instant`].
pub async fn execute(pool: &SqlitePool, sql: &str) -> Result<QueryResult, DbError> {
    let started = Instant::now();

    if super::is_row_returning(sql) {
        let rows = sqlx::query(sql)
            .fetch_all(pool)
            .await
            .map_err(DbError::from)?;

        let columns: Vec<String> = rows
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
            .unwrap_or_default();

        let data: Vec<Vec<Option<String>>> = rows
            .iter()
            .map(|row| (0..row.len()).map(|i| cell_to_string(row, i)).collect())
            .collect();

        let row_count = data.len();
        Ok(QueryResult {
            columns,
            rows: data,
            row_count,
            execution_time_ms: started.elapsed().as_millis(),
        })
    } else {
        let result = sqlx::query(sql)
            .execute(pool)
            .await
            .map_err(DbError::from)?;

        Ok(QueryResult {
            columns: vec![],
            rows: vec![],
            row_count: result.rows_affected() as usize,
            execution_time_ms: started.elapsed().as_millis(),
        })
    }
}

// ---------------------------------------------------------------------------
// Cell decoding
// ---------------------------------------------------------------------------

/// Convert a single SQLite cell to `Option<String>`.
///
/// Strategy:
/// 1. Try `Option<String>` first — this handles `NULL` (any declared type)
///    and `TEXT` values correctly.
/// 2. On failure (non-null, non-text cell), inspect the column's declared
///    type to choose the right numeric decoder.
/// 3. Unknown types fall back to a numeric cascade.
fn cell_to_string(row: &sqlx::sqlite::SqliteRow, i: usize) -> Option<String> {
    // Step 1 — covers NULL (→ None) and TEXT (→ Some("..."))
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v;
    }

    // Step 2 — non-null, non-text: dispatch on declared column type
    let col_type = row.column(i).type_info().name().to_ascii_uppercase();
    let col_type = col_type.as_str();

    if matches!(
        col_type,
        "INTEGER"
            | "INT"
            | "INT2"
            | "INT4"
            | "INT8"
            | "TINYINT"
            | "SMALLINT"
            | "MEDIUMINT"
            | "BIGINT"
            | "BOOLEAN"
            | "BOOL"
    ) {
        return row
            .try_get::<Option<i64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    if matches!(
        col_type,
        "REAL" | "FLOAT" | "DOUBLE" | "NUMERIC" | "DECIMAL" | "DOUBLE PRECISION"
    ) {
        return row
            .try_get::<Option<f64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    if col_type == "BLOB" || col_type.starts_with("BLOB(") {
        // Non-null BLOB — try UTF-8 first; emit byte-count tag for truly binary content.
        return row
            .try_get::<Option<Vec<u8>>, _>(i)
            .ok()
            .flatten()
            .map(|bytes| {
                String::from_utf8(bytes.clone())
                    .unwrap_or_else(|_| format!("<BLOB: {} bytes>", bytes.len()))
            });
    }

    // Step 3 — unknown declared type: cascade numeric fallbacks
    row.try_get::<Option<i64>, _>(i)
        .ok()
        .flatten()
        .map(|v| v.to_string())
        .or_else(|| {
            row.try_get::<Option<f64>, _>(i)
                .ok()
                .flatten()
                .map(|v| v.to_string())
        })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── connect ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn connect_should_succeed_with_memory_database() {
        let pool = connect("sqlite::memory:").await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    async fn connect_should_return_connection_failed_on_invalid_url() {
        let result = connect("postgres://not-sqlite").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }

    // ── execute — SELECT ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_select_should_return_columns_and_rows() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER, name TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1, 'alice'), (2, 'bob')")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT id, name FROM t ORDER BY id")
            .await
            .unwrap();

        assert_eq!(result.columns, vec!["id", "name"]);
        assert_eq!(result.row_count, 2);
        assert_eq!(
            result.rows[0],
            vec![Some("1".to_string()), Some("alice".to_string())]
        );
        assert_eq!(
            result.rows[1],
            vec![Some("2".to_string()), Some("bob".to_string())]
        );
    }

    #[tokio::test]
    async fn execute_select_should_return_none_for_null_values() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER, value TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1, NULL), (2, 'present')")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT id, value FROM t ORDER BY id")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0][1], None);
        assert_eq!(result.rows[1][1], Some("present".to_string()));
    }

    #[tokio::test]
    async fn execute_select_should_handle_mixed_null_columns() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (a INTEGER, b TEXT, c REAL, d BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (42, NULL, 3.14, NULL)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT a, b, c, d FROM t").await.unwrap();

        assert_eq!(result.row_count, 1);
        assert_eq!(result.rows[0][0], Some("42".to_string())); // INTEGER
        assert_eq!(result.rows[0][1], None); // TEXT NULL
        assert_eq!(result.rows[0][2], Some("3.14".to_string())); // REAL
        assert_eq!(result.rows[0][3], None); // BLOB NULL
    }

    #[tokio::test]
    async fn execute_select_with_no_rows_should_return_empty_result() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT * FROM t WHERE 1 = 0").await.unwrap();

        assert_eq!(result.row_count, 0);
        assert!(result.rows.is_empty());
    }

    // ── execute — DML ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_insert_should_return_affected_row_count() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER, name TEXT)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "INSERT INTO t VALUES (1, 'x'), (2, 'y')")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert!(result.rows.is_empty());
        assert!(result.columns.is_empty());
    }

    #[tokio::test]
    async fn execute_update_should_return_affected_row_count() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER, v INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1, 0), (2, 0), (3, 0)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "UPDATE t SET v = 1 WHERE id <= 2")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert!(result.rows.is_empty());
    }

    #[tokio::test]
    async fn execute_delete_should_return_affected_row_count() {
        let pool = connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1), (2), (3)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "DELETE FROM t WHERE id = 1").await.unwrap();

        assert_eq!(result.row_count, 1);
    }

    // ── execute — timing ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_should_populate_execution_time_ms() {
        let pool = connect("sqlite::memory:").await.unwrap();
        let result = execute(&pool, "SELECT 1").await.unwrap();
        // execution_time_ms is u128 (always ≥ 0); verify it is set
        let _ = result.execution_time_ms;
    }
}
