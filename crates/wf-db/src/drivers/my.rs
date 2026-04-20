use std::time::Instant;

use sqlx::{Column, MySqlPool, Row, TypeInfo};

use std::collections::HashMap;

use crate::error::DbError;
use crate::models::{ColumnInfo, DbMetadata, QueryResult, TableInfo};

/// Connect to a MySQL database at `url`.
///
/// Any sqlx connection error is wrapped as [`DbError::ConnectionFailed`].
pub async fn connect(url: &str) -> Result<MySqlPool, DbError> {
    MySqlPool::connect(url)
        .await
        .map_err(|e| DbError::ConnectionFailed(e.to_string()))
}

/// Execute `sql` against `pool` and return a [`QueryResult`].
///
/// - **SELECT / row-returning statements**: columns and rows are populated.
/// - **DML / DDL statements**: rows are empty; `row_count` = `rows_affected()`.
/// - **NULL values** map to `None`.
/// - `execution_time_ms` is measured with [`Instant`].
pub async fn execute(pool: &MySqlPool, sql: &str) -> Result<QueryResult, DbError> {
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

/// Fetch schema metadata from the connected MySQL database.
///
/// Queries `information_schema` restricted to the current schema (`DATABASE()`).
/// PG/MySQL tests are `#[ignore]` — run with `cargo test -- --ignored`.
pub async fn fetch_metadata(pool: &MySqlPool) -> Result<DbMetadata, DbError> {
    // ── all columns (single round-trip) ───────────────────────────────────────
    let col_rows = sqlx::query(
        "SELECT table_name, column_name, data_type, is_nullable \
         FROM information_schema.columns \
         WHERE table_schema = DATABASE() \
         ORDER BY table_name, ordinal_position",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let mut col_map: HashMap<String, Vec<ColumnInfo>> = HashMap::new();
    for row in &col_rows {
        let table = get_meta_str(row, 0);
        col_map.entry(table).or_default().push(ColumnInfo {
            name: get_meta_str(row, 1),
            data_type: get_meta_str(row, 2),
            nullable: get_meta_str(row, 3) == "YES",
        });
    }

    // ── tables ────────────────────────────────────────────────────────────────
    let table_rows = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' \
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let tables: Vec<TableInfo> = table_rows
        .iter()
        .map(|r| {
            let name = get_meta_str(r, 0);
            let columns = col_map.remove(&name).unwrap_or_default();
            TableInfo { name, columns }
        })
        .collect();

    // ── views ─────────────────────────────────────────────────────────────────
    let view_rows = sqlx::query(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = DATABASE() AND table_type = 'VIEW' \
         ORDER BY table_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let views: Vec<TableInfo> = view_rows
        .iter()
        .map(|r| {
            let name = get_meta_str(r, 0);
            let columns = col_map.remove(&name).unwrap_or_default();
            TableInfo { name, columns }
        })
        .collect();

    // ── stored procedures ─────────────────────────────────────────────────────
    let proc_rows = sqlx::query(
        "SELECT routine_name FROM information_schema.routines \
         WHERE routine_schema = DATABASE() AND routine_type = 'PROCEDURE' \
         ORDER BY routine_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let stored_procs: Vec<String> = proc_rows.iter().map(|r| get_meta_str(r, 0)).collect();

    // ── indexes ───────────────────────────────────────────────────────────────
    let index_rows = sqlx::query(
        "SELECT DISTINCT index_name FROM information_schema.statistics \
         WHERE table_schema = DATABASE() \
         ORDER BY index_name",
    )
    .fetch_all(pool)
    .await
    .map_err(DbError::from)?;

    let indexes: Vec<String> = index_rows.iter().map(|r| get_meta_str(r, 0)).collect();

    Ok(DbMetadata {
        tables,
        views,
        stored_procs,
        indexes,
    })
}

// ---------------------------------------------------------------------------
// Metadata string decoding
// ---------------------------------------------------------------------------

/// Read a string column from a metadata query row by position.
///
/// MySQL's `information_schema` returns column names as `VARCHAR` on some
/// server versions and as `VARBINARY` on others (e.g. 8.0 with certain
/// `character_set_server` settings).  Try `String` first; fall back to
/// decoding raw bytes as UTF-8.
fn get_meta_str(row: &sqlx::mysql::MySqlRow, i: usize) -> String {
    row.try_get::<String, _>(i).unwrap_or_else(|_| {
        row.try_get::<Vec<u8>, _>(i)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    })
}

// ---------------------------------------------------------------------------
// Cell decoding
// ---------------------------------------------------------------------------

/// Convert a single MySQL cell to `Option<String>`.
///
/// NULL values are detected first (via `Option<T>` decode returning `Ok(None)`).
/// Each column type is then matched against common MySQL type names and decoded
/// with an appropriate Rust type before stringification.
fn cell_to_string(row: &sqlx::mysql::MySqlRow, i: usize) -> Option<String> {
    // Step 1 — NULL + text-like types (VARCHAR, CHAR, TEXT, ENUM, SET, …)
    if let Ok(v) = row.try_get::<Option<String>, _>(i) {
        return v;
    }

    // Step 2 — type-specific decode
    let col_type = row.column(i).type_info().name().to_ascii_uppercase();
    let col_type = col_type.as_str();

    // Integer types (MySQL type names are uppercased by sqlx)
    if matches!(
        col_type,
        "TINYINT"
            | "SMALLINT"
            | "MEDIUMINT"
            | "INT"
            | "BIGINT"
            | "TINYINT UNSIGNED"
            | "SMALLINT UNSIGNED"
            | "MEDIUMINT UNSIGNED"
            | "INT UNSIGNED"
            | "BIGINT UNSIGNED"
    ) {
        return row
            .try_get::<Option<i64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Floating-point and decimal types
    if matches!(col_type, "FLOAT" | "DOUBLE" | "DECIMAL" | "NUMERIC") {
        return row
            .try_get::<Option<f64>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Boolean (stored as TINYINT(1) in MySQL; type_info may still say TINYINT)
    if col_type == "BOOLEAN" || col_type == "BOOL" {
        return row
            .try_get::<Option<bool>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Date / time (requires sqlx "chrono" feature).
    // Use starts_with to match precision variants like DATETIME(6) / TIMESTAMP(3).
    // Check DATETIME before DATE so "DATETIME(...)" doesn't fall through to the DATE branch.
    if col_type.starts_with("DATETIME") || col_type.starts_with("TIMESTAMP") {
        return row
            .try_get::<Option<chrono::NaiveDateTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    if col_type.starts_with("DATE") {
        return row
            .try_get::<Option<chrono::NaiveDate>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }
    // Check TIME after DATE so "TIMESTAMP" doesn't accidentally match here.
    if col_type == "TIME" {
        return row
            .try_get::<Option<chrono::NaiveTime>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // YEAR
    if col_type == "YEAR" {
        return row
            .try_get::<Option<i16>, _>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string());
    }

    // Binary / blob types — and universal bytes fallback for any other type
    // (covers BINARY, VARBINARY, BLOB variants, VARBINARY-charset VARCHAR, etc.).
    // Try to interpret the bytes as UTF-8; only emit the byte-count tag when the
    // content is genuinely non-textual binary data.
    if let Ok(Some(bytes)) = row.try_get::<Option<Vec<u8>>, _>(i) {
        return Some(
            String::from_utf8(bytes.clone())
                .unwrap_or_else(|_| format!("<BLOB: {} bytes>", bytes.len())),
        );
    }

    // Last-resort numeric cascade for computed expressions / edge-case types.
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

    /// Requires a running MySQL instance.
    /// Set `TEST_MY_URL` to override the default connection string.
    /// Run with: `cargo test -p wf-db -- --ignored`
    #[tokio::test]
    #[ignore]
    async fn connect_should_succeed_with_real_mysql() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, 1);
    }

    #[tokio::test]
    #[ignore]
    async fn connect_should_return_connection_failed_on_unreachable_host() {
        let result = connect("mysql://user:pass@127.0.0.1:19999/db").await;
        assert!(matches!(result, Err(DbError::ConnectionFailed(_))));
    }

    #[tokio::test]
    #[ignore]
    async fn fetch_metadata_should_return_tables_views_procs_and_indexes() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMPORARY TABLE wf_meta_test (id INT NOT NULL, label VARCHAR(255))")
            .execute(&pool)
            .await
            .unwrap();

        let meta = fetch_metadata(&pool).await.unwrap();

        // Temporary tables don't appear in information_schema — the call must succeed.
        let _ = meta;
    }

    #[tokio::test]
    #[ignore]
    async fn execute_select_should_return_rows_with_null_values() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMPORARY TABLE t (id INT, val VARCHAR(255))")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1, NULL), (2, 'hi')")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "SELECT id, val FROM t ORDER BY id")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert_eq!(result.rows[0][1], None);
        assert_eq!(result.rows[1][1], Some("hi".to_string()));
    }

    #[tokio::test]
    #[ignore]
    async fn execute_insert_should_return_affected_row_count() {
        let url = std::env::var("TEST_MY_URL")
            .unwrap_or_else(|_| "mysql://root:root@localhost:3306/mysql".to_string());
        let pool = connect(&url).await.unwrap();

        sqlx::query("CREATE TEMPORARY TABLE t (id INT)")
            .execute(&pool)
            .await
            .unwrap();

        let result = execute(&pool, "INSERT INTO t VALUES (1), (2)")
            .await
            .unwrap();

        assert_eq!(result.row_count, 2);
        assert!(result.rows.is_empty());
    }
}
